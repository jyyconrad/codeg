//! 综合轮次记录和对话总结，生成工作、能力与知识笔记。
//! engine/worker触发本模块；模型依据当前来源路径和项目元数据自主整理，
//! 主机补充来源关联，再通过commit写文件、wiki_pipeline_service登记处理进度。
//! 输入摘要只用于重复处理去重，文件版本变化不构成整理前置条件。

use crate::db::entities::wiki_job;
use crate::db::error::DbError;
use crate::db::service::{wiki_pipeline_service as pipeline, wiki_service};
use crate::wiki::commit::{self, StagedProposal};
use crate::wiki::llm::{WikiLlm, WikiLlmError, SYNTHESIZE_CONTRACT_VERSION};
use crate::wiki::raw::content_hash;
use crate::wiki::result::{JobOutputManifest, ProcessedInput, WikiInput, WikiOutput};
use crate::wiki::vault::{self, CONTENT_END, CONTENT_START};
use chrono::Utc;
use sea_orm::DatabaseConnection;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[path = "synthesis/proposals.rs"]
mod proposals;
use proposals::validate_output;
#[path = "synthesis/batching.rs"]
mod batching;
use batching::plan_batches;

pub const LEAF_BODY_HARD_CHARS: usize = 100_000;
pub const DOMAIN_TYPES: &[&str] = &[
    "work-record",
    "decision",
    "outcome",
    "project",
    "area",
    "capability",
    "concept",
    "method",
    "entity",
];

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Blocked(String),
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Llm(#[from] WikiLlmError),
    #[error(transparent)]
    Db(#[from] DbError),
}
impl CompileError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "invalid_output",
            Self::Conflict(_) => "write_conflict",
            Self::Blocked(_) => "model_unavailable",
            Self::Failed(_) => "source_read_failed",
            Self::Llm(e) => e.error_code(),
            Self::Db(DbError::Conflict(_)) => "write_conflict",
            Self::Db(_) => "database",
        }
    }
    pub fn retryable(&self) -> bool {
        match self {
            Self::Llm(e) => e.retryable(),
            Self::Db(DbError::Database(_)) => true,
            _ => false,
        }
    }
}
impl From<commit::CommitError> for CompileError {
    fn from(e: commit::CommitError) -> Self {
        match e {
            commit::CommitError::Conflict(s) => Self::Conflict(s),
            commit::CommitError::Validation(s) => Self::Validation(s),
            commit::CommitError::Io(s) => Self::Failed(s),
        }
    }
}
impl From<crate::wiki::fs_policy::FsPolicyError> for CompileError {
    fn from(e: crate::wiki::fs_policy::FsPolicyError) -> Self {
        Self::Validation(e.to_string())
    }
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryNoteRef {
    pub rel: String,
    #[serde(skip_serializing)]
    pub content_hash: String,
    pub project_binding_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Default)]
pub struct PendingInputs {
    pub memory_notes: Vec<MemoryNoteRef>,
}
#[derive(Debug, Clone, Default)]
pub struct NoteIndexEntry {
    pub note_id: String,
    pub rel: String,
    pub page_type: String,
    pub title: String,
    pub hash: String,
    /// 替换旧笔记时保留原生成区正文，避免替代关系抹掉历史内容。
    pub body: String,
}
#[derive(Clone)]
struct LoadedMemory {
    reference: MemoryNoteRef,
    text: String,
    input: WikiInput,
}

pub async fn run_compile_job(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<JobOutputManifest, CompileError> {
    vault::initialize_vault(vault).map_err(|e| CompileError::Failed(e.to_string()))?;
    vault::initialize_state_root(state_root).map_err(|e| CompileError::Failed(e.to_string()))?;
    // 先完成已写入页面的数据库登记，避免重试重新分配身份或重复生成同一批内容。
    recover_job_batches(conn, state_root, Some(&job.id)).await?;
    // 每次执行按当前文件重新列出待整理材料，排队记录只用于界面展示，不锁定输入版本。
    let manifest = list_pending_inputs(conn, &job.vault_id, vault).await?;
    wiki_service::set_job_input_manifest(
        conn,
        &job.id,
        &serde_json::to_string(&manifest).map_err(|e| CompileError::Validation(e.to_string()))?,
    )
    .await?;
    let loaded = manifest
        .memory_notes
        .iter()
        .map(|reference| load_memory(vault, reference))
        .collect::<Result<Vec<_>, _>>()?;
    let mut aggregate = pipeline::load_job_result(conn, &job.id)
        .await?
        .unwrap_or_else(|| empty_result("no_new_input"));
    if loaded.is_empty() {
        if !aggregate.outputs.is_empty() {
            aggregate.outcome = "generated".into();
        } else if !aggregate.processed_inputs.is_empty() {
            aggregate.outcome = "no_content".into();
            aggregate.reason_code = Some("no_durable_content".into());
        }
        aggregate.remaining_inputs.clear();
        return Ok(aggregate);
    }
    let all_inputs: Vec<_> = loaded.iter().map(|n| n.input.clone()).collect();
    let budget = synthesis_budget(llm, &scan_note_index(vault))?;
    let batches = plan_batches(loaded, budget);
    let mut completed: HashSet<(String, String)> = aggregate
        .processed_inputs
        .iter()
        .map(|i| (i.rel.clone(), i.content_hash.clone()))
        .collect();
    for (number, batch) in batches.into_iter().enumerate() {
        llm.check_cancelled()?;
        let batch_id = format!("batch-{:04}", number + 1);
        let staging = state_root
            .join("staging")
            .join(&job.id)
            .join(format!("attempt-{}", job.attempt))
            .join(&batch_id);
        fs::create_dir_all(&staging).map_err(|e| CompileError::Failed(e.to_string()))?;
        let index = scan_note_index(vault);
        let batch_inputs: Vec<_> = batch.iter().map(|n| n.input.clone()).collect();
        let sources: Vec<_> = batch_inputs
            .iter()
            .map(|input| {
                json!({
                    "rel": input.rel, "source_ids": input.source_ids
                })
            })
            .collect();
        let project_ids: Vec<_> = batch
            .iter()
            .flat_map(|note| note.reference.project_binding_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let project_metadata =
            crate::wiki::project_metadata::collect_for_bindings(conn, &job.vault_id, &project_ids)
                .await?;
        let payload = json!({
            "schema":SYNTHESIZE_CONTRACT_VERSION,"job_id":job.id,"attempt":job.attempt,
            "source_references":sources,
            "project_metadata":project_metadata,
            "batch_id":batch_id,
            "index":index_payload(&index),
            "vault_abs":vault.to_string_lossy(),"staging_abs":staging.to_string_lossy(),
            "max_turns":crate::agent::model::WIKI_COMPILE_MAX_TURNS,
        });
        let run = llm.complete_json("synthesize", payload).await?;
        let mut validated = validate_output(run.output, &batch_inputs, &index)?;
        decorate_sources(conn, &job.vault_id, &batch, &mut validated.proposals).await?;
        llm.check_cancelled()?;
        completed.extend(
            validated
                .processed
                .iter()
                .map(|i| (i.rel.clone(), i.content_hash.clone())),
        );
        let remaining = all_inputs
            .iter()
            .filter(|i| !completed.contains(&(i.rel.clone(), i.content_hash.clone())))
            .cloned()
            .collect();
        let mut result = empty_result(if validated.proposals.is_empty() {
            "no_content"
        } else {
            "generated"
        });
        if validated.proposals.is_empty() {
            result.reason_code = Some("no_durable_content".into());
        }
        result.processed_inputs = validated.processed;
        result.remaining_inputs = remaining;
        result.warnings = validated.warnings;
        let metadata = commit::BatchMetadata {
            vault_id: job.vault_id.clone(),
            attempt: job.attempt,
            batch_id,
            contract_version: SYNTHESIZE_CONTRACT_VERSION.into(),
            result,
            source_ids_by_note: BTreeMap::new(),
        };
        // 进入提交后必须完成文件与数据库登记；取消只截断下一步，避免留下无法追踪的页面。
        let mut committed =
            commit::commit_batch(vault, state_root, &job.id, &validated.proposals, metadata)?;
        let batch = committed.batch.as_mut().expect("batch commit metadata");
        batch.result.outputs = outputs_from_commit(&committed.files, &committed.after_contents)?;
        commit::persist_manifest(state_root, &committed)?;
        aggregate = finalize_committed(conn, state_root, &mut committed).await?;
    }
    aggregate.outcome = if aggregate.outputs.is_empty() {
        "no_content"
    } else {
        "generated"
    }
    .into();
    aggregate.reason_code = if aggregate.outputs.is_empty() {
        Some("no_durable_content".into())
    } else {
        None
    };
    aggregate.remaining_inputs.clear();
    Ok(aggregate)
}

fn index_payload(index: &[NoteIndexEntry]) -> Vec<Value> {
    index
        .iter()
        .filter(|e| DOMAIN_TYPES.contains(&e.page_type.as_str()))
        .map(|e| json!({"note_id":e.note_id,"path":e.rel,"type":e.page_type,"title":e.title}))
        .collect()
}
fn synthesis_budget(llm: &dyn WikiLlm, index: &[NoteIndexEntry]) -> Result<u64, CompileError> {
    let cost = crate::agent::context::budget::estimate_json(&index_payload(index));
    let available = llm.input_budget().saturating_sub(cost);
    if available < 1024 {
        return Err(CompileError::Blocked(
            "Wiki index does not fit the configured model context window".into(),
        ));
    }
    Ok(available)
}

async fn decorate_sources(
    conn: &DatabaseConnection,
    vault_id: &str,
    batch: &[LoadedMemory],
    proposals: &mut [StagedProposal],
) -> Result<(), CompileError> {
    for proposal in proposals {
        let source_ids = yaml_list(&proposal.after, "source_ids");
        let mut projects = BTreeSet::new();
        let mut source_paths = BTreeSet::new();
        let mut source_urls = BTreeSet::new();
        for source_id in &source_ids {
            let source = match wiki_service::get_source_model(conn, source_id).await {
                Ok(source) => source,
                Err(DbError::NotFound(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            if source.vault_id != vault_id {
                continue;
            }
            source_paths.extend(source.raw_path);
            source_urls.extend(source.source_url);
            if let Some(raw) = source.project_ids {
                let ids: Vec<String> = serde_json::from_str(&raw)
                    .map_err(|e| CompileError::Validation(e.to_string()))?;
                projects.extend(ids);
            }
        }
        let refs = yaml_list(&proposal.after, "sources");
        for note in batch {
            if refs.contains(&note.reference.rel) {
                projects.extend(note.reference.project_binding_ids.iter().cloned());
            }
        }
        let (yaml, body) = commit::split_frontmatter(&proposal.after)
            .ok_or_else(|| CompileError::Validation("missing staged metadata".into()))?;
        let mut metadata: serde_yaml::Mapping =
            serde_yaml::from_str(&yaml).map_err(|e| CompileError::Validation(e.to_string()))?;
        metadata.insert(
            "projects".into(),
            serde_yaml::to_value(projects).map_err(|e| CompileError::Validation(e.to_string()))?,
        );
        metadata.insert(
            "source_paths".into(),
            serde_yaml::to_value(source_paths)
                .map_err(|e| CompileError::Validation(e.to_string()))?,
        );
        metadata.insert(
            "source_urls".into(),
            serde_yaml::to_value(source_urls)
                .map_err(|e| CompileError::Validation(e.to_string()))?,
        );
        proposal.after = format!(
            "---\n{}---\n{}",
            serde_yaml::to_string(&metadata)
                .map_err(|e| CompileError::Validation(e.to_string()))?,
            body
        );
    }
    Ok(())
}

fn outputs_from_commit(
    files: &[commit::CommitFile],
    contents: &BTreeMap<String, String>,
) -> Result<Vec<WikiOutput>, CompileError> {
    files
        .iter()
        .map(|file| {
            let text = contents
                .get(&file.rel)
                .ok_or_else(|| CompileError::Validation("committed body missing".into()))?;
            Ok(WikiOutput {
                note_id: yaml_string(text, "codeg_note_id")
                    .ok_or_else(|| CompileError::Validation("committed identity missing".into()))?,
                path: file.rel.clone(),
                title: yaml_string(text, "title").unwrap_or_default(),
                page_type: file.page_type.clone(),
                content_hash: file.after_hash.clone(),
            })
        })
        .collect()
}
async fn finalize_committed(
    conn: &DatabaseConnection,
    state_root: &Path,
    manifest: &mut commit::CommitManifest,
) -> Result<JobOutputManifest, CompileError> {
    let metadata = manifest
        .batch
        .as_ref()
        .ok_or_else(|| CompileError::Validation("batch metadata missing".into()))?;
    let result = pipeline::finalize_batch(
        conn,
        &manifest.job_id,
        metadata,
        &commit::manifest_path(state_root, manifest.storage_id()).to_string_lossy(),
        &commit::immutable_manifest_hash(manifest)?,
    )
    .await?;
    manifest.finalized = true;
    commit::persist_manifest(state_root, manifest)?;
    Ok(result)
}
/// Recover jobs independently so one conflicting note cannot block unrelated
/// durable batches or prevent the scheduler from settling interrupted jobs.
pub async fn recover_batches_outcomes(
    conn: &DatabaseConnection,
    state_root: &Path,
) -> Vec<(String, Result<JobOutputManifest, CompileError>)> {
    let entries = match fs::read_dir(commit::commits_dir(state_root)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![(String::new(), Err(CompileError::Failed(e.to_string())))],
    };
    let mut jobs = BTreeSet::new();
    let mut outcomes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let parsed = fs::read_to_string(&path)
            .map_err(|e| CompileError::Failed(e.to_string()))
            .and_then(|raw| {
                serde_json::from_str::<commit::CommitManifest>(&raw)
                    .map_err(|e| CompileError::Validation(e.to_string()))
            });
        match parsed {
            Ok(m) if m.batch.is_some() && !m.finalized => {
                jobs.insert(m.job_id);
            }
            Ok(_) => {}
            Err(error) => outcomes.push((
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .split("-a")
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                Err(error),
            )),
        }
    }
    for id in jobs {
        match recover_job_batches(conn, state_root, Some(&id)).await {
            Ok(results) => {
                if let Some((_, result)) = results.into_iter().last() {
                    outcomes.push((id, Ok(result)));
                }
            }
            Err(error) => outcomes.push((id, Err(error))),
        }
    }
    outcomes
}

#[cfg(test)]
pub async fn recover_batches(
    conn: &DatabaseConnection,
    state_root: &Path,
) -> Result<Vec<(String, JobOutputManifest)>, CompileError> {
    recover_batches_outcomes(conn, state_root)
        .await
        .into_iter()
        .map(|(id, result)| result.map(|r| (id, r)))
        .collect()
}
async fn recover_job_batches(
    conn: &DatabaseConnection,
    state_root: &Path,
    job_id: Option<&str>,
) -> Result<Vec<(String, JobOutputManifest)>, CompileError> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(commit::commits_dir(state_root)) else {
        return Ok(out);
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for path in paths {
        if job_id.is_some_and(|id| {
            !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.starts_with(&format!("{id}-a")))
        }) {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| CompileError::Failed(e.to_string()))?;
        let mut manifest: commit::CommitManifest =
            serde_json::from_str(&text).map_err(|e| CompileError::Validation(e.to_string()))?;
        if manifest.batch.is_none()
            || manifest.finalized
            || job_id.is_some_and(|id| id != manifest.job_id)
        {
            continue;
        }
        let meta = manifest.batch.as_ref().unwrap();
        let hash = commit::immutable_manifest_hash(&manifest)?;
        // 数据库已登记的批次只补finalized标记，不能复活用户在提交后删除的笔记。
        if pipeline::batch_finalized(conn, &manifest.job_id, meta.attempt, &meta.batch_id, &hash)
            .await?
        {
            manifest.finalized = true;
            commit::persist_manifest(state_root, &manifest)?;
            if let Some(result) = pipeline::load_job_result(conn, &manifest.job_id).await? {
                out.push((manifest.job_id.clone(), result));
            }
            continue;
        }
        let vault = PathBuf::from(&manifest.vault);
        commit::recover_manifest(&vault, state_root, &mut manifest)?;
        if let Some(meta) = manifest.batch.as_mut() {
            meta.result.outputs = outputs_from_commit(&manifest.files, &manifest.after_contents)?;
        }
        commit::persist_manifest(state_root, &manifest)?;
        let result = finalize_committed(conn, state_root, &mut manifest).await?;
        out.push((manifest.job_id.clone(), result));
    }
    Ok(out)
}
fn load_memory(vault: &Path, reference: &MemoryNoteRef) -> Result<LoadedMemory, CompileError> {
    let path = crate::wiki::fs_policy::WikiFsPolicy::new(vault).check_commit_rel(&reference.rel)?;
    let text = fs::read_to_string(path)
        .map_err(|e| CompileError::Failed(format!("{}: {e}", reference.rel)))?;
    let mut source_ids = yaml_list(&text, "source_ids");
    source_ids.extend(yaml_list(&text, "sources").iter().filter_map(|s| {
        s.trim_start_matches("[[")
            .strip_prefix("sources/")
            .map(|s| {
                s.split(['|', ']', '.'])
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
    }));
    if let Some(source) = yaml_string(&text, "codeg_source_id") {
        source_ids.push(source);
    }
    source_ids.sort();
    source_ids.dedup();
    let input = WikiInput {
        rel: reference.rel.clone(),
        content_hash: content_hash(&text),

        source_ids,
    };
    Ok(LoadedMemory {
        reference: reference.clone(),
        text,
        input,
    })
}
fn empty_result(outcome: &str) -> JobOutputManifest {
    JobOutputManifest {
        version: 1,
        outcome: outcome.into(),
        reason_code: None,
        outputs: Vec::new(),
        processed_inputs: Vec::new(),
        remaining_inputs: Vec::new(),
        warnings: Vec::new(),
    }
}

pub async fn list_pending_inputs(
    conn: &DatabaseConnection,
    vault_id: &str,
    vault: &Path,
) -> Result<PendingInputs, CompileError> {
    let mut memory_notes = Vec::new();
    for note in scan_memory_notes_checked(vault)? {
        if !pipeline::memory_consumed(
            conn,
            vault_id,
            &note.rel,
            &note.content_hash,
            SYNTHESIZE_CONTRACT_VERSION,
        )
        .await?
        {
            memory_notes.push(note);
        }
    }
    memory_notes.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(PendingInputs { memory_notes })
}

pub(crate) fn check_leaf_body(page_type: &str, after: &str) -> Result<(), CompileError> {
    let length = body_of(after).chars().count();
    if length > LEAF_BODY_HARD_CHARS {
        return Err(CompileError::Validation(format!(
            "{page_type} exceeds the {LEAF_BODY_HARD_CHARS} character limit"
        )));
    }
    Ok(())
}
#[cfg(test)]
pub fn scan_memory_notes(vault: &Path) -> Vec<MemoryNoteRef> {
    scan_memory_notes_checked(vault).unwrap_or_default()
}
fn scan_memory_notes_checked(vault: &Path) -> Result<Vec<MemoryNoteRef>, CompileError> {
    let mut out = Vec::new();
    let policy = crate::wiki::fs_policy::WikiFsPolicy::new(vault);
    for dir in ["work/turns", "work/sessions"] {
        let path = vault.join(dir);
        if !path.exists() {
            continue;
        }
        for entry in fs::read_dir(&path).map_err(|e| CompileError::Failed(e.to_string()))? {
            let entry = entry.map_err(|e| CompileError::Failed(e.to_string()))?;
            if entry.path().extension().and_then(|e| e.to_str()) != Some("md")
                || entry.file_name() == "index.md"
            {
                continue;
            }
            let rel = format!("{dir}/{}", entry.file_name().to_string_lossy());
            let safe = policy.check_commit_rel(&rel)?;
            let text = fs::read_to_string(&safe)
                .map_err(|e| CompileError::Failed(format!("{rel}: {e}")))?;
            out.push(MemoryNoteRef {
                rel,
                content_hash: content_hash(&text),
                project_binding_ids: yaml_list(&text, "codeg_project_binding_id"),
            });
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

pub fn scan_note_index(vault: &Path) -> Vec<NoteIndexEntry> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(vault).into_iter();
    for ent in walker.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(vault)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if rel.starts_with("raw/") || rel.contains(".obsidian/") || rel.contains("/.git/") {
            continue;
        }
        if crate::wiki::fs_policy::WikiFsPolicy::new(vault)
            .check_commit_rel(&rel)
            .is_err()
        {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Some(note_id) = yaml_string(&text, "codeg_note_id") else {
            continue;
        };
        let page_type = yaml_string(&text, "type").unwrap_or_default();
        let title = yaml_string(&text, "title").unwrap_or_default();
        out.push(NoteIndexEntry {
            note_id,
            rel,
            page_type,
            title,
            hash: content_hash(&text),
            body: generated_body(&text),
        });
    }
    out
}

pub(crate) fn yaml_string(md: &str, key: &str) -> Option<String> {
    let (yaml, _) = commit::split_frontmatter(md)?;
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).ok()?;
    parsed.get(key)?.as_str().map(str::to_owned)
}

pub(crate) fn yaml_list(md: &str, key: &str) -> Vec<String> {
    let Some((yaml, _)) = commit::split_frontmatter(md) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_yaml::from_str::<serde_yaml::Value>(&yaml) else {
        return Vec::new();
    };
    match parsed.get(key) {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(serde_yaml::Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}
fn generated_body(md: &str) -> String {
    let body = body_of(md);
    body.split_once(CONTENT_START)
        .and_then(|(_, rest)| rest.split_once(CONTENT_END))
        .map(|(body, _)| body.trim().to_string())
        .unwrap_or(body)
}

pub(crate) fn body_of(md: &str) -> String {
    commit::split_frontmatter(md)
        .map(|(_, b)| b)
        .unwrap_or_else(|| md.to_string())
}
#[cfg(test)]
#[path = "synthesis/tests.rs"]
mod tests;

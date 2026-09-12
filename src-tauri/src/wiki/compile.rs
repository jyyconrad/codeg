//! Freeze compile input, segment, four host steps, stage, commit.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::entities::{wiki_job, wiki_source};
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::commit::{self, StagedProposal};
use crate::wiki::fs_policy::{page_type_matches_rel, WikiFsPolicy};
use crate::wiki::llm::{WikiLlm, WikiLlmError, COMPILE_CONTRACT_VERSION};
use crate::wiki::raw::{self, content_hash};
use crate::wiki::vault::{self, CONTENT_END, CONTENT_START};

pub const SEGMENT_CHAR_BUDGET: usize = 8000;
pub const MAX_CANDIDATES_PER_SEGMENT: usize = 5;
pub const ANALYSIS_CONFIG_REVISION: &str = "0";

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
    Db(#[from] DbError),
}

impl CompileError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation",
            Self::Conflict(_) => "conflict",
            Self::Blocked(_) => crate::wiki::llm::BLOCKED_BY_CONFIGURATION,
            Self::Failed(_) => "compile_failed",
            Self::Db(_) => "database",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Db(_))
    }
}

impl From<WikiLlmError> for CompileError {
    fn from(e: WikiLlmError) -> Self {
        match e {
            WikiLlmError::Blocked(s) => Self::Blocked(s),
            WikiLlmError::Failed(s) => Self::Failed(s),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenInput {
    pub source_id: String,
    pub raw_hash: String,
    pub annotation_revision: i32,
    pub segment_ids: Vec<String>,
    pub compile_contract_version: String,
    pub analysis_config_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileJobManifest {
    pub cutoff_source_seq: i64,
    pub inputs: Vec<FrozenInput>,
}

#[derive(Debug, Clone)]
pub struct NoteIndexEntry {
    pub note_id: String,
    pub rel: String,
    pub page_type: String,
    pub title: String,
    pub hash: String,
}

#[derive(Debug, Clone)]
pub struct CompileOutcome {
    pub committed: Vec<String>,
    pub nothing_to_persist: bool,
}

pub async fn run_compile_job(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<CompileOutcome, CompileError> {
    vault::initialize_vault(vault).map_err(|e| CompileError::Failed(e.to_string()))?;
    vault::initialize_state_root(state_root).map_err(|e| CompileError::Failed(e.to_string()))?;
    let staging = state_root.join("staging").join(&job.id);
    fs::create_dir_all(&staging).map_err(|e| CompileError::Failed(e.to_string()))?;

    let manifest = freeze_or_load_manifest(conn, job, vault).await?;
    wiki_service::set_job_input_manifest(
        conn,
        &job.id,
        &serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".into()),
    )
    .await?;

    if manifest.inputs.is_empty() {
        append_compile_log(vault, &job.id, "compile nothing_to_persist")?;
        return Ok(CompileOutcome {
            committed: Vec::new(),
            nothing_to_persist: true,
        });
    }

    let index = scan_note_index(vault);
    let mut all_candidates: Vec<Value> = Vec::new();
    let mut allowed_reads: Vec<PathBuf> = Vec::new();
    allowed_reads.push(vault.join("AGENTS.md"));
    for entry in &index {
        allowed_reads.push(vault.join(&entry.rel));
    }

    for input in &manifest.inputs {
        let source = wiki_service::get_source_model(conn, &input.source_id).await?;
        let raw_path = source
            .raw_path
            .as_deref()
            .ok_or_else(|| CompileError::Validation(format!("source {} has no raw", source.id)))?;
        let abs = vault.join(raw_path);
        allowed_reads.push(abs.clone());
        let raw_text = fs::read_to_string(&abs)
            .map_err(|e| CompileError::Failed(format!("read raw: {e}")))?;
        if source.raw_hash.as_deref() != Some(input.raw_hash.as_str()) {
            return Err(CompileError::Conflict(format!(
                "source {} raw hash changed",
                source.id
            )));
        }
        let segments = persist_segments(conn, &source, &raw_text, &input.segment_ids).await?;

        for (seg_id, seg_text) in &segments {
            let payload = json!({
                "source_id": source.id,
                "raw_hash": input.raw_hash,
                "annotation_revision": input.annotation_revision,
                "segment_ids": [seg_id],
                "segment_text": redacted_excerpt(seg_text),
                "compile_contract_version": COMPILE_CONTRACT_VERSION,
            });
            let out = llm.complete_json("candidates", payload).await?;
            let cands = validate_candidates(&out, &source.id, seg_id)?;
            all_candidates.extend(cands);
        }
    }

    let match_in = json!({
        "candidates": all_candidates,
        "index": index.iter().map(|e| json!({
            "codeg_note_id": e.note_id,
            "path": e.rel,
            "type": e.page_type,
            "title": e.title,
        })).collect::<Vec<_>>(),
    });
    let match_out = llm.complete_json("match", match_in.clone()).await?;
    let matches = match_out
        .get("matches")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let merge_in = json!({
        "candidates": all_candidates,
        "matches": matches,
        "index": match_in["index"],
    });
    let merge_out = llm.complete_json("merge", merge_in).await?;

    let policy = WikiFsPolicy::for_compile_job(vault, state_root, &staging, &allowed_reads);
    let mut proposals = proposals_from_merge(&merge_out, vault)?;
    if proposals.is_empty() {
        proposals = host_pages_from_candidates(
            vault,
            &all_candidates,
            &matches,
            &index,
            &manifest.inputs,
        )?;
    }
    for p in &proposals {
        policy.check_page_type_path(&p.page_type, &p.rel)?;
        let staged = staging.join(p.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = staged.parent() {
            fs::create_dir_all(parent).map_err(|e| CompileError::Failed(e.to_string()))?;
        }
        fs::write(&staged, &p.after).map_err(|e| CompileError::Failed(e.to_string()))?;
        policy
            .check_stage_write(&staged)
            .map_err(|e| CompileError::Validation(e.to_string()))?;
    }

    let _ = llm
        .complete_json(
            "finalize",
            json!({ "processed_inputs": manifest.inputs, "pages": proposals.iter().map(|p| &p.rel).collect::<Vec<_>>() }),
        )
        .await;

    if proposals.is_empty() {
        register_consumed(conn, &job.id, &manifest).await?;
        append_compile_log(vault, &job.id, "compile nothing_to_persist")?;
        return Ok(CompileOutcome {
            committed: Vec::new(),
            nothing_to_persist: true,
        });
    }

    let committed = commit::commit_proposals(vault, state_root, &job.id, &proposals)?;
    register_consumed(conn, &job.id, &manifest).await?;
    for file in &committed.files {
        let note_id = proposals
            .iter()
            .find(|p| p.rel == file.rel)
            .and_then(|p| yaml_string(&p.after, "codeg_note_id"))
            .unwrap_or_default();
        if note_id.is_empty() {
            continue;
        }
        for input in &manifest.inputs {
            wiki_service::insert_contribution(
                conn,
                &input.source_id,
                &input.raw_hash,
                input.annotation_revision,
                &note_id,
                Some(&job.id),
            )
            .await?;
        }
    }
    append_compile_log(
        vault,
        &job.id,
        &format!("compile succeeded files={}", committed.files.len()),
    )?;
    touch_index_and_journal(vault, &job.id)?;
    Ok(CompileOutcome {
        committed: committed.files.into_iter().map(|f| f.rel).collect(),
        nothing_to_persist: false,
    })
}

async fn freeze_or_load_manifest(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    vault: &Path,
) -> Result<CompileJobManifest, CompileError> {
    if let Some(raw) = job.input_manifest.as_deref().filter(|s| !s.is_empty()) {
        if let Ok(m) = serde_json::from_str::<CompileJobManifest>(raw) {
            if !m.inputs.is_empty() || raw.contains("cutoff_source_seq") {
                return Ok(m);
            }
        }
    }
    let vault_row = wiki_service::active_vault(conn)
        .await?
        .ok_or_else(|| CompileError::Validation("no active vault".into()))?;
    let cutoff = wiki_service::max_source_seq(conn, &vault_row.id)
        .await?
        .unwrap_or(0);
    freeze_manifest(conn, &vault_row.id, cutoff, vault).await
}

pub async fn freeze_manifest(
    conn: &DatabaseConnection,
    vault_id: &str,
    cutoff_source_seq: i64,
    vault: &Path,
) -> Result<CompileJobManifest, CompileError> {
    let sources = wiki_service::ready_sources_up_to_seq(conn, vault_id, cutoff_source_seq).await?;
    let mut inputs = Vec::new();
    for source in sources {
        let Some(raw_hash) = source.raw_hash.clone() else {
            continue;
        };
        if wiki_service::compile_input_consumed(
            conn,
            &source.id,
            &raw_hash,
            source.annotation_revision,
            COMPILE_CONTRACT_VERSION,
        )
        .await?
        {
            continue;
        }
        let raw_path = source.raw_path.as_deref().unwrap_or("");
        let abs = vault.join(raw_path);
        let text = fs::read_to_string(&abs).unwrap_or_default();
        let segs = split_segments(&body_of(&text));
        let segment_ids: Vec<String> = segs.iter().map(|(id, _)| id.clone()).collect();
        inputs.push(FrozenInput {
            source_id: source.id,
            raw_hash,
            annotation_revision: source.annotation_revision,
            segment_ids,
            compile_contract_version: COMPILE_CONTRACT_VERSION.into(),
            analysis_config_revision: ANALYSIS_CONFIG_REVISION.into(),
        });
    }
    Ok(CompileJobManifest {
        cutoff_source_seq,
        inputs,
    })
}

pub fn split_segments(text: &str) -> Vec<(String, String)> {
    let count = text.chars().count();
    if count <= SEGMENT_CHAR_BUDGET {
        return vec![("s1".into(), text.to_string())];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let heading = line.starts_with('#');
        let next_len = current.chars().count() + line.chars().count() + 1;
        if heading && !current.is_empty() && current.chars().count() >= SEGMENT_CHAR_BUDGET / 4 {
            chunks.push(std::mem::take(&mut current));
        } else if next_len > SEGMENT_CHAR_BUDGET && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, t)| (format!("s{}", i + 1), t))
        .collect()
}

async fn persist_segments(
    conn: &DatabaseConnection,
    source: &wiki_source::Model,
    raw_text: &str,
    expected_ids: &[String],
) -> Result<Vec<(String, String)>, CompileError> {
    let segs = split_segments(&body_of(raw_text));
    let segs = if expected_ids.is_empty() {
        segs
    } else {
        segs.into_iter()
            .filter(|(id, _)| expected_ids.contains(id))
            .collect()
    };
    for (id, text) in &segs {
        wiki_service::upsert_source_segment(
            conn,
            &source.id,
            id,
            &content_hash(text),
            source.annotation_revision,
            COMPILE_CONTRACT_VERSION,
            "candidates",
        )
        .await?;
    }
    Ok(segs)
}

fn validate_candidates(
    out: &Value,
    source_id: &str,
    segment_id: &str,
) -> Result<Vec<Value>, CompileError> {
    let Some(arr) = out.get("candidates").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    if arr.len() > MAX_CANDIDATES_PER_SEGMENT {
        return Err(CompileError::Validation(format!(
            "segment {segment_id} returned more than {MAX_CANDIDATES_PER_SEGMENT} candidates"
        )));
    }
    let mut kept = Vec::new();
    for c in arr {
        let loc = c.get("locator").ok_or_else(|| {
            CompileError::Validation("candidate missing locator".into())
        })?;
        let et = c
            .get("evidence_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CompileError::Validation("candidate missing evidence_type".into()))?;
        if !matches!(et, "reference" | "application" | "result" | "reflection") {
            return Err(CompileError::Validation(format!(
                "invalid evidence_type {et}"
            )));
        }
        let loc_source = loc.get("source_id").and_then(|v| v.as_str()).unwrap_or("");
        if !loc_source.is_empty() && loc_source != source_id {
            return Err(CompileError::Validation(
                "candidate locator source_id does not match the frozen input".into(),
            ));
        }
        let mut c = c.clone();
        if let Some(obj) = c.get_mut("locator").and_then(|v| v.as_object_mut()) {
            obj.entry("source_id")
                .or_insert_with(|| json!(source_id));
            obj.entry("segment_id")
                .or_insert_with(|| json!(segment_id));
        }
        kept.push(c);
    }
    Ok(kept)
}

fn proposals_from_merge(
    merge_out: &Value,
    vault: &Path,
) -> Result<Vec<StagedProposal>, CompileError> {
    let Some(arr) = merge_out.get("page_proposals").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for p in arr {
        let body = p
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if body.trim().is_empty() {
            continue;
        }
        let page_type = p
            .get("type")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| yaml_string(&body, "type"))
            .unwrap_or_default();
        let rel = p
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| p.get("rel").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_default();
        if rel.is_empty() || page_type.is_empty() {
            continue;
        }
        if !page_type_matches_rel(&page_type, &rel) {
            return Err(CompileError::Validation(format!(
                "proposal path {rel} does not match type {page_type}"
            )));
        }
        let dest = vault.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let before_hash = p
            .get("before_hash")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| commit::file_hash(&dest).unwrap_or_default());
        let op = p
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or(if dest.exists() { "update" } else { "create" })
            .to_string();
        out.push(StagedProposal {
            rel,
            page_type,
            before_hash,
            after: commit::ensure_content_markers(&body),
            op,
        });
    }
    Ok(out)
}

fn host_pages_from_candidates(
    vault: &Path,
    candidates: &[Value],
    matches: &[Value],
    index: &[NoteIndexEntry],
    inputs: &[FrozenInput],
) -> Result<Vec<StagedProposal>, CompileError> {
    let mut proposals = Vec::new();
    let today = Utc::now().date_naive().to_string();
    for c in candidates {
        let id = c
            .get("candidate_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let relation = matches
            .iter()
            .find(|m| m.get("candidate_id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|m| m.get("relation").and_then(|v| v.as_str()))
            .unwrap_or("new");
        // Never merge on title alone. "same" requires an existing note id.
        let existing_id = matches
            .iter()
            .find(|m| m.get("candidate_id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|m| m.get("existing_note_id").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty());
        if relation == "same" {
            if let Some(note_id) = existing_id {
                if let Some(entry) = index.iter().find(|e| e.note_id == note_id) {
                    let dest = vault.join(&entry.rel);
                    let current = fs::read_to_string(&dest).unwrap_or_default();
                    let after = append_evidence(&current, c, today.as_str())?;
                    proposals.push(StagedProposal {
                        rel: entry.rel.clone(),
                        page_type: entry.page_type.clone(),
                        before_hash: content_hash(&current),
                        after,
                        op: "update".into(),
                    });
                    continue;
                }
            }
        }
        let title = c
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("concept");
        let page_type = kind_to_type(kind);
        let note_id = Uuid::new_v4().to_string();
        let rel = path_for_type(page_type, title, &note_id);
        let source_id = c
            .pointer("/locator/source_id")
            .and_then(|v| v.as_str())
            .or_else(|| inputs.first().map(|i| i.source_id.as_str()))
            .unwrap_or("");
        let claim = c.get("claim").and_then(|v| v.as_str()).unwrap_or("");
        let evidence_type = c
            .get("evidence_type")
            .and_then(|v| v.as_str())
            .unwrap_or("reference");
        let pointer = c
            .pointer("/locator/pointer")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let body = render_new_page(
            page_type,
            title,
            &note_id,
            source_id,
            claim,
            evidence_type,
            pointer,
            &today,
        );
        proposals.push(StagedProposal {
            rel,
            page_type: page_type.into(),
            before_hash: String::new(),
            after: body,
            op: "create".into(),
        });
    }

    // Always emit a source reading page for each frozen input.
    for input in inputs {
        let rel = format!("sources/{}.md", input.source_id);
        let dest = vault.join(&rel);
        if dest.exists() {
            continue;
        }
        if proposals.iter().any(|p| p.rel == rel) {
            continue;
        }
        let body = render_source_page(&input.source_id, &today);
        proposals.push(StagedProposal {
            rel,
            page_type: "source".into(),
            before_hash: String::new(),
            after: body,
            op: "create".into(),
        });
    }
    Ok(proposals)
}

fn kind_to_type(kind: &str) -> &'static str {
    match kind {
        "capability" | "capability_evidence" => "capability",
        "decision" => "decision",
        "outcome" => "outcome",
        "method" => "method",
        "concept" => "concept",
        "work_context" | "work-record" => "work-record",
        "project" => "project",
        "source" => "source",
        _ => "concept",
    }
}

fn path_for_type(page_type: &str, title: &str, note_id: &str) -> String {
    let slug = slugify(title);
    let short = note_id.get(..8).unwrap_or(note_id);
    match page_type {
        "capability" => format!("capabilities/{slug}-{short}.md"),
        "work-record" => format!("work/records/{slug}-{short}.md"),
        "decision" => format!("work/decisions/{slug}-{short}.md"),
        "outcome" => format!("work/outcomes/{slug}-{short}.md"),
        "project" => format!("work/projects/{slug}-{short}.md"),
        "concept" => format!("knowledge/concepts/{slug}-{short}.md"),
        "method" => format!("knowledge/methods/{slug}-{short}.md"),
        "source" => format!("sources/{note_id}.md"),
        "daily" => format!("journal/{title}.md"),
        _ => format!("knowledge/concepts/{slug}-{short}.md"),
    }
}

fn slugify(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "note".into()
    } else {
        s.chars().take(48).collect()
    }
}

fn render_new_page(
    page_type: &str,
    title: &str,
    note_id: &str,
    source_id: &str,
    claim: &str,
    evidence_type: &str,
    pointer: &str,
    today: &str,
) -> String {
    let evidence_level = if evidence_type == "reference" || page_type == "capability" {
        "knowledge_only"
    } else {
        "knowledge_only"
    };
    let extra = if page_type == "capability" {
        format!("evidence_level: {evidence_level}\n")
    } else {
        String::new()
    };
    let source_link = format!("\"[[sources/{source_id}]]\"");
    let body = if page_type == "capability" {
        format!(
            "# {title}\n\n\
## 能力范围\n\
{claim}\n\n\
## 方法与检查点\n\
整理自来源，尚未形成个人实践步骤。\n\n\
## 实践证据\n\
- evidence_type: {evidence_type}\n\
- verification_status: source_reported\n\
- locator: {pointer}\n\
- 来源：[[sources/{source_id}]]\n\n\
参考资料单独只能支持 knowledge_only，不构成个人实践。\n\n\
## 当前边界\n\
尚无本人角色证据。\n\n\
## 下一次实践\n\
建议在一次相关工作中记录本人角色与可核对结果。\n"
        )
    } else {
        format!(
            "# {title}\n\n{claim}\n\n来源：[[sources/{source_id}]] {pointer}\n"
        )
    };
    format!(
        "---\n\
title: {title}\n\
type: {page_type}\n\
summary: \"{claim}\"\n\
tags:\n  - type/{page_type}\n\
date: {today}\n\
updated: {today}\n\
status: draft\n\
{extra}\
sources:\n  - {source_link}\n\
codeg_note_id: \"{note_id}\"\n\
---\n\n\
{CONTENT_START}\n\
{body}\
{CONTENT_END}\n"
    )
}

fn render_source_page(source_id: &str, today: &str) -> String {
    format!(
        "---\n\
title: Source {source_id}\n\
type: source\n\
tags:\n  - type/source\n\
date: {today}\n\
updated: {today}\n\
status: draft\n\
codeg_source_id: \"{source_id}\"\n\
codeg_note_id: \"{source_id}\"\n\
---\n\n\
{CONTENT_START}\n\
# Source {source_id}\n\n\
派生阅读页。原始证据见 `raw/`，compile 不会改写 raw。\n\
{CONTENT_END}\n"
    )
}

fn append_evidence(current: &str, candidate: &Value, _today: &str) -> Result<String, CompileError> {
    let claim = candidate.get("claim").and_then(|v| v.as_str()).unwrap_or("");
    let pointer = candidate
        .pointer("/locator/pointer")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_id = candidate
        .pointer("/locator/source_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let addition = format!("\n- {claim} （[[sources/{source_id}]] {pointer}）\n");
    if current.contains(CONTENT_END) {
        Ok(current.replace(
            CONTENT_END,
            &format!("{addition}{CONTENT_END}"),
        ))
    } else {
        Ok(format!("{current}\n{addition}"))
    }
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
        });
    }
    out
}

fn yaml_string(md: &str, key: &str) -> Option<String> {
    let (yaml, _) = commit::split_frontmatter(md)?;
    for line in yaml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("{key}:")) {
            let v = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn body_of(md: &str) -> String {
    commit::split_frontmatter(md)
        .map(|(_, b)| b)
        .unwrap_or_else(|| md.to_string())
}

fn redacted_excerpt(text: &str) -> String {
    const MAX: usize = SEGMENT_CHAR_BUDGET;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        text.chars().take(MAX).collect()
    }
}

async fn register_consumed(
    conn: &DatabaseConnection,
    job_id: &str,
    manifest: &CompileJobManifest,
) -> Result<(), CompileError> {
    for input in &manifest.inputs {
        wiki_service::insert_compile_input(
            conn,
            &input.source_id,
            &input.raw_hash,
            input.annotation_revision,
            &input.compile_contract_version,
            job_id,
        )
        .await?;
    }
    Ok(())
}

fn append_compile_log(vault: &Path, job_id: &str, line: &str) -> Result<(), CompileError> {
    let log_line = format!("{} {line} job={job_id}", Utc::now().to_rfc3339());
    raw::append_log_idempotent(&vault.join("log.md"), job_id, &log_line)
        .map_err(|e| CompileError::Failed(e.to_string()))
}

fn touch_index_and_journal(vault: &Path, job_id: &str) -> Result<(), CompileError> {
    let today = Utc::now().date_naive().to_string();
    let journal = vault.join("journal").join(format!("{today}.md"));
    if !journal.exists() {
        fs::create_dir_all(journal.parent().unwrap())
            .map_err(|e| CompileError::Failed(e.to_string()))?;
        let body = format!(
            "---\ntitle: \"{today}\"\ntype: daily\ntags:\n  - type/daily\ndate: {today}\n---\n\n{CONTENT_START}\n- wiki compile job `{job_id}`\n{CONTENT_END}\n"
        );
        fs::write(&journal, body).map_err(|e| CompileError::Failed(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::llm::MockWikiLlm;
    use sea_orm::{ActiveModelTrait, Set};

    #[test]
    fn small_raw_is_one_segment() {
        let segs = split_segments("hello world");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, "s1");
    }

    #[test]
    fn long_raw_splits_on_headings() {
        let mut text = String::new();
        for i in 0..20 {
            text.push_str(&format!("# Heading {i}\n"));
            text.push_str(&"x".repeat(500));
            text.push('\n');
        }
        let segs = split_segments(&text);
        assert!(segs.len() > 1);
    }

    #[tokio::test]
    async fn mock_llm_writes_capability_page_with_source_link() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        vault::initialize_vault(&vault).unwrap();
        vault::initialize_state_root(&state).unwrap();
        let db = fresh_in_memory_db().await;
        let now = Utc::now();
        let vault_row = wiki_vault::ActiveModel {
            id: Set("v1".into()),
            canonical_path: Set(vault.to_string_lossy().into_owned()),
            config_revision: Set(0),
            next_compile_at: Set(None),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let source_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let raw_rel = format!("raw/sessions/{source_id}.md");
        let raw_body = format!(
            "---\ntitle: spec\ntype: session-dump\n---\n\n# API spec\n\nRequire an idempotency key on retried POSTs.\n"
        );
        let hash = content_hash(&raw_body);
        fs::create_dir_all(vault.join("raw/sessions")).unwrap();
        fs::write(vault.join(&raw_rel), &raw_body).unwrap();
        wiki_source::ActiveModel {
            id: Set(source_id.into()),
            source_group_id: Set(source_id.into()),
            vault_id: Set(vault_row.id.clone()),
            source_kind: Set("document".into()),
            source_seq: Set(1),
            run_id: Set(None),
            original_hash: Set(None),
            raw_path: Set(Some(raw_rel)),
            raw_hash: Set(Some(hash.clone())),
            extractor_version: Set(None),
            coverage_status: Set(None),
            eligibility: Set("ready".into()),
            material_role: Set(Some("reference".into())),
            personal_role: Set(None),
            annotation_revision: Set(0),
            conversation_id: Set(None),
            folder_id: Set(None),
            root_folder_id: Set(None),
            agent_type: Set(None),
            model: Set(None),
            mode: Set(None),
            captured_at: Set(None),
            occurred_at: Set(None),
            truncated: Set(false),
            redacted: Set(false),
            request_id: Set(None),
            original_filename: Set(None),
            format: Set(None),
            source_title: Set(None),
            source_url: Set(None),
            author: Set(None),
            project_ids: Set(None),
            area_ids: Set(None),
            warnings: Set(None),
            page_count: Set(None),
            previous_source_id: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let job = wiki_job::ActiveModel {
            id: Set("job-compile-1".into()),
            vault_id: Set(vault_row.id),
            source_id: Set(Some(source_id.into())),
            kind: Set("compile".into()),
            status: Set("running".into()),
            dedupe_key: Set(Some("req-1".into())),
            input_manifest: Set(None),
            config_version: Set(Some(COMPILE_CONTRACT_VERSION.into())),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            started_at: Set(Some(now)),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();

        let llm = MockWikiLlm::default();
        let out = run_compile_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap();
        assert!(!out.nothing_to_persist);
        let cap = fs::read_dir(vault.join("capabilities"))
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| {
                e.path().extension().and_then(|x| x.to_str()) == Some("md")
                    && e.file_name() != "index.md"
            })
            .expect("capability page");
        let text = fs::read_to_string(cap.path()).unwrap();
        assert!(text.contains("type: capability"));
        assert!(text.contains(&format!("[[sources/{source_id}]]")));
        assert!(vault.join(format!("sources/{source_id}.md")).is_file());

        // Retry: unique compile_input makes a second freeze empty; committing
        // the same after_hash is still success.
        let job2 = wiki_job::ActiveModel {
            id: Set("job-compile-2".into()),
            vault_id: Set(job.vault_id.clone()),
            source_id: Set(Some(source_id.into())),
            kind: Set("compile".into()),
            status: Set("running".into()),
            dedupe_key: Set(Some("req-2".into())),
            input_manifest: Set(None),
            config_version: Set(Some(COMPILE_CONTRACT_VERSION.into())),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            started_at: Set(Some(now)),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let out2 = run_compile_job(&db.conn, &job2, &llm, &vault, &state)
            .await
            .unwrap();
        assert!(out2.nothing_to_persist || out2.committed.is_empty());
    }
}

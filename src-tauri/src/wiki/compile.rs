//! Freeze synthesize input from memory notes, stage page proposals, commit.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::entities::{wiki_job, wiki_source};
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::commit::{self, StagedProposal};
use crate::wiki::fs_policy::{page_type_matches_rel, WikiFsPolicy};
use crate::wiki::llm::{
    WikiLlm, WikiLlmError, COMPILE_CONTRACT_VERSION, SYNTHESIZE_CONTRACT_VERSION,
};
use crate::wiki::raw::{self, content_hash};
use crate::wiki::settings;
use crate::wiki::vault::{self, CONTENT_END, CONTENT_START};

pub const SEGMENT_CHAR_BUDGET: usize = 32_000;
#[allow(dead_code)]
pub const MAX_CANDIDATES_PER_SEGMENT: usize = 80;
pub const ANALYSIS_CONFIG_REVISION: &str = "2";
/// Prompt + verify only. The model decides how much to write under this.
pub const LEAF_BODY_SOFT_CHARS: usize = 12_000;
/// Reject only a runaway page body. Never silently truncate model prose.
pub const LEAF_BODY_HARD_CHARS: usize = 100_000;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrozenInput {
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub raw_hash: String,
    #[serde(default)]
    pub annotation_revision: i32,
    #[serde(default)]
    pub segment_ids: Vec<String>,
    #[serde(default)]
    pub compile_contract_version: String,
    #[serde(default)]
    pub analysis_config_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryNoteRef {
    pub rel: String,
    pub content_hash: String,
    #[serde(default)]
    pub page_type: String,
    #[serde(default)]
    pub project_binding_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompileJobManifest {
    #[serde(default)]
    pub cutoff_source_seq: i64,
    #[serde(default)]
    pub inputs: Vec<FrozenInput>,
    #[serde(default)]
    pub memory_notes: Vec<MemoryNoteRef>,
    #[serde(default)]
    pub extra_read_roots: Vec<String>,
    #[serde(default)]
    pub compile_contract_version: String,
}

#[derive(Debug, Clone)]
pub struct NoteIndexEntry {
    pub note_id: String,
    pub rel: String,
    pub page_type: String,
    pub title: String,
    pub hash: String,
    pub evidence_level: Option<String>,
    pub projects: Vec<String>,
    pub summary: Option<String>,
    pub date: Option<String>,
}

#[derive(Debug, Clone)]
struct CompiledPage {
    rel: String,
    page_type: String,
    title: String,
    source_id: String,
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

    if manifest.memory_notes.is_empty() {
        append_compile_log(vault, &job.id, "wiki_synthesize nothing_to_persist")?;
        return Ok(CompileOutcome {
            committed: Vec::new(),
            nothing_to_persist: true,
        });
    }

    let timezone = settings::load_settings(conn)
        .await
        .ok()
        .map(|s| s.timezone)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "UTC".into());
    let today = local_date(Utc::now(), &timezone).to_string();
    let index = scan_note_index(vault);
    let mut allowed_reads: Vec<PathBuf> = Vec::new();
    allowed_reads.push(vault.join("AGENTS.md"));
    for entry in &index {
        allowed_reads.push(vault.join(&entry.rel));
    }
    for note in &manifest.memory_notes {
        allowed_reads.push(vault.join(&note.rel));
    }

    let payload = json!({
        "schema": SYNTHESIZE_CONTRACT_VERSION,
        "memory_notes": manifest.memory_notes.iter().map(|n| json!({
            "rel": n.rel,
            "content_hash": n.content_hash,
        })).collect::<Vec<_>>(),
        "extra_read_roots": manifest.extra_read_roots,
        "index": index.iter().map(|e| json!({
            "codeg_note_id": e.note_id,
            "path": e.rel,
            "type": e.page_type,
            "title": e.title,
        })).collect::<Vec<_>>(),
        "vault_abs": vault.to_string_lossy(),
        "staging_abs": staging.to_string_lossy(),
        "max_turns": crate::agent::model::WIKI_COMPILE_MAX_TURNS,
        "instruction": "Read listed memory notes with read_file. Do not return a candidates array. Write page_proposals as full markdown pages. Not reading project folders is success.",
    });
    if payload.get("candidates").is_some() {
        return Err(CompileError::Validation(
            "host synthesize payload must not include candidates".into(),
        ));
    }

    let out = llm.complete_json("synthesize", payload).await?;
    let nothing = out
        .get("nothing_to_persist")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut proposals = proposals_from_merge(&out, vault)?;
    proposals.retain(|p| !matches!(p.page_type.as_str(), "turn-summary" | "session-summary"));

    let mut skipped_hard = 0usize;
    let mut kept = Vec::new();
    for p in proposals {
        match check_leaf_body(&p.page_type, &p.after) {
            Ok(w) => {
                if let Some(w) = w {
                    tracing::info!(rel = %p.rel, warning = %w, "[wiki] leaf body warning");
                    let _ = append_compile_log(vault, &job.id, &format!("{} {w}", p.rel));
                }
                kept.push(p);
            }
            Err(e) => {
                skipped_hard += 1;
                tracing::info!(rel = %p.rel, error = %e, "[wiki] reject over-long leaf page");
                let _ = append_compile_log(vault, &job.id, &format!("{} rejected: {e}", p.rel));
            }
        }
    }
    let mut proposals = kept;
    let sources_by_id: HashMap<String, wiki_source::Model> = HashMap::new();
    sanitize_capability_evidence(&mut proposals, &sources_by_id);

    if nothing || proposals.is_empty() {
        if skipped_hard > 0 && !nothing {
            return Err(CompileError::Validation(
                "all synthesize page proposals were rejected".into(),
            ));
        }
        register_consumed(conn, &job.id, &manifest).await?;
        append_compile_log(vault, &job.id, "wiki_synthesize nothing_to_persist")?;
        return Ok(CompileOutcome {
            committed: Vec::new(),
            nothing_to_persist: true,
        });
    }

    let policy = WikiFsPolicy::for_compile_job(vault, state_root, &staging, &allowed_reads);
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

    let compiled_pages = compiled_pages_from(&proposals);
    let committed = commit::commit_proposals(vault, state_root, &job.id, &proposals)?;
    register_consumed(conn, &job.id, &manifest).await?;
    let _ = today;
    let _ = sources_by_id;
    touch_index_and_journal(vault, &job.id, &timezone, &HashMap::new(), &compiled_pages)?;
    append_compile_log(
        vault,
        &job.id,
        &format!(
            "wiki_synthesize succeeded files={} skipped_hard={skipped_hard}",
            committed.files.len()
        ),
    )?;
    Ok(CompileOutcome {
        nothing_to_persist: committed.files.is_empty(),
        committed: committed.files.into_iter().map(|f| f.rel).collect(),
    })
}

#[allow(dead_code)]
async fn compile_one_input(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
    staging: &Path,
    input: &FrozenInput,
    timezone: &str,
    today: &str,
) -> Result<CompileOutcome, CompileError> {
    let index = scan_note_index(vault);
    let mut allowed_reads: Vec<PathBuf> = Vec::new();
    allowed_reads.push(vault.join("AGENTS.md"));
    for entry in &index {
        allowed_reads.push(vault.join(&entry.rel));
    }

    let source = wiki_service::get_source_model(conn, &input.source_id).await?;
    let raw_path = source
        .raw_path
        .as_deref()
        .ok_or_else(|| CompileError::Validation(format!("source {} has no raw", source.id)))?;
    let abs = vault.join(raw_path);
    allowed_reads.push(abs.clone());
    let raw_text =
        fs::read_to_string(&abs).map_err(|e| CompileError::Failed(format!("read raw: {e}")))?;
    if source.raw_hash.as_deref() != Some(input.raw_hash.as_str()) {
        return Err(CompileError::Conflict(format!(
            "source {} raw hash changed",
            source.id
        )));
    }
    let rows = wiki_service::list_contributions_for_source(conn, &source.id).await?;
    let mut prior_contrib: HashMap<String, Vec<String>> = HashMap::new();
    prior_contrib.insert(
        source.id.clone(),
        rows.into_iter().map(|r| r.note_id).collect(),
    );
    let segments = persist_segments(conn, &source, &raw_text, &input.segment_ids).await?;
    let segment_ids: Vec<String> = segments.iter().map(|(id, _)| id.clone()).collect();
    let payload = candidates_llm_input(
        &source.id,
        raw_path,
        &input.raw_hash,
        input.annotation_revision,
        &segment_ids,
        &vault.to_string_lossy(),
        &staging.to_string_lossy(),
    );
    let out = llm.complete_json("candidates", payload).await?;
    let fallback = segment_ids.first().map(String::as_str).unwrap_or("s1");
    let (all_candidates, warnings) = validate_candidates(&out, &source.id, fallback, &segment_ids)?;
    for w in &warnings {
        tracing::info!(source_id = %source.id, warning = %w, "[wiki] compile candidate warning");
    }
    let mut sources_by_id: HashMap<String, wiki_source::Model> = HashMap::new();
    sources_by_id.insert(source.id.clone(), source);

    let match_in = json!({
        "candidates": all_candidates,
        "index": index.iter().map(|e| json!({
            "codeg_note_id": e.note_id,
            "path": e.rel,
            "type": e.page_type,
            "title": e.title,
        })).collect::<Vec<_>>(),
        "vault_abs": vault.to_string_lossy(),
        "staging_abs": staging.to_string_lossy(),
        "instruction": "Read existing notes from the index paths with read_file when you need the body. related≠same.",
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
        "vault_abs": vault.to_string_lossy(),
        "staging_abs": staging.to_string_lossy(),
    });
    let merge_out = llm.complete_json("merge", merge_in).await?;

    let policy = WikiFsPolicy::for_compile_job(vault, state_root, staging, &allowed_reads);
    let one_input = vec![input.clone()];
    let mut proposals = proposals_from_merge(&merge_out, vault)?;
    if proposals.is_empty() {
        proposals = host_pages_from_candidates(
            vault,
            &all_candidates,
            &matches,
            &index,
            &one_input,
            &sources_by_id,
            today,
        )?;
    }
    let contributed: Vec<(String, String, String)> = proposals
        .iter()
        .filter(|p| !matches!(p.page_type.as_str(), "source" | "index" | "daily"))
        .map(|p| {
            (
                yaml_string(&p.after, "codeg_note_id").unwrap_or_default(),
                p.rel.clone(),
                yaml_string(&p.after, "title").unwrap_or_default(),
            )
        })
        .collect();
    ensure_source_pages(
        &mut proposals,
        vault,
        &one_input,
        &sources_by_id,
        &contributed,
        &prior_contrib,
        &index,
        today,
    )?;
    sanitize_capability_evidence(&mut proposals, &sources_by_id);
    for p in &proposals {
        if let Some(w) = check_leaf_body(&p.page_type, &p.after)? {
            tracing::info!(rel = %p.rel, warning = %w, "[wiki] leaf body warning");
            let _ = append_compile_log(vault, &job.id, &format!("{} {w}", p.rel));
        }
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
            json!({ "processed_inputs": one_input, "pages": proposals.iter().map(|p| &p.rel).collect::<Vec<_>>() }),
        )
        .await;

    let one_manifest = CompileJobManifest {
        cutoff_source_seq: 0,
        inputs: one_input.clone(),
        ..Default::default()
    };
    if proposals.is_empty() {
        register_consumed(conn, &job.id, &one_manifest).await?;
        return Ok(CompileOutcome {
            committed: Vec::new(),
            nothing_to_persist: true,
        });
    }

    let compiled_pages = compiled_pages_from(&proposals);
    let committed = commit::commit_proposals(vault, state_root, &job.id, &proposals)?;
    register_consumed(conn, &job.id, &one_manifest).await?;
    for file in &committed.files {
        let Some(p) = proposals.iter().find(|p| p.rel == file.rel) else {
            continue;
        };
        if matches!(p.page_type.as_str(), "source" | "index" | "daily") {
            continue;
        }
        let note_id = yaml_string(&p.after, "codeg_note_id").unwrap_or_default();
        if note_id.is_empty() {
            continue;
        }
        if page_cites_source(&p.after, &input.source_id) {
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
    touch_index_and_journal(vault, &job.id, timezone, &sources_by_id, &compiled_pages)?;
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
            if !m.memory_notes.is_empty()
                || raw.contains("memory_notes")
                || raw.contains(SYNTHESIZE_CONTRACT_VERSION)
            {
                return Ok(m);
            }
        }
    }
    let vault_row = wiki_service::active_vault(conn)
        .await?
        .ok_or_else(|| CompileError::Validation("no active vault".into()))?;
    freeze_manifest(conn, &vault_row.id, 0, vault).await
}

pub async fn freeze_manifest(
    conn: &DatabaseConnection,
    vault_id: &str,
    _cutoff_source_seq: i64,
    vault: &Path,
) -> Result<CompileJobManifest, CompileError> {
    let notes = scan_memory_notes(vault);
    let mut memory_notes = Vec::new();
    let mut binding_ids: Vec<String> = Vec::new();
    for note in notes {
        if wiki_service::compile_input_consumed(
            conn,
            &note.rel,
            &note.content_hash,
            0,
            SYNTHESIZE_CONTRACT_VERSION,
        )
        .await?
        {
            continue;
        }
        binding_ids.extend(note.project_binding_ids.iter().cloned());
        memory_notes.push(note);
    }
    binding_ids.sort();
    binding_ids.dedup();
    let extra_read_roots = extra_project_roots(conn, vault_id, &binding_ids, &memory_notes).await?;
    Ok(CompileJobManifest {
        cutoff_source_seq: 0,
        inputs: Vec::new(),
        memory_notes,
        extra_read_roots,
        compile_contract_version: SYNTHESIZE_CONTRACT_VERSION.into(),
    })
}

pub fn scan_memory_notes(vault: &Path) -> Vec<MemoryNoteRef> {
    let mut out = Vec::new();
    for dir_rel in ["work/turns", "work/sessions"] {
        let dir = vault.join(dir_rel);
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(vault)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let page_type = yaml_string(&text, "type").unwrap_or_default();
            let mut project_binding_ids = yaml_list(&text, "codeg_project_binding_id");
            if project_binding_ids.is_empty() {
                if let Some(id) = yaml_string(&text, "codeg_project_binding_id") {
                    project_binding_ids.push(id);
                }
            }
            out.push(MemoryNoteRef {
                rel,
                content_hash: content_hash(&text),
                page_type,
                project_binding_ids,
            });
        }
    }
    out
}

async fn extra_project_roots(
    conn: &DatabaseConnection,
    vault_id: &str,
    yaml_bindings: &[String],
    notes: &[MemoryNoteRef],
) -> Result<Vec<String>, CompileError> {
    let mut ids: Vec<String> = yaml_bindings.to_vec();
    for note in notes {
        ids.extend(note.project_binding_ids.iter().cloned());
        if !note.project_binding_ids.is_empty() {
            continue;
        }
        let source_id = note
            .rel
            .rsplit('/')
            .next()
            .and_then(|n| n.strip_suffix(".md"))
            .map(str::to_string);
        if let Some(sid) = source_id {
            if let Ok(src) = wiki_service::get_source_model(conn, &sid).await {
                if let Some(list) = src.project_ids.as_deref() {
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(list) {
                        ids.extend(parsed);
                    }
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    let bindings = wiki_service::list_project_bindings(conn, Some(vault_id)).await?;
    let mut roots = Vec::new();
    for b in bindings {
        if !ids.is_empty() && !ids.iter().any(|id| id == &b.id) {
            continue;
        }
        if ids.is_empty() {
            continue;
        }
        let Some(path) = b.root_folder_path.clone() else {
            continue;
        };
        let p = PathBuf::from(&path);
        if p.components().any(|c| {
            matches!(
                c.as_os_str().to_string_lossy().as_ref(),
                ".git" | ".obsidian" | "originals"
            )
        }) {
            continue;
        }
        if p.is_dir() {
            roots.push(path);
        }
    }
    Ok(roots)
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

pub(crate) fn candidates_llm_input(
    source_id: &str,
    raw_path: &str,
    raw_hash: &str,
    annotation_revision: i32,
    segment_ids: &[String],
    vault_abs: &str,
    staging_abs: &str,
) -> Value {
    json!({
        "source_id": source_id,
        "raw_path": raw_path,
        "raw_hash": raw_hash,
        "annotation_revision": annotation_revision,
        "segment_ids": segment_ids,
        "compile_contract_version": COMPILE_CONTRACT_VERSION,
        "vault_abs": vault_abs,
        "staging_abs": staging_abs,
        "instruction": "Read the converted markdown at raw_path with read_file/grep. Do not wait for an embedded excerpt. Cite locators using the host segment_ids.",
    })
}

const LEAF_PAGE_TYPES: &[&str] = &[
    "project",
    "area",
    "work-record",
    "decision",
    "outcome",
    "capability",
    "method",
    "concept",
    "entity",
    "turn-summary",
    "session-summary",
];

pub(crate) fn check_leaf_body(
    page_type: &str,
    after: &str,
) -> Result<Option<String>, CompileError> {
    if !LEAF_PAGE_TYPES.contains(&page_type) {
        return Ok(None);
    }
    let n = body_of(after).chars().count();
    if n > LEAF_BODY_HARD_CHARS {
        return Err(CompileError::Validation(format!(
            "{page_type} body is {n} characters; over the runaway cap of {LEAF_BODY_HARD_CHARS}"
        )));
    }
    if n > LEAF_BODY_SOFT_CHARS {
        return Ok(Some(format!(
            "{page_type} body is {n} characters (soft guidance {LEAF_BODY_SOFT_CHARS}); kept as the model wrote it"
        )));
    }
    Ok(None)
}

fn validate_candidates(
    out: &Value,
    source_id: &str,
    segment_id: &str,
    allowed_segments: &[String],
) -> Result<(Vec<Value>, Vec<String>), CompileError> {
    let Some(arr) = out.get("candidates").and_then(|v| v.as_array()) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut warnings = Vec::new();
    let slice = if arr.len() > MAX_CANDIDATES_PER_SEGMENT {
        warnings.push(format!(
            "segment {segment_id} returned {} candidates; keeping first {MAX_CANDIDATES_PER_SEGMENT}",
            arr.len()
        ));
        &arr[..MAX_CANDIDATES_PER_SEGMENT]
    } else {
        arr.as_slice()
    };
    let mut kept = Vec::new();
    for c in slice {
        let loc = c
            .get("locator")
            .ok_or_else(|| CompileError::Validation("candidate missing locator".into()))?;
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
            obj.entry("source_id").or_insert_with(|| json!(source_id));
            let cited = obj
                .get("segment_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let seg = cited
                .filter(|id| allowed_segments.iter().any(|a| a == id))
                .unwrap_or_else(|| segment_id.to_string());
            obj.insert("segment_id".into(), json!(seg));
        }
        kept.push(c);
    }
    Ok((kept, warnings))
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
    sources: &HashMap<String, wiki_source::Model>,
    today: &str,
) -> Result<Vec<StagedProposal>, CompileError> {
    let mut proposals = Vec::new();
    for c in candidates {
        let kind = cand_str(c, "kind");
        let page_type = kind_to_type(if kind.is_empty() { "concept" } else { kind });
        let source_id = c
            .pointer("/locator/source_id")
            .and_then(|v| v.as_str())
            .or_else(|| inputs.first().map(|i| i.source_id.as_str()))
            .unwrap_or("");
        let source = sources.get(source_id);
        let action = resolve_match(c, matches, index, page_type);
        match action {
            MatchAction::Merge(entry) => {
                let dest = vault.join(&entry.rel);
                let current = fs::read_to_string(&dest).unwrap_or_default();
                if marker_conflict(&current) {
                    return Err(CompileError::Conflict(format!(
                        "{} has missing or duplicate codeg-content markers",
                        entry.rel
                    )));
                }
                let after = append_evidence(&current, c, today, source)?;
                proposals.push(StagedProposal {
                    rel: entry.rel.clone(),
                    page_type: entry.page_type.clone(),
                    before_hash: content_hash(&current),
                    after,
                    op: "update".into(),
                });
            }
            MatchAction::New { related } => {
                if page_type == "area" {
                    // Model may only suggest existing areas; host does not mint new ones.
                    continue;
                }
                if matches!(page_type, "source" | "index" | "daily") {
                    continue;
                }
                let title = {
                    let t = cand_str(c, "title");
                    if t.is_empty() {
                        "Untitled"
                    } else {
                        t
                    }
                };
                let note_id = Uuid::new_v4().to_string();
                let rel = path_for_type(page_type, title, &note_id);
                let body = render_new_page(RenderPage {
                    page_type,
                    title,
                    note_id: &note_id,
                    source_id,
                    candidate: c,
                    source,
                    related: related.as_ref(),
                    today,
                });
                proposals.push(StagedProposal {
                    rel,
                    page_type: page_type.into(),
                    before_hash: String::new(),
                    after: body,
                    op: "create".into(),
                });
            }
        }
    }
    Ok(proposals)
}

enum MatchAction {
    Merge(NoteIndexEntry),
    New { related: Option<NoteIndexEntry> },
}

fn resolve_match(
    candidate: &Value,
    matches: &[Value],
    index: &[NoteIndexEntry],
    page_type: &str,
) -> MatchAction {
    let id = cand_str(candidate, "candidate_id");
    let found = matches
        .iter()
        .find(|m| m.get("candidate_id").and_then(|v| v.as_str()) == Some(id));
    let relation = found
        .and_then(|m| m.get("relation").and_then(|v| v.as_str()))
        .unwrap_or("new");
    let existing_id = found
        .and_then(|m| m.get("existing_note_id").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty());
    let entry = existing_id.and_then(|nid| index.iter().find(|e| e.note_id == nid).cloned());

    // Host filter: same requires existing codeg_note_id AND matching type AND
    // overlapping work scope/claim. Title-only is never same.
    if matches!(relation, "same" | "contradictory") {
        if let Some(entry) = entry {
            if entry.page_type == page_type && overlapping_scope_or_claim(candidate, &entry) {
                return MatchAction::Merge(entry);
            }
            return MatchAction::New {
                related: Some(entry),
            };
        }
        return MatchAction::New { related: None };
    }
    if relation == "related" {
        return MatchAction::New { related: entry };
    }
    MatchAction::New { related: None }
}

fn overlapping_scope_or_claim(candidate: &Value, entry: &NoteIndexEntry) -> bool {
    let cand_projects = candidate_projects(candidate);
    if !cand_projects.is_empty() && !entry.projects.is_empty() {
        return cand_projects.iter().any(|p| {
            entry
                .projects
                .iter()
                .any(|e| e == p || e.contains(p) || p.contains(e))
        });
    }
    // note_id + type already matched; unknown/omitted projects do not conflict.
    // Title-only matches without a note id are rejected in `resolve_match`.
    true
}

fn candidate_projects(candidate: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = candidate.get("project_id").and_then(|v| v.as_str()) {
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
    if let Some(p) = candidate.get("work_scope").and_then(|v| v.as_str()) {
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
    match candidate.get("projects") {
        Some(Value::Array(arr)) => {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
            }
        }
        Some(Value::String(s)) if !s.is_empty() => out.push(s.clone()),
        _ => {}
    }
    out
}

fn kind_to_type(kind: &str) -> &'static str {
    match kind {
        "capability" | "capability_evidence" => "capability",
        "decision" => "decision",
        "outcome" => "outcome",
        "method" => "method",
        "concept" => "concept",
        "entity" => "entity",
        "work_context" | "work-record" | "work_record" => "work-record",
        "project" => "project",
        "area" | "responsibility" => "area",
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
        "area" => format!("work/areas/{slug}-{short}.md"),
        "concept" => format!("knowledge/concepts/{slug}-{short}.md"),
        "method" => format!("knowledge/methods/{slug}-{short}.md"),
        "entity" => format!("knowledge/entities/{slug}-{short}.md"),
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

struct RenderPage<'a> {
    page_type: &'a str,
    title: &'a str,
    note_id: &'a str,
    source_id: &'a str,
    candidate: &'a Value,
    source: Option<&'a wiki_source::Model>,
    related: Option<&'a NoteIndexEntry>,
    today: &'a str,
}

fn render_new_page(args: RenderPage<'_>) -> String {
    let claim = cand_str(args.candidate, "claim");
    let evidence_type = {
        let et = cand_str(args.candidate, "evidence_type");
        if et.is_empty() {
            "reference"
        } else {
            et
        }
    };
    let pointer = args
        .candidate
        .pointer("/locator/pointer")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let actor = {
        let a = cand_str(args.candidate, "actor");
        if a.is_empty() {
            "unspecified"
        } else {
            a
        }
    };
    let personal_role = candidate_personal_role(args.candidate, args.source);
    // Source annotations are the only trusted material/personal classification.
    let material_role = args.source.and_then(|s| s.material_role.as_deref());
    let evidence_level =
        compute_evidence_level(evidence_type, actor, personal_role, pointer, material_role);
    let verification = verification_status(args.candidate);
    let decision_state = if args.page_type == "decision" {
        Some(candidate_decision_state(args.candidate))
    } else {
        None
    };
    let related_link = args.related.map(|e| wikilink(&e.rel, &e.title));
    let body = render_page_body(
        args.page_type,
        args.title,
        claim,
        evidence_type,
        pointer,
        args.source_id,
        actor,
        personal_role,
        verification,
        evidence_level,
        related_link.as_deref(),
        args.candidate,
    );
    let summary = if claim.is_empty() {
        args.title.to_string()
    } else {
        claim.chars().take(120).collect()
    };
    let mut fm = String::from("---\n");
    push_yaml_scalar(&mut fm, "title", args.title);
    push_yaml_plain(&mut fm, "type", args.page_type);
    push_yaml_quoted(&mut fm, "summary", &summary);
    push_yaml_list(&mut fm, "tags", &[format!("type/{}", args.page_type)]);
    push_yaml_plain(&mut fm, "date", args.today);
    push_yaml_plain(&mut fm, "updated", args.today);
    push_yaml_plain(&mut fm, "status", "draft");
    if args.page_type == "capability" {
        push_yaml_plain(&mut fm, "evidence_level", evidence_level);
    }
    if let Some(ds) = decision_state {
        push_yaml_plain(&mut fm, "decision_state", ds);
    }
    if !args.source_id.is_empty() {
        push_yaml_list(
            &mut fm,
            "sources",
            &[format!("[[sources/{}]]", args.source_id)],
        );
    }
    push_yaml_quoted(&mut fm, "codeg_note_id", args.note_id);
    fm.push_str("---\n\n");
    format!("{fm}{CONTENT_START}\n{body}{CONTENT_END}\n")
}

fn render_page_body(
    page_type: &str,
    title: &str,
    claim: &str,
    evidence_type: &str,
    pointer: &str,
    source_id: &str,
    actor: &str,
    personal_role: Option<&str>,
    verification: &str,
    evidence_level: &str,
    related: Option<&str>,
    candidate: &Value,
) -> String {
    let source_line = if source_id.is_empty() {
        String::new()
    } else if pointer.is_empty() {
        format!("来源：[[sources/{source_id}]]")
    } else {
        format!("来源：[[sources/{source_id}]] {pointer}")
    };
    let related_block = related
        .map(|l| format!("\n## 相关\n- {l}\n"))
        .unwrap_or_default();
    let text = |key: &str| {
        let v = cand_str(candidate, key);
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    };
    let or_claim = |key: &str, fallback: &str| {
        text(key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback.to_string())
    };
    match page_type {
        "project" => {
            let role = personal_role.filter(|s| !s.is_empty()).unwrap_or("未说明");
            format!(
                "# {title}\n\n\
## 背景与目标\n\
{}\n\n\
## 本人角色\n\
{role}\n\n\
## 当前进展\n\
{}\n\n\
## 关键决策\n\
{}\n\n\
## 成果\n\
{}\n\n\
## 关联能力\n\
{}\n\
{related_block}",
                or_claim(
                    "background",
                    if claim.is_empty() { "未说明" } else { claim }
                ),
                or_claim("progress", "未说明"),
                or_claim("decisions", "未说明"),
                or_claim("outcomes", "未说明"),
                or_claim("capabilities", "未说明"),
            )
        }
        "area" => format!(
            "# {title}\n\n\
## 长期职责\n\
{}\n\n\
## 工作标准\n\
{}\n\n\
## 关联项目\n\
{}\n\n\
## 可复用方法\n\
{}\n\n\
## 持续问题\n\
{}\n\
{related_block}",
            or_claim(
                "responsibility",
                if claim.is_empty() { "未说明" } else { claim }
            ),
            or_claim("standards", "未说明"),
            or_claim("projects", "未说明"),
            or_claim("methods", "未说明"),
            or_claim("open_problems", "未说明"),
        ),
        "work-record" => {
            let contribution = personal_role
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    if actor == "user" {
                        or_claim("personal_contribution", claim)
                    } else {
                        format!("执行者是 {actor}，不能记为用户独立完成。")
                    }
                });
            format!(
                "# {title}\n\n\
## 问题\n\
{}\n\n\
## 上下文\n\
{}\n\n\
## 采取的行动\n\
{}\n\n\
## 观察到的结果\n\
{}\n\n\
## 个人贡献\n\
{contribution}\n\n\
## 来源\n\
{source_line}\n\
{related_block}",
                or_claim("problem", if claim.is_empty() { "未说明" } else { claim }),
                or_claim("context", "未说明"),
                or_claim("actions", "未说明"),
                or_claim("results", "未说明"),
            )
        }
        "decision" => {
            let state = candidate_decision_state(candidate);
            format!(
                "# {title}\n\n\
## 约束\n\
{}\n\n\
## 备选方案\n\
{}\n\n\
## 选择\n\
{}\n\n\
## 理由\n\
{}\n\n\
decision_state: {state}（与笔记 status 分开；仅提议时不得写成已采纳）。\n\n\
{source_line}\n\
{related_block}",
                or_claim("constraints", "未说明"),
                or_claim("options", "未说明"),
                or_claim("choice", if claim.is_empty() { "未说明" } else { claim }),
                or_claim("rationale", "未说明"),
            )
        }
        "outcome" => {
            let pending = if evidence_type != "result" {
                "\n状态：待验证。只有计划或“已完成”自述，不编造收益数字。\n"
            } else {
                "\n不编造收益数字；仅记录来源中出现的结果。\n"
            };
            format!(
                "# {title}\n\n\
## 交付物\n\
{}\n\n\
## 本人贡献\n\
{}\n\n\
## 结果证据\n\
{}\n\n\
## 可复用部分\n\
{}\n\
{pending}\n\
{source_line}\n\
{related_block}",
                or_claim(
                    "deliverable",
                    if claim.is_empty() { "未说明" } else { claim }
                ),
                personal_role.filter(|s| !s.is_empty()).unwrap_or("未说明"),
                or_claim("evidence", "未说明"),
                or_claim("reusable", "未说明"),
            )
        }
        "capability" => render_capability_body(
            title,
            claim,
            evidence_type,
            pointer,
            source_id,
            actor,
            personal_role,
            verification,
            evidence_level,
            related,
        ),
        "method" => format!(
            "# {title}\n\n\
## 输入\n\
{}\n\n\
## 步骤\n\
{}\n\n\
## 输出\n\
{}\n\n\
## 适用边界\n\
{}\n\n\
{source_line}\n\
{related_block}",
            or_claim("inputs", "未说明"),
            or_claim("steps", if claim.is_empty() { "未说明" } else { claim }),
            or_claim("outputs", "未说明"),
            or_claim("bounds", "未说明"),
        ),
        "entity" => format!(
            "# {title}\n\n\
## 实体\n\
{}\n\n\
{source_line}\n\
{related_block}",
            if claim.is_empty() { "未说明" } else { claim },
        ),
        _ => format!(
            "# {title}\n\n\
## 概念\n\
{}\n\n\
## 规则与约束\n\
{}\n\n\
{source_line}\n\
{related_block}",
            if claim.is_empty() { "未说明" } else { claim },
            or_claim("constraints", "未说明"),
        ),
    }
}

fn render_capability_body(
    title: &str,
    claim: &str,
    evidence_type: &str,
    pointer: &str,
    source_id: &str,
    actor: &str,
    personal_role: Option<&str>,
    verification: &str,
    evidence_level: &str,
    related: Option<&str>,
) -> String {
    let related_block = related
        .map(|l| format!("\n## 相关\n- {l}\n"))
        .unwrap_or_default();
    let practice = if evidence_level == "knowledge_only" {
        format!(
            "暂无个人实践。导入或参考资料只构成 knowledge_only，不能证明本人已实践。\n\n\
参考资料：\n\
- case: {title}\n\
- actor: {actor}\n\
- personal_role: {}\n\
- evidence_type: {evidence_type}\n\
- verification_status: {verification}\n\
- source: [[sources/{source_id}]]\n\
- locator: {pointer}\n",
            personal_role.unwrap_or("未说明")
        )
    } else {
        format!(
            "- case: {title}\n\
- actor: {actor}\n\
- personal_role: {}\n\
- evidence_type: {evidence_type}\n\
- verification_status: {verification}\n\
- source: [[sources/{source_id}]]\n\
- locator: {pointer}\n\n\
自动生成时仍按来源记录（{verification}），不能改写成产品亲自验证。\n",
            personal_role.unwrap_or("未说明")
        )
    };
    let scope = if claim.is_empty() {
        "未说明。本文尚无充分来源描述适用条件。".to_string()
    } else {
        claim.to_string()
    };
    format!(
        "# {title}\n\n\
## 能力范围\n\
{scope}\n\n\
## 方法与检查点\n\
整理自来源，尚未形成稳定的个人实践步骤。\n\
来源：[[sources/{source_id}]]{}.\n\n\
## 实践证据\n\
{practice}\n\
## 当前边界\n\
{}\n\n\
## 下一次实践\n\
建议在一次相关工作中记录本人角色、可观察结果与核对对象；只生成建议，不创建自动任务。\n\
{related_block}",
        if pointer.is_empty() {
            String::new()
        } else {
            format!("，{pointer}")
        },
        if evidence_level == "knowledge_only" {
            "尚无本人角色证据。成功、失败和反例需一并保留，不能用资料主张代替实践。"
        } else {
            "已记录与本人角色相连的案例；单个案例不能证明所有场景有效。"
        }
    )
}

fn ensure_source_pages(
    proposals: &mut Vec<StagedProposal>,
    vault: &Path,
    inputs: &[FrozenInput],
    sources: &HashMap<String, wiki_source::Model>,
    contributed: &[(String, String, String)],
    prior_contrib: &HashMap<String, Vec<String>>,
    index: &[NoteIndexEntry],
    today: &str,
) -> Result<(), CompileError> {
    for input in inputs {
        let rel = format!("sources/{}.md", input.source_id);
        let dest = vault.join(&rel);
        let mut pages: Vec<(String, String, String)> = Vec::new();
        if let Some(ids) = prior_contrib.get(&input.source_id) {
            for nid in ids {
                if let Some(e) = index.iter().find(|e| e.note_id == *nid) {
                    if e.page_type == "source" {
                        continue;
                    }
                    pages.push((e.note_id.clone(), e.rel.clone(), e.title.clone()));
                }
            }
        }
        for (nid, prel, title) in contributed {
            if nid.is_empty() {
                continue;
            }
            if pages.iter().any(|(n, _, _)| n == nid) {
                continue;
            }
            pages.push((nid.clone(), prel.clone(), title.clone()));
        }
        let source = sources.get(&input.source_id);
        let body = render_source_page(&input.source_id, source, &pages, today);
        if dest.exists() {
            let current = fs::read_to_string(&dest).unwrap_or_default();
            if marker_conflict(&current) {
                continue;
            }
            proposals.retain(|p| p.rel != rel);
            proposals.push(StagedProposal {
                rel,
                page_type: "source".into(),
                before_hash: content_hash(&current),
                after: body,
                op: "update".into(),
            });
        } else {
            proposals.retain(|p| p.rel != rel);
            proposals.push(StagedProposal {
                rel,
                page_type: "source".into(),
                before_hash: String::new(),
                after: body,
                op: "create".into(),
            });
        }
    }
    Ok(())
}

fn render_source_page(
    source_id: &str,
    source: Option<&wiki_source::Model>,
    contributed: &[(String, String, String)],
    today: &str,
) -> String {
    let title = source
        .and_then(|s| {
            s.source_title
                .as_deref()
                .filter(|t| !t.is_empty())
                .or_else(|| s.original_filename.as_deref().filter(|t| !t.is_empty()))
        })
        .unwrap_or("来源");
    let material_role = source
        .and_then(|s| s.material_role.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or("unspecified");
    let personal_role = source
        .and_then(|s| s.personal_role.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or("未说明");
    let coverage = source
        .and_then(|s| s.coverage_status.as_deref())
        .unwrap_or("unspecified");
    let raw_path = source.and_then(|s| s.raw_path.as_deref()).unwrap_or("");
    let format = source.and_then(|s| s.format.as_deref()).unwrap_or("");
    let page_count = source.and_then(|s| s.page_count).unwrap_or(0);
    let source_kind = source.map(|s| s.source_kind.as_str()).unwrap_or("");
    let mut contrib_body = String::new();
    if contributed.is_empty() {
        contrib_body.push_str("暂无。\n");
    } else {
        for (_nid, rel, page_title) in contributed {
            let label = if page_title.is_empty() {
                rel_without_md(rel).to_string()
            } else {
                page_title.clone()
            };
            contrib_body.push_str(&format!("- {}\n", wikilink(rel, &label)));
        }
    }
    let mut fm = String::from("---\n");
    push_yaml_scalar(&mut fm, "title", title);
    push_yaml_plain(&mut fm, "type", "source");
    push_yaml_list(&mut fm, "tags", &["type/source".into()]);
    push_yaml_plain(&mut fm, "date", today);
    push_yaml_plain(&mut fm, "updated", today);
    push_yaml_plain(&mut fm, "status", "draft");
    if !source_kind.is_empty() {
        push_yaml_plain(&mut fm, "source_kind", source_kind);
    }
    push_yaml_quoted(&mut fm, "codeg_source_id", source_id);
    push_yaml_quoted(&mut fm, "codeg_note_id", source_id);
    push_yaml_plain(&mut fm, "material_role", material_role);
    if personal_role != "未说明" {
        push_yaml_quoted(&mut fm, "personal_role", personal_role);
    }
    fm.push_str("---\n\n");
    let pages_line = if page_count > 0 {
        format!("- page_count: {page_count}\n")
    } else {
        String::new()
    };
    let format_line = if format.is_empty() {
        String::new()
    } else {
        format!("- format: {format}\n")
    };
    format!(
        "{fm}{CONTENT_START}\n\
# {title}\n\n\
派生阅读页。原始证据见 `raw/`，compile 不会改写 raw。\n\n\
- material_role: {material_role}\n\
- personal_role: {personal_role}\n\
- extraction coverage: {coverage}\n\
{pages_line}{format_line}\
- raw: `{raw_path}`\n\n\
## 贡献到的页面\n\
{contrib_body}\
{CONTENT_END}\n"
    )
}

fn append_evidence(
    current: &str,
    candidate: &Value,
    today: &str,
    source: Option<&wiki_source::Model>,
) -> Result<String, CompileError> {
    if marker_conflict(current) {
        return Err(CompileError::Conflict(
            "duplicate or missing codeg-content markers; not rewriting".into(),
        ));
    }
    let claim = cand_str(candidate, "claim");
    let pointer = candidate
        .pointer("/locator/pointer")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_id = candidate
        .pointer("/locator/source_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let evidence_type = {
        let et = cand_str(candidate, "evidence_type");
        if et.is_empty() {
            "reference"
        } else {
            et
        }
    };
    let actor = {
        let a = cand_str(candidate, "actor");
        if a.is_empty() {
            "unspecified"
        } else {
            a
        }
    };
    let personal_role = candidate_personal_role(candidate, source);
    let title = {
        let t = cand_str(candidate, "title");
        if t.is_empty() {
            "case"
        } else {
            t
        }
    };
    let verification = verification_status(candidate);
    let addition = format!(
        "\n### 补充证据（{today}）\n\
- case: {title}\n\
- actor: {actor}\n\
- personal_role: {}\n\
- evidence_type: {evidence_type}\n\
- verification_status: {verification}\n\
- source: [[sources/{source_id}]]\n\
- locator: {pointer}\n\
- claim: {claim}\n\
并存主张：新来源不覆盖旧记录。\n",
        personal_role.unwrap_or("未说明")
    );
    let mut next = if let Some(idx) = current.find(CONTENT_END) {
        let mut s = String::new();
        s.push_str(&current[..idx]);
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&addition);
        s.push_str(&current[idx..]);
        s
    } else {
        format!("{current}\n{addition}")
    };
    if !source_id.is_empty() {
        next = ensure_yaml_source(&next, source_id);
    }
    next = replace_yaml_key(&next, "updated", today);
    let material_role = source.and_then(|s| s.material_role.as_deref());
    let new_level =
        compute_evidence_level(evidence_type, actor, personal_role, pointer, material_role);
    next = maybe_raise_evidence_level(&next, new_level);
    Ok(next)
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
            evidence_level: yaml_string(&text, "evidence_level"),
            projects: yaml_list(&text, "projects"),
            summary: yaml_string(&text, "summary"),
            date: yaml_string(&text, "date"),
        });
    }
    out
}

pub(crate) fn yaml_string(md: &str, key: &str) -> Option<String> {
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

pub(crate) fn yaml_list(md: &str, key: &str) -> Vec<String> {
    let Some((yaml, _)) = commit::split_frontmatter(md) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_list = false;
    for line in yaml.lines() {
        let trimmed = line.trim();
        if in_list {
            if let Some(rest) = trimmed.strip_prefix('-') {
                let v = rest.trim().trim_matches('"').trim_matches('\'').to_string();
                if !v.is_empty() {
                    out.push(v);
                }
                continue;
            }
            if !trimmed.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
                break;
            }
        }
        if trimmed == format!("{key}:") || trimmed.starts_with(&format!("{key}:")) {
            if trimmed.contains('[') && trimmed.contains(']') {
                break;
            }
            in_list = true;
        }
    }
    out
}

fn cand_str<'a>(c: &'a Value, key: &str) -> &'a str {
    c.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn candidate_personal_role<'a>(
    candidate: &'a Value,
    source: Option<&'a wiki_source::Model>,
) -> Option<&'a str> {
    let _ = candidate;
    source
        .and_then(|s| s.personal_role.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "unspecified" && *s != "未说明")
}

fn candidate_decision_state(candidate: &Value) -> &'static str {
    match cand_str(candidate, "decision_state") {
        "adopted" => "adopted",
        "replaced" => "replaced",
        _ => "proposed",
    }
}

fn verification_status(_candidate: &Value) -> &'static str {
    // No host-side verification record exists yet; model output cannot claim one.
    "source_reported"
}

fn compute_evidence_level(
    evidence_type: &str,
    actor: &str,
    personal_role: Option<&str>,
    locator: &str,
    material_role: Option<&str>,
) -> &'static str {
    let material_ref = material_role
        .map(|s| s.eq_ignore_ascii_case("reference"))
        .unwrap_or(false);
    if material_ref || evidence_type == "reference" {
        return "knowledge_only";
    }
    let personal = personal_role
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "unspecified" && *s != "未说明");
    if personal.is_none() {
        return "knowledge_only";
    }
    let agent_or_team = matches!(actor.to_ascii_lowercase().as_str(), "agent" | "team");
    let concrete = !locator.trim().is_empty();
    match evidence_type {
        "result" if concrete && !agent_or_team => "practice_supported",
        "application" | "reflection" | "result" => "practice_reported",
        _ => "knowledge_only",
    }
}

fn evidence_rank(level: &str) -> u8 {
    match level {
        "practice_supported" => 2,
        "practice_reported" => 1,
        _ => 0,
    }
}

fn sanitize_capability_evidence(
    proposals: &mut [commit::StagedProposal],
    sources: &HashMap<String, wiki_source::Model>,
) {
    for p in proposals.iter_mut() {
        if p.page_type != "capability" {
            continue;
        }
        let source_ids = source_ids_in_markdown(&p.after);
        // Every cited source must independently carry an own-work annotation;
        // one trusted source must not unlock evidence attributed to another.
        let host_allows_practice = !source_ids.is_empty()
            && source_ids.iter().all(|id| {
                sources.get(id).is_some_and(|s| {
                    let role = s.personal_role.as_deref().map(str::trim).unwrap_or("");
                    let material = s.material_role.as_deref().unwrap_or("");
                    !role.is_empty()
                        && role != "unspecified"
                        && role != "未说明"
                        && !material.eq_ignore_ascii_case("reference")
                })
            });
        let trusted_role = if source_ids.len() == 1 {
            sources
                .get(&source_ids[0])
                .and_then(|s| s.personal_role.as_deref())
                .filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        let mut rewritten = String::with_capacity(p.after.len());
        for line in p.after.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("verification_status:")
                || trimmed.starts_with("- verification_status:")
            {
                let prefix = &line[..line.len() - trimmed.len()];
                let bullet = if trimmed.starts_with('-') { "- " } else { "" };
                rewritten.push_str(prefix);
                rewritten.push_str(bullet);
                rewritten.push_str("verification_status: source_reported");
            } else if trimmed.starts_with("personal_role:")
                || trimmed.starts_with("- personal_role:")
            {
                let role = trusted_role.unwrap_or("未说明");
                let prefix = &line[..line.len() - trimmed.len()];
                let bullet = if trimmed.starts_with('-') { "- " } else { "" };
                rewritten.push_str(prefix);
                rewritten.push_str(bullet);
                rewritten.push_str(&format!("personal_role: {role}"));
            } else if line.contains("evidence_level: practice_supported")
                || line.contains("evidence_level: practice_reported")
            {
                if host_allows_practice {
                    rewritten.push_str(line);
                } else {
                    rewritten.push_str(
                        &line
                            .replace("practice_supported", "knowledge_only")
                            .replace("practice_reported", "knowledge_only"),
                    );
                }
            } else {
                rewritten.push_str(line);
            }
            rewritten.push('\n');
        }
        if !p.after.ends_with('\n') {
            rewritten.pop();
        }
        p.after = rewritten;
    }
}

fn source_id_from_text(text: &str) -> Option<String> {
    let marker = "[[sources/";
    let start = text.find(marker)? + marker.len();
    let end = text[start..].find(|c: char| c == ']' || c == '|')?;
    let id = text[start..start + end].trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn source_ids_in_markdown(md: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = md;
    while let Some(pos) = rest.find("[[sources/") {
        let tail = &rest[pos..];
        if let Some(id) = source_id_from_text(tail) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        rest = &tail[2..];
    }
    ids
}

fn yaml_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn yaml_needs_quote(s: &str) -> bool {
    s.is_empty()
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.contains("[[")
        || s.bytes().any(|b| {
            matches!(
                b,
                b':' | b'#'
                    | b'{'
                    | b'['
                    | b']'
                    | b','
                    | b'&'
                    | b'*'
                    | b'!'
                    | b'|'
                    | b'>'
                    | b'"'
                    | b'\''
                    | b'\\'
                    | b'\n'
                    | b'%'
                    | b'@'
                    | b'`'
            )
        })
}

fn push_yaml_plain(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn push_yaml_quoted(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&yaml_quote(value));
    out.push('\n');
}

fn push_yaml_scalar(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    if yaml_needs_quote(value) {
        out.push_str(&yaml_quote(value));
    } else {
        out.push_str(value);
    }
    out.push('\n');
}

fn push_yaml_list(out: &mut String, key: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(key);
    out.push_str(":\n");
    for item in items {
        out.push_str("  - ");
        out.push_str(&yaml_quote(item));
        out.push('\n');
    }
}

fn rel_without_md(rel: &str) -> &str {
    rel.strip_suffix(".md").unwrap_or(rel)
}

fn wikilink(rel: &str, title: &str) -> String {
    let path = rel_without_md(rel);
    if title.is_empty() || title == path {
        format!("[[{path}]]")
    } else {
        format!("[[{path}|{title}]]")
    }
}

fn marker_conflict(text: &str) -> bool {
    let starts = text.matches(CONTENT_START).count();
    let ends = text.matches(CONTENT_END).count();
    starts != 1 || ends != 1 || {
        let s = text.find(CONTENT_START).unwrap_or(0);
        let e = text.find(CONTENT_END).unwrap_or(0);
        e < s
    }
}

fn local_date(at: DateTime<Utc>, timezone: &str) -> chrono::NaiveDate {
    let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    at.with_timezone(&tz).date_naive()
}

fn page_cites_source(md: &str, source_id: &str) -> bool {
    md.contains(&format!("[[sources/{source_id}]]")) || md.contains(&format!("sources/{source_id}"))
}

fn compiled_pages_from(proposals: &[StagedProposal]) -> Vec<CompiledPage> {
    proposals
        .iter()
        .filter(|p| !matches!(p.page_type.as_str(), "source" | "index" | "daily"))
        .map(|p| CompiledPage {
            rel: p.rel.clone(),
            page_type: p.page_type.clone(),
            title: yaml_string(&p.after, "title").unwrap_or_default(),
            source_id: first_source_id(&p.after),
        })
        .collect()
}

fn first_source_id(md: &str) -> String {
    for item in yaml_list(md, "sources") {
        if let Some(rest) = item
            .trim_matches('"')
            .strip_prefix("[[sources/")
            .or_else(|| item.strip_prefix("[[sources/"))
        {
            let id = rest.split(['|', ']']).next().unwrap_or("").trim();
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }
    String::new()
}

fn ensure_yaml_source(md: &str, source_id: &str) -> String {
    let link = format!("[[sources/{source_id}]]");
    let Some((yaml, body)) = commit::split_frontmatter(md) else {
        return md.to_string();
    };
    if yaml.contains(&link) {
        return md.to_string();
    }
    let item = format!("  - {}", yaml_quote(&link));
    let mut lines: Vec<String> = yaml.lines().map(|s| s.to_string()).collect();
    if let Some(idx) = lines
        .iter()
        .position(|l| l.trim() == "sources:" || l.trim().starts_with("sources:"))
    {
        if lines[idx].trim() == "sources: []" || lines[idx].contains("[]") {
            lines[idx] = "sources:".into();
            lines.insert(idx + 1, item);
        } else {
            lines.insert(idx + 1, item);
        }
    } else {
        lines.push("sources:".into());
        lines.push(item);
    }
    let yaml = lines.join("\n") + "\n";
    format!("---\n{yaml}---{body}")
}

fn replace_yaml_key(md: &str, key: &str, value: &str) -> String {
    let Some((yaml, body)) = commit::split_frontmatter(md) else {
        return md.to_string();
    };
    let mut found = false;
    let mut out = String::new();
    let prefix = format!("{key}:");
    for line in yaml.lines() {
        if line.trim().starts_with(&prefix) {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push('\n');
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        out.push_str(key);
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
    format!("---\n{out}---{body}")
}

fn maybe_raise_evidence_level(md: &str, new_level: &str) -> String {
    let current = yaml_string(md, "evidence_level").unwrap_or_else(|| "knowledge_only".into());
    if evidence_rank(new_level) <= evidence_rank(&current) {
        return md.to_string();
    }
    replace_yaml_key(md, "evidence_level", new_level)
}

/// Rebuild the generated region. Skip (do not blank) when markers are missing
/// or duplicated.
fn rebuild_generated_region(path: &Path, inner: &str) -> Result<bool, CompileError> {
    if !path.exists() {
        return Ok(false);
    }
    let current = fs::read_to_string(path).map_err(|e| CompileError::Failed(e.to_string()))?;
    if marker_conflict(&current) {
        return Ok(false);
    }
    let start = current
        .find(CONTENT_START)
        .ok_or_else(|| CompileError::Conflict("missing codeg-content start".into()))?;
    let end = current
        .find(CONTENT_END)
        .ok_or_else(|| CompileError::Conflict("missing codeg-content end".into()))?;
    let mut out = String::new();
    out.push_str(&current[..start]);
    out.push_str(CONTENT_START);
    out.push('\n');
    let inner = inner.trim_end();
    out.push_str(inner);
    if !inner.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(CONTENT_END);
    out.push_str(&current[end + CONTENT_END.len()..]);
    fs::write(path, out).map_err(|e| CompileError::Failed(e.to_string()))?;
    Ok(true)
}

pub(crate) fn body_of(md: &str) -> String {
    commit::split_frontmatter(md)
        .map(|(_, b)| b)
        .unwrap_or_else(|| md.to_string())
}

pub(crate) async fn register_consumed(
    conn: &DatabaseConnection,
    job_id: &str,
    manifest: &CompileJobManifest,
) -> Result<(), CompileError> {
    let version = if manifest.compile_contract_version.is_empty() {
        SYNTHESIZE_CONTRACT_VERSION
    } else {
        manifest.compile_contract_version.as_str()
    };
    for note in &manifest.memory_notes {
        wiki_service::insert_compile_input(conn, &note.rel, &note.content_hash, 0, version, job_id)
            .await?;
    }
    for input in &manifest.inputs {
        let ver = if input.compile_contract_version.is_empty() {
            version
        } else {
            input.compile_contract_version.as_str()
        };
        wiki_service::insert_compile_input(
            conn,
            &input.source_id,
            &input.raw_hash,
            input.annotation_revision,
            ver,
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

fn touch_index_and_journal(
    vault: &Path,
    job_id: &str,
    timezone: &str,
    sources: &HashMap<String, wiki_source::Model>,
    pages: &[CompiledPage],
) -> Result<(), CompileError> {
    let notes = scan_note_index(vault);
    rebuild_root_index(vault, &notes)?;
    rebuild_work_index(vault, &notes)?;
    rebuild_capability_index(vault, &notes)?;
    write_journals(vault, job_id, timezone, sources, pages)?;
    Ok(())
}

fn rebuild_root_index(vault: &Path, notes: &[NoteIndexEntry]) -> Result<(), CompileError> {
    let mut inner =
        String::from("- [[work/index|工作]]\n- [[capabilities/index|能力]]\n- [[sources|资料]]\n");
    inner.push_str("\n## 项目\n");
    let projects: Vec<_> = notes.iter().filter(|n| n.page_type == "project").collect();
    if projects.is_empty() {
        inner.push_str("- 暂无\n");
    } else {
        for p in projects {
            inner.push_str(&format!("- {}\n", wikilink(&p.rel, &p.title)));
        }
    }
    inner.push_str("\n## 能力\n");
    let caps: Vec<_> = notes
        .iter()
        .filter(|n| n.page_type == "capability")
        .collect();
    if caps.is_empty() {
        inner.push_str("- 暂无\n");
    } else {
        for c in &caps {
            let level = c.evidence_level.as_deref().unwrap_or("knowledge_only");
            inner.push_str(&format!(
                "- {} — evidence_level: {level}\n",
                wikilink(&c.rel, &c.title)
            ));
        }
    }
    inner.push_str("\n## 最近工作\n");
    inner.push_str(&recent_records_md(notes));
    inner.push_str("\n## 待解决问题\n");
    inner.push_str(&pending_gaps_md(notes));
    let _ = rebuild_generated_region(&vault.join("index.md"), &inner)?;
    Ok(())
}

fn rebuild_work_index(vault: &Path, notes: &[NoteIndexEntry]) -> Result<(), CompileError> {
    let mut inner = String::from("## 项目\n");
    let projects: Vec<_> = notes.iter().filter(|n| n.page_type == "project").collect();
    if projects.is_empty() {
        inner.push_str("- 暂无\n");
    } else {
        for p in projects {
            inner.push_str(&format!("- {}\n", wikilink(&p.rel, &p.title)));
        }
    }
    inner.push_str("\n## 职责\n");
    let areas: Vec<_> = notes.iter().filter(|n| n.page_type == "area").collect();
    if areas.is_empty() {
        inner.push_str("- 暂无（模型只能建议已有职责，不能新建）\n");
    } else {
        for a in areas {
            inner.push_str(&format!("- {}\n", wikilink(&a.rel, &a.title)));
        }
    }
    inner.push_str("\n## 最近工作\n");
    inner.push_str(&recent_records_md(notes));
    inner.push_str("\n## 待解决问题\n");
    inner.push_str(&pending_gaps_md(notes));
    let _ = rebuild_generated_region(&vault.join("work/index.md"), &inner)?;
    Ok(())
}

fn rebuild_capability_index(vault: &Path, notes: &[NoteIndexEntry]) -> Result<(), CompileError> {
    let mut inner = String::from("## 能力目录\n");
    let caps: Vec<_> = notes
        .iter()
        .filter(|n| n.page_type == "capability")
        .collect();
    if caps.is_empty() {
        inner.push_str("- 暂无\n");
    } else {
        for c in &caps {
            let level = c.evidence_level.as_deref().unwrap_or("knowledge_only");
            inner.push_str(&format!("- {} — `{level}`\n", wikilink(&c.rel, &c.title)));
        }
    }
    inner.push_str("\n## 证据缺口\n");
    let gaps: Vec<_> = caps
        .iter()
        .filter(|c| c.evidence_level.as_deref().unwrap_or("knowledge_only") == "knowledge_only")
        .collect();
    if gaps.is_empty() {
        inner.push_str("- 暂无\n");
    } else {
        for c in gaps {
            inner.push_str(&format!("- {}：暂无个人实践\n", c.title));
        }
    }
    inner.push_str("\n## 下一次实践\n");
    inner.push_str("- 在一次相关工作中记录本人角色与可核对结果。\n");
    let _ = rebuild_generated_region(&vault.join("capabilities/index.md"), &inner)?;
    Ok(())
}

fn recent_records_md(notes: &[NoteIndexEntry]) -> String {
    let mut records: Vec<_> = notes
        .iter()
        .filter(|n| n.page_type == "work-record" || n.page_type == "decision")
        .collect();
    records.sort_by(|a, b| b.date.cmp(&a.date));
    if records.is_empty() {
        return "- 暂无\n".into();
    }
    let mut out = String::new();
    for r in records.into_iter().take(10) {
        out.push_str(&format!("- {}\n", wikilink(&r.rel, &r.title)));
    }
    out
}

fn pending_gaps_md(notes: &[NoteIndexEntry]) -> String {
    let mut lines = Vec::new();
    for n in notes {
        if n.page_type == "capability"
            && n.evidence_level.as_deref().unwrap_or("knowledge_only") == "knowledge_only"
        {
            lines.push(format!("- 能力「{}」仅有 knowledge_only 证据", n.title));
        }
        if n.page_type == "decision" {
            lines.push(format!("- 决策「{}」待核对", n.title));
        }
    }
    if lines.is_empty() {
        "- 暂无\n".into()
    } else {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    }
}

fn write_journals(
    vault: &Path,
    job_id: &str,
    timezone: &str,
    sources: &HashMap<String, wiki_source::Model>,
    pages: &[CompiledPage],
) -> Result<(), CompileError> {
    let mut by_source: HashMap<String, Vec<&CompiledPage>> = HashMap::new();
    for p in pages {
        if p.source_id.is_empty() {
            continue;
        }
        by_source.entry(p.source_id.clone()).or_default().push(p);
    }
    for (source_id, source) in sources {
        let Some(occurred) = source.occurred_at else {
            continue;
        };
        let date = local_date(occurred, timezone).to_string();
        let pages_for = by_source.get(source_id).cloned().unwrap_or_default();
        let mut bullets = Vec::new();
        for p in pages_for
            .iter()
            .filter(|p| p.page_type == "work-record" || p.page_type == "decision")
        {
            bullets.push(format!(
                "- {} <!-- source:{source_id} -->",
                wikilink(&p.rel, &p.title)
            ));
        }
        if bullets.is_empty() {
            bullets.push(format!(
                "- 整理来源 [[sources/{source_id}]] job=`{job_id}` <!-- source:{source_id} -->"
            ));
        }
        append_journal(vault, &date, &bullets, source_id)?;
    }
    Ok(())
}

fn append_journal(
    vault: &Path,
    date: &str,
    bullets: &[String],
    source_id: &str,
) -> Result<(), CompileError> {
    let path = vault.join("journal").join(format!("{date}.md"));
    if path.exists() {
        let current = fs::read_to_string(&path).map_err(|e| CompileError::Failed(e.to_string()))?;
        if marker_conflict(&current) {
            return Ok(());
        }
        if current.contains(&format!("source:{source_id}")) {
            return Ok(());
        }
        let addition: String = bullets.iter().map(|b| format!("{b}\n")).collect();
        let start = current.find(CONTENT_START);
        let end = current.find(CONTENT_END);
        let (Some(_s), Some(end)) = (start, end) else {
            return Ok(());
        };
        let mut next = String::new();
        next.push_str(&current[..end]);
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&addition);
        next.push_str(&current[end..]);
        fs::write(&path, next).map_err(|e| CompileError::Failed(e.to_string()))?;
        return Ok(());
    }
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| CompileError::Failed(e.to_string()))?;
    let mut inner = String::new();
    for b in bullets {
        inner.push_str(b);
        inner.push('\n');
    }
    let body = format!(
        "---\ntitle: {date}\ntype: daily\ntags:\n  - \"type/daily\"\ndate: {date}\n---\n\n{CONTENT_START}\n{inner}{CONTENT_END}\n"
    );
    fs::write(&path, body).map_err(|e| CompileError::Failed(e.to_string()))?;
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
            text.push_str(&"x".repeat(2_000));
            text.push('\n');
        }
        let segs = split_segments(&text);
        assert!(segs.len() > 1);
    }

    fn candidate_list(source_id: &str, n: usize) -> Value {
        let cands: Vec<Value> = (0..n)
            .map(|i| {
                candidate_json(
                    &format!("c{i}"),
                    "method",
                    &format!("Method {i}"),
                    "reference",
                    source_id,
                    json!({}),
                )
            })
            .collect();
        json!({ "candidates": cands })
    }

    #[test]
    fn candidates_payload_points_at_raw_file_not_excerpt() {
        let v = candidates_llm_input(
            "sid",
            "raw/sessions/sid.md",
            "hash",
            0,
            &["s1".into(), "s2".into()],
            "/tmp/wiki-vault",
            "/tmp/wiki-state/staging/job",
        );
        assert_eq!(v["raw_path"], "raw/sessions/sid.md");
        assert_eq!(v["source_id"], "sid");
        assert!(
            v.get("segment_ids")
                .and_then(|x| x.as_array())
                .unwrap()
                .len()
                == 2
        );
        assert!(v.get("segment_text").is_none());
        assert!(v.get("redacted_text").is_none());
        let dumped = v.to_string();
        assert!(
            !dumped.contains("Require an idempotency"),
            "payload must not embed source body"
        );
    }

    #[test]
    fn leaf_body_over_soft_cap_warns_without_truncating() {
        let body = "x".repeat(LEAF_BODY_SOFT_CHARS + 50);
        let after = format!("---\ntitle: t\ntype: capability\n---\n\n{body}\n");
        let warning = check_leaf_body("capability", &after)
            .unwrap()
            .expect("soft cap should warn");
        assert!(warning.contains("capability"));
        assert!(body_of(&after).chars().count() >= LEAF_BODY_SOFT_CHARS + 50);
    }

    #[test]
    fn leaf_body_runaway_is_rejected() {
        let body = "x".repeat(LEAF_BODY_HARD_CHARS + 1);
        let after = format!("---\ntitle: t\ntype: method\n---\n\n{body}\n");
        assert!(check_leaf_body("method", &after).is_err());
    }

    #[test]
    fn index_pages_are_not_leaf_body_capped() {
        let body = "x".repeat(LEAF_BODY_HARD_CHARS + 1);
        let after = format!("---\ntype: index\n---\n\n{body}\n");
        assert!(check_leaf_body("index", &after).unwrap().is_none());
    }

    fn write_turn_note(vault: &Path, source_id: &str, body: &str) -> MemoryNoteRef {
        let rel = format!("work/turns/{source_id}.md");
        let page = format!(
            "---\ntitle: Turn work\ntype: turn-summary\ncodeg_note_id: \"n-{source_id}\"\ncodeg_source_id: \"{source_id}\"\n---\n\n{CONTENT_START}\n{body}\n{CONTENT_END}\n"
        );
        fs::create_dir_all(vault.join("work/turns")).unwrap();
        fs::write(vault.join(&rel), &page).unwrap();
        MemoryNoteRef {
            rel,
            content_hash: content_hash(&page),
            page_type: "turn-summary".into(),
            project_binding_ids: Vec::new(),
        }
    }

    #[test]
    fn synthesize_payload_has_memory_rels_no_candidates() {
        let v = json!({
            "schema": SYNTHESIZE_CONTRACT_VERSION,
            "memory_notes": [{"rel": "work/turns/sid.md", "content_hash": "abc"}],
            "extra_read_roots": [],
        });
        assert!(v.get("candidates").is_none());
        assert!(v.get("segment_ids").is_none());
        assert_eq!(v["memory_notes"][0]["rel"], "work/turns/sid.md");
        assert!(!v.to_string().contains("raw/sessions"));
    }

    #[tokio::test]
    async fn mock_llm_writes_capability_page_from_memory_notes() {
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
        write_turn_note(&vault, source_id, "Fixed list API cursor pagination.");
        let job = wiki_job::ActiveModel {
            id: Set("job-syn-1".into()),
            vault_id: Set(vault_row.id.clone()),
            source_id: Set(None),
            kind: Set("wiki_synthesize".into()),
            status: Set("running".into()),
            dedupe_key: Set(Some("req-1".into())),
            input_manifest: Set(None),
            config_version: Set(Some(SYNTHESIZE_CONTRACT_VERSION.into())),
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
        let cap = vault.join("capabilities/interface-design-mock.md");
        assert!(cap.is_file(), "capability page from synthesize");
        let text = fs::read_to_string(&cap).unwrap();
        assert!(text.contains("type: capability"));
        assert!(vault.join(format!("work/turns/{source_id}.md")).is_file());

        let frozen = freeze_manifest(&db.conn, &vault_row.id, 0, &vault)
            .await
            .unwrap();
        assert!(
            frozen.memory_notes.is_empty()
                || wiki_service::compile_input_consumed(
                    &db.conn,
                    &frozen.memory_notes[0].rel,
                    &frozen.memory_notes[0].content_hash,
                    0,
                    SYNTHESIZE_CONTRACT_VERSION
                )
                .await
                .unwrap()
                || frozen.memory_notes.is_empty()
        );
        let job2 = wiki_job::ActiveModel {
            id: Set("job-syn-2".into()),
            vault_id: Set(vault_row.id),
            source_id: Set(None),
            kind: Set("wiki_synthesize".into()),
            status: Set("running".into()),
            dedupe_key: Set(Some("req-2".into())),
            input_manifest: Set(None),
            config_version: Set(Some(SYNTHESIZE_CONTRACT_VERSION.into())),
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

    #[tokio::test]
    async fn leaf_over_hard_cap_rejects_that_page_only() {
        let fx = Fixture::new().await;
        write_turn_note(&fx.vault, "sid-cap", "did work");
        let huge = "x".repeat(LEAF_BODY_HARD_CHARS + 10);
        let ok_body = format!(
            "---\ntitle: \"Ok page\"\ntype: method\ntags:\n  - \"type/method\"\ncodeg_note_id: \"ok-note\"\n---\n\n{CONTENT_START}\n# Ok\n\nshort.\n{CONTENT_END}\n"
        );
        let long_body = format!(
            "---\ntitle: \"Too long\"\ntype: capability\ntags:\n  - \"type/capability\"\ncodeg_note_id: \"long\"\n---\n\n{CONTENT_START}\n{huge}\n{CONTENT_END}\n"
        );
        let llm = MockWikiLlm::default().with_stage(
            "synthesize",
            json!({
                "schema": SYNTHESIZE_CONTRACT_VERSION,
                "processed_inputs": [{"rel": "work/turns/sid-cap.md", "content_hash": "x"}],
                "page_proposals": [
                    {
                        "op": "create",
                        "type": "capability",
                        "path": "capabilities/too-long.md",
                        "body": long_body
                    },
                    {
                        "op": "create",
                        "type": "method",
                        "path": "knowledge/methods/ok.md",
                        "body": ok_body
                    }
                ],
                "nothing_to_persist": false,
                "warnings": []
            }),
        );
        let job = fx.add_job("job-cap", "sid-cap").await;
        let out = run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        assert!(!fx.vault.join("capabilities/too-long.md").exists());
        assert!(fx.vault.join("knowledge/methods/ok.md").is_file());
        assert!(out
            .committed
            .iter()
            .any(|r| r.contains("knowledge/methods/ok.md")));
    }

    #[tokio::test]
    async fn synthesize_failure_does_not_roll_back_turn_page() {
        let fx = Fixture::new().await;
        write_turn_note(&fx.vault, "keep-turn", "turn body stays");
        let job = fx.add_job("job-fail", "keep-turn").await;
        let err = run_compile_job(
            &fx.db.conn,
            &job,
            &MockWikiLlm::failing(),
            &fx.vault,
            &fx.state,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("mock") || err.retryable());
        assert!(fx.vault.join("work/turns/keep-turn.md").is_file());
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        vault: PathBuf,
        state: PathBuf,
        db: crate::db::AppDatabase,
        vault_id: String,
    }

    impl Fixture {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let vault = dir.path().join("vault");
            let state = dir.path().join("state");
            vault::initialize_vault(&vault).unwrap();
            vault::initialize_state_root(&state).unwrap();
            let db = fresh_in_memory_db().await;
            let now = Utc::now();
            wiki_vault::ActiveModel {
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
            let _ = settings::save_settings(
                &db.conn,
                &settings::WikiSettings {
                    timezone: "UTC".into(),
                    ..Default::default()
                },
            )
            .await;
            Self {
                _dir: dir,
                vault,
                state,
                db,
                vault_id: "v1".into(),
            }
        }

        async fn add_source(
            &self,
            source_id: &str,
            seq: i64,
            occurred_at: Option<DateTime<Utc>>,
            material_role: Option<&str>,
            personal_role: Option<&str>,
            source_title: Option<&str>,
            original_filename: Option<&str>,
            coverage_status: Option<&str>,
        ) {
            let now = Utc::now();
            let raw_rel = format!("raw/imports/{source_id}.md");
            let raw_body = format!(
                "---\ntitle: spec\ntype: document-dump\n---\n\n# API spec\n\nRequire an idempotency key on retried POSTs.\n"
            );
            let hash = content_hash(&raw_body);
            fs::create_dir_all(self.vault.join("raw/imports")).unwrap();
            fs::write(self.vault.join(&raw_rel), &raw_body).unwrap();
            wiki_source::ActiveModel {
                id: Set(source_id.into()),
                source_group_id: Set(source_id.into()),
                vault_id: Set(self.vault_id.clone()),
                source_kind: Set("document".into()),
                source_seq: Set(seq),
                run_id: Set(None),
                original_hash: Set(None),
                raw_path: Set(Some(raw_rel)),
                raw_hash: Set(Some(hash)),
                extractor_version: Set(None),
                coverage_status: Set(coverage_status.map(|s| s.to_string())),
                eligibility: Set("ready".into()),
                material_role: Set(material_role.map(|s| s.to_string())),
                personal_role: Set(personal_role.map(|s| s.to_string())),
                annotation_revision: Set(0),
                conversation_id: Set(None),
                folder_id: Set(None),
                root_folder_id: Set(None),
                agent_type: Set(None),
                model: Set(None),
                mode: Set(None),
                captured_at: Set(None),
                occurred_at: Set(occurred_at),
                truncated: Set(false),
                redacted: Set(false),
                request_id: Set(None),
                original_filename: Set(original_filename.map(|s| s.to_string())),
                format: Set(Some("markdown".into())),
                source_title: Set(source_title.map(|s| s.to_string())),
                source_url: Set(None),
                author: Set(None),
                project_ids: Set(None),
                area_ids: Set(None),
                warnings: Set(None),
                page_count: Set(Some(3)),
                previous_source_id: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&self.db.conn)
            .await
            .unwrap();
        }

        async fn add_job(&self, job_id: &str, source_id: &str) -> wiki_job::Model {
            let now = Utc::now();
            wiki_job::ActiveModel {
                id: Set(job_id.into()),
                vault_id: Set(self.vault_id.clone()),
                source_id: Set(Some(source_id.into())),
                kind: Set("wiki_synthesize".into()),
                status: Set("running".into()),
                dedupe_key: Set(Some(job_id.into())),
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
            .insert(&self.db.conn)
            .await
            .unwrap()
        }
    }

    fn md_pages(dir: &Path) -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("md")
                    && p.file_name().and_then(|n| n.to_str()) != Some("index.md")
            })
            .collect()
    }

    fn candidate_json(
        id: &str,
        kind: &str,
        title: &str,
        evidence_type: &str,
        source_id: &str,
        extra: Value,
    ) -> Value {
        let mut c = json!({
            "candidate_id": id,
            "kind": kind,
            "title": title,
            "claim": "The spec requires an idempotency key on retried POSTs.",
            "evidence_type": evidence_type,
            "actor": "unspecified",
            "locator": {
                "source_id": source_id,
                "segment_id": "s1",
                "pointer": "§3.2"
            }
        });
        if let Some(obj) = extra.as_object() {
            if let Some(dst) = c.as_object_mut() {
                for (k, v) in obj {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
        c
    }

    #[test]
    fn evidence_level_rules() {
        assert_eq!(
            compute_evidence_level("reference", "user", Some("author"), "§1", None),
            "knowledge_only"
        );
        assert_eq!(
            compute_evidence_level("application", "user", None, "§1", None),
            "knowledge_only"
        );
        assert_eq!(
            compute_evidence_level("application", "agent", None, "§1", None),
            "knowledge_only"
        );
        assert_eq!(
            compute_evidence_level("application", "user", Some("implemented"), "§1", None),
            "practice_reported"
        );
        assert_eq!(
            compute_evidence_level("result", "user", Some("owner"), "test.log:12", None),
            "practice_supported"
        );
        assert_eq!(
            compute_evidence_level(
                "result",
                "user",
                Some("owner"),
                "test.log:12",
                Some("reference")
            ),
            "knowledge_only"
        );
    }

    #[test]
    fn candidate_personal_role_cannot_override_source_annotation() {
        let candidate = json!({"personal_role": "主导开发"});
        let source = wiki_source::Model {
            id: "src".into(),
            source_group_id: "src".into(),
            vault_id: "v".into(),
            source_kind: "document".into(),
            source_seq: 1,
            run_id: None,
            original_hash: None,
            raw_path: None,
            raw_hash: None,
            extractor_version: None,
            coverage_status: None,
            eligibility: "ready".into(),
            material_role: Some("reference".into()),
            personal_role: None,
            annotation_revision: 0,
            conversation_id: None,
            folder_id: None,
            root_folder_id: None,
            agent_type: None,
            model: None,
            mode: None,
            captured_at: None,
            occurred_at: None,
            truncated: false,
            redacted: false,
            request_id: None,
            original_filename: None,
            format: None,
            source_title: None,
            source_url: None,
            author: None,
            project_ids: None,
            area_ids: None,
            warnings: None,
            page_count: None,
            previous_source_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(candidate_personal_role(&candidate, Some(&source)), None);
        assert_eq!(
            compute_evidence_level(
                "result",
                "user",
                candidate_personal_role(&candidate, Some(&source)),
                "build.log:1",
                source.material_role.as_deref()
            ),
            "knowledge_only"
        );
    }

    #[test]
    fn journal_date_uses_vault_timezone() {
        let at = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(local_date(at, "UTC").to_string(), "2026-01-02");
        assert_eq!(local_date(at, "Asia/Shanghai").to_string(), "2026-01-02");
        let evening = DateTime::parse_from_rfc3339("2026-01-01T16:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            local_date(evening, "Asia/Shanghai").to_string(),
            "2026-01-02"
        );
        assert_eq!(local_date(evening, "UTC").to_string(), "2026-01-01");
    }

    #[test]
    fn duplicate_markers_skip_index_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.md");
        let original = format!(
            "keep\n{CONTENT_START}\nold\n{CONTENT_END}\n{CONTENT_START}\ndup\n{CONTENT_END}\n"
        );
        fs::write(&path, &original).unwrap();
        let wrote = rebuild_generated_region(&path, "new").unwrap();
        assert!(!wrote);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn append_evidence_keeps_old_claims() {
        let current = format!(
            "---\ntitle: Cap\ntype: capability\nevidence_level: knowledge_only\nsources:\n  - \"[[sources/old]]\"\ncodeg_note_id: \"n1\"\n---\n\n{CONTENT_START}\n# Cap\n\n## 实践证据\n- claim: old claim from first source\n{CONTENT_END}\n"
        );
        let candidate = json!({
            "title": "Cap",
            "claim": "new contradictory claim",
            "evidence_type": "reference",
            "actor": "unspecified",
            "locator": { "source_id": "newsrc", "pointer": "p.2" }
        });
        let after = append_evidence(&current, &candidate, "2026-09-12", None).unwrap();
        assert!(after.contains("old claim from first source"));
        assert!(after.contains("new contradictory claim"));
        assert!(after.contains("并存主张"));
        assert!(after.contains("[[sources/newsrc]]"));
        assert!(after.contains("[[sources/old]]"));
    }

    #[tokio::test]
    async fn freeze_manifest_lists_memory_notes_not_raw_segments() {
        let fx = Fixture::new().await;
        write_turn_note(&fx.vault, "mem-1", "did the pagination work");
        let manifest = freeze_manifest(&fx.db.conn, &fx.vault_id, 0, &fx.vault)
            .await
            .unwrap();
        assert_eq!(manifest.memory_notes.len(), 1);
        assert_eq!(manifest.memory_notes[0].rel, "work/turns/mem-1.md");
        assert!(manifest.inputs.is_empty());
        assert_eq!(
            manifest.compile_contract_version,
            SYNTHESIZE_CONTRACT_VERSION
        );
        let dumped = serde_json::to_string(&manifest).unwrap();
        assert!(!dumped.contains("candidates"));
        assert!(!dumped.contains("segment_ids"));
    }

    #[test]
    fn extra_project_roots_only_on_synthesize_policy() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let staging = dir.path().join("staging");
        let project = dir.path().join("proj");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/lib.rs"), "fn x() {}").unwrap();
        let turn = crate::acp::file_system_runtime::FsAccessPolicy::wiki_worker(&vault, &staging);
        assert!(turn.check_read(&project.join("src/lib.rs")).is_err());
        let syn = crate::acp::file_system_runtime::FsAccessPolicy::wiki_worker_with_extra_reads(
            &vault,
            &staging,
            &[project.clone()],
        );
        assert!(syn.check_read(&project.join("src/lib.rs")).is_ok());
    }
}

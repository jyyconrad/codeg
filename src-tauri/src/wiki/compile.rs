//! Freeze compile input, segment, four host steps, stage, commit.

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
use crate::wiki::llm::{WikiLlm, WikiLlmError, COMPILE_CONTRACT_VERSION};
use crate::wiki::raw::{self, content_hash};
use crate::wiki::settings;
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

    if manifest.inputs.is_empty() {
        append_compile_log(vault, &job.id, "compile nothing_to_persist")?;
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
    let mut all_candidates: Vec<Value> = Vec::new();
    let mut allowed_reads: Vec<PathBuf> = Vec::new();
    let mut sources_by_id: HashMap<String, wiki_source::Model> = HashMap::new();
    let mut prior_contrib: HashMap<String, Vec<String>> = HashMap::new();
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
        let raw_text =
            fs::read_to_string(&abs).map_err(|e| CompileError::Failed(format!("read raw: {e}")))?;
        if source.raw_hash.as_deref() != Some(input.raw_hash.as_str()) {
            return Err(CompileError::Conflict(format!(
                "source {} raw hash changed",
                source.id
            )));
        }
        let rows = wiki_service::list_contributions_for_source(conn, &source.id).await?;
        prior_contrib.insert(
            source.id.clone(),
            rows.into_iter().map(|r| r.note_id).collect(),
        );
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
        sources_by_id.insert(source.id.clone(), source);
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
            &sources_by_id,
            &today,
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
        &manifest.inputs,
        &sources_by_id,
        &contributed,
        &prior_contrib,
        &index,
        &today,
    )?;
    sanitize_capability_evidence(&mut proposals, &sources_by_id);
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

    let compiled_pages = compiled_pages_from(&proposals);
    let committed = commit::commit_proposals(vault, state_root, &job.id, &proposals)?;
    register_consumed(conn, &job.id, &manifest).await?;
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
        for input in &manifest.inputs {
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
    }
    append_compile_log(
        vault,
        &job.id,
        &format!("compile succeeded files={}", committed.files.len()),
    )?;
    touch_index_and_journal(vault, &job.id, &timezone, &sources_by_id, &compiled_pages)?;
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
            obj.entry("segment_id").or_insert_with(|| json!(segment_id));
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

fn yaml_list(md: &str, key: &str) -> Vec<String> {
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

pub(crate) async fn register_consumed(
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
                kind: Set("compile".into()),
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
    async fn reference_candidate_is_knowledge_only_without_personal_practice() {
        let fx = Fixture::new().await;
        let source_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            Some("接口规范"),
            Some("api-spec.md"),
            Some("complete"),
        )
        .await;
        let job = fx.add_job("job-ref", source_id).await;
        let llm = MockWikiLlm::default();
        run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        let pages = md_pages(&fx.vault.join("capabilities"));
        assert_eq!(pages.len(), 1);
        let text = fs::read_to_string(&pages[0]).unwrap();
        assert!(text.contains("evidence_level: knowledge_only"));
        assert!(text.contains("暂无个人实践"));
        assert!(text.contains("verification_status: source_reported"));
        assert!(!text.contains("product verified"));
        assert!(!text.contains("practice_supported"));
    }

    #[tokio::test]
    async fn malicious_candidate_role_cannot_upgrade_reference_source() {
        let fx = Fixture::new().await;
        let source_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            Some("规范"),
            Some("spec.md"),
            Some("complete"),
        )
        .await;
        let job = fx.add_job("job-malicious-role", source_id).await;
        let llm = MockWikiLlm::default().with_stage("candidates", json!({
            "candidates": [candidate_json(
                "c1", "capability", "Interface design", "result", source_id,
                json!({"actor": "user", "personal_role": "owner", "verification_status": "user_confirmed"}),
            )]
        }));
        run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        let pages = md_pages(&fx.vault.join("capabilities"));
        assert_eq!(pages.len(), 1);
        let text = fs::read_to_string(&pages[0]).unwrap();
        assert!(text.contains("evidence_level: knowledge_only"));
        assert!(text.contains("personal_role: 未说明"));
        assert!(text.contains("verification_status: source_reported"));
        assert!(!text.contains("practice_supported"));
    }

    #[tokio::test]
    async fn same_title_without_note_id_creates_two_pages() {
        let fx = Fixture::new().await;
        let source_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            None,
            None,
            None,
        )
        .await;
        let existing = format!(
            "---\ntitle: Interface design\ntype: capability\nstatus: draft\nevidence_level: knowledge_only\ncodeg_note_id: \"11111111-1111-4111-8111-111111111111\"\n---\n\n{CONTENT_START}\n# Interface design\n\nexisting page body\n{CONTENT_END}\n"
        );
        fs::write(
            fx.vault.join("capabilities/interface-design-11111111.md"),
            &existing,
        )
        .unwrap();
        let job = fx.add_job("job-same-title", source_id).await;
        let llm = MockWikiLlm::default()
            .with_stage(
                "candidates",
                json!({
                    "candidates": [candidate_json(
                        "c1",
                        "capability",
                        "Interface design",
                        "reference",
                        source_id,
                        json!({}),
                    )]
                }),
            )
            .with_stage(
                "match",
                json!({
                    "matches": [{
                        "candidate_id": "c1",
                        "relation": "same",
                        "existing_note_id": null,
                        "reason": "same title only"
                    }]
                }),
            );
        run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        let pages = md_pages(&fx.vault.join("capabilities"));
        assert_eq!(pages.len(), 2, "title-only same must not merge");
        let existing_now =
            fs::read_to_string(fx.vault.join("capabilities/interface-design-11111111.md")).unwrap();
        assert!(existing_now.contains("existing page body"));
        assert!(!existing_now.contains("补充证据"));
    }

    #[tokio::test]
    async fn occurred_at_writes_journal_for_source_day_not_compile_today() {
        let fx = Fixture::new().await;
        let source_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let occurred = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        fx.add_source(
            source_id,
            1,
            Some(occurred),
            Some("own-work"),
            Some("engineer"),
            None,
            None,
            None,
        )
        .await;
        let job = fx.add_job("job-journal", source_id).await;
        let llm = MockWikiLlm::default()
            .with_stage(
                "candidates",
                json!({
                    "candidates": [candidate_json(
                        "c1",
                        "work-record",
                        "Fix retry",
                        "application",
                        source_id,
                        json!({"actor": "user", "personal_role": "engineer"}),
                    )]
                }),
            )
            .with_stage(
                "match",
                json!({
                    "matches": [{
                        "candidate_id": "c1",
                        "relation": "new",
                        "existing_note_id": null
                    }]
                }),
            );
        run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        let dated = fx.vault.join("journal/2026-01-02.md");
        assert!(dated.is_file(), "journal must follow occurred_at");
        let journal = fs::read_to_string(&dated).unwrap();
        assert!(journal.contains("source:dddddddd-dddd-4ddd-8ddd-dddddddddddd"));
        assert!(journal.contains("[[work/records/") || journal.contains("Fix retry"));
        let today = Utc::now().date_naive().to_string();
        if today != "2026-01-02" {
            assert!(
                !fx.vault.join(format!("journal/{today}.md")).exists(),
                "must not dump historical work onto compile-today"
            );
        }
    }

    #[tokio::test]
    async fn missing_occurred_at_does_not_dump_journal() {
        let fx = Fixture::new().await;
        let source_id = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            None,
            None,
            None,
        )
        .await;
        let job = fx.add_job("job-no-journal", source_id).await;
        run_compile_job(
            &fx.db.conn,
            &job,
            &MockWikiLlm::default(),
            &fx.vault,
            &fx.state,
        )
        .await
        .unwrap();
        let journal_md = md_pages(&fx.vault.join("journal"));
        assert!(
            journal_md.is_empty(),
            "external import without occurred_at must not write daily journal content"
        );
        let log = fs::read_to_string(fx.vault.join("log.md")).unwrap();
        assert!(log.contains("compile succeeded") || log.contains("job-no-journal"));
    }

    #[tokio::test]
    async fn source_page_lists_contribution_after_commit() {
        let fx = Fixture::new().await;
        let source_id = "ffffffff-ffff-4fff-8fff-ffffffffffff";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            Some("接口规范"),
            Some("api-spec.md"),
            Some("complete"),
        )
        .await;
        let job = fx.add_job("job-contrib", source_id).await;
        run_compile_job(
            &fx.db.conn,
            &job,
            &MockWikiLlm::default(),
            &fx.vault,
            &fx.state,
        )
        .await
        .unwrap();
        let src = fs::read_to_string(fx.vault.join(format!("sources/{source_id}.md"))).unwrap();
        assert!(src.contains("接口规范") || src.contains("api-spec.md"));
        assert!(src.contains("material_role: reference"));
        assert!(src.contains("extraction coverage: complete"));
        assert!(src.contains("raw/imports/"));
        assert!(src.contains("## 贡献到的页面"));
        assert!(src.contains("[[capabilities/") || src.contains("Interface design"));
        let caps = md_pages(&fx.vault.join("capabilities"));
        assert_eq!(caps.len(), 1);
        let cap_rel = caps[0]
            .strip_prefix(&fx.vault)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let link_path = cap_rel.trim_end_matches(".md");
        assert!(src.contains(&format!("[[{link_path}")));
    }

    #[tokio::test]
    async fn index_generated_region_updates_and_keeps_user_text() {
        let fx = Fixture::new().await;
        let source_id = "99999999-9999-4999-8999-999999999999";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            None,
            None,
            None,
        )
        .await;
        for rel in ["index.md", "work/index.md", "capabilities/index.md"] {
            let path = fx.vault.join(rel);
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str("\n用户备注：请保留\n");
            fs::write(&path, text).unwrap();
        }
        let job = fx.add_job("job-index", source_id).await;
        run_compile_job(
            &fx.db.conn,
            &job,
            &MockWikiLlm::default(),
            &fx.vault,
            &fx.state,
        )
        .await
        .unwrap();
        for rel in ["index.md", "work/index.md", "capabilities/index.md"] {
            let text = fs::read_to_string(fx.vault.join(rel)).unwrap();
            assert!(
                text.contains("用户备注：请保留"),
                "{rel} must keep user text outside markers"
            );
            assert_eq!(text.matches(CONTENT_START).count(), 1);
            assert_eq!(text.matches(CONTENT_END).count(), 1);
        }
        let caps_index = fs::read_to_string(fx.vault.join("capabilities/index.md")).unwrap();
        assert!(caps_index.contains("knowledge_only") || caps_index.contains("Interface design"));
        let root = fs::read_to_string(fx.vault.join("index.md")).unwrap();
        assert!(root.contains("[[work/index"));
        assert!(root.contains("[[capabilities/index"));
    }

    #[tokio::test]
    async fn proposal_only_decision_has_proposed_state() {
        let fx = Fixture::new().await;
        let source_id = "12121212-1212-4121-8121-121212121212";
        fx.add_source(
            source_id,
            1,
            None,
            Some("reference"),
            None,
            None,
            None,
            None,
        )
        .await;
        let job = fx.add_job("job-decision", source_id).await;
        let llm = MockWikiLlm::default()
            .with_stage(
                "candidates",
                json!({
                    "candidates": [candidate_json(
                        "c1",
                        "decision",
                        "Adopt Postgres",
                        "reference",
                        source_id,
                        json!({
                            "constraints": "Must run on existing hosts",
                            "options": "Postgres or MySQL",
                            "choice": "Propose Postgres",
                            "rationale": "JSON support"
                        }),
                    )]
                }),
            )
            .with_stage(
                "match",
                json!({
                    "matches": [{
                        "candidate_id": "c1",
                        "relation": "new",
                        "existing_note_id": null
                    }]
                }),
            );
        run_compile_job(&fx.db.conn, &job, &llm, &fx.vault, &fx.state)
            .await
            .unwrap();
        let pages = md_pages(&fx.vault.join("work/decisions"));
        assert_eq!(pages.len(), 1);
        let text = fs::read_to_string(&pages[0]).unwrap();
        assert!(text.contains("decision_state: proposed"));
        assert!(text.contains("status: draft"));
        assert!(!text.contains("status: proposed"));
        assert!(!text.contains("decision_state: adopted"));
        assert!(text.contains("## 约束"));
        assert!(text.contains("## 备选方案"));
        assert!(text.contains("## 选择"));
        assert!(text.contains("## 理由"));
    }
}

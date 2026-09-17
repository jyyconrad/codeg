//! 把完成的ACP单轮记录整理为work/turns中的Wiki笔记。
//! source负责归档原始材料，engine/worker触发整理；Agent依据来源路径产出正文，
//! 本模块补身份、项目与来源元数据，经commit保护人工修改后写入并登记来源关联。

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::entities::wiki_job;
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::commit::{self, StagedProposal};
use crate::wiki::compile::{self, check_leaf_body, yaml_string, CompileError};
use crate::wiki::llm::{SourceReference, WikiLlm, WikiLlmError, WIKI_TURN_SUMMARY_MAX_TURNS};
use crate::wiki::raw;
use crate::wiki::result::{JobOutputManifest, WikiInput, WikiOutput};
use crate::wiki::vault::{self, CONTENT_END, CONTENT_START};
use crate::wiki::worker::WorkerError;

pub const KIND: &str = "turn_summary";
pub const PAGE_TYPE: &str = "turn-summary";

pub fn page_rel(source_id: &str) -> String {
    format!("work/turns/{source_id}.md")
}

pub fn dedupe_key(source_id: &str, raw_hash: &str) -> String {
    format!("turn_summary:{source_id}:{raw_hash}")
}

pub async fn enqueue_for_source(
    conn: &DatabaseConnection,
    vault_id: &str,
    source_id: &str,
    raw_hash: &str,
    conversation_id: Option<i32>,
    raw_path: &str,
) -> Result<wiki_job::Model, DbError> {
    let key = dedupe_key(source_id, raw_hash);
    let rel = page_rel(source_id);
    let manifest = json!({
        "source_id": source_id,
        "raw_hash": raw_hash,
        "raw_path": raw_path,
        "conversation_id": conversation_id,
        "rel": rel,
    })
    .to_string();
    wiki_service::insert_kind_job(
        conn,
        vault_id,
        KIND,
        &key,
        Some(source_id),
        Some(&manifest),
        wiki_service::InsertMode::ReuseTerminal,
    )
    .await
}

pub fn yaml_quote(s: &str) -> String {
    // JSON quoted scalars are valid YAML and correctly escape control bytes.
    serde_json::to_string(s).expect("serializing a string cannot fail")
}

pub fn wrap_memory_page(fields: MemoryPageFields<'_>) -> String {
    let mut yaml = String::from("---\ncodeg_schema_version: 2\n");
    yaml.push_str("title: ");
    yaml.push_str(&yaml_quote(fields.title));
    yaml.push('\n');
    yaml.push_str("summary: ");
    yaml.push_str(&yaml_quote(&first_body_paragraph(fields.body)));
    yaml.push_str("\nupdated_at: ");
    yaml.push_str(&yaml_quote(&Utc::now().to_rfc3339()));
    yaml.push('\n');
    yaml.push_str("type: ");
    yaml.push_str(fields.page_type);
    yaml.push('\n');
    yaml.push_str("tags:\n  - ");
    yaml.push_str(&yaml_quote(&format!("type/{}", fields.page_type)));
    yaml.push('\n');
    yaml.push_str("codeg_note_id: ");
    yaml.push_str(&yaml_quote(fields.note_id));
    yaml.push('\n');
    if let Some(sid) = fields.source_id {
        yaml.push_str("codeg_source_id: ");
        yaml.push_str(&yaml_quote(sid));
        yaml.push('\n');
    }
    match fields.conversation_id {
        Some(cid) => {
            yaml.push_str("codeg_conversation_id: ");
            yaml.push_str(&cid.to_string());
            yaml.push('\n');
        }
        None => yaml.push_str("codeg_conversation_id:\n"),
    }
    if let Some(at) = fields.occurred_at {
        yaml.push_str("occurred_at: ");
        yaml.push_str(&yaml_quote(&at.to_rfc3339()));
        yaml.push('\n');
        yaml.push_str("date: ");
        yaml.push_str(&yaml_quote(&at.date_naive().to_string()));
        yaml.push('\n');
    }
    if let Some(binding) = fields.project_binding_id {
        yaml.push_str("codeg_project_binding_id: ");
        yaml.push_str(&yaml_quote(binding));
        yaml.push('\n');
    }
    if !fields.turn_rels.is_empty() {
        yaml.push_str("turns:\n");
        for rel in fields.turn_rels {
            let link = rel.trim_end_matches(".md");
            yaml.push_str("  - ");
            yaml.push_str(&yaml_quote(&format!("[[{link}]]")));
            yaml.push('\n');
        }
    }
    if !fields.source_references.is_empty() {
        let source_ids: std::collections::BTreeSet<_> = fields
            .source_references
            .iter()
            .flat_map(|source| source.source_ids.iter())
            .collect();
        yaml.push_str("source_ids:\n");
        for id in source_ids {
            yaml.push_str(&format!("  - {}\n", yaml_quote(id)));
        }
    }
    yaml.push_str("---\n\n");
    yaml.push_str(CONTENT_START);
    yaml.push('\n');
    let body = fields.body.trim();
    yaml.push_str(body);
    if !body.ends_with('\n') {
        yaml.push('\n');
    }
    yaml.push_str(CONTENT_END);
    yaml.push('\n');
    yaml
}

pub struct MemoryPageFields<'a> {
    pub title: &'a str,
    pub page_type: &'a str,
    pub note_id: &'a str,
    pub source_id: Option<&'a str>,
    pub conversation_id: Option<i32>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub project_binding_id: Option<&'a str>,
    pub turn_rels: &'a [String],
    pub body: &'a str,
    pub source_references: &'a [SourceReference],
}

pub fn first_body_paragraph(md: &str) -> String {
    let body = compile::body_of(md);
    let inner = match (body.find(CONTENT_START), body.find(CONTENT_END)) {
        (Some(s), Some(e)) if e > s => {
            let start = s + CONTENT_START.len();
            body[start..e].trim()
        }
        _ => body.trim(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for line in inner.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        if t.is_empty() {
            if !parts.is_empty() {
                break;
            }
            continue;
        }
        parts.push(t);
        if parts.join(" ").chars().count() >= 240 {
            break;
        }
    }
    let mut s = parts.join(" ");
    if s.chars().count() > 280 {
        s = s.chars().take(280).collect();
    }
    s
}

pub fn sanitize_turn_title(title: &str, source_id: &str) -> String {
    let mut t = title.trim().to_string();
    if let Some(rest) = t.strip_prefix("ACP turn:") {
        t = rest.trim().to_string();
    }
    if t.eq_ignore_ascii_case(source_id) || t.contains(source_id) {
        t = "Turn work".into();
    }
    if t.is_empty() {
        t = "Turn work".into();
    }
    t
}

pub async fn run_turn_summary_job(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<Value, WorkerError> {
    vault::initialize_vault(vault).map_err(|e| WorkerError::Failed(e.to_string()))?;
    vault::initialize_state_root(state_root).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let source_id = job
        .source_id
        .as_deref()
        .ok_or_else(|| WikiLlmError::InvalidOutput("turn job missing source".into()))?;
    let source = wiki_service::get_source_model(conn, source_id).await?;
    if source.vault_id != job.vault_id {
        return Err(WikiLlmError::InvalidOutput("source belongs to another vault".into()).into());
    }
    let rel = page_rel(&source.id);
    let existing = read_existing(vault, &rel)?;
    let staging = state_root
        .join("staging")
        .join(&job.id)
        .join(job.attempt.to_string());
    fs::create_dir_all(&staging).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let material = material_markdown_for_source(job, &source, vault)?;
    let staging_source = staging.join("source.md");
    fs::write(&staging_source, &material).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let required = source_inputs(&[(format!("source:{}", source.id), vec![source.id.clone()])])?;
    let source_references = vec![crate::wiki::llm::SourceReference {
        rel: staging_source.to_string_lossy().into_owned(),
        source_ids: vec![source.id.clone()],
    }];
    let input = json!({
        "schema": crate::wiki::llm::TURN_SUMMARY_CONTRACT_VERSION,
        "job_id": job.id, "attempt": job.attempt,
        "source_id": source.id, "source_kind": source.source_kind,
        "source_title": source.source_title, "occurred_at": source.occurred_at,
        "conversation_id": source.conversation_id, "rel": rel,
        "source_references": source_references,
        "vault_abs": vault, "staging_abs": staging,
        "max_turns": WIKI_TURN_SUMMARY_MAX_TURNS,
    });
    let run = llm.complete_json(KIND, input).await?;
    let parsed = validate_turn_summary(&run.output, &source.id)?;
    if parsed.nothing_to_summarize {
        crate::wiki::worker::ensure_commit_allowed(conn, &job.id, llm).await?;
        let result = JobOutputManifest::no_content(
            &required,
            parsed
                .reason_code
                .as_deref()
                .unwrap_or("no_durable_content"),
            parsed.warnings,
        );
        return save_result(conn, job, &result).await;
    }
    let note_id = existing
        .as_deref()
        .and_then(|t| yaml_string(t, "codeg_note_id"))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let binding = first_project_id(&source.project_ids);
    let title = sanitize_turn_title(&parsed.title, &source.id);
    let after = wrap_memory_page(MemoryPageFields {
        title: &title,
        page_type: PAGE_TYPE,
        note_id: &note_id,
        source_id: Some(&source.id),
        conversation_id: source.conversation_id,
        occurred_at: source.occurred_at,
        project_binding_id: binding.as_deref(),
        turn_rels: &[],
        body: &parsed.body,
        source_references: &source_references,
    });
    check_leaf_body(PAGE_TYPE, &after)?;
    crate::wiki::worker::ensure_commit_allowed(conn, &job.id, llm).await?;
    let proposal = StagedProposal {
        rel: rel.clone(),
        page_type: PAGE_TYPE.into(),
        before_hash: existing
            .as_deref()
            .map(raw::content_hash)
            .unwrap_or_default(),
        after,
        op: if existing.is_some() {
            "update".into()
        } else {
            "create".into()
        },
    };
    commit::commit_proposals(vault, state_root, &job.id, &[proposal])
        .map_err(CompileError::from)?;
    let actual =
        fs::read_to_string(vault.join(&rel)).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let result = JobOutputManifest::generated(
        vec![WikiOutput {
            note_id: note_id.clone(),
            path: rel,
            title: title.clone(),
            page_type: PAGE_TYPE.into(),
            content_hash: raw::content_hash(&actual),
        }],
        &required,
        parsed.warnings,
    );
    // Persist the output before nonessential title updates; a crash can recover
    // the protected commit without invoking the model again.
    let out = save_result(conn, job, &result).await?;
    register_memory_contributions(conn, job, &result).await?;
    let _ = wiki_service::fill_source_title_if_empty(conn, &source.id, &title).await;
    commit::mark_finalized(state_root, &job.id).map_err(CompileError::from)?;
    Ok(out)
}

/// The plain memory commit manifest can be recovered without a model call.
/// Register source links idempotently before marking that manifest finalized.
pub(crate) async fn register_memory_contributions(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    result: &JobOutputManifest,
) -> Result<(), WorkerError> {
    let source_ids: std::collections::BTreeSet<_> = result
        .processed_inputs
        .iter()
        .flat_map(|input| input.source_ids.iter())
        .collect();
    for source_id in source_ids {
        let source = wiki_service::get_source_model(conn, source_id).await?;
        let existing = wiki_service::list_contributions_for_source(conn, source_id).await?;
        for output in &result.outputs {
            if existing.iter().any(|item| {
                item.note_id == output.note_id && item.commit_id.as_deref() == Some(&job.id)
            }) {
                continue;
            }
            wiki_service::insert_contribution(
                conn,
                source_id,
                source.raw_hash.as_deref().unwrap_or(""),
                source.annotation_revision,
                &output.note_id,
                Some(&job.id),
            )
            .await?;
        }
    }
    Ok(())
}

pub(crate) async fn save_result(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    result: &JobOutputManifest,
) -> Result<Value, WorkerError> {
    let output = serde_json::to_value(result).map_err(|e| WorkerError::Failed(e.to_string()))?;
    wiki_service::set_job_output_manifest(conn, &job.id, &output.to_string()).await?;
    Ok(output)
}

pub(crate) fn read_existing(vault: &Path, rel: &str) -> Result<Option<String>, WorkerError> {
    match fs::read_to_string(vault.join(rel)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(WikiLlmError::SourceReadFailed(e.to_string()).into()),
    }
}

pub(crate) fn source_inputs(
    paths: &[(String, Vec<String>)],
) -> Result<Vec<WikiInput>, WorkerError> {
    paths
        .iter()
        .map(|(rel, source_ids)| {
            let indexed = rel.starts_with("source:");
            if !indexed && !crate::wiki::paths::is_safe_vault_relative(rel) {
                return Err(WikiLlmError::SourceReadFailed("unsafe source path".into()).into());
            }
            Ok(WikiInput {
                rel: rel.clone(),
                source_ids: source_ids.clone(),
                ..Default::default()
            })
        })
        .collect()
}

fn material_markdown_for_source(
    job: &wiki_job::Model,
    source: &crate::db::entities::wiki_source::Model,
    vault: &Path,
) -> Result<String, WorkerError> {
    if let Some(manifest) = job.input_manifest.as_deref() {
        if let Ok(value) = serde_json::from_str::<Value>(manifest) {
            if let Some(markdown) = value
                .get("material_markdown")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Ok(markdown.to_string());
            }
        }
    }
    if let Some(rel) = source
        .raw_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return fs::read_to_string(vault.join(rel))
            .map_err(|_| WikiLlmError::SourceMissing(source.id.clone()).into());
    }
    Err(WikiLlmError::SourceMissing(source.id.clone()).into())
}

pub(crate) struct ParsedTurn {
    pub title: String,
    pub body: String,
    pub nothing_to_summarize: bool,
    pub reason_code: Option<String>,
    pub warnings: Vec<String>,
}

fn validate_turn_summary(v: &Value, source_id: &str) -> Result<ParsedTurn, WorkerError> {
    if v.get("source_id").and_then(Value::as_str) != Some(source_id) {
        return Err(
            WikiLlmError::InvalidOutput("source_id must match the supplied source".into()).into(),
        );
    }
    let parsed = validate_memory_output(v, crate::wiki::llm::TURN_SUMMARY_CONTRACT_VERSION)?;
    if !parsed.nothing_to_summarize
        && (parsed.title.contains(source_id) || parsed.title.starts_with("ACP turn:"))
    {
        return Err(WikiLlmError::InvalidOutput(
            "title must describe the work, not echo source identity".into(),
        )
        .into());
    }
    Ok(parsed)
}

pub(crate) fn validate_memory_output(v: &Value, schema: &str) -> Result<ParsedTurn, WorkerError> {
    let invalid = |message: &str| WorkerError::Llm(WikiLlmError::InvalidOutput(message.into()));
    if v.get("schema").and_then(Value::as_str) != Some(schema) {
        return Err(invalid("schema must be v2"));
    }
    let nothing = v
        .get("nothing_to_summarize")
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid("nothing_to_summarize is required"))?;
    let reason = v
        .get("reason_code")
        .and_then(Value::as_str)
        .map(str::to_string);
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let body = v
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if nothing {
        if !matches!(
            reason.as_deref(),
            Some("empty_input" | "fully_redacted" | "no_durable_content")
        ) {
            return Err(invalid("no_content requires a supported reason_code"));
        }
    } else if title.is_empty()
        || !has_substantive_body(&body)
        || body.starts_with("---")
        || body.contains(CONTENT_START)
        || body.contains(CONTENT_END)
    {
        return Err(invalid(
            "title and substantive Markdown body are required; host owns YAML and markers",
        ));
    }
    let warnings = serde_json::from_value(v.get("warnings").cloned().unwrap_or_else(|| json!([])))
        .map_err(|_| invalid("warnings must be strings"))?;
    Ok(ParsedTurn {
        title,
        body,
        nothing_to_summarize: nothing,
        reason_code: reason,
        warnings,
    })
}

pub(crate) fn has_substantive_body(body: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim();
        let content = line.trim_start_matches(['-', '*', ' ']);
        !content.is_empty()
            && !content.starts_with('#')
            && !content.starts_with("<!--")
            && !(content.starts_with('[') && (content.ends_with(')') || content.ends_with("]]")))
            && content.chars().filter(|c| c.is_alphanumeric()).count() >= 4
    })
}

fn first_project_id(raw: &Option<String>) -> Option<String> {
    let s = raw.as_deref()?.trim();
    if s.is_empty() {
        return None;
    }
    let ids: Vec<String> = serde_json::from_str(s).ok()?;
    ids.into_iter().find(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::llm::MockWikiLlm;
    use crate::wiki::raw::content_hash;
    use sea_orm::{ActiveModelTrait, Set};
    use tempfile::tempdir;

    #[test]
    fn heading_only_model_body_is_invalid_output() {
        let result = validate_turn_summary(
            &json!({"schema":"codeg.wiki.turn_summary.v2","source_id":"s1","title":"Work","body":"# Work","nothing_to_summarize":false}),
            "s1",
        );
        assert!(result.is_err(), "a heading alone is not a generated memory");
    }

    #[test]
    fn no_content_requires_an_enumerated_reason() {
        let result = validate_turn_summary(
            &json!({"schema":"codeg.wiki.turn_summary.v2","source_id":"s1","nothing_to_summarize":true,"warnings":["read failed"]}),
            "s1",
        );
        assert!(
            result.is_err(),
            "a model warning cannot certify legal no-content"
        );
    }

    #[test]
    fn wrap_writes_host_yaml_and_markers() {
        let page = wrap_memory_page(MemoryPageFields {
            title: "Fixed pagination",
            page_type: PAGE_TYPE,
            note_id: "note-1",
            source_id: Some("src-1"),
            conversation_id: Some(9),
            occurred_at: None,
            project_binding_id: Some("bind-1"),
            turn_rels: &[],
            body: "Edited list.rs.",
            source_references: &[],
        });
        assert!(page.contains("type: turn-summary"));
        assert!(page.contains("codeg_source_id: \"src-1\""));
        assert!(page.contains("codeg_conversation_id: 9"));
        assert!(page.contains(CONTENT_START));
        assert!(page.contains("Edited list.rs."));
        assert!(!page.contains("ACP turn:"));
    }

    #[test]
    fn wrap_records_source_ids_without_dump_wikilinks() {
        let refs = [crate::wiki::llm::SourceReference {
            rel: "raw/sessions/src-1.md".into(),
            source_ids: vec!["src-1".into()],
        }];
        let page = wrap_memory_page(MemoryPageFields {
            title: "Fixed pagination",
            page_type: PAGE_TYPE,
            note_id: "note-1",
            source_id: Some("src-1"),
            conversation_id: Some(9),
            occurred_at: None,
            project_binding_id: None,
            turn_rels: &[],
            body: "Edited list.rs.",
            source_references: &refs,
        });
        assert!(page.contains("source_ids:\n  - \"src-1\""));
        assert!(!page.contains("[[raw/sessions/src-1]]"));
        assert!(!page.contains("## 来源"));
    }

    #[test]
    fn page_rel_is_stable_per_source_id() {
        assert_eq!(page_rel("abc"), "work/turns/abc.md");
        assert_eq!(dedupe_key("abc", "hash"), "turn_summary:abc:hash");
    }

    async fn fixture() -> (
        tempfile::TempDir,
        crate::db::AppDatabase,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
        wiki_job::Model,
        std::path::PathBuf,
    ) {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        vault::initialize_vault(&vault).unwrap();
        vault::initialize_state_root(&state).unwrap();
        let db = fresh_in_memory_db().await;
        let now = Utc::now();
        let v = wiki_vault::ActiveModel {
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
        let source_id = "src-turn-1";
        let rel = format!("raw/sessions/{source_id}.md");
        let abs = vault.join(&rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let raw_body = "---\ntitle: t\n---\n\nhello raw\n";
        fs::write(&abs, raw_body).unwrap();
        let hash = content_hash(raw_body);
        wiki_source::ActiveModel {
            id: Set(source_id.into()),
            source_group_id: Set(source_id.into()),
            vault_id: Set(v.id.clone()),
            source_kind: Set("acp-turn".into()),
            source_seq: Set(1),
            run_id: Set(Some("run".into())),
            original_hash: Set(None),
            raw_path: Set(Some(rel)),
            raw_hash: Set(Some(hash.clone())),
            extractor_version: Set(None),
            coverage_status: Set(None),
            eligibility: Set("ready".into()),
            material_role: Set(None),
            personal_role: Set(None),
            annotation_revision: Set(0),
            conversation_id: Set(Some(3)),
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
            id: Set("job-turn".into()),
            vault_id: Set(v.id),
            source_id: Set(Some(source_id.into())),
            kind: Set(KIND.into()),
            status: Set("running".into()),
            dedupe_key: Set(Some(dedupe_key(source_id, &hash))),
            input_manifest: Set(None),
            config_version: Set(None),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            next_attempt_at: Set(None),
            started_at: Set(Some(now)),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        (dir, db, vault, state, source_id.into(), job, abs)
    }

    fn fixture_llm(_vault: &Path) -> MockWikiLlm {
        MockWikiLlm::default()
    }

    #[tokio::test]
    async fn failure_does_not_delete_raw() {
        let (_dir, db, vault, state, _sid, job, abs) = fixture().await;
        let llm = MockWikiLlm::failing();
        let err = run_turn_summary_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap_err();
        assert!(abs.is_file(), "raw must survive summary failure: {err}");
        assert!(fs::read_to_string(&abs).unwrap().contains("hello raw"));
        assert!(!vault.join("work/turns/src-turn-1.md").exists());
    }

    #[tokio::test]
    async fn changed_source_can_generate_memory_without_line_evidence() {
        let (_dir, db, vault, state, _sid, job, abs) = fixture().await;
        fs::write(&abs, "Source changed after capture; still valid material.").unwrap();
        let out = run_turn_summary_job(&db.conn, &job, &MockWikiLlm::default(), &vault, &state)
            .await
            .unwrap();
        assert_eq!(out["outcome"], "generated");
        let note = fs::read_to_string(vault.join("work/turns/src-turn-1.md")).unwrap();
        assert!(note.contains("codeg_source_id: \"src-turn-1\""));
        assert!(!note.contains("[[raw/sessions/src-turn-1]]"));
        assert!(!note.contains("start_line:"));
        let current = wiki_service::get_job_model(&db.conn, &job.id)
            .await
            .unwrap();
        assert!(!current
            .input_manifest
            .unwrap_or_default()
            .contains("required_inputs"));
    }

    #[tokio::test]
    async fn cancelled_job_does_not_commit_generated_memory() {
        let (_dir, db, vault, state, _sid, job, _abs) = fixture().await;
        wiki_service::cancel_job(&db.conn, &job.id).await.unwrap();
        let err = run_turn_summary_job(&db.conn, &job, &fixture_llm(&vault), &vault, &state)
            .await
            .unwrap_err();
        assert_eq!(err.error_code(), "cancelled");
        assert!(!vault.join("work/turns/src-turn-1.md").exists());
    }

    #[tokio::test]
    async fn nothing_to_summarize_succeeds_without_page() {
        let (_dir, db, vault, state, _sid, job, abs) = fixture().await;
        let llm = fixture_llm(&vault).with_stage(
            KIND,
            json!({
                "schema": "codeg.wiki.turn_summary.v2",
                "source_id": "src-turn-1",
                "title": "",
                "body": "",
                "nothing_to_summarize": true,
                "warnings": [],
                "reason_code": "no_durable_content"
            }),
        );
        let out = run_turn_summary_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap();
        assert_eq!(out["outcome"], "no_content");
        assert!(out["outputs"].as_array().unwrap().is_empty());
        assert!(abs.is_file());
        assert!(!vault.join("work/turns/src-turn-1.md").exists());
    }

    #[tokio::test]
    async fn session_rollup_uses_source_paths_and_registers_references() {
        let (_dir, db, vault, state, sid, turn_job, _abs) = fixture().await;
        let job = crate::wiki::session_rollup::enqueue_session_job(&db.conn, &turn_job.vault_id, 3)
            .await
            .unwrap();
        let out = crate::wiki::session_rollup::run_session_rollup_job(
            &db.conn,
            &job,
            &fixture_llm(&vault),
            &vault,
            &state,
        )
        .await
        .unwrap();
        assert_eq!(out["outcome"], "generated");
        assert_eq!(out["outputs"][0]["path"], "work/sessions/c3.md");
        let text = fs::read_to_string(vault.join("work/sessions/c3.md")).unwrap();
        assert!(text.contains("type: session-summary"));
        assert!(text.contains("codeg_source_id:") || text.contains("source_ids:"));
        assert!(!text.contains("[[raw/sessions/src-turn-1]]"));
        let contributions = wiki_service::list_contributions_for_source(&db.conn, &sid)
            .await
            .unwrap();
        assert_eq!(contributions.len(), 1);
        let result: JobOutputManifest = serde_json::from_value(out).unwrap();
        register_memory_contributions(&db.conn, &job, &result)
            .await
            .unwrap();
        assert_eq!(
            wiki_service::list_contributions_for_source(&db.conn, &sid)
                .await
                .unwrap()
                .len(),
            1,
            "recovery must not duplicate source references"
        );
    }

    #[tokio::test]
    async fn success_writes_one_turn_page_and_is_idempotent_on_hash() {
        let (_dir, db, vault, state, sid, job, _) = fixture().await;
        let llm = fixture_llm(&vault);
        let out = run_turn_summary_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap();
        let page = vault.join("work/turns/src-turn-1.md");
        assert!(page.is_file());
        let text = fs::read_to_string(&page).unwrap();
        assert!(text.contains("type: turn-summary"));
        assert!(text.contains("codeg_source_id: \"src-turn-1\""));
        assert!(!text.contains("ACP turn:"));
        assert_eq!(out["outputs"][0]["path"], "work/turns/src-turn-1.md");
        assert_eq!(out["outputs"][0]["content_hash"], raw::content_hash(&text));
        let src = wiki_service::get_source_model(&db.conn, &sid)
            .await
            .unwrap();
        let again = enqueue_for_source(
            &db.conn,
            &src.vault_id,
            &sid,
            src.raw_hash.as_deref().unwrap(),
            src.conversation_id,
            src.raw_path.as_deref().unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(again.id, job.id);
    }
}

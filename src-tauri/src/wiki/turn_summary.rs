//! Host-wrapped ACP turn memory pages (`work/turns/{source_id}.md`).

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
use crate::wiki::llm::{WikiLlm, WIKI_TURN_SUMMARY_MAX_TURNS};
use crate::wiki::raw;
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
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

pub fn wrap_memory_page(fields: MemoryPageFields<'_>) -> String {
    let mut yaml = String::from("---\n");
    yaml.push_str("title: ");
    yaml.push_str(&yaml_quote(fields.title));
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
        yaml.push_str("sources:\n  - ");
        yaml.push_str(&yaml_quote(&format!("[[sources/{sid}]]")));
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
        .ok_or_else(|| WorkerError::Failed("turn_summary job has no source_id".into()))?;
    let source = wiki_service::get_source_model(conn, source_id).await?;
    let Some(raw_rel) = source.raw_path.as_deref().filter(|s| !s.is_empty()) else {
        return Err(WorkerError::Failed(
            "turn_summary source has no frozen raw".into(),
        ));
    };
    let abs = vault.join(raw_rel);
    if !abs.is_file() {
        return Err(WorkerError::Failed(format!(
            "raw file missing: {}",
            abs.display()
        )));
    }
    let raw_hash = source.raw_hash.clone().unwrap_or_default();
    let rel = page_rel(&source.id);
    let staging = state_root.join("staging").join(&job.id);
    fs::create_dir_all(&staging).map_err(|e| WorkerError::Failed(e.to_string()))?;

    let input = json!({
        "schema": "codeg.wiki.turn_summary.v1",
        "source_id": source.id,
        "source_kind": source.source_kind,
        "raw_path": raw_rel,
        "raw_hash": raw_hash,
        "conversation_id": source.conversation_id,
        "rel": rel,
        "vault_abs": vault.to_string_lossy(),
        "staging_abs": staging.to_string_lossy(),
        "max_turns": WIKI_TURN_SUMMARY_MAX_TURNS,
        "instruction": "Read the converted markdown at raw_path with read_file. Return JSON title+body. Do not emit YAML front matter. Do not write work/capability pages.",
    });
    let out = llm.complete_json(KIND, input).await.map_err(|e| match e {
        crate::wiki::llm::WikiLlmError::Blocked(s) => WorkerError::Blocked(s),
        crate::wiki::llm::WikiLlmError::Failed(s) => WorkerError::Failed(s),
    })?;
    let parsed = validate_turn_summary(&out, &source.id)?;
    if parsed.nothing_to_summarize {
        let output = json!({
            "rel": rel,
            "nothing_to_summarize": true,
            "warnings": parsed.warnings,
            "raw_preserved": true,
        });
        wiki_service::set_job_output_manifest(
            conn,
            &job.id,
            &serde_json::to_string(&output).unwrap_or_else(|_| "{}".into()),
        )
        .await?;
        let _ = raw::append_log_idempotent(
            &vault.join("log.md"),
            &job.id,
            &format!(
                "{} turn_summary nothing_to_summarize job={} source={}",
                Utc::now().to_rfc3339(),
                job.id,
                source.id
            ),
        );
        return Ok(output);
    }

    let dest = vault.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let existing = fs::read_to_string(&dest).ok();
    let note_id = existing
        .as_deref()
        .and_then(|t| yaml_string(t, "codeg_note_id"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let before_hash = existing
        .as_deref()
        .map(crate::wiki::raw::content_hash)
        .unwrap_or_default();
    let title = sanitize_turn_title(&parsed.title, &source.id);
    let binding = first_project_id(&source.project_ids);
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
    });
    if let Err(e) = check_leaf_body(PAGE_TYPE, &after) {
        return Err(WorkerError::Compile(e));
    }
    let proposal = StagedProposal {
        rel: rel.clone(),
        page_type: PAGE_TYPE.into(),
        before_hash,
        after: after.clone(),
        op: if dest.exists() {
            "update".into()
        } else {
            "create".into()
        },
    };
    commit::commit_proposals(vault, state_root, &job.id, &[proposal])
        .map_err(CompileError::from)?;

    let summary = first_body_paragraph(&after);
    let _ = wiki_service::fill_source_title_if_empty(conn, &source.id, &title).await;
    wiki_service::insert_contribution(
        conn,
        &source.id,
        &raw_hash,
        source.annotation_revision,
        &note_id,
        Some(&job.id),
    )
    .await?;

    let output = json!({
        "rel": rel,
        "codeg_note_id": note_id,
        "title": title,
        "summary": summary,
        "nothing_to_summarize": false,
        "warnings": parsed.warnings,
        "raw_preserved": true,
    });
    wiki_service::set_job_output_manifest(
        conn,
        &job.id,
        &serde_json::to_string(&output).unwrap_or_else(|_| "{}".into()),
    )
    .await?;
    let _ = raw::append_log_idempotent(
        &vault.join("log.md"),
        &job.id,
        &format!(
            "{} turn_summary succeeded job={} source={} rel={}",
            Utc::now().to_rfc3339(),
            job.id,
            source.id,
            rel
        ),
    );
    if !abs.is_file() {
        return Err(WorkerError::Failed(
            "raw was deleted during turn_summary".into(),
        ));
    }
    Ok(output)
}

struct ParsedTurn {
    title: String,
    body: String,
    nothing_to_summarize: bool,
    warnings: Vec<String>,
}

fn validate_turn_summary(v: &Value, source_id: &str) -> Result<ParsedTurn, WorkerError> {
    let obj = v
        .as_object()
        .ok_or_else(|| WorkerError::Failed("turn_summary is not an object".into()))?;
    if let Some(echo) = obj.get("source_id").and_then(|x| x.as_str()) {
        if !echo.is_empty() && echo != source_id {
            return Err(WorkerError::Failed(
                "turn_summary JSON source_id does not match the frozen source".into(),
            ));
        }
    }
    let nothing = obj
        .get("nothing_to_summarize")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let warnings = obj
        .get("warnings")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let title = obj
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let body = obj
        .get("body")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !nothing && (title.is_empty() || body.is_empty()) {
        return Err(WorkerError::Failed(
            "turn_summary JSON missing title/body".into(),
        ));
    }
    Ok(ParsedTurn {
        title,
        body,
        nothing_to_summarize: nothing,
        warnings,
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
        });
        assert!(page.contains("type: turn-summary"));
        assert!(page.contains("codeg_source_id: \"src-1\""));
        assert!(page.contains("codeg_conversation_id: 9"));
        assert!(page.contains(CONTENT_START));
        assert!(page.contains("Edited list.rs."));
        assert!(!page.contains("ACP turn:"));
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
    async fn nothing_to_summarize_succeeds_without_page() {
        let (_dir, db, vault, state, _sid, job, abs) = fixture().await;
        let llm = MockWikiLlm::default().with_stage(
            KIND,
            json!({
                "schema": "codeg.wiki.turn_summary.v1",
                "source_id": "src-turn-1",
                "title": "",
                "body": "",
                "nothing_to_summarize": true,
                "warnings": ["empty"]
            }),
        );
        let out = run_turn_summary_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap();
        assert_eq!(out["nothing_to_summarize"], true);
        assert!(abs.is_file());
        assert!(!vault.join("work/turns/src-turn-1.md").exists());
    }

    #[tokio::test]
    async fn success_writes_one_turn_page_and_is_idempotent_on_hash() {
        let (_dir, db, vault, state, sid, job, _) = fixture().await;
        let llm = MockWikiLlm::default();
        let out = run_turn_summary_job(&db.conn, &job, &llm, &vault, &state)
            .await
            .unwrap();
        let page = vault.join("work/turns/src-turn-1.md");
        assert!(page.is_file());
        let text = fs::read_to_string(&page).unwrap();
        assert!(text.contains("type: turn-summary"));
        assert!(text.contains("codeg_source_id: \"src-turn-1\""));
        assert!(!text.contains("ACP turn:"));
        assert_eq!(out["rel"], "work/turns/src-turn-1.md");
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

//! Run turn_summary, session_rollup, and wiki_synthesize attempts.

use std::fs;
use std::path::Path;

use sea_orm::DatabaseConnection;
use serde_json::{json, Value};

use crate::db::entities::wiki_job;
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::compile::{self, CompileError};
use crate::wiki::llm::{parse_json_object, WikiLlm, WikiLlmError};
use crate::wiki::raw;
use crate::wiki::session_rollup;
use crate::wiki::turn_summary;

const INGEST_MAX_TURNS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("{0}")]
    Failed(String),
    #[error("blocked-by-configuration: {0}")]
    Blocked(String),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl WorkerError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Failed(_) => "worker_failed",
            Self::Blocked(_) => crate::wiki::llm::BLOCKED_BY_CONFIGURATION,
            Self::Compile(e) => e.error_code(),
            Self::Db(_) => "database",
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Failed(_) | Self::Db(_) => true,
            Self::Blocked(_) => false,
            Self::Compile(e) => e.retryable(),
        }
    }
}

pub async fn run_turn_summary(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    turn_summary::run_turn_summary_job(conn, job, llm, vault, state_root).await?;
    Ok(())
}

pub async fn run_session_rollup(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    session_rollup::run_session_rollup_job(conn, job, llm, vault, state_root).await?;
    Ok(())
}

/// Legacy ingest helper kept for existing tests. Production uses turn_summary.
pub async fn run_ingest_summary(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: Option<&dyn WikiLlm>,
    vault: &Path,
) -> Result<serde_json::Value, WorkerError> {
    let source_id = job
        .source_id
        .as_deref()
        .ok_or_else(|| WorkerError::Failed("ingest job has no source_id".into()))?;
    let source = wiki_service::get_source_model(conn, source_id).await?;
    let Some(raw_rel) = source.raw_path.as_deref().filter(|s| !s.is_empty()) else {
        return Err(WorkerError::Failed(
            "ingest source has no frozen raw".into(),
        ));
    };
    let abs = vault.join(raw_rel);
    if !abs.is_file() {
        return Err(WorkerError::Failed(format!(
            "raw file missing: {}",
            abs.display()
        )));
    }
    let _raw_text = fs::read_to_string(&abs).map_err(|e| WorkerError::Failed(e.to_string()))?;

    let mut output = json!({
        "raw_preserved": true,
        "raw_path": raw_rel,
        "source_id": source.id,
        "ingest_max_turns": INGEST_MAX_TURNS,
    });
    let mut warnings: Vec<String> = Vec::new();

    match llm {
        None => {
            warnings.push("no model bound; source summary skipped".into());
        }
        Some(llm) => {
            let input = ingest_llm_input(
                &source.id,
                &source.source_kind,
                raw_rel,
                &vault.to_string_lossy(),
            );
            match llm.complete_json("ingest", input).await {
                Ok(v) => match validate_ingest_summary(&v, &source.id) {
                    Ok(summary) => {
                        output
                            .as_object_mut()
                            .unwrap()
                            .insert("summary".into(), summary);
                    }
                    Err(w) => warnings.push(w),
                },
                Err(WikiLlmError::Blocked(s)) => {
                    warnings.push(format!("summary skipped: {s}"));
                }
                Err(WikiLlmError::Failed(s)) => {
                    warnings.push(format!("summary failed: {s}"));
                }
            }
        }
    }

    output
        .as_object_mut()
        .unwrap()
        .insert("warnings".into(), json!(warnings));

    if !abs.is_file() {
        return Err(WorkerError::Failed("raw was deleted during ingest".into()));
    }

    wiki_service::set_job_output_manifest(
        conn,
        &job.id,
        &serde_json::to_string(&output).unwrap_or_else(|_| "{}".into()),
    )
    .await?;
    if let Some(summary) = output
        .pointer("/summary/source_summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let _ = wiki_service::fill_source_title_if_empty(conn, &source.id, summary).await;
    }

    let log_line = format!(
        "{} ingest succeeded job={} source={}",
        chrono::Utc::now().to_rfc3339(),
        job.id,
        source.id
    );
    let _ = raw::append_log_idempotent(&vault.join("log.md"), &job.id, &log_line);

    Ok(output)
}

fn validate_ingest_summary(v: &Value, source_id: &str) -> Result<Value, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| "ingest summary is not an object".to_string())?;
    let nothing = obj
        .get("nothing_to_summarize")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let summary = obj
        .get("source_summary")
        .or_else(|| obj.get("summary"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if !nothing && summary.is_none() {
        return Err("ingest JSON missing source_summary".into());
    }
    if let Some(echo) = obj.get("source_id").and_then(|x| x.as_str()) {
        if !echo.is_empty() && echo != source_id {
            return Err("ingest JSON source_id does not match the frozen source".into());
        }
    }
    let topics = obj
        .get("topic_suggestions")
        .or_else(|| obj.get("topics"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    Ok(json!({
        "source_summary": summary.unwrap_or(""),
        "topic_suggestions": topics,
        "nothing_to_summarize": nothing,
    }))
}

pub(crate) fn ingest_llm_input(
    source_id: &str,
    source_kind: &str,
    raw_path: &str,
    vault_abs: &str,
) -> Value {
    json!({
        "source_id": source_id,
        "source_kind": source_kind,
        "raw_path": raw_path,
        "vault_abs": vault_abs,
        "instruction": "Read the converted markdown at raw_path with read_file. Do not wait for an embedded excerpt. Return a one-line source_summary.",
    })
}

pub async fn run_compile_attempt(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    compile::run_compile_job(conn, job, llm, vault, state_root).await?;
    Ok(())
}

/// Parse a model JSON blob; used by tests that feed raw completions.
pub fn parse_model_json(text: &str) -> Result<Value, WikiLlmError> {
    parse_json_object(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::llm::MockWikiLlm;
    use crate::wiki::vault;
    use chrono::Utc;
    use sea_orm::{ActiveModelTrait, Set};
    use tempfile::tempdir;

    #[test]
    fn ingest_payload_points_at_raw_file_not_clipped_body() {
        let v = ingest_llm_input("sid", "acp-turn", "raw/sessions/sid.md", "/tmp/wiki-vault");
        assert_eq!(v["raw_path"], "raw/sessions/sid.md");
        assert_eq!(v["source_id"], "sid");
        assert!(v.get("redacted_text").is_none());
        assert!(v.get("excerpt").is_none());
    }

    #[tokio::test]
    async fn ingest_summary_failure_does_not_delete_raw() {
        let dir = tempdir().unwrap();
        let vault_path = dir.path().join("vault");
        vault::initialize_vault(&vault_path).unwrap();
        let db = fresh_in_memory_db().await;
        let now = Utc::now();
        let v = wiki_vault::ActiveModel {
            id: Set("v1".into()),
            canonical_path: Set(vault_path.to_string_lossy().into_owned()),
            config_revision: Set(0),
            next_compile_at: Set(None),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let source_id = "src-raw-1";
        let rel = format!("raw/sessions/{source_id}.md");
        let abs = vault_path.join(&rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, "---\ntitle: t\n---\n\nhello raw\n").unwrap();
        wiki_source::ActiveModel {
            id: Set(source_id.into()),
            source_group_id: Set(source_id.into()),
            vault_id: Set(v.id.clone()),
            source_kind: Set("acp-turn".into()),
            source_seq: Set(1),
            run_id: Set(Some("run".into())),
            original_hash: Set(None),
            raw_path: Set(Some(rel)),
            raw_hash: Set(Some("abc".into())),
            extractor_version: Set(None),
            coverage_status: Set(None),
            eligibility: Set("ready".into()),
            material_role: Set(None),
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
            id: Set("job-ing".into()),
            vault_id: Set(v.id),
            source_id: Set(Some(source_id.into())),
            kind: Set("turn_summary".into()),
            status: Set("running".into()),
            dedupe_key: Set(None),
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

        let llm = MockWikiLlm::failing();
        let out = run_ingest_summary(&db.conn, &job, Some(&llm), &vault_path)
            .await
            .unwrap();
        assert!(abs.is_file(), "raw must survive summary failure");
        assert_eq!(
            fs::read_to_string(&abs).unwrap().contains("hello raw"),
            true
        );
        let warnings = out["warnings"].as_array().unwrap();
        assert!(!warnings.is_empty());
        let src = wiki_service::get_source_model(&db.conn, source_id)
            .await
            .unwrap();
        assert_eq!(src.eligibility, "ready");
    }
}

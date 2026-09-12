//! Host ingest of an ACP turn snapshot: filter → redact → source/job → raw.

use sea_orm::DatabaseConnection;

use crate::db::error::DbError;
use crate::db::service::wiki_service::{InsertedSource, NewAcpSource};
use crate::db::service::{conversation_service, folder_service, wiki_service};
use crate::wiki::filter::{self, FilterContext};
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::raw::{self, RawSessionMeta, RawWriteOutcome};
use crate::wiki::redact;
use crate::wiki::settings;
use crate::wiki::snapshot::WikiTurnSnapshot;
use crate::wiki::vault;

#[derive(Debug, Clone)]
pub struct PersistOutcome {
    pub source_id: String,
    pub job_id: String,
    pub created: bool,
    pub skipped: Option<String>,
}

/// Persist a frozen ACP snapshot. Never reads `session_store`. No model calls.
pub async fn persist_acp_turn(
    conn: &DatabaseConnection,
    mut snap: WikiTurnSnapshot,
) -> Result<PersistOutcome, DbError> {
    let settings = settings::load_settings(conn).await?;
    let (kind, folder_id, root_folder_id, folder_path, git_branch, conv_model) =
        enrich_from_db(conn, &snap).await?;

    let ctx = FilterContext {
        settings: settings.clone(),
        conversation_kind: kind,
        folder_id,
        root_folder_id,
    };
    if let Some(reason) = filter::evaluate(&snap, &ctx) {
        tracing::info!(
            run_id = %snap.run_id,
            reason = reason.as_str(),
            "[wiki] skip ACP persist"
        );
        return Ok(PersistOutcome {
            source_id: String::new(),
            job_id: String::new(),
            created: false,
            skipped: Some(reason.as_str().to_string()),
        });
    }

    let redacted = redact::redact_snapshot(&mut snap);
    let truncated = snap.user_truncated || snap.assistant_truncated || snap.tool_dropped_count > 0;

    let mut model = snap.model.clone();
    let mut model_fallback = false;
    if model.is_none() {
        if let Some(m) = conv_model {
            snap.model = Some(m.clone());
            model = Some(m);
            model_fallback = true;
        }
    }

    let vault_path = resolve_vault_path(settings.vault_path.as_deref());
    let state_root = resolve_state_root(settings.vault_path.as_deref());
    vault::initialize_vault(&vault_path).map_err(DbError::from)?;
    vault::initialize_state_root(&state_root).map_err(DbError::from)?;
    let canonical = vault_path.to_string_lossy().to_string();
    let vault_row = wiki_service::ensure_active_vault(conn, &canonical).await?;
    let _ = settings::ensure_db_instance_id(conn).await?;

    let inserted = wiki_service::insert_acp_source_and_ingest_job(
        conn,
        NewAcpSource {
            vault_id: vault_row.id.clone(),
            run_id: snap.run_id.clone(),
            conversation_id: snap.conversation_id,
            folder_id,
            root_folder_id,
            agent_type: Some(snap.agent_type.clone()),
            model: model.clone(),
            mode: snap.mode.clone(),
            captured_at: snap.captured_at,
            occurred_at: Some(snap.occurred_at),
            truncated,
            redacted,
        },
    )
    .await?;

    if !inserted.created {
        return Ok(PersistOutcome {
            source_id: inserted.source.id,
            job_id: inserted.job.id,
            created: false,
            skipped: None,
        });
    }

    let meta = RawSessionMeta {
        source_id: &inserted.source.id,
        source_group_id: &inserted.source.source_group_id,
        captured_at: snap.captured_at,
        occurred_at: Some(snap.occurred_at),
        redacted,
        folder_path: folder_path.as_deref(),
        git_branch: git_branch.as_deref(),
        root_folder_id,
        model_fallback,
    };
    if let Err(e) = freeze_raw_and_log(conn, &vault_path, &snap, &inserted, meta).await {
        let msg = e.to_string();
        let _ = wiki_service::mark_job(
            conn,
            &inserted.job.id,
            "failed",
            Some("raw_write"),
            Some(&msg),
        )
        .await;
        let _ = wiki_service::mark_source_raw(conn, &inserted.source.id, "", "", "failed").await;
        return Err(e);
    }

    Ok(PersistOutcome {
        source_id: inserted.source.id,
        job_id: inserted.job.id,
        created: true,
        skipped: None,
    })
}

async fn freeze_raw_and_log(
    conn: &DatabaseConnection,
    vault_path: &std::path::Path,
    snap: &WikiTurnSnapshot,
    inserted: &InsertedSource,
    meta: RawSessionMeta<'_>,
) -> Result<(), DbError> {
    wiki_service::mark_job(conn, &inserted.job.id, "running", None, None).await?;
    let (doc, hash) = raw::render_session_raw(snap, &meta);
    let path = raw::raw_session_path(vault_path, &inserted.source.id);
    match raw::write_raw_exclusive(&path, &doc, &hash).map_err(DbError::from)? {
        RawWriteOutcome::Created { .. } | RawWriteOutcome::Identical { .. } => {}
        RawWriteOutcome::Conflict {
            existing_hash,
            new_hash,
            ..
        } => {
            return Err(DbError::Conflict(format!(
                "raw path exists with hash {existing_hash}, new hash {new_hash}"
            )));
        }
    }
    let rel = format!("raw/sessions/{}.md", inserted.source.id);
    wiki_service::mark_source_raw(conn, &inserted.source.id, &rel, &hash, "ready").await?;
    wiki_service::mark_job(conn, &inserted.job.id, "succeeded", None, None).await?;
    let log_line = format!(
        "{} ingest succeeded job={} source={} run={}",
        chrono::Utc::now().to_rfc3339(),
        inserted.job.id,
        inserted.source.id,
        snap.run_id
    );
    raw::append_log_idempotent(&vault_path.join("log.md"), &inserted.job.id, &log_line)
        .map_err(DbError::from)?;
    Ok(())
}

async fn enrich_from_db(
    conn: &DatabaseConnection,
    snap: &WikiTurnSnapshot,
) -> Result<
    (
        Option<crate::db::entities::conversation::ConversationKind>,
        Option<i32>,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    DbError,
> {
    let Some(cid) = snap.conversation_id else {
        return Ok((None, snap.folder_id, None, None, None, None));
    };
    let conv = match conversation_service::get_by_id(conn, cid).await {
        Ok(c) => c,
        Err(_) => return Ok((None, snap.folder_id, None, None, None, None)),
    };
    let folder_id = snap.folder_id.or(Some(conv.folder_id));
    let mut root_folder_id = folder_id;
    let mut folder_path = None;
    if let Some(fid) = folder_id {
        if let Ok(Some(folder)) = folder_service::get_folder_by_id(conn, fid).await {
            folder_path = Some(folder.path);
            root_folder_id = Some(folder.parent_id.unwrap_or(fid));
        }
    }
    Ok((
        Some(conv.kind),
        folder_id,
        root_folder_id,
        folder_path,
        conv.git_branch,
        conv.model,
    ))
}

/// Off-hot-path enqueue. Failures do not change conversation success.
pub fn enqueue_persist(conn: DatabaseConnection, snap: WikiTurnSnapshot) {
    tokio::spawn(async move {
        match persist_acp_turn(&conn, snap).await {
            Ok(out) => {
                if let Some(reason) = out.skipped {
                    tracing::info!(reason = %reason, "[wiki] persist skipped");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "[wiki] persist failed");
                if let Err(e2) = wiki_service::insert_failed_job(
                    &conn,
                    None,
                    None,
                    "persist_failed",
                    &e.to_string(),
                )
                .await
                {
                    tracing::warn!(error = %e2, "[wiki] failed to record wiki_job");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::conversation::{self, ConversationKind};
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::AgentType;
    use crate::wiki::settings::WikiSettings;
    use crate::wiki::snapshot::{WikiToolObservation, WikiTurnSnapshot};
    use chrono::Utc;
    use sea_orm::{ActiveModelTrait, NotSet, Set};
    use tempfile::tempdir;

    fn snap_for(run_id: &str, conversation_id: i32, folder_id: i32) -> WikiTurnSnapshot {
        WikiTurnSnapshot {
            run_id: run_id.into(),
            connection_id: "conn-1".into(),
            conversation_id: Some(conversation_id),
            agent_type: "claude_code".into(),
            working_dir: Some("/tmp/proj".into()),
            folder_id: Some(folder_id),
            model: Some("sonnet".into()),
            mode: None,
            occurred_at: Utc::now(),
            captured_at: Utc::now(),
            user_text: "please fix the bug".into(),
            assistant_text: "patched src/lib.rs".into(),
            user_original_chars: 19,
            assistant_original_chars: 16,
            user_truncated: false,
            assistant_truncated: false,
            tool_observations: vec![WikiToolObservation {
                id: "t1".into(),
                path: Some("src/lib.rs".into()),
                kind: "edit".into(),
                status: "completed".into(),
                summary: "ok".into(),
            }],
            tool_dropped_count: 0,
            file_changes: vec![],
        }
    }

    async fn enable_wiki(conn: &DatabaseConnection, vault: &std::path::Path) {
        let mut s = WikiSettings::default();
        s.enabled = true;
        s.vault_path = Some(vault.to_string_lossy().to_string());
        settings::save_settings(conn, &s).await.unwrap();
    }

    #[tokio::test]
    async fn persist_disabled_settings_inserts_no_row() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-disabled").await;
        let cid = seed_conversation(&db, folder, AgentType::ClaudeCode).await;
        let dir = tempdir().unwrap();
        let mut s = WikiSettings::default();
        s.enabled = false;
        s.vault_path = Some(dir.path().to_string_lossy().to_string());
        settings::save_settings(&db.conn, &s).await.unwrap();
        let out = persist_acp_turn(&db.conn, snap_for("run-d", cid, folder))
            .await
            .unwrap();
        assert_eq!(out.skipped.as_deref(), Some("wiki_disabled"));
        let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
            .await
            .unwrap();
        assert!(sources.is_empty());
    }

    #[tokio::test]
    async fn persist_delegate_kind_skipped() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-delegate").await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        let now = Utc::now();
        let agent = serde_json::to_value(AgentType::ClaudeCode)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let child = conversation::ActiveModel {
            id: NotSet,
            folder_id: Set(folder),
            title: Set(Some("child".into())),
            title_locked: Set(false),
            agent_type: Set(agent),
            status: Set(conversation::ConversationStatus::InProgress),
            kind: Set(ConversationKind::Delegate),
            model: Set(None),
            git_branch: Set(None),
            external_id: Set(None),
            parent_id: Set(Some(1)),
            parent_tool_use_id: Set(Some("tc".into())),
            delegation_call_id: Set(Some("call".into())),
            message_count: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
            pinned_at: Set(None),
            origin_cwd: Set(None),
        };
        let inserted = child.insert(&db.conn).await.unwrap();
        let out = persist_acp_turn(&db.conn, snap_for("run-del", inserted.id, folder))
            .await
            .unwrap();
        assert_eq!(out.skipped.as_deref(), Some("conversation_kind_delegate"));
        let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
            .await
            .unwrap();
        assert!(sources.is_empty());
    }

    #[tokio::test]
    async fn persist_vault_run_unique_second_returns_same_source() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-unique").await;
        let cid = seed_conversation(&db, folder, AgentType::ClaudeCode).await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        let first = persist_acp_turn(&db.conn, snap_for("run-u", cid, folder))
            .await
            .unwrap();
        assert!(first.created);
        let second = persist_acp_turn(&db.conn, snap_for("run-u", cid, folder))
            .await
            .unwrap();
        assert!(!second.created);
        assert_eq!(first.source_id, second.source_id);
        let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
            .await
            .unwrap();
        assert_eq!(sources.len(), 1);
    }

    #[tokio::test]
    async fn persist_redacts_secrets_in_raw_file() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-redact").await;
        let cid = seed_conversation(&db, folder, AgentType::ClaudeCode).await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        let mut snap = snap_for("run-sec", cid, folder);
        snap.user_text = "use API_KEY=sk-live-999 and https://u:p@host/repo".into();
        snap.assistant_text = "Authorization: Bearer tok_abc Cookie: sid=1".into();
        let out = persist_acp_turn(&db.conn, snap).await.unwrap();
        assert!(out.created);
        let src = wiki_service::get_source(&db.conn, &out.source_id)
            .await
            .unwrap();
        assert!(src.redacted);
        let path = dir.path().join(src.raw_path.as_ref().unwrap());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("sk-live-999"));
        assert!(!raw.contains("tok_abc"));
        assert!(!raw.contains("u:p@"));
        assert!(raw.contains("[REDACTED]"));
    }
}

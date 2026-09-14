//! compile-now / job retry / cancel. Separate from `commands/wiki.rs`.

use sea_orm::DatabaseConnection;

use crate::app_error::AppCommandError;
use crate::db::error::DbError;
use crate::db::service::wiki_service::{self, WikiJobInfo};
use crate::wiki::compile;
use crate::wiki::engine;
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::settings;
use crate::wiki::vault;

#[cfg(feature = "tauri-runtime")]
use crate::db::AppDatabase;

pub async fn wiki_compile_now_core(
    conn: &DatabaseConnection,
    request_id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    let request_id = request_id.trim().to_string();
    if request_id.is_empty() {
        return Err(AppCommandError::invalid_input("request_id is required"));
    }
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    if !settings.enabled {
        return Err(AppCommandError::configuration_invalid("wiki is disabled"));
    }
    if !settings.synthesize.enabled {
        return Err(AppCommandError::configuration_invalid(
            "synthesize is disabled",
        ));
    }
    let vault = resolve_vault_path(settings.vault_path.as_deref());
    vault::initialize_vault(&vault).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    vault::initialize_state_root(&resolve_state_root(settings.vault_path.as_deref()))
        .map_err(|e| AppCommandError::io_error(e.to_string()))?;
    let canonical = vault.to_string_lossy().to_string();
    let vault_row = wiki_service::ensure_active_vault(conn, &canonical)
        .await
        .map_err(AppCommandError::from)?;
    let cutoff = wiki_service::max_source_seq(conn, &vault_row.id)
        .await
        .map_err(AppCommandError::from)?
        .unwrap_or(0);
    let manifest = compile::freeze_manifest(conn, &vault_row.id, cutoff, &vault)
        .await
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let json = serde_json::to_string(&manifest)
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let dedupe = format!("wiki_synthesize:now:{request_id}");
    let job = wiki_service::insert_compile_job(conn, &vault_row.id, &dedupe, Some(&json))
        .await
        .map_err(AppCommandError::from)?;
    engine::notify_jobs();
    wiki_service::get_job(conn, &job.id)
        .await
        .map_err(AppCommandError::from)
}

pub async fn wiki_retry_job_core(
    conn: &DatabaseConnection,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    let job = wiki_service::retry_job(conn, &id).await.map_err(map_db)?;
    engine::clear_cancellation(&job.id);
    engine::notify_jobs();
    wiki_service::get_job(conn, &job.id)
        .await
        .map_err(AppCommandError::from)
}

pub async fn wiki_cancel_job_core(
    conn: &DatabaseConnection,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    let was_running = wiki_service::get_job_model(conn, &id)
        .await
        .map(|j| j.status == "running")
        .unwrap_or(false);
    let job = wiki_service::cancel_job(conn, &id).await.map_err(map_db)?;
    // Wake an in-flight Worker so it can abandon the attempt before commit.
    if was_running {
        engine::request_cancel(&job.id);
    }
    engine::notify_jobs();
    wiki_service::get_job(conn, &job.id)
        .await
        .map_err(AppCommandError::from)
}

fn map_db(err: DbError) -> AppCommandError {
    match err {
        DbError::NotFound(s) => AppCommandError::not_found(s),
        DbError::Validation(s) => AppCommandError::invalid_input(s),
        other => AppCommandError::from(other),
    }
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_compile_now(
    db: tauri::State<'_, AppDatabase>,
    request_id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    wiki_compile_now_core(&db.conn, request_id).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_retry_job(
    db: tauri::State<'_, AppDatabase>,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    wiki_retry_job_core(&db.conn, id).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_cancel_job(
    db: tauri::State<'_, AppDatabase>,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    wiki_cancel_job_core(&db.conn, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::settings::WikiSettings;
    use tempfile::tempdir;

    #[tokio::test]
    async fn compile_now_is_idempotent_on_request_id() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        let mut s = WikiSettings::default();
        s.enabled = true;
        s.synthesize.enabled = true;
        s.vault_path = Some(dir.path().join("vault").to_string_lossy().into_owned());
        settings::save_settings(&db.conn, &s).await.unwrap();
        let a = wiki_compile_now_core(&db.conn, "req-abc".into())
            .await
            .unwrap();
        let b = wiki_compile_now_core(&db.conn, "req-abc".into())
            .await
            .unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, "wiki_synthesize");
        let c = wiki_compile_now_core(&db.conn, "req-other".into())
            .await
            .unwrap();
        assert_ne!(a.id, c.id);
    }

    #[tokio::test]
    async fn compile_now_respects_compile_enabled() {
        let db = fresh_in_memory_db().await;
        let mut s = WikiSettings::default();
        s.enabled = true;
        s.synthesize.enabled = false;
        settings::save_settings(&db.conn, &s).await.unwrap();
        let err = wiki_compile_now_core(&db.conn, "r1".into())
            .await
            .unwrap_err();
        assert!(err.message.contains("synthesize is disabled"));
    }
}

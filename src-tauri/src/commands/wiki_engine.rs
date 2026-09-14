//! 把立即整理、重试和取消操作连接到 Wiki 后台调度器。
//! 共享 core 用例由桌面与 HTTP 调用，负责请求去重、任务状态变更及唤醒 engine。
//! 本层不运行模型；返回任务详情时由读模型补充当前产物状态。

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
    let _transition = crate::wiki::lifecycle::lock().await;
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
    let vault = resolve_vault_path(settings.vault_path.as_deref());
    vault::initialize_vault(&vault).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    vault::initialize_state_root(&resolve_state_root())
        .map_err(|e| AppCommandError::io_error(e.to_string()))?;
    let canonical = vault.to_string_lossy().to_string();
    let vault_row = wiki_service::ensure_active_vault(conn, &canonical)
        .await
        .map_err(AppCommandError::from)?;
    let manifest = compile::list_pending_inputs(conn, &vault_row.id, &vault)
        .await
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let json = serde_json::to_string(&manifest)
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let dedupe = format!("wiki_synthesize:now:{request_id}");
    let job = wiki_service::insert_compile_job(conn, &vault_row.id, &dedupe, Some(&json))
        .await
        .map_err(AppCommandError::from)?;
    engine::notify_jobs();
    crate::wiki::read_model::read_job(conn, &job.id)
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
    crate::wiki::read_model::read_job(conn, &job.id)
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
    crate::wiki::read_model::read_job(conn, &job.id)
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
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
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
    use crate::wiki::settings::{WikiSettings, WikiSynthesizeSettings};
    use tempfile::tempdir;

    #[tokio::test]
    async fn compile_now_is_idempotent_on_request_id() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        let s = WikiSettings {
            enabled: true,
            vault_path: Some(dir.path().join("vault").to_string_lossy().into_owned()),
            ..WikiSettings::default()
        };
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
    async fn compile_now_is_allowed_when_automatic_synthesis_is_disabled() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        let settings = WikiSettings {
            enabled: true,
            vault_path: Some(dir.path().join("vault").to_string_lossy().into_owned()),
            synthesize: WikiSynthesizeSettings {
                enabled: false,
                ..WikiSynthesizeSettings::default()
            },
            ..WikiSettings::default()
        };
        settings::save_settings(&db.conn, &settings).await.unwrap();
        let job = wiki_compile_now_core(&db.conn, "manual-without-schedule".into())
            .await
            .unwrap();
        assert_eq!(job.kind, "wiki_synthesize");
        assert_eq!(job.status, "queued");
    }

    #[tokio::test]
    async fn compile_now_rejects_disabled_wiki() {
        let db = fresh_in_memory_db().await;
        let error = wiki_compile_now_core(&db.conn, "disabled-wiki".into())
            .await
            .unwrap_err();
        assert!(error.message.contains("wiki is disabled"));
    }
}

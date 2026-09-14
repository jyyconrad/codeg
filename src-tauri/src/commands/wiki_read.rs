//! 个人 Wiki 阅读与目录刷新命令的 Tauri 适配层。
//! 将桌面参数转交 wiki/read_model 或 library，并统一映射业务错误。
//! HTTP 使用同一业务入口；正文解析、分页和文件存在性不在适配器重复实现。

use crate::app_error::AppCommandError;

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_refresh_library(
    db: tauri::State<'_, crate::db::AppDatabase>,
) -> Result<crate::wiki::library::WikiLibrary, AppCommandError> {
    crate::wiki::library::refresh(&db.conn)
        .await
        .map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
use crate::wiki::read_model::*;
#[cfg(feature = "tauri-runtime")]
use crate::{db::AppDatabase, wiki::read_model};

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_get_overview(
    db: tauri::State<'_, AppDatabase>,
) -> Result<WikiOverview, AppCommandError> {
    read_model::get_overview(&db.conn).await.map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_list_notes(
    db: tauri::State<'_, AppDatabase>,
    query: WikiNoteQuery,
) -> Result<WikiPage<WikiNoteSummary>, AppCommandError> {
    read_model::list_notes(&db.conn, query)
        .await
        .map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_read_note(
    db: tauri::State<'_, AppDatabase>,
    path: Option<String>,
    note_id: Option<String>,
) -> Result<WikiNoteDetail, AppCommandError> {
    read_model::read_note(&db.conn, path, note_id)
        .await
        .map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_read_source_document(
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
) -> Result<WikiSourceDocument, AppCommandError> {
    read_model::read_source_document(&db.conn, &source_id)
        .await
        .map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_list_jobs_page(
    db: tauri::State<'_, AppDatabase>,
    status: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<WikiPage<crate::db::service::wiki_service::WikiJobInfo>, AppCommandError> {
    read_model::list_jobs(&db.conn, status, offset, limit)
        .await
        .map_err(map_error)
}

#[cfg(feature = "tauri-runtime")]
#[tauri::command]
pub async fn wiki_list_sources_page(
    db: tauri::State<'_, AppDatabase>,
    query: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<WikiPage<crate::db::service::wiki_service::WikiSourceInfo>, AppCommandError> {
    read_model::list_sources(&db.conn, query, offset, limit)
        .await
        .map_err(map_error)
}

/// Keep transport error semantics stable without changing unrelated DB APIs.
pub(crate) fn map_error(error: crate::db::error::DbError) -> AppCommandError {
    use crate::app_error::AppErrorCode;
    use crate::db::error::DbError;
    match error {
        DbError::Validation(message) => AppCommandError::invalid_input(message),
        DbError::NotFound(message) => AppCommandError::new(AppErrorCode::NotFound, message),
        DbError::Conflict(message) => AppCommandError::new(AppErrorCode::AlreadyExists, message),
        DbError::Io(error) => AppCommandError::io_error(error.to_string()),
        other => AppCommandError::from(other),
    }
}

//! 个人 Wiki 设置、导入、文件浏览与任务操作的 HTTP 适配层。
//! 与桌面调用同一 settings/commands/import 用例，仅负责 JSON 参数、响应和刷新事件。
//! 分页笔记/来源/任务阅读集中在相邻 wiki_read，避免保留旧重复列表接口。

use std::sync::Arc;

use axum::{extract::Extension, Json};
use serde::Deserialize;

use crate::app_error::AppCommandError;
use crate::app_state::AppState;
use crate::commands::wiki as core;
use crate::commands::wiki_engine as engine_core;
use crate::db::service::wiki_service::{
    WikiImportResult, WikiJobInfo, WikiProjectBindingInfo, WikiSourceInfo,
};
use crate::wiki::import::{
    ImportBatchResult, ImportFilesParams, ImportTextParams, LinkVersionParams,
    UpdateAnnotationsParams,
};
use crate::wiki::session_import::{
    ImportDirectoryParams, ImportLocalSessionsParams, WikiBulkImportResult,
};
use crate::wiki::settings::{WikiSettings, WikiSettingsView};

#[derive(Deserialize)]
pub struct UpdateWikiSettingsParams {
    pub settings: WikiSettings,
}

#[derive(Deserialize)]
pub struct IdParams {
    pub id: String,
}

#[derive(Deserialize)]
pub struct VaultReadParams {
    pub path: String,
}

pub async fn get_wiki_settings(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<WikiSettingsView>, AppCommandError> {
    Ok(Json(
        core::get_wiki_settings_core(&state.db.conn)
            .await
            .map_err(AppCommandError::from)?,
    ))
}

pub async fn update_wiki_settings(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<UpdateWikiSettingsParams>,
) -> Result<Json<WikiSettingsView>, AppCommandError> {
    let result = async {
        Ok(Json(
            core::update_wiki_settings_core(&state.db.conn, params.settings)
                .await
                .map_err(AppCommandError::from)?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_get_job(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<IdParams>,
) -> Result<Json<WikiJobInfo>, AppCommandError> {
    Ok(Json(
        crate::wiki::read_model::read_job(&state.db.conn, &params.id)
            .await
            .map_err(AppCommandError::from)?,
    ))
}

#[derive(Deserialize)]
pub struct ListProjectBindingsParams {
    #[serde(default)]
    pub vault_id: Option<String>,
}

pub async fn wiki_list_project_bindings(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ListProjectBindingsParams>,
) -> Result<Json<Vec<WikiProjectBindingInfo>>, AppCommandError> {
    Ok(Json(
        crate::db::service::wiki_service::list_project_bindings(
            &state.db.conn,
            params.vault_id.as_deref(),
        )
        .await?,
    ))
}

pub async fn wiki_vault_tree(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<core::WikiVaultTreeParams>,
) -> Result<Json<Vec<core::WikiVaultTreeEntry>>, AppCommandError> {
    Ok(Json(
        core::wiki_vault_tree_core(&state.db.conn, params).await?,
    ))
}

pub async fn wiki_vault_read(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<VaultReadParams>,
) -> Result<Json<crate::wiki::read_model::WikiVaultFile>, AppCommandError> {
    Ok(Json(
        crate::wiki::read_model::read_vault_file(&state.db.conn, params.path).await?,
    ))
}

pub async fn wiki_import_text(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportTextParams>,
) -> Result<Json<WikiImportResult>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::import_text(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_import_files(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportFilesParams>,
) -> Result<Json<ImportBatchResult>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::import_files_with_result(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_import_local_sessions(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportLocalSessionsParams>,
) -> Result<Json<WikiBulkImportResult>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::session_import::import_local_sessions(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_import_directory(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportDirectoryParams>,
) -> Result<Json<WikiBulkImportResult>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::session_import::import_directory(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

#[derive(Deserialize)]
pub struct SourceIdParams {
    pub source_id: String,
}

pub async fn wiki_accept_extraction(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<SourceIdParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::accept_extraction(&state.db.conn, params.source_id).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_update_source_annotations(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<UpdateAnnotationsParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::update_source_annotations(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_reextract(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<SourceIdParams>,
) -> Result<Json<WikiImportResult>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::reextract(&state.db.conn, params.source_id).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

pub async fn wiki_link_source_version(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<LinkVersionParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    let result = async {
        Ok(Json(
            crate::wiki::import::link_source_version(&state.db.conn, params).await?,
        ))
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(&state.db.conn, &state.emitter).await;
    }
    result
}

#[derive(Deserialize)]
pub struct CompileNowParams {
    pub request_id: String,
}

pub async fn wiki_compile_now(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<CompileNowParams>,
) -> Result<Json<WikiJobInfo>, AppCommandError> {
    Ok(Json(
        engine_core::wiki_compile_now_core(&state.db.conn, params.request_id).await?,
    ))
}

pub async fn wiki_retry_job(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<IdParams>,
) -> Result<Json<WikiJobInfo>, AppCommandError> {
    Ok(Json(
        engine_core::wiki_retry_job_core(&state.db.conn, params.id).await?,
    ))
}

pub async fn wiki_cancel_job(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<IdParams>,
) -> Result<Json<WikiJobInfo>, AppCommandError> {
    Ok(Json(
        engine_core::wiki_cancel_job_core(&state.db.conn, params.id).await?,
    ))
}

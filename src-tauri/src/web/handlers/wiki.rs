use std::sync::Arc;

use axum::{extract::Extension, Json};
use serde::Deserialize;

use crate::app_error::AppCommandError;
use crate::app_state::AppState;
use crate::commands::wiki as core;
use crate::commands::wiki::WikiMemoryNote;
use crate::commands::wiki_engine as engine_core;
use crate::db::service::wiki_service::{
    WikiImportResult, WikiJobInfo, WikiProjectBindingInfo, WikiSourceInfo,
};
use crate::wiki::import::{
    ImportFilesParams, ImportFilesResult, ImportTextParams, LinkVersionParams,
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
pub struct ListJobsParams {
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct IdParams {
    pub id: String,
}

#[derive(Deserialize)]
pub struct ListSourcesParams {
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
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
    Ok(Json(
        core::update_wiki_settings_core(&state.db.conn, params.settings)
            .await
            .map_err(AppCommandError::from)?,
    ))
}

pub async fn wiki_list_jobs(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ListJobsParams>,
) -> Result<Json<Vec<WikiJobInfo>>, AppCommandError> {
    Ok(Json(
        core::wiki_list_jobs_core(&state.db.conn, params.limit, params.offset, params.status)
            .await
            .map_err(AppCommandError::from)?,
    ))
}

pub async fn wiki_get_job(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<IdParams>,
) -> Result<Json<WikiJobInfo>, AppCommandError> {
    Ok(Json(
        core::wiki_get_job_core(&state.db.conn, params.id)
            .await
            .map_err(AppCommandError::from)?,
    ))
}

pub async fn wiki_list_sources(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ListSourcesParams>,
) -> Result<Json<Vec<WikiSourceInfo>>, AppCommandError> {
    Ok(Json(
        core::wiki_list_sources_core(
            &state.db.conn,
            params.limit,
            params.offset,
            params.source_kind,
            params.project_id,
        )
        .await
        .map_err(AppCommandError::from)?,
    ))
}

#[derive(Deserialize)]
pub struct ListProjectBindingsParams {
    #[serde(default)]
    pub vault_id: Option<String>,
}

pub async fn wiki_list_memory_notes(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Vec<WikiMemoryNote>>, AppCommandError> {
    Ok(Json(
        core::wiki_list_memory_notes_core(&state.db.conn).await?,
    ))
}

pub async fn wiki_list_project_bindings(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ListProjectBindingsParams>,
) -> Result<Json<Vec<WikiProjectBindingInfo>>, AppCommandError> {
    Ok(Json(
        core::wiki_list_project_bindings_core(&state.db.conn, params.vault_id).await?,
    ))
}

pub async fn wiki_get_source(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<IdParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    Ok(Json(
        core::wiki_get_source_core(&state.db.conn, params.id)
            .await
            .map_err(AppCommandError::from)?,
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
) -> Result<Json<core::WikiVaultFile>, AppCommandError> {
    Ok(Json(
        core::wiki_vault_read_core(&state.db.conn, params.path).await?,
    ))
}

pub async fn wiki_import_text(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportTextParams>,
) -> Result<Json<WikiImportResult>, AppCommandError> {
    Ok(Json(
        core::wiki_import_text_core(&state.db.conn, params).await?,
    ))
}

pub async fn wiki_import_files(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportFilesParams>,
) -> Result<Json<ImportFilesResult>, AppCommandError> {
    Ok(Json(
        core::wiki_import_files_core(&state.db.conn, params).await?,
    ))
}

pub async fn wiki_import_local_sessions(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportLocalSessionsParams>,
) -> Result<Json<WikiBulkImportResult>, AppCommandError> {
    Ok(Json(
        core::wiki_import_local_sessions_core(&state.db.conn, params).await?,
    ))
}

pub async fn wiki_import_directory(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<ImportDirectoryParams>,
) -> Result<Json<WikiBulkImportResult>, AppCommandError> {
    Ok(Json(
        core::wiki_import_directory_core(&state.db.conn, params).await?,
    ))
}

#[derive(Deserialize)]
pub struct SourceIdParams {
    pub source_id: String,
}

pub async fn wiki_accept_extraction(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<SourceIdParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    Ok(Json(
        core::wiki_accept_extraction_core(&state.db.conn, params.source_id).await?,
    ))
}

pub async fn wiki_update_source_annotations(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<UpdateAnnotationsParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    Ok(Json(
        core::wiki_update_source_annotations_core(&state.db.conn, params).await?,
    ))
}

pub async fn wiki_reextract(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<SourceIdParams>,
) -> Result<Json<WikiImportResult>, AppCommandError> {
    Ok(Json(
        core::wiki_reextract_core(&state.db.conn, params.source_id).await?,
    ))
}

pub async fn wiki_link_source_version(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<LinkVersionParams>,
) -> Result<Json<WikiSourceInfo>, AppCommandError> {
    Ok(Json(
        core::wiki_link_source_version_core(&state.db.conn, params).await?,
    ))
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

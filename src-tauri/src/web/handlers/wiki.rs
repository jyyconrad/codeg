use std::sync::Arc;

use axum::{extract::Extension, Json};
use serde::Deserialize;

use crate::app_error::AppCommandError;
use crate::app_state::AppState;
use crate::commands::wiki as core;
use crate::db::service::wiki_service::{WikiJobInfo, WikiSourceInfo};
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
}

#[derive(Deserialize)]
pub struct VaultPathParams {
    #[serde(default)]
    pub path: Option<String>,
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
        )
        .await
        .map_err(AppCommandError::from)?,
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
    Json(params): Json<VaultPathParams>,
) -> Result<Json<Vec<core::WikiVaultTreeEntry>>, AppCommandError> {
    Ok(Json(
        core::wiki_vault_tree_core(&state.db.conn, params.path).await?,
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

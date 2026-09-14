//! HTTP handlers for code-intelligence settings — the web-mode mirror of the
//! Tauri commands in `commands::code_intel`.

use axum::Json;
use serde::Deserialize;

use crate::agent::code_intel::CodeIntelConfig;
use crate::app_error::AppCommandError;
use crate::commands::code_intel::{
    get_code_intel_settings_core, get_code_intel_status_core, set_code_intel_settings_core,
    CodeIntelStatus,
};

pub async fn get_code_intel_settings() -> Result<Json<CodeIntelConfig>, AppCommandError> {
    Ok(Json(get_code_intel_settings_core()))
}

#[derive(Deserialize)]
pub struct SetCodeIntelSettingsParams {
    pub settings: CodeIntelConfig,
}

pub async fn set_code_intel_settings(
    Json(params): Json<SetCodeIntelSettingsParams>,
) -> Result<Json<CodeIntelConfig>, AppCommandError> {
    Ok(Json(set_code_intel_settings_core(params.settings)?))
}

#[derive(Default, Deserialize)]
pub struct GetCodeIntelStatusParams {
    #[serde(default)]
    pub cwd: Option<String>,
}

pub async fn get_code_intel_status(
    Json(params): Json<GetCodeIntelStatusParams>,
) -> Result<Json<CodeIntelStatus>, AppCommandError> {
    Ok(Json(get_code_intel_status_core(params.cwd)))
}

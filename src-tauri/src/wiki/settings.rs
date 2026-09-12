//! `wiki_settings` app_metadata JSON (spec §12.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::error::DbError;
use crate::db::service::app_metadata_service;
use sea_orm::{ConnectionTrait, DatabaseConnection};

pub const WIKI_SETTINGS_KEY: &str = "wiki_settings";
pub const WIKI_DB_INSTANCE_ID_KEY: &str = "wiki_db_instance_id";
pub const DEFAULT_COMPILE_CRON: &str = "0 3 * * *";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WikiCaptureSettings {
    #[serde(default = "default_true")]
    pub acp_enabled: bool,
    #[serde(default)]
    pub exclude_agent_types: Vec<String>,
    #[serde(default)]
    pub exclude_folder_ids: Vec<i32>,
}

impl Default for WikiCaptureSettings {
    fn default() -> Self {
        Self {
            acp_enabled: true,
            exclude_agent_types: Vec::new(),
            exclude_folder_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WikiIngestSettings {
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WikiCompileSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl Default for WikiCompileSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model_id: None,
            prompt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WikiSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub vault_path: Option<String>,
    #[serde(default = "default_timezone")]
    pub timezone: String,
    #[serde(default = "default_cron")]
    pub compile_cron: String,
    #[serde(default)]
    pub capture: WikiCaptureSettings,
    #[serde(default)]
    pub ingest: WikiIngestSettings,
    #[serde(default)]
    pub compile: WikiCompileSettings,
}

impl Default for WikiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_path: None,
            timezone: default_timezone(),
            compile_cron: default_cron(),
            capture: WikiCaptureSettings::default(),
            ingest: WikiIngestSettings::default(),
            compile: WikiCompileSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSettingsView {
    #[serde(flatten)]
    pub settings: WikiSettings,
    pub next_compile_at: Option<DateTime<Utc>>,
    pub pending_source_count: u64,
}

fn default_true() -> bool {
    true
}

fn default_cron() -> String {
    DEFAULT_COMPILE_CRON.to_string()
}

pub fn default_timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

pub async fn load_settings(conn: &DatabaseConnection) -> Result<WikiSettings, DbError> {
    let raw = app_metadata_service::get_value(conn, WIKI_SETTINGS_KEY).await?;
    match raw {
        None => Ok(WikiSettings::default()),
        Some(s) => serde_json::from_str(&s)
            .map_err(|e| DbError::Validation(format!("failed to parse wiki_settings: {e}"))),
    }
}

pub async fn save_settings(
    conn: &DatabaseConnection,
    settings: &WikiSettings,
) -> Result<(), DbError> {
    validate_settings(settings)?;
    let json = serde_json::to_string(settings)
        .map_err(|e| DbError::Validation(format!("serialize wiki_settings: {e}")))?;
    app_metadata_service::upsert_value(conn, WIKI_SETTINGS_KEY, &json).await
}

pub fn validate_settings(settings: &WikiSettings) -> Result<(), DbError> {
    crate::db::service::automation_service::compute_next_run(
        &settings.compile_cron,
        &settings.timezone,
        Utc::now(),
    )?;
    if let Some(p) = settings.vault_path.as_deref() {
        if p.contains('\0') {
            return Err(DbError::Validation("vault_path is invalid".into()));
        }
    }
    Ok(())
}

pub fn next_compile_at(settings: &WikiSettings) -> Option<DateTime<Utc>> {
    if !settings.enabled || !settings.compile.enabled {
        return None;
    }
    crate::db::service::automation_service::compute_next_run(
        &settings.compile_cron,
        &settings.timezone,
        Utc::now(),
    )
    .ok()
    .flatten()
}

pub async fn ensure_db_instance_id<C: ConnectionTrait>(conn: &C) -> Result<String, DbError> {
    if let Some(existing) =
        app_metadata_service::get_value_conn(conn, WIKI_DB_INSTANCE_ID_KEY).await?
    {
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    app_metadata_service::upsert_value(conn, WIKI_DB_INSTANCE_ID_KEY, &id).await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = WikiSettings::default();
        assert!(!s.enabled);
        assert!(s.capture.acp_enabled);
        assert!(s.compile.enabled);
        assert_eq!(s.compile_cron, "0 3 * * *");
        assert!(!s.timezone.is_empty());
    }
}

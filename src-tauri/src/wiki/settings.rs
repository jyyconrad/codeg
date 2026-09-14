//! `wiki_settings` app_metadata JSON (spec §12.1 / memory pipeline §4.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub struct WikiPromptSettings {
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WikiSynthesizeSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl Default for WikiSynthesizeSettings {
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
    pub turn_summary: WikiPromptSettings,
    #[serde(default)]
    pub session_rollup: WikiPromptSettings,
    #[serde(default)]
    pub synthesize: WikiSynthesizeSettings,
}

impl Default for WikiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_path: None,
            timezone: default_timezone(),
            compile_cron: default_cron(),
            capture: WikiCaptureSettings::default(),
            turn_summary: WikiPromptSettings::default(),
            session_rollup: WikiPromptSettings::default(),
            synthesize: WikiSynthesizeSettings::default(),
        }
    }
}

pub const WIKI_TURN_SUMMARY_BUILTIN: &str =
    include_str!("../../agent-skills/wiki-turn-summary/SKILL.md");
pub const WIKI_SESSION_ROLLUP_BUILTIN: &str =
    include_str!("../../agent-skills/wiki-session-rollup/SKILL.md");
pub const WIKI_SYNTHESIZE_BUILTIN: &str =
    include_str!("../../agent-skills/wiki-synthesize/SKILL.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSettingsView {
    #[serde(flatten)]
    pub settings: WikiSettings,
    pub next_compile_at: Option<DateTime<Utc>>,
    pub pending_source_count: u64,
    #[serde(default)]
    pub turn_summary_builtin_prompt: String,
    #[serde(default)]
    pub session_rollup_builtin_prompt: String,
    #[serde(default)]
    pub synthesize_builtin_prompt: String,
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
        Some(s) => parse_settings_json(&s),
    }
}

fn parse_settings_json(s: &str) -> Result<WikiSettings, DbError> {
    let value: Value = serde_json::from_str(s)
        .map_err(|e| DbError::Validation(format!("failed to parse wiki_settings: {e}")))?;
    let mut settings: WikiSettings = serde_json::from_value(value.clone())
        .map_err(|e| DbError::Validation(format!("failed to parse wiki_settings: {e}")))?;
    if value.pointer("/synthesize/enabled").is_none() {
        settings.synthesize.enabled = value
            .pointer("/compile/enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
    }
    // Never copy old ingest/compile custom prompts into the new slots.
    if value.get("turn_summary").is_none() {
        settings.turn_summary = WikiPromptSettings::default();
    }
    if value.get("session_rollup").is_none() {
        settings.session_rollup = WikiPromptSettings::default();
    }
    if value
        .get("synthesize")
        .and_then(|v| v.get("prompt"))
        .is_none()
        && value
            .get("synthesize")
            .and_then(|v| v.get("model_id"))
            .is_none()
    {
        if value.get("synthesize").is_none() {
            settings.synthesize.model_id = None;
            settings.synthesize.prompt = None;
        }
    }
    Ok(settings)
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
    if !settings.enabled || !settings.synthesize.enabled {
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
        assert!(s.synthesize.enabled);
        assert_eq!(s.compile_cron, "0 3 * * *");
        assert!(!s.timezone.is_empty());
        assert!(s.turn_summary.prompt.is_none());
        assert!(s.session_rollup.prompt.is_none());
        assert!(s.synthesize.prompt.is_none());
    }

    #[test]
    fn builtin_prompts_are_full_skills() {
        assert!(WIKI_TURN_SUMMARY_BUILTIN.contains("name: wiki-turn-summary"));
        assert!(WIKI_SESSION_ROLLUP_BUILTIN.contains("name: wiki-session-rollup"));
        assert!(WIKI_SYNTHESIZE_BUILTIN.contains("name: wiki-synthesize"));
        assert!(!WIKI_SYNTHESIZE_BUILTIN.contains("Four steps"));
        assert!(WIKI_SYNTHESIZE_BUILTIN.contains("Do not return a `candidates` array"));
    }

    #[test]
    fn read_copies_compile_enabled_not_prompts() {
        let raw = r#"{
            "enabled": true,
            "compile_cron": "0 3 * * *",
            "ingest": {"model_id": "old-in", "prompt": "do ingest"},
            "compile": {"enabled": false, "model_id": "old-co", "prompt": "do compile"}
        }"#;
        let s = parse_settings_json(raw).unwrap();
        assert!(s.enabled);
        assert!(!s.synthesize.enabled);
        assert!(s.synthesize.prompt.is_none());
        assert!(s.synthesize.model_id.is_none());
        assert!(s.turn_summary.prompt.is_none());
        assert!(s.session_rollup.prompt.is_none());
    }

    #[test]
    fn synthesize_enabled_wins_over_compile() {
        let raw = r#"{
            "enabled": true,
            "compile": {"enabled": false},
            "synthesize": {"enabled": true, "prompt": "custom"}
        }"#;
        let s = parse_settings_json(raw).unwrap();
        assert!(s.synthesize.enabled);
        assert_eq!(s.synthesize.prompt.as_deref(), Some("custom"));
    }
}

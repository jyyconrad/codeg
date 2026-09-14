//! 保存与校验 Wiki 设置：开关、目录、采集过滤、定时整理和三个模型阶段。
//! commands/wiki 负责设置用例与初次模型建议，engine 按这里的计划调度任务。
//! 统一使用 wiki_settings 保存配置；模型绑定独立，不跟随聊天模型隐式变化。

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
pub struct WikiPromptSettings {
    #[serde(default)]
    pub provider_id: Option<i32>,
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
    pub provider_id: Option<i32>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
}

impl Default for WikiSynthesizeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            provider_id: None,
            model_id: None,
            prompt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
    serde_json::from_str(s)
        .map_err(|e| DbError::Validation(format!("failed to parse wiki_settings: {e}")))
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
        if p.contains('\0') || (!p.trim().is_empty() && !std::path::Path::new(p).is_absolute()) {
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

    #[tokio::test]
    async fn settings_use_one_stable_key_and_preserve_database_identity() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let instance_id = ensure_db_instance_id(&db.conn).await.unwrap();
        let saved = WikiSettings {
            enabled: true,
            vault_path: Some("/tmp/personal-notes".into()),
            ..Default::default()
        };
        save_settings(&db.conn, &saved).await.unwrap();
        assert_eq!(WIKI_SETTINGS_KEY, "wiki_settings");
        let stored = app_metadata_service::get_value(&db.conn, "wiki_settings")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parse_settings_json(&stored).unwrap(), saved);
        assert_eq!(load_settings(&db.conn).await.unwrap(), saved);
        assert_eq!(ensure_db_instance_id(&db.conn).await.unwrap(), instance_id);
    }

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
        assert!(WIKI_SYNTHESIZE_BUILTIN.contains("codeg.wiki.synthesize.v2"));
    }

    #[test]
    fn unknown_configuration_fields_are_rejected() {
        let raw = r#"{"enabled":true,"unsupported_setting":{}}"#;
        assert!(parse_settings_json(raw).is_err());
    }

    #[test]
    fn current_configuration_roundtrips_without_chat_defaults() {
        let mut expected = WikiSettings::default();
        expected.turn_summary.provider_id = Some(7);
        expected.turn_summary.model_id = Some("saved-model".into());
        let encoded = serde_json::to_string(&expected).unwrap();
        assert_eq!(parse_settings_json(&encoded).unwrap(), expected);
    }
}

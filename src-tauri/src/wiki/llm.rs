//! Bind the Codeg Agent channel for WikiWorker. Keys are never persisted.

use std::collections::HashMap;

use async_trait::async_trait;
use rig::client::AgentClientExt;
use sea_orm::DatabaseConnection;
use serde_json::Value;

use crate::acp::native_config::{
    resolve_codeg_agent_config, BoundProvider, CodegProtocol, WireProtocol,
};
use crate::agent::model::{resolve_session_wire_protocol, CodegLlmClient};
use crate::models::AgentType;

pub const BLOCKED_BY_CONFIGURATION: &str = "blocked-by-configuration";
pub const COMPILE_CONTRACT_VERSION: &str = "codeg.wiki.compile.v1";

#[derive(Debug, Clone, thiserror::Error)]
pub enum WikiLlmError {
    #[error("blocked-by-configuration: {0}")]
    Blocked(String),
    #[error("{0}")]
    Failed(String),
}

impl WikiLlmError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Blocked(_) => BLOCKED_BY_CONFIGURATION,
            Self::Failed(_) => "model_failed",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

#[async_trait]
pub trait WikiLlm: Send + Sync {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<Value, WikiLlmError>;
}

/// Bound client. `api_key` lives only on this value — never written to jobs/raw/logs.
pub struct BoundWikiModel {
    pub client: CodegLlmClient,
    pub model_id: String,
    pub protocol: WireProtocol,
}

pub struct ProductionWikiLlm {
    bound: BoundWikiModel,
    skill: String,
    extra_prompt: Option<String>,
}

impl ProductionWikiLlm {
    pub fn new(bound: BoundWikiModel, skill: String, extra_prompt: Option<String>) -> Self {
        Self {
            bound,
            skill,
            extra_prompt,
        }
    }

    pub fn model_id(&self) -> &str {
        &self.bound.model_id
    }

    pub fn protocol(&self) -> WireProtocol {
        self.bound.protocol
    }
}

#[async_trait]
impl WikiLlm for ProductionWikiLlm {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<Value, WikiLlmError> {
        let mut preamble = self.skill.clone();
        if let Some(extra) = &self.extra_prompt {
            preamble.push_str("\n\n# User extra prompt (cannot raise permissions)\n\n");
            preamble.push_str(extra);
        }
        preamble
            .push_str("\n\nReturn ONLY JSON. No markdown fence. The host validates the schema.\n");
        let user = format!("stage={stage}\ninput={input}\n");
        let text = one_shot(&self.bound, &preamble, &user).await?;
        parse_json_object(&text)
    }
}

async fn one_shot(
    bound: &BoundWikiModel,
    preamble: &str,
    user: &str,
) -> Result<String, WikiLlmError> {
    use crate::agent::hook::CodegHook;
    use crate::agent::hook::HookTrace;
    use futures::StreamExt;
    use rig::agent::MultiTurnStreamItem;
    use rig::completion::Message;

    let trace = HookTrace::new();
    let hook = CodegHook::auto_allow(trace.clone());
    let prompt = Message::user(user);
    let stream = match &bound.client {
        CodegLlmClient::Completions(c) => {
            c.agent(&bound.model_id)
                .preamble(preamble)
                .build()
                .runner(prompt)
                .max_turns(4)
                .add_hook(hook)
                .stream()
                .await
        }
        CodegLlmClient::Responses(c) => {
            c.agent(&bound.model_id)
                .preamble(preamble)
                .build()
                .runner(prompt)
                .max_turns(4)
                .add_hook(hook)
                .stream()
                .await
        }
    };
    let mut stream = stream;
    let mut last_err: Option<String> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::FinalResponse(_)) => break,
            Ok(_) => {}
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    let text = trace.aggregated_text();
    if text.trim().is_empty() {
        return Err(WikiLlmError::Failed(
            last_err.unwrap_or_else(|| "empty model response".into()),
        ));
    }
    Ok(text)
}

pub fn parse_json_object(text: &str) -> Result<Value, WikiLlmError> {
    let trimmed = text.trim();
    let body = strip_fence(trimmed);
    let value: Value = serde_json::from_str(body)
        .map_err(|e| WikiLlmError::Failed(format!("model output is not JSON: {e}")))?;
    if !value.is_object() {
        return Err(WikiLlmError::Failed("model JSON must be an object".into()));
    }
    Ok(value)
}

fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest);
    let rest = rest.trim_start_matches('\n');
    rest.rsplit_once("```")
        .map(|(a, _)| a.trim())
        .unwrap_or(rest)
}

pub async fn bind_wiki_model(
    conn: &DatabaseConnection,
    model_override: Option<&str>,
) -> Result<BoundWikiModel, WikiLlmError> {
    let (env, provider) = load_codeg_bind(conn).await;
    let Some(provider) = provider else {
        return Err(WikiLlmError::Blocked(
            "no Codeg Agent model provider is bound".into(),
        ));
    };
    let mut config = resolve_codeg_agent_config(&env, Some(&provider))
        .map_err(|e| WikiLlmError::Blocked(format!("native config: {e:?}")))?;
    if let Some(id) = model_override.map(str::trim).filter(|s| !s.is_empty()) {
        if !config.context_windows.contains_key(id) && config.model_id != id {
            return Err(WikiLlmError::Blocked(format!(
                "model_id {id} is not in the bound channel catalog"
            )));
        }
        config.model_id = id.to_string();
    }
    let wire = match config.protocol {
        CodegProtocol::Auto => resolve_session_wire_protocol(&config).await,
        _ => config.wire_protocol(),
    };
    // Rebuild client after resolving wire; key stays in memory only.
    let client = CodegLlmClient::build(&config.api_key, &config.api_base_url, wire)
        .map_err(|e| WikiLlmError::Blocked(e.to_string()))?;
    Ok(BoundWikiModel {
        client,
        model_id: config.model_id,
        protocol: wire,
    })
}

async fn load_codeg_bind(
    conn: &DatabaseConnection,
) -> (
    std::collections::BTreeMap<String, String>,
    Option<BoundProvider>,
) {
    let setting =
        crate::db::service::agent_setting_service::get_by_agent_type(conn, AgentType::CodegAgent)
            .await
            .ok()
            .flatten();
    let env = setting
        .as_ref()
        .and_then(|m| m.env_json.as_deref())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let provider = match setting.as_ref().and_then(|s| s.model_provider_id) {
        Some(id) => crate::db::service::model_provider_service::get_by_id(conn, id)
            .await
            .ok()
            .flatten()
            .map(|p| BoundProvider {
                api_url: p.api_url,
                api_key: p.api_key,
                model: p.model,
            }),
        None => None,
    };
    (env, provider)
}

/// In-memory LLM for host tests. Never touches the network.
pub struct MockWikiLlm {
    pub by_stage: HashMap<String, Value>,
    pub fail: bool,
}

impl MockWikiLlm {
    pub fn failing() -> Self {
        Self {
            by_stage: HashMap::new(),
            fail: true,
        }
    }

    pub fn with_stage(mut self, stage: &str, value: Value) -> Self {
        self.by_stage.insert(stage.to_string(), value);
        self
    }
}

impl Default for MockWikiLlm {
    fn default() -> Self {
        Self {
            by_stage: HashMap::new(),
            fail: false,
        }
    }
}

#[async_trait]
impl WikiLlm for MockWikiLlm {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<Value, WikiLlmError> {
        if self.fail {
            return Err(WikiLlmError::Failed("mock llm failure".into()));
        }
        if let Some(v) = self.by_stage.get(stage) {
            return Ok(v.clone());
        }
        default_mock_stage(stage, &input)
    }
}

fn default_mock_stage(stage: &str, input: &Value) -> Result<Value, WikiLlmError> {
    let source_id = input
        .get("source_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            input
                .pointer("/inputs/0/source_id")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            input
                .pointer("/candidates/0/locator/source_id")
                .and_then(|v| v.as_str())
        })
        .unwrap_or("source");
    let segment_id = input
        .pointer("/segment_ids/0")
        .and_then(|v| v.as_str())
        .or_else(|| {
            input
                .pointer("/segments/0/segment_id")
                .and_then(|v| v.as_str())
        })
        .unwrap_or("s1");
    match stage {
        "ingest" | "summary" => Ok(serde_json::json!({
            "schema": "codeg.wiki.ingest.v1",
            "source_id": source_id,
            "source_summary": "One-line source summary from mock.",
            "topic_suggestions": [{"kind": "capability", "title": "Interface design"}],
            "nothing_to_summarize": false,
            "warnings": []
        })),
        "candidates" => Ok(serde_json::json!({
            "candidates": [{
                "candidate_id": "c1",
                "kind": "capability",
                "title": "Interface design",
                "claim": "The spec requires an idempotency key on retried POSTs.",
                "evidence_type": "reference",
                "actor": "unspecified",
                "locator": {
                    "source_id": source_id,
                    "segment_id": segment_id,
                    "pointer": "§3.2"
                }
            }]
        })),
        "match" => Ok(serde_json::json!({
            "matches": [{
                "candidate_id": "c1",
                "relation": "new",
                "existing_note_id": null,
                "reason": "No existing capability identity; related≠same."
            }]
        })),
        "merge" => Ok(serde_json::json!({
            "page_proposals": []
        })),
        "finalize" => Ok(serde_json::json!({
            "nothing_to_persist": false
        })),
        _ => Ok(serde_json::json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_json_strips_fence_and_rejects_non_object() {
        let v = parse_json_object("```json\n{\"summary\":\"x\"}\n```").unwrap();
        assert_eq!(v["summary"], "x");
        assert!(parse_json_object("[1]").is_err());
        assert!(parse_json_object("not json").is_err());
    }

    #[test]
    fn blocked_error_is_not_retryable() {
        let e = WikiLlmError::Blocked("no provider".into());
        assert_eq!(e.error_code(), BLOCKED_BY_CONFIGURATION);
        assert!(!e.retryable());
        assert!(WikiLlmError::Failed("net".into()).retryable());
    }
}

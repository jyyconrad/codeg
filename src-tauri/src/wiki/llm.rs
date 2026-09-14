//! Bind the Codeg Agent channel for WikiWorker. Keys are never persisted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rig::client::AgentClientExt;
use sea_orm::DatabaseConnection;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
use crate::acp::native_config::{
    resolve_codeg_agent_config, BoundProvider, CodegProtocol, WireProtocol,
};
use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder};
use crate::agent::model::{
    resolve_session_wire_protocol, CodegLlmClient, NativeTurnTools, WIKI_COMPILE_MAX_TURNS,
};
use crate::agent::tools::NativeToolCtx;
use crate::models::AgentType;

pub const BLOCKED_BY_CONFIGURATION: &str = "blocked-by-configuration";
pub const COMPILE_CONTRACT_VERSION: &str = "codeg.wiki.compile.v1";
pub const SYNTHESIZE_CONTRACT_VERSION: &str = "codeg.wiki.synthesize.v1";
pub const TURN_SUMMARY_CONTRACT_VERSION: &str = "codeg.wiki.turn_summary.v1";
pub const SESSION_ROLLUP_CONTRACT_VERSION: &str = "codeg.wiki.session_rollup.v1";
pub const WIKI_TURN_SUMMARY_MAX_TURNS: usize = 8;
pub const WIKI_SESSION_ROLLUP_MAX_TURNS: usize = 16;

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

/// User-edited prompt replaces the built-in skill body. Empty/whitespace
/// falls back to the built-in skill. Host schema reminder is always appended.
pub fn resolve_wiki_preamble(builtin: &str, user_prompt: Option<&str>) -> String {
    let body = user_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(builtin);
    format!("{body}\n\nReturn ONLY JSON. No markdown fence. The host validates the schema.\n")
}

#[async_trait]
impl WikiLlm for ProductionWikiLlm {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<Value, WikiLlmError> {
        let preamble = resolve_wiki_preamble(&self.skill, self.extra_prompt.as_deref());
        let user = format!("stage={stage}\ninput={input}\n");
        let vault = input
            .get("vault_abs")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let staging = input
            .get("staging_abs")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let extra_roots: Vec<PathBuf> = input
            .get("extra_read_roots")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(PathBuf::from))
                    .collect()
            })
            .unwrap_or_default();
        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(WIKI_COMPILE_MAX_TURNS);
        let workspace = match (vault.as_deref(), staging.as_deref()) {
            (Some(v), Some(s)) => Some((v, s, extra_roots.as_slice())),
            (Some(v), None) => Some((v, v, extra_roots.as_slice())),
            _ => None,
        };
        let text = one_shot(&self.bound, &preamble, &user, workspace, max_turns).await?;
        parse_json_object(&text)
    }
}

pub fn wiki_fs_policy(
    vault: &Path,
    staging: &Path,
    extra_read_roots: &[PathBuf],
) -> FsAccessPolicy {
    FsAccessPolicy::wiki_worker_with_extra_reads(vault, staging, extra_read_roots)
}

fn wiki_tool_ctx(vault: &Path, staging: &Path, extra_read_roots: &[PathBuf]) -> NativeToolCtx {
    let store = Arc::new(Mutex::new(ContextStore::new("wiki-worker")));
    let recorder = Arc::new(FactRecorder::memory(Arc::clone(&store)));
    NativeToolCtx {
        turn_id: 1,
        identity: Arc::new(CallIdentityBridge::new()),
        recorder,
        cancel: CancellationToken::new(),
        launch_cwd: vault.to_path_buf(),
        fs: Arc::new(FileSystemRuntime::with_policy(
            FsAccessPolicy::wiki_worker_with_extra_reads(vault, staging, extra_read_roots),
        )),
        session_id: "wiki-worker".into(),
        spill_dir: staging.join("spills"),
    }
}

async fn one_shot(
    bound: &BoundWikiModel,
    preamble: &str,
    user: &str,
    workspace: Option<(&Path, &Path, &[PathBuf])>,
    max_turns: usize,
) -> Result<String, WikiLlmError> {
    use crate::agent::hook::CodegHook;
    use crate::agent::hook::HookTrace;
    use futures::StreamExt;
    use rig::agent::MultiTurnStreamItem;
    use rig::completion::Message;

    let trace = HookTrace::new();
    let hook = CodegHook::auto_allow(trace.clone());
    let prompt = Message::user(user);
    let tools = workspace.map(|(vault, staging, extra)| {
        NativeTurnTools::wiki_compile(wiki_tool_ctx(vault, staging, extra))
    });
    let max_turns = if tools.is_some() { max_turns.max(1) } else { 4 };
    let stream = match &bound.client {
        CodegLlmClient::Completions(c) => {
            stream_with_optional_tools(c, &bound.model_id, preamble, prompt, tools, hook, max_turns)
                .await
        }
        CodegLlmClient::Responses(c) => {
            stream_with_optional_tools(c, &bound.model_id, preamble, prompt, tools, hook, max_turns)
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

async fn stream_with_optional_tools<C>(
    client: &C,
    model_id: &str,
    preamble: &str,
    prompt: rig::completion::Message,
    tools: Option<NativeTurnTools>,
    hook: crate::agent::hook::CodegHook,
    max_turns: usize,
) -> rig::agent::StreamingResult
where
    C: AgentClientExt + Sync,
    C::CompletionModel: 'static,
{
    let max_turns = max_turns.max(1);
    if let Some(tools) = tools {
        let mut builder = client
            .agent(model_id)
            .preamble(preamble)
            .default_max_turns(max_turns)
            .tool(tools.read)
            .tool(tools.recall)
            .tool(tools.glob)
            .tool(tools.grep)
            .tool(tools.skill);
        if let Some(write) = tools.write {
            builder = builder.tool(write);
        }
        if let Some(edit) = tools.edit {
            builder = builder.tool(edit);
        }
        builder
            .build()
            .runner(prompt)
            .max_turns(max_turns)
            .add_hook(hook)
            .stream()
            .await
    } else {
        client
            .agent(model_id)
            .preamble(preamble)
            .default_max_turns(max_turns)
            .build()
            .runner(prompt)
            .max_turns(max_turns)
            .add_hook(hook)
            .stream()
            .await
    }
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
    match stage {
        "ingest" | "summary" | "turn_summary" => Ok(serde_json::json!({
            "schema": "codeg.wiki.turn_summary.v1",
            "source_id": source_id,
            "title": "Fixed list API cursor pagination",
            "body": "The agent edited the list handler to use cursor pagination.\n\nNo file changes in the snapshot were independently verified. This is not user mastery.",
            "nothing_to_summarize": false,
            "warnings": []
        })),
        "session_rollup" => Ok(serde_json::json!({
            "schema": "codeg.wiki.session_rollup.v1",
            "conversation_id": input.get("conversation_id").cloned().unwrap_or(serde_json::json!(1)),
            "title": "Shipped cursor pagination for the list API",
            "body": "This conversation implemented cursor pagination on the list handler.",
            "nothing_to_summarize": false,
            "warnings": []
        })),
        "synthesize" | "compile" => {
            let notes = input
                .get("memory_notes")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            Ok(serde_json::json!({
                "schema": "codeg.wiki.synthesize.v1",
                "processed_inputs": notes,
                "page_proposals": [{
                    "op": "create",
                    "type": "capability",
                    "title": "Interface design",
                    "path": "capabilities/interface-design-mock.md",
                    "body": "---\ntitle: \"Interface design\"\ntype: capability\ntags:\n  - \"type/capability\"\ncodeg_note_id: \"22222222-2222-4222-8222-222222222222\"\nevidence_level: knowledge_only\n---\n\n<!-- codeg-content:start -->\n# Interface design\n\nDerived from memory notes. Agent actions are not user mastery.\n<!-- codeg-content:end -->\n"
                }],
                "nothing_to_persist": false,
                "warnings": [],
                "needs_review": []
            }))
        }
        "candidates" => Ok(serde_json::json!({
            "page_proposals": []
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

    #[test]
    fn user_prompt_replaces_builtin_skill() {
        let preamble = resolve_wiki_preamble("# builtin\nnever invent", Some("# custom\nbe terse"));
        assert!(preamble.starts_with("# custom\nbe terse"));
        assert!(!preamble.contains("never invent"));
        assert!(preamble.contains("Return ONLY JSON"));
    }

    #[test]
    fn empty_user_prompt_keeps_builtin() {
        let preamble = resolve_wiki_preamble("# builtin", Some("  \n"));
        assert!(preamble.starts_with("# builtin"));
        let none = resolve_wiki_preamble("# builtin", None);
        assert!(none.starts_with("# builtin"));
    }
}

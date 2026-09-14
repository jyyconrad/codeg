//! 为Wiki整理任务绑定Codeg Agent模型并执行工具调用。
//! engine提供模型配置，轮次/对话/综合任务提供轻量来源路径；本模块负责
//! 内置提示词、Wiki读取与staging写入权限、取消、JSON解析及来源读取告警。
//! 密钥仅保存在运行时内存，读取覆盖率和输入版本不作为产出条件。

#[cfg(test)]
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

pub const BLOCKED_BY_CONFIGURATION: &str = "model_unavailable";
pub const SYNTHESIZE_CONTRACT_VERSION: &str = "codeg.wiki.synthesize.v2";
pub const TURN_SUMMARY_CONTRACT_VERSION: &str = "codeg.wiki.turn_summary.v2";
pub const SESSION_ROLLUP_CONTRACT_VERSION: &str = "codeg.wiki.session_rollup.v2";
pub const WIKI_TURN_SUMMARY_MAX_TURNS: usize = 8;
pub const WIKI_SESSION_ROLLUP_MAX_TURNS: usize = 16;

#[derive(Debug, Clone, thiserror::Error)]
pub enum WikiLlmError {
    #[error("model unavailable: {0}")]
    Blocked(String),
    #[error("model failed: {0}")]
    Failed(String),
    #[error("invalid output: {0}")]
    InvalidOutput(String),
    #[error("source missing: {0}")]
    SourceMissing(String),
    #[error("source read failed: {0}")]
    SourceReadFailed(String),
    #[error("cancelled")]
    Cancelled,
    #[error("deadline exceeded")]
    DeadlineExceeded,
}

impl WikiLlmError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Blocked(_) => "model_unavailable",
            Self::Failed(_) => "model_failed",
            Self::InvalidOutput(_) => "invalid_output",
            Self::SourceMissing(_) => "source_missing",
            Self::SourceReadFailed(_) => "source_read_failed",
            Self::Cancelled => "cancelled",
            Self::DeadlineExceeded => "deadline_exceeded",
        }
    }
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

#[async_trait]
pub trait WikiLlm: Send + Sync {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<WikiLlmRun, WikiLlmError>;
    fn check_cancelled(&self) -> Result<(), WikiLlmError> {
        Ok(())
    }
    fn input_budget(&self) -> u64 {
        (128_000 - 8_192 - 16_000) * 60 / 100
    }
}

/// File locations the model may consult while organizing the Wiki.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SourceReference {
    pub rel: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WikiLlmRun {
    pub output: Value,
}

/// 把未成功读取的来源留为告警，不阻断对其余材料的整理；同路径重试成功后清除旧失败。
fn read_warnings(trace: &crate::agent::hook::HookTrace) -> Vec<String> {
    let mut reads = std::collections::BTreeMap::new();
    for result in trace.tool_results() {
        if result.tool_name != "read_file" {
            continue;
        }
        let args = serde_json::from_str::<Value>(&result.args).unwrap_or(Value::Null);
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("read_file")
            .to_string();
        reads.insert(path, result.status == "success");
    }
    reads
        .into_iter()
        .filter(|(_, success)| !success)
        .map(|(path, _)| format!("Source could not be read: {path}"))
        .collect()
}

/// Bound client. `api_key` lives only on this value — never written to jobs/raw/logs.
pub struct BoundWikiModel {
    pub client: CodegLlmClient,
    pub model_id: String,
    /// engine用实际协议记录任务尝试，保留排查模型调用问题所需的运行信息。
    pub protocol: WireProtocol,
    pub context_window: u64,
    pub max_output: u64,
}

pub struct ProductionWikiLlm {
    bound: BoundWikiModel,
    skill: String,
    extra_prompt: Option<String>,
    cancel: CancellationToken,
}

impl ProductionWikiLlm {
    pub fn new(bound: BoundWikiModel, skill: String, extra_prompt: Option<String>) -> Self {
        Self {
            bound,
            skill,
            extra_prompt,
            cancel: CancellationToken::new(),
        }
    }

    pub fn with_cancellation(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
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

/// Contract comes from the shipped skill, independently of user prose. A
/// custom prompt may change editorial preference but cannot remove host fields.
pub fn resolve_wiki_stage_preamble(
    stage: &str,
    builtin: &str,
    user_prompt: Option<&str>,
) -> String {
    let contract_source = match stage {
        "turn_summary" => include_str!("../../agent-skills/wiki-turn-summary/SKILL.md"),
        "session_rollup" => include_str!("../../agent-skills/wiki-session-rollup/SKILL.md"),
        _ => include_str!("../../agent-skills/wiki-synthesize/SKILL.md"),
    };
    let contract = contract_source
        .split_once("## Return value")
        .map(|(_, section)| section.split("## Failure").next().unwrap_or(section))
        .unwrap_or(contract_source);
    let output_rule = "Generated output must have a substantive body. The host records source links; line evidence and read receipts are not required.";
    format!("{}\nHost output contract for {stage} (mandatory):\n{contract}\nUse source_references to read the material needed for the Wiki. Record unavailable sources in warnings and use the available material; do not invent source contents. {output_rule}\n", resolve_wiki_preamble(builtin, user_prompt))
}

#[async_trait]
impl WikiLlm for ProductionWikiLlm {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<WikiLlmRun, WikiLlmError> {
        let preamble =
            resolve_wiki_stage_preamble(stage, &self.skill, self.extra_prompt.as_deref());
        let user = format!("stage={stage}\ninput={input}\n");
        let vault = input
            .get("vault_abs")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let staging = input
            .get("staging_abs")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(WIKI_COMPILE_MAX_TURNS);
        let workspace = match (vault.as_deref(), staging.as_deref()) {
            (Some(v), Some(s)) => Some((v, s)),
            (Some(v), None) => Some((v, v)),
            _ => None,
        };
        self.check_cancelled()?;
        let session = format!(
            "wiki:{}:{}",
            input
                .get("job_id")
                .and_then(Value::as_str)
                .unwrap_or("attempt"),
            input.get("attempt").and_then(Value::as_i64).unwrap_or(1)
        );
        let (text, trace) = tokio::select! {
            _ = self.cancel.cancelled() => return Err(WikiLlmError::Cancelled),
            result = one_shot(&self.bound, &preamble, &user, workspace, max_turns, &session, self.cancel.clone()) => result?,
        };
        self.check_cancelled()?;
        let mut output = parse_json_object(&text)?;
        let warnings = read_warnings(&trace);
        if !warnings.is_empty() {
            if !output["warnings"].is_array() {
                output["warnings"] = Value::Array(Vec::new());
            }
            output["warnings"]
                .as_array_mut()
                .expect("warnings array")
                .extend(warnings.into_iter().map(Value::String));
        }
        Ok(WikiLlmRun { output })
    }
    fn check_cancelled(&self) -> Result<(), WikiLlmError> {
        if self.cancel.is_cancelled() {
            Err(WikiLlmError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn input_budget(&self) -> u64 {
        self.bound
            .context_window
            .saturating_sub(self.bound.max_output + 16_000)
            * 60
            / 100
    }
}

pub fn wiki_fs_policy(vault: &Path, staging: &Path) -> FsAccessPolicy {
    FsAccessPolicy::wiki_worker_with_extra_reads(vault, staging, &[staging.to_path_buf()])
}

fn wiki_tool_ctx(
    vault: &Path,
    staging: &Path,
    session: &str,
    cancel: CancellationToken,
) -> NativeToolCtx {
    let store = Arc::new(Mutex::new(ContextStore::new(session)));
    let recorder =
        Arc::new(FactRecorder::memory(Arc::clone(&store)).with_spill_dir(staging.join("spills")));
    NativeToolCtx {
        turn_id: 1,
        identity: Arc::new(CallIdentityBridge::new()),
        recorder,
        cancel,
        launch_cwd: vault.to_path_buf(),
        fs: Arc::new(FileSystemRuntime::with_policy(wiki_fs_policy(
            vault, staging,
        ))),
        session_id: session.into(),
        spill_dir: staging.join("spills"),
    }
}

async fn one_shot(
    bound: &BoundWikiModel,
    preamble: &str,
    user: &str,
    workspace: Option<(&Path, &Path)>,
    max_turns: usize,
    session: &str,
    cancel: CancellationToken,
) -> Result<(String, crate::agent::hook::HookTrace), WikiLlmError> {
    use crate::agent::hook::CodegHook;
    use crate::agent::hook::HookTrace;
    use futures::StreamExt;
    use rig::agent::MultiTurnStreamItem;
    use rig::completion::Message;

    let trace = HookTrace::new();
    let context =
        workspace.map(|(vault, staging)| wiki_tool_ctx(vault, staging, session, cancel.clone()));
    let hook = context
        .clone()
        .map(|ctx| CodegHook::auto_allow_with_tools(trace.clone(), ctx))
        .unwrap_or_else(|| CodegHook::auto_allow(trace.clone()));
    let prompt = Message::user(user);
    let tools = context.map(NativeTurnTools::wiki_compile);
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
    let mut final_text = None;
    while let Some(item) = tokio::select! {
        _ = cancel.cancelled() => return Err(WikiLlmError::Cancelled),
        item = stream.next() => item,
    } {
        match item {
            Ok(MultiTurnStreamItem::FinalResponse(response)) => final_text = Some(response.output),
            Ok(_) => {}
            Err(e) => {
                let message = e.to_string();
                let lower = message.to_ascii_lowercase();
                if lower.contains("401")
                    || lower.contains("403")
                    || lower.contains("unauthorized")
                    || lower.contains("authentication")
                {
                    return Err(WikiLlmError::Blocked(message));
                }
                return Err(WikiLlmError::Failed(message));
            }
        }
    }
    let text = final_text
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| WikiLlmError::Failed("stream ended without final response".into()))?;
    Ok((text, trace))
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
            .tool_concurrency(1)
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
            .tool_concurrency(1)
            .add_hook(hook)
            .stream()
            .await
    }
}

pub fn parse_json_object(text: &str) -> Result<Value, WikiLlmError> {
    let trimmed = text.trim();
    let body = strip_fence(trimmed);
    let value: Value = serde_json::from_str(body)
        .map_err(|e| WikiLlmError::InvalidOutput(format!("model output is not JSON: {e}")))?;
    if !value.is_object() {
        return Err(WikiLlmError::InvalidOutput(
            "model JSON must be an object".into(),
        ));
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
    provider_id: Option<i32>,
    model_override: Option<&str>,
) -> Result<BoundWikiModel, WikiLlmError> {
    if provider_id.is_none() || model_override.is_none_or(|id| id.trim().is_empty()) {
        return Err(WikiLlmError::Blocked(
            "Wiki requires an explicit provider and model".into(),
        ));
    }
    let (env, provider) = load_codeg_bind(conn, provider_id).await;
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
                "model_id {id} is not in the selected channel catalog"
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
        context_window: u64::from(
            *config
                .context_windows
                .get(&config.model_id)
                .unwrap_or(&128_000),
        ),
        max_output: u64::from(config.max_output_tokens),
        model_id: config.model_id,
        protocol: wire,
    })
}

async fn load_codeg_bind(
    conn: &DatabaseConnection,
    provider_id: Option<i32>,
) -> (
    std::collections::BTreeMap<String, String>,
    Option<BoundProvider>,
) {
    let setting =
        crate::db::service::agent_setting_service::get_by_agent_type(conn, AgentType::CodegAgent)
            .await
            .ok()
            .flatten();
    let mut env = setting
        .as_ref()
        .and_then(|m| m.env_json.as_deref())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let fallback_provider_id = setting.as_ref().and_then(|s| s.model_provider_id);
    let selected_provider_id = provider_id.or(fallback_provider_id);
    let provider_row = match selected_provider_id {
        Some(id) => crate::db::service::model_provider_service::get_by_id(conn, id)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let provider = provider_row.map(|p| BoundProvider {
        api_url: p.api_url,
        api_key: p.api_key,
        model: p.model,
    });
    if let Some(provider) = provider.as_ref() {
        crate::acp::native_config::project_bound_provider_catalog(&mut env, provider);
        if !env.contains_key(crate::acp::native_config::PROTOCOL_KEY) {
            if let Some(model_id) =
                crate::acp::native_config::completions_model_id(provider.model.as_deref())
            {
                if let Ok(raw) = serde_json::to_string(&std::collections::BTreeMap::from([(
                    model_id,
                    128_000_u32,
                )])) {
                    env.insert(crate::acp::native_config::CONTEXT_WINDOWS_KEY.into(), raw);
                }
            }
        }
    }
    (env, provider)
}

/// In-memory LLM for host tests. Never touches the network.
#[cfg(test)]
#[derive(Default)]
pub struct MockWikiLlm {
    pub by_stage: HashMap<String, Value>,
    pub fail: bool,
}

#[cfg(test)]
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

#[cfg(test)]
#[async_trait]
impl WikiLlm for MockWikiLlm {
    async fn complete_json(&self, stage: &str, input: Value) -> Result<WikiLlmRun, WikiLlmError> {
        if self.fail {
            return Err(WikiLlmError::Failed("mock llm failure".into()));
        }
        if let Some(v) = self.by_stage.get(stage) {
            return Ok(WikiLlmRun { output: v.clone() });
        }
        Ok(WikiLlmRun {
            output: default_mock_stage(stage, &input)?,
        })
    }
}

#[cfg(test)]
fn default_mock_stage(stage: &str, input: &Value) -> Result<Value, WikiLlmError> {
    let (schema, key, identity) = match stage {
        "turn_summary" => (
            TURN_SUMMARY_CONTRACT_VERSION,
            "source_id",
            input["source_id"].clone(),
        ),
        "session_rollup" => (
            SESSION_ROLLUP_CONTRACT_VERSION,
            "conversation_id",
            input["conversation_id"].clone(),
        ),
        _ => {
            return Err(WikiLlmError::InvalidOutput(
                "test must explicitly supply output for this stage".into(),
            ))
        }
    };
    let mut value = serde_json::json!({
        "schema":schema,"title":"Fixed list API cursor pagination",
        "body":"The agent edited the list handler to use cursor pagination.\n\nNo file changes in the snapshot were independently verified. This is not user mastery.",
        "nothing_to_summarize":false,"reason_code":null,"warnings":[]
    });
    value[key] = identity;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A scripted local HTTP endpoint replaces only the paid provider; the
    // production runner, hook, filesystem policy and native read tool are real.
    async fn scripted_model(script: Vec<Value>) -> (BoundWikiModel, Arc<Mutex<Vec<Value>>>) {
        use axum::{
            extract::Json,
            http::{header, StatusCode},
            routing::post,
            Router,
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let replies = Arc::new(Mutex::new(script));
        let app = Router::new().fallback(post(move |Json(body): Json<Value>| {
            let received = received.clone();
            let replies = replies.clone();
            async move {
                received.lock().unwrap().push(body);
                let reply = replies.lock().unwrap().remove(0);
                let delta = reply.get("delta").cloned().unwrap_or(serde_json::json!({"content":"{}"}));
                let finish = if delta.get("tool_calls").is_some() { "tool_calls" } else { "stop" };
                let first = serde_json::json!({"id":"wiki-test","object":"chat.completion.chunk","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let last = serde_json::json!({"id":"wiki-test","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":finish}],"usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}});
                (StatusCode::OK, [(header::CONTENT_TYPE,"text/event-stream")], if reply.get("broken").and_then(Value::as_bool) == Some(true) { format!("data: {first}\n\ndata: invalid-json\n\n") } else { format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n") })
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let bound = BoundWikiModel {
            client: CodegLlmClient::Completions(
                crate::agent::model::completions_client("test-key", format!("http://{address}/v1"))
                    .unwrap(),
            ),
            model_id: "test-model".into(),
            protocol: WireProtocol::ChatCompletions,
            context_window: 128_000,
            max_output: 8_192,
        };
        (bound, requests)
    }

    #[tokio::test]
    async fn production_runner_delivers_required_file_to_model() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(vault.join("raw/sessions")).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(
            vault.join("raw/sessions/input.md"),
            "HOST_READ_PROOF\nsecond line\n",
        )
        .unwrap();
        let (bound, requests) = scripted_model(vec![
            serde_json::json!({"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"read-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"raw/sessions/input.md\"}"}}]}}),
            serde_json::json!({"delta":{"role":"assistant","content":"{\"title\":\"Read material\",\"body\":\"Read complete\"}"}}),
        ]).await;
        let llm = ProductionWikiLlm::new(bound, "Read the file".into(), None);
        let run = llm.complete_json("turn_summary", serde_json::json!({"vault_abs":vault,"staging_abs":staging,"max_turns":4,"source_references":[{"rel":"raw/sessions/input.md","source_ids":["source"]}]})).await.unwrap();
        assert_eq!(run.output["title"], "Read material");
        let observed = requests.lock().unwrap();
        assert_eq!(observed.len(), 2);
        assert!(
            observed[1].to_string().contains("HOST_READ_PROOF"),
            "actual tool text must reach the next model call: {}",
            observed[1]
        );
        assert!(!observed[1].to_string().contains("call identity rejected"));
    }

    #[tokio::test]
    async fn interrupted_stream_never_accepts_partial_json_as_final_output() {
        let (bound, _) = scripted_model(vec![serde_json::json!({"broken":true,"delta":{"role":"assistant","content":"{\"valid_looking\":true}"}})]).await;
        let model = ProductionWikiLlm::new(bound, "JSON".into(), None);
        let error = model
            .complete_json("turn_summary", serde_json::json!({}))
            .await
            .expect_err("unfinished stream must fail");
        assert_eq!(error.error_code(), "model_failed");
    }

    #[tokio::test]
    async fn model_can_return_no_content_without_read_coverage_receipts() {
        let (bound, _) = scripted_model(vec![serde_json::json!({"delta":{"role":"assistant","content":"{\"nothing_to_summarize\":true,\"reason_code\":\"no_durable_content\"}"}})]).await;
        let model = ProductionWikiLlm::new(bound, "JSON".into(), None);
        let run = model
            .complete_json(
                "turn_summary",
                serde_json::json!({
                    "source_references":[{"rel":"raw/optional.md","source_ids":["source"]}]
                }),
            )
            .await
            .unwrap();
        assert_eq!(run.output["nothing_to_summarize"], true);
    }

    #[tokio::test]
    async fn unavailable_source_is_reported_without_blocking_the_agent_result() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        let (bound, _) = scripted_model(vec![
            serde_json::json!({"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"read-missing","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"raw/missing.md\"}"}}]}}),
            serde_json::json!({"delta":{"role":"assistant","content":"{\"title\":\"Claimed success\",\"body\":\"Completed\"}"}}),
        ]).await;
        let model = ProductionWikiLlm::new(bound, "JSON".into(), None);
        let run = model
            .complete_json(
                "turn_summary",
                serde_json::json!({
                    "vault_abs":vault,"staging_abs":staging,"max_turns":4,
                    "source_references":[{"rel":"raw/missing.md","source_ids":["source"]}]
                }),
            )
            .await
            .unwrap();
        assert!(run.output["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str().unwrap().contains("raw/missing.md")));
    }

    #[tokio::test]
    async fn missing_or_invalid_wiki_binding_never_falls_back_to_chat() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let missing = bind_wiki_model(&db.conn, None, None).await.err().unwrap();
        assert_eq!(missing.error_code(), "model_unavailable");
        let invalid = bind_wiki_model(&db.conn, Some(999999), Some("not-configured"))
            .await
            .err()
            .unwrap();
        assert_eq!(invalid.error_code(), "model_unavailable");
    }

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
    fn custom_prompt_cannot_remove_stage_schema_or_source_contract() {
        for stage in ["turn_summary", "session_rollup", "synthesize"] {
            let preamble = resolve_wiki_stage_preamble(
                stage,
                "editorial instructions",
                Some("Write briefly."),
            );
            assert!(preamble.starts_with("Write briefly."));
            assert!(preamble.contains("\"schema\""));
            assert!(preamble.contains("source_references"));
            assert!(!preamble.contains("Read every required input completely"));
        }
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

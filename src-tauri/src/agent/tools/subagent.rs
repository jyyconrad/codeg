//! In-process async `subagent` tool. Spawns a read-only inner runner and
//! returns immediately; the session shell injects the result later.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::agent::model::CodegLlmClient;
use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, RequestPatch};
use rig::client::AgentClientExt;
use rig::completion::{Document, Message};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::codegraph::should_inject_codegraph;
use super::{
    schema_for, CodegraphTool, GlobTool, GrepTool, NativeToolCtx, ReadFileTool, RecallTool,
    SkillCatalog, SkillTool, WriteExploreReportTool,
};
use crate::acp::process_owner::ProcessOwnerRegistry;
use crate::agent::code_intel::{load_code_intel_config, resolve_codegraph_binary, CodeIntelConfig};
use crate::agent::context::budget::BudgetConfig;
use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder};
use crate::agent::hook::{CodegHook, HookTrace, NativeRunState};
use crate::agent::mode::{explore_path, format_explore_handoff, EXPLORE_PREAMBLE_INTRO};
use crate::agent::model::{NativeTurnOutcome, DEFAULT_INVALID_TOOL_CALL_RETRIES};

/// Inner subagent turn budget (initial call plus tool retries).
pub const SUBAGENT_MAX_TURNS: usize = 12;
/// Cap on the parent preamble copy forwarded to the inner agent.
pub const SUBAGENT_PARENT_PREAMBLE_MAX: usize = 2048;
/// `RequestPatch.extra_context` document id (U-K8).
pub const SUBAGENT_SPEC_ID: &str = "codeg-subagent-spec";

pub const SUBAGENT_SPEC_TEXT: &str = "\
When to use the subagent tool:
- Use `subagent` (type explore) for time-consuming research, broad codebase search, or multi-file investigation.
- For a short read of a known path, call `read_file` yourself instead of spawning a subagent.
- The call returns immediately after the subagent starts (`started`). Do not assume that tool result contains the report.
- Later a user message beginning with `Explore report ready.` arrives. It contains `path:` and `summary:` only.
- Read the file at `path` with `read_file` before planning or implementing. The summary is not a substitute for the report.
- Do not poll or start a second parallel subagent. Only one subagent may run at a time.\
";

/// Internal inject sent on the supervisor's dedicated channel. Not a
/// `ConnectionCommand` (that enum is shared with Claude/Codex).
#[derive(Debug, Clone)]
pub enum NativeInject {
    SubagentFinished {
        id: String,
        ok: bool,
        output: String,
        tool_call_id: String,
    },
}

struct InflightSubagent {
    id: String,
    cancel: CancellationToken,
    handle: Option<JoinHandle<()>>,
}

/// Supervisor-owned cap-1 table. The tool registers under the mutex; the
/// command loop cancels / clears on Cancel, Disconnect, and inject apply.
#[derive(Default)]
pub struct SubagentTable {
    inflight: Option<InflightSubagent>,
}

impl SubagentTable {
    pub fn is_inflight(&self) -> bool {
        self.inflight.is_some()
    }

    pub fn inflight_id(&self) -> Option<&str> {
        self.inflight.as_ref().map(|slot| slot.id.as_str())
    }

    /// Cancel the running child and drop the slot (cap 1 is free). The task
    /// is left running until it observes the token; it must not inject.
    pub fn cancel_inflight(&mut self) {
        if let Some(slot) = self.inflight.take() {
            slot.cancel.cancel();
        }
    }

    /// Session teardown: cancel and abort leftover inner tasks.
    pub fn shutdown(&mut self) {
        if let Some(mut slot) = self.inflight.take() {
            slot.cancel.cancel();
            if let Some(handle) = slot.handle.take() {
                handle.abort();
            }
        }
    }

    fn try_register(&mut self, id: String, cancel: CancellationToken) -> bool {
        if self.inflight.is_some() {
            return false;
        }
        self.inflight = Some(InflightSubagent {
            id,
            cancel,
            handle: None,
        });
        true
    }

    fn attach_handle(&mut self, id: &str, handle: JoinHandle<()>) {
        if let Some(slot) = self.inflight.as_mut() {
            if slot.id == id {
                slot.handle = Some(handle);
            }
        }
    }

    pub fn clear_if(&mut self, id: &str) {
        if self.inflight.as_ref().is_some_and(|slot| slot.id == id) {
            self.inflight = None;
        }
    }
}

/// Built-in usage spec injected via `RequestPatch.extra_context` when this
/// tool is on the runner. Not vector `dynamic_context`.
pub fn subagent_spec_document() -> Document {
    Document {
        id: SUBAGENT_SPEC_ID.to_string(),
        text: SUBAGENT_SPEC_TEXT.to_string(),
        additional_props: Default::default(),
    }
}

pub fn attach_subagent_extra_context(patch: RequestPatch, tool_schemas: &[Value]) -> RequestPatch {
    if tool_schemas
        .iter()
        .any(|schema| schema.get("name").and_then(Value::as_str) == Some(SubagentTool::NAME))
    {
        patch.context(subagent_spec_document())
    } else {
        patch
    }
}

pub fn subagent_preamble(parent: &str, thoroughness: &str, skill_body: Option<&str>) -> String {
    let parent = truncate_to_bytes(parent, SUBAGENT_PARENT_PREAMBLE_MAX);
    let mut text = format!("{EXPLORE_PREAMBLE_INTRO}\nThoroughness: {thoroughness}.\n\n{parent}");
    if let Some(body) = skill_body.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n# Preloaded explore skill\n\n");
        text.push_str(body);
    }
    text
}

fn truncate_to_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Host-facing async subagent. Inner runner is read-only and does not share
/// the parent identity bridge (parent tools keep running after start).
#[derive(Clone)]
pub struct SubagentTool {
    ctx: NativeToolCtx,
    client: CodegLlmClient,
    model_id: String,
    parent_preamble: String,
    catalog: SkillCatalog,
    budget: BudgetConfig,
    table: Arc<Mutex<SubagentTable>>,
    inject_tx: mpsc::Sender<NativeInject>,
    artifacts_dir: PathBuf,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
}

impl SubagentTool {
    pub fn new(
        ctx: NativeToolCtx,
        client: CodegLlmClient,
        model_id: impl Into<String>,
        parent_preamble: impl Into<String>,
        catalog: SkillCatalog,
        budget: BudgetConfig,
        table: Arc<Mutex<SubagentTable>>,
        inject_tx: mpsc::Sender<NativeInject>,
        artifacts_dir: PathBuf,
    ) -> Self {
        Self {
            ctx,
            client,
            model_id: model_id.into(),
            parent_preamble: parent_preamble.into(),
            catalog,
            budget,
            table,
            inject_tx,
            artifacts_dir,
            owners: None,
        }
    }

    pub fn with_owners(mut self, owners: Arc<Mutex<ProcessOwnerRegistry>>) -> Self {
        self.owners = Some(owners);
        self
    }
}

#[derive(Debug, Deserialize)]
pub struct SubagentArgs {
    pub prompt: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub subagent_type: Option<String>,
    #[serde(default)]
    pub thoroughness: Option<String>,
}

fn normalize_explore_type(raw: Option<&str>) -> Result<(), String> {
    let value = raw.unwrap_or("explore").trim();
    if value.is_empty() || value.eq_ignore_ascii_case("explore") {
        Ok(())
    } else {
        Err(format!(
            "unsupported subagent_type `{value}`; only `explore` is available"
        ))
    }
}

fn normalize_thoroughness(raw: Option<&str>) -> String {
    match raw.unwrap_or("medium").trim().to_ascii_lowercase().as_str() {
        "quick" => "quick".into(),
        "very_thorough" | "very-thorough" | "very thorough" => "very_thorough".into(),
        _ => "medium".into(),
    }
}

impl Tool for SubagentTool {
    const NAME: &'static str = "subagent";
    type Args = SubagentArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Start a background explore subagent. Returns immediately after start. \
         When it finishes, a user message with path and summary arrives; read \
         the report file with read_file. Only one subagent may run at a time."
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Task for the subagent"
                },
                "label": {
                    "type": "string",
                    "description": "Optional card title; defaults to a truncated prompt"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Must be `explore` (the only type in this release)"
                },
                "thoroughness": {
                    "type": "string",
                    "description": "quick | medium | very_thorough"
                }
            },
            "required": ["prompt"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "prompt": args.prompt,
            "label": args.label,
            "subagent_type": args.subagent_type,
            "thoroughness": args.thoroughness,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        let prompt = args.prompt.trim();
        if prompt.is_empty() {
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::invalid_args("prompt must not be empty")
                        .with_model_feedback("prompt must not be empty"),
                )
                .await);
        }
        if let Err(message) = normalize_explore_type(args.subagent_type.as_deref()) {
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::invalid_args(message.clone()).with_model_feedback(message),
                )
                .await);
        }
        let thoroughness = normalize_thoroughness(args.thoroughness.as_deref());
        let id = format!("sa-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let child = self.ctx.cancel.child_token();
        let registered = {
            let mut table = self.table.lock().expect("subagent table");
            table.try_register(id.clone(), child.clone())
        };
        if !registered {
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::other("a subagent is already running").with_model_feedback(
                        "a subagent is already running; wait for it to finish before starting another",
                    ),
                )
                .await);
        }

        let spawn = InnerSpawn {
            id: id.clone(),
            prompt: prompt.to_string(),
            child: child.clone(),
            parent_cancel: self.ctx.cancel.clone(),
            client: self.client.clone(),
            model_id: self.model_id.clone(),
            parent_preamble: self.parent_preamble.clone(),
            catalog: self.catalog.clone(),
            budget: self.budget,
            launch_cwd: self.ctx.launch_cwd.clone(),
            fs: Arc::clone(&self.ctx.fs),
            session_id: self.ctx.session_id.clone(),
            table: Arc::clone(&self.table),
            inject_tx: self.inject_tx.clone(),
            tool_call_id: fact.tool_call_id.clone(),
            artifacts_dir: self.artifacts_dir.clone(),
            thoroughness,
            owners: self.owners.clone(),
        };
        let handle = tokio::spawn(run_inner_and_inject(spawn));
        self.table
            .lock()
            .expect("subagent table")
            .attach_handle(&id, handle);

        let presentation = format!("subagent {id} started");
        self.ctx.finish_ok(fact, presentation).await
    }
}

struct InnerSpawn {
    id: String,
    prompt: String,
    child: CancellationToken,
    parent_cancel: CancellationToken,
    client: CodegLlmClient,
    model_id: String,
    parent_preamble: String,
    catalog: SkillCatalog,
    budget: BudgetConfig,
    launch_cwd: std::path::PathBuf,
    fs: Arc<crate::acp::file_system_runtime::FileSystemRuntime>,
    session_id: String,
    table: Arc<Mutex<SubagentTable>>,
    inject_tx: mpsc::Sender<NativeInject>,
    tool_call_id: String,
    artifacts_dir: PathBuf,
    thoroughness: String,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
}

async fn run_inner_and_inject(spawn: InnerSpawn) {
    let InnerSpawn {
        id,
        prompt,
        child,
        parent_cancel,
        client,
        model_id,
        parent_preamble,
        catalog,
        budget,
        launch_cwd,
        fs,
        session_id,
        table,
        inject_tx,
        tool_call_id,
        artifacts_dir,
        thoroughness,
        owners,
    } = spawn;

    let (outcome, text) = tokio::select! {
        _ = parent_cancel.cancelled() => (NativeTurnOutcome::Cancelled, String::new()),
        _ = child.cancelled() => (NativeTurnOutcome::Cancelled, String::new()),
        result = run_inner_subagent(
            client,
            model_id,
            parent_preamble,
            prompt,
            catalog,
            budget,
            launch_cwd,
            fs,
            session_id,
            child.clone(),
            artifacts_dir.clone(),
            thoroughness,
            owners,
        ) => result,
    };

    let cancelled = child.is_cancelled()
        || parent_cancel.is_cancelled()
        || matches!(outcome, NativeTurnOutcome::Cancelled);
    if cancelled {
        table.lock().expect("subagent table").clear_if(&id);
        return;
    }

    let (ok, output) = match outcome {
        NativeTurnOutcome::Complete => {
            let path = explore_path(&artifacts_dir);
            let status = if path.is_file()
                && std::fs::metadata(&path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
            {
                "ok"
            } else {
                "missing_report"
            };
            (true, format_explore_handoff(&path, status, &text))
        }
        NativeTurnOutcome::Failed(message) => {
            let path = explore_path(&artifacts_dir);
            (false, format_explore_handoff(&path, "failed", &message))
        }
        NativeTurnOutcome::Cancelled => {
            table.lock().expect("subagent table").clear_if(&id);
            return;
        }
    };
    if child.is_cancelled() || parent_cancel.is_cancelled() {
        table.lock().expect("subagent table").clear_if(&id);
        return;
    }
    let _ = inject_tx
        .send(NativeInject::SubagentFinished {
            id: id.clone(),
            ok,
            output,
            tool_call_id,
        })
        .await;
    table.lock().expect("subagent table").clear_if(&id);
}

/// Same injection predicate as the parent: master + codegraph enabled.
/// A missing binary still injects so the inner model sees the install hint.
fn inner_codegraph_tool(cfg: &CodeIntelConfig, _binary: Option<&Path>) -> bool {
    should_inject_codegraph(cfg)
}

fn inner_codegraph_path(binary: Option<&Path>) -> PathBuf {
    binary
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("codegraph"))
}

fn build_inner_codegraph(
    ctx: NativeToolCtx,
    cfg: &CodeIntelConfig,
    binary: Option<&Path>,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
) -> Option<CodegraphTool> {
    inner_codegraph_tool(cfg, binary)
        .then(|| CodegraphTool::new(ctx, inner_codegraph_path(binary), owners))
}

fn inner_tool_schemas(
    read: &ReadFileTool,
    recall: &RecallTool,
    glob: &GlobTool,
    grep: &GrepTool,
    skill: &SkillTool,
    write_explore: &WriteExploreReportTool,
    codegraph: Option<&CodegraphTool>,
) -> Vec<Value> {
    let mut schemas = vec![
        schema_for(read),
        schema_for(recall),
        schema_for(glob),
        schema_for(grep),
        schema_for(skill),
        schema_for(write_explore),
    ];
    if let Some(tool) = codegraph {
        schemas.push(schema_for(tool));
    }
    schemas
}

#[allow(clippy::too_many_arguments)]
async fn run_inner_subagent(
    client: CodegLlmClient,
    model_id: String,
    parent_preamble: String,
    prompt: String,
    catalog: SkillCatalog,
    budget: BudgetConfig,
    launch_cwd: std::path::PathBuf,
    fs: Arc<crate::acp::file_system_runtime::FileSystemRuntime>,
    session_id: String,
    cancel: CancellationToken,
    artifacts_dir: PathBuf,
    thoroughness: String,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
) -> (NativeTurnOutcome, String) {
    let identity = Arc::new(CallIdentityBridge::new());
    let store = Arc::new(Mutex::new(ContextStore::new(format!("sub:{session_id}"))));
    let recorder = Arc::new(FactRecorder::memory(Arc::clone(&store)));
    let inner_ctx = NativeToolCtx {
        turn_id: 1,
        identity: Arc::clone(&identity),
        recorder: Arc::clone(&recorder),
        cancel: cancel.clone(),
        launch_cwd,
        fs,
        session_id,
        spill_dir: PathBuf::new(),
    };
    let intel = load_code_intel_config();
    let resolved = resolve_codegraph_binary(&intel.codegraph);
    let codegraph = build_inner_codegraph(inner_ctx.clone(), &intel, resolved.as_deref(), owners);
    let read = ReadFileTool::new(inner_ctx.clone());
    let recall = RecallTool::new(inner_ctx.clone());
    let glob = GlobTool::new(inner_ctx.clone());
    let grep = GrepTool::new(inner_ctx.clone());
    let skill = SkillTool::new(inner_ctx.clone(), catalog.clone());
    let write_explore = WriteExploreReportTool::new(inner_ctx, artifacts_dir);
    let skill_body = catalog.skill_body("explore");
    let preamble = subagent_preamble(&parent_preamble, &thoroughness, skill_body.as_deref());
    let tool_schemas = inner_tool_schemas(
        &read,
        &recall,
        &glob,
        &grep,
        &skill,
        &write_explore,
        codegraph.as_ref(),
    );
    let native = NativeRunState {
        turn_id: 1,
        turn_key: "sub:1".into(),
        budget,
        preamble: preamble.clone(),
        tool_schemas,
        store,
        identity,
        recorder,
        last_estimate: Arc::new(Mutex::new(0)),
        last_usage_input: Arc::new(Mutex::new(None)),
        feedback: None,
        mcp_readonly: Arc::new(HashSet::new()),
        compact: None,
    };
    let (perm_tx, _perm_rx) = mpsc::channel(1);
    let trace = HookTrace::new();
    let hook = CodegHook::waiting(trace.clone(), perm_tx, cancel.clone()).with_native(native);

    let user_prompt = Message::user(prompt);
    let stream = tokio::select! {
        _ = cancel.cancelled() => return (NativeTurnOutcome::Cancelled, String::new()),
        stream = async {
            match client {
                CodegLlmClient::Completions(client) => {
                    assemble_inner(
                        client,
                        model_id,
                        preamble,
                        user_prompt,
                        read,
                        recall,
                        glob,
                        grep,
                        skill,
                        write_explore,
                        codegraph,
                        hook,
                    )
                    .await
                }
                CodegLlmClient::Responses(client) => {
                    assemble_inner(
                        client,
                        model_id,
                        preamble,
                        user_prompt,
                        read,
                        recall,
                        glob,
                        grep,
                        skill,
                        write_explore,
                        codegraph,
                        hook,
                    )
                    .await
                }
            }
        } => stream,
    };
    let outcome = drain_inner(stream, cancel).await;
    let mut text = trace.aggregated_text();
    if text.trim().is_empty() {
        if let Some(last) = trace.tool_results().last() {
            text = last.presentation.clone();
        }
    }
    (outcome, text)
}

/// Assemble a read-only inner agent. No write/edit/bash/mcp/companion/plan/subagent.
#[allow(clippy::too_many_arguments)]
async fn assemble_inner<C>(
    client: C,
    model_id: String,
    preamble: String,
    prompt: Message,
    read: ReadFileTool,
    recall: RecallTool,
    glob: GlobTool,
    grep: GrepTool,
    skill: SkillTool,
    write_explore: WriteExploreReportTool,
    codegraph: Option<CodegraphTool>,
    hook: CodegHook,
) -> rig::agent::StreamingResult
where
    C: AgentClientExt + Send,
    C::CompletionModel: 'static,
{
    let mut builder = client
        .agent(&model_id)
        .preamble(&preamble)
        .default_max_turns(SUBAGENT_MAX_TURNS)
        .tool(read)
        .tool(recall)
        .tool(glob)
        .tool(grep)
        .tool(skill)
        .tool(write_explore);
    if let Some(codegraph) = codegraph {
        builder = builder.tool(codegraph);
    }
    builder
        .build()
        .runner(prompt)
        .history(Vec::<Message>::new())
        .max_turns(SUBAGENT_MAX_TURNS)
        .tool_concurrency(1)
        .max_invalid_tool_call_retries(DEFAULT_INVALID_TOOL_CALL_RETRIES)
        .add_hook(hook)
        .stream()
        .await
}

async fn drain_inner(
    mut stream: rig::agent::StreamingResult,
    cancel: CancellationToken,
) -> NativeTurnOutcome {
    loop {
        match stream.next().await {
            None => {
                return if cancel.is_cancelled() {
                    NativeTurnOutcome::Cancelled
                } else {
                    NativeTurnOutcome::Complete
                };
            }
            Some(Ok(MultiTurnStreamItem::FinalResponse(_))) => {
                return if cancel.is_cancelled() {
                    NativeTurnOutcome::Cancelled
                } else {
                    NativeTurnOutcome::Complete
                };
            }
            Some(Err(err)) => {
                let message = err.to_string();
                if cancel.is_cancelled() || message.to_ascii_lowercase().contains("cancel") {
                    return NativeTurnOutcome::Cancelled;
                }
                return NativeTurnOutcome::Failed(message);
            }
            Some(Ok(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::{completions_client, CodegLlmClient};
    use crate::agent::tools::{test_tool_ctx, tool_kind, tool_requires_permission};
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use rig::tool::Tool;
    use serde_json::json;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn budget() -> BudgetConfig {
        BudgetConfig::new(128_000, 4096)
    }

    async fn spawn_completions(hang: bool) -> String {
        let hang = Arc::new(hang);
        let app = Router::new().fallback(post(move |uri: Uri, Json(_body): Json<Value>| {
            let hang = Arc::clone(&hang);
            async move {
                let _ = uri;
                if *hang {
                    std::future::pending::<()>().await;
                }
                let sse = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"inner-ok\"}}]}\n\n\
                     data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n\
                     data: [DONE]\n\n";
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    sse.to_string(),
                )
                    .into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}/v1")
    }

    fn harness(
        base: &str,
        cancel: CancellationToken,
    ) -> (
        SubagentTool,
        mpsc::Receiver<NativeInject>,
        Arc<Mutex<SubagentTable>>,
    ) {
        let mut ctx = test_tool_ctx(Path::new("/tmp"), SubagentTool::NAME, "call_sa");
        ctx.cancel = cancel;
        let client = CodegLlmClient::Completions(completions_client("sk", base).expect("client"));
        let table = Arc::new(Mutex::new(SubagentTable::default()));
        let (tx, rx) = mpsc::channel(4);
        let tool = SubagentTool::new(
            ctx,
            client,
            "m",
            "You are Codeg Agent.",
            SkillCatalog::default(),
            budget(),
            Arc::clone(&table),
            tx,
            PathBuf::from("/tmp"),
        );
        (tool, rx, table)
    }

    #[test]
    fn subagent_is_other_kind_and_needs_permission() {
        assert_eq!(tool_kind("subagent"), "other");
        assert!(tool_requires_permission("subagent"));
    }

    #[test]
    fn extra_context_helper_attaches_only_when_registered() {
        let patch = RequestPatch::new();
        let none = attach_subagent_extra_context(patch.clone(), &[]);
        assert!(none.extra_context.is_empty());
        let other = attach_subagent_extra_context(patch.clone(), &[json!({"name": "read_file"})]);
        assert!(other.extra_context.is_empty());
        let with = attach_subagent_extra_context(patch, &[json!({"name": "subagent"})]);
        assert_eq!(with.extra_context.len(), 1);
        assert_eq!(with.extra_context[0].id, SUBAGENT_SPEC_ID);
        assert!(with.extra_context[0].text.contains("Explore report ready"));
        assert!(with.extra_context[0]
            .text
            .contains("Only one subagent may run at a time"));
    }

    #[test]
    fn parent_preamble_is_truncated_to_2kib() {
        let parent = "x".repeat(4096);
        let text = subagent_preamble(&parent, "medium", None);
        assert!(text.starts_with(EXPLORE_PREAMBLE_INTRO));
        assert!(text.contains("Thoroughness: medium"));
        assert!(text.len() <= EXPLORE_PREAMBLE_INTRO.len() + 64 + SUBAGENT_PARENT_PREAMBLE_MAX);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn started_returns_immediately_while_inner_still_running() {
        let base = spawn_completions(true).await;
        let cancel = CancellationToken::new();
        let (tool, mut inject_rx, table) = harness(&base, cancel);
        let mut tctx = ToolContext::new();
        let out = tokio::time::timeout(
            Duration::from_secs(2),
            tool.call(
                &mut tctx,
                SubagentArgs {
                    prompt: "research the tree".into(),
                    label: Some("research".into()),
                    subagent_type: None,
                    thoroughness: None,
                },
            ),
        )
        .await
        .expect("tool must not wait for the inner runner")
        .expect("started");
        assert!(
            out.starts_with("subagent ") && out.ends_with(" started"),
            "{out}"
        );
        assert!(table
            .lock()
            .expect("table")
            .inflight_id()
            .is_some_and(|id| id.starts_with("sa-")));
        assert!(
            inject_rx.try_recv().is_err(),
            "inner still running; no inject yet"
        );
        table.lock().expect("table").shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cap_one_rejects_second_call() {
        let base = spawn_completions(true).await;
        let cancel = CancellationToken::new();
        let (tool, _rx, table) = harness(&base, cancel);
        let mut tctx = ToolContext::new();
        tool.call(
            &mut tctx,
            SubagentArgs {
                prompt: "first".into(),
                label: None,
                subagent_type: None,
                thoroughness: None,
            },
        )
        .await
        .expect("first starts");
        let err = tool
            .call(
                &mut tctx,
                SubagentArgs {
                    prompt: "second".into(),
                    label: None,
                    subagent_type: None,
                    thoroughness: None,
                },
            )
            .await
            .expect_err("cap 1");
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("already running"),
            "{err:?}"
        );
        table.lock().expect("table").shutdown();
    }

    #[tokio::test]
    async fn rejects_non_explore_type() {
        let base = spawn_completions(false).await;
        let cancel = CancellationToken::new();
        let (tool, _rx, table) = harness(&base, cancel);
        let mut tctx = ToolContext::new();
        let err = tool
            .call(
                &mut tctx,
                SubagentArgs {
                    prompt: "x".into(),
                    label: None,
                    subagent_type: Some("general-purpose".into()),
                    thoroughness: None,
                },
            )
            .await
            .expect_err("type");
        assert!(
            err.model_feedback().unwrap_or_default().contains("explore"),
            "{err:?}"
        );
        table.lock().expect("table").shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn finished_inner_sends_inject() {
        let base = spawn_completions(false).await;
        let cancel = CancellationToken::new();
        let (tool, mut inject_rx, table) = harness(&base, cancel);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                SubagentArgs {
                    prompt: "summarize".into(),
                    label: None,
                    subagent_type: Some("explore".into()),
                    thoroughness: None,
                },
            )
            .await
            .expect("started");
        assert!(out.contains("started"), "{out}");
        let inject = tokio::time::timeout(Duration::from_secs(5), inject_rx.recv())
            .await
            .expect("inject timeout")
            .expect("inject closed");
        match inject {
            NativeInject::SubagentFinished { ok, output, id, .. } => {
                assert!(ok, "output={output}");
                assert!(output.contains("Explore report ready"), "{output}");
                assert!(output.contains("path:"), "{output}");
                assert!(output.contains("summary:"), "{output}");
                assert!(
                    output.contains("inner-ok") || output.contains("(no summary)"),
                    "{output}"
                );
                assert!(!output.contains("# huge"), "{output}");
                assert!(id.starts_with("sa-"), "{id}");
            }
        }
        assert!(!table.lock().expect("table").is_inflight());
    }

    fn inner_tool_names(
        cfg: &crate::agent::code_intel::CodeIntelConfig,
        binary: Option<&Path>,
    ) -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "inner", "c1");
        let read = ReadFileTool::new(ctx.clone());
        let recall = RecallTool::new(ctx.clone());
        let glob = GlobTool::new(ctx.clone());
        let grep = GrepTool::new(ctx.clone());
        let skill = SkillTool::new(ctx.clone(), SkillCatalog::default());
        let write_explore = WriteExploreReportTool::new(ctx.clone(), dir.path().to_path_buf());
        let codegraph = build_inner_codegraph(ctx, cfg, binary, None);
        inner_tool_schemas(
            &read,
            &recall,
            &glob,
            &grep,
            &skill,
            &write_explore,
            codegraph.as_ref(),
        )
        .iter()
        .filter_map(|schema| {
            schema
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
    }

    #[test]
    fn inner_codegraph_tool_follows_parent_injection_predicate() {
        let mut cfg = crate::agent::code_intel::default_config();
        assert!(!inner_codegraph_tool(&cfg, None));
        cfg.enabled = true;
        assert!(inner_codegraph_tool(&cfg, None));
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake-codegraph");
        std::fs::write(&binary, "ok").unwrap();
        assert!(inner_codegraph_tool(&cfg, Some(binary.as_path())));
        cfg.codegraph.enabled = false;
        assert!(!inner_codegraph_tool(&cfg, Some(binary.as_path())));
    }

    #[test]
    fn inner_schema_includes_codegraph_when_enabled_and_binary_exists() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake-codegraph");
        std::fs::write(&binary, "ok").unwrap();
        let mut cfg = crate::agent::code_intel::default_config();
        cfg.enabled = true;
        let names = inner_tool_names(&cfg, Some(binary.as_path()));
        assert!(names.iter().any(|n| n == "codegraph"), "{names:?}");
        assert!(names.iter().any(|n| n == "read_file"), "{names:?}");
        assert!(names.iter().any(|n| n == "grep"), "{names:?}");
        for forbidden in [
            "write_file",
            "edit_file",
            "bash",
            "subagent",
            "lsp",
            "update_plan",
            "write_plan",
            "enter_plan_mode",
            "exit_plan_mode",
        ] {
            assert!(
                !names.iter().any(|n| n == forbidden),
                "{names:?} contains {forbidden}"
            );
        }
    }

    #[test]
    fn inner_schema_includes_codegraph_when_enabled_without_binary() {
        let mut cfg = crate::agent::code_intel::default_config();
        cfg.enabled = true;
        assert!(inner_codegraph_tool(&cfg, None));
        let names = inner_tool_names(&cfg, None);
        assert!(names.iter().any(|n| n == "codegraph"), "{names:?}");
    }

    #[test]
    fn inner_schema_omits_codegraph_when_disabled() {
        let cfg = crate::agent::code_intel::default_config();
        assert!(!inner_codegraph_tool(&cfg, None));
        let names = inner_tool_names(&cfg, None);
        assert!(!names.iter().any(|n| n == "codegraph"), "{names:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_parent_token_stops_inner_without_inject() {
        let base = spawn_completions(true).await;
        let cancel = CancellationToken::new();
        let (tool, mut inject_rx, table) = harness(&base, cancel.clone());
        let mut tctx = ToolContext::new();
        tool.call(
            &mut tctx,
            SubagentArgs {
                prompt: "hang please".into(),
                label: None,
                subagent_type: None,
                thoroughness: None,
            },
        )
        .await
        .expect("started");
        assert!(table.lock().expect("table").is_inflight());
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if !table.lock().expect("table").is_inflight() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("inner should observe parent cancel");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            inject_rx.try_recv().is_err(),
            "cancelled inner must not inject"
        );
    }
}

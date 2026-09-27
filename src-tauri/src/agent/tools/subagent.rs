//! In-process async `subagent` tool. Spawns a read-only inner runner and
//! returns immediately; the session shell injects the result later.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::agent::model::CodegLlmClient;
use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, RequestPatch};
use rig::completion::{Document, Message};
use rig::tool::{DynamicTool, Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{
    schema_for, GlobTool, GrepTool, NativeToolCtx, ReadFileTool, RecallTool, SkillCatalog,
    SkillTool, WriteExploreReportTool,
};
use crate::acp::terminal_runtime::TerminalRuntime;
use crate::agent::code_intel::MCP_SERVER_NAME;
use crate::agent::context::budget::BudgetConfig;
use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder};
use crate::agent::hook::PendingPermission;
use crate::agent::hook::{CodegHook, HookTrace, NativeRunState};
use crate::agent::mode::{explore_path, format_explore_handoff, EXPLORE_PREAMBLE_INTRO};
use crate::agent::model::{NativeTurnOutcome, DEFAULT_INVALID_TOOL_CALL_RETRIES};
use crate::agent::tools::{BashTool, EditFileTool, WriteFileTool};

/// Inner subagent turn budget (initial call plus tool retries).
pub const SUBAGENT_MAX_TURNS: usize = 12;
/// `RequestPatch.extra_context` document id (U-K8).
pub const SUBAGENT_SPEC_ID: &str = "codeg-subagent-spec";
const REPORT_FOLLOWUP_PROMPT: &str = "Please use write_explore_report to write the complete exploration conclusion to the required report file now. Include the evidence, exact paths, verification results, open questions, and next steps. Do not finish until the report has been written.";

pub const SUBAGENT_SPEC_TEXT: &str = "\
When to use the subagent tool:
- Use `subagent` (type explore) for time-consuming research, broad codebase search, or multi-file investigation.
- For a short read of a known path, call `read_file` yourself instead of spawning a subagent.
- The call returns immediately after the subagent starts (`started`). Do not assume that tool result contains the report.
- Later a user message beginning with `Explore report ready.` arrives. It contains `path:` and `summary:` only.
- Read the file at `path` with `read_file` before planning or implementing. The summary is not a substitute for the report.
- If a report-enabled child finishes without a report, Codeg sends one follow-up user message asking it to write the report; a second miss is reported as `missing_report`.
- If the call explicitly sets `allowed_tools: []` or omits `write_explore_report`, the completion is text-only and has no report path to read.
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

/// Optional services inherited from the parent session. The supervisor owns
/// the permission receiver; the child only gets a sender clone so every ask
/// still appears in the parent UI. MCP bindings are reused but execute with a
/// child identity and cancellation token.
#[derive(Clone)]
pub struct ParentRuntime {
    pub terminals: Arc<TerminalRuntime>,
    pub mcp: Arc<crate::agent::tools::McpSession>,
    pub permissions: Option<mpsc::Sender<PendingPermission>>,
    pub auto_allow: bool,
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

pub fn subagent_preamble(
    parent: &str,
    thoroughness: &str,
    skill_body: Option<&str>,
    tree: Option<&str>,
) -> String {
    let mut text = format!("{EXPLORE_PREAMBLE_INTRO}\nThoroughness: {thoroughness}.\n\n{parent}");
    if let Some(tree) = tree.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n");
        text.push_str(tree);
    }
    if let Some(body) = skill_body.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n# Preloaded explore skill\n\n");
        text.push_str(body);
    }
    text
}

/// Host-facing async subagent. The default inner runner is read-only; a parent
/// runtime may opt into its write, shell, and MCP tools through an allowlist.
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
    workspace_tree: Option<String>,
    /// Models bound to the parent session and their context windows. A child
    /// may select only one of these entries.
    model_context_windows: BTreeMap<String, u32>,
    /// `None` keeps the default Explore tool set; `Some` is an explicit
    /// allowlist, including an empty allowlist (text-only child).
    allowed_tools: Option<HashSet<String>>,
    parent_runtime: Option<ParentRuntime>,
}

impl SubagentTool {
    #[allow(clippy::too_many_arguments)]
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
        let model_id = model_id.into();
        Self {
            ctx,
            client,
            model_id: model_id.clone(),
            parent_preamble: parent_preamble.into(),
            catalog,
            budget,
            table,
            inject_tx,
            artifacts_dir,
            workspace_tree: None,
            model_context_windows: {
                let mut models = BTreeMap::new();
                models.insert(model_id, budget.window.min(u32::MAX as u64) as u32);
                models
            },
            allowed_tools: None,
            parent_runtime: None,
        }
    }

    pub fn with_workspace_tree(mut self, tree: Option<String>) -> Self {
        self.workspace_tree = tree;
        self
    }

    /// Replace the model catalog inherited from the parent session. The map
    /// is also the source of truth for each selected model's context window.
    pub fn with_model_context_windows(mut self, windows: BTreeMap<String, u32>) -> Self {
        self.model_context_windows = windows;
        self
    }

    /// Set an explicit inner-tool allowlist. Passing an empty iterator is
    /// intentional and registers no tools, leaving only final text output.
    pub fn with_allowed_tools<I, S>(mut self, allowed: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_tools = Some(allowed.into_iter().map(Into::into).collect());
        self
    }

    /// Attach parent-scoped mutating tools and MCP bindings. The child still
    /// receives only names selected by `allowed_tools`/call arguments.
    pub fn with_parent_runtime(mut self, runtime: ParentRuntime) -> Self {
        self.parent_runtime = Some(runtime);
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
    /// Optional model selected from the parent session's bound catalog.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Optional allowlist of actual inner tool names. `[]` means no tools.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
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

fn available_inner_tool_names(parent_runtime: Option<&ParentRuntime>) -> Vec<String> {
    let mut names = vec![
        ReadFileTool::NAME,
        RecallTool::NAME,
        GlobTool::NAME,
        GrepTool::NAME,
        SkillTool::NAME,
        WriteExploreReportTool::NAME,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if let Some(runtime) = parent_runtime {
        names.extend([
            WriteFileTool::NAME.to_owned(),
            EditFileTool::NAME.to_owned(),
            BashTool::NAME.to_owned(),
        ]);
        names.extend(
            runtime
                .mcp
                .bindings()
                .iter()
                .map(|binding| binding.local_name.clone()),
        );
    }
    names
}

fn configured_inner_tool_names(
    allowed: Option<&HashSet<String>>,
    available: &[String],
) -> Vec<String> {
    available
        .iter()
        .filter(|name| allowed.is_none_or(|set| set.contains(name.as_str())))
        .cloned()
        .collect()
}

fn validate_inner_tool_allowlist(
    requested: Option<&[String]>,
    configured: Option<&HashSet<String>>,
    available: &[String],
    default_allowlist: Option<&HashSet<String>>,
) -> Result<Option<HashSet<String>>, String> {
    let Some(requested) = requested else {
        return Ok(match (configured, default_allowlist) {
            (Some(configured), Some(defaults)) => {
                Some(configured.intersection(defaults).cloned().collect())
            }
            (Some(configured), None) => Some(configured.clone()),
            (None, Some(defaults)) => Some(defaults.clone()),
            (None, None) => None,
        });
    };
    let available: HashSet<&str> = available.iter().map(String::as_str).collect();
    let mut selected = HashSet::new();
    for raw in requested {
        let name = raw.trim();
        if name.is_empty() {
            return Err("allowed_tools entries must not be empty".into());
        }
        if !available.contains(name) {
            return Err(format!("unknown subagent tool `{name}`"));
        }
        if configured.is_some_and(|set| !set.contains(name)) {
            return Err(format!(
                "subagent tool `{name}` is not allowed by the parent"
            ));
        }
        selected.insert(name.to_owned());
    }
    Ok(Some(selected))
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
        let available = available_inner_tool_names(self.parent_runtime.as_ref());
        let configured = configured_inner_tool_names(self.allowed_tools.as_ref(), &available);
        let mut schema = json!({
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
                    "description": "quick | medium | very_thorough",
                    "enum": ["quick", "medium", "very_thorough"]
                },
                "model_id": {
                    "type": "string",
                    "description": "Optional model from the parent session's bound model catalog",
                    "enum": self.model_context_windows.keys().collect::<Vec<_>>()
                },
                "allowed_tools": {
                    "type": "array",
                    "description": "Optional allowlist of inner tools; [] enables text-only completion",
                    "items": { "type": "string", "enum": configured }
                }
            },
            "required": ["prompt"]
        });
        // Keep an explicit enum for every known model, while allowing the
        // omitted field to inherit the parent's current model.
        if let Some(models) = schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .and_then(|props| props.get_mut("model_id"))
        {
            models["enum"] = Value::Array(
                self.model_context_windows
                    .keys()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            );
        }
        schema
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
            "model_id": args.model_id,
            "allowed_tools": args.allowed_tools,
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
        let selected_model = args
            .model_id
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(self.model_id.as_str());
        let Some(&selected_window) = self.model_context_windows.get(selected_model) else {
            let message = format!(
                "unknown subagent model `{selected_model}`; choose a model from the parent context window catalog"
            );
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::invalid_args(message.clone()).with_model_feedback(message),
                )
                .await);
        };
        if selected_window == 0 {
            let message =
                format!("subagent model `{selected_model}` has an invalid context window");
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::invalid_args(message.clone()).with_model_feedback(message),
                )
                .await);
        }
        let available = available_inner_tool_names(self.parent_runtime.as_ref());
        let default_allowlist = self.parent_runtime.as_ref().map(|runtime| {
            available
                .iter()
                .filter(|name| !excluded_from_default_explore(name, runtime))
                .cloned()
                .collect::<HashSet<_>>()
        });
        let allowed_tools = match validate_inner_tool_allowlist(
            args.allowed_tools.as_deref(),
            self.allowed_tools.as_ref(),
            &available,
            default_allowlist.as_ref(),
        ) {
            Ok(allowed) => allowed,
            Err(message) => {
                return Err(self
                    .ctx
                    .finish_err(
                        fact,
                        ToolExecutionError::invalid_args(message.clone())
                            .with_model_feedback(message),
                    )
                    .await)
            }
        };
        let id = format!("sa-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let report_enabled = allowed_tools
            .as_ref()
            .is_none_or(|set| set.contains(WriteExploreReportTool::NAME));
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
            model_id: selected_model.to_owned(),
            parent_preamble: self.parent_preamble.clone(),
            catalog: self.catalog.clone(),
            budget: BudgetConfig {
                window: selected_window.into(),
                ..self.budget
            },
            launch_cwd: self.ctx.launch_cwd.clone(),
            fs: Arc::clone(&self.ctx.fs),
            session_id: self.ctx.session_id.clone(),
            table: Arc::clone(&self.table),
            inject_tx: self.inject_tx.clone(),
            tool_call_id: fact.tool_call_id.clone(),
            artifacts_dir: self.artifacts_dir.join("subagents").join(&id),
            thoroughness,
            workspace_tree: self.workspace_tree.clone(),
            allowed_tools,
            parent_runtime: self.parent_runtime.clone(),
            report_enabled,
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
    workspace_tree: Option<String>,
    allowed_tools: Option<HashSet<String>>,
    parent_runtime: Option<ParentRuntime>,
    report_enabled: bool,
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
        workspace_tree,
        allowed_tools,
        parent_runtime,
        report_enabled,
    } = spawn;

    let (mut outcome, mut text) = run_inner_subagent(
        client.clone(),
        model_id.clone(),
        parent_preamble.clone(),
        prompt.clone(),
        Vec::new(),
        catalog.clone(),
        budget,
        launch_cwd.clone(),
        fs.clone(),
        session_id.clone(),
        child.clone(),
        artifacts_dir.clone(),
        thoroughness.clone(),
        workspace_tree.clone(),
        allowed_tools.clone(),
        parent_runtime.clone(),
    )
    .await;

    let report_path = explore_path(&artifacts_dir);
    let has_report = report_path.is_file()
        && std::fs::metadata(&report_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
    if !child.is_cancelled()
        && !parent_cancel.is_cancelled()
        && matches!(outcome, NativeTurnOutcome::Complete)
        && report_enabled
        && !has_report
    {
        let mut history = vec![Message::user(prompt)];
        if !text.trim().is_empty() {
            history.push(Message::assistant(text.clone()));
        }
        (outcome, text) = run_inner_subagent(
            client,
            model_id,
            parent_preamble,
            REPORT_FOLLOWUP_PROMPT.to_string(),
            history,
            catalog,
            budget,
            launch_cwd,
            fs,
            session_id,
            child.clone(),
            artifacts_dir.clone(),
            thoroughness,
            workspace_tree,
            allowed_tools,
            parent_runtime,
        )
        .await;
    }

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
            let has_report = path.is_file()
                && std::fs::metadata(&path)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false);
            if has_report {
                (true, format_explore_handoff(&path, "ok", &text))
            } else if report_enabled {
                (
                    false,
                    format_explore_handoff(&path, "missing_report", &text),
                )
            } else {
                (
                    true,
                    format!("Explore report ready.\nstatus: text_only\nsummary:\n{text}\n"),
                )
            }
        }
        NativeTurnOutcome::Failed(message) => {
            let path = explore_path(&artifacts_dir);
            if path.is_file() {
                (false, format_explore_handoff(&path, "failed", &message))
            } else {
                (
                    false,
                    format!("Explore report failed.\nstatus: failed\nsummary:\n{message}\n"),
                )
            }
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

fn excluded_from_default_explore(name: &str, runtime: &ParentRuntime) -> bool {
    if matches!(
        name,
        WriteFileTool::NAME | EditFileTool::NAME | BashTool::NAME
    ) {
        return true;
    }
    runtime.mcp.bindings().iter().any(|binding| {
        binding.local_name == name && !(binding.server_key == MCP_SERVER_NAME && binding.read_only)
    })
}

#[allow(clippy::too_many_arguments)]
fn inner_tool_schemas(
    read: &ReadFileTool,
    recall: &RecallTool,
    glob: &GlobTool,
    grep: &GrepTool,
    skill: &SkillTool,
    write_explore: &WriteExploreReportTool,
    write: Option<&WriteFileTool>,
    edit: Option<&EditFileTool>,
    bash: Option<&BashTool>,
    dynamic: &[DynamicTool],
    allowed_tools: Option<&HashSet<String>>,
) -> Vec<Value> {
    let mut schemas = vec![
        schema_for(read),
        schema_for(recall),
        schema_for(glob),
        schema_for(grep),
        schema_for(skill),
        schema_for(write_explore),
    ];
    if let Some(tool) =
        write.filter(|_| allowed_tools.is_none_or(|set| set.contains(WriteFileTool::NAME)))
    {
        schemas.push(schema_for(tool));
    }
    if let Some(tool) =
        edit.filter(|_| allowed_tools.is_none_or(|set| set.contains(EditFileTool::NAME)))
    {
        schemas.push(schema_for(tool));
    }
    if let Some(tool) =
        bash.filter(|_| allowed_tools.is_none_or(|set| set.contains(BashTool::NAME)))
    {
        schemas.push(schema_for(tool));
    }
    schemas.extend(
        dynamic
            .iter()
            .filter(|tool| allowed_tools.is_none_or(|set| set.contains(tool.name())))
            .filter_map(|tool| serde_json::to_value(tool.definition()).ok()),
    );
    schemas
        .into_iter()
        .filter(|schema| {
            allowed_tools.is_none_or(|set| {
                schema
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| set.contains(name))
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
async fn run_inner_subagent(
    client: CodegLlmClient,
    model_id: String,
    parent_preamble: String,
    prompt: String,
    history: Vec<Message>,
    catalog: SkillCatalog,
    budget: BudgetConfig,
    launch_cwd: std::path::PathBuf,
    fs: Arc<crate::acp::file_system_runtime::FileSystemRuntime>,
    session_id: String,
    cancel: CancellationToken,
    artifacts_dir: PathBuf,
    thoroughness: String,
    workspace_tree: Option<String>,
    allowed_tools: Option<HashSet<String>>,
    parent_runtime: Option<ParentRuntime>,
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
        loaded_skills: crate::agent::tools::LoadedSkills::shared_with(["using-plan-explore"]),
    };
    let read = ReadFileTool::new(inner_ctx.clone());
    let recall = RecallTool::new(inner_ctx.clone());
    let glob = GlobTool::new(inner_ctx.clone());
    let grep = GrepTool::new(inner_ctx.clone());
    let skill = SkillTool::new(inner_ctx.clone(), catalog.clone());
    let write_explore = WriteExploreReportTool::new(inner_ctx.clone(), artifacts_dir);
    let write = parent_runtime
        .as_ref()
        .map(|_| WriteFileTool::new(inner_ctx.clone()));
    let edit = parent_runtime
        .as_ref()
        .map(|_| EditFileTool::new(inner_ctx.clone()));
    let bash = parent_runtime
        .as_ref()
        .map(|runtime| BashTool::new(inner_ctx.clone(), Arc::clone(&runtime.terminals)));
    let dynamic = parent_runtime
        .as_ref()
        .map(|runtime| runtime.mcp.dynamic_tools(inner_ctx.clone()))
        .unwrap_or_default();
    let skill_body = catalog.skill_body("explore");
    let preamble = subagent_preamble(
        &parent_preamble,
        &thoroughness,
        skill_body.as_deref(),
        workspace_tree.as_deref(),
    );
    let tool_schemas = inner_tool_schemas(
        &read,
        &recall,
        &glob,
        &grep,
        &skill,
        &write_explore,
        write.as_ref(),
        edit.as_ref(),
        bash.as_ref(),
        &dynamic,
        allowed_tools.as_ref(),
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
        mcp_readonly: Arc::new(
            parent_runtime
                .as_ref()
                .map(|runtime| {
                    runtime
                        .mcp
                        .readonly_local_names()
                        .into_iter()
                        .filter(|name| allowed_tools.as_ref().is_none_or(|set| set.contains(name)))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        session_memory: None,
    };
    let trace = HookTrace::new();
    let hook = match parent_runtime.as_ref() {
        Some(_runtime) if _runtime.auto_allow => {
            CodegHook::auto_allow_with_tools(trace.clone(), inner_ctx.clone())
        }
        Some(runtime) => {
            let permissions = runtime.permissions.clone().unwrap_or_else(|| {
                let (tx, _rx) = mpsc::channel(1);
                tx
            });
            CodegHook::waiting(trace.clone(), permissions, cancel.clone())
        }
        None => CodegHook::auto_allow_with_tools(trace.clone(), inner_ctx.clone()),
    }
    .with_native(native);

    let user_prompt = Message::user(prompt);
    let stream = tokio::select! {
        _ = cancel.cancelled() => return (NativeTurnOutcome::Cancelled, String::new()),
        stream = assemble_inner(
            client,
            model_id,
            preamble,
            user_prompt,
            history,
            read,
            recall,
            glob,
            grep,
            skill,
            write_explore,
            hook,
            allowed_tools,
            write,
            edit,
            bash,
            dynamic,
            cancel.clone(),
        ) => stream,
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

/// Assemble the inner agent from the default Explore tools plus any explicitly
/// inherited parent runtime tools. No companion/plan/subagent controls.
#[allow(clippy::too_many_arguments)]
async fn assemble_inner(
    client: CodegLlmClient,
    model_id: String,
    preamble: String,
    prompt: Message,
    history: Vec<Message>,
    read: ReadFileTool,
    recall: RecallTool,
    glob: GlobTool,
    grep: GrepTool,
    skill: SkillTool,
    write_explore: WriteExploreReportTool,
    hook: CodegHook,
    allowed_tools: Option<HashSet<String>>,
    write: Option<WriteFileTool>,
    edit: Option<EditFileTool>,
    bash: Option<BashTool>,
    dynamic: Vec<DynamicTool>,
    cancel: CancellationToken,
) -> rig::agent::StreamingResult {
    let model = crate::agent::model::model_with_cancel(client, model_id, cancel);
    let mut builder = rig::agent::AgentBuilder::from_model_handle(model)
        .preamble(&preamble)
        .default_max_turns(SUBAGENT_MAX_TURNS)
        .dynamic_tools(Vec::new());
    let enabled = |name: &str| allowed_tools.as_ref().is_none_or(|set| set.contains(name));
    if enabled(ReadFileTool::NAME) {
        builder = builder.tool(read);
    }
    if enabled(RecallTool::NAME) {
        builder = builder.tool(recall);
    }
    if enabled(GlobTool::NAME) {
        builder = builder.tool(glob);
    }
    if enabled(GrepTool::NAME) {
        builder = builder.tool(grep);
    }
    if enabled(SkillTool::NAME) {
        builder = builder.tool(skill);
    }
    if enabled(WriteExploreReportTool::NAME) {
        builder = builder.tool(write_explore);
    }
    if let Some(write) = write.filter(|_| enabled(WriteFileTool::NAME)) {
        builder = builder.tool(write);
    }
    if let Some(edit) = edit.filter(|_| enabled(EditFileTool::NAME)) {
        builder = builder.tool(edit);
    }
    if let Some(bash) = bash.filter(|_| enabled(BashTool::NAME)) {
        builder = builder.tool(bash);
    }
    builder
        .dynamic_tools(
            dynamic
                .into_iter()
                .filter(|tool| enabled(tool.name()))
                .collect(),
        )
        .build()
        .runner(prompt)
        .history(history)
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    async fn spawn_followup_completions() -> (String, Arc<Mutex<Vec<Value>>>) {
        fn text_sse(text: &str) -> String {
            let first = json!({
                "id": "c",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {"role": "assistant", "content": text},
                    "finish_reason": null
                }]
            });
            let last = json!({
                "id": "c",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            });
            format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n")
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_count = Arc::new(AtomicUsize::new(0));
        let app = Router::new().fallback(post({
            let requests = Arc::clone(&requests);
            let request_count = Arc::clone(&request_count);
            move |Json(body): Json<Value>| {
                let requests = Arc::clone(&requests);
                let request_count = Arc::clone(&request_count);
                async move {
                    requests.lock().expect("requests").push(body);
                    let response = match request_count.fetch_add(1, Ordering::SeqCst) {
                        0 => text_sse("initial-summary"),
                        1 => {
                            let call = json!({
                                "id": "write-followup",
                                "type": "function",
                                "function": {
                                    "name": "write_explore_report",
                                    "arguments": json!({"content": "# Follow-up report\\n"}).to_string()
                                }
                            });
                            format!(
                                "data: {}\n\ndata: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n",
                                json!({
                                    "id": "c",
                                    "object": "chat.completion.chunk",
                                    "choices": [{
                                        "index": 0,
                                        "delta": {"role": "assistant", "content": null, "tool_calls": [call]},
                                        "finish_reason": null
                                    }]
                                })
                            )
                        }
                        _ => text_sse("report-written"),
                    };
                    (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        response,
                    )
                        .into_response()
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind followup");
        let addr = listener.local_addr().expect("followup addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/v1"), requests)
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
    fn parent_preamble_preserves_project_rules() {
        let parent = "x".repeat(4096);
        let text = subagent_preamble(&parent, "medium", None, None);
        assert!(text.starts_with(EXPLORE_PREAMBLE_INTRO));
        assert!(text.contains("Thoroughness: medium"));
        assert!(text.ends_with(&parent));
    }

    #[test]
    fn explicit_empty_allowlist_disables_every_inner_tool() {
        let available = available_inner_tool_names(None);
        let selected = validate_inner_tool_allowlist(Some(&[]), None, &available, None)
            .expect("empty allowlist is valid")
            .expect("explicit list is retained");
        assert!(selected.is_empty());
        assert!(configured_inner_tool_names(Some(&selected), &available).is_empty());
    }

    #[test]
    fn allowlist_rejects_tools_that_are_not_registered() {
        let available = available_inner_tool_names(None);
        let err = validate_inner_tool_allowlist(Some(&["bash".into()]), None, &available, None)
            .expect_err("bash is not registered by the default Explore child");
        assert!(err.contains("unknown subagent tool"), "{err}");
    }

    #[test]
    fn allowlist_cannot_escape_parent_configured_tools() {
        let available = available_inner_tool_names(None);
        let configured = HashSet::from(["read_file".to_string()]);
        let err = validate_inner_tool_allowlist(
            Some(&["grep".into()]),
            Some(&configured),
            &available,
            None,
        )
        .expect_err("child cannot add a tool outside parent allowlist");
        assert!(err.contains("not allowed by the parent"), "{err}");
    }

    #[test]
    fn subagent_preamble_appends_workspace_tree() {
        let text = subagent_preamble(
            "parent",
            "quick",
            None,
            Some("Workspace tree (max 3 levels injected; deeper paths omitted):\n```\n.\n└── src/\n```"),
        );
        assert!(text.contains("max 3 levels injected"), "{text}");
        assert!(text.contains("src/"), "{text}");
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
                    model_id: None,
                    allowed_tools: None,
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
                model_id: None,
                allowed_tools: None,
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
                    model_id: None,
                    allowed_tools: None,
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
                    model_id: None,
                    allowed_tools: None,
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
        let (base, requests) = spawn_followup_completions().await;
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
                    model_id: None,
                    allowed_tools: None,
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
                assert!(output.contains("status: ok"), "{output}");
                assert!(output.contains("path:"), "{output}");
                assert!(output.contains("summary:"), "{output}");
                assert!(
                    output.contains("report-written") || output.contains("(no summary)"),
                    "{output}"
                );
                assert!(!output.contains("# huge"), "{output}");
                assert!(id.starts_with("sa-"), "{id}");

                let body_dump = requests.lock().expect("requests");
                assert!(
                    body_dump.len() >= 3,
                    "expected follow-up tool turn: {body_dump:?}"
                );
                assert!(
                    body_dump[1]
                        .to_string()
                        .contains("Please use write_explore_report"),
                    "follow-up user message missing: {:?}",
                    body_dump[1]
                );
                let path = output
                    .lines()
                    .find_map(|line| line.strip_prefix("path: "))
                    .expect("report path")
                    .trim();
                assert!(Path::new(path).is_file(), "report missing at {path}");
            }
        }
        assert!(!table.lock().expect("table").is_inflight());
    }

    fn inner_tool_names() -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "inner", "c1");
        let read = ReadFileTool::new(ctx.clone());
        let recall = RecallTool::new(ctx.clone());
        let glob = GlobTool::new(ctx.clone());
        let grep = GrepTool::new(ctx.clone());
        let skill = SkillTool::new(ctx.clone(), SkillCatalog::default());
        let write_explore = WriteExploreReportTool::new(ctx.clone(), dir.path().to_path_buf());
        inner_tool_schemas(
            &read,
            &recall,
            &glob,
            &grep,
            &skill,
            &write_explore,
            None,
            None,
            None,
            &[],
            None,
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
    fn inner_schema_omits_native_lsp_and_codegraph() {
        let names = inner_tool_names();
        assert!(names.iter().any(|name| name == "read_file"), "{names:?}");
        assert!(names.iter().any(|name| name == "grep"), "{names:?}");
        for forbidden in [
            "codegraph",
            "lsp",
            "write_file",
            "edit_file",
            "bash",
            "subagent",
            "update_plan",
            "write_plan",
            "enter_plan_mode",
            "exit_plan_mode",
        ] {
            assert!(
                !names.iter().any(|name| name == forbidden),
                "{names:?} contains {forbidden}"
            );
        }
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
                model_id: None,
                allowed_tools: None,
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

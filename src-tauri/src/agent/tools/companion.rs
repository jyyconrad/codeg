//! Companion tools wrapping the same host access traits as `codeg-mcp`.
//!
//! Schema is the shared `tool_schema.json` (no copy). Tools are registered only
//! when the corresponding feature is on; authoring still re-checks at call time.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rig::tool::{DynamicTool, ToolContext, ToolExecutionError, ToolOutput};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use super::NativeToolCtx;
use crate::acp::chat_authoring::AuthoringContext;
use crate::acp::connection::{delegate_target_args, CompanionFeatureFlags, DelegationInjection};
use crate::acp::delegation::broker::StatusWait;
use crate::acp::delegation::companion::{
    append_custom_agents_to_delegate_enum, normalize_status_task_ids, parse_automation_spec,
    parse_max_messages, parse_session_id, parse_work_task_spec,
    remove_disabled_agents_from_delegate_enum, render_ask_result, render_authoring_result,
    render_feedback_result, render_session_result, render_status_result, render_task_ack,
    render_task_report, CompanionFeatures, TOOL_SCHEMA_JSON,
};
use crate::acp::delegation::listener::STATUS_WAIT_MAX_MS;
use crate::acp::delegation::types::{DelegationRequest, ResumeDelegationRequest};
use crate::acp::feedback::{bounded_feedback_batch, MAX_FEEDBACK_RESPONSE_BYTES};
use crate::acp::question::{parse_questions, QuestionOutcome};
use crate::acp::session_state::SessionState;
use crate::agent::context::ContextStore;
use crate::models::agent::AgentType;

const COMPANION_TOOL_NAMES: &[&str] = &[
    "delegate_to_agent",
    "get_delegation_status",
    "cancel_delegation",
    "resume_delegation",
    "check_user_feedback",
    "ask_user_question",
    "get_session_info",
    "create_automation",
    "create_work_task",
    "task_progress",
    "task_complete",
];

/// One tool loaded from the shared schema after feature filtering.
#[derive(Clone, Debug)]
pub(crate) struct CompanionToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Launch-time companion registration. Capability bits are derived from `defs`.
#[derive(Clone, Default)]
pub(crate) struct CompanionPlan {
    pub defs: Arc<Vec<CompanionToolDef>>,
}

impl CompanionPlan {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn feedback_registered(&self) -> bool {
        self.defs.iter().any(|d| d.name == "check_user_feedback")
    }

    pub fn delegation_registered(&self) -> bool {
        self.defs.iter().any(|d| d.name == "delegate_to_agent")
    }
}

pub(crate) fn is_companion_tool(name: &str) -> bool {
    COMPANION_TOOL_NAMES.contains(&name)
}

/// Load tool defs from the same `tool_schema.json` the MCP companion ships.
pub(crate) fn load_companion_defs(
    features: CompanionFeatures,
    custom_agents: &[String],
    disabled_agents: &[String],
) -> Result<Vec<CompanionToolDef>, String> {
    let mut all: Value = serde_json::from_str(TOOL_SCHEMA_JSON)
        .map_err(|err| format!("embedded schema invalid: {err}"))?;
    remove_disabled_agents_from_delegate_enum(&mut all, disabled_agents);
    append_custom_agents_to_delegate_enum(&mut all, custom_agents);
    let arr = all
        .as_array()
        .ok_or_else(|| "embedded schema is not an array".to_string())?;
    let mut defs = Vec::new();
    for tool in arr {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() || !features.allows_tool(&name) {
            continue;
        }
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let parameters = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        defs.push(CompanionToolDef {
            name,
            description,
            parameters,
        });
    }
    Ok(defs)
}

pub(crate) async fn companion_plan_from_injection(
    injection: Option<&DelegationInjection>,
    flags: CompanionFeatureFlags,
) -> CompanionPlan {
    let Some(injection) = injection else {
        return CompanionPlan::empty();
    };
    let (custom_agents, disabled_agents) = if flags.delegation {
        let disabled = injection
            .agent_availability
            .disabled_agent_wire_slugs()
            .await;
        delegate_target_args(&disabled)
    } else {
        (Vec::new(), Vec::new())
    };
    match load_companion_defs(flags.into(), &custom_agents, &disabled_agents) {
        Ok(defs) => CompanionPlan {
            defs: Arc::new(defs),
        },
        Err(err) => {
            tracing::warn!("[ACP] native companion schema unavailable: {err}");
            CompanionPlan {
                defs: Arc::new(Vec::new()),
            }
        }
    }
}

/// Pending `check_user_feedback` ids. Commit only after the next model request
/// whose projected history still contains the tool result (K20 / PR7a).
pub(crate) struct FeedbackDelivery {
    access: Arc<dyn crate::acp::feedback::SessionFeedbackAccess>,
    connection_id: String,
    pending: Mutex<Vec<PendingFeedbackCommit>>,
}

struct PendingFeedbackCommit {
    tool_call_id: String,
    ids: Vec<String>,
}

impl FeedbackDelivery {
    pub fn new(
        access: Arc<dyn crate::acp::feedback::SessionFeedbackAccess>,
        connection_id: impl Into<String>,
    ) -> Self {
        Self {
            access,
            connection_id: connection_id.into(),
            pending: Mutex::new(Vec::new()),
        }
    }

    pub fn enqueue(&self, tool_call_id: String, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        self.pending
            .lock()
            .expect("feedback pending")
            .push(PendingFeedbackCommit { tool_call_id, ids });
    }

    #[cfg(test)]
    pub fn pending_ids(&self) -> Vec<String> {
        self.pending
            .lock()
            .expect("feedback pending")
            .iter()
            .flat_map(|item| item.ids.iter().cloned())
            .collect()
    }

    pub async fn commit_present(&self, present_call_ids: &HashSet<String>) {
        let to_commit: Vec<String> = {
            let mut pending = self.pending.lock().expect("feedback pending");
            let mut ids = Vec::new();
            pending.retain(|item| {
                if present_call_ids.contains(&item.tool_call_id) {
                    ids.extend(item.ids.iter().cloned());
                    false
                } else {
                    true
                }
            });
            ids
        };
        if to_commit.is_empty() {
            return;
        }
        self.access
            .commit_feedback_delivered(&self.connection_id, to_commit)
            .await;
    }
}

pub(crate) fn projected_tool_call_ids(store: &ContextStore) -> HashSet<String> {
    store
        .facts()
        .map(|fact| fact.tool_call_id.clone())
        .collect()
}

/// Host services captured for companion DynamicTools. No stubs.
#[derive(Clone)]
pub(crate) struct CompanionRuntime {
    pub tool_ctx: NativeToolCtx,
    pub connection_id: String,
    pub launch_cwd: PathBuf,
    pub injection: DelegationInjection,
    pub session_state: Arc<RwLock<SessionState>>,
    pub feedback: Option<Arc<FeedbackDelivery>>,
}

pub(crate) fn build_companion_tools(
    runtime: CompanionRuntime,
    defs: &[CompanionToolDef],
) -> Vec<DynamicTool> {
    defs.iter()
        .map(|def| {
            let runtime = runtime.clone();
            let name = def.name.clone();
            DynamicTool::new(
                def.name.clone(),
                def.description.clone(),
                def.parameters.clone(),
                move |_context: &mut ToolContext, arguments: Value| {
                    let runtime = runtime.clone();
                    let name = name.clone();
                    Box::pin(async move {
                        let text = call_companion_tool(&runtime, &name, arguments).await?;
                        Ok(ToolOutput::text(text))
                    })
                },
            )
        })
        .collect()
}

pub(crate) fn schema_for_companion_def(def: &CompanionToolDef) -> Value {
    json!({
        "name": def.name,
        "description": def.description,
        "parameters": def.parameters,
    })
}

pub(crate) async fn call_companion_tool(
    runtime: &CompanionRuntime,
    name: &str,
    args: Value,
) -> Result<String, ToolExecutionError> {
    match name {
        "delegate_to_agent" => call_delegate(runtime, args).await,
        "get_delegation_status" => call_status(runtime, args).await,
        "cancel_delegation" => call_cancel_task(runtime, args).await,
        "resume_delegation" => call_resume(runtime, args).await,
        "check_user_feedback" => call_feedback(runtime, args).await,
        "ask_user_question" => call_ask(runtime, args).await,
        "get_session_info" => call_session_info(runtime, args).await,
        "create_automation" => call_create_automation(runtime, args).await,
        "create_work_task" => call_create_work_task(runtime, args).await,
        "task_progress" => call_task_progress(runtime, args).await,
        "task_complete" => call_task_complete(runtime, args).await,
        other => Err(args_err(format!("unknown tool: {other}"))),
    }
}

fn args_err(message: impl Into<String>) -> ToolExecutionError {
    let message = message.into();
    ToolExecutionError::invalid_args(message.clone()).with_model_feedback(message)
}

fn cancelled_err() -> ToolExecutionError {
    ToolExecutionError::cancelled("tool was cancelled").with_model_feedback("tool was cancelled")
}

fn rendered_text(rendered: &Value) -> String {
    rendered
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn parse_agent_type(raw: &str) -> Option<AgentType> {
    serde_json::from_value(Value::String(raw.to_string())).ok()
}

async fn parent_conversation_id(runtime: &CompanionRuntime) -> Option<i32> {
    runtime.session_state.read().await.conversation_id
}

async fn call_delegate(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let fact = runtime
        .tool_ctx
        .begin("delegate_to_agent", args.clone())
        .await?;
    let agent_type = match args.get("agent_type").and_then(Value::as_str) {
        Some(raw) => match parse_agent_type(raw) {
            Some(agent) => agent,
            None => {
                let report = json!({
                    "status": "failed",
                    "error_code": "invalid_agent_type",
                    "message": format!("invalid agent_type: {raw}"),
                });
                return runtime
                    .tool_ctx
                    .finish_ok(fact, rendered_text(&render_task_report(&report)))
                    .await;
            }
        },
        None => {
            return Err(runtime
                .tool_ctx
                .finish_err(fact, args_err("delegate_to_agent requires agent_type"))
                .await);
        }
    };
    let task = match args.get("task").and_then(Value::as_str).map(str::trim) {
        Some(task) if !task.is_empty() => task.to_string(),
        _ => {
            return Err(runtime
                .tool_ctx
                .finish_err(
                    fact,
                    args_err("delegate_to_agent requires a non-empty task"),
                )
                .await);
        }
    };
    let requested_working_dir = args
        .get("working_dir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let working_dir = requested_working_dir
        .clone()
        .or_else(|| Some(runtime.launch_cwd.to_string_lossy().into_owned()));
    let Some(parent_conversation_id) = parent_conversation_id(runtime).await else {
        let report = json!({
            "status": "canceled",
            "message": "parent has no active conversation",
        });
        return runtime
            .tool_ctx
            .finish_ok(fact, rendered_text(&render_task_report(&report)))
            .await;
    };
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let handle = uuid::Uuid::new_v4().to_string();
    let req = DelegationRequest {
        parent_connection_id: runtime.connection_id.clone(),
        parent_conversation_id,
        parent_tool_use_id: fact.tool_call_id.clone(),
        agent_type,
        task,
        working_dir,
        requested_working_dir,
        external_handle: Some(handle.clone()),
    };
    let broker = Arc::clone(&runtime.injection.broker);
    let mut start = tokio::spawn({
        let broker = Arc::clone(&broker);
        async move { broker.start_delegation(req).await }
    });
    let report = tokio::select! {
        result = &mut start => result.unwrap_or_else(|_| json_canceled("delegation worker dropped")),
        _ = runtime.tool_ctx.cancel.cancelled() => {
            broker
                .cancel_by_external_handle(&handle, "cancelled".into())
                .await;
            let _ = start.await;
            return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
        }
    };
    let value = serde_json::to_value(&report).unwrap_or(json!({ "status": "failed" }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_task_report(&value)))
        .await
}

fn json_canceled(message: &str) -> crate::acp::delegation::types::DelegationTaskReport {
    crate::acp::delegation::types::DelegationTaskReport {
        task_id: None,
        status: crate::acp::delegation::types::TaskStatus::Canceled,
        child_conversation_id: None,
        agent_type: None,
        text: None,
        error_code: Some("canceled".into()),
        message: Some(message.into()),
        duration_ms: None,
        blocked_on: None,
    }
}

async fn call_status(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let fact = runtime
        .tool_ctx
        .begin("get_delegation_status", args.clone())
        .await?;
    let task_ids = match normalize_status_task_ids(&args) {
        Ok(ids) if !ids.is_empty() => ids,
        Ok(_) => {
            return Err(runtime
                .tool_ctx
                .finish_err(
                    fact,
                    args_err("get_delegation_status requires a non-empty task_ids array"),
                )
                .await);
        }
        Err(msg) => {
            return Err(runtime.tool_ctx.finish_err(fact, args_err(msg)).await);
        }
    };
    let wait = match args.get("wait_ms").and_then(Value::as_u64) {
        None => StatusWait::Immediate,
        Some(0) => StatusWait::Infinite,
        Some(ms) => StatusWait::Bounded(ms.min(STATUS_WAIT_MAX_MS)),
    };
    let parent_conversation_id = parent_conversation_id(runtime).await;
    let broker = Arc::clone(&runtime.injection.broker);
    let connection_id = runtime.connection_id.clone();
    let mut status = tokio::spawn(async move {
        broker
            .get_tasks_status(&connection_id, parent_conversation_id, &task_ids, wait)
            .await
    });
    let reports = tokio::select! {
        result = &mut status => result.unwrap_or_default(),
        _ = runtime.tool_ctx.cancel.cancelled() => {
            status.abort();
            return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
        }
    };
    let envelope = json!({ "tasks": reports });
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_status_result(&envelope)))
        .await
}

async fn call_cancel_task(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let fact = runtime
        .tool_ctx
        .begin("cancel_delegation", args.clone())
        .await?;
    let task_id = match args.get("task_id").and_then(Value::as_str).map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            return Err(runtime
                .tool_ctx
                .finish_err(
                    fact,
                    args_err("cancel_delegation requires a non-empty string task_id"),
                )
                .await);
        }
    };
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let parent_conversation_id = parent_conversation_id(runtime).await;
    let report = runtime
        .injection
        .broker
        .cancel_task_by_id(&runtime.connection_id, parent_conversation_id, &task_id)
        .await;
    let value = serde_json::to_value(&report).unwrap_or(json!({ "status": "unknown" }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_task_report(&value)))
        .await
}

async fn call_resume(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let fact = runtime
        .tool_ctx
        .begin("resume_delegation", args.clone())
        .await?;
    let task_id = match args.get("task_id").and_then(Value::as_str).map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            return Err(runtime
                .tool_ctx
                .finish_err(
                    fact,
                    args_err("resume_delegation requires a non-empty string task_id"),
                )
                .await);
        }
    };
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let Some(parent_conversation_id) = parent_conversation_id(runtime).await else {
        let report = json!({
            "status": "canceled",
            "message": "parent has no active conversation",
        });
        return runtime
            .tool_ctx
            .finish_ok(fact, rendered_text(&render_task_report(&report)))
            .await;
    };
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let handle = uuid::Uuid::new_v4().to_string();
    let req = ResumeDelegationRequest {
        parent_connection_id: runtime.connection_id.clone(),
        parent_conversation_id,
        task_id,
        reason,
        external_handle: Some(handle.clone()),
    };
    let broker = Arc::clone(&runtime.injection.broker);
    let mut start = tokio::spawn({
        let broker = Arc::clone(&broker);
        async move { broker.resume_delegation(req).await }
    });
    let report = tokio::select! {
        result = &mut start => result.unwrap_or_else(|_| json_canceled("resume worker dropped")),
        _ = runtime.tool_ctx.cancel.cancelled() => {
            broker
                .cancel_by_external_handle(&handle, "cancelled".into())
                .await;
            let _ = start.await;
            return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
        }
    };
    let value = serde_json::to_value(&report).unwrap_or(json!({ "status": "failed" }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_task_report(&value)))
        .await
}

async fn call_feedback(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let fact = runtime
        .tool_ctx
        .begin("check_user_feedback", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let pending = runtime
        .injection
        .feedback_access
        .read_pending_feedback(&runtime.connection_id)
        .await;
    let batch = bounded_feedback_batch(pending, MAX_FEEDBACK_RESPONSE_BYTES);
    let ids: Vec<String> = batch.iter().map(|item| item.id.clone()).collect();
    let notes: Vec<Value> = batch
        .iter()
        .map(|item| json!({ "text": item.text, "created_at": item.created_at }))
        .collect();
    let outcome = json!({
        "count": notes.len(),
        "feedback": notes,
        "_commit_ids": ids,
    });
    let text = rendered_text(&render_feedback_result(&outcome));
    let call_id = fact.tool_call_id.clone();
    let out = runtime.tool_ctx.finish_ok(fact, text).await?;
    if let Some(delivery) = &runtime.feedback {
        delivery.enqueue(call_id, ids);
    }
    Ok(out)
}

async fn call_ask(runtime: &CompanionRuntime, args: Value) -> Result<String, ToolExecutionError> {
    let questions = match parse_questions(&args) {
        Ok(questions) => questions,
        Err(msg) => return Err(args_err(msg)),
    };
    let fact = runtime
        .tool_ctx
        .begin("ask_user_question", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let Some(reg) = runtime
        .injection
        .questions
        .register_question(&runtime.connection_id, questions)
        .await
    else {
        let declined = QuestionOutcome {
            answers: Vec::new(),
            declined: true,
        };
        let value = serde_json::to_value(&declined).unwrap_or(json!({ "declined": true }));
        return runtime
            .tool_ctx
            .finish_ok(fact, rendered_text(&render_ask_result(&value)))
            .await;
    };
    let question_id = reg.question_id;
    let mut answer_rx = reg.answer_rx;
    let outcome = tokio::select! {
        biased;
        ans = &mut answer_rx => ans.unwrap_or(QuestionOutcome { answers: Vec::new(), declined: true }),
        _ = runtime.tool_ctx.cancel.cancelled() => {
            runtime
                .injection
                .questions
                .cancel_question(&runtime.connection_id, &question_id)
                .await;
            return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
        }
    };
    let value = serde_json::to_value(&outcome).unwrap_or(json!({ "declined": true }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_ask_result(&value)))
        .await
}

async fn call_session_info(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let Some(session_id) = parse_session_id(&args) else {
        return Err(args_err(
            "get_session_info requires an integer `session_id` \
             (the number in the codeg://session/<id> reference)",
        ));
    };
    let fact = runtime
        .tool_ctx
        .begin("get_session_info", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let max_messages = parse_max_messages(&args);
    let info = runtime
        .injection
        .session_info_access
        .resolve(session_id, max_messages)
        .await;
    let value =
        serde_json::to_value(&info).unwrap_or(json!({ "found": false, "session_id": session_id }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_session_result(&value)))
        .await
}

async fn authoring_context(runtime: &CompanionRuntime) -> AuthoringContext {
    AuthoringContext {
        conversation_id: parent_conversation_id(runtime).await,
        working_dir: runtime.launch_cwd.clone(),
    }
}

async fn call_create_automation(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let spec = match parse_automation_spec(&args) {
        Ok(spec) => spec,
        Err(msg) => return Err(args_err(msg)),
    };
    let fact = runtime
        .tool_ctx
        .begin("create_automation", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let ctx = authoring_context(runtime).await;
    let outcome = runtime
        .injection
        .authoring_access
        .create_automation(ctx, spec)
        .await;
    let value = serde_json::to_value(&outcome).unwrap_or(json!({ "created": false }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_authoring_result(&value)))
        .await
}

async fn call_create_work_task(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let spec = match parse_work_task_spec(&args) {
        Ok(spec) => spec,
        Err(msg) => return Err(args_err(msg)),
    };
    let fact = runtime
        .tool_ctx
        .begin("create_work_task", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let ctx = authoring_context(runtime).await;
    let outcome = runtime
        .injection
        .authoring_access
        .create_work_task(ctx, spec)
        .await;
    let value = serde_json::to_value(&outcome).unwrap_or(json!({ "created": false }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_authoring_result(&value)))
        .await
}

async fn call_task_progress(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let Some(message) = message else {
        return Err(args_err(
            "task_progress requires a non-empty `message` string",
        ));
    };
    let fact = runtime
        .tool_ctx
        .begin("task_progress", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let ack = runtime
        .injection
        .tasks
        .report_progress(&runtime.connection_id, &message)
        .await;
    let value = serde_json::to_value(&ack).unwrap_or(json!({ "recorded": false }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_task_ack(&value)))
        .await
}

async fn call_task_complete(
    runtime: &CompanionRuntime,
    args: Value,
) -> Result<String, ToolExecutionError> {
    let verdict = args
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !matches!(verdict, "success" | "needs_review" | "blocked") {
        return Err(args_err(
            "task_complete requires `verdict` of success | needs_review | blocked",
        ));
    };
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let fact = runtime
        .tool_ctx
        .begin("task_complete", args.clone())
        .await?;
    if runtime.tool_ctx.cancel.is_cancelled() {
        return Err(runtime.tool_ctx.finish_err(fact, cancelled_err()).await);
    }
    let ack = runtime
        .injection
        .tasks
        .complete(&runtime.connection_id, verdict, summary.as_deref())
        .await;
    let value = serde_json::to_value(&ack).unwrap_or(json!({ "recorded": false }));
    runtime
        .tool_ctx
        .finish_ok(fact, rendered_text(&render_task_ack(&value)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::chat_authoring::{
        AuthoringOutcome, ChatAuthoringAccess, ChatAuthoringConfig, NewAutomationSpec,
        NewWorkTaskSpec,
    };
    use crate::acp::connection::AgentAvailabilityLookup;
    use crate::acp::delegation::broker::{
        ConversationDepthLookup, DelegationBroker, DelegationConfig,
    };
    use crate::acp::delegation::listener::TokenRegistry;
    use crate::acp::delegation::spawner::{mock::MockSpawner, ConnectionSpawner};
    use crate::acp::delegation::types::DelegationError;
    use crate::acp::feedback::{FeedbackConfig, PendingFeedback, SessionFeedbackAccess};
    use crate::acp::plan_approval::SessionPlanApprovalAccess;
    use crate::acp::question::{
        QuestionConfig, QuestionSpec, RegisteredQuestion, SessionQuestionAccess,
    };
    use crate::acp::session_info::{SessionInfo, SessionInfoAccess, SessionInfoConfig};
    use crate::acp::work_task_tools::{TaskReportAck, WorkTaskToolAccess};
    use crate::agent::context::{CallIdentity, ContextStore, ExecutionFact};
    use crate::agent::tools::test_tool_ctx;
    use crate::models::agent::AgentType;
    use chrono::Utc;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    struct NoDepth;
    #[async_trait::async_trait]
    impl ConversationDepthLookup for NoDepth {
        async fn parent_of(&self, _id: i32) -> Result<Option<i32>, DelegationError> {
            Ok(None)
        }
    }

    struct AllAgents;
    #[async_trait::async_trait]
    impl AgentAvailabilityLookup for AllAgents {
        async fn disabled_agent_wire_slugs(&self) -> Vec<String> {
            Vec::new()
        }
    }

    struct MemQuestions {
        registered: StdMutex<usize>,
        cancelled: StdMutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl SessionQuestionAccess for MemQuestions {
        async fn register_question(
            &self,
            _parent_connection_id: &str,
            questions: Vec<QuestionSpec>,
        ) -> Option<RegisteredQuestion> {
            *self.registered.lock().expect("q") += 1;
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(QuestionOutcome {
                declined: false,
                answers: questions
                    .into_iter()
                    .map(|q| crate::acp::question::QuestionAnsweredItem {
                        question: q.question,
                        header: q.header,
                        multi_select: q.multi_select,
                        selected: vec!["A".into()],
                    })
                    .collect(),
            });
            Some(RegisteredQuestion {
                question_id: "q1".into(),
                answer_rx: rx,
            })
        }
        async fn cancel_question(&self, _parent: &str, question_id: &str) {
            self.cancelled
                .lock()
                .expect("c")
                .push(question_id.to_string());
        }
        async fn cancel_questions_by_parent(&self, _parent: &str) {}
    }

    struct NoPlan;
    #[async_trait::async_trait]
    impl SessionPlanApprovalAccess for NoPlan {
        async fn register_plan_approval(
            &self,
            _parent: &str,
            _tool_call_id: String,
            _plan_markdown: String,
        ) -> Option<crate::acp::plan_approval::RegisteredPlanApproval> {
            None
        }
        async fn cancel_plan_approvals_by_parent(&self, _parent: &str) {}
    }

    struct MemTasks {
        progress: StdMutex<Vec<String>>,
        complete: StdMutex<Vec<(String, Option<String>)>>,
    }
    #[async_trait::async_trait]
    impl WorkTaskToolAccess for MemTasks {
        async fn report_progress(&self, _parent: &str, message: &str) -> TaskReportAck {
            self.progress.lock().expect("p").push(message.to_string());
            TaskReportAck::recorded()
        }
        async fn complete(
            &self,
            _parent: &str,
            verdict: &str,
            summary: Option<&str>,
        ) -> TaskReportAck {
            self.complete
                .lock()
                .expect("c")
                .push((verdict.to_string(), summary.map(str::to_string)));
            TaskReportAck::recorded()
        }
    }

    struct MemFeedback {
        pending: StdMutex<Vec<PendingFeedback>>,
        committed: StdMutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl SessionFeedbackAccess for MemFeedback {
        async fn read_pending_feedback(&self, _parent: &str) -> Vec<PendingFeedback> {
            self.pending.lock().expect("p").clone()
        }
        async fn commit_feedback_delivered(&self, _parent: &str, ids: Vec<String>) {
            self.committed.lock().expect("c").extend(ids);
        }
    }

    struct MemSessions;
    #[async_trait::async_trait]
    impl SessionInfoAccess for MemSessions {
        async fn resolve(&self, session_id: i32, _max_messages: u32) -> SessionInfo {
            if session_id == 7 {
                SessionInfo {
                    found: true,
                    session_id,
                    title: Some("Fix auth".into()),
                    agent_type: Some("claude_code".into()),
                    ..Default::default()
                }
            } else {
                SessionInfo::not_found(session_id)
            }
        }
    }

    struct MemAuthoring {
        automations: StdMutex<usize>,
        tasks: StdMutex<usize>,
        reject: bool,
    }
    #[async_trait::async_trait]
    impl ChatAuthoringAccess for MemAuthoring {
        async fn create_automation(
            &self,
            _ctx: AuthoringContext,
            spec: NewAutomationSpec,
        ) -> AuthoringOutcome {
            if self.reject {
                return AuthoringOutcome::rejected("automation", "turned off");
            }
            *self.automations.lock().expect("a") += 1;
            AuthoringOutcome {
                created: true,
                kind: "automation".into(),
                id: Some(1),
                title: Some(spec.name),
                ..Default::default()
            }
        }
        async fn create_work_task(
            &self,
            _ctx: AuthoringContext,
            spec: NewWorkTaskSpec,
        ) -> AuthoringOutcome {
            if self.reject {
                return AuthoringOutcome::rejected("work_task", "turned off");
            }
            *self.tasks.lock().expect("t") += 1;
            AuthoringOutcome {
                created: true,
                kind: "work_task".into(),
                id: Some(2),
                title: Some(spec.title),
                ..Default::default()
            }
        }
    }

    async fn test_injection(
        feedback: Arc<MemFeedback>,
        questions: Arc<MemQuestions>,
        tasks: Arc<MemTasks>,
        authoring: Arc<MemAuthoring>,
        broker_enabled: bool,
    ) -> DelegationInjection {
        let broker = Arc::new(DelegationBroker::new(
            Arc::new(MockSpawner::default()) as Arc<dyn ConnectionSpawner>,
            Arc::new(NoDepth) as Arc<dyn ConversationDepthLookup>,
        ));
        if broker_enabled {
            broker
                .set_config(DelegationConfig {
                    enabled: true,
                    ..DelegationConfig::default()
                })
                .await;
        }
        let injection = DelegationInjection {
            broker,
            tokens: Arc::new(TokenRegistry::default()),
            socket_path: PathBuf::from("/tmp/codeg-mcp.sock"),
            agent_availability: Arc::new(AllAgents) as Arc<dyn AgentAvailabilityLookup>,
            feedback: crate::acp::feedback::FeedbackRuntimeConfig::new(),
            ask: crate::acp::question::QuestionRuntimeConfig::new(),
            sessions: crate::acp::session_info::SessionInfoRuntimeConfig::new(),
            authoring: crate::acp::chat_authoring::ChatAuthoringRuntimeConfig::new(),
            questions: questions as Arc<dyn SessionQuestionAccess>,
            plan_approvals: Arc::new(NoPlan) as Arc<dyn SessionPlanApprovalAccess>,
            tasks: tasks as Arc<dyn WorkTaskToolAccess>,
            feedback_access: feedback as Arc<dyn SessionFeedbackAccess>,
            session_info_access: Arc::new(MemSessions) as Arc<dyn SessionInfoAccess>,
            authoring_access: authoring as Arc<dyn ChatAuthoringAccess>,
        };
        injection
            .feedback
            .set(FeedbackConfig { enabled: true })
            .await;
        injection.ask.set(QuestionConfig { enabled: true }).await;
        injection
            .sessions
            .set(SessionInfoConfig { enabled: true })
            .await;
        injection
            .authoring
            .set(ChatAuthoringConfig {
                automations_enabled: true,
                work_tasks_enabled: true,
            })
            .await;
        injection
    }

    fn runtime_for(
        name: &str,
        injection: DelegationInjection,
        feedback: Option<Arc<FeedbackDelivery>>,
    ) -> CompanionRuntime {
        let dir = std::env::temp_dir();
        let tool_ctx = test_tool_ctx(&dir, name, "call_1");
        let mut state = SessionState::new(
            "conn-1".into(),
            AgentType::CodegAgent,
            Some(dir.clone()),
            "main".into(),
            None,
        );
        state.conversation_id = Some(42);
        CompanionRuntime {
            tool_ctx,
            connection_id: "conn-1".into(),
            launch_cwd: dir,
            injection,
            session_state: Arc::new(RwLock::new(state)),
            feedback,
        }
    }

    #[test]
    fn schema_is_the_shared_tool_schema_json() {
        let all: Value = serde_json::from_str(TOOL_SCHEMA_JSON).unwrap();
        let features = CompanionFeatures {
            delegation: true,
            feedback: true,
            ask: true,
            sessions: true,
            tasks: true,
            automations: true,
            taskboard: true,
        };
        let defs = load_companion_defs(features, &[], &[]).unwrap();
        let json_names: Vec<&str> = all
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        let def_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(json_names, def_names);
        let delegate = defs.iter().find(|d| d.name == "delegate_to_agent").unwrap();
        let enum_vals = delegate
            .parameters
            .pointer("/properties/agent_type/enum")
            .and_then(Value::as_array)
            .unwrap();
        assert!(enum_vals.iter().any(|v| v.as_str() == Some("codeg_agent")));
    }

    #[test]
    fn registered_tools_match_feature_flags() {
        let none = CompanionFeatures {
            delegation: false,
            feedback: false,
            ask: false,
            sessions: false,
            tasks: false,
            automations: false,
            taskboard: false,
        };
        assert!(load_companion_defs(none, &[], &[]).unwrap().is_empty());

        let feedback_only = CompanionFeatures {
            feedback: true,
            ..none
        };
        let names: Vec<_> = load_companion_defs(feedback_only, &[], &[])
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["check_user_feedback".to_string()]);

        let ask_only = CompanionFeatures { ask: true, ..none };
        let names: Vec<_> = load_companion_defs(ask_only, &[], &[])
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["ask_user_question".to_string()]);

        let authoring = CompanionFeatures {
            automations: true,
            taskboard: false,
            ..none
        };
        let names: Vec<_> = load_companion_defs(authoring, &[], &[])
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["create_automation".to_string()]);

        let delegation_off = CompanionFeatures {
            delegation: false,
            feedback: true,
            ask: true,
            sessions: true,
            tasks: true,
            automations: true,
            taskboard: true,
        };
        let names: Vec<_> = load_companion_defs(delegation_off, &[], &[])
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert!(!names.iter().any(|n| n.contains("delegat")));
        assert!(names.contains(&"check_user_feedback".to_string()));
        assert!(names.contains(&"task_progress".to_string()));
    }

    #[test]
    fn capability_bits_follow_actual_defs() {
        let flags = CompanionFeatureFlags {
            delegation: true,
            feedback: true,
            ..CompanionFeatureFlags::default()
        };
        let defs = load_companion_defs(flags.into(), &[], &[]).unwrap();
        let plan = CompanionPlan {
            defs: Arc::new(defs),
        };
        assert!(plan.feedback_registered());
        assert!(plan.delegation_registered());

        let empty = CompanionPlan {
            defs: Arc::new(Vec::new()),
        };
        assert!(!empty.feedback_registered());
        assert!(!empty.delegation_registered());
    }

    #[tokio::test]
    async fn feedback_read_does_not_commit_until_projected() {
        let notes = vec![PendingFeedback {
            id: "n1".into(),
            text: "use UserService".into(),
            created_at: Utc::now(),
        }];
        let feedback = Arc::new(MemFeedback {
            pending: StdMutex::new(notes),
            committed: StdMutex::new(Vec::new()),
        });
        let questions = Arc::new(MemQuestions {
            registered: StdMutex::new(0),
            cancelled: StdMutex::new(Vec::new()),
        });
        let tasks = Arc::new(MemTasks {
            progress: StdMutex::new(Vec::new()),
            complete: StdMutex::new(Vec::new()),
        });
        let authoring = Arc::new(MemAuthoring {
            automations: StdMutex::new(0),
            tasks: StdMutex::new(0),
            reject: false,
        });
        let injection =
            test_injection(Arc::clone(&feedback), questions, tasks, authoring, false).await;
        let delivery = Arc::new(FeedbackDelivery::new(
            Arc::clone(&injection.feedback_access),
            "conn-1",
        ));
        let runtime = runtime_for(
            "check_user_feedback",
            injection,
            Some(Arc::clone(&delivery)),
        );
        let text = call_companion_tool(&runtime, "check_user_feedback", json!({}))
            .await
            .expect("feedback");
        assert!(text.contains("UserService"), "{text}");
        assert!(
            feedback.committed.lock().expect("c").is_empty(),
            "must not commit on read"
        );
        assert_eq!(delivery.pending_ids(), vec!["n1".to_string()]);

        let mut store = ContextStore::new("s");
        delivery.commit_present(&HashSet::new()).await;
        assert!(
            feedback.committed.lock().expect("c").is_empty(),
            "empty present-id set must not commit"
        );

        store.record_fact(ExecutionFact::pending(
            "s:1",
            "call_1",
            "check_user_feedback",
            json!({}),
        ));
        let present = projected_tool_call_ids(&store);
        delivery.commit_present(&present).await;
        assert_eq!(
            feedback.committed.lock().expect("c").clone(),
            vec!["n1".to_string()]
        );
        assert!(delivery.pending_ids().is_empty());
    }

    #[tokio::test]
    async fn ask_registers_one_card() {
        let feedback = Arc::new(MemFeedback {
            pending: StdMutex::new(Vec::new()),
            committed: StdMutex::new(Vec::new()),
        });
        let questions = Arc::new(MemQuestions {
            registered: StdMutex::new(0),
            cancelled: StdMutex::new(Vec::new()),
        });
        let tasks = Arc::new(MemTasks {
            progress: StdMutex::new(Vec::new()),
            complete: StdMutex::new(Vec::new()),
        });
        let authoring = Arc::new(MemAuthoring {
            automations: StdMutex::new(0),
            tasks: StdMutex::new(0),
            reject: false,
        });
        let injection =
            test_injection(feedback, Arc::clone(&questions), tasks, authoring, false).await;
        let runtime = runtime_for("ask_user_question", injection, None);
        let args = json!({
            "questions": [{
                "question": "Which approach?",
                "header": "Approach",
                "multiSelect": false,
                "options": [
                    { "label": "A", "description": "one" },
                    { "label": "B", "description": "two" }
                ]
            }]
        });
        let text = call_companion_tool(&runtime, "ask_user_question", args)
            .await
            .expect("ask");
        assert!(text.contains("Which approach"), "{text}");
        assert_eq!(*questions.registered.lock().expect("q"), 1);
    }

    #[tokio::test]
    async fn session_info_and_tasks_hit_real_access() {
        let feedback = Arc::new(MemFeedback {
            pending: StdMutex::new(Vec::new()),
            committed: StdMutex::new(Vec::new()),
        });
        let questions = Arc::new(MemQuestions {
            registered: StdMutex::new(0),
            cancelled: StdMutex::new(Vec::new()),
        });
        let tasks = Arc::new(MemTasks {
            progress: StdMutex::new(Vec::new()),
            complete: StdMutex::new(Vec::new()),
        });
        let authoring = Arc::new(MemAuthoring {
            automations: StdMutex::new(0),
            tasks: StdMutex::new(0),
            reject: false,
        });
        let injection =
            test_injection(feedback, questions, Arc::clone(&tasks), authoring, false).await;

        let info_rt = runtime_for("get_session_info", injection.clone(), None);
        let text = call_companion_tool(&info_rt, "get_session_info", json!({ "session_id": 7 }))
            .await
            .expect("info");
        assert!(text.contains("Fix auth"), "{text}");

        let prog_rt = runtime_for("task_progress", injection.clone(), None);
        let text = call_companion_tool(
            &prog_rt,
            "task_progress",
            json!({ "message": "tests passing" }),
        )
        .await
        .expect("progress");
        assert!(text.contains("Recorded"), "{text}");
        assert_eq!(
            tasks.progress.lock().expect("p").clone(),
            vec!["tests passing".to_string()]
        );

        let done_rt = runtime_for("task_complete", injection, None);
        let _ = call_companion_tool(
            &done_rt,
            "task_complete",
            json!({ "verdict": "success", "summary": "done" }),
        )
        .await
        .expect("complete");
        assert_eq!(
            tasks.complete.lock().expect("c").clone(),
            vec![("success".to_string(), Some("done".to_string()))]
        );
    }

    #[tokio::test]
    async fn authoring_call_time_reject_is_soft() {
        let feedback = Arc::new(MemFeedback {
            pending: StdMutex::new(Vec::new()),
            committed: StdMutex::new(Vec::new()),
        });
        let questions = Arc::new(MemQuestions {
            registered: StdMutex::new(0),
            cancelled: StdMutex::new(Vec::new()),
        });
        let tasks = Arc::new(MemTasks {
            progress: StdMutex::new(Vec::new()),
            complete: StdMutex::new(Vec::new()),
        });
        let authoring = Arc::new(MemAuthoring {
            automations: StdMutex::new(0),
            tasks: StdMutex::new(0),
            reject: true,
        });
        let injection = test_injection(feedback, questions, tasks, authoring, false).await;
        let runtime = runtime_for("create_automation", injection, None);
        let text = call_companion_tool(
            &runtime,
            "create_automation",
            json!({ "name": "Nightly", "prompt": "audit deps" }),
        )
        .await
        .expect("soft reject");
        assert!(text.contains("turned off"), "{text}");
    }

    #[test]
    fn identity_rejects_cross_tool_name() {
        let dir = std::env::temp_dir();
        let ctx = test_tool_ctx(&dir, "read_file", "call_x");
        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_x".into(),
            function_name: "read_file".into(),
        });
        assert!(is_companion_tool("check_user_feedback"));
        assert!(!is_companion_tool("read_file"));
    }
}

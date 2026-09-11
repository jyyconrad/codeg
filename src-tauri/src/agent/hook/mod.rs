//! One CodegHook per Runner: permission oneshot, deltas, RequestPatch, invalid Retry.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rig::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
    InvalidToolCallContext, ModelTurnAction, ModelTurnFinished, ObservationAction, RequestPatch,
    TextDelta, ToolCall, ToolCallAction, ToolResultAction, ToolResultEvent,
};
use rig::completion::Message;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::acp::session_state::SessionState;
use crate::acp::types::AcpEvent;
use crate::agent::context::budget::{
    per_call_patch, truncate_presentation, BudgetConfig, BudgetInputs,
};
use crate::agent::context::compact::{project_compacted, LlmCompactor};
use crate::agent::context::{
    AssistantPart, AssistantRecord, CallIdentity, CallIdentityBridge, ContextStore, ExecutionFact,
    FactRecorder, ModelCommit, ToolOutcome, ToolPhase,
};
use crate::agent::session::TurnCoordinator;
use crate::agent::tools::{
    acp_card_status_for_tool, attach_subagent_extra_context, projected_tool_call_ids, tool_kind,
    tool_requires_permission, FeedbackDelivery,
};
use crate::web::event_bridge::{emit_with_state, EventEmitter};

/// Decision for one tool-call permission card.
#[derive(Debug, Clone)]
pub enum PermissionDecision {
    Allow,
    Reject { reason: String },
    Cancel { reason: String },
}

/// In-flight permission wait. The hook parks on `reply`.
pub struct PendingPermission {
    pub tool_name: String,
    pub tool_call_id: Option<String>,
    pub args: String,
    pub reply: oneshot::Sender<PermissionDecision>,
}

impl PendingPermission {
    pub fn allow(self) {
        let _ = self.reply.send(PermissionDecision::Allow);
    }

    pub fn reject(self, reason: impl Into<String>) {
        let _ = self.reply.send(PermissionDecision::Reject {
            reason: reason.into(),
        });
    }

    pub fn cancel(self, reason: impl Into<String>) {
        let _ = self.reply.send(PermissionDecision::Cancel {
            reason: reason.into(),
        });
    }
}

/// How [`CodegHook::on_tool_call`] resolves permission.
pub enum PermissionPolicy {
    AutoAllow,
    Wait(mpsc::Sender<PendingPermission>),
}

/// Recorded Hook observations for contract tests and later transcript glue.
#[derive(Clone, Default)]
pub struct HookTrace {
    inner: Arc<Mutex<HookTraceInner>>,
}

#[derive(Default)]
struct HookTraceInner {
    text_deltas: Vec<String>,
    last_aggregated: String,
    tool_calls: Vec<String>,
    tool_results: Vec<ToolResultRecord>,
    invalid_calls: Vec<String>,
    completion_calls: usize,
    completion_histories: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ToolResultRecord {
    pub tool_name: String,
    pub status: String,
    pub presentation: String,
}

impl HookTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text_deltas(&self) -> Vec<String> {
        self.inner.lock().expect("hook trace").text_deltas.clone()
    }

    pub fn aggregated_text(&self) -> String {
        self.inner
            .lock()
            .expect("hook trace")
            .last_aggregated
            .clone()
    }

    pub fn tool_calls(&self) -> Vec<String> {
        self.inner.lock().expect("hook trace").tool_calls.clone()
    }

    pub fn tool_results(&self) -> Vec<ToolResultRecord> {
        self.inner.lock().expect("hook trace").tool_results.clone()
    }

    pub fn invalid_calls(&self) -> Vec<String> {
        self.inner.lock().expect("hook trace").invalid_calls.clone()
    }

    pub fn completion_calls(&self) -> usize {
        self.inner.lock().expect("hook trace").completion_calls
    }

    /// Serialized Runner history observed at each `on_completion_call`.
    /// This is canonical state, not the patched HTTP body.
    pub fn completion_histories(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("hook trace")
            .completion_histories
            .clone()
    }
}

/// Host event sink used by a live session Hook. Tests can omit this.
#[derive(Clone)]
pub struct HostBridge {
    pub emitter: EventEmitter,
    pub session_state: Arc<tokio::sync::RwLock<SessionState>>,
    pub turn: Arc<TurnCoordinator>,
    pub turn_id: u64,
}

/// Per-run native state: budget, facts, call identity.
#[derive(Clone)]
pub struct NativeRunState {
    pub turn_id: u64,
    pub turn_key: String,
    pub budget: BudgetConfig,
    pub preamble: String,
    pub tool_schemas: Vec<Value>,
    pub store: Arc<Mutex<ContextStore>>,
    pub identity: Arc<CallIdentityBridge>,
    pub recorder: Arc<FactRecorder>,
    pub last_estimate: Arc<Mutex<u64>>,
    pub last_usage_input: Arc<Mutex<Option<u64>>>,
    pub(crate) feedback: Option<Arc<FeedbackDelivery>>,
    /// MCP tools whose original `readOnlyHint` is true skip the permission card.
    pub mcp_readonly: Arc<HashSet<String>>,
    pub compact: Option<LlmCompactor>,
}

/// Unique per-run Hook. Register only on the Runner, never on AgentBuilder.
pub struct CodegHook {
    permission: PermissionPolicy,
    cancel: CancellationToken,
    trace: HookTrace,
    history_override: Option<Vec<Message>>,
    max_tokens: Option<u64>,
    host: Option<HostBridge>,
    native: Option<NativeRunState>,
}

impl CodegHook {
    pub fn auto_allow(trace: HookTrace) -> Self {
        Self {
            permission: PermissionPolicy::AutoAllow,
            cancel: CancellationToken::new(),
            trace,
            history_override: None,
            max_tokens: None,
            host: None,
            native: None,
        }
    }

    pub fn waiting(
        trace: HookTrace,
        permissions: mpsc::Sender<PendingPermission>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            permission: PermissionPolicy::Wait(permissions),
            cancel,
            trace,
            history_override: None,
            max_tokens: None,
            host: None,
            native: None,
        }
    }

    pub fn for_session(
        trace: HookTrace,
        permissions: mpsc::Sender<PendingPermission>,
        cancel: CancellationToken,
        host: HostBridge,
    ) -> Self {
        Self {
            permission: PermissionPolicy::Wait(permissions),
            cancel,
            trace,
            history_override: None,
            max_tokens: None,
            host: Some(host),
            native: None,
        }
    }

    /// Replace request history / max_tokens on every `on_completion_call`.
    /// Does not rewrite Runner persisted history.
    pub fn with_request_patch(mut self, history: Vec<Message>, max_tokens: Option<u64>) -> Self {
        self.history_override = Some(history);
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_native(mut self, native: NativeRunState) -> Self {
        self.native = Some(native);
        self
    }

    fn request_patch(&self) -> Option<RequestPatch> {
        match (&self.history_override, self.max_tokens) {
            (None, None) => None,
            (history, max_tokens) => Some(per_call_patch(
                history.clone().unwrap_or_default(),
                max_tokens,
            )),
        }
    }

    async fn emit_tool_call(&self, event: &ToolCall<'_>, status: &str) {
        let Some(host) = &self.host else {
            return;
        };
        if !host.turn.is_current(host.turn_id) {
            return;
        }
        emit_with_state(
            &host.session_state,
            &host.emitter,
            AcpEvent::ToolCall {
                tool_call_id: tool_call_id(event),
                title: event.tool_name.to_string(),
                kind: tool_kind(event.tool_name).to_string(),
                status: status.to_string(),
                content: None,
                raw_input: Some(event.args.to_string()),
                raw_output: None,
                locations: None,
                meta: None,
                images: None,
            },
        )
        .await;
    }

    async fn emit_tool_update(&self, tool_call_id: &str, status: &str, output: Option<String>) {
        let Some(host) = &self.host else {
            return;
        };
        if !host.turn.is_current(host.turn_id) {
            return;
        }
        emit_with_state(
            &host.session_state,
            &host.emitter,
            AcpEvent::ToolCallUpdate {
                tool_call_id: tool_call_id.to_string(),
                title: None,
                status: Some(status.to_string()),
                content: output.clone(),
                raw_input: None,
                raw_output: output,
                raw_output_append: None,
                locations: None,
                meta: None,
                images: None,
            },
        )
        .await;
    }

    async fn record_unexecuted(&self, event: &ToolCall<'_>, outcome: ToolOutcome, reason: String) {
        let Some(native) = &self.native else {
            return;
        };
        let args = serde_json::from_str(event.args)
            .unwrap_or_else(|_| Value::String(event.args.to_string()));
        let mut fact = ExecutionFact::pending(
            native.turn_key.clone(),
            tool_call_id(event),
            event.tool_name,
            args,
        );
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(outcome);
        fact.executed = Some(false);
        fact.reason = Some(reason);
        fact.model_presentation = Some(fact.outcome_feedback());
        let _ = native.recorder.record_terminal(&fact).await;
    }
}

fn tool_call_id(event: &ToolCall<'_>) -> String {
    event
        .tool_call_id
        .map(str::to_string)
        .unwrap_or_else(|| event.internal_call_id.to_string())
}

fn tool_result_id(event: &ToolResultEvent<'_>) -> String {
    event
        .tool_call_id
        .map(str::to_string)
        .unwrap_or_else(|| event.internal_call_id.to_string())
}

impl AgentHook for CodegHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let history =
            serde_json::to_string(event.history).unwrap_or_else(|_| format!("{:?}", event.history));
        {
            let mut inner = self.trace.inner.lock().expect("hook trace");
            inner.completion_calls += 1;
            inner.completion_histories.push(history);
        }
        if self.cancel.is_cancelled() {
            return CompletionCallAction::stop("cancelled");
        }
        if let Some(native) = &self.native {
            let snapshot = native.store.lock().expect("store").clone();
            let compact = native.compact.clone();
            let result = project_compacted(
                BudgetInputs {
                    store: &snapshot,
                    config: native.budget,
                    preamble: &native.preamble,
                    tool_schemas: &native.tool_schemas,
                    prompt: event.prompt,
                },
                compact.as_ref(),
            )
            .await;
            let (action, present) = match result {
                Ok((view, new_record)) => {
                    if let Some(record) = new_record {
                        native
                            .store
                            .lock()
                            .expect("store")
                            .set_compact(record.clone());
                        let _ = native.recorder.record_compact(&record).await;
                    }
                    *native.last_estimate.lock().expect("estimate") = view.estimated_tokens;
                    let store = native.store.lock().expect("store");
                    let present = projected_tool_call_ids(&store, view.omitted_turns);
                    (
                        CompletionCallAction::patch(attach_subagent_extra_context(
                            per_call_patch(view.messages, Some(native.budget.max_output)),
                            &native.tool_schemas,
                        )),
                        Some(present),
                    )
                }
                Err(err) => (CompletionCallAction::stop(err.to_string()), None),
            };
            if let (Some(delivery), Some(present)) = (&native.feedback, present) {
                delivery.commit_present(&present).await;
            }
            if self.cancel.is_cancelled() {
                return CompletionCallAction::stop("cancelled");
            }
            return action;
        }
        match self.request_patch() {
            Some(patch) => CompletionCallAction::patch(patch),
            None => CompletionCallAction::continue_run(),
        }
    }

    async fn on_text_delta(&self, _ctx: &HookContext, event: TextDelta<'_>) -> ObservationAction {
        {
            let mut inner = self.trace.inner.lock().expect("hook trace");
            inner.text_deltas.push(event.delta.to_string());
            inner.last_aggregated = event.aggregated.to_string();
        }
        if let Some(host) = &self.host {
            if host.turn.is_current(host.turn_id) && !event.delta.is_empty() {
                emit_with_state(
                    &host.session_state,
                    &host.emitter,
                    AcpEvent::ContentDelta {
                        text: event.delta.to_string(),
                        parent_tool_use_id: None,
                    },
                )
                .await;
            }
        }
        ObservationAction::continue_run()
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        if let Some(native) = &self.native {
            if let Err(err) = record_model_batch(native, event.content).await {
                return ModelTurnAction::stop(err);
            }
            let used = if event.usage.input_tokens > 0 {
                event.usage.input_tokens
            } else {
                *native.last_estimate.lock().expect("estimate")
            };
            *native.last_usage_input.lock().expect("usage") = Some(used);
            if let Some(host) = &self.host {
                if host.turn.is_current(host.turn_id) {
                    emit_with_state(
                        &host.session_state,
                        &host.emitter,
                        AcpEvent::UsageUpdate {
                            used,
                            size: native.budget.window,
                        },
                    )
                    .await;
                }
            }
        }
        ModelTurnAction::continue_run()
    }

    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        self.trace
            .inner
            .lock()
            .expect("hook trace")
            .invalid_calls
            .push(event.tool_name.clone());
        if self.cancel.is_cancelled() {
            return Some(InvalidToolCallAction::stop("cancelled"));
        }
        Some(InvalidToolCallAction::retry(format!(
            "unknown or disallowed tool `{}`; retry with a registered tool",
            event.tool_name
        )))
    }

    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        self.trace
            .inner
            .lock()
            .expect("hook trace")
            .tool_calls
            .push(event.tool_name.to_string());

        if self.cancel.is_cancelled() {
            self.record_unexecuted(&event, ToolOutcome::Cancelled, "cancelled".into())
                .await;
            self.emit_tool_call(&event, "failed").await;
            return ToolCallAction::stop("cancelled");
        }

        if let Some(native) = &self.native {
            native.identity.set(CallIdentity {
                turn_id: native.turn_id,
                turn_key: native.turn_key.clone(),
                tool_call_id: tool_call_id(&event),
                function_name: event.tool_name.to_string(),
            });
        }

        self.emit_tool_call(&event, "pending").await;

        let mcp_readonly = self
            .native
            .as_ref()
            .is_some_and(|native| native.mcp_readonly.contains(event.tool_name));
        let wait_tx = match &self.permission {
            PermissionPolicy::AutoAllow => None,
            PermissionPolicy::Wait(_)
                if !tool_requires_permission(event.tool_name) || mcp_readonly =>
            {
                None
            }
            PermissionPolicy::Wait(tx) => Some(tx.clone()),
        };
        let Some(tx) = wait_tx else {
            return ToolCallAction::run();
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = PendingPermission {
            tool_name: event.tool_name.to_string(),
            tool_call_id: event.tool_call_id.map(str::to_string),
            args: event.args.to_string(),
            reply: reply_tx,
        };
        if tx.send(pending).await.is_err() {
            return ToolCallAction::stop("permission channel closed");
        }
        tokio::select! {
            _ = self.cancel.cancelled() => {
                self.record_unexecuted(&event, ToolOutcome::Cancelled, "cancelled".into())
                    .await;
                self.emit_tool_update(&tool_call_id(&event), "failed", Some("cancelled".into()))
                    .await;
                ToolCallAction::stop("cancelled")
            }
            decision = reply_rx => match decision {
                Ok(PermissionDecision::Allow) => ToolCallAction::run(),
                Ok(PermissionDecision::Reject { reason }) => {
                    self.record_unexecuted(&event, ToolOutcome::Rejected, reason.clone())
                        .await;
                    self.emit_tool_update(
                        &tool_call_id(&event),
                        "failed",
                        Some(reason.clone()),
                    )
                    .await;
                    ToolCallAction::skip(reason)
                }
                Ok(PermissionDecision::Cancel { reason }) => {
                    self.record_unexecuted(&event, ToolOutcome::Cancelled, reason.clone())
                        .await;
                    self.emit_tool_update(
                        &tool_call_id(&event),
                        "failed",
                        Some(reason.clone()),
                    )
                    .await;
                    ToolCallAction::stop(reason)
                }
                Err(_) => ToolCallAction::stop("permission dropped"),
            }
        }
    }

    async fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let presentation = event
            .presentation
            .as_text()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{:?}", event.presentation));
        let status_name = event.raw_result.status_name().to_string();
        self.trace
            .inner
            .lock()
            .expect("hook trace")
            .tool_results
            .push(ToolResultRecord {
                tool_name: event.tool_name.to_string(),
                status: status_name.clone(),
                presentation: presentation.clone(),
            });
        let card_status = acp_card_status_for_tool(event.tool_name, &status_name);
        let id = tool_result_id(&event);
        if let Some(native) = &self.native {
            native.identity.clear();
            let remaining = native
                .budget
                .input_budget()
                .unwrap_or(native.budget.window)
                .saturating_sub(*native.last_estimate.lock().expect("estimate"));
            let (shown, truncated) = truncate_presentation(&presentation, remaining.max(1024));
            native
                .store
                .lock()
                .expect("store")
                .upsert_fact(&id, |fact| {
                    fact.model_presentation = Some(shown.clone());
                    fact.truncated = truncated;
                });
            self.emit_tool_update(&id, card_status, Some(shown.clone()))
                .await;
            if truncated {
                return ToolResultAction::rewrite(shown);
            }
            return ToolResultAction::keep();
        }
        self.emit_tool_update(&id, card_status, Some(presentation))
            .await;
        ToolResultAction::keep()
    }
}

async fn record_model_batch(
    native: &NativeRunState,
    content: &[rig::completion::message::AssistantContent],
) -> Result<(), String> {
    let mut parts = Vec::new();
    let mut call_ids = Vec::new();
    let mut last_payload = None;
    for (index, item) in content.iter().enumerate() {
        match item {
            rig::completion::message::AssistantContent::Text(text) => {
                parts.push(AssistantPart::Text(text.text.clone()));
                let mut meta = crate::agent::context::transcript::NativeMeta::v1();
                meta.turn_id = Some(native.turn_key.clone());
                meta.part_index = Some(index as u32);
                last_payload = Some(crate::agent::context::transcript::agent_message_chunk(
                    &text.text, &meta,
                ));
            }
            rig::completion::message::AssistantContent::ToolCall(tc) => {
                let id = tc
                    .provider
                    .as_ref()
                    .map(|p| p.call_id.clone())
                    .unwrap_or_else(|| tc.id.as_ref().to_string());
                let name = tc.function.name.clone();
                let args = tc.function.arguments.clone();
                call_ids.push(id.clone());
                parts.push(AssistantPart::ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                });
                let fact = ExecutionFact::pending(
                    native.turn_key.clone(),
                    id.clone(),
                    name.clone(),
                    args.clone(),
                );
                native
                    .store
                    .lock()
                    .expect("store")
                    .record_fact(fact.clone());
                last_payload = Some(crate::agent::context::transcript::tool_call_payload(
                    &id,
                    &name,
                    "pending",
                    &args,
                    &fact.native_meta(),
                ));
            }
            _ => {}
        }
    }
    let commit = ModelCommit {
        batch_id: format!("{}-{}", native.turn_key, call_ids.len()),
        parts: (0..parts.len()).map(|i| format!("p{i}")).collect(),
        call_ids,
    };
    native
        .recorder
        .record_model_commit(&native.turn_key, &commit, last_payload)
        .await
        .map_err(|_| "model_commit write was not acknowledged".to_string())?;
    native.store.lock().expect("store").commit_assistant(
        &native.turn_key,
        AssistantRecord {
            model_message_id: Some(commit.batch_id.clone()),
            committed: true,
            parts,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::{attach_subagent_extra_context, SUBAGENT_SPEC_ID};
    use serde_json::json;

    #[test]
    fn extra_context_document_is_attached_when_subagent_is_registered() {
        let patch = per_call_patch(Vec::new(), Some(128));
        let without = attach_subagent_extra_context(patch.clone(), &[json!({"name": "read_file"})]);
        assert!(
            without.extra_context.is_empty(),
            "unregistered subagent must not insert the spec document"
        );
        let with = attach_subagent_extra_context(patch, &[json!({"name": "subagent"})]);
        assert_eq!(with.extra_context.len(), 1);
        assert_eq!(with.extra_context[0].id, SUBAGENT_SPEC_ID);
    }
}

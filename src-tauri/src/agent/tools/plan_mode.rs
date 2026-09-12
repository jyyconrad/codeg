//! `enter_plan_mode` / `exit_plan_mode` for the in-process Codeg Agent.

use std::path::PathBuf;
use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;

use super::NativeToolCtx;
use crate::acp::plan_approval::{PlanApprovalDecision, SessionPlanApprovalAccess};
use crate::acp::session_state::SessionState;
use crate::agent::mode::{apply_mode, plan_path, MODE_CODE, MODE_PLAN};
use crate::web::event_bridge::EventEmitter;

#[derive(Clone)]
pub struct EnterPlanModeTool {
    ctx: NativeToolCtx,
    session_mode: Arc<RwLock<String>>,
    session_state: Arc<RwLock<SessionState>>,
    emitter: EventEmitter,
    artifacts_dir: PathBuf,
}

#[derive(Clone)]
pub struct ExitPlanModeTool {
    ctx: NativeToolCtx,
    session_mode: Arc<RwLock<String>>,
    session_state: Arc<RwLock<SessionState>>,
    emitter: EventEmitter,
    artifacts_dir: PathBuf,
    connection_id: String,
    plan_approvals: Option<Arc<dyn SessionPlanApprovalAccess>>,
    pending_continue: Arc<std::sync::Mutex<Option<String>>>,
}

impl EnterPlanModeTool {
    pub fn new(
        ctx: NativeToolCtx,
        session_mode: Arc<RwLock<String>>,
        session_state: Arc<RwLock<SessionState>>,
        emitter: EventEmitter,
        artifacts_dir: PathBuf,
    ) -> Self {
        Self {
            ctx,
            session_mode,
            session_state,
            emitter,
            artifacts_dir,
        }
    }
}

impl ExitPlanModeTool {
    pub fn new(
        ctx: NativeToolCtx,
        session_mode: Arc<RwLock<String>>,
        session_state: Arc<RwLock<SessionState>>,
        emitter: EventEmitter,
        artifacts_dir: PathBuf,
        connection_id: String,
        plan_approvals: Option<Arc<dyn SessionPlanApprovalAccess>>,
        pending_continue: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            ctx,
            session_mode,
            session_state,
            emitter,
            artifacts_dir,
            connection_id,
            plan_approvals,
            pending_continue,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EnterPlanModeArgs {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExitPlanModeArgs {}

impl Tool for EnterPlanModeTool {
    const NAME: &'static str = "enter_plan_mode";
    type Args = EnterPlanModeArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Request to enter plan mode. Source edits and bash are blocked until the user \
approves a plan via exit_plan_mode. Call this when the implementation approach is ambiguous, \
including mid-implementation if evidence invalidates the current approach."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "Why plan mode is needed" }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({ "reason": args.reason });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        if self.session_mode.read().await.as_str() == MODE_PLAN {
            return self
                .ctx
                .finish_ok(fact, "Already in plan mode.".to_string())
                .await;
        }
        apply_mode(
            &self.session_state,
            &self.emitter,
            &self.session_mode,
            &self.artifacts_dir,
            MODE_PLAN,
        )
        .await;
        let reason = args
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("planning");
        self.ctx
            .finish_ok(
                fact,
                format!(
                    "Entered plan mode ({reason}). Research, write_plan, then exit_plan_mode. \
Source edits apply on the next turn after the plan is approved."
                ),
            )
            .await
    }
}

impl Tool for ExitPlanModeTool {
    const NAME: &'static str = "exit_plan_mode";
    type Args = ExitPlanModeArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Present the written plan for user approval and leave plan mode if approved. \
Reads the session plan.md. Stay in plan mode if the user requests changes."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let fact = self.ctx.begin(Self::NAME, json!({})).await?;
        let path = plan_path(&self.artifacts_dir);
        let markdown = std::fs::read_to_string(&path).unwrap_or_default();
        let Some(access) = self.plan_approvals.as_ref() else {
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::other("plan approval is not available")
                        .with_model_feedback("plan approval host is not available"),
                )
                .await);
        };
        let Some(registered) = access
            .register_plan_approval(&self.connection_id, fact.tool_call_id.clone(), markdown)
            .await
        else {
            return Err(self
                .ctx
                .finish_err(
                    fact,
                    ToolExecutionError::other("a plan approval is already pending")
                        .with_model_feedback("a plan approval is already pending"),
                )
                .await);
        };
        let answer = match registered.answer_rx.await {
            Ok(answer) => answer,
            Err(_) => {
                return Err(self
                    .ctx
                    .finish_err(
                        fact,
                        ToolExecutionError::cancelled("plan approval cancelled")
                            .with_model_feedback("plan approval was cancelled; stay in plan mode"),
                    )
                    .await);
            }
        };
        let feedback = answer.normalized_feedback();
        match answer.decision {
            PlanApprovalDecision::Approve => {
                apply_mode(
                    &self.session_state,
                    &self.emitter,
                    &self.session_mode,
                    &self.artifacts_dir,
                    MODE_CODE,
                )
                .await;
                let mut continue_text = "The user approved the plan. Implement it now.".to_string();
                if !feedback.is_empty() {
                    continue_text.push_str("\n\nReview comments:\n");
                    continue_text.push_str(&feedback);
                }
                *self.pending_continue.lock().expect("pending continue") = Some(continue_text);
                self.ctx
                    .finish_ok(
                        fact,
                        "Plan approved. Implementation starts on the next turn.".into(),
                    )
                    .await
            }
            PlanApprovalDecision::RequestChanges => {
                self.ctx
                    .finish_ok(
                        fact,
                        format!(
                            "Stay in plan mode and revise the plan.\n{}",
                            if feedback.is_empty() {
                                "(no additional notes)".to_string()
                            } else {
                                feedback
                            }
                        ),
                    )
                    .await
            }
            PlanApprovalDecision::Abandon => {
                apply_mode(
                    &self.session_state,
                    &self.emitter,
                    &self.session_mode,
                    &self.artifacts_dir,
                    MODE_CODE,
                )
                .await;
                self.ctx
                    .finish_ok(fact, "Plan abandoned. Plan mode is off.".into())
                    .await
            }
        }
    }
}

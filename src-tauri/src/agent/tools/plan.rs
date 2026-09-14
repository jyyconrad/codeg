//! Native `update_plan` tool. Full-replaces the session plan via `PlanUpdate`.
//! Does not implement plan mode (`EnterPlanMode` / mode switch).

use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;

use super::NativeToolCtx;
use crate::acp::session_state::SessionState;
use crate::acp::types::{AcpEvent, PlanEntryInfo};
use crate::web::event_bridge::{emit_with_state, EventEmitter};

const PLAN_STATUSES: &[&str] = &["pending", "in_progress", "completed"];
const PLAN_PRIORITIES: &[&str] = &["low", "medium", "high"];

/// Host-facing plan tool. Emits `AcpEvent::PlanUpdate` through the same
/// `emit_with_state` path as other session events.
#[derive(Clone)]
pub struct UpdatePlanTool {
    ctx: NativeToolCtx,
    emitter: EventEmitter,
    session_state: Arc<RwLock<SessionState>>,
}

impl UpdatePlanTool {
    pub fn new(
        ctx: NativeToolCtx,
        emitter: EventEmitter,
        session_state: Arc<RwLock<SessionState>>,
    ) -> Self {
        Self {
            ctx,
            emitter,
            session_state,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePlanArgs {
    pub entries: Vec<PlanEntryInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanEntryInput {
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
}

impl Tool for UpdatePlanTool {
    const NAME: &'static str = "update_plan";
    type Args = UpdatePlanArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Replace the current session plan with the given entries. Provide the \
         full list each time (full replace, not a patch). Each entry needs \
         content and status (pending, in_progress, completed); priority \
         (low, medium, high) defaults to medium."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "entries": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["low", "medium", "high"]
                            }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["entries"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "entries": args.entries.iter().map(|entry| {
                json!({
                    "content": entry.content,
                    "status": entry.status,
                    "priority": entry.priority,
                })
            }).collect::<Vec<_>>(),
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match parse_plan_entries(&args.entries) {
            Ok(entries) => {
                let presentation = plan_presentation(&entries);
                emit_with_state(
                    &self.session_state,
                    &self.emitter,
                    AcpEvent::PlanUpdate { entries },
                )
                .await;
                self.ctx.finish_ok(fact, presentation).await
            }
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

fn parse_plan_entries(
    entries: &[PlanEntryInput],
) -> Result<Vec<PlanEntryInfo>, ToolExecutionError> {
    let mut out = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let status = entry.status.trim();
        if !PLAN_STATUSES.contains(&status) {
            let message = format!("entries[{i}].status must be pending, in_progress, or completed");
            let feedback = format!(
                "invalid plan status {:?}; expected pending, in_progress, or completed",
                entry.status
            );
            return Err(ToolExecutionError::invalid_args(message).with_model_feedback(feedback));
        }
        let priority = match entry
            .priority
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => "medium",
            Some(priority) if PLAN_PRIORITIES.contains(&priority) => priority,
            Some(priority) => {
                let message = format!("entries[{i}].priority must be low, medium, or high");
                let feedback =
                    format!("invalid plan priority {priority:?}; expected low, medium, or high");
                return Err(ToolExecutionError::invalid_args(message).with_model_feedback(feedback));
            }
        };
        out.push(PlanEntryInfo {
            content: entry.content.clone(),
            status: status.to_string(),
            priority: priority.to_string(),
        });
    }
    Ok(out)
}

fn plan_presentation(entries: &[PlanEntryInfo]) -> String {
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut completed = 0usize;
    for entry in entries {
        match entry.status.as_str() {
            "pending" => pending += 1,
            "in_progress" => in_progress += 1,
            "completed" => completed += 1,
            _ => {}
        }
    }
    format!(
        "Plan updated: {} entries ({} pending, {} in_progress, {} completed)",
        entries.len(),
        pending,
        in_progress,
        completed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::session_state::LiveContentBlock;
    use crate::acp::types::AcpEvent;
    use crate::agent::context::ToolOutcome;
    use crate::agent::tools::{test_tool_ctx, tool_kind, tool_requires_permission};
    use crate::models::agent::AgentType;
    use crate::web::event_bridge::EventEmitter;
    use rig::tool::Tool;
    use serde_json::json;

    fn plan_harness() -> (UpdatePlanTool, NativeToolCtx, Arc<RwLock<SessionState>>) {
        let cwd = std::path::Path::new("/tmp");
        let ctx = test_tool_ctx(cwd, UpdatePlanTool::NAME, "call_plan");
        let state = Arc::new(RwLock::new(SessionState::new(
            "conn-plan".into(),
            AgentType::CodegAgent,
            Some(cwd.to_path_buf()),
            "main".into(),
            None,
        )));
        let tool = UpdatePlanTool::new(ctx.clone(), EventEmitter::Noop, Arc::clone(&state));
        (tool, ctx, state)
    }

    fn sample_entries() -> Vec<PlanEntryInput> {
        vec![
            PlanEntryInput {
                content: "design".into(),
                status: "completed".into(),
                priority: Some("high".into()),
            },
            PlanEntryInput {
                content: "implement".into(),
                status: "in_progress".into(),
                priority: None,
            },
            PlanEntryInput {
                content: "test".into(),
                status: "pending".into(),
                priority: Some("low".into()),
            },
        ]
    }

    fn live_plan_entries(state: &SessionState) -> Vec<PlanEntryInfo> {
        let live = state
            .live_message
            .as_ref()
            .expect("PlanUpdate must create live_message");
        let plan_blocks: Vec<_> = live
            .content
            .iter()
            .filter(|block| matches!(block, LiveContentBlock::Plan { .. }))
            .collect();
        assert_eq!(plan_blocks.len(), 1, "exactly one Plan block");
        match plan_blocks[0] {
            LiveContentBlock::Plan { entries } => {
                serde_json::from_value(entries.clone()).expect("PlanEntryInfo[]")
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn parse_plan_entries_defaults_priority() {
        let entries = parse_plan_entries(&sample_entries()).expect("valid");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].content, "design");
        assert_eq!(entries[0].status, "completed");
        assert_eq!(entries[0].priority, "high");
        assert_eq!(entries[1].content, "implement");
        assert_eq!(entries[1].status, "in_progress");
        assert_eq!(entries[1].priority, "medium");
        assert_eq!(entries[2].priority, "low");
    }

    #[test]
    fn invalid_status_is_rejected() {
        let err = parse_plan_entries(&[PlanEntryInput {
            content: "x".into(),
            status: "running".into(),
            priority: None,
        }])
        .expect_err("running is not a plan status");
        assert!(
            err.message().contains("status"),
            "message={}",
            err.message()
        );
        assert_eq!(
            err.model_feedback(),
            Some("invalid plan status \"running\"; expected pending, in_progress, or completed")
        );
    }

    #[test]
    fn update_plan_is_other_kind_and_skips_permission() {
        assert_eq!(tool_kind("update_plan"), "other");
        assert!(!tool_requires_permission("update_plan"));
        assert_eq!(tool_kind("enter_plan_mode"), "think");
        assert!(tool_requires_permission("enter_plan_mode"));
        assert!(!tool_requires_permission("exit_plan_mode"));
        assert!(!tool_requires_permission("write_plan"));
        assert!(!tool_requires_permission("write_explore_report"));
    }

    #[test]
    fn parameters_match_spec_shape() {
        let (tool, _, _) = plan_harness();
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert_eq!(params["required"], json!(["entries"]));
        assert_eq!(
            params["properties"]["entries"]["items"]["required"],
            json!(["content", "status"])
        );
        assert_eq!(
            params["properties"]["entries"]["items"]["properties"]["status"]["enum"],
            json!(["pending", "in_progress", "completed"])
        );
        assert_eq!(
            params["properties"]["entries"]["items"]["properties"]["priority"]["enum"],
            json!(["low", "medium", "high"])
        );
    }

    #[tokio::test]
    async fn valid_entries_emit_plan_update() {
        let (tool, ctx, state) = plan_harness();
        let mut rx = state.read().await.event_stream().subscribe();
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                UpdatePlanArgs {
                    entries: sample_entries(),
                },
            )
            .await
            .expect("update_plan");
        assert!(
            out.contains("3 entries")
                && out.contains("1 pending")
                && out.contains("1 in_progress")
                && out.contains("1 completed"),
            "{out}"
        );

        let env = rx.try_recv().expect("PlanUpdate event");
        match &env.payload {
            AcpEvent::PlanUpdate { entries } => {
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[0].content, "design");
                assert_eq!(entries[0].status, "completed");
                assert_eq!(entries[0].priority, "high");
                assert_eq!(entries[1].priority, "medium");
                assert_eq!(entries[2].status, "pending");
            }
            other => panic!("expected PlanUpdate, got {other:?}"),
        }
        assert!(live_plan_entries(&*state.read().await)
            .iter()
            .any(|e| e.content == "implement" && e.priority == "medium"));

        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_plan")
            .cloned();
        assert_eq!(
            fact.as_ref().and_then(|f| f.outcome),
            Some(ToolOutcome::Success)
        );

        let out = tool
            .call(
                &mut tctx,
                UpdatePlanArgs {
                    entries: vec![PlanEntryInput {
                        content: "only".into(),
                        status: "pending".into(),
                        priority: None,
                    }],
                },
            )
            .await
            .expect("replace");
        assert!(out.contains("1 entries"), "{out}");
        let replaced = live_plan_entries(&*state.read().await);
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].content, "only");
    }

    #[tokio::test]
    async fn invalid_status_does_not_emit_plan_update() {
        let (tool, ctx, state) = plan_harness();
        let mut rx = state.read().await.event_stream().subscribe();
        let mut tctx = ToolContext::new();
        let err = tool
            .call(
                &mut tctx,
                UpdatePlanArgs {
                    entries: vec![PlanEntryInput {
                        content: "x".into(),
                        status: "running".into(),
                        priority: None,
                    }],
                },
            )
            .await
            .expect_err("invalid status");
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("invalid plan status"),
            "{err:?}"
        );
        assert!(
            rx.try_recv().is_err(),
            "invalid status must not emit PlanUpdate"
        );
        assert!(state.read().await.live_message.is_none());
        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_plan")
            .cloned();
        assert_eq!(
            fact.as_ref().and_then(|f| f.outcome),
            Some(ToolOutcome::Error)
        );
    }

    #[tokio::test]
    async fn update_plan_does_not_enter_plan_mode() {
        let (tool, _, state) = plan_harness();
        {
            let mut session = state.write().await;
            session.current_mode = Some("code".into());
        }
        let mut rx = state.read().await.event_stream().subscribe();
        let mut tctx = ToolContext::new();
        tool.call(
            &mut tctx,
            UpdatePlanArgs {
                entries: vec![PlanEntryInput {
                    content: "step".into(),
                    status: "pending".into(),
                    priority: None,
                }],
            },
        )
        .await
        .expect("update_plan");

        let env = rx.try_recv().expect("PlanUpdate");
        assert!(
            matches!(&env.payload, AcpEvent::PlanUpdate { .. }),
            "expected PlanUpdate, got {:?}",
            env.payload
        );
        assert!(
            rx.try_recv().is_err(),
            "update_plan must not emit ModeChanged or plan-mode events"
        );
        let session = state.read().await;
        assert_eq!(session.current_mode.as_deref(), Some("code"));
        assert!(session.pending_plan_approval.is_none());
    }
}

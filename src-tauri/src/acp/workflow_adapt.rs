//! Agent-specific workflow wire → canonical [`WorkflowDelta`].
//!
//! Long-running background workflows are not one ACP variant. Grok publishes
//! `_x.ai/session/update` `workflow_updated`; Claude (and any AIR speaker)
//! publishes `async_task_*` with `taskType: "workflow"`; Codex/OpenCode can
//! join the same AIR channel later without a UI change. This module is the
//! conversion layer: one output shape, one live strip.

use std::collections::HashSet;

use serde_json::Value;

use crate::acp::types::{AsyncTaskDelta, WorkflowAgent, WorkflowDelta, WorkflowPhase};
use crate::models::agent::AgentType;

const GROK_EXT_UPDATE_METHODS: [&str; 2] = ["_x.ai/session_notification", "_x.ai/session/update"];

/// Grok `workflow_updated` → [`WorkflowDelta`]. `seen` tracks run ids already
/// announced on this connection so only the first frame is `spawned`.
pub fn adapt_grok_workflow(
    agent_type: AgentType,
    method: &str,
    params: &Value,
    seen: &mut HashSet<String>,
) -> Option<WorkflowDelta> {
    if !matches!(agent_type, AgentType::Grok) {
        return None;
    }
    if !GROK_EXT_UPDATE_METHODS.contains(&method) {
        return None;
    }
    let update = params.get("update")?;
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("workflow_updated") {
        return None;
    }
    let run_id = update
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let spawned = seen.insert(run_id.clone());
    Some(grok_delta(run_id, spawned, update))
}

/// AIR `async_task` with `taskType=workflow` (Claude `local_workflow` today;
/// Codex/OpenCode if they start publishing the same friendly type) →
/// [`WorkflowDelta`]. `known` is true when this id is already a workflow row,
/// so a progress frame that omits `taskType` still routes here.
pub fn adapt_air_workflow(delta: &AsyncTaskDelta, known: bool) -> Option<WorkflowDelta> {
    let is_workflow = delta.task_type.as_deref() == Some("workflow") || known;
    if !is_workflow {
        return None;
    }
    Some(WorkflowDelta {
        run_id: delta.task_id.clone(),
        spawned: delta.spawned,
        name: delta.name.clone(),
        objective: delta.description.clone().filter(|s| !s.is_empty()),
        state: delta.state.clone(),
        phases: None,
        current_phase: delta.last_tool_name.clone(),
        agents: None,
        agents_done: None,
        agents_running: None,
        agents_used: delta.usage.as_ref().map(|u| u.tool_uses as u32),
        agent_budget: None,
        agents_remaining: None,
        elapsed_ms: delta.usage.as_ref().map(|u| u.duration_ms),
        last_event: delta.summary.clone().or(delta.last_tool_name.clone()),
        last_event_detail: None,
        can_stop: delta.can_stop,
    })
}

fn grok_delta(run_id: String, spawned: bool, update: &Value) -> WorkflowDelta {
    let status = update.get("status").and_then(Value::as_str).unwrap_or("");
    let state = match status {
        "complete" | "completed" => "completed",
        "failed" => "failed",
        "cancelled" | "canceled" | "interrupted" => "stopped",
        "paused" => "paused",
        _ => "running",
    }
    .to_string();
    let current_phase = update
        .get("current_phase")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let phases = update.get("phases").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(|p| {
                let title = p
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())?
                    .to_string();
                let state = p
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("pending")
                    .to_string();
                Some(WorkflowPhase {
                    title,
                    detail: opt_trim(p.get("detail")),
                    state,
                })
            })
            .collect::<Vec<_>>()
    });
    let agents = grok_agents(update);
    let active_agents = update
        .get("active_agents")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let (done, running) = agent_counts(agents.as_ref(), active_agents);
    WorkflowDelta {
        run_id,
        spawned,
        name: update
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        objective: update
            .get("objective")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        state: Some(state),
        phases,
        current_phase,
        agents,
        agents_done: Some(done),
        agents_running: Some(running),
        agents_used: update
            .get("agents_used")
            .and_then(Value::as_u64)
            .map(|n| n as u32),
        agent_budget: update
            .get("agent_budget")
            .and_then(Value::as_u64)
            .map(|n| n as u32),
        agents_remaining: update
            .get("agents_remaining")
            .and_then(Value::as_u64)
            .map(|n| n as u32),
        elapsed_ms: update.get("elapsed_ms").and_then(Value::as_u64),
        last_event: opt_trim(update.get("last_event")),
        last_event_detail: opt_trim(update.get("last_event_detail")),
        can_stop: Some(false),
    }
}

fn opt_trim(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn grok_agents(update: &Value) -> Option<Vec<WorkflowAgent>> {
    update.get("agents").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(|a| {
                let label = opt_trim(a.get("label"));
                let agent_id = opt_trim(a.get("agent_id")).or_else(|| label.clone())?;
                let label = label.unwrap_or_else(|| agent_id.clone());
                Some(WorkflowAgent {
                    agent_id,
                    label,
                    phase: opt_trim(a.get("phase")),
                    state: a
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("running")
                        .to_string(),
                    summary: opt_trim(a.get("summary"))
                        .or_else(|| opt_trim(a.get("task")))
                        .or_else(|| opt_trim(a.get("prompt")))
                        .or_else(|| opt_trim(a.get("objective")))
                        .or_else(|| opt_trim(a.get("description"))),
                    tokens_used: a.get("tokens_used").and_then(Value::as_u64),
                    duration_ms: a.get("duration_ms").and_then(Value::as_u64),
                })
            })
            .collect()
    })
}

fn agent_counts(agents: Option<&Vec<WorkflowAgent>>, active_agents: u64) -> (u32, u32) {
    let done = agents
        .map(|list| list.iter().filter(|a| a.state == "done").count())
        .unwrap_or(0) as u32;
    let running_from_list = agents
        .map(|list| {
            list.iter()
                .filter(|a| {
                    !matches!(
                        a.state.as_str(),
                        "done" | "failed" | "cancelled" | "canceled"
                    )
                })
                .count()
        })
        .unwrap_or(0) as u32;
    (done, running_from_list.max(active_agents as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn grok_params(update: Value) -> Value {
        json!({ "sessionId": "s", "update": update })
    }

    #[test]
    fn grok_first_frame_spawns_a_running_row() {
        let mut seen = HashSet::new();
        let delta = adapt_grok_workflow(
            AgentType::Grok,
            "_x.ai/session/update",
            &grok_params(json!({
                "sessionUpdate": "workflow_updated",
                "run_id": "wf_1",
                "revision": 1,
                "name": "deep-research",
                "objective": "survey sensors",
                "status": "active",
                "phases": [
                    {"title": "Plan", "detail": "Choose research questions", "state": "pending"},
                    {"title": "Research", "state": "pending"}
                ],
                "agent_budget": 128,
                "agents_used": 0,
                "agents_remaining": 128,
                "active_agents": 0,
                "elapsed_ms": 7
            })),
            &mut seen,
        )
        .expect("mapped");
        assert!(delta.spawned);
        assert_eq!(delta.run_id, "wf_1");
        assert_eq!(delta.name.as_deref(), Some("deep-research"));
        assert_eq!(delta.objective.as_deref(), Some("survey sensors"));
        assert_eq!(delta.state.as_deref(), Some("running"));
        assert_eq!(delta.can_stop, Some(false));
        let phases = delta.phases.expect("phases");
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].title, "Plan");
        assert_eq!(
            phases[0].detail.as_deref(),
            Some("Choose research questions")
        );
        assert_eq!(delta.agents_done, Some(0));
        assert_eq!(delta.agents_running, Some(0));
        assert_eq!(delta.agent_budget, Some(128));
        assert_eq!(delta.agents_remaining, Some(128));
    }

    #[test]
    fn grok_later_frame_is_progress_with_phase_and_counts() {
        let mut seen = HashSet::new();
        seen.insert("wf_1".into());
        let delta = adapt_grok_workflow(
            AgentType::Grok,
            "_x.ai/session_notification",
            &grok_params(json!({
                "sessionUpdate": "workflow_updated",
                "run_id": "wf_1",
                "status": "active",
                "current_phase": "Research",
                "phases": [
                    {"title": "Plan", "state": "done"},
                    {"title": "Research", "state": "active"},
                    {"title": "Verify", "state": "pending"},
                    {"title": "Report", "state": "pending"}
                ],
                "agents_used": 1,
                "active_agents": 4,
                "elapsed_ms": 43362,
                "last_event": "phase_entered",
                "last_event_detail": "Research",
                "agents": [{
                    "agent_id": "ag_1",
                    "label": "research-planner",
                    "phase": "Plan",
                    "state": "done",
                    "task": "Break the query into independent questions",
                    "tokens_used": 22284,
                    "duration_ms": 43281
                }]
            })),
            &mut seen,
        )
        .expect("mapped");
        assert!(!delta.spawned);
        assert_eq!(delta.current_phase.as_deref(), Some("Research"));
        assert_eq!(delta.agents_done, Some(1));
        assert_eq!(delta.agents_running, Some(4));
        assert_eq!(delta.elapsed_ms, Some(43362));
        assert_eq!(delta.last_event.as_deref(), Some("phase_entered"));
        assert_eq!(delta.last_event_detail.as_deref(), Some("Research"));
        assert_eq!(delta.agent_budget, None);
        let agents = delta.agents.expect("agents");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_id, "ag_1");
        assert_eq!(agents[0].label, "research-planner");
        assert_eq!(
            agents[0].summary.as_deref(),
            Some("Break the query into independent questions")
        );
        assert_eq!(agents[0].phase.as_deref(), Some("Plan"));
        assert_eq!(agents[0].state, "done");
        assert_eq!(agents[0].tokens_used, Some(22284));
        assert_eq!(agents[0].duration_ms, Some(43281));
    }

    #[test]
    fn grok_complete_is_terminal() {
        let mut seen = HashSet::new();
        seen.insert("wf_1".into());
        let delta = adapt_grok_workflow(
            AgentType::Grok,
            "_x.ai/session/update",
            &grok_params(json!({
                "sessionUpdate": "workflow_updated",
                "run_id": "wf_1",
                "name": "deep-research",
                "status": "complete",
                "current_phase": "Report",
                "last_event": "workflow_completed"
            })),
            &mut seen,
        )
        .expect("mapped");
        assert_eq!(delta.state.as_deref(), Some("completed"));
        assert_eq!(delta.last_event.as_deref(), Some("workflow_completed"));
    }

    #[test]
    fn grok_adapter_ignores_other_agents_and_variants() {
        let mut seen = HashSet::new();
        let update = json!({
            "sessionUpdate": "workflow_updated",
            "run_id": "wf_1",
            "status": "active"
        });
        assert!(adapt_grok_workflow(
            AgentType::ClaudeCode,
            "_x.ai/session/update",
            &grok_params(update.clone()),
            &mut seen
        )
        .is_none());
        assert!(adapt_grok_workflow(
            AgentType::Grok,
            "session/update",
            &grok_params(update),
            &mut seen
        )
        .is_none());
        assert!(seen.is_empty());
    }

    fn air(task_type: &str, spawned: bool) -> AsyncTaskDelta {
        AsyncTaskDelta {
            task_id: "t-wf".into(),
            spawned,
            name: Some("explore".into()),
            task_type: Some(task_type.into()),
            description: Some("scan the repo".into()),
            show_in_transcript: Some(false),
            can_stop: Some(true),
            state: Some("running".into()),
            summary: Some("searching".into()),
            last_tool_name: Some("Grep".into()),
            usage: None,
            output_file_path: None,
            tool_call_id: Some("call-1".into()),
        }
    }

    #[test]
    fn air_workflow_type_is_adapted() {
        let delta = adapt_air_workflow(&air("workflow", true), false).expect("mapped");
        assert!(delta.spawned);
        assert_eq!(delta.run_id, "t-wf");
        assert_eq!(delta.name.as_deref(), Some("explore"));
        assert_eq!(delta.objective.as_deref(), Some("scan the repo"));
        assert_eq!(delta.current_phase.as_deref(), Some("Grep"));
        assert_eq!(delta.last_event.as_deref(), Some("searching"));
        assert_eq!(delta.can_stop, Some(true));
    }

    #[test]
    fn air_shell_is_not_a_workflow() {
        assert!(adapt_air_workflow(&air("shell", true), false).is_none());
    }

    #[test]
    fn air_progress_without_type_still_routes_when_known() {
        let mut delta = air("workflow", false);
        delta.task_type = None;
        assert!(adapt_air_workflow(&delta, false).is_none());
        let mapped = adapt_air_workflow(&delta, true).expect("known");
        assert!(!mapped.spawned);
        assert_eq!(mapped.run_id, "t-wf");
    }
}

//! Run-settled hook: collect turn-stop context (file changes, session store).
//!
//! Channel IM for turn complete / agent error is owned by the Events-tab
//! subscriber (`chat_channel::message_formatter::format_session_card`). This
//! module does not send a second card.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::RwLock;

use crate::acp::manager::ConnectionManager;
use crate::acp::session_state::{SessionState, ToolCallState, ToolKind};
use crate::chat_channel::terminal_message::TerminalKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeOp {
    Edit,
    Delete,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub operation: FileChangeOp,
}

pub fn collect_file_changes_from_tools(
    tool_calls: &BTreeMap<String, ToolCallState>,
) -> Vec<FileChange> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for call in tool_calls.values() {
        let Some(operation) = op_from_kind(&call.kind) else {
            continue;
        };
        for path in paths_from_tool_call(call) {
            if seen.insert(path.clone()) {
                out.push(FileChange { path, operation });
            }
        }
    }
    out
}

fn op_from_kind(kind: &ToolKind) -> Option<FileChangeOp> {
    match kind {
        ToolKind::Edit => Some(FileChangeOp::Edit),
        ToolKind::Delete => Some(FileChangeOp::Delete),
        ToolKind::Move => Some(FileChangeOp::Move),
        _ => None,
    }
}

fn paths_from_tool_call(call: &ToolCallState) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    if let Some(loc) = call.locations.as_ref() {
        push_paths_from_value(loc, &mut paths, &mut seen);
    }
    if let Some(input) = call.input.as_ref() {
        push_paths_from_value(input, &mut paths, &mut seen);
    }
    paths
}

fn push_paths_from_value(
    value: &serde_json::Value,
    paths: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                push_paths_from_value(item, paths, seen);
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["path", "file_path", "filePath", "target_file", "targetFile"] {
                if let Some(serde_json::Value::String(s)) = map.get(key) {
                    push_path(s, paths, seen);
                }
            }
        }
        _ => {}
    }
}

fn push_path(raw: &str, paths: &mut Vec<String>, seen: &mut HashSet<String>) {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return;
    }
    if seen.insert(normalized.clone()) {
        paths.push(normalized);
    }
}

/// Best-effort native transcript location. Missing is normal: Codex rollout
/// lookup is a full-tree walk (too heavy on the turn-end path), some agents
/// have no store, and the file may not have been flushed yet.
pub fn resolve_session_store(
    agent_type: crate::models::AgentType,
    external_id: Option<&str>,
) -> Option<PathBuf> {
    let session_id = external_id.filter(|id| !id.is_empty())?;
    match agent_type {
        crate::models::AgentType::ClaudeCode => {
            crate::parsers::claude::find_session_file(session_id)
        }
        crate::models::AgentType::Grok => {
            crate::parsers::grok::GrokParser::new().find_session_dir(session_id)
        }
        _ => None,
    }
}

/// Channel IM is owned by the Events-tab subscriber. Wiki persist is queued
/// off the ACP hot path from a frozen snapshot — never re-read last_*.
pub async fn dispatch_run_settled(
    db_conn: &DatabaseConnection,
    _manager: &ConnectionManager,
    _state_arc: &Arc<RwLock<SessionState>>,
    kind: TerminalKind,
    _error: Option<&str>,
    snapshot: Option<crate::wiki::snapshot::WikiTurnSnapshot>,
) {
    if kind != TerminalKind::Completed {
        return;
    }
    let Some(snapshot) = snapshot else {
        return;
    };
    crate::wiki::source::enqueue_persist(db_conn.clone(), snapshot);
}

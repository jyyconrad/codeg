//! Run-settled hook: one context per turn stop, handlers consume what they need.
//!
//! Built at the existing lifecycle/cancel/error sites (the agent has stopped:
//! normal `end_turn`, user cancel, or error). Channel fan-out is one handler
//! and only sends `last_message` text; other handlers can use session identity
//! and file changes without going through IM.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio::sync::RwLock;

use crate::acp::manager::ConnectionManager;
use crate::acp::session_state::{SessionState, ToolCallState, ToolKind};
use crate::chat_channel::i18n::{self, Lang};
use crate::chat_channel::terminal_message::{publish_run_terminal_message, TerminalKind};
use crate::chat_channel::types::{MessageLevel, RichMessage};
use crate::db::entities::conversation::ConversationKind;
use crate::db::service::{app_metadata_service, conversation_service, folder_service};
use crate::models::AgentType;

const MESSAGE_LANGUAGE_KEY: &str = "chat_message_language";

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

/// One turn-stop payload. Channel handler uses `last_message` only; other
/// handlers may read session identity and `file_changes`.
#[derive(Debug, Clone)]
pub struct RunSettled {
    pub conversation_id: i32,
    pub connection_id: String,
    pub folder_id: Option<i32>,
    pub agent_type: AgentType,
    pub conversation_kind: ConversationKind,
    pub kind: TerminalKind,
    pub last_message: Option<String>,
    pub conversation_title: Option<String>,
    pub folder_name: Option<String>,
    pub external_id: Option<String>,
    pub session_store: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub file_changes: Vec<FileChange>,
    pub error: Option<String>,
}

const MAX_BODY_CHARS: usize = 2000;
const MAX_FILE_PATHS: usize = 6;

fn nonempty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut iter = s.chars();
    let head: String = iter.by_ref().take(max).collect();
    if iter.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn format_files_field(paths: &[String], lang: Lang) -> String {
    if paths.len() <= MAX_FILE_PATHS {
        return paths.join(", ");
    }
    let shown = paths[..MAX_FILE_PATHS].join(", ");
    format!(
        "{shown}{}",
        i18n::files_more_suffix(lang, paths.len() - MAX_FILE_PATHS)
    )
}

/// Progress card for folder-bound IM: title (status + agent), concluding
/// text, then session / folder / files / problem fields.
pub fn format_run_settled_card(event: &RunSettled, lang: Lang) -> Option<RichMessage> {
    let last = nonempty_trimmed(event.last_message.as_deref()).map(str::to_string);
    let err = nonempty_trimmed(event.error.as_deref()).map(str::to_string);
    if event.kind == TerminalKind::Completed && last.is_none() && event.file_changes.is_empty() {
        return None;
    }

    let agent = event.agent_type.to_string();
    let (title, level) = match event.kind {
        TerminalKind::Completed => (
            i18n::run_settled_complete_title(lang, &agent),
            MessageLevel::Info,
        ),
        TerminalKind::Stopped => (
            i18n::run_settled_stopped_title(lang, &agent),
            MessageLevel::Warning,
        ),
        TerminalKind::Error => (
            i18n::run_settled_error_title(lang, &agent),
            MessageLevel::Error,
        ),
    };

    let body = match event.kind {
        TerminalKind::Completed => last.clone().unwrap_or_default(),
        TerminalKind::Stopped => last
            .clone()
            .unwrap_or_else(|| i18n::user_stopped_message(lang).to_string()),
        TerminalKind::Error => last.clone().unwrap_or_else(|| {
            err.clone()
                .unwrap_or_else(|| i18n::agent_error_fallback(lang).to_string())
        }),
    };
    let body = truncate_chars(&body, MAX_BODY_CHARS);

    let mut msg = RichMessage {
        title: Some(title),
        body,
        fields: Vec::new(),
        level,
    };
    if let Some(session) = nonempty_trimmed(event.conversation_title.as_deref()) {
        msg = msg.with_field(i18n::session_field_label(lang), session);
    }
    if let Some(folder) = nonempty_trimmed(event.folder_name.as_deref()) {
        msg = msg.with_field(i18n::folder_field_label(lang), folder);
    }
    if !event.file_changes.is_empty() {
        let paths: Vec<String> = event.file_changes.iter().map(|f| f.path.clone()).collect();
        msg = msg.with_field(
            i18n::files_field_label(lang),
            format_files_field(&paths, lang),
        );
    }
    if event.kind == TerminalKind::Error {
        if let Some(problem) = err {
            if last.is_some() {
                msg = msg.with_field(i18n::problem_field_label(lang), problem);
            }
        }
    }
    Some(msg)
}

/// Fan-out at most one run-settled dispatch per connection run. Replaces the
/// old IM-only `maybe_publish_run_terminal` body: one-shot, then handlers.
pub async fn dispatch_run_settled(
    db_conn: &DatabaseConnection,
    manager: &ConnectionManager,
    state_arc: &Arc<RwLock<SessionState>>,
    kind: TerminalKind,
    error: Option<&str>,
) {
    let snapshot = {
        let mut snap = state_arc.write().await;
        if snap.terminal_message_published {
            return;
        }
        let Some(conversation_id) = snap.conversation_id else {
            return;
        };
        let last_message = snap.concluding_assistant_text();
        let file_changes = snap.concluding_file_changes();
        // Empty successful turn with no files: do not consume the one-shot; a
        // later Error/Stopped can still publish.
        if kind == TerminalKind::Completed && last_message.is_none() && file_changes.is_empty() {
            return;
        }
        snap.terminal_message_published = true;
        DispatchSnapshot {
            conversation_id,
            connection_id: snap.connection_id.clone(),
            folder_id: snap.folder_id,
            agent_type: snap.agent_type,
            last_message,
            external_id: snap.external_id.clone(),
            working_dir: snap.working_dir.clone(),
            file_changes,
        }
    };

    let row = conversation_service::get_by_id(db_conn, snapshot.conversation_id)
        .await
        .ok();
    let conversation_kind = row
        .as_ref()
        .map(|r| r.kind.clone())
        .unwrap_or(ConversationKind::Regular);
    let conversation_title = row.as_ref().and_then(|r| r.title.clone());
    let folder_id = row.as_ref().map(|r| r.folder_id).or(snapshot.folder_id);
    let folder_name = match folder_id {
        Some(id) => folder_service::get_folder_by_id(db_conn, id)
            .await
            .ok()
            .flatten()
            .map(|f| f.alias.unwrap_or(f.name)),
        None => None,
    };

    let event = RunSettled {
        conversation_id: snapshot.conversation_id,
        connection_id: snapshot.connection_id,
        folder_id,
        agent_type: snapshot.agent_type,
        conversation_kind,
        kind,
        last_message: snapshot.last_message,
        conversation_title,
        folder_name,
        session_store: resolve_session_store(snapshot.agent_type, snapshot.external_id.as_deref()),
        external_id: snapshot.external_id,
        working_dir: snapshot.working_dir,
        file_changes: snapshot.file_changes,
        error: error.map(str::to_string),
    };

    channel_handler(db_conn, manager, &event).await;
}

struct DispatchSnapshot {
    conversation_id: i32,
    connection_id: String,
    folder_id: Option<i32>,
    agent_type: AgentType,
    last_message: Option<String>,
    external_id: Option<String>,
    working_dir: Option<PathBuf>,
    file_changes: Vec<FileChange>,
}

/// Folder-bound IM: last_message text only. Delegate children are silent.
/// Gated by the same Events-tab filter as the global chat-channel feed
/// (`turn_complete` / `error`).
async fn channel_handler(
    db_conn: &DatabaseConnection,
    manager: &ConnectionManager,
    event: &RunSettled,
) {
    if event.conversation_kind == ConversationKind::Delegate {
        return;
    }
    let event_type = crate::chat_channel::event_filter::event_type_for_terminal(event.kind);
    if !crate::chat_channel::event_filter::event_enabled(db_conn, event_type).await {
        return;
    }
    let Some(ccm) = manager.chat_channel() else {
        return;
    };
    let extra_targets = {
        let bridge = ccm.session_bridge();
        let guard = bridge.lock().await;
        guard
            .all_sessions()
            .filter(|session| session.conversation_id == event.conversation_id)
            .map(|session| session.target.clone())
            .collect::<Vec<_>>()
    };
    let lang = chat_message_lang(db_conn).await;
    let Some(message) = format_run_settled_card(event, lang) else {
        return;
    };
    let _ = publish_run_terminal_message(
        db_conn,
        &ccm,
        &extra_targets,
        event.conversation_id,
        &message,
    )
    .await;
}

async fn chat_message_lang(db: &DatabaseConnection) -> Lang {
    app_metadata_service::get_value(db, MESSAGE_LANGUAGE_KEY)
        .await
        .ok()
        .flatten()
        .map(|v| Lang::from_str_lossy(&v))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn settled(
        kind: TerminalKind,
        last: Option<&str>,
        error: Option<&str>,
        files: &[&str],
    ) -> RunSettled {
        RunSettled {
            conversation_id: 1,
            connection_id: "c1".into(),
            folder_id: Some(1),
            agent_type: AgentType::Codex,
            conversation_kind: ConversationKind::Regular,
            kind,
            last_message: last.map(str::to_string),
            conversation_title: Some("Fix auth".into()),
            folder_name: Some("switchgear".into()),
            external_id: None,
            session_store: None,
            working_dir: Some(PathBuf::from("/tmp")),
            file_changes: files
                .iter()
                .map(|p| FileChange {
                    path: (*p).into(),
                    operation: FileChangeOp::Edit,
                })
                .collect(),
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn completed_card_carries_progress_fields() {
        let msg = format_run_settled_card(
            &settled(
                TerminalKind::Completed,
                Some("110 tests passed"),
                None,
                &["src/a.rs", "src/b.rs"],
            ),
            Lang::En,
        )
        .unwrap();
        assert_eq!(msg.title.as_deref(), Some("Turn complete · Codex CLI"));
        assert!(msg.body.contains("110 tests passed"));
        assert!(msg
            .fields
            .iter()
            .any(|(k, v)| k == "Session" && v == "Fix auth"));
        assert!(msg
            .fields
            .iter()
            .any(|(k, v)| k == "Folder" && v == "switchgear"));
        assert!(msg.fields.iter().any(|(k, v)| k == "Files changed"
            && v.contains("src/a.rs")
            && v.contains("src/b.rs")));
    }

    #[test]
    fn error_card_keeps_answer_and_surfaces_the_problem() {
        let msg = format_run_settled_card(
            &settled(TerminalKind::Error, Some("partial"), Some("refusal"), &[]),
            Lang::En,
        )
        .unwrap();
        assert_eq!(msg.level, MessageLevel::Error);
        assert!(msg.body.contains("partial"));
        assert!(msg
            .fields
            .iter()
            .any(|(k, v)| k == "Problem" && v == "refusal"));
    }

    #[test]
    fn completed_without_text_or_files_is_silent() {
        assert!(format_run_settled_card(
            &settled(TerminalKind::Completed, None, None, &[]),
            Lang::En
        )
        .is_none());
    }
}

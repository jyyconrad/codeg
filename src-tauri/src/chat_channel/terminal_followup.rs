//! Deliver a channel message into a terminal-mode conversation.
//!
//! A live PTY for that native session receives the text as a user submit.
//! Otherwise the native CLI is resumed (or started) with the prompt, on a
//! stable terminal id the desktop can attach to later.

use std::path::Path;
use std::sync::Arc;

use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};

use crate::db::entities::conversation;
use crate::db::AppDatabase;
use crate::models::agent::AgentType;
use crate::models::conversation::DbConversationSummary;
use crate::terminal::agent_hooks::AgentHookRegistry;
use crate::terminal::agent_launch::spawn_agent;
use crate::terminal::manager::TerminalManager;
use crate::web::event_bridge::EventEmitter;

pub fn channel_terminal_id(conversation_id: i32) -> String {
    format!("codeg-channel-conv-{conversation_id}")
}

/// Terminal the desktop should attach to instead of starting a second CLI.
pub fn live_terminal_id(
    manager: &TerminalManager,
    hooks: &AgentHookRegistry,
    native_session_id: Option<&str>,
    conversation_id: Option<i32>,
) -> Option<String> {
    let session_id = native_session_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(session_id) = session_id {
        if let Some(terminal_id) = hooks.active_terminal_for_session(session_id) {
            if manager.snapshot(&terminal_id).alive {
                return Some(terminal_id);
            }
        }
    }
    let conversation_id = conversation_id.filter(|id| *id > 0)?;
    let terminal_id = channel_terminal_id(conversation_id);
    manager.snapshot(&terminal_id).alive.then_some(terminal_id)
}

/// Text a TUI treats as one submitted user message.
pub fn pty_submit(text: &str) -> Vec<u8> {
    let text = text.trim();
    if text.contains('\n') {
        format!("\u{1b}[200~{text}\u{1b}[201~\r").into_bytes()
    } else {
        format!("{text}\r").into_bytes()
    }
}

pub struct TerminalHost {
    pub manager: TerminalManager,
    pub hooks: Arc<AgentHookRegistry>,
}

pub async fn deliver_terminal_user_message(
    host: &TerminalHost,
    db: &DatabaseConnection,
    conv: &DbConversationSummary,
    working_dir: &str,
    prompt: &str,
    data_dir: &Path,
    emitter: EventEmitter,
) -> Result<(), String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("empty prompt".to_string());
    }
    if let Some(terminal_id) = live_terminal(host, conv) {
        host.manager
            .write(&terminal_id, &pty_submit(prompt))
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let terminal_id = channel_terminal_id(conv.id);
    host.hooks
        .reserve_conversation(&terminal_id, conv.id, conv.folder_id);
    let runtime_env = crate::commands::acp::build_terminal_runtime_env(
        &AppDatabase { conn: db.clone() },
        conv.agent_type,
        conv.external_id.as_deref(),
        data_dir,
    )
    .await
    .map_err(|error| error.to_string())?;
    let spawned = spawn_agent(
        &host.manager,
        host.hooks.clone(),
        emitter,
        conv.agent_type,
        working_dir.to_string(),
        prompt.to_string(),
        conv.external_id.clone(),
        terminal_id.clone(),
        None,
        "chat-channel".to_string(),
        data_dir,
        runtime_env,
        true,
    )
    .await;
    if let Err(error) = spawned {
        host.hooks.clear_reservation(&terminal_id);
        return Err(error.to_string());
    }
    Ok(())
}

fn live_terminal(host: &TerminalHost, conv: &DbConversationSummary) -> Option<String> {
    let session_id = conv
        .external_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if let Some(session_id) = session_id {
        if let Some(terminal_id) = host.hooks.active_terminal_for_session(session_id) {
            if host.manager.snapshot(&terminal_id).alive {
                return Some(terminal_id);
            }
        }
    }
    let stable = channel_terminal_id(conv.id);
    host.manager.snapshot(&stable).alive.then_some(stable)
}

pub async fn stamp_terminal_launch(
    db: &DatabaseConnection,
    conversation_id: i32,
) -> Result<(), String> {
    let row = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("conversation {conversation_id} not found"))?;
    let mut active = row.into_active_model();
    active.launch_mode = Set("terminal".to_string());
    active.update(db).await.map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn agent_uses_terminal(db: &DatabaseConnection, agent_type: AgentType) -> bool {
    match crate::db::service::agent_setting_service::get_by_agent_type(db, agent_type).await {
        Ok(Some(setting)) => {
            crate::acp::types::AgentLaunchMode::from_storage(&setting.launch_mode)
                == crate::acp::types::AgentLaunchMode::Terminal
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::pty_submit;

    #[test]
    fn single_line_submits_with_enter() {
        assert_eq!(pty_submit("  hi  "), b"hi\r");
    }

    #[test]
    fn multiline_uses_bracketed_paste() {
        let bytes = pty_submit("one\ntwo");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("\u{1b}[200~"));
        assert!(text.contains("one\ntwo"));
        assert!(text.ends_with("\u{1b}[201~\r"));
    }
}

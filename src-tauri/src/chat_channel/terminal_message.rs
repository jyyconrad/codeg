use std::collections::HashSet;

use sea_orm::{DatabaseConnection, EntityTrait};

use super::i18n::{self, Lang};
use super::manager::ChatChannelManager;
use super::session_bridge::SessionBridge;
use super::types::{ChannelMessageTarget, RichMessage};
use crate::db::entities::{chat_channel_thread_binding, conversation};
use crate::db::service::{
    chat_channel_message_log_service, folder_chat_channel_service, thread_binding_service,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Completed,
    Stopped,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPublish {
    pub conversation_id: i32,
    pub kind: TerminalKind,
    pub body: String,
}

pub fn terminal_body(
    kind: TerminalKind,
    last_assistant: Option<&str>,
    error: Option<&str>,
    lang: Lang,
) -> Option<String> {
    let assistant = nonempty_trimmed(last_assistant);
    match kind {
        TerminalKind::Completed => assistant.map(str::to_string),
        TerminalKind::Stopped => {
            let notice = i18n::user_stopped_message(lang);
            Some(match assistant {
                Some(text) => format!("{notice}\n{text}"),
                None => notice.to_string(),
            })
        }
        TerminalKind::Error => {
            let err = nonempty_trimmed(error);
            match (assistant, err) {
                (Some(text), Some(err)) => Some(format!("{text}\n{err}")),
                (Some(text), None) => Some(text.to_string()),
                (None, Some(err)) => Some(err.to_string()),
                (None, None) => Some(i18n::agent_error_fallback(lang).to_string()),
            }
        }
    }
}

pub fn target_dedupe_key(target: &ChannelMessageTarget) -> String {
    format!(
        "{}|{}|{}",
        target.channel_id,
        target.chat_id.as_deref().unwrap_or(""),
        target.thread_key.as_deref().unwrap_or("")
    )
}

/// Fan-out the final run message to every bound folder channel (plus the
/// live Bridge target). Never returns `Err`; send failures are logged.
pub async fn publish_run_terminal_message(
    db: &DatabaseConnection,
    manager: &ChatChannelManager,
    bridge: Option<&SessionBridge>,
    conversation_id: i32,
    kind: TerminalKind,
    body: &str,
) -> usize {
    let targets = collect_terminal_targets(db, bridge, conversation_id).await;
    if targets.is_empty() {
        return 0;
    }

    let message = match kind {
        TerminalKind::Error => RichMessage::error(body),
        TerminalKind::Completed | TerminalKind::Stopped => RichMessage::info(body),
    };

    let mut sent = 0usize;
    for target in targets {
        match manager.send_to_target(&target, &message).await {
            Ok(_) => sent += 1,
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    channel_id = target.channel_id,
                    error = %e,
                    "[ChatChannel] failed to publish terminal message"
                );
                let _ = chat_channel_message_log_service::create_log(
                    db,
                    target.channel_id,
                    "outbound",
                    "terminal_message",
                    body,
                    "failed",
                    Some(e.to_string()),
                )
                .await;
            }
        }
    }
    sent
}

fn nonempty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn target_from_binding(binding: &chat_channel_thread_binding::Model) -> ChannelMessageTarget {
    ChannelMessageTarget {
        channel_id: binding.channel_id,
        chat_id: Some(binding.chat_id.clone()),
        thread_key: Some(binding.thread_key.clone()),
        thread_kind: Some(binding.thread_kind.clone()),
        provider_payload: binding
            .provider_payload_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok()),
    }
}

fn push_unique(
    targets: &mut Vec<ChannelMessageTarget>,
    seen: &mut HashSet<String>,
    target: ChannelMessageTarget,
) {
    if seen.insert(target_dedupe_key(&target)) {
        targets.push(target);
    }
}

async fn collect_terminal_targets(
    db: &DatabaseConnection,
    bridge: Option<&SessionBridge>,
    conversation_id: i32,
) -> Vec<ChannelMessageTarget> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();

    match conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await
    {
        Ok(Some(conv)) => match folder_chat_channel_service::list_channel_ids(db, conv.folder_id)
            .await
        {
            Ok(channel_ids) => {
                let bindings =
                    match thread_binding_service::list_by_conversation(db, conversation_id).await {
                        Ok(bindings) => bindings,
                        Err(e) => {
                            tracing::warn!(
                                conversation_id,
                                error = %e,
                                "[ChatChannel] failed to list thread bindings for terminal fan-out"
                            );
                            Vec::new()
                        }
                    };
                for channel_id in channel_ids {
                    if let Some(binding) = bindings.iter().find(|b| b.channel_id == channel_id) {
                        push_unique(&mut targets, &mut seen, target_from_binding(binding));
                    } else {
                        push_unique(
                            &mut targets,
                            &mut seen,
                            ChannelMessageTarget::channel(channel_id),
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    conversation_id,
                    folder_id = conv.folder_id,
                    error = %e,
                    "[ChatChannel] failed to list folder channels for terminal fan-out"
                );
            }
        },
        Ok(None) => {
            tracing::warn!(
                conversation_id,
                "[ChatChannel] terminal fan-out skipped folder channels: conversation not found"
            );
        }
        Err(e) => {
            tracing::warn!(
                conversation_id,
                error = %e,
                "[ChatChannel] failed to load conversation for terminal fan-out"
            );
        }
    }

    if let Some(bridge) = bridge {
        for session in bridge.all_sessions() {
            if session.conversation_id == conversation_id {
                push_unique(&mut targets, &mut seen, session.target.clone());
            }
        }
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_skips_blank() {
        assert_eq!(
            terminal_body(TerminalKind::Completed, Some("  "), None, Lang::En),
            None
        );
        assert_eq!(
            terminal_body(TerminalKind::Completed, Some("done."), None, Lang::En).as_deref(),
            Some("done.")
        );
    }

    #[test]
    fn stopped_always_sends() {
        let s = terminal_body(TerminalKind::Stopped, None, None, Lang::ZhCn).unwrap();
        assert!(s.contains("停止") || s.contains("stop") || s.contains("已停止"));
    }

    #[test]
    fn error_uses_error_text() {
        let s = terminal_body(TerminalKind::Error, None, Some("boom"), Lang::En).unwrap();
        assert!(s.contains("boom"));
    }

    #[test]
    fn dedupe_key_joins_channel_chat_thread() {
        let t = ChannelMessageTarget {
            channel_id: 1,
            chat_id: Some("c".into()),
            thread_key: Some("9".into()),
            thread_kind: None,
            provider_payload: None,
        };
        assert_eq!(target_dedupe_key(&t), "1|c|9");
    }
}

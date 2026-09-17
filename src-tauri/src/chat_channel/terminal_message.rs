use std::collections::HashSet;

use sea_orm::{DatabaseConnection, EntityTrait};

use super::folder_inbound::remember_sent_message;
use super::i18n::{self, Lang};
use super::manager::ChatChannelManager;
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

/// Fan-out the final run message to every bound folder channel (plus any
/// already-snapshotted Bridge targets). Never returns `Err`; send failures
/// are logged. Callers must clone Bridge targets under the session mutex
/// and drop that guard before calling — this function does DB + IM I/O.
pub async fn publish_run_terminal_message(
    db: &DatabaseConnection,
    manager: &ChatChannelManager,
    extra_targets: &[ChannelMessageTarget],
    conversation_id: i32,
    message: &RichMessage,
) -> usize {
    let targets = collect_terminal_targets(db, extra_targets, conversation_id).await;
    if targets.is_empty() {
        return 0;
    }

    let mut sent = 0usize;
    for target in targets {
        match manager.send_to_target(&target, message).await {
            Ok(id) => {
                remember_sent_message(db, &target, &id.0, conversation_id).await;
                sent += 1;
            }
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
                    &message.to_plain_text(),
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

fn target_from_folder_binding(channel_id: i32, chat_id: Option<&str>) -> ChannelMessageTarget {
    match nonempty_trimmed(chat_id) {
        Some(chat_id) => ChannelMessageTarget::with_chat_id(channel_id, chat_id),
        None => ChannelMessageTarget::channel(channel_id),
    }
}

fn is_bare_channel_target(target: &ChannelMessageTarget) -> bool {
    nonempty_trimmed(target.chat_id.as_deref()).is_none()
        && nonempty_trimmed(target.thread_key.as_deref()).is_none()
}

fn push_unique(
    targets: &mut Vec<ChannelMessageTarget>,
    seen: &mut HashSet<String>,
    target: ChannelMessageTarget,
) {
    let key = target_dedupe_key(&target);
    if seen.contains(&key) {
        return;
    }

    if is_bare_channel_target(&target) {
        // A live Bridge session for the same default chat is more specific
        // (`"1|<chat_id>|"` vs `"1||"`). Keep the specific target only.
        if targets.iter().any(|existing| {
            existing.channel_id == target.channel_id && !is_bare_channel_target(existing)
        }) {
            return;
        }
    } else if let Some(idx) = targets.iter().position(|existing| {
        existing.channel_id == target.channel_id && is_bare_channel_target(existing)
    }) {
        seen.remove(&target_dedupe_key(&targets[idx]));
        targets.remove(idx);
    }

    seen.insert(key);
    targets.push(target);
}

async fn collect_terminal_targets(
    db: &DatabaseConnection,
    extra_targets: &[ChannelMessageTarget],
    conversation_id: i32,
) -> Vec<ChannelMessageTarget> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();

    match conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await
    {
        Ok(Some(conv)) => {
            match folder_chat_channel_service::list_bindings(db, conv.folder_id).await {
                Ok(folder_bindings) => {
                    let bindings = match thread_binding_service::list_by_conversation(
                        db,
                        conversation_id,
                    )
                    .await
                    {
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
                    for folder_binding in folder_bindings {
                        if let Some(binding) = bindings
                            .iter()
                            .find(|b| b.channel_id == folder_binding.channel_id)
                        {
                            push_unique(&mut targets, &mut seen, target_from_binding(binding));
                        } else {
                            push_unique(
                                &mut targets,
                                &mut seen,
                                target_from_folder_binding(
                                    folder_binding.channel_id,
                                    folder_binding.chat_id.as_deref(),
                                ),
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
            }
        }
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

    for target in extra_targets {
        push_unique(&mut targets, &mut seen, target.clone());
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
    fn error_includes_stop_reason_with_assistant() {
        let s = terminal_body(
            TerminalKind::Error,
            Some("partial"),
            Some("refusal"),
            Lang::En,
        )
        .unwrap();
        assert!(s.contains("partial"));
        assert!(s.contains("refusal"));
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

    #[test]
    fn push_unique_replaces_bare_channel_with_specific_target() {
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        push_unique(&mut targets, &mut seen, ChannelMessageTarget::channel(1));
        push_unique(
            &mut targets,
            &mut seen,
            ChannelMessageTarget::telegram_general(1, "-100123"),
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].chat_id.as_deref(), Some("-100123"));
        assert!(!seen.contains("1||"));
        assert!(seen.contains("1|-100123|"));
    }

    #[test]
    fn push_unique_skips_bare_channel_when_specific_exists() {
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        push_unique(
            &mut targets,
            &mut seen,
            ChannelMessageTarget::telegram_general(1, "-100123"),
        );
        push_unique(&mut targets, &mut seen, ChannelMessageTarget::channel(1));
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].chat_id.as_deref(), Some("-100123"));
    }

    #[test]
    fn folder_binding_uses_chat_id_override_or_channel_default() {
        let override_target = target_from_folder_binding(7, Some("-100999"));
        assert_eq!(override_target.channel_id, 7);
        assert_eq!(override_target.chat_id.as_deref(), Some("-100999"));

        let default_target = target_from_folder_binding(7, Some("  "));
        assert_eq!(default_target, ChannelMessageTarget::channel(7));
        assert_eq!(
            target_from_folder_binding(7, None),
            ChannelMessageTarget::channel(7)
        );
    }
}

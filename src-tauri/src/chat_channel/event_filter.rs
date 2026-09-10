//! Shared Events-tab filter (Settings → 消息渠道 → 事件).
//!
//! Same contract as `event_subscriber`: a null/absent filter is the default
//! set (everything except opt-in content events). An explicit list is an
//! allow-list. Used by both the global event feed and the run-settled
//! last-message fan-out so toggling 「对话完成」/「代理错误」 gates both.

use sea_orm::DatabaseConnection;

use crate::db::service::app_metadata_service;

pub const EVENT_FILTER_KEY: &str = "chat_event_filter";

/// Events that export user-authored content. Off unless the stored filter
/// explicitly includes them. Mirrors `event_subscriber::DEFAULT_OFF_EVENTS`.
pub const DEFAULT_OFF_EVENTS: &[&str] = &["user_prompt_sent"];

pub fn allows(global_filter: Option<&[String]>, event_type: &str) -> bool {
    match global_filter {
        Some(filter) => filter.iter().any(|id| id == event_type),
        None => !DEFAULT_OFF_EVENTS.contains(&event_type),
    }
}

/// Load the stored filter and decide. Fail closed on a DB or parse error so a
/// corrupt Events config cannot leak last-message text.
pub async fn event_enabled(db: &DatabaseConnection, event_type: &str) -> bool {
    match app_metadata_service::get_value(db, EVENT_FILTER_KEY).await {
        Ok(None) => allows(None, event_type),
        Ok(Some(json)) => match serde_json::from_str::<Option<Vec<String>>>(&json) {
            Ok(parsed) => allows(parsed.as_deref(), event_type),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Map a run-settled kind onto the Events-tab id.
pub fn event_type_for_terminal(
    kind: crate::chat_channel::terminal_message::TerminalKind,
) -> &'static str {
    match kind {
        crate::chat_channel::terminal_message::TerminalKind::Completed
        | crate::chat_channel::terminal_message::TerminalKind::Stopped => "turn_complete",
        crate::chat_channel::terminal_message::TerminalKind::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_channel::terminal_message::TerminalKind;

    #[test]
    fn default_filter_enables_turn_complete_and_error() {
        assert!(allows(None, "turn_complete"));
        assert!(allows(None, "error"));
        assert!(!allows(None, "user_prompt_sent"));
    }

    #[test]
    fn explicit_list_is_an_allow_list() {
        let filter = vec!["error".to_string()];
        assert!(!allows(Some(&filter), "turn_complete"));
        assert!(allows(Some(&filter), "error"));
    }

    #[test]
    fn terminal_kind_maps_onto_event_ids() {
        assert_eq!(
            event_type_for_terminal(TerminalKind::Completed),
            "turn_complete"
        );
        assert_eq!(
            event_type_for_terminal(TerminalKind::Stopped),
            "turn_complete"
        );
        assert_eq!(event_type_for_terminal(TerminalKind::Error), "error");
    }
}

//! Host-side capture filter. Runs before persist; no model.

use crate::db::entities::conversation::ConversationKind;
use crate::wiki::settings::WikiSettings;
use crate::wiki::snapshot::WikiTurnSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterSkipReason {
    Disabled,
    AcpDisabled,
    ExcludedAgent,
    ExcludedFolder,
    Delegate,
    Loop,
    NoConversation,
    ConversationUnreadable,
    EmptyContent,
}

impl FilterSkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "wiki_disabled",
            Self::AcpDisabled => "acp_capture_disabled",
            Self::ExcludedAgent => "excluded_agent_type",
            Self::ExcludedFolder => "excluded_folder",
            Self::Delegate => "conversation_kind_delegate",
            Self::Loop => "conversation_kind_loop",
            Self::NoConversation => "no_conversation_id",
            Self::ConversationUnreadable => "conversation_unreadable",
            Self::EmptyContent => "empty_assistant_and_tools",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterContext {
    pub settings: WikiSettings,
    pub conversation_kind: Option<ConversationKind>,
    pub folder_id: Option<i32>,
    pub root_folder_id: Option<i32>,
}

/// Skip and record a reason when the turn must not become a source.
pub fn evaluate(snap: &WikiTurnSnapshot, ctx: &FilterContext) -> Option<FilterSkipReason> {
    if !ctx.settings.enabled {
        return Some(FilterSkipReason::Disabled);
    }
    if !ctx.settings.capture.acp_enabled {
        return Some(FilterSkipReason::AcpDisabled);
    }
    if ctx
        .settings
        .capture
        .exclude_agent_types
        .iter()
        .any(|t| t == &snap.agent_type)
    {
        return Some(FilterSkipReason::ExcludedAgent);
    }
    let excluded = &ctx.settings.capture.exclude_folder_ids;
    if let Some(fid) = ctx.folder_id.or(snap.folder_id) {
        if excluded.contains(&fid) {
            return Some(FilterSkipReason::ExcludedFolder);
        }
    }
    if let Some(rid) = ctx.root_folder_id {
        if excluded.contains(&rid) {
            return Some(FilterSkipReason::ExcludedFolder);
        }
    }
    if snap.conversation_id.is_none() {
        return Some(FilterSkipReason::NoConversation);
    }
    match ctx.conversation_kind {
        Some(ConversationKind::Delegate) => return Some(FilterSkipReason::Delegate),
        Some(ConversationKind::Loop) => return Some(FilterSkipReason::Loop),
        None => return Some(FilterSkipReason::ConversationUnreadable),
        _ => {}
    }
    if !snap.has_visible_content() {
        return Some(FilterSkipReason::EmptyContent);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiki::settings::{WikiCaptureSettings, WikiSettings};
    use chrono::Utc;

    fn snap() -> WikiTurnSnapshot {
        WikiTurnSnapshot {
            run_id: "run-1".into(),
            connection_id: "c".into(),
            conversation_id: Some(9),
            agent_type: "claude_code".into(),
            working_dir: None,
            folder_id: Some(3),
            model: None,
            mode: None,
            occurred_at: Utc::now(),
            captured_at: Utc::now(),
            user_text: "u".into(),
            assistant_text: "a".into(),
            user_original_chars: 1,
            assistant_original_chars: 1,
            user_truncated: false,
            assistant_truncated: false,
            tool_observations: vec![],
            tool_dropped_count: 0,
            file_changes: vec![],
        }
    }

    fn ctx(settings: WikiSettings, kind: Option<ConversationKind>) -> FilterContext {
        FilterContext {
            folder_id: Some(3),
            root_folder_id: Some(1),
            conversation_kind: kind,
            settings,
        }
    }

    fn enabled() -> WikiSettings {
        WikiSettings {
            enabled: true,
            ..WikiSettings::default()
        }
    }

    #[test]
    fn filter_disabled_skips() {
        let settings = WikiSettings {
            enabled: false,
            ..WikiSettings::default()
        };
        assert_eq!(
            evaluate(&snap(), &ctx(settings, Some(ConversationKind::Regular))),
            Some(FilterSkipReason::Disabled)
        );
    }

    #[test]
    fn filter_acp_disabled_skips() {
        let mut settings = enabled();
        settings.capture = WikiCaptureSettings {
            acp_enabled: false,
            ..WikiCaptureSettings::default()
        };
        assert_eq!(
            evaluate(&snap(), &ctx(settings, Some(ConversationKind::Regular))),
            Some(FilterSkipReason::AcpDisabled)
        );
    }

    #[test]
    fn filter_delegate_kind_skipped() {
        assert_eq!(
            evaluate(&snap(), &ctx(enabled(), Some(ConversationKind::Delegate))),
            Some(FilterSkipReason::Delegate)
        );
    }

    #[test]
    fn filter_loop_kind_skipped() {
        assert_eq!(
            evaluate(&snap(), &ctx(enabled(), Some(ConversationKind::Loop))),
            Some(FilterSkipReason::Loop)
        );
    }

    #[test]
    fn filter_excluded_agent_and_folder() {
        let mut settings = enabled();
        settings.capture.exclude_agent_types = vec!["claude_code".into()];
        assert_eq!(
            evaluate(
                &snap(),
                &ctx(settings.clone(), Some(ConversationKind::Regular))
            ),
            Some(FilterSkipReason::ExcludedAgent)
        );
        settings.capture.exclude_agent_types.clear();
        settings.capture.exclude_folder_ids = vec![1];
        assert_eq!(
            evaluate(&snap(), &ctx(settings, Some(ConversationKind::Regular))),
            Some(FilterSkipReason::ExcludedFolder)
        );
    }

    #[test]
    fn filter_no_conversation_and_empty() {
        let mut s = snap();
        s.conversation_id = None;
        assert_eq!(
            evaluate(&s, &ctx(enabled(), Some(ConversationKind::Regular))),
            Some(FilterSkipReason::NoConversation)
        );
        s.conversation_id = Some(1);
        s.assistant_text.clear();
        s.tool_observations.clear();
        assert_eq!(
            evaluate(&s, &ctx(enabled(), Some(ConversationKind::Regular))),
            Some(FilterSkipReason::EmptyContent)
        );
    }

    #[test]
    fn filter_regular_with_text_passes() {
        assert_eq!(
            evaluate(&snap(), &ctx(enabled(), Some(ConversationKind::Regular))),
            None
        );
    }

    #[test]
    fn filter_missing_kind_fails_closed() {
        assert_eq!(
            evaluate(&snap(), &ctx(enabled(), None)),
            Some(FilterSkipReason::ConversationUnreadable)
        );
    }
}

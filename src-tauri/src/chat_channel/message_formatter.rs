use std::path::Path;

use super::i18n::{self, Lang};
use super::tool_detail;
use super::types::{MessageLevel, RichMessage};
use crate::acp::question::QuestionSpec;
use crate::models::AgentType;

const MAX_BODY_CHARS: usize = 2000;
const MAX_FILE_PATHS: usize = 6;
const MAX_SESSION_TITLE_CHARS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEventKind {
    Complete,
    Error,
    Notice,
    Question,
    UserMessage,
}

pub struct SessionEventCard<'a> {
    pub kind: SessionEventKind,
    pub agent_type: &'a str,
    pub conversation_title: Option<&'a str>,
    pub last_message: Option<&'a str>,
    pub error: Option<&'a str>,
    pub file_paths: &'a [String],
    pub working_dir: Option<&'a Path>,
}

fn nonempty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn take_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
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

/// Prefer the product display name when `raw` is a wire id (`claude_code`);
/// otherwise keep the live event's Display string (`Claude Code` / `Grok`).
fn agent_card_name(raw: &str) -> String {
    AgentType::from_wire(raw)
        .map(|a| a.to_string())
        .unwrap_or_else(|| raw.to_string())
}

fn relative_file_path(path: &str, working_dir: Option<&Path>) -> String {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return String::new();
    }
    working_dir
        .and_then(|wd| {
            let wd = wd.to_string_lossy().replace('\\', "/");
            let wd = wd.trim_end_matches('/');
            normalized
                .strip_prefix(wd)
                .map(|s| s.trim_start_matches('/').to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or(normalized)
}

fn format_files_field(paths: &[String], working_dir: Option<&Path>) -> String {
    let shown: Vec<String> = paths
        .iter()
        .map(|p| relative_file_path(p, working_dir))
        .filter(|s| !s.is_empty())
        .take(MAX_FILE_PATHS)
        .collect();
    let mut text = shown.join("\n");
    if paths.len() > MAX_FILE_PATHS {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("...");
    }
    text
}

fn format_card_title(card: &SessionEventCard<'_>, lang: Lang) -> String {
    let status = match card.kind {
        SessionEventKind::Complete => i18n::session_event_status_complete(lang),
        SessionEventKind::Error => i18n::session_event_status_error(lang),
        SessionEventKind::Notice => i18n::session_event_status_notice(lang),
        SessionEventKind::Question => i18n::session_event_status_question(lang),
        SessionEventKind::UserMessage => i18n::user_message_title(lang),
    };
    let mut parts = vec![status.to_string()];
    if let Some(session) = nonempty_trimmed(card.conversation_title) {
        parts.push(take_chars(session, MAX_SESSION_TITLE_CHARS));
    }
    let agent = agent_card_name(card.agent_type);
    if !agent.is_empty() {
        parts.push(agent);
    }
    parts.join(" ")
}

/// Events-tab session card: `{status} {session10} {agent}`, last-message body,
/// then `{n} files changed` listing at most six paths.
pub fn format_session_card(card: &SessionEventCard<'_>, lang: Lang) -> RichMessage {
    let last = nonempty_trimmed(card.last_message).map(str::to_string);
    let err = nonempty_trimmed(card.error).map(str::to_string);
    let body = match card.kind {
        SessionEventKind::Complete | SessionEventKind::Question | SessionEventKind::UserMessage => {
            last.unwrap_or_default()
        }
        SessionEventKind::Notice => {
            last.unwrap_or_else(|| i18n::user_stopped_message(lang).to_string())
        }
        SessionEventKind::Error => last
            .unwrap_or_else(|| err.unwrap_or_else(|| i18n::agent_error_fallback(lang).to_string())),
    };
    let level = match card.kind {
        SessionEventKind::Complete | SessionEventKind::UserMessage => MessageLevel::Info,
        SessionEventKind::Notice | SessionEventKind::Question => MessageLevel::Warning,
        SessionEventKind::Error => MessageLevel::Error,
    };
    let mut msg = RichMessage {
        title: Some(format_card_title(card, lang)),
        body: truncate_chars(&body, MAX_BODY_CHARS),
        fields: Vec::new(),
        level,
    };
    if !card.file_paths.is_empty() {
        msg = msg.with_field(
            i18n::files_changed_count_label(lang, card.file_paths.len()),
            format_files_field(card.file_paths, card.working_dir),
        );
    }
    msg
}

pub fn format_turn_complete(
    agent_type: &str,
    conversation_title: Option<&str>,
    last_message: Option<&str>,
    file_paths: &[String],
    working_dir: Option<&Path>,
    lang: Lang,
) -> RichMessage {
    format_session_card(
        &SessionEventCard {
            kind: SessionEventKind::Complete,
            agent_type,
            conversation_title,
            last_message,
            error: None,
            file_paths,
            working_dir,
        },
        lang,
    )
}

pub fn format_agent_error(
    agent_type: &str,
    conversation_title: Option<&str>,
    last_message: Option<&str>,
    error: Option<&str>,
    file_paths: &[String],
    working_dir: Option<&Path>,
    lang: Lang,
) -> RichMessage {
    format_session_card(
        &SessionEventCard {
            kind: SessionEventKind::Error,
            agent_type,
            conversation_title,
            last_message,
            error,
            file_paths,
            working_dir,
        },
        lang,
    )
}

/// Build the global-event-push notification for an agent permission request.
///
/// This is the passive, notification-only surface for sessions NOT initiated
/// from a chat channel (desktop / web): it tells the user an agent is blocked
/// waiting for approval and to act in Codeg. Chat-channel-initiated sessions
/// keep their interactive `/approve`,`/deny` flow in `session_event_subscriber`
/// and are suppressed here by the event subscriber (see `process_envelope`).
///
/// `tool_call` is the raw ACP tool-call object; the requested operation is
/// rendered with the same shared detail formatter the session relay uses, so a
/// `Bash` / `Write` / `Read` reads identically across both surfaces.
pub fn format_permission_request(tool_call: &serde_json::Value, lang: Lang) -> RichMessage {
    let tool_title = tool_call
        .get("title")
        .and_then(|v| v.as_str())
        .or_else(|| tool_call.get("tool_name").and_then(|v| v.as_str()))
        .unwrap_or("Unknown tool");

    let raw_input = tool_call
        .get("rawInput")
        .or_else(|| tool_call.get("raw_input"))
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Null => None,
            other => Some(other.to_string()),
        });

    let tool_desc = tool_detail::format_tool_call_detail(tool_title, raw_input.as_deref());

    RichMessage {
        title: Some(i18n::permission_request_title(lang).to_string()),
        body: i18n::permission_request_body(lang).to_string(),
        fields: vec![(
            i18n::permission_operation_label(lang).to_string(),
            tool_desc,
        )],
        level: MessageLevel::Warning,
    }
}

/// Build the "user message" notification for a prompt the user submitted from
/// the Codeg conversation UI. `text_preview` is the already-bounded message
/// text (see `ConnectionManager::send_prompt_linked`); it becomes the body so a
/// channel / webhook consumer sees what was sent.
pub fn format_user_prompt_sent(
    text_preview: &str,
    conversation_title: Option<&str>,
    agent_type: Option<&str>,
    lang: Lang,
) -> RichMessage {
    format_session_card(
        &SessionEventCard {
            kind: SessionEventKind::UserMessage,
            agent_type: agent_type.unwrap_or(""),
            conversation_title,
            last_message: Some(text_preview),
            error: None,
            file_paths: &[],
            working_dir: None,
        },
        lang,
    )
}

/// Build the global-event-push notification for an agent's `ask_user_question`
/// call. Like a permission request it is a blocking interactive gate — the
/// agent is parked until the user answers — so it carries `Warning` level and,
/// in the subscriber, bypasses the debounce (a blocked agent emits no further
/// event to re-trigger a lost nudge).
///
/// Each question becomes one field: the label is its `header` chip (falling
/// back to the localized "Question" when empty), and the value is the question
/// text with its option labels appended on their own lines, so an IM / webhook
/// consumer sees what is being asked and the available choices.
pub fn format_question_request(questions: &[QuestionSpec], lang: Lang) -> RichMessage {
    let fields: Vec<(String, String)> = questions
        .iter()
        .map(|q| {
            let label = if q.header.trim().is_empty() {
                i18n::question_label(lang).to_string()
            } else {
                q.header.clone()
            };
            let mut value = q.question.clone();
            for opt in &q.options {
                value.push_str("\n• ");
                value.push_str(&opt.label);
            }
            (label, value)
        })
        .collect();

    RichMessage {
        title: Some(i18n::question_request_title(lang).to_string()),
        body: i18n::question_request_body(lang).to_string(),
        fields,
        level: MessageLevel::Warning,
    }
}

pub struct DailyReportData {
    pub date: String,
    pub conversations_by_agent: Vec<(String, u32)>,
    pub total_conversations: u32,
    pub projects_involved: Vec<String>,
    pub key_activities: Vec<String>,
}

pub fn format_daily_report(report: &DailyReportData, lang: Lang) -> RichMessage {
    let mut body = i18n::daily_report_summary(lang, &report.date);

    body.push_str(&format!(
        "\n\n{}",
        i18n::total_sessions(lang, report.total_conversations)
    ));

    if !report.conversations_by_agent.is_empty() {
        body.push_str(&format!("\n\n{}", i18n::by_agent_label(lang)));
        for (agent, count) in &report.conversations_by_agent {
            body.push_str(&format!(
                "\n  {}",
                i18n::agent_session_count(lang, agent, *count)
            ));
        }
    }

    if !report.projects_involved.is_empty() {
        body.push_str(&format!(
            "\n\n{}",
            i18n::projects_label(lang, &report.projects_involved.join(", "))
        ));
    }

    if !report.key_activities.is_empty() {
        body.push_str(&format!("\n\n{}", i18n::key_activities_label(lang)));
        for activity in &report.key_activities {
            body.push_str(&format!("\n  • {}", activity));
        }
    }

    RichMessage::info(body).with_title(i18n::daily_report_title(lang))
}

#[cfg(test)]
mod permission_request_tests {
    use super::*;

    #[test]
    fn renders_title_warning_and_operation_from_object_input() {
        let tool_call = serde_json::json!({
            "title": "Bash",
            "rawInput": { "command": "rm -rf build" }
        });
        let msg = format_permission_request(&tool_call, Lang::En);
        assert_eq!(msg.level, MessageLevel::Warning);
        assert_eq!(msg.title.as_deref(), Some("Permission Request"));
        let text = msg.to_plain_text();
        assert!(text.contains("Bash: rm -rf build"), "got {text}");
    }

    #[test]
    fn handles_bare_string_raw_input_and_localizes_title() {
        let tool_call = serde_json::json!({
            "title": "Bash",
            "rawInput": "ls -la"
        });
        let msg = format_permission_request(&tool_call, Lang::ZhCn);
        assert_eq!(msg.title.as_deref(), Some("权限请求"));
        assert!(msg.to_plain_text().contains("Bash: ls -la"));
    }

    #[test]
    fn falls_back_to_unknown_tool_when_empty() {
        let msg = format_permission_request(&serde_json::json!({}), Lang::En);
        assert!(msg.to_plain_text().contains("Unknown tool"));
    }
}

#[cfg(test)]
mod user_prompt_sent_tests {
    use super::*;

    #[test]
    fn renders_localized_title_and_message_as_body() {
        let msg = format_user_prompt_sent("refactor the auth module", None, None, Lang::En);
        assert_eq!(msg.level, MessageLevel::Info);
        assert_eq!(msg.title.as_deref(), Some("Start"));
        assert_eq!(msg.body, "refactor the auth module");
    }

    #[test]
    fn localizes_title_per_language() {
        let msg = format_user_prompt_sent("你好", None, None, Lang::ZhCn);
        assert_eq!(msg.title.as_deref(), Some("开始任务"));
        assert!(msg.to_plain_text().contains("你好"));
    }

    #[test]
    fn title_is_status_session10_agent() {
        let msg = format_user_prompt_sent(
            "你好",
            Some("User Greeting and Session Start"),
            Some("Grok"),
            Lang::ZhCn,
        );
        assert_eq!(msg.title.as_deref(), Some("开始任务 User Greet Grok"));
        assert_eq!(msg.body, "你好");
        assert!(msg.fields.is_empty());
    }
}

#[cfg(test)]
mod question_request_tests {
    use super::*;
    use crate::acp::question::QuestionOption;

    fn spec(header: &str, question: &str, options: &[&str]) -> QuestionSpec {
        QuestionSpec {
            id: "q1".into(),
            question: question.into(),
            header: header.into(),
            multi_select: false,
            options: options
                .iter()
                .map(|l| QuestionOption {
                    label: (*l).into(),
                    description: String::new(),
                })
                .collect(),
            is_secret: false,
        }
    }

    #[test]
    fn renders_title_warning_header_and_option_labels() {
        let q = spec(
            "Approach",
            "Which approach should we take?",
            &["MVP first", "Risk first"],
        );
        let msg = format_question_request(&[q], Lang::En);
        assert_eq!(msg.level, MessageLevel::Warning);
        assert_eq!(msg.title.as_deref(), Some("Agent Question"));
        let text = msg.to_plain_text();
        assert!(text.contains("Approach"), "got {text}");
        assert!(
            text.contains("Which approach should we take?"),
            "got {text}"
        );
        assert!(text.contains("MVP first"), "got {text}");
        assert!(text.contains("Risk first"), "got {text}");
    }

    #[test]
    fn empty_header_falls_back_to_localized_question_label() {
        let msg = format_question_request(&[spec("", "Proceed?", &[])], Lang::En);
        assert_eq!(msg.fields[0].0, "Question");
        assert_eq!(msg.fields[0].1, "Proceed?");
    }

    #[test]
    fn one_field_per_question() {
        let msg = format_question_request(
            &[spec("A", "first?", &[]), spec("B", "second?", &[])],
            Lang::En,
        );
        assert_eq!(msg.fields.len(), 2);
    }

    #[test]
    fn localizes_title_per_language() {
        let msg = format_question_request(&[spec("方式", "选哪个？", &[])], Lang::ZhCn);
        assert_eq!(msg.title.as_deref(), Some("智能体提问"));
    }
}

#[cfg(test)]
mod session_event_card_tests {
    use super::*;
    use std::path::Path;

    fn complete<'a>(
        agent: &'a str,
        title: Option<&'a str>,
        last: Option<&'a str>,
        files: &'a [String],
        wd: Option<&'a Path>,
    ) -> SessionEventCard<'a> {
        SessionEventCard {
            kind: SessionEventKind::Complete,
            agent_type: agent,
            conversation_title: title,
            last_message: last,
            error: None,
            file_paths: files,
            working_dir: wd,
        }
    }

    #[test]
    fn completed_title_is_status_session10_agent() {
        let msg = format_session_card(
            &complete(
                "Grok",
                Some("User Greeting and Session Start"),
                Some("Hi — ready to help."),
                &[],
                None,
            ),
            Lang::ZhCn,
        );
        assert_eq!(msg.title.as_deref(), Some("完成 User Greet Grok"));
        assert_eq!(msg.body, "Hi — ready to help.");
        assert!(msg.fields.is_empty());
    }

    #[test]
    fn wire_agent_id_becomes_display_name() {
        let msg = format_turn_complete(
            "claude_code",
            Some("Fix auth"),
            Some("done"),
            &[],
            None,
            Lang::En,
        );
        assert_eq!(msg.title.as_deref(), Some("Done Fix auth Claude Code"));
        assert_eq!(msg.body, "done");
    }

    #[test]
    fn files_field_lists_first_six_relative_paths_and_ellipsis() {
        let files: Vec<String> = (b'a'..=b'h')
            .map(|c| format!("/work/app/src/{}.rs", c as char))
            .collect();
        let msg = format_session_card(
            &complete(
                "Grok",
                Some("Fix auth"),
                Some("done"),
                &files,
                Some(Path::new("/work/app")),
            ),
            Lang::ZhCn,
        );
        let (label, value) = msg
            .fields
            .iter()
            .find(|(k, _)| k.contains("文件被改动"))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .expect("files field");
        assert_eq!(label, "8个文件被改动");
        assert!(value.contains("src/a.rs") && value.contains("src/f.rs"));
        assert!(!value.contains("src/g.rs") && !value.contains("src/h.rs"));
        assert!(!value.contains("/work/app"));
        assert!(
            value.lines().any(|line| line.trim() == "..."),
            "got {value}"
        );
        assert_eq!(value.lines().filter(|line| line.contains(".rs")).count(), 6);
    }

    #[test]
    fn error_title_uses_exception_status_and_last_message_body() {
        let msg = format_agent_error(
            "Codex CLI",
            Some("Fix auth"),
            Some("partial"),
            Some("refusal"),
            &[],
            None,
            Lang::ZhCn,
        );
        assert_eq!(msg.level, MessageLevel::Error);
        assert_eq!(msg.title.as_deref(), Some("异常 Fix auth Codex CLI"));
        assert_eq!(msg.body, "partial");
        assert!(
            !msg.fields
                .iter()
                .any(|(k, _)| k == "问题" || k == "错误信息"),
            "got {:?}",
            msg.fields
        );
    }

    #[test]
    fn completed_without_text_or_files_still_sends_title() {
        let msg = format_turn_complete("Grok", Some("Fix auth"), None, &[], None, Lang::ZhCn);
        assert_eq!(msg.title.as_deref(), Some("完成 Fix auth Grok"));
        assert!(msg.body.is_empty());
        assert!(msg.fields.is_empty());
    }
}

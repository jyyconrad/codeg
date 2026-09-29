//! Pure text for a host workflow-completion prompt.
//!
//! Evidence is whatever the caller took from an accepted terminal frame or a
//! later accepted terminal revision. This module does not read `WorkflowRun`
//! or `report.md`, and it does not touch the database or send prompts.

use crate::models::AgentType;

pub const RESULT_BUDGET_BYTES: usize = 24 * 1024;
pub const RESULT_HEAD_BYTES: usize = 8 * 1024;
pub const RESULT_TAIL_BYTES: usize = 16 * 1024;
pub const RESULT_OMISSION_MARK: &str = "\n\n[... workflow result truncated ...]\n\n";
pub const UNAVAILABLE_DETAIL: &str = "No conclusion/details were provided.";

/// Live `user_message` id prefix. History parsing uses the same string so a
/// reconnect collapses onto one turn. Kept here, and duplicated in the Grok
/// parser: that parser must not import this module (it would cycle through
/// `acp`).
pub const FOLLOW_UP_MESSAGE_ID_PREFIX: &str = "codeg-workflow-follow-up:";

pub fn follow_up_message_id(run_id: &str) -> String {
    format!("{FOLLOW_UP_MESSAGE_ID_PREFIX}{run_id}")
}

/// Unicode scalars kept from a workflow name or run id before an ellipsis.
const FIELD_SCALAR_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowResultSource {
    FinalSummary,
    ErrorDetail,
    Unavailable,
}

impl WorkflowResultSource {
    /// Exact template phrase: "workflow final summary" | "workflow error detail" | "unavailable"
    pub fn label(self) -> &'static str {
        match self {
            Self::FinalSummary => "workflow final summary",
            Self::ErrorDetail => "workflow error detail",
            Self::Unavailable => "unavailable",
        }
    }

    /// Column value: `final_summary` | `error_detail` | `unavailable`.
    pub fn storage_key(self) -> &'static str {
        match self {
            Self::FinalSummary => "final_summary",
            Self::ErrorDetail => "error_detail",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowResultBody {
    pub source: WorkflowResultSource,
    /// Truncated but NOT escaped.
    pub text: String,
}

/// Evidence that arrived on an accepted terminal frame or a later accepted
/// terminal revision.
///
/// Do not read `WorkflowRun`. A progress-frame summary must not be passed in
/// here by callers. `terminal_status` is `"completed"` or `"failed"`.
///
/// Summary wins. On failed only, fall back to trimmed `last_event_detail`.
/// If that detail trims equal to trimmed `current_phase`, it is a progress
/// name, not an error conclusion: ignore it. Completed never uses
/// `last_event_detail` as the body. Blank / whitespace-only counts as missing.
/// When both are missing, source is [`WorkflowResultSource::Unavailable`] and
/// text is [`UNAVAILABLE_DETAIL`] (not truncated specially). Otherwise text is
/// [`truncate_workflow_result`] of the chosen string.
pub fn extract_terminal_evidence(
    terminal_status: &str,
    terminal_summary: Option<&str>,
    terminal_error_detail: Option<&str>,
    current_phase: Option<&str>,
) -> WorkflowResultBody {
    if let Some(summary) = trimmed_nonempty(terminal_summary) {
        return WorkflowResultBody {
            source: WorkflowResultSource::FinalSummary,
            text: truncate_workflow_result(summary),
        };
    }
    if let Some(detail) = failed_error_detail(terminal_status, terminal_error_detail, current_phase)
    {
        return WorkflowResultBody {
            source: WorkflowResultSource::ErrorDetail,
            text: truncate_workflow_result(detail),
        };
    }
    WorkflowResultBody {
        source: WorkflowResultSource::Unavailable,
        text: UNAVAILABLE_DETAIL.to_string(),
    }
}

/// If `text` is <= 24 KiB UTF-8, return it unchanged.
///
/// If longer, keep a char-boundary head of at most 8 KiB and a char-boundary
/// tail of at most 16 KiB, joined by [`RESULT_OMISSION_MARK`]. Do not split a
/// char. Status, ids, and instructions are not part of this string.
pub fn truncate_workflow_result(text: &str) -> String {
    if text.len() <= RESULT_BUDGET_BYTES {
        return text.to_string();
    }
    let head = char_boundary_prefix(text, RESULT_HEAD_BYTES);
    let tail = char_boundary_suffix(text, RESULT_TAIL_BYTES);
    // Head + tail budgets equal the 24 KiB cap, and we only slice past that
    // cap, so the kept prefix and suffix cannot overlap.
    debug_assert!(head.len() + tail.len() < text.len());
    let mut out = String::with_capacity(head.len() + RESULT_OMISSION_MARK.len() + tail.len());
    out.push_str(head);
    out.push_str(RESULT_OMISSION_MARK);
    out.push_str(tail);
    out
}

/// Escape `&`, `<`, `>` as `&amp;`, `&lt;`, `&gt;` so a result cannot forge the
/// template's end tag. Escape `&` first.
pub fn escape_workflow_result(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowUpPrompt<'a> {
    pub workflow_name: &'a str,
    /// `"completed"` or `"failed"`.
    pub status: &'a str,
}

/// One user-text prompt. The name is trimmed, internal whitespace (including
/// newlines) is collapsed to a single space, then capped at 200 Unicode
/// scalars. A cut appends `…` after those 200 scalars. The result body stays
/// in the delivery record and is not repeated here.
pub fn render_workflow_follow_up_prompt(input: FollowUpPrompt<'_>) -> String {
    let mut name = sanitize_prompt_field(input.workflow_name);
    if name.is_empty() {
        name = "Workflow".to_string();
    }
    let failed = input.status == "failed";
    let status = if failed { "failed" } else { "completed" };
    let actions = if failed {
        "\
- State why this run failed.\n\
- Take the next justified step now. If the goal cannot continue, summarize the blocker and stop.\n"
    } else {
        "\
- If the goal is met, summarize what was completed, then stop.\n\
- If work remains, continue that work now. Do not stop at a proposal.\n"
    };
    format!(
        "[system notification]\n\
Workflow: {name}\n\
Status: {status}\n\
\n\
This run has ended. Use the outcome already in this conversation. Compare it with the user's goal and do the next required step in this turn.\n\
{actions}\
Do not restart this workflow merely because this notification arrived."
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostWakePolicy {
    /// This agent can carry a workflow completion into the current session, so
    /// the host sends one follow-up prompt.
    Wake,
    /// This agent has no workflow channel Codeg advertises or adapts. The
    /// delivery row is recorded as skipped.
    Skip,
}

/// Allow-list of agents whose protocol can deliver a workflow completion.
///
/// Grok publishes `workflow_updated`. Claude Code and Codex are the adapters
/// offered `asyncTasks`; a `taskType=workflow` frame on either becomes the
/// same terminal edge. OpenCode is not offered that capability. Codeg Agent,
/// custom agents, and the other built-ins have no workflow channel. A new
/// [`AgentType`] must choose Wake or Skip here.
pub fn host_wake_policy(agent_type: AgentType) -> HostWakePolicy {
    match agent_type {
        AgentType::Grok | AgentType::ClaudeCode | AgentType::Codex => HostWakePolicy::Wake,
        AgentType::OpenCode
        | AgentType::Gemini
        | AgentType::OpenClaw
        | AgentType::Cline
        | AgentType::Hermes
        | AgentType::CodeBuddy
        | AgentType::KimiCode
        | AgentType::Pi
        | AgentType::Cursor
        | AgentType::DeepSeek
        | AgentType::Qoder
        | AgentType::Antigravity
        | AgentType::CodegAgent
        | AgentType::Custom(_) => HostWakePolicy::Skip,
    }
}

fn trimmed_nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// Error text for a failed terminal frame, or `None` when it is missing or is
/// only the current phase name.
fn failed_error_detail<'a>(
    terminal_status: &str,
    terminal_error_detail: Option<&'a str>,
    current_phase: Option<&str>,
) -> Option<&'a str> {
    if terminal_status != "failed" {
        return None;
    }
    let detail = trimmed_nonempty(terminal_error_detail)?;
    if trimmed_nonempty(current_phase) == Some(detail) {
        return None;
    }
    Some(detail)
}

fn char_boundary_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn char_boundary_suffix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn sanitize_prompt_field(raw: &str) -> String {
    let mut collapsed = String::new();
    let mut word_gap = false;
    for ch in raw.trim().chars() {
        if ch.is_whitespace() {
            word_gap = !collapsed.is_empty();
            continue;
        }
        if word_gap {
            collapsed.push(' ');
            word_gap = false;
        }
        collapsed.push(ch);
    }
    cap_scalars(&collapsed, FIELD_SCALAR_CAP)
}

/// Keep at most `max_scalars` Unicode scalars. If that cuts the string, append
/// `…` after the kept scalars (the ellipsis is not part of the cap).
fn cap_scalars(text: &str, max_scalars: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if count == max_scalars {
            out.push('\u{2026}');
            break;
        }
        out.push(ch);
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AgentType;

    #[test]
    fn completed_uses_summary_and_ignores_last_event_detail() {
        let body = extract_terminal_evidence(
            "completed",
            Some("  <done> & dust\nline2\n"),
            Some("should be ignored"),
            Some("phase"),
        );
        assert_eq!(body.source, WorkflowResultSource::FinalSummary);
        assert_eq!(body.text, "<done> & dust\nline2");
    }

    #[test]
    fn failed_prefers_summary_over_detail() {
        let body = extract_terminal_evidence(
            "failed",
            Some("  summary wins  "),
            Some("phase-name"),
            Some("phase-name"),
        );
        assert_eq!(body.source, WorkflowResultSource::FinalSummary);
        assert_eq!(body.text, "summary wins");
    }

    #[test]
    fn failed_uses_detail_when_summary_is_missing() {
        let missing =
            extract_terminal_evidence("failed", None, Some("  disk full  "), Some("pack"));
        assert_eq!(missing.source, WorkflowResultSource::ErrorDetail);
        assert_eq!(missing.text, "disk full");

        let blank_summary =
            extract_terminal_evidence("failed", Some(" \n "), Some("disk full"), None);
        assert_eq!(blank_summary.source, WorkflowResultSource::ErrorDetail);
        assert_eq!(blank_summary.text, "disk full");
    }

    #[test]
    fn failed_ignores_detail_equal_to_current_phase() {
        let body = extract_terminal_evidence("failed", None, Some("  verify\n"), Some("\tverify"));
        assert_eq!(body.source, WorkflowResultSource::Unavailable);
        assert_eq!(body.text, UNAVAILABLE_DETAIL);

        let kept =
            extract_terminal_evidence("failed", Some("   "), Some("verify failed"), Some("verify"));
        assert_eq!(kept.source, WorkflowResultSource::ErrorDetail);
        assert_eq!(kept.text, "verify failed");
    }

    #[test]
    fn whitespace_only_summary_and_detail_become_unavailable() {
        for status in ["completed", "failed"] {
            let body =
                extract_terminal_evidence(status, Some(" \n\t "), Some(" \r\n "), Some("phase"));
            assert_eq!(body.source, WorkflowResultSource::Unavailable, "{status}");
            assert_eq!(body.text, UNAVAILABLE_DETAIL);
        }
    }

    #[test]
    fn completed_without_summary_is_unavailable_detail() {
        let body = extract_terminal_evidence(
            "completed",
            None,
            Some("should not be used even when it differs from the phase"),
            Some("phase"),
        );
        assert_eq!(body.source, WorkflowResultSource::Unavailable);
        assert_eq!(body.text, UNAVAILABLE_DETAIL);

        let blank = extract_terminal_evidence("completed", Some(" \n\t "), Some("detail"), None);
        assert_eq!(blank.source, WorkflowResultSource::Unavailable);
        assert_eq!(blank.text, UNAVAILABLE_DETAIL);
    }

    #[test]
    fn truncate_keeps_multibyte_scalars_on_cut_edges_and_leaves_short_text() {
        let short = "短结论";
        assert_eq!(truncate_workflow_result(short), short);
        let exact = "x".repeat(RESULT_BUDGET_BYTES);
        assert_eq!(truncate_workflow_result(&exact), exact);

        let mut ascii = String::new();
        ascii.push_str(&"h".repeat(RESULT_HEAD_BYTES));
        ascii.push_str(&"m".repeat(10));
        ascii.push_str(&"t".repeat(RESULT_TAIL_BYTES));
        assert_eq!(
            truncate_workflow_result(&ascii),
            format!(
                "{}{RESULT_OMISSION_MARK}{}",
                "h".repeat(RESULT_HEAD_BYTES),
                "t".repeat(RESULT_TAIL_BYTES)
            )
        );

        // A 3-byte scalar sits on each byte cut. It stays whole: the kept head
        // and tail are a source prefix and suffix, so the scalar is dropped
        // entirely rather than split. Taking it would pass the byte ceilings.
        let cjk = "测";
        assert_eq!(cjk.len(), 3);

        let mut raw = String::new();
        raw.push_str(&"a".repeat(RESULT_HEAD_BYTES - 1));
        raw.push_str(cjk);
        let head_cjk_at = RESULT_HEAD_BYTES - 1;
        assert!(!raw.is_char_boundary(RESULT_HEAD_BYTES));

        let second_cjk_at = 9000;
        raw.push_str(&"b".repeat(second_cjk_at - raw.len()));
        raw.push_str(cjk);
        let final_len = second_cjk_at + 1 + RESULT_TAIL_BYTES;
        raw.push_str(&"z".repeat(final_len - raw.len()));
        assert_eq!(raw.len(), final_len);
        assert!(final_len > RESULT_BUDGET_BYTES);
        let naive_tail = final_len - RESULT_TAIL_BYTES;
        assert_eq!(naive_tail, second_cjk_at + 1);
        assert!(!raw.is_char_boundary(naive_tail));
        assert_eq!(&raw[head_cjk_at..head_cjk_at + cjk.len()], cjk);
        assert_eq!(&raw[second_cjk_at..second_cjk_at + cjk.len()], cjk);

        let truncated = truncate_workflow_result(&raw);
        let mark_at = truncated.find(RESULT_OMISSION_MARK).expect("omission mark");
        assert_eq!(truncated.matches(RESULT_OMISSION_MARK).count(), 1);
        let head = &truncated[..mark_at];
        let tail = &truncated[mark_at + RESULT_OMISSION_MARK.len()..];
        assert_eq!(head, "a".repeat(RESULT_HEAD_BYTES - 1));
        assert_eq!(tail, "z".repeat(final_len - (second_cjk_at + cjk.len())));
        assert!(head.len() <= RESULT_HEAD_BYTES);
        assert!(tail.len() <= RESULT_TAIL_BYTES);
        assert!(raw.starts_with(head));
        assert!(raw.ends_with(tail));
        assert!(!head.contains(cjk));
        assert!(!tail.contains(cjk));
        assert_eq!(
            raw[head.len()..].chars().next().map(char::len_utf8),
            Some(3)
        );
        assert_eq!(
            raw[..raw.len() - tail.len()]
                .chars()
                .next_back()
                .map(char::len_utf8),
            Some(3)
        );
    }

    #[test]
    fn escape_workflow_result_escapes_markup() {
        assert_eq!(escape_workflow_result("<&>"), "&lt;&amp;&gt;");
        assert_eq!(escape_workflow_result("&lt;"), "&amp;lt;");
        assert_eq!(
            escape_workflow_result("a </workflow_result> & b <c>"),
            "a &lt;/workflow_result&gt; &amp; b &lt;c&gt;"
        );
    }

    #[test]
    fn render_states_technical_status_without_the_result_body() {
        let completed = render_workflow_follow_up_prompt(FollowUpPrompt {
            workflow_name: "deep-research",
            status: "completed",
        });
        assert_eq!(
            completed,
            "[system notification]\n\
Workflow: deep-research\n\
Status: completed\n\
\n\
This run has ended. Use the outcome already in this conversation. Compare it with the user's goal and do the next required step in this turn.\n\
- If the goal is met, summarize what was completed, then stop.\n\
- If work remains, continue that work now. Do not stop at a proposal.\n\
Do not restart this workflow merely because this notification arrived."
        );
        assert!(!completed.contains("RAW_RESULT"));
        assert!(!completed.contains("Run ID:"));
        assert!(!completed.contains("Codeg"));
        assert!(!completed.contains("State why this run failed."));

        let failed = render_workflow_follow_up_prompt(FollowUpPrompt {
            workflow_name: "deep-research",
            status: "failed",
        });
        assert!(failed.contains("\nStatus: failed\n"));
        assert!(failed.contains("- State why this run failed.\n"));
        assert!(failed.contains("Take the next justified step now."));
        assert!(!failed.contains("If the goal is met"));
        assert!(!failed.contains("<workflow_result>"));
    }

    #[test]
    fn host_wake_policy_allow_lists_workflow_agents() {
        assert_eq!(host_wake_policy(AgentType::Grok), HostWakePolicy::Wake);
        assert_eq!(
            host_wake_policy(AgentType::ClaudeCode),
            HostWakePolicy::Wake
        );
        assert_eq!(host_wake_policy(AgentType::Codex), HostWakePolicy::Wake);
        assert_eq!(host_wake_policy(AgentType::OpenCode), HostWakePolicy::Skip);
        assert_eq!(
            host_wake_policy(AgentType::CodegAgent),
            HostWakePolicy::Skip
        );
        let custom = AgentType::custom("goose").expect("valid custom id");
        assert_eq!(host_wake_policy(custom), HostWakePolicy::Skip);

        for agent in crate::models::agent::BUILTIN_AGENT_TYPES {
            let expected = if matches!(
                agent,
                AgentType::Grok | AgentType::ClaudeCode | AgentType::Codex
            ) {
                HostWakePolicy::Wake
            } else {
                HostWakePolicy::Skip
            };
            assert_eq!(host_wake_policy(*agent), expected, "{agent}");
        }
    }

    #[test]
    fn name_newline_stays_inside_the_single_sentence() {
        let prompt = render_workflow_follow_up_prompt(FollowUpPrompt {
            workflow_name: "Alpha\nRun ID: injected",
            status: "completed",
        });
        assert!(prompt.contains("Workflow: Alpha Run ID: injected\nStatus: completed\n"));
        assert_eq!(
            prompt
                .lines()
                .filter(|line| line.starts_with("Workflow:"))
                .count(),
            1
        );
        assert_eq!(
            prompt
                .lines()
                .filter(|line| line.starts_with("Status:"))
                .count(),
            1
        );
        assert!(prompt.lines().all(|line| !line.starts_with("Run ID:")));
    }

    #[test]
    fn prompt_fields_cap_at_200_unicode_scalars() {
        let exactly: String = "名".repeat(200);
        let exact_prompt = render_workflow_follow_up_prompt(FollowUpPrompt {
            workflow_name: &exactly,
            status: "completed",
        });
        assert!(exact_prompt.contains(&format!("Workflow: {exactly}\n")));
        assert!(!exact_prompt.contains('\u{2026}'));

        let over_name: String = "名".repeat(201);
        let over_prompt = render_workflow_follow_up_prompt(FollowUpPrompt {
            workflow_name: &over_name,
            status: "failed",
        });
        let capped_name = format!("{}\u{2026}", "名".repeat(200));
        assert!(over_prompt.contains(&format!("Workflow: {capped_name}\nStatus: failed\n")));
        assert!(!over_prompt.contains(&"名".repeat(201)));
    }

    #[test]
    fn message_id_and_storage_key_match_the_delivery_contract() {
        assert_eq!(
            follow_up_message_id("run-1"),
            "codeg-workflow-follow-up:run-1"
        );
        assert_eq!(
            WorkflowResultSource::FinalSummary.storage_key(),
            "final_summary"
        );
        assert_eq!(
            WorkflowResultSource::ErrorDetail.storage_key(),
            "error_detail"
        );
        assert_eq!(
            WorkflowResultSource::Unavailable.storage_key(),
            "unavailable"
        );
    }
}

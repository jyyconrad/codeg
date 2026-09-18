//! Thin [`MemoryPolicy`] over Rig [`TokenWindowMemory`].
//!
//! The token window selects a suffix; this wrapper only moves that cut to a
//! legal message boundary (tool pairing, pending prompt, committed summary
//! coverage). It does not reimplement the window walk.

use std::collections::HashSet;
use std::sync::Mutex;

use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::Message;
use rig_memory::{HeuristicTokenCounter, MemoryError, MemoryPolicy, TokenWindowMemory};

use super::budget::{BudgetConfig, BudgetError};

/// Default real user turns to retain when the token window still allows it.
pub const DEFAULT_PROTECT_RECENT_USER_TURNS: usize = 2;
/// Default complete assistant+tool-result groups to retain when the window allows it.
pub const DEFAULT_PROTECT_RECENT_TOOL_GROUPS: usize = 3;

/// Per-request snapshot that [`CodegContextPolicy`] must not look up later.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestScope {
    /// 1-based seq already absorbed into a committed summary; demoted must
    /// cover at least this prefix. Expanding the window must not put that
    /// prefix back into kept.
    pub covers_through_seq: Option<usize>,
    /// Current pending prompt (not necessarily in `messages`). If it equals
    /// the last real user message, that message and any incomplete trailing
    /// assistant/tool group stay in kept.
    pub pending_prompt: Option<Message>,
    /// Real user turns to retain when the window allows (tool-result `User`
    /// messages do not count). Default 2.
    pub protect_recent_user_turns: usize,
    /// Complete assistant-toolcall + matching tool-result groups to retain
    /// when the window allows. Default 3.
    pub protect_recent_tool_groups: usize,
}

impl Default for RequestScope {
    fn default() -> Self {
        Self {
            covers_through_seq: None,
            pending_prompt: None,
            protect_recent_user_turns: DEFAULT_PROTECT_RECENT_USER_TURNS,
            protect_recent_tool_groups: DEFAULT_PROTECT_RECENT_TOOL_GROUPS,
        }
    }
}

impl RequestScope {
    pub fn new() -> Self {
        Self::default()
    }
}

/// `MemoryPolicy` for coding sessions that need pairing, pending, and coverage.
///
/// Simple text sessions can use [`TokenWindowMemory`] directly.
///
/// [`RequestScope`] is interior-mutable so one [`CodegContextPolicy`] (and the
/// [`rig_memory::CompactingMemory`] that owns it) can be reused across model
/// turns without reconstructing the wrapper and losing the absorbed watermark.
#[derive(Debug)]
pub struct CodegContextPolicy {
    window: TokenWindowMemory,
    scope: Mutex<RequestScope>,
}

impl CodegContextPolicy {
    pub fn new(window: TokenWindowMemory, scope: RequestScope) -> Self {
        Self {
            window,
            scope: Mutex::new(scope),
        }
    }

    pub fn token_window(tail_budget: usize, scope: RequestScope) -> Self {
        Self::new(
            TokenWindowMemory::new(tail_budget, HeuristicTokenCounter::openai()),
            scope,
        )
    }

    pub fn scope(&self) -> RequestScope {
        self.scope.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replace the per-request snapshot in place. Does not change the cut-point
    /// algorithm or the token window.
    pub fn bind_scope(&self, scope: RequestScope) {
        *self.scope.lock().unwrap_or_else(|e| e.into_inner()) = scope;
    }

    fn min_demote_len(scope: &RequestScope, n: usize) -> usize {
        match scope.covers_through_seq {
            Some(seq) if seq > 0 => seq.min(n),
            _ => 0,
        }
    }
}

/// `safety_margin = max(1024, ceil(context_window × 5%))`.
pub fn safety_margin(context_window: u64) -> u64 {
    BudgetConfig::new(context_window, 0).safety_margin()
}

/// `input_budget = context_window - max_output_tokens - safety_margin`.
pub fn input_budget(context_window: u64, max_output_tokens: u64) -> Result<u64, BudgetError> {
    BudgetConfig::new(context_window, max_output_tokens).input_budget()
}

impl MemoryPolicy for CodegContextPolicy {
    fn apply(&self, messages: Vec<Message>) -> Result<Vec<Message>, MemoryError> {
        Ok(self.apply_with_demoted(messages)?.0)
    }

    fn apply_with_demoted(
        &self,
        messages: Vec<Message>,
    ) -> Result<(Vec<Message>, Vec<Message>), MemoryError> {
        if messages.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let (tw_kept, tw_demoted) = self.window.apply_with_demoted(messages)?;
        let token_cut = tw_demoted.len();
        let mut original = tw_demoted;
        original.extend(tw_kept);
        let n = original.len();

        let scope = self.scope();
        let groups = tool_groups(&original);
        let min_cut = Self::min_demote_len(&scope, n);
        let must_from = pending_keep_from(&original, &scope);
        let preferred_from = preferred_keep_from(
            &original,
            &groups,
            scope.protect_recent_user_turns,
            scope.protect_recent_tool_groups,
        );

        if let Some(must) = must_from {
            if min_cut > must || token_cut > must {
                return Err(budget_exceeded(
                    "protected pending prompt or incomplete tool group does not fit the token window",
                ));
            }
        }

        let mut cut = token_cut.max(min_cut);
        cut = legalize_cut(cut, &original, &groups, min_cut, &self.window);

        if let Some(must) = must_from {
            if cut > must {
                if must < min_cut || !suffix_fits(&self.window, &original, must) {
                    return Err(budget_exceeded(
                        "covers_through_seq would demote the protected pending prompt",
                    ));
                }
                cut = legalize_cut(must, &original, &groups, min_cut, &self.window);
                if cut > must {
                    return Err(budget_exceeded(
                        "no legal boundary keeps the pending prompt and its tool group",
                    ));
                }
            }
        }

        if let Some(preferred) = preferred_from {
            let candidate = preferred.max(min_cut);
            if candidate < cut && suffix_fits(&self.window, &original, candidate) {
                let legal = legalize_cut(candidate, &original, &groups, min_cut, &self.window);
                if legal < cut && suffix_fits(&self.window, &original, legal) {
                    cut = legal;
                }
            }
        }

        cut = drop_leading_orphans(&original, cut);
        if let Some(must) = must_from {
            if cut > must {
                return Err(budget_exceeded(
                    "leading tool-result adjustment would demote the pending prompt",
                ));
            }
        }

        cut = cut.min(n);
        let demoted = original[..cut].to_vec();
        let kept = original[cut..].to_vec();
        if has_unpaired_tool_result(&kept) {
            return Err(budget_exceeded(
                "no legal cut preserves tool call/result pairing",
            ));
        }
        Ok((kept, demoted))
    }
}

fn budget_exceeded(detail: &str) -> MemoryError {
    MemoryError::Policy(format!("context_budget_exceeded: {detail}"))
}

struct ToolGroup {
    start: usize,
    end: usize,
    complete: bool,
}

fn tool_groups(messages: &[Message]) -> Vec<ToolGroup> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let ids = assistant_tool_call_ids(&messages[i]);
        if ids.is_empty() {
            i += 1;
            continue;
        }
        let mut open: HashSet<&str> = ids.into_iter().collect();
        let start = i;
        let mut end = i + 1;
        i += 1;
        while i < messages.len() {
            if is_real_user_turn(&messages[i]) || !assistant_tool_call_ids(&messages[i]).is_empty()
            {
                break;
            }
            let results = tool_result_ids(&messages[i]);
            if results.is_empty() {
                break;
            }
            if !results.iter().any(|id| open.contains(*id)) {
                break;
            }
            for id in results {
                open.remove(id);
            }
            i += 1;
            end = i;
        }
        groups.push(ToolGroup {
            start,
            end,
            complete: open.is_empty(),
        });
    }
    groups
}

fn legalize_cut(
    mut cut: usize,
    messages: &[Message],
    groups: &[ToolGroup],
    min_cut: usize,
    window: &TokenWindowMemory,
) -> usize {
    for _ in 0..=groups.len() {
        let Some(group) = groups.iter().find(|g| g.start < cut && cut < g.end) else {
            break;
        };
        if group.start >= min_cut && suffix_fits(window, messages, group.start) {
            cut = group.start;
        } else {
            cut = group.end;
        }
    }
    cut
}

fn suffix_fits(window: &TokenWindowMemory, messages: &[Message], cut: usize) -> bool {
    if cut >= messages.len() {
        return true;
    }
    let suffix = messages[cut..].to_vec();
    let suffix_len = suffix.len();
    match window.apply_with_demoted(suffix) {
        Ok((kept, demoted)) => demoted.is_empty() && kept.len() == suffix_len,
        Err(_) => false,
    }
}

fn drop_leading_orphans(messages: &[Message], mut cut: usize) -> usize {
    while cut < messages.len() && is_leading_tool_result(&messages[cut]) {
        cut += 1;
    }
    cut
}

fn pending_keep_from(messages: &[Message], scope: &RequestScope) -> Option<usize> {
    let pending = scope.pending_prompt.as_ref()?;
    let (idx, last) = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| is_real_user_turn(message))?;
    if last != pending {
        return None;
    }
    Some(idx)
}

fn preferred_keep_from(
    messages: &[Message],
    groups: &[ToolGroup],
    protect_user_turns: usize,
    protect_tool_groups: usize,
) -> Option<usize> {
    let user_cut = nth_from_end(
        messages
            .iter()
            .enumerate()
            .filter(|(_, message)| is_real_user_turn(message))
            .map(|(idx, _)| idx),
        protect_user_turns,
    );
    let group_cut = nth_from_end(
        groups
            .iter()
            .filter(|group| group.complete)
            .map(|group| group.start),
        protect_tool_groups,
    );
    match (user_cut, group_cut) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn nth_from_end(indices: impl IntoIterator<Item = usize>, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let indices: Vec<usize> = indices.into_iter().collect();
    if indices.is_empty() {
        return None;
    }
    let skip = indices.len().saturating_sub(n);
    indices.get(skip).copied()
}

fn is_real_user_turn(message: &Message) -> bool {
    match message {
        Message::User { content } => content
            .iter()
            .any(|part| !matches!(part, UserContent::ToolResult(_))),
        _ => false,
    }
}

fn is_leading_tool_result(message: &Message) -> bool {
    match message {
        Message::User { content } => matches!(content.first(), Some(UserContent::ToolResult(_))),
        _ => false,
    }
}

fn assistant_tool_call_ids(message: &Message) -> Vec<&str> {
    match message {
        Message::Assistant { content, .. } => content
            .iter()
            .filter_map(|part| match part {
                AssistantContent::ToolCall(call) => Some(call.id.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn tool_result_ids(message: &Message) -> Vec<&str> {
    match message {
        Message::User { content } => content
            .iter()
            .filter_map(|part| match part {
                UserContent::ToolResult(result) => Some(result.call.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn has_unpaired_tool_result(kept: &[Message]) -> bool {
    let mut open = HashSet::new();
    for message in kept {
        for id in assistant_tool_call_ids(message) {
            open.insert(id.to_string());
        }
        for id in tool_result_ids(message) {
            if !open.remove(id) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::{AssistantContent, ToolResultContent, UserContent};
    use serde_json::json;

    fn unit_window(budget: usize) -> TokenWindowMemory {
        TokenWindowMemory::new(budget, |_: &Message| 1)
    }

    fn policy(budget: usize, scope: RequestScope) -> CodegContextPolicy {
        CodegContextPolicy::new(unit_window(budget), scope)
    }

    fn split(
        budget: usize,
        scope: RequestScope,
        messages: Vec<Message>,
    ) -> (Vec<Message>, Vec<Message>) {
        policy(budget, scope)
            .apply_with_demoted(messages)
            .expect("policy")
    }

    fn assert_prefix_suffix(original: &[Message], kept: &[Message], demoted: &[Message]) {
        assert_eq!(
            demoted.len() + kept.len(),
            original.len(),
            "kept+demoted length"
        );
        assert_eq!(demoted, &original[..demoted.len()], "demoted is prefix");
        assert_eq!(kept, &original[demoted.len()..], "kept is suffix");
    }

    fn tool_call(id: &str, name: &str) -> Message {
        Message::Assistant {
            id: None,
            content: vec![AssistantContent::tool_call(id, name, json!({}))],
        }
    }

    fn tool_calls(ids: &[&str], name: &str) -> Message {
        Message::Assistant {
            id: None,
            content: ids
                .iter()
                .map(|id| AssistantContent::tool_call(*id, name, json!({})))
                .collect(),
        }
    }

    fn tool_result(id: &str, name: &str) -> Message {
        Message::tool_result(id, name, "ok")
    }

    fn reasoning_and_call(id: &str, name: &str) -> Message {
        Message::Assistant {
            id: None,
            content: vec![
                AssistantContent::reasoning("plan the call"),
                AssistantContent::tool_call(id, name, json!({"x": 1})),
            ],
        }
    }

    fn mixed_sequence() -> Vec<Message> {
        vec![
            Message::user("goal-1"),
            Message::assistant("ack-1"),
            tool_calls(&["c1", "c2"], "bash"),
            tool_result("c1", "bash"),
            tool_result("c2", "bash"),
            Message::user("goal-2"),
            reasoning_and_call("c3", "read_file"),
            tool_result("c3", "read_file"),
            Message::user("goal-3"),
            Message::assistant("done"),
        ]
    }

    fn group_unsplit(
        original: &[Message],
        kept: &[Message],
        demoted: &[Message],
        group: &[Message],
    ) {
        let in_kept = group.iter().all(|m| kept.contains(m));
        let in_demoted = group.iter().all(|m| demoted.contains(m));
        assert!(
            in_kept || in_demoted,
            "group split: original={original:?} kept={kept:?} demoted={demoted:?}"
        );
        if in_kept {
            assert!(
                !group.iter().any(|m| demoted.contains(m)),
                "group partially demoted"
            );
        }
    }

    #[test]
    fn demoted_is_always_a_prefix_over_mixed_sequence() {
        let messages = mixed_sequence();
        for budget in 0..=messages.len() + 3 {
            for covers in [None, Some(1), Some(3), Some(messages.len()), Some(99)] {
                let scope = RequestScope {
                    covers_through_seq: covers,
                    ..RequestScope::default()
                };
                let original = messages.clone();
                let (kept, demoted) = split(budget, scope, messages.clone());
                assert_prefix_suffix(&original, &kept, &demoted);
                assert!(
                    !has_unpaired_tool_result(&kept),
                    "budget={budget} covers={covers:?} unpaired in kept={kept:?}"
                );
            }
        }
    }

    #[test]
    fn wrapping_preserves_token_window_prefix_suffix_on_text() {
        let messages = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
            Message::assistant("d"),
            Message::user("e"),
        ];
        for budget in 0..=messages.len() + 2 {
            let raw = unit_window(budget)
                .apply_with_demoted(messages.clone())
                .expect("token window");
            let wrapped = split(budget, RequestScope::default(), messages.clone());
            assert_prefix_suffix(&messages, &wrapped.0, &wrapped.1);
            assert_prefix_suffix(&messages, &raw.0, &raw.1);
            assert_eq!(
                wrapped, raw,
                "text session should match TokenWindowMemory at budget {budget}"
            );
        }
    }

    #[test]
    fn wrapping_too_small_window_still_returns_prefix_suffix() {
        let messages = mixed_sequence();
        let original = messages.clone();
        let (kept, demoted) = split(1, RequestScope::default(), messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert!(kept.len() <= 1);
    }

    #[test]
    fn multi_call_tool_group_is_never_split() {
        let group = vec![
            tool_calls(&["a", "b"], "bash"),
            tool_result("a", "bash"),
            tool_result("b", "bash"),
        ];
        let messages = vec![
            Message::user("old"),
            group[0].clone(),
            group[1].clone(),
            group[2].clone(),
            Message::user("next"),
        ];
        for budget in 0..=messages.len() + 1 {
            let original = messages.clone();
            let (kept, demoted) = split(budget, RequestScope::default(), messages.clone());
            assert_prefix_suffix(&original, &kept, &demoted);
            group_unsplit(&original, &kept, &demoted, &group);
        }
    }

    #[test]
    fn pending_prompt_and_incomplete_group_stay_in_kept() {
        let pending = Message::user("current task");
        let incomplete = tool_call("live", "bash");
        let messages = vec![
            Message::user("old-1"),
            Message::assistant("old-ack"),
            pending.clone(),
            incomplete.clone(),
        ];
        let scope = RequestScope {
            covers_through_seq: Some(2),
            pending_prompt: Some(pending.clone()),
            ..RequestScope::default()
        };
        let original = messages.clone();
        let (kept, demoted) = split(16, scope, messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert!(
            kept.iter().any(|m| m == &pending),
            "pending missing from kept={kept:?}"
        );
        assert!(
            kept.iter().any(|m| m == &incomplete),
            "incomplete group missing from kept={kept:?}"
        );
        assert_eq!(demoted.len(), 2);
    }

    #[test]
    fn pending_that_does_not_fit_returns_context_budget_exceeded() {
        let pending = Message::user("current task");
        let messages = vec![
            Message::user("old"),
            pending.clone(),
            tool_call("live", "bash"),
        ];
        let scope = RequestScope {
            pending_prompt: Some(pending),
            ..RequestScope::default()
        };
        let err = policy(1, scope)
            .apply_with_demoted(messages)
            .expect_err("pending suffix exceeds budget");
        assert!(err.to_string().contains("context_budget_exceeded"), "{err}");
    }

    #[test]
    fn covers_through_seq_does_not_return_summarized_prefix_to_kept() {
        let messages = mixed_sequence();
        let scope = RequestScope {
            covers_through_seq: Some(4),
            ..RequestScope::default()
        };
        let original = messages.clone();
        let (kept, demoted) = split(usize::MAX, scope, messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert!(
            demoted.len() >= 4,
            "summarized prefix leaked into kept: demoted={} kept={kept:?}",
            demoted.len()
        );
        assert_eq!(&demoted[..4], &original[..4]);
        assert_eq!(kept, &original[demoted.len()..]);
    }

    #[test]
    fn covers_through_seq_demotes_whole_group_instead_of_summarized_keep() {
        let messages = vec![
            Message::user("old"),
            tool_calls(&["a", "b"], "bash"),
            tool_result("a", "bash"),
            tool_result("b", "bash"),
            Message::user("next"),
        ];
        let scope = RequestScope {
            covers_through_seq: Some(3),
            ..RequestScope::default()
        };
        let original = messages.clone();
        let (kept, demoted) = split(usize::MAX, scope, messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert_eq!(kept, vec![Message::user("next")]);
        assert_eq!(demoted.len(), 4);
    }

    #[test]
    fn leading_orphan_tool_result_is_not_kept_unpaired() {
        let messages = vec![
            tool_call("c1", "bash"),
            tool_result("c1", "bash"),
            Message::user("after"),
        ];
        let original = messages.clone();
        let (kept, demoted) = split(2, RequestScope::default(), messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert_eq!(kept, vec![Message::user("after")]);
        assert!(!has_unpaired_tool_result(&kept));
        assert!(
            !kept
                .iter()
                .any(|m| is_leading_tool_result(m) && assistant_tool_call_ids(m).is_empty()),
            "orphan tool result stayed in kept={kept:?}"
        );
    }

    #[test]
    fn leading_orphan_at_start_of_full_window_is_demoted() {
        let messages = vec![
            tool_result("orphan", "bash"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        let original = messages.clone();
        let (kept, demoted) = split(8, RequestScope::default(), messages);
        assert_prefix_suffix(&original, &kept, &demoted);
        assert!(!kept.is_empty());
        assert!(!is_leading_tool_result(&kept[0]));
        assert!(is_leading_tool_result(&demoted[0]));
    }

    #[test]
    fn reasoning_and_toolcall_in_same_assistant_stay_together() {
        let asst = reasoning_and_call("r1", "bash");
        let messages = vec![
            Message::user("old"),
            asst.clone(),
            tool_result("r1", "bash"),
            Message::user("next"),
        ];
        for budget in 0..=messages.len() + 1 {
            let original = messages.clone();
            let (kept, demoted) = split(budget, RequestScope::default(), messages.clone());
            assert_prefix_suffix(&original, &kept, &demoted);
            let in_kept = kept.iter().any(|m| m == &asst);
            let in_demoted = demoted.iter().any(|m| m == &asst);
            assert!(in_kept ^ in_demoted, "assistant missing at budget {budget}");
            if in_kept {
                let msg = kept.iter().find(|m| *m == &asst).expect("asst");
                match msg {
                    Message::Assistant { content, .. } => {
                        assert!(content
                            .iter()
                            .any(|c| matches!(c, AssistantContent::Reasoning(_))));
                        assert!(content
                            .iter()
                            .any(|c| matches!(c, AssistantContent::ToolCall(_))));
                    }
                    _ => panic!("expected assistant"),
                }
            }
        }
    }

    #[test]
    fn safety_margin_matches_budget_config() {
        assert_eq!(safety_margin(10_000), 1024);
        assert_eq!(safety_margin(128_000), 6400);
        assert_eq!(
            input_budget(128_000, 4096).expect("budget"),
            128_000 - 4096 - 6400
        );
    }

    #[test]
    fn combined_tool_results_in_one_user_message_stay_with_calls() {
        let asst = tool_calls(&["a", "b"], "bash");
        let results = Message::User {
            content: vec![
                UserContent::tool_result("a", "bash", vec![ToolResultContent::text("ok-a")]),
                UserContent::tool_result("b", "bash", vec![ToolResultContent::text("ok-b")]),
            ],
        };
        let messages = vec![
            Message::user("old"),
            asst.clone(),
            results.clone(),
            Message::user("next"),
        ];
        for budget in 0..=4 {
            let original = messages.clone();
            let (kept, demoted) = split(budget, RequestScope::default(), messages.clone());
            assert_prefix_suffix(&original, &kept, &demoted);
            group_unsplit(&original, &kept, &demoted, &[asst.clone(), results.clone()]);
        }
    }

    #[test]
    fn token_window_constructor_uses_heuristic_counter() {
        let pending = Message::user("short");
        let messages = vec![Message::user("old"), pending.clone()];
        let scope = RequestScope {
            pending_prompt: Some(pending),
            ..RequestScope::default()
        };
        let policy = CodegContextPolicy::token_window(10_000, scope);
        let (kept, demoted) = policy.apply_with_demoted(messages.clone()).expect("apply");
        assert_prefix_suffix(&messages, &kept, &demoted);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn bind_scope_updates_pending_in_place() {
        let pending = Message::user("current task");
        let messages = vec![
            Message::user("old"),
            pending.clone(),
            tool_call("live", "bash"),
        ];
        let policy = CodegContextPolicy::new(unit_window(16), RequestScope::default());
        let (kept_before, _) = policy
            .apply_with_demoted(messages.clone())
            .expect("unbound");
        assert_eq!(kept_before.len(), 3);

        policy.bind_scope(RequestScope {
            covers_through_seq: Some(1),
            pending_prompt: Some(pending.clone()),
            ..RequestScope::default()
        });
        assert_eq!(policy.scope().pending_prompt.as_ref(), Some(&pending));
        let original = messages.clone();
        let (kept, demoted) = policy.apply_with_demoted(messages).expect("bound");
        assert_prefix_suffix(&original, &kept, &demoted);
        assert_eq!(demoted.len(), 1);
        assert!(kept.iter().any(|m| m == &pending));
    }
}

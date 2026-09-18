//! Per-request W/O/S gate. A patch is non-sticky: recompute on every HTTP call.

use rig::agent::RequestPatch;
use rig::completion::Message;
use serde_json::Value;

/// Default cap on a single tool presentation sent back to the model.
pub const MAX_TOOL_PRESENTATION_BYTES: usize = 32 * 1024;
/// Soft target: keep this many most-recent turns before lite compact.
pub const RECENT_TURN_TARGET: usize = 6;
/// Extra tokens counted per serialized message for role/envelope overhead.
const MESSAGE_WRAP: u64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetConfig {
    pub window: u64,
    pub max_output: u64,
    pub compact_soft_percent: u8,
    pub compact_recent_turns: usize,
}

impl BudgetConfig {
    pub const fn new(window: u64, max_output: u64) -> Self {
        Self {
            window,
            max_output,
            compact_soft_percent: 80,
            compact_recent_turns: RECENT_TURN_TARGET,
        }
    }

    pub const fn with_compact(mut self, percent: u8, recent_turns: usize) -> Self {
        self.compact_soft_percent = if percent == 0 {
            80
        } else if percent > 100 {
            100
        } else {
            percent
        };
        self.compact_recent_turns = if recent_turns == 0 {
            RECENT_TURN_TARGET
        } else {
            recent_turns
        };
        self
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("context_budget: window {window} cannot fit output {max_output} plus safety {safety}")]
    WindowTooSmall {
        window: u64,
        max_output: u64,
        safety: u64,
    },
    #[error("context_budget: current prompt and required context exceed input budget {budget}")]
    PromptExceedsBudget { budget: u64, estimated: u64 },
    #[error("context_budget: minimal legal history still exceeds input budget {budget}")]
    HistoryExceedsBudget { budget: u64, estimated: u64 },
}

impl BudgetConfig {
    pub fn safety_margin(self) -> u64 {
        let five_pct = self.window.saturating_mul(5).div_ceil(100);
        five_pct.max(1024)
    }

    pub fn input_budget(self) -> Result<u64, BudgetError> {
        let safety = self.safety_margin();
        let used = self.max_output.saturating_add(safety);
        if self.window <= used {
            return Err(BudgetError::WindowTooSmall {
                window: self.window,
                max_output: self.max_output,
                safety,
            });
        }
        Ok(self.window - used)
    }
}

/// Build a per-call [`RequestPatch`]. It does not mutate Agent baseline or
/// Runner persisted history; the next model request must apply a new patch.
pub fn per_call_patch(history: Vec<Message>, max_tokens: Option<u64>) -> RequestPatch {
    let mut patch = RequestPatch::new().history(history);
    if let Some(max_tokens) = max_tokens {
        patch = patch.max_tokens(max_tokens);
    }
    patch
}

/// Conservative request-size estimate: UTF-8 byte length of the serialized
/// payload plus a per-message envelope. Not `chars/4`.
pub fn estimate_tokens_bytes(text: &str) -> u64 {
    text.len() as u64
}

pub fn estimate_json(value: &impl serde::Serialize) -> u64 {
    serde_json::to_vec(value)
        .map(|v| v.len() as u64)
        .unwrap_or(0)
}

pub fn estimate_request(
    preamble: &str,
    tool_schemas: &[Value],
    history: &[Message],
    prompt: &Message,
) -> u64 {
    let mut n = estimate_tokens_bytes(preamble) + MESSAGE_WRAP;
    for schema in tool_schemas {
        n = n.saturating_add(estimate_json(schema));
    }
    for message in history {
        n = n
            .saturating_add(estimate_json(message))
            .saturating_add(MESSAGE_WRAP);
    }
    n.saturating_add(estimate_json(prompt))
        .saturating_add(MESSAGE_WRAP)
}

/// Truncate a tool presentation to 32KiB at a line/char boundary.
///
/// Overall request budget is [`estimate_request`] plus [`BudgetConfig::input_budget`].
/// Shrinking a single page to the leftover input bytes ate continuation hints
/// (`pass offset=N`) and told the model the rest of the file was gone.
pub fn truncate_presentation(raw: &str, remaining_input_bytes: u64) -> (String, bool) {
    let _ = remaining_input_bytes;
    let cap = MAX_TOOL_PRESENTATION_BYTES;
    if raw.len() <= cap {
        return (raw.to_string(), false);
    }
    let keep = cap.saturating_sub(160);
    let mut end = keep.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(i) = raw[..end].rfind('\n') {
        end = i;
    }
    let omitted = raw.len().saturating_sub(end);
    (
        format!(
            "{}\n[truncated: {omitted} bytes omitted; recall this tool_call_id or re-read with a higher offset to continue]",
            &raw[..end]
        ),
        true,
    )
}

/// Fail closed when preamble + tools + history + prompt cannot fit the input budget.
pub fn check_request_budget(
    config: BudgetConfig,
    preamble: &str,
    tool_schemas: &[Value],
    history: &[Message],
    prompt: &Message,
) -> Result<u64, BudgetError> {
    let budget = config.input_budget()?;
    let estimated = estimate_request(preamble, tool_schemas, history, prompt);
    if estimated >= budget {
        if history.is_empty() {
            Err(BudgetError::PromptExceedsBudget { budget, estimated })
        } else {
            Err(BudgetError::HistoryExceedsBudget { budget, estimated })
        }
    } else {
        Ok(estimated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(window: u64, output: u64) -> BudgetConfig {
        BudgetConfig::new(window, output)
    }

    #[test]
    fn safety_margin_is_max_of_1024_and_five_percent() {
        assert_eq!(cfg(10_000, 100).safety_margin(), 1024);
        assert_eq!(cfg(128_000, 4096).safety_margin(), 6400);
    }

    #[test]
    fn estimate_is_utf8_bytes_not_chars_div_4() {
        let text = "a".repeat(4000);
        assert_eq!(estimate_tokens_bytes(&text), 4000);
        assert_ne!(estimate_tokens_bytes(&text), 1000);
    }

    #[test]
    fn one_megabyte_tool_output_is_truncated_before_the_gate() {
        let raw = "x".repeat(1_000_000);
        let (shown, truncated) = truncate_presentation(&raw, 50_000);
        assert!(truncated);
        assert!(shown.len() < 40_000);
        assert!(
            shown.contains("recall") || shown.contains("pass offset="),
            "truncation must tell the model how to get the rest: {shown}"
        );
        assert!(
            !shown.contains("remaining output unavailable"),
            "must not claim the rest of the output is gone: {shown}"
        );
    }

    #[test]
    fn a_read_page_under_32kib_is_not_eaten_by_a_tight_remaining_budget() {
        let mut raw = String::from("# f.rs (lines 1-400)\n");
        raw.push_str(&"line-body\n".repeat(400));
        raw.push_str("[truncated: showing 400 lines; pass offset=401 to continue from this path]");
        assert!(raw.len() < MAX_TOOL_PRESENTATION_BYTES);
        let (shown, truncated) = truncate_presentation(&raw, 1024);
        assert!(
            !truncated,
            "32KiB page must survive a tight remaining budget"
        );
        assert_eq!(shown, raw);
        assert!(shown.contains("pass offset=401"), "{shown}");
        assert!(!shown.contains("remaining output unavailable"), "{shown}");
    }

    #[test]
    fn truncate_presentation_stays_on_a_char_boundary() {
        let raw = format!("a{}", "你".repeat(20_000));
        let (shown, truncated) = truncate_presentation(&raw, 1024);
        assert!(truncated);
        assert!(shown.is_char_boundary(shown.len()), "len {}", shown.len());
        assert!(!shown.contains("remaining output unavailable"), "{shown}");
    }

    #[test]
    fn estimate_request_counts_preamble_history_and_prompt() {
        let prompt = Message::user("hello");
        let history = vec![Message::user("prior"), Message::assistant("noted")];
        let estimated = estimate_request("preamble", &[], &history, &prompt);
        assert!(estimated > estimate_tokens_bytes("preamble"));
        assert!(estimated > estimate_json(&prompt));
    }

    #[test]
    fn empty_history_prompt_that_exceeds_input_budget_is_rejected() {
        let prompt = Message::user("x".repeat(5000));
        let err = check_request_budget(cfg(4096, 2048), "p", &[], &[], &prompt).unwrap_err();
        assert!(
            matches!(
                err,
                BudgetError::WindowTooSmall { .. } | BudgetError::PromptExceedsBudget { .. }
            ),
            "{err}"
        );
    }

    #[test]
    fn too_small_window_cannot_fit_output_and_safety() {
        let prompt = Message::user("x");
        let err = check_request_budget(cfg(2048, 2048), "p", &[], &[], &prompt).unwrap_err();
        assert!(matches!(err, BudgetError::WindowTooSmall { .. }), "{err}");
    }
}

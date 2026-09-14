//! Per-request W/O/S gate. A patch is non-sticky: recompute on every HTTP call.

use rig::agent::RequestPatch;
use rig::completion::Message;
use serde_json::Value;

use super::store::{AssistantPart, CanonicalTurn, ContextStore, ContextView, ExecutionFact};

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

/// Truncate a tool presentation to 32KiB and the remaining input budget.
pub fn truncate_presentation(raw: &str, remaining_input_bytes: u64) -> (String, bool) {
    let cap = MAX_TOOL_PRESENTATION_BYTES.min(remaining_input_bytes as usize);
    if raw.len() <= cap {
        return (raw.to_string(), false);
    }
    let keep = cap.saturating_sub(80);
    let omitted = raw.len().saturating_sub(keep);
    (
        format!(
            "{}\n[truncated: {omitted} bytes omitted; remaining output unavailable]",
            &raw[..keep]
        ),
        true,
    )
}

pub struct BudgetInputs<'a> {
    pub store: &'a ContextStore,
    pub config: BudgetConfig,
    pub preamble: &'a str,
    pub tool_schemas: &'a [Value],
    pub prompt: &'a Message,
}

/// Project canonical facts into a request-sized history. L1 template compact
/// may run; L2 LLM compact is [`super::compact::project_compacted`].
pub async fn project_view(inputs: BudgetInputs<'_>) -> Result<ContextView, BudgetError> {
    super::compact::project_view(inputs).await
}

pub(crate) fn messages_from_turns(turns: &[CanonicalTurn], store: &ContextStore) -> Vec<Message> {
    let mut out = Vec::new();
    for turn in turns {
        if !turn.user_text.is_empty() {
            out.push(Message::user(turn.user_text.clone()));
        }
        if let Some(asst) = &turn.assistant {
            if !asst.committed {
                continue;
            }
            let mut content = Vec::new();
            let mut call_ids = Vec::new();
            for part in &asst.parts {
                match part {
                    AssistantPart::Text(text) if !text.is_empty() => {
                        content.push(rig::completion::message::AssistantContent::text(text));
                    }
                    AssistantPart::ToolCall { id, name, args } => {
                        call_ids.push((id.clone(), name.clone()));
                        let call = rig::completion::message::ToolCall::new(
                            rig::completion::message::ToolCallId::new_or_mint(id.clone()),
                            rig::completion::message::ToolFunction::new(name.clone(), args.clone()),
                        );
                        content.push(rig::completion::message::AssistantContent::ToolCall(call));
                    }
                    AssistantPart::Text(_) => {}
                }
            }
            if !content.is_empty() {
                out.push(Message::Assistant {
                    id: asst.model_message_id.clone(),
                    content,
                });
            }
            let results: Vec<_> = call_ids
                .into_iter()
                .filter_map(|(id, name)| {
                    store
                        .fact(&id)
                        .map(|fact| tool_result_message(&id, &name, fact))
                })
                .collect();
            if !results.is_empty() {
                out.push(Message::User { content: results });
            }
        }
    }
    out
}

fn tool_result_message(
    id: &str,
    name: &str,
    fact: &ExecutionFact,
) -> rig::completion::message::UserContent {
    let body = fact
        .model_presentation
        .clone()
        .unwrap_or_else(|| fact.outcome_feedback());
    rig::completion::message::UserContent::tool_result(
        id,
        name,
        vec![rig::completion::message::ToolResultContent::text(body)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::store::{AssistantRecord, ContextStore};
    use crate::agent::context::{ToolOutcome, ToolPhase};

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
        assert!(shown.contains("remaining output unavailable"));
    }

    #[tokio::test]
    async fn lite_projection_drops_old_turns_and_does_not_keep_originals() {
        let mut store = ContextStore::new("s");
        for i in 0..8 {
            store.append_user(
                format!("s:{i}"),
                format!("turn-{i}-{}", "word ".repeat(200)),
            );
            store.commit_assistant(
                &format!("s:{i}"),
                AssistantRecord {
                    model_message_id: Some(format!("m{i}")),
                    committed: true,
                    parts: vec![AssistantPart::Text(format!("reply-{i}"))],
                },
            );
        }
        let prompt = Message::user("current question");
        let view = project_view(BudgetInputs {
            store: &store,
            config: cfg(8_000, 1024),
            preamble: "short",
            tool_schemas: &[],
            prompt: &prompt,
        })
        .await
        .expect("fits after compact");
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(
            dumped.contains("Conversation summary") || dumped.contains("[omitted:"),
            "{dumped}"
        );
        assert_eq!(view.compact_level, 1);
        assert!(
            !dumped.contains("\"id\":\"m0\""),
            "evicted assistant turns must not remain as live history: {dumped}"
        );
        assert_eq!(store.turns().len(), 8, "JSONL turns are not deleted");
        // RequestPatch cannot rewrite the current prompt; it stays on the runner.
        assert_eq!(prompt, Message::user("current question"));
    }

    #[tokio::test]
    async fn current_tool_pair_is_kept_when_old_turns_are_dropped() {
        let mut store = ContextStore::new("s");
        store.append_user("s:0".into(), "old ".repeat(400));
        store.commit_assistant(
            "s:0",
            AssistantRecord {
                model_message_id: Some("old".into()),
                committed: true,
                parts: vec![AssistantPart::Text("old-reply".into())],
            },
        );
        store.append_user("s:1".into(), "please write".into());
        store.record_fact(ExecutionFact {
            tool_call_id: "call_a".into(),
            function_name: "write_mem".into(),
            raw_input: serde_json::json!({"text": "A"}),
            phase: ToolPhase::Terminal,
            outcome: Some(ToolOutcome::Success),
            executed: Some(true),
            model_presentation: Some("wrote A".into()),
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: "s:1".into(),
        });
        store.commit_assistant(
            "s:1",
            AssistantRecord {
                model_message_id: Some("m1".into()),
                committed: true,
                parts: vec![AssistantPart::ToolCall {
                    id: "call_a".into(),
                    name: "write_mem".into(),
                    args: serde_json::json!({"text": "A"}),
                }],
            },
        );
        let prompt = Message::user("continue");
        let view = project_view(BudgetInputs {
            store: &store,
            config: cfg(6_000, 1024),
            preamble: "p",
            tool_schemas: &[],
            prompt: &prompt,
        })
        .await
        .expect("pair retained");
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(dumped.contains("call_a"), "{dumped}");
        assert!(dumped.contains("write_mem"), "{dumped}");
        assert!(dumped.contains("wrote A"), "{dumped}");
    }

    #[tokio::test]
    async fn too_small_window_stops_instead_of_sending() {
        let store = ContextStore::new("s");
        let prompt = Message::user("x".repeat(5000));
        let err = project_view(BudgetInputs {
            store: &store,
            config: cfg(4096, 2048),
            preamble: "p",
            tool_schemas: &[],
            prompt: &prompt,
        })
        .await
        .unwrap_err();
        assert!(
            matches!(
                err,
                BudgetError::WindowTooSmall { .. } | BudgetError::PromptExceedsBudget { .. }
            ),
            "{err}"
        );
    }
}

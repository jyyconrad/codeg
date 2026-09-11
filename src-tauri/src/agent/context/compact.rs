//! Two-level context compression using rig-memory Compactor / MemoryPolicy.
//!
//! L1 is [`TemplateCompactor`] (no HTTP). L2 is [`LlmCompactor`] on the second
//! session trigger. Never call `AgentBuilder::memory()` — transcript is truth.

use rig::client::CompletionClient;
use rig::completion::{AssistantContent, CompletionModel, Message};
use rig::providers::openai::CompletionsClient;
use rig_memory::{Compactor, MemoryError, MemoryPolicy, SlidingWindowMemory, TemplateCompactor};

use super::budget::{
    estimate_request, messages_from_turns, BudgetConfig, BudgetError, BudgetInputs,
    RECENT_TURN_TARGET,
};
use super::store::{CanonicalTurn, CompactRecord, ContextStore, ContextView, UsageSource};
use crate::acp_transcript::now_epoch_ms;

/// Cap on L2 compact `max_tokens` (min of this and the session setting).
pub const L2_MAX_TOKENS: u64 = 2048;
const L1_SUMMARY_MAX_BYTES: usize = 8 * 1024;

/// Artifact produced by [`LlmCompactor`].
#[derive(Clone, Debug)]
pub struct CompactArtifact(pub String);

impl From<CompactArtifact> for Message {
    fn from(value: CompactArtifact) -> Self {
        Message::user(value.0)
    }
}

/// L2 LLM summarizer. Completions only: no tools, no fake compaction card.
#[derive(Clone)]
pub struct LlmCompactor {
    client: CompletionsClient,
    model_id: String,
    compact_prompt: String,
    max_tokens: u64,
}

impl LlmCompactor {
    pub fn new(
        client: CompletionsClient,
        model_id: impl Into<String>,
        compact_prompt: impl Into<String>,
        max_tokens: u64,
    ) -> Self {
        Self {
            client,
            model_id: model_id.into(),
            compact_prompt: compact_prompt.into(),
            max_tokens: max_tokens.clamp(1, L2_MAX_TOKENS),
        }
    }

    pub fn max_tokens(&self) -> u64 {
        self.max_tokens
    }

    pub fn compact_prompt(&self) -> &str {
        &self.compact_prompt
    }

    async fn summarize(
        &self,
        evicted: &[Message],
        carry_over: Option<&str>,
    ) -> Result<String, String> {
        let mut body = String::new();
        if let Some(prev) = carry_over.map(str::trim).filter(|s| !s.is_empty()) {
            body.push_str("Previous summary:\n");
            body.push_str(prev);
            body.push_str("\n\n");
        }
        body.push_str("Evicted turns:\n");
        body.push_str(&messages_as_text(evicted));
        let response = self
            .client
            .completion_model(&self.model_id)
            .completion_request(Message::user(body))
            .preamble(self.compact_prompt.clone())
            .max_tokens(self.max_tokens)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let text = choice_text(&response);
        if text.trim().is_empty() {
            return Err("empty compact summary".into());
        }
        Ok(text)
    }
}

impl Compactor for LlmCompactor {
    type Artifact = CompactArtifact;

    fn compact<'a>(
        &'a self,
        _conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            self.summarize(evicted, carry_over.map(|a| a.0.as_str()))
                .await
                .map(CompactArtifact)
                .map_err(MemoryError::Internal)
        })
    }
}

/// Project canonical facts. At most one compact-level upgrade per call.
///
/// Returns a new [`CompactRecord`] when this call created L1 or L2. Original
/// turns stay in the store; the caller persists the record to JSONL.
pub async fn project_compacted(
    inputs: BudgetInputs<'_>,
    llm: Option<&LlmCompactor>,
) -> Result<(ContextView, Option<CompactRecord>), BudgetError> {
    let budget = inputs.config.input_budget()?;
    let prompt_cost = estimate_request(inputs.preamble, inputs.tool_schemas, &[], inputs.prompt);
    if prompt_cost >= budget {
        return Err(BudgetError::PromptExceedsBudget {
            budget,
            estimated: prompt_cost,
        });
    }

    let existing = inputs.store.compact().cloned();
    let level = existing.as_ref().map(|r| r.level).unwrap_or(0);
    let mut live = live_turns(inputs.store, existing.as_ref()).to_vec();
    let mut newly_evicted: Vec<CanonicalTurn> = Vec::new();
    let omitted_at_80 = (budget * 80) / 100;
    let summary_text = existing.as_ref().map(|r| r.summary.clone());

    loop {
        let history = history_with_summary(&live, inputs.store, summary_text.as_deref());
        let estimated = estimate_request(
            inputs.preamble,
            inputs.tool_schemas,
            &history,
            inputs.prompt,
        );
        let over_hard = estimated > budget;
        let over_soft = estimated >= omitted_at_80 && live.len() > RECENT_TURN_TARGET;
        let want_upgrade = matches!(level, 0 | 1) && (over_hard || over_soft);
        if !want_upgrade {
            return hard_drop_view(inputs, budget, live, summary_text.as_deref(), level, None);
        }
        if live.len() <= 1 {
            if over_hard {
                return Err(BudgetError::HistoryExceedsBudget { budget, estimated });
            }
            return Ok((
                view_from(
                    history,
                    omitted_count(inputs.store, &live),
                    estimated,
                    inputs.config,
                    budget,
                    level,
                ),
                None,
            ));
        }
        newly_evicted.push(live.remove(0));
        let trial = history_with_summary(&live, inputs.store, summary_text.as_deref());
        let trial_est =
            estimate_request(inputs.preamble, inputs.tool_schemas, &trial, inputs.prompt);
        let trial_hard = trial_est > budget;
        let trial_soft = trial_est >= omitted_at_80 && live.len() > RECENT_TURN_TARGET;
        if !trial_hard && !trial_soft {
            break;
        }
        if live.len() <= 1 {
            break;
        }
    }

    upgrade_one_level(inputs, budget, live, newly_evicted, existing, llm).await
}

/// Lite / L1 projection (no L2 HTTP).
pub async fn project_view(inputs: BudgetInputs<'_>) -> Result<ContextView, BudgetError> {
    project_compacted(inputs, None).await.map(|(view, _)| view)
}

async fn upgrade_one_level(
    inputs: BudgetInputs<'_>,
    budget: u64,
    live: Vec<CanonicalTurn>,
    newly_evicted: Vec<CanonicalTurn>,
    existing: Option<CompactRecord>,
    llm: Option<&LlmCompactor>,
) -> Result<(ContextView, Option<CompactRecord>), BudgetError> {
    let level = existing.as_ref().map(|r| r.level).unwrap_or(0);
    if newly_evicted.is_empty() {
        return hard_drop_view(
            inputs,
            budget,
            live,
            existing.as_ref().map(|r| r.summary.as_str()),
            level,
            None,
        );
    }

    let through_turn = newly_evicted
        .last()
        .map(|t| t.turn_id.clone())
        .unwrap_or_default();
    let (evicted_msgs, _kept_msgs) = split_policy_messages(&newly_evicted, &live, inputs.store);

    if level == 0 {
        let summary = template_summary(inputs.store.session_id(), &evicted_msgs).await;
        let record = CompactRecord {
            level: 1,
            through_turn,
            summary,
            created_at_ms: now_epoch_ms(),
        };
        return hard_drop_view(inputs, budget, live, None, 1, Some(record));
    }

    if level == 1 {
        let carry = existing
            .as_ref()
            .map(|r| CompactArtifact(r.summary.clone()));
        let Some(compactor) = llm else {
            return keep_l1(inputs, budget, live, existing);
        };
        tracing::info!("codeg agent compacting context with LLM");
        match Compactor::compact(
            compactor,
            inputs.store.session_id(),
            &evicted_msgs,
            carry.as_ref(),
        )
        .await
        {
            Ok(artifact) if !artifact.0.trim().is_empty() => {
                let record = CompactRecord {
                    level: 2,
                    through_turn,
                    summary: artifact.0,
                    created_at_ms: now_epoch_ms(),
                };
                hard_drop_view(inputs, budget, live, None, 2, Some(record))
            }
            Ok(_) => {
                tracing::warn!("L2 compact returned empty text; retaining L1");
                keep_l1(inputs, budget, live, existing)
            }
            Err(err) => {
                tracing::warn!(error = %err, "L2 compact failed; retaining L1");
                keep_l1(inputs, budget, live, existing)
            }
        }
    } else {
        hard_drop_view(
            inputs,
            budget,
            live,
            existing.as_ref().map(|r| r.summary.as_str()),
            level,
            None,
        )
    }
}

fn keep_l1(
    inputs: BudgetInputs<'_>,
    budget: u64,
    live: Vec<CanonicalTurn>,
    existing: Option<CompactRecord>,
) -> Result<(ContextView, Option<CompactRecord>), BudgetError> {
    hard_drop_view(
        inputs,
        budget,
        live,
        existing.as_ref().map(|r| r.summary.as_str()),
        existing.as_ref().map(|r| r.level).unwrap_or(1),
        None,
    )
}

fn hard_drop_view(
    inputs: BudgetInputs<'_>,
    budget: u64,
    mut live: Vec<CanonicalTurn>,
    summary: Option<&str>,
    level: u8,
    new_record: Option<CompactRecord>,
) -> Result<(ContextView, Option<CompactRecord>), BudgetError> {
    let summary_owned = new_record
        .as_ref()
        .map(|r| r.summary.clone())
        .or_else(|| summary.map(str::to_string));
    let compact_level = new_record.as_ref().map(|r| r.level).unwrap_or(level);
    loop {
        let history = history_with_summary(&live, inputs.store, summary_owned.as_deref());
        let estimated = estimate_request(
            inputs.preamble,
            inputs.tool_schemas,
            &history,
            inputs.prompt,
        );
        if estimated <= budget {
            return Ok((
                view_from(
                    history,
                    omitted_count(inputs.store, &live),
                    estimated,
                    inputs.config,
                    budget,
                    compact_level,
                ),
                new_record,
            ));
        }
        if live.len() <= 1 {
            return Err(BudgetError::HistoryExceedsBudget { budget, estimated });
        }
        live.remove(0);
    }
}

fn view_from(
    messages: Vec<Message>,
    omitted_turns: usize,
    estimated: u64,
    config: BudgetConfig,
    budget: u64,
    compact_level: u8,
) -> ContextView {
    ContextView {
        messages,
        omitted_turns,
        estimated_tokens: estimated,
        window: config.window,
        input_budget: budget,
        source: UsageSource::Estimated,
        compact_level,
    }
}

fn live_turns<'a>(store: &'a ContextStore, compact: Option<&CompactRecord>) -> &'a [CanonicalTurn] {
    let turns = store.turns();
    let Some(record) = compact else {
        return turns;
    };
    match turns.iter().position(|t| t.turn_id == record.through_turn) {
        Some(idx) => &turns[idx + 1..],
        None => turns,
    }
}

fn omitted_count(store: &ContextStore, live: &[CanonicalTurn]) -> usize {
    store.turns().len().saturating_sub(live.len())
}

fn history_with_summary(
    live: &[CanonicalTurn],
    store: &ContextStore,
    summary: Option<&str>,
) -> Vec<Message> {
    let mut out = Vec::new();
    if let Some(summary) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(Message::user(summary.to_string()));
    }
    out.extend(messages_from_turns(live, store));
    out
}

fn split_policy_messages(
    evicted_turns: &[CanonicalTurn],
    kept_turns: &[CanonicalTurn],
    store: &ContextStore,
) -> (Vec<Message>, Vec<Message>) {
    let mut all = messages_from_turns(evicted_turns, store);
    let kept = messages_from_turns(kept_turns, store);
    all.extend(kept.clone());
    let keep_n = kept.len().max(1);
    match SlidingWindowMemory::last_messages(keep_n).apply_with_demoted(all) {
        Ok((kept_msgs, evicted_msgs)) => (evicted_msgs, kept_msgs),
        Err(_) => (
            messages_from_turns(evicted_turns, store),
            messages_from_turns(kept_turns, store),
        ),
    }
}

async fn template_summary(session_id: &str, evicted: &[Message]) -> String {
    let compactor = TemplateCompactor::new().with_max_bytes(L1_SUMMARY_MAX_BYTES);
    match compactor.compact(session_id, evicted, None).await {
        Ok(artifact) => artifact.into_string(),
        Err(_) => format!("[omitted: {} turns compacted]", evicted.len().max(1)),
    }
}

fn messages_as_text(messages: &[Message]) -> String {
    serde_json::to_string(messages).unwrap_or_else(|_| format!("{messages:?}"))
}

fn choice_text(response: &rig::completion::CompletionResponse) -> String {
    response
        .choice
        .iter()
        .filter_map(|part| match part {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::store::{AssistantPart, AssistantRecord, ContextStore};
    use crate::agent::model::completions_client;
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use rig::completion::Message;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    fn cfg(window: u64, output: u64) -> BudgetConfig {
        BudgetConfig {
            window,
            max_output: output,
        }
    }

    fn fill_turns(store: &mut ContextStore, start: usize, n: usize, pad: usize) {
        for i in start..start + n {
            store.append_user(
                format!("s:{i}"),
                format!("turn-{i}-{}", "word ".repeat(pad)),
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
    }

    async fn spawn_json_completions(script: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(script));
        let state_bodies = bodies.clone();
        let state_script = script.clone();
        let app = Router::new().fallback(post(move |uri: Uri, Json(body): Json<Value>| {
            let bodies = state_bodies.clone();
            let script = state_script.clone();
            async move {
                let _ = uri;
                bodies.lock().expect("bodies").push(body);
                let next = {
                    let mut script = script.lock().expect("script");
                    if script.is_empty() {
                        json!({"ok": false})
                    } else {
                        script.remove(0)
                    }
                };
                if next.get("fail").and_then(Value::as_bool) == Some(true) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [(header::CONTENT_TYPE, "application/json")],
                        r#"{"error":"compact failed"}"#.to_string(),
                    )
                        .into_response();
                }
                let text = next
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("L2-SUMMARY");
                let payload = json!({
                    "id": "chatcmpl-compact",
                    "object": "chat.completion",
                    "created": 1,
                    "model": "m",
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": text },
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 20, "completion_tokens": 8, "total_tokens": 28 }
                });
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    payload.to_string(),
                )
                    .into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/v1"), bodies)
    }

    fn llm(base: &str, prompt: &str, max_tokens: u64) -> LlmCompactor {
        let client = completions_client("sk-test", base).expect("client");
        LlmCompactor::new(client, "m", prompt, max_tokens)
    }

    #[tokio::test]
    async fn l1_template_compactor_does_not_call_http() {
        let mut store = ContextStore::new("s");
        fill_turns(&mut store, 0, 8, 200);
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "SHOULD-NOT-RUN"})]).await;
        let compact = llm(&base, "CODEG_AGENT_COMPACT_PROMPT marker", 4096);
        let prompt = Message::user("current question");
        let (view, record) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("l1");
        let rec = record.expect("l1 record");
        assert_eq!(rec.level, 1);
        assert_eq!(view.compact_level, 1);
        assert_eq!(store.turns().len(), 8, "JSONL turns stay in the store");
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(
            dumped.contains("Conversation summary") || dumped.contains("[omitted:"),
            "{dumped}"
        );
        assert!(
            !dumped.contains("\"id\":\"m0\""),
            "evicted assistant turns must not remain as live history: {dumped}"
        );
        assert!(
            bodies.lock().expect("bodies").is_empty(),
            "L1 must not call Completions: {:?}",
            bodies.lock().expect("bodies")
        );
    }

    #[tokio::test]
    async fn l2_makes_one_compact_call_and_resume_reuses_record() {
        let mut store = ContextStore::new("s");
        fill_turns(&mut store, 0, 8, 200);
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "L2-SUMMARY-BODY"})]).await;
        let compact = llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 4096);
        let prompt = Message::user("current question");
        let (view1, rec1) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("l1");
        assert_eq!(view1.compact_level, 1);
        let rec1 = rec1.expect("l1");
        store.set_compact(rec1);
        fill_turns(&mut store, 8, 8, 200);
        let (view2, rec2) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("l2");
        let rec2 = rec2.expect("l2 record");
        assert_eq!(rec2.level, 2);
        assert_eq!(view2.compact_level, 2);
        assert_eq!(store.turns().len(), 16);
        let dumped = serde_json::to_string(&view2.messages).unwrap();
        assert!(dumped.contains("L2-SUMMARY-BODY"), "{dumped}");
        assert!(!dumped.contains("turn-0-"), "{dumped}");
        let captured = bodies.lock().expect("bodies").clone();
        assert_eq!(
            captured.len(),
            1,
            "L2 makes one compact HTTP call: {captured:?}"
        );
        let body = captured[0].to_string();
        assert!(body.contains("CODEG-COMPACT-PROMPT-MARKER"), "{body}");
        assert!(
            !body.contains("\"tools\"") || body.contains("\"tools\":[]"),
            "{body}"
        );
        let cap = captured[0]
            .get("max_tokens")
            .or_else(|| captured[0].get("max_completion_tokens"));
        assert_eq!(cap, Some(&json!(2048)), "{:?}", captured[0]);

        store.set_compact(rec2.clone());
        let (view3, rec3) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("resume");
        assert!(rec3.is_none(), "resume must not re-compact");
        assert_eq!(view3.compact_level, 2);
        let dumped = serde_json::to_string(&view3.messages).unwrap();
        assert!(dumped.contains("L2-SUMMARY-BODY"), "{dumped}");
        assert_eq!(
            bodies.lock().expect("bodies").len(),
            1,
            "resume reuses L2 and must not spend another compact call"
        );
    }

    #[tokio::test]
    async fn l2_failure_retains_l1() {
        let mut store = ContextStore::new("s");
        fill_turns(&mut store, 0, 8, 200);
        let (base, bodies) = spawn_json_completions(vec![json!({"fail": true})]).await;
        let compact = llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 2048);
        let prompt = Message::user("current question");
        let (_, rec1) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("l1");
        let rec1 = rec1.expect("l1");
        let l1_summary = rec1.summary.clone();
        store.set_compact(rec1);
        fill_turns(&mut store, 8, 8, 200);
        let (view2, rec2) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("l2 fail keeps l1");
        assert!(rec2.is_none(), "failed L2 must not replace L1");
        assert_eq!(view2.compact_level, 1);
        assert_eq!(store.compact().map(|c| c.level), Some(1));
        let dumped = serde_json::to_string(&view2.messages).unwrap();
        assert!(
            dumped.contains(&l1_summary) || dumped.contains("Conversation summary"),
            "{dumped}"
        );
        assert_eq!(bodies.lock().expect("bodies").len(), 1);
    }

    #[tokio::test]
    async fn one_project_view_upgrades_at_most_one_level() {
        let mut store = ContextStore::new("s");
        fill_turns(&mut store, 0, 8, 200);
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "L2-SHOULD-WAIT"})]).await;
        let compact = llm(&base, "marker", 2048);
        let prompt = Message::user("current question");
        let (view, record) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(8_000, 1024),
                preamble: "short",
                tool_schemas: &[],
                prompt: &prompt,
            },
            Some(&compact),
        )
        .await
        .expect("one level");
        assert_eq!(record.expect("l1").level, 1);
        assert_eq!(view.compact_level, 1);
        assert!(
            bodies.lock().expect("bodies").is_empty(),
            "first trigger is L1 only"
        );
    }
}

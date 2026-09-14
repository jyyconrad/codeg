//! Two-level context compression using rig-memory Compactor / MemoryPolicy.
//!
//! L1 is [`TemplateCompactor`] (no HTTP). L2 is [`LlmCompactor`] on the second
//! session trigger. Never call `AgentBuilder::memory()` — transcript is truth.

use std::path::{Component, Path, PathBuf};

use rig::client::CompletionClient;
use rig::completion::message::{ToolResultContent, UserContent};
use rig::completion::{AssistantContent, CompletionModel, Message};
use rig_memory::{Compactor, MemoryError, MemoryPolicy, SlidingWindowMemory, TemplateCompactor};

use super::budget::{
    estimate_request, messages_from_turns, BudgetConfig, BudgetError, BudgetInputs,
};
use super::store::{
    CanonicalTurn, CompactRecord, ContextStore, ContextView, ExecutionFact, UsageSource,
};
use super::tool_prune::{
    distill_tool_result, hard_clear_tool_result, tool_skips_hard_clear, DistillKind,
};
use crate::acp_transcript::now_epoch_ms;
use crate::agent::model::CodegLlmClient;

/// Cap on L2 compact `max_tokens` (min of this and the session setting).
pub const L2_MAX_TOKENS: u64 = 2048;
const L1_SUMMARY_MAX_BYTES: usize = 8 * 1024;
/// OpenCode prune: keep the last two user turns' tool output intact.
const PROTECT_RECENT_USER_TURNS: usize = 2;
/// Claude Code microcompact / OpenClaw `keep=3`: never hard-clear the newest results.
const PROTECT_RECENT_TOOL_RESULTS: usize = 3;

/// Artifact produced by [`LlmCompactor`].
#[derive(Clone, Debug)]
pub struct CompactFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct CompactArtifact {
    pub summary: String,
    pub files: Vec<CompactFile>,
}

impl From<CompactArtifact> for Message {
    fn from(value: CompactArtifact) -> Self {
        Message::user(value.summary)
    }
}

/// L2 LLM summarizer. Same session client/protocol as the main turn.
#[derive(Clone)]
pub struct LlmCompactor {
    client: CodegLlmClient,
    model_id: String,
    compact_prompt: String,
    max_tokens: u64,
    workspace: Option<PathBuf>,
}

impl LlmCompactor {
    pub fn new(
        client: CodegLlmClient,
        model_id: impl Into<String>,
        compact_prompt: impl Into<String>,
        max_tokens: u64,
    ) -> Self {
        Self {
            client,
            model_id: model_id.into(),
            compact_prompt: compact_prompt.into(),
            max_tokens: max_tokens.clamp(1, L2_MAX_TOKENS),
            workspace: None,
        }
    }

    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    pub fn max_tokens(&self) -> u64 {
        self.max_tokens
    }

    pub fn compact_prompt(&self) -> &str {
        &self.compact_prompt
    }

    async fn summarize(
        &self,
        conversation_id: &str,
        evicted: &[Message],
        carry_over: Option<&str>,
    ) -> Result<CompactArtifact, String> {
        let mut body = String::new();
        if let Some(prev) = carry_over.map(str::trim).filter(|s| !s.is_empty()) {
            body.push_str("Previous summary:\n");
            body.push_str(prev);
            body.push_str("\n\n");
        }
        body.push_str("Evicted turns:\n");
        body.push_str(&messages_as_text(evicted));
        let prompt = Message::user(body);
        let response = match &self.client {
            CodegLlmClient::Completions(client) => client
                .completion_model(&self.model_id)
                .completion_request(prompt)
                .preamble(self.compact_prompt.clone())
                .max_tokens(self.max_tokens)
                .send()
                .await
                .map_err(|err| err.to_string())?,
            CodegLlmClient::Responses(client) => client
                .completion_model(&self.model_id)
                .completion_request(prompt)
                .preamble(self.compact_prompt.clone())
                .max_tokens(self.max_tokens)
                .send()
                .await
                .map_err(|err| err.to_string())?,
        };
        let text = choice_text(&response);
        if text.trim().is_empty() {
            return Err("empty compact summary".into());
        }
        let mut artifact = parse_compact_artifact(&text)?;
        if let Some(workspace) = &self.workspace {
            let written = write_compact_files(workspace, conversation_id, &artifact.files)?;
            if !written.is_empty() {
                artifact.summary.push_str("\n\nContext files written:\n");
                for path in written {
                    artifact.summary.push_str("- ");
                    artifact.summary.push_str(&path);
                    artifact.summary.push('\n');
                }
            }
        }
        Ok(artifact)
    }
}

impl Compactor for LlmCompactor {
    type Artifact = CompactArtifact;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            self.summarize(
                conversation_id,
                evicted,
                carry_over.map(|a| a.summary.as_str()),
            )
            .await
            .map_err(MemoryError::Internal)
        })
    }
}

#[derive(serde::Deserialize)]
struct CompactEnvelope {
    summary: String,
    #[serde(default)]
    files: Vec<CompactFileEnvelope>,
}

#[derive(serde::Deserialize)]
struct CompactFileEnvelope {
    path: String,
    content: String,
}

fn parse_compact_artifact(text: &str) -> Result<CompactArtifact, String> {
    let trimmed = text.trim();
    let parsed = serde_json::from_str::<CompactEnvelope>(trimmed).or_else(|_| {
        let body = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```JSON"))
            .or_else(|| trimmed.strip_prefix("```"))
            .map(|s| s.trim().trim_end_matches("```").trim())
            .unwrap_or(trimmed);
        serde_json::from_str::<CompactEnvelope>(body)
    });
    match parsed {
        Ok(envelope) if !envelope.summary.trim().is_empty() => Ok(CompactArtifact {
            summary: envelope.summary,
            files: envelope
                .files
                .into_iter()
                .map(|file| CompactFile {
                    path: file.path,
                    content: file.content,
                })
                .collect(),
        }),
        Ok(_) => Err("empty compact summary".into()),
        Err(_) => Ok(CompactArtifact {
            summary: trimmed.to_string(),
            files: Vec::new(),
        }),
    }
}

fn write_compact_files(
    workspace: &Path,
    conversation_id: &str,
    files: &[CompactFile],
) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let session = conversation_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(80)
        .collect::<String>();
    let root = workspace
        .join(".codeg")
        .join("context")
        .join(if session.is_empty() {
            "session"
        } else {
            &session
        });
    std::fs::create_dir_all(&root).map_err(|e| format!("create compact context dir: {e}"))?;
    let mut written = Vec::new();
    for file in files {
        let rel = Path::new(file.path.trim());
        if rel.extension().and_then(|s| s.to_str()) != Some("md")
            || rel.is_absolute()
            || rel.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!("invalid compact markdown path: {}", file.path));
        }
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create compact file dir: {e}"))?;
        }
        std::fs::write(&path, &file.content)
            .map_err(|e| format!("write compact markdown {}: {e}", path.display()))?;
        written.push(path.to_string_lossy().to_string());
    }
    Ok(written)
}

/// Project canonical facts. At most one compact-level upgrade per call.
///
/// Tool results are distilled per tool **before** turn eviction or L1/L2
/// summarization. Canonical facts in the store are not rewritten.
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
    let omitted_at_80 =
        (budget * u64::from(inputs.config.compact_soft_percent.clamp(1, 100))) / 100;
    let recent_target = inputs.config.compact_recent_turns.max(1);
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
        let over_soft = estimated >= omitted_at_80 && live.len() > recent_target;
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
        let trial_soft = trial_est >= omitted_at_80 && live.len() > recent_target;
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
            files: Vec::new(),
            created_at_ms: now_epoch_ms(),
        };
        return hard_drop_view(inputs, budget, live, None, 1, Some(record));
    }

    if level == 1 {
        let carry = existing.as_ref().map(|r| CompactArtifact {
            summary: r.summary.clone(),
            files: r
                .files
                .iter()
                .map(|path| CompactFile {
                    path: path.clone(),
                    content: String::new(),
                })
                .collect(),
        });
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
            Ok(artifact) if !artifact.summary.trim().is_empty() => {
                let record = CompactRecord {
                    level: 2,
                    through_turn,
                    summary: artifact.summary,
                    files: artifact
                        .files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect(),
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
    prune_tool_results_in_messages(&mut out, store);
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
        Ok((kept_msgs, mut evicted_msgs)) => {
            prune_tool_results_for_summarization(&mut evicted_msgs, store);
            (evicted_msgs, kept_msgs)
        }
        Err(_) => {
            let mut evicted = messages_from_turns(evicted_turns, store);
            prune_tool_results_for_summarization(&mut evicted, store);
            (evicted, messages_from_turns(kept_turns, store))
        }
    }
}

/// Hard-clear old tool results, then distill the protected tail per tool.
///
/// Protects the last [`PROTECT_RECENT_USER_TURNS`] user-text turns and at least
/// the last [`PROTECT_RECENT_TOOL_RESULTS`] tool-result messages.
fn prune_tool_results_in_messages(messages: &mut [Message], store: &ContextStore) {
    let cutoff = protect_cutoff(messages);
    for (index, message) in messages.iter_mut().enumerate() {
        if index < cutoff {
            rewrite_tool_results(message, store, |name, fact, text| {
                if tool_skips_hard_clear(name) {
                    distill_tool_result(name, fact, text, DistillKind::Live)
                } else {
                    hard_clear_tool_result(name, fact, text)
                }
            });
        } else {
            rewrite_tool_results(message, store, |name, fact, text| {
                distill_tool_result(name, fact, text, DistillKind::Live)
            });
        }
    }
}

fn prune_tool_results_for_summarization(messages: &mut [Message], store: &ContextStore) {
    for message in messages.iter_mut() {
        rewrite_tool_results(message, store, |name, fact, text| {
            distill_tool_result(name, fact, text, DistillKind::Summarize)
        });
    }
}

fn protect_cutoff(messages: &[Message]) -> usize {
    let user_turns: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| is_user_text_message(message))
        .map(|(index, _)| index)
        .collect();
    let turn_cutoff = user_turns
        .len()
        .checked_sub(PROTECT_RECENT_USER_TURNS)
        .and_then(|index| user_turns.get(index).copied())
        .unwrap_or(0);
    let tool_msgs: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| has_tool_result(message))
        .map(|(index, _)| index)
        .collect();
    let tool_cutoff = tool_msgs
        .len()
        .checked_sub(PROTECT_RECENT_TOOL_RESULTS)
        .and_then(|index| tool_msgs.get(index).copied())
        .unwrap_or(0);
    turn_cutoff.min(tool_cutoff)
}

fn is_user_text_message(message: &Message) -> bool {
    match message {
        Message::User { content } => content
            .iter()
            .any(|part| matches!(part, UserContent::Text(_))),
        _ => false,
    }
}

fn has_tool_result(message: &Message) -> bool {
    match message {
        Message::User { content } => content
            .iter()
            .any(|part| matches!(part, UserContent::ToolResult(_))),
        _ => false,
    }
}

fn rewrite_tool_results(
    message: &mut Message,
    store: &ContextStore,
    mut rewrite: impl FnMut(&str, Option<&ExecutionFact>, &str) -> String,
) {
    let Message::User { content } = message else {
        return;
    };
    for part in content {
        let UserContent::ToolResult(result) = part else {
            continue;
        };
        let text = tool_result_text(&result.content);
        let fact = store.fact(result.call.as_str());
        let next = rewrite(&result.name, fact, &text);
        if next != text {
            result.content = vec![ToolResultContent::text(next)];
        }
    }
}

fn tool_result_text(content: &[ToolResultContent]) -> String {
    content
        .iter()
        .filter_map(ToolResultContent::as_text)
        .collect::<Vec<_>>()
        .join("\n")
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
    use crate::agent::context::store::{
        AssistantPart, AssistantRecord, ContextStore, ExecutionFact,
    };
    use crate::agent::context::{ToolOutcome, ToolPhase};
    use crate::agent::model::{completions_client, CodegLlmClient};
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use rig::completion::Message;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    fn cfg(window: u64, output: u64) -> BudgetConfig {
        BudgetConfig::new(window, output)
    }

    #[test]
    fn compact_envelope_preserves_summary_and_writes_session_markdown() {
        let parsed = parse_compact_artifact(
            r##"{"summary":"goal and next step","files":[{"path":"api.md","content":"# API"}]}"##,
        )
        .expect("valid compact envelope");
        assert_eq!(parsed.summary, "goal and next step");
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = write_compact_files(dir.path(), "session/unsafe", &parsed.files)
            .expect("write compact file");
        assert_eq!(paths.len(), 1);
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), "# API");
        assert!(paths[0].contains("sessionunsafe"));
    }

    #[test]
    fn compact_files_reject_path_traversal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = write_compact_files(
            dir.path(),
            "session",
            &[CompactFile {
                path: "../escape.md".into(),
                content: "x".into(),
            }],
        )
        .expect_err("traversal must be rejected");
        assert!(err.contains("invalid compact markdown path"));
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

    fn record_read(store: &mut ContextStore, i: usize, body: &str) {
        let turn_id = format!("s:{i}");
        let call_id = format!("call_{i}");
        store.append_user(turn_id.clone(), format!("ask-{i}"));
        store.record_fact(ExecutionFact {
            tool_call_id: call_id.clone(),
            function_name: "read_file".into(),
            raw_input: json!({"path": format!("f{i}.txt")}),
            phase: ToolPhase::Terminal,
            outcome: Some(ToolOutcome::Success),
            executed: Some(true),
            model_presentation: Some(body.to_string()),
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: turn_id.clone(),
        });
        store.commit_assistant(
            &turn_id,
            AssistantRecord {
                model_message_id: Some(format!("m{i}")),
                committed: true,
                parts: vec![AssistantPart::ToolCall {
                    id: call_id,
                    name: "read_file".into(),
                    args: json!({"path": format!("f{i}.txt")}),
                }],
            },
        );
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
        LlmCompactor::new(CodegLlmClient::Completions(client), "m", prompt, max_tokens)
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

    #[tokio::test]
    async fn prunes_old_tool_results_before_rolling_compact() {
        let mut store = ContextStore::new("s");
        let old = format!("OLD-TOOL-BODY-{}", "x".repeat(20_000));
        record_read(&mut store, 0, &old);
        record_read(
            &mut store,
            1,
            &format!("MID-TOOL-BODY-{}", "z".repeat(20_000)),
        );
        record_read(
            &mut store,
            2,
            &format!("RECENT-TOOL-BODY-{}", "y".repeat(20_000)),
        );
        record_read(
            &mut store,
            3,
            &format!("LATEST-TOOL-BODY-{}", "w".repeat(200)),
        );
        let prompt = Message::user("continue");
        let (view, record) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(30_000, 1024),
                preamble: "p",
                tool_schemas: &[],
                prompt: &prompt,
            },
            None,
        )
        .await
        .expect("fits after pruning tool results");
        assert!(
            record.is_none(),
            "tool-result prune must run before L1 eviction"
        );
        assert_eq!(view.compact_level, 0);
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(
            dumped.contains("f0.txt") && dumped.contains("re-read"),
            "hard-cleared reads must keep the path and a re-read hint: {dumped}"
        );
        assert!(
            !dumped.contains("OLD-TOOL-BODY-"),
            "oldest tool body must not remain in the projection: {dumped}"
        );
        assert!(
            dumped.contains("LATEST-TOOL-BODY-"),
            "the newest tool result stays: {dumped}"
        );
        assert_eq!(
            store
                .fact("call_0")
                .and_then(|f| f.model_presentation.as_ref())
                .map(String::len),
            Some(old.len()),
            "canonical facts stay untruncated"
        );
    }

    #[tokio::test]
    async fn soft_trims_protected_tool_results_head_and_tail() {
        let mut store = ContextStore::new("s");
        let recent = format!("HEAD-MARKER-{}-TAIL-MARKER", "n".repeat(10_000));
        record_read(&mut store, 0, &recent);
        let prompt = Message::user("continue");
        let (view, record) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(20_000, 1024),
                preamble: "p",
                tool_schemas: &[],
                prompt: &prompt,
            },
            None,
        )
        .await
        .expect("fits after soft-trim");
        assert!(record.is_none());
        assert_eq!(view.compact_level, 0);
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(dumped.contains("HEAD-MARKER-"), "{dumped}");
        assert!(dumped.contains("TAIL-MARKER"), "{dumped}");
        assert!(
            dumped.contains("f0.txt") && dumped.contains("re-read"),
            "oversized reads keep path, excerpts, and a re-read hint: {dumped}"
        );
        assert!(
            dumped.len() < recent.len(),
            "projection must be smaller than the raw tool body"
        );
    }

    fn record_write(store: &mut ContextStore, i: usize, path: &str, content: &str, body: &str) {
        let turn_id = format!("s:{i}");
        let call_id = format!("call_{i}");
        store.append_user(turn_id.clone(), format!("write-{i}"));
        store.record_fact(ExecutionFact {
            tool_call_id: call_id.clone(),
            function_name: "write_file".into(),
            raw_input: json!({"path": path, "content": content}),
            phase: ToolPhase::Terminal,
            outcome: Some(ToolOutcome::Success),
            executed: Some(true),
            model_presentation: Some(body.to_string()),
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: turn_id.clone(),
        });
        store.commit_assistant(
            &turn_id,
            AssistantRecord {
                model_message_id: Some(format!("m{i}")),
                committed: true,
                parts: vec![AssistantPart::ToolCall {
                    id: call_id,
                    name: "write_file".into(),
                    args: json!({"path": path, "content": content}),
                }],
            },
        );
    }

    #[tokio::test]
    async fn write_file_soft_trim_is_status_not_file_body() {
        let mut store = ContextStore::new("s");
        let content = "hello world";
        let body = format!("FILE-BODY-{}", "w".repeat(10_000));
        record_write(&mut store, 0, "src/out.rs", content, &body);
        let prompt = Message::user("continue");
        let (view, record) = project_compacted(
            BudgetInputs {
                store: &store,
                config: cfg(20_000, 1024),
                preamble: "p",
                tool_schemas: &[],
                prompt: &prompt,
            },
            None,
        )
        .await
        .expect("fits after write distill");
        assert!(record.is_none());
        let dumped = serde_json::to_string(&view.messages).unwrap();
        assert!(dumped.contains("src/out.rs"), "{dumped}");
        assert!(
            dumped.contains("11 chars") || dumped.contains("11 characters"),
            "write distill reports written size: {dumped}"
        );
        assert!(
            !dumped.contains("FILE-BODY-"),
            "write distill must not keep the file body: {dumped}"
        );
    }

    #[tokio::test]
    async fn l2_summarize_input_does_not_replay_full_tool_bodies() {
        let mut store = ContextStore::new("s");
        fill_turns(&mut store, 0, 8, 200);
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "L2-SUMMARY-BODY"})]).await;
        let compact = llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 4096);
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
        store.set_compact(rec1.expect("l1"));
        let bulky = format!("EVICTED-TOOL-BODY-{}", "x".repeat(8_000));
        record_read(&mut store, 8, &bulky);
        record_read(
            &mut store,
            9,
            &format!("NEXT-TOOL-BODY-{}", "y".repeat(8_000)),
        );
        fill_turns(&mut store, 10, 8, 200);
        let (_, rec2) = project_compacted(
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
        assert_eq!(rec2.expect("l2").level, 2);
        let captured = bodies.lock().expect("bodies").clone();
        assert_eq!(captured.len(), 1);
        let body = captured[0].to_string();
        assert!(
            !body.contains(&"x".repeat(4_000)),
            "L2 compact request must distill tool results first ({} bytes)",
            body.len()
        );
        assert!(
            body.contains("f8.txt"),
            "L2 input must keep the read path instead of a 2000-char prefix cut ({} bytes)",
            body.len()
        );
    }
}

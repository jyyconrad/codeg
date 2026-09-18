//! Per-session [`CompactingMemory`] + [`RunRecorder`] composition.
//!
//! One instance is reused for the whole session so Rig's in-process absorbed
//! watermark survives model turns. Session close calls [`Self::forget`] on the
//! wrapper; it never [`ConversationMemory::clear`]s the inner store.
//!
//! Residual vs a Rig `drive_agent` committed-messages checkpoint: tool-result
//! `Message`s are appended from hook events (`on_completion_call` sees the
//! grouped prompt; `on_model_turn_finished` saves the accepted assistant).
//! Concurrent tool completion order can differ from Rig's final `run.messages()`
//! order when tool concurrency > 1. Confirmed results are not dropped.
//! `auto_replay_ids` stays empty.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rig::completion::Message;
use rig::memory::ConversationMemory;
use rig_memory::{CompactingMemory, MemoryError};
use sha2::{Digest, Sha256};

use super::budget::{estimate_request, BudgetConfig};
use super::checkpoint::{CheckpointStore, CheckpointedCompactor, RequestBudgetSnapshot};
use super::compact::LlmCompactor;
use super::compact_llm::{CompactLlmConfig, CompactLlmSessionView, CompactionControl};
use super::message_memory::{CodegMessageMemory, RunHandle, RunRecorder};
use super::policy::{CodegContextPolicy, RequestScope};
use crate::agent::model::CodegLlmClient;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

type SessionCompacting =
    CompactingMemory<CodegMessageMemory, CodegContextPolicy, CheckpointedCompactor<LlmCompactor>>;

/// Compacted-or-raw history for one model request. Does not include the
/// current pending prompt; Rig adds that once via `.runner(prompt)`.
#[derive(Clone, Debug)]
pub struct LoadedHistory {
    pub messages: Vec<Message>,
    /// True when the view is summary+kept rather than the original prefix.
    pub compacted: bool,
    pub estimated_tokens: u64,
}

/// Session-lifetime CompactingMemory + RunRecorder. Clone is cheap (`Arc`).
#[derive(Clone)]
pub struct SessionMemory {
    inner: CodegMessageMemory,
    recorder: RunRecorder,
    compacting: Arc<SessionCompacting>,
    compact_llm: Option<CompactLlmSessionView>,
    compaction_cancel: Arc<Mutex<CancellationToken>>,
    conversation_id: String,
    load_lock: Arc<tokio::sync::Mutex<()>>,
    covers_through_seq: Arc<Mutex<Option<usize>>>,
}

impl SessionMemory {
    /// Session under [`crate::paths::codeg_agent_sessions_root`].
    pub fn open(
        session_id: impl Into<String>,
        cwd: impl AsRef<str>,
        tail_budget: usize,
        model_id: &str,
        llm: LlmCompactor,
    ) -> Self {
        Self::open_in(
            crate::paths::codeg_agent_sessions_root(),
            session_id,
            cwd,
            tail_budget,
            model_id,
            llm,
        )
    }

    /// Session under an injected root (tests).
    pub fn open_in(
        root: impl Into<PathBuf>,
        session_id: impl Into<String>,
        cwd: impl AsRef<str>,
        tail_budget: usize,
        model_id: &str,
        llm: LlmCompactor,
    ) -> Self {
        let inner = CodegMessageMemory::with_root(root, session_id, cwd);
        Self::from_inner(inner, tail_budget, model_id, llm)
    }

    fn from_inner(
        inner: CodegMessageMemory,
        tail_budget: usize,
        model_id: &str,
        llm: LlmCompactor,
    ) -> Self {
        let fingerprint = format!(
            "{model_id}|{}|heuristic",
            sha256_hex(llm.compact_prompt().as_bytes())
        );
        let policy = CodegContextPolicy::token_window(tail_budget, RequestScope::default());
        let store = CheckpointStore::new(inner.session_dir());
        let snapshot = RequestBudgetSnapshot {
            source_last_seq: None,
            max_summary_tokens: Some(llm.max_tokens()),
        };
        let compactor = CheckpointedCompactor::new(llm, store)
            .with_fingerprint(fingerprint)
            .with_request_snapshot(snapshot);
        let conversation_id = inner.session_id().to_string();
        let recorder = RunRecorder::new(inner.clone());
        let compacting = CompactingMemory::new(inner.clone(), policy, compactor);
        Self {
            inner,
            recorder,
            compacting: Arc::new(compacting),
            compact_llm: None,
            compaction_cancel: Arc::new(Mutex::new(CancellationToken::new())),
            conversation_id,
            load_lock: Arc::new(tokio::sync::Mutex::new(())),
            covers_through_seq: Arc::new(Mutex::new(None)),
        }
    }

    /// Opt in to the isolated compact-llm view. No-op when `config.enabled` is false.
    pub fn with_compact_llm(
        mut self,
        client: CodegLlmClient,
        config: CompactLlmConfig,
        tail_budget: usize,
    ) -> Self {
        if !config.enabled {
            return self;
        }
        self.compact_llm = Some(CompactLlmSessionView::new(
            self.inner.clone(),
            client,
            config,
            tail_budget,
        ));
        self
    }

    /// Per-turn cancellation for compact-llm. Ignored on the default path.
    pub fn set_compaction_control(&self, cancel: CancellationToken) {
        *self
            .compaction_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = cancel;
    }

    pub fn session_id(&self) -> &str {
        self.inner.session_id()
    }

    pub fn session_dir(&self) -> &Path {
        self.inner.session_dir()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn inner(&self) -> &CodegMessageMemory {
        &self.inner
    }

    pub fn recorder(&self) -> &RunRecorder {
        &self.recorder
    }

    pub fn covers_through_seq(&self) -> Option<usize> {
        if let Some(view) = &self.compact_llm {
            return view.covers_through_seq();
        }
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Tail token budget for [`CodegContextPolicy::token_window`].
    ///
    /// Falls back to a huge window when the request cannot fit output+safety
    /// so we do not demote everything; [`Self::load_history`] still fail-closes
    /// with `context_budget_exceeded`.
    pub fn tail_budget_from(budget: BudgetConfig) -> usize {
        match budget.input_budget() {
            Ok(input) => {
                let target = input.saturating_mul(u64::from(budget.compact_soft_percent)) / 100;
                let reserve = super::compact::L2_MAX_TOKENS.max(1024);
                target.saturating_sub(reserve).max(1) as usize
            }
            Err(_) => usize::MAX / 4,
        }
    }

    /// Save the current user prompt once. Runner history is messages before it.
    pub async fn begin_run(&self, prompt: &Message) -> Result<RunHandle, MemoryError> {
        self.recorder.begin_run(prompt).await
    }

    /// Bind pending/coverage on the live policy without rebuilding CompactingMemory.
    pub fn bind_scope(&self, scope: RequestScope) {
        if let Some(view) = &self.compact_llm {
            view.bind_scope(scope);
            return;
        }
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = scope.covers_through_seq;
        self.compacting.policy().bind_scope(scope);
    }

    /// Append newly confirmed run messages. Idempotent on `(run_id, ordinal)`.
    pub async fn persist_messages(&self, messages: Vec<Message>) -> Result<(), MemoryError> {
        if messages.is_empty() {
            return Ok(());
        }
        self.recorder.append_messages(messages).await
    }

    /// Append messages that are not already in the committed prefix.
    ///
    /// Equality skip is a safety net for hook retries; ordinal idempotency on
    /// the recorder is the real contract. Residual: reconstructed tool-result
    /// messages that do not `PartialEq` the Rig prompt can still be appended
    /// as a later ordinal.
    pub async fn persist_if_new(&self, messages: &[Message]) -> Result<(), MemoryError> {
        let committed = self.inner.load_committed().await?;
        let new: Vec<Message> = messages
            .iter()
            .filter(|message| {
                !is_history_summary_message(message) && !committed.iter().any(|c| c == *message)
            })
            .cloned()
            .collect();
        self.persist_messages(new).await
    }

    /// Serialize same-session loads, rebind pending, compact, then budget-check.
    pub async fn load_history(
        &self,
        pending: &Message,
        preamble: &str,
        tool_schemas: &[Value],
        budget: BudgetConfig,
    ) -> Result<LoadedHistory, MemoryError> {
        if let Some(view) = &self.compact_llm {
            let cancel = self
                .compaction_cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            return view
                .load(
                    pending,
                    preamble,
                    tool_schemas,
                    budget,
                    CompactionControl::new(cancel),
                )
                .await;
        }
        let _serial = self.load_lock.lock().await;
        let committed = self.inner.load_committed().await?;
        let pending_seq = bind_seq_excluding_pending(&committed, pending);
        self.inner.bind_history_before(pending_seq).await;

        self.compacting.policy().bind_scope(RequestScope {
            pending_prompt: Some(pending.clone()),
            covers_through_seq: self.covers_through_seq(),
            ..RequestScope::default()
        });

        let loaded = ConversationMemory::load(self.compacting.as_ref(), &self.conversation_id)
            .await
            .map_err(policy_budget_error)?;

        let original: Vec<Message> = {
            let n = pending_seq.saturating_sub(1) as usize;
            committed.iter().take(n).cloned().collect()
        };
        let compacted = loaded != original;
        if compacted {
            if let Some(covers) = covers_through(&original, &loaded) {
                *self
                    .covers_through_seq
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(covers);
            }
        }

        let estimated_tokens = estimate_request(preamble, tool_schemas, &loaded, pending);
        match budget.input_budget() {
            Ok(input) if estimated_tokens > input => {
                return Err(MemoryError::Policy(format!(
                    "context_budget_exceeded: estimated {estimated_tokens} exceeds input budget {input}"
                )));
            }
            Ok(_) => {}
            Err(err) => {
                return Err(MemoryError::Policy(format!(
                    "context_budget_exceeded: {err}"
                )));
            }
        }

        Ok(LoadedHistory {
            messages: loaded,
            compacted,
            estimated_tokens,
        })
    }

    /// Drop in-process compaction state. Does not delete `messages.jsonl`.
    pub fn forget(&self) {
        if let Some(view) = &self.compact_llm {
            view.forget();
        }
        self.compacting.forget(&self.conversation_id);
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// 1-based seq to pass to [`CodegMessageMemory::bind_history_before`].
///
/// If `pending` is the last committed message, exclude it. Otherwise include
/// the full committed prefix (`committed_count + 1`).
fn bind_seq_excluding_pending(committed: &[Message], pending: &Message) -> u64 {
    if committed.last() == Some(pending) {
        committed.len() as u64
    } else {
        committed.len() as u64 + 1
    }
}

fn covers_through(original: &[Message], loaded: &[Message]) -> Option<usize> {
    if loaded.is_empty() {
        return (!original.is_empty()).then_some(original.len());
    }
    if let Some(kept) = loaded.get(1..) {
        if original.ends_with(kept) {
            return Some(original.len().saturating_sub(kept.len()));
        }
    }
    if original.ends_with(loaded) {
        return Some(original.len().saturating_sub(loaded.len()));
    }
    None
}

fn policy_budget_error(err: MemoryError) -> MemoryError {
    match err {
        MemoryError::Policy(msg) if msg.contains("context_budget_exceeded") => {
            MemoryError::Policy(msg)
        }
        MemoryError::Policy(msg) => MemoryError::Policy(format!("context_budget_exceeded: {msg}")),
        other => other,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_history_summary_message(message: &Message) -> bool {
    let Message::User { content } = message else {
        return false;
    };
    let mut text = String::new();
    for part in content {
        if let rig::completion::message::UserContent::Text(t) = part {
            text.push_str(&t.text);
        }
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    trimmed.starts_with("历史摘要：")
        || trimmed.starts_with("历史摘要:")
        || trimmed.starts_with("Conversation summary:")
        || trimmed.starts_with("Conversation summary")
        || lower.starts_with("history summary:")
        || lower.starts_with("history summary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::compact::LlmCompactor;
    use crate::agent::model::{completions_client, CodegLlmClient};
    use rig::completion::Message;

    fn dummy_llm() -> LlmCompactor {
        let client = CodegLlmClient::Completions(
            completions_client("sk-test", "http://127.0.0.1:9/v1").expect("client"),
        );
        LlmCompactor::new(client, "codeg-test", "summarize", 128)
    }

    fn temp_session() -> (tempfile::TempDir, SessionMemory) {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory = SessionMemory::open_in(
            dir.path(),
            "sess-1",
            "/tmp/demo",
            usize::MAX / 4,
            "m",
            dummy_llm(),
        );
        (dir, memory)
    }

    fn budget() -> BudgetConfig {
        BudgetConfig::new(128_000, 4096)
    }

    #[tokio::test]
    async fn begin_run_then_load_excludes_prompt() {
        let (_dir, memory) = temp_session();
        let prior = Message::user("already there");
        ConversationMemory::append(
            memory.inner(),
            memory.conversation_id(),
            vec![prior.clone()],
        )
        .await
        .expect("seed");
        let prompt = Message::user("current instruction");
        let handle = memory.begin_run(&prompt).await.expect("begin_run");
        let loaded = memory
            .load_history(&prompt, "preamble", &[], budget())
            .await
            .expect("load");
        assert_eq!(loaded.messages, vec![prior.clone()]);
        assert!(!loaded.compacted);
        assert_eq!(handle.prompt_seq(), 2);
        let full = memory.inner().load_committed().await.expect("committed");
        assert_eq!(full, vec![prior, prompt]);
    }

    #[tokio::test]
    async fn persist_assistant_is_visible_on_next_load() {
        let (_dir, memory) = temp_session();
        let prompt = Message::user("do work");
        memory.begin_run(&prompt).await.expect("begin_run");
        let assistant = Message::assistant("calling echo");
        memory
            .persist_messages(vec![assistant.clone()])
            .await
            .expect("persist");
        let tool_prompt = Message::user("tool-output");
        let loaded = memory
            .load_history(&tool_prompt, "preamble", &[], budget())
            .await
            .expect("load");
        assert!(
            loaded.messages.iter().any(|m| m == &prompt),
            "saved user prompt must be in later history: {:?}",
            loaded.messages
        );
        assert!(
            loaded.messages.iter().any(|m| m == &assistant),
            "saved assistant must be in later history: {:?}",
            loaded.messages
        );
        assert!(
            loaded.messages.iter().all(|m| m != &tool_prompt),
            "pending prompt must not be in history: {:?}",
            loaded.messages
        );
        assert!(!loaded.compacted);
    }

    #[tokio::test]
    async fn persist_if_new_skips_already_committed() {
        let (_dir, memory) = temp_session();
        let prompt = Message::user("once");
        memory.begin_run(&prompt).await.expect("begin_run");
        memory
            .persist_if_new(std::slice::from_ref(&prompt))
            .await
            .expect("skip");
        let full = memory.inner().load_committed().await.expect("committed");
        assert_eq!(full, vec![prompt]);
    }

    #[tokio::test]
    async fn persist_if_new_does_not_write_compaction_summary_into_originals() {
        let (_dir, memory) = temp_session();
        let prompt = Message::user("commit this");
        let kept = Message::assistant("kept suffix");
        memory.begin_run(&prompt).await.expect("begin_run");
        memory
            .persist_messages(vec![kept.clone()])
            .await
            .expect("kept");
        let summary = Message::user("Conversation summary: earlier turns were compacted.");
        memory
            .persist_if_new(&[summary.clone(), prompt.clone(), kept.clone()])
            .await
            .expect("skip summary");
        let full = memory.inner().load_committed().await.expect("committed");
        assert_eq!(full, vec![prompt, kept]);
        assert!(
            full.iter().all(|m| m != &summary),
            "derived summary must not be appended to messages.jsonl: {full:?}"
        );
    }

    #[tokio::test]
    async fn forget_does_not_clear_messages() {
        let (_dir, memory) = temp_session();
        let prompt = Message::user("keep me");
        memory.begin_run(&prompt).await.expect("begin_run");
        memory.forget();
        let full = memory.inner().load_committed().await.expect("committed");
        assert_eq!(full, vec![prompt]);
        assert!(memory.session_dir().join("messages.jsonl").exists());
    }

    #[tokio::test]
    async fn bind_scope_reuses_the_same_compacting_instance() {
        let (_dir, memory) = temp_session();
        let prompt = Message::user("p");
        memory.begin_run(&prompt).await.expect("begin_run");
        memory.bind_scope(RequestScope {
            pending_prompt: Some(prompt.clone()),
            covers_through_seq: Some(0),
            ..RequestScope::default()
        });
        let first = memory
            .load_history(&prompt, "preamble", &[], budget())
            .await
            .expect("first");
        let second = memory
            .load_history(&prompt, "preamble", &[], budget())
            .await
            .expect("second");
        assert_eq!(first.messages, second.messages);
        assert!(!first.compacted);
        assert!(!second.compacted);
    }

    #[test]
    fn tail_budget_is_positive_on_a_normal_window() {
        let n = SessionMemory::tail_budget_from(BudgetConfig::new(128_000, 4096));
        assert!(n > 1, "{n}");
    }
}

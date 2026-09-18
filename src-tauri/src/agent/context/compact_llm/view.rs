//! Opt-in compact-llm session view and [`Compactor`].
//!
//! Runs at the request boundary. Summaries stay in checkpoints; original
//! `messages.jsonl` is read-only for this module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rig::completion::Message;
use rig::memory::ConversationMemory;
use rig_memory::{CompactingMemory, Compactor, HeuristicTokenCounter, MemoryError, TokenCounter};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::branch::CompactLlmBranch;
use super::checkpoint::CompactLlmCheckpointStore;
use super::sandbox::CompactLlmSandbox;
use super::{
    annotate_history_summary, source_slice_sha256, CompactLlmCheckpoint,
    CompactLlmCheckpointStatus, CompactLlmConfig, CompactLlmError, CompactionControl,
};
use crate::agent::context::budget::{estimate_request, BudgetConfig};
use crate::agent::context::message_memory::CodegMessageMemory;
use crate::agent::context::policy::{CodegContextPolicy, RequestScope};
use crate::agent::context::session_memory::LoadedHistory;
use crate::agent::model::CodegLlmClient;

type ViewCompacting = CompactingMemory<CodegMessageMemory, CodegContextPolicy, CompactLlmCompactor>;

/// Artifact spliced by Rig as the leading history message.
#[derive(Clone, Debug)]
pub struct CompactLlmArtifact {
    pub checkpoint: CompactLlmCheckpoint,
    pub view_message: Message,
}

impl From<CompactLlmArtifact> for Message {
    fn from(value: CompactLlmArtifact) -> Self {
        value.view_message
    }
}

/// [`Compactor`] that runs the isolated compact-llm branch and writes pending
/// checkpoints. The session view commits only after the full request budget
/// check passes.
#[derive(Clone)]
pub struct CompactLlmCompactor {
    store: CompactLlmCheckpointStore,
    client: CodegLlmClient,
    config: CompactLlmConfig,
    control: Arc<Mutex<CompactionControl>>,
    last_pending: Arc<Mutex<Option<String>>>,
    epoch: Arc<Mutex<u64>>,
    source_last_seq: Arc<Mutex<u64>>,
}

impl CompactLlmCompactor {
    fn new(
        store: CompactLlmCheckpointStore,
        client: CodegLlmClient,
        config: CompactLlmConfig,
        control: Arc<Mutex<CompactionControl>>,
    ) -> Self {
        Self {
            store,
            client,
            config,
            control,
            last_pending: Arc::new(Mutex::new(None)),
            epoch: Arc::new(Mutex::new(1)),
            source_last_seq: Arc::new(Mutex::new(0)),
        }
    }

    fn set_control(&self, control: CompactionControl) {
        *self.control.lock().unwrap_or_else(|e| e.into_inner()) = control;
    }

    fn set_epoch(&self, epoch: u64) {
        *self.epoch.lock().unwrap_or_else(|e| e.into_inner()) = epoch.max(1);
    }

    fn set_source_last_seq(&self, seq: u64) {
        *self
            .source_last_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = seq;
    }

    fn take_pending_id(&self) -> Option<String> {
        self.last_pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    fn set_pending_id(&self, id: Option<String>) {
        *self.last_pending.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }

    fn control(&self) -> CompactionControl {
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn epoch(&self) -> u64 {
        *self.epoch.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn source_last_seq(&self) -> u64 {
        *self
            .source_last_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    async fn compact_now(
        &self,
        conversation_id: &str,
        evicted: &[Message],
        carry_over: Option<&CompactLlmArtifact>,
    ) -> Result<CompactLlmArtifact, MemoryError> {
        if evicted.is_empty() {
            return Err(MemoryError::Internal(
                "compact-llm received empty evicted prefix".into(),
            ));
        }
        if self.control().is_cancelled() {
            return Err(CompactLlmError::Cancelled.into());
        }

        let carry_cp = carry_over.map(|a| &a.checkpoint);
        match lookup(
            &self.store,
            evicted,
            carry_cp,
            conversation_id,
            self.epoch(),
            &self.config.fingerprint(),
        )? {
            Lookup::Exact(hit) => return self.artifact_from_record(hit),
            Lookup::Prefix {
                stored,
                suffix_start,
            } => {
                let suffix = evicted.get(suffix_start..).ok_or_else(|| {
                    MemoryError::Internal("compact-llm prefix suffix_start out of range".into())
                })?;
                if suffix.is_empty() {
                    return self.artifact_from_record(stored);
                }
                return self
                    .run_branch_pending(
                        conversation_id,
                        suffix,
                        Some(&stored),
                        stored.covers_through_seq,
                    )
                    .await;
            }
            Lookup::Miss => {}
        }

        let origin = carry_cp.map(|c| c.covers_through_seq).unwrap_or(0);
        self.run_branch_pending(conversation_id, evicted, carry_cp, origin)
            .await
    }

    async fn run_branch_pending(
        &self,
        conversation_id: &str,
        new_slice: &[Message],
        previous: Option<&CompactLlmCheckpoint>,
        origin_covers: u64,
    ) -> Result<CompactLlmArtifact, MemoryError> {
        let id = format!("compact-{}", uuid::Uuid::new_v4());
        let summary_text = annotate_history_summary("pending");
        let draft = CompactLlmCheckpoint {
            id: id.clone(),
            previous_id: previous.map(|c| c.id.clone()),
            conversation_id: conversation_id.to_string(),
            epoch: previous.map(|c| c.epoch).unwrap_or_else(|| self.epoch()),
            covers_through_seq: origin_covers.saturating_add(new_slice.len() as u64),
            source_last_seq: self.source_last_seq().max(origin_covers),
            source_slice_sha256: source_slice_sha256(new_slice),
            summary_message: Message::user(summary_text),
            files: Vec::new(),
            fingerprint: self.config.fingerprint(),
            status: CompactLlmCheckpointStatus::Pending,
            estimated_summary_tokens: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let pending_dir = self
            .store
            .write_pending(&draft)
            .await
            .map_err(MemoryError::from)?;
        self.set_pending_id(Some(id.clone()));

        let sandbox = CompactLlmSandbox::new(pending_dir.clone(), &self.config);
        let branch =
            CompactLlmBranch::new(self.client.clone(), self.config.clone(), self.control());
        let carry_text = previous.map(|c| summary_text_of(&c.summary_message));
        let output = match branch
            .run(conversation_id, new_slice, carry_text.as_deref(), &sandbox)
            .await
        {
            Ok(output) => output,
            Err(err) => {
                let _ = self.store.abort_pending(&id).await;
                self.set_pending_id(None);
                return Err(err.into());
            }
        };

        let summary_message = Message::user(output.summary);
        let estimated = HeuristicTokenCounter::openai().count(&summary_message) as u64;
        let mut checkpoint = draft;
        checkpoint.summary_message = summary_message;
        checkpoint.files = output.files;
        checkpoint.estimated_summary_tokens = Some(estimated);
        checkpoint.source_last_seq = self.source_last_seq().max(checkpoint.covers_through_seq);
        self.store
            .write_pending(&checkpoint)
            .await
            .map_err(MemoryError::from)?;

        let view_message = self
            .store
            .expand_summary_at(&checkpoint, &pending_dir)
            .map_err(MemoryError::from)?;
        Ok(CompactLlmArtifact {
            checkpoint,
            view_message,
        })
    }

    fn artifact_from_record(
        &self,
        checkpoint: CompactLlmCheckpoint,
    ) -> Result<CompactLlmArtifact, MemoryError> {
        let files_root = match checkpoint.status {
            CompactLlmCheckpointStatus::Committed => self.store.committed_files_dir(&checkpoint.id),
            CompactLlmCheckpointStatus::Pending => {
                self.set_pending_id(Some(checkpoint.id.clone()));
                self.store.pending_files_dir(&checkpoint.id)
            }
        };
        let view_message = self
            .store
            .expand_summary_at(&checkpoint, &files_root)
            .map_err(MemoryError::from)?;
        Ok(CompactLlmArtifact {
            checkpoint,
            view_message,
        })
    }
}

impl Compactor for CompactLlmCompactor {
    type Artifact = CompactLlmArtifact;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move { self.compact_now(conversation_id, evicted, carry_over).await })
    }
}

/// Request-boundary compact-llm history loader. Owns policy, wrapper, and lock.
#[derive(Clone)]
pub struct CompactLlmSessionView {
    inner: CodegMessageMemory,
    store: CompactLlmCheckpointStore,
    compacting: Arc<ViewCompacting>,
    compactor: CompactLlmCompactor,
    conversation_id: String,
    load_lock: Arc<tokio::sync::Mutex<()>>,
    covers_through_seq: Arc<Mutex<Option<usize>>>,
}

impl CompactLlmSessionView {
    pub fn new(
        inner: CodegMessageMemory,
        client: CodegLlmClient,
        config: CompactLlmConfig,
        tail_budget: usize,
    ) -> Self {
        let store = CompactLlmCheckpointStore::new(inner.clone());
        let control = Arc::new(Mutex::new(CompactionControl::new(CancellationToken::new())));
        let compactor = CompactLlmCompactor::new(store.clone(), client, config, control);
        let policy = CodegContextPolicy::token_window(tail_budget, RequestScope::default());
        let conversation_id = inner.session_id().to_string();
        let compacting = CompactingMemory::new(inner.clone(), policy, compactor.clone());
        Self {
            inner,
            store,
            compacting: Arc::new(compacting),
            compactor,
            conversation_id,
            load_lock: Arc::new(tokio::sync::Mutex::new(())),
            covers_through_seq: Arc::new(Mutex::new(None)),
        }
    }

    pub fn covers_through_seq(&self) -> Option<usize> {
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn bind_scope(&self, scope: RequestScope) {
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = scope.covers_through_seq;
        self.compacting.policy().bind_scope(scope);
    }

    pub fn forget(&self) {
        self.compacting.forget(&self.conversation_id);
        *self
            .covers_through_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub async fn load(
        &self,
        pending: &Message,
        preamble: &str,
        tool_schemas: &[Value],
        budget: BudgetConfig,
        control: CompactionControl,
    ) -> Result<LoadedHistory, MemoryError> {
        let _serial = self.load_lock.lock().await;
        let active = self.store.recover().await.map_err(MemoryError::from)?;
        let committed = self.inner.load_committed().await?;
        let pending_seq = bind_seq_excluding_pending(&committed, pending);
        self.inner.bind_history_before(pending_seq).await;

        let covers = active
            .as_ref()
            .map(|c| c.covers_through_seq as usize)
            .or_else(|| self.covers_through_seq());
        self.compacting.policy().bind_scope(RequestScope {
            pending_prompt: Some(pending.clone()),
            covers_through_seq: covers,
            ..RequestScope::default()
        });

        self.compactor.set_control(control);
        self.compactor
            .set_source_last_seq(pending_seq.saturating_sub(1));
        self.compactor
            .set_epoch(active.as_ref().map(|c| c.epoch).unwrap_or(1));
        self.compactor.set_pending_id(None);

        let loaded =
            match ConversationMemory::load(self.compacting.as_ref(), &self.conversation_id).await {
                Ok(loaded) => loaded,
                Err(err) => {
                    if let Some(id) = self.compactor.take_pending_id() {
                        let _ = self.store.abort_pending(&id).await;
                    }
                    return Err(policy_budget_error(err));
                }
            };

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
        let pending_id = self.compactor.take_pending_id();
        match budget.input_budget() {
            Ok(input) if estimated_tokens > input => {
                if let Some(id) = pending_id {
                    let _ = self.store.abort_pending(&id).await;
                }
                self.compacting.forget(&self.conversation_id);
                return Err(MemoryError::Policy(format!(
                    "context_budget_exceeded: estimated {estimated_tokens} exceeds input budget {input}"
                )));
            }
            Ok(_) => {}
            Err(err) => {
                if let Some(id) = pending_id {
                    let _ = self.store.abort_pending(&id).await;
                }
                self.compacting.forget(&self.conversation_id);
                return Err(MemoryError::Policy(format!(
                    "context_budget_exceeded: {err}"
                )));
            }
        }

        if let Some(id) = pending_id {
            self.store.commit(&id).await.map_err(MemoryError::from)?;
        }

        Ok(LoadedHistory {
            messages: loaded,
            compacted,
            estimated_tokens,
        })
    }
}

enum Lookup {
    Exact(CompactLlmCheckpoint),
    Prefix {
        stored: CompactLlmCheckpoint,
        suffix_start: usize,
    },
    Miss,
}

fn lookup(
    store: &CompactLlmCheckpointStore,
    evicted: &[Message],
    carry_over: Option<&CompactLlmCheckpoint>,
    conversation_id: &str,
    epoch: u64,
    fingerprint: &str,
) -> Result<Lookup, MemoryError> {
    let records = store.load_records().map_err(MemoryError::from)?;
    let origin = carry_over.map(|c| c.covers_through_seq).unwrap_or(0);
    let target = origin.saturating_add(evicted.len() as u64);
    let usable: Vec<&CompactLlmCheckpoint> = records
        .iter()
        .filter(|c| {
            c.conversation_id == conversation_id && c.epoch == epoch && c.fingerprint == fingerprint
        })
        .collect();
    let by_id: HashMap<&str, &CompactLlmCheckpoint> =
        usable.iter().map(|c| (c.id.as_str(), *c)).collect();

    let mut exact = None;
    let mut best_prefix: Option<&CompactLlmCheckpoint> = None;
    for cp in &usable {
        if !verifies(cp, evicted, origin, carry_over, &by_id) {
            continue;
        }
        if cp.covers_through_seq == target {
            exact = Some(*cp);
            break;
        }
        if cp.covers_through_seq > origin && cp.covers_through_seq < target {
            let better = best_prefix
                .map(|p| cp.covers_through_seq > p.covers_through_seq)
                .unwrap_or(true);
            if better {
                best_prefix = Some(*cp);
            }
        }
    }

    if let Some(hit) = exact {
        return Ok(Lookup::Exact(hit.clone()));
    }
    if let Some(stored) = best_prefix {
        let suffix_start = stored.covers_through_seq.saturating_sub(origin) as usize;
        return Ok(Lookup::Prefix {
            stored: stored.clone(),
            suffix_start,
        });
    }
    Ok(Lookup::Miss)
}

fn verifies(
    cp: &CompactLlmCheckpoint,
    evicted: &[Message],
    origin: u64,
    carry_over: Option<&CompactLlmCheckpoint>,
    by_id: &HashMap<&str, &CompactLlmCheckpoint>,
) -> bool {
    if cp.covers_through_seq <= origin {
        return false;
    }
    if cp.covers_through_seq > origin.saturating_add(evicted.len() as u64) {
        return false;
    }
    let Some(chain) = lineage(cp, by_id, carry_over) else {
        return false;
    };
    let mut cursor = origin;
    for node in chain {
        let Some((start, end)) = increment_range(node, by_id, carry_over) else {
            return false;
        };
        if end <= origin {
            continue;
        }
        if start != cursor {
            return false;
        }
        let i0 = (start.saturating_sub(origin)) as usize;
        let i1 = (end.saturating_sub(origin)) as usize;
        if i1 > evicted.len() || i0 >= i1 {
            return false;
        }
        if source_slice_sha256(&evicted[i0..i1]) != node.source_slice_sha256 {
            return false;
        }
        cursor = end;
    }
    cursor == cp.covers_through_seq
}

fn lineage<'a>(
    cp: &'a CompactLlmCheckpoint,
    by_id: &HashMap<&str, &'a CompactLlmCheckpoint>,
    carry_over: Option<&CompactLlmCheckpoint>,
) -> Option<Vec<&'a CompactLlmCheckpoint>> {
    let mut chain = vec![cp];
    let mut cur = cp;
    for _ in 0..1024 {
        let Some(pid) = cur.previous_id.as_deref() else {
            chain.reverse();
            return Some(chain);
        };
        if carry_over.map(|c| c.id.as_str()) == Some(pid) {
            chain.reverse();
            return Some(chain);
        }
        let prev = by_id.get(pid)?;
        if chain.iter().any(|n| n.id == prev.id) {
            return None;
        }
        chain.push(*prev);
        cur = *prev;
    }
    None
}

fn increment_range(
    node: &CompactLlmCheckpoint,
    by_id: &HashMap<&str, &CompactLlmCheckpoint>,
    carry_over: Option<&CompactLlmCheckpoint>,
) -> Option<(u64, u64)> {
    let end = node.covers_through_seq;
    let start = match node.previous_id.as_deref() {
        None => 0,
        Some(pid) => {
            if let Some(prev) = by_id.get(pid) {
                prev.covers_through_seq
            } else if carry_over.map(|c| c.id.as_str()) == Some(pid) {
                carry_over?.covers_through_seq
            } else {
                return None;
            }
        }
    };
    (start < end).then_some((start, end))
}

fn summary_text_of(message: &Message) -> String {
    let Message::User { content } = message else {
        return String::new();
    };
    let mut text = String::new();
    for part in content {
        if let rig::completion::message::UserContent::Text(t) = part {
            text.push_str(&t.text);
        }
    }
    text
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::{completions_client, CodegLlmClient};
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use rig::memory::ConversationMemory;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    fn view_at(dir: &std::path::Path, base: &str, tail: usize) -> CompactLlmSessionView {
        let memory = CodegMessageMemory::with_root(dir, "sess-llm", "/tmp/demo");
        let client =
            CodegLlmClient::Completions(completions_client("sk-test", base).expect("client"));
        let mut config = CompactLlmConfig::enabled("m", "CODEG-COMPACT-PROMPT-MARKER");
        config.max_output_tokens = 128;
        CompactLlmSessionView::new(memory, client, config, tail)
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
                        json!({"text": "ok"})
                    } else {
                        script.remove(0)
                    }
                };
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    compact_sse(&next),
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

    fn compact_sse(script: &Value) -> String {
        let text = script
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("L2-SUMMARY");
        let frames = [
            json!({
                "id": "chatcmpl-compact",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": text },
                    "finish_reason": null
                }]
            })
            .to_string(),
            json!({
                "id": "chatcmpl-compact",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]
            })
            .to_string(),
            "[DONE]".to_string(),
        ];
        let mut out = String::new();
        for payload in &frames {
            if payload == "[DONE]" {
                out.push_str("data: [DONE]\n\n");
            } else {
                out.push_str("data: ");
                out.push_str(payload);
                out.push_str("\n\n");
            }
        }
        out
    }

    async fn seed(memory: &CodegMessageMemory, n: usize) {
        let mut msgs = Vec::new();
        for i in 0..n {
            msgs.push(Message::user(format!("user-{i} {}", "word ".repeat(40))));
            msgs.push(Message::assistant(format!(
                "asst-{i} {}",
                "word ".repeat(40)
            )));
        }
        ConversationMemory::append(memory, memory.session_id(), msgs)
            .await
            .expect("seed");
    }

    #[tokio::test]
    async fn restart_hits_committed_checkpoint_without_llm() {
        let dir = tempfile::tempdir().expect("temp");
        let (base, bodies) =
            spawn_json_completions(vec![json!({"text": "COMPACT-LLM-SUMMARY"})]).await;
        let view = view_at(dir.path(), &base, 32);
        seed(&view.inner, 6).await;
        let pending = Message::user("current-prompt");
        let loaded = view
            .load(
                &pending,
                "preamble",
                &[],
                BudgetConfig::new(128_000, 4096),
                CompactionControl::new(CancellationToken::new()),
            )
            .await
            .expect("load");
        assert!(loaded.compacted, "expected compacted view");
        assert!(
            super::super::is_history_summary_message(&loaded.messages[0]),
            "{:?}",
            loaded.messages[0]
        );
        let first_calls = bodies.lock().expect("bodies").len();
        assert!(first_calls >= 1, "{first_calls}");

        let jsonl = std::fs::read_to_string(view.inner.session_dir().join("messages.jsonl"))
            .expect("jsonl");
        assert!(
            !jsonl.contains("COMPACT-LLM-SUMMARY"),
            "summary must not enter messages.jsonl: {jsonl}"
        );
        assert!(jsonl.contains("user-0"), "{jsonl}");

        let restarted = view_at(dir.path(), &base, 32);
        let loaded2 = restarted
            .load(
                &pending,
                "preamble",
                &[],
                BudgetConfig::new(128_000, 4096),
                CompactionControl::new(CancellationToken::new()),
            )
            .await
            .expect("reload");
        assert!(loaded2.compacted);
        let second_calls = bodies.lock().expect("bodies").len();
        assert_eq!(
            second_calls, first_calls,
            "restart must not call LLM again: {second_calls} vs {first_calls}"
        );
        assert!(restarted
            .inner
            .active_compaction_id()
            .await
            .expect("pointer")
            .is_some());
    }

    #[tokio::test]
    async fn budget_failure_does_not_activate_checkpoint() {
        let dir = tempfile::tempdir().expect("temp");
        let (base, _bodies) =
            spawn_json_completions(vec![json!({"text": "SHOULD-NOT-STAY"})]).await;
        let view = view_at(dir.path(), &base, 32);
        seed(&view.inner, 6).await;
        let pending = Message::user("current-prompt");
        let err = view
            .load(
                &pending,
                "preamble",
                &[],
                BudgetConfig::new(1100, 1),
                CompactionControl::new(CancellationToken::new()),
            )
            .await
            .expect_err("budget");
        let msg = err.to_string();
        assert!(msg.contains("context_budget_exceeded"), "{msg}");
        assert_eq!(
            view.inner.active_compaction_id().await.expect("pointer"),
            None
        );
    }

    #[tokio::test]
    async fn history_summary_user_text_stays_in_jsonl() {
        let dir = tempfile::tempdir().expect("temp");
        let (base, _bodies) = spawn_json_completions(vec![json!({"text": "x"})]).await;
        let view = view_at(dir.path(), &base, usize::MAX / 4);
        let keep = Message::user("历史摘要：please still store me as a real user turn");
        ConversationMemory::append(&view.inner, view.inner.session_id(), vec![keep.clone()])
            .await
            .expect("append");
        let pending = Message::user("next");
        let loaded = view
            .load(
                &pending,
                "preamble",
                &[],
                BudgetConfig::new(128_000, 4096),
                CompactionControl::new(CancellationToken::new()),
            )
            .await
            .expect("load");
        assert!(!loaded.compacted);
        assert_eq!(loaded.messages, vec![keep.clone()]);
        let jsonl = std::fs::read_to_string(view.inner.session_dir().join("messages.jsonl"))
            .expect("jsonl");
        assert!(
            jsonl.contains("please still store me as a real user turn"),
            "{jsonl}"
        );
    }

    #[tokio::test]
    async fn cancelled_control_errors_without_activating() {
        let dir = tempfile::tempdir().expect("temp");
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "SHOULD-NOT-RUN"})]).await;
        let view = view_at(dir.path(), &base, 32);
        seed(&view.inner, 6).await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let pending = Message::user("current-prompt");
        let result = view
            .load(
                &pending,
                "preamble",
                &[],
                BudgetConfig::new(128_000, 4096),
                CompactionControl::new(cancel),
            )
            .await;
        assert!(result.is_err(), "{result:?}");
        assert_eq!(
            view.inner.active_compaction_id().await.expect("pointer"),
            None
        );
        assert!(
            bodies.lock().expect("bodies").is_empty()
                || result.err().map(|e| e.to_string()).is_some(),
            "cancelled compact-llm should not activate"
        );
    }
}

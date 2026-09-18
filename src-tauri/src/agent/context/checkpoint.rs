//! Persist/reuse compaction artifacts across process restarts.
//!
//! Rig 0.42.0 `CompactingMemory` keeps the summary and absorbed watermark in
//! process only. [`CheckpointedCompactor`] implements the same `Compactor`
//! trait so a restart can reuse `{session_dir}/compactions/<id>.json` without
//! rewriting CompactingMemory.
//!
//! Integrator wiring:
//! - `session_dir`: [`CheckpointStore::new`] — files go under
//!   `{session_dir}/compactions/`.
//! - `conversation_id` / epoch: pass `{session_id}` or `{session_id}:{epoch}`
//!   to `CompactingMemory::load`; Rig forwards it to `Compactor::compact`.
//! - model/strategy fingerprint: [`CheckpointedCompactor::with_fingerprint`]
//!   (e.g. `{model}|{compact_prompt_hash}|{token_counter}`).
//! - optional request snapshot: [`CheckpointedCompactor::with_request_snapshot`]
//!   for summary token checks (`Compactor` params do not include kept/prompt).
//!
//! Seq convention: `covers_through_seq` is a 1-based prefix length (the number
//! of original messages absorbed). When `compact` is called with
//! `carry_over = None`, that equals `evicted.len() as u64` — tests use this.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::Message;
use rig_memory::{Compactor, HeuristicTokenCounter, MemoryError, TokenCounter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::compact::CompactArtifact;

const CHECKPOINT_VERSION: u32 = 1;
const INDEX_VERSION: u32 = 1;
const STATUS_COMMITTED: &str = "committed";
const HISTORY_SUMMARY_ZH: &str = "历史摘要：";
const HISTORY_SUMMARY_EN: &str = "History summary:";
const INDEX_FILE: &str = "index.json";

/// Rebuild an inner [`Compactor::Artifact`] from persisted summary text.
///
/// Required so a disk/restart carry_over can be passed into the inner
/// compactor. [`CompactArtifact`] implements this for `LlmCompactor`.
pub trait FromSummaryText: Sized {
    fn from_summary_text(text: &str) -> Self;
}

impl FromSummaryText for CompactArtifact {
    fn from_summary_text(text: &str) -> Self {
        Self {
            summary: text.to_string(),
            files: Vec::new(),
        }
    }
}

/// Disk record under `{session_dir}/compactions/<id>.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactionCheckpoint {
    pub version: u32,
    pub id: String,
    pub previous_id: Option<String>,
    #[serde(default)]
    pub conversation_id: String,
    /// 1-based prefix length of original messages absorbed into this summary.
    pub covers_through_seq: u64,
    pub source_last_seq: u64,
    /// SHA-256 (hex) of the *increment* compacted to produce this record
    /// (the `evicted` slice passed to the inner compact that created it).
    pub source_prefix_sha256: String,
    pub summary_message: Message,
    pub status: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_summary_tokens: Option<u64>,
}

impl From<CompactionCheckpoint> for Message {
    fn from(value: CompactionCheckpoint) -> Self {
        value.summary_message
    }
}

/// Optional immutable request snapshot for budget checks at persist time.
#[derive(Clone, Debug, Default)]
pub struct RequestBudgetSnapshot {
    /// Last seq of the confirmed source snapshot (may exceed demoted length).
    pub source_last_seq: Option<u64>,
    pub max_summary_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CompactionIndex {
    version: u32,
    #[serde(default)]
    active: BTreeMap<String, String>,
}

struct StoreInner {
    cache: HashMap<String, Vec<CompactionCheckpoint>>,
}

/// `{session_dir}/compactions` plus a small active-pointer index.
///
/// `forget_generation` drops the in-memory cache only; it does not delete
/// compaction files or `messages.jsonl`.
#[derive(Clone)]
pub struct CheckpointStore {
    session_dir: PathBuf,
    inner: Arc<Mutex<StoreInner>>,
}

impl CheckpointStore {
    pub fn new(session_dir: impl AsRef<Path>) -> Self {
        Self {
            session_dir: session_dir.as_ref().to_path_buf(),
            inner: Arc::new(Mutex::new(StoreInner {
                cache: HashMap::new(),
            })),
        }
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn compactions_dir(&self) -> PathBuf {
        self.session_dir.join("compactions")
    }

    /// Drop the in-memory index for `conversation_id`. Disk artifacts remain.
    pub fn forget_generation(&self, conversation_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.cache.remove(conversation_id);
        }
    }

    pub fn forget_all(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.cache.clear();
        }
    }

    pub fn active_id(&self, conversation_id: &str) -> Option<String> {
        let path = self.compactions_dir().join(INDEX_FILE);
        let raw = std::fs::read_to_string(path).ok()?;
        let index: CompactionIndex = serde_json::from_str(&raw).ok()?;
        index.active.get(conversation_id).cloned()
    }

    fn load_committed(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<CompactionCheckpoint>, MemoryError> {
        {
            let guard = self
                .inner
                .lock()
                .map_err(|e| MemoryError::Internal(e.to_string()))?;
            if let Some(cached) = guard.cache.get(conversation_id) {
                return Ok(cached.clone());
            }
        }
        let loaded = self.read_disk(conversation_id)?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| MemoryError::Internal(e.to_string()))?;
        guard
            .cache
            .insert(conversation_id.to_string(), loaded.clone());
        Ok(loaded)
    }

    fn read_disk(&self, conversation_id: &str) -> Result<Vec<CompactionCheckpoint>, MemoryError> {
        let dir = self.compactions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir).map_err(MemoryError::backend)?;
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(MemoryError::backend)?;
            let path = entry.path();
            if !is_checkpoint_file(&path) {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "skip unreadable compaction checkpoint"
                    );
                    continue;
                }
            };
            let parsed: CompactionCheckpoint = match serde_json::from_str(&raw) {
                Ok(parsed) => parsed,
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "skip malformed compaction checkpoint"
                    );
                    continue;
                }
            };
            if parsed.conversation_id == conversation_id
                && parsed.status == STATUS_COMMITTED
                && parsed.version == CHECKPOINT_VERSION
            {
                out.push(parsed);
            }
        }
        Ok(out)
    }

    fn persist(&self, checkpoint: &CompactionCheckpoint) -> Result<(), MemoryError> {
        let dir = self.compactions_dir();
        std::fs::create_dir_all(&dir).map_err(MemoryError::backend)?;
        let path = dir.join(format!("{}.json", checkpoint.id));
        let bytes = serde_json::to_vec_pretty(checkpoint)
            .map_err(|e| MemoryError::Internal(format!("serialize compaction checkpoint: {e}")))?;
        atomic_write(&path, &bytes)?;

        if let Err(err) = self.write_active(&checkpoint.conversation_id, &checkpoint.id) {
            tracing::warn!(
                conversation_id = %checkpoint.conversation_id,
                error = %err,
                "compaction checkpoint persisted; active index not updated"
            );
        }

        let mut guard = self
            .inner
            .lock()
            .map_err(|e| MemoryError::Internal(e.to_string()))?;
        let entries = guard
            .cache
            .entry(checkpoint.conversation_id.clone())
            .or_default();
        if let Some(existing) = entries.iter_mut().find(|c| c.id == checkpoint.id) {
            *existing = checkpoint.clone();
        } else {
            entries.push(checkpoint.clone());
        }
        Ok(())
    }

    fn write_active(&self, conversation_id: &str, id: &str) -> Result<(), MemoryError> {
        let dir = self.compactions_dir();
        let path = dir.join(INDEX_FILE);
        let mut index = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(CompactionIndex {
                version: INDEX_VERSION,
                active: BTreeMap::new(),
            });
        index.version = INDEX_VERSION;
        index
            .active
            .insert(conversation_id.to_string(), id.to_string());
        let bytes = serde_json::to_vec_pretty(&index)
            .map_err(|e| MemoryError::Internal(format!("serialize compaction index: {e}")))?;
        atomic_write(&path, &bytes)
    }
}

/// Disk-backed [`Compactor`] decorator. Generic over any inner `C: Compactor`
/// whose [`Compactor::Artifact`] implements [`FromSummaryText`].
pub struct CheckpointedCompactor<C: Compactor> {
    inner: C,
    store: CheckpointStore,
    fingerprint: String,
    snapshot: Option<RequestBudgetSnapshot>,
}

impl<C: Compactor> CheckpointedCompactor<C> {
    pub fn new(inner: C, store: CheckpointStore) -> Self {
        Self {
            inner,
            store,
            fingerprint: String::new(),
            snapshot: None,
        }
    }

    pub fn with_fingerprint(mut self, fingerprint: impl Into<String>) -> Self {
        self.fingerprint = fingerprint.into();
        self
    }

    pub fn with_request_snapshot(mut self, snapshot: RequestBudgetSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn store(&self) -> &CheckpointStore {
        &self.store
    }
}

impl<C> Compactor for CheckpointedCompactor<C>
where
    C: Compactor,
    C::Artifact: FromSummaryText,
{
    type Artifact = CompactionCheckpoint;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move { self.compact_now(conversation_id, evicted, carry_over).await })
    }
}

impl<C> CheckpointedCompactor<C>
where
    C: Compactor,
    C::Artifact: FromSummaryText,
{
    async fn compact_now(
        &self,
        conversation_id: &str,
        evicted: &[Message],
        carry_over: Option<&CompactionCheckpoint>,
    ) -> Result<CompactionCheckpoint, MemoryError> {
        if evicted.is_empty() {
            return Err(MemoryError::Internal(
                "checkpointed compact received empty evicted prefix".into(),
            ));
        }

        let records = self.store.load_committed(conversation_id)?;
        match lookup(
            &records,
            evicted,
            carry_over,
            conversation_id,
            &self.fingerprint,
        ) {
            Lookup::Exact(hit) => {
                if valid_summary(&hit.summary_message) {
                    return Ok(hit);
                }
            }
            Lookup::Prefix {
                stored,
                suffix_start,
            } => {
                let suffix = evicted.get(suffix_start..).ok_or_else(|| {
                    MemoryError::Internal("compaction prefix suffix_start out of range".into())
                })?;
                if suffix.is_empty() {
                    return Ok(stored);
                }
                let recovered = recover_inner::<C::Artifact>(&stored);
                return self
                    .run_inner_and_persist(
                        conversation_id,
                        suffix,
                        Some(&recovered),
                        Some(stored.id.clone()),
                        stored.covers_through_seq,
                    )
                    .await;
            }
            Lookup::Miss => {}
        }

        let recovered = carry_over.map(recover_inner::<C::Artifact>);
        let previous_id = carry_over.map(|c| c.id.clone());
        let origin = carry_over.map(|c| c.covers_through_seq).unwrap_or(0);
        self.run_inner_and_persist(
            conversation_id,
            evicted,
            recovered.as_ref(),
            previous_id,
            origin,
        )
        .await
    }

    async fn run_inner_and_persist(
        &self,
        conversation_id: &str,
        inner_evicted: &[Message],
        inner_carry: Option<&C::Artifact>,
        previous_id: Option<String>,
        origin_covers: u64,
    ) -> Result<CompactionCheckpoint, MemoryError> {
        let inner_artifact = self
            .inner
            .compact(conversation_id, inner_evicted, inner_carry)
            .await?;

        let summary_message = labeled_user_summary(inner_artifact.clone());
        if !valid_summary(&summary_message) {
            return Err(MemoryError::Internal(
                "inner compact produced an empty or non-user history summary".into(),
            ));
        }

        let covers_through_seq = origin_covers.saturating_add(inner_evicted.len() as u64);
        let estimated = estimate_summary_tokens(&summary_message);
        if let Some(max) = self.snapshot.as_ref().and_then(|s| s.max_summary_tokens) {
            if estimated > max {
                return Err(MemoryError::Policy(format!(
                    "context_budget_exceeded: summary tokens {estimated} exceed max {max}"
                )));
            }
        }

        let source_last_seq = self
            .snapshot
            .as_ref()
            .and_then(|s| s.source_last_seq)
            .unwrap_or(covers_through_seq);

        let checkpoint = CompactionCheckpoint {
            version: CHECKPOINT_VERSION,
            id: format!("compact-{}", uuid::Uuid::new_v4()),
            previous_id,
            conversation_id: conversation_id.to_string(),
            covers_through_seq,
            source_last_seq,
            source_prefix_sha256: prefix_sha256(inner_evicted)?,
            summary_message,
            status: STATUS_COMMITTED.to_string(),
            fingerprint: self.fingerprint.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            estimated_summary_tokens: Some(estimated),
        };
        self.store.persist(&checkpoint)?;
        Ok(checkpoint)
    }
}

enum Lookup {
    Exact(CompactionCheckpoint),
    Prefix {
        stored: CompactionCheckpoint,
        suffix_start: usize,
    },
    Miss,
}

fn lookup(
    records: &[CompactionCheckpoint],
    evicted: &[Message],
    carry_over: Option<&CompactionCheckpoint>,
    conversation_id: &str,
    fingerprint: &str,
) -> Lookup {
    let origin = carry_over.map(|c| c.covers_through_seq).unwrap_or(0);
    let target = origin.saturating_add(evicted.len() as u64);
    let usable: Vec<&CompactionCheckpoint> = records
        .iter()
        .filter(|c| {
            c.conversation_id == conversation_id
                && c.fingerprint == fingerprint
                && c.status == STATUS_COMMITTED
        })
        .collect();
    let by_id: HashMap<&str, &CompactionCheckpoint> =
        usable.iter().map(|c| (c.id.as_str(), *c)).collect();

    let mut exact = None;
    let mut best_prefix: Option<&CompactionCheckpoint> = None;
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
        return Lookup::Exact(hit.clone());
    }
    if let Some(stored) = best_prefix {
        let suffix_start = stored.covers_through_seq.saturating_sub(origin) as usize;
        return Lookup::Prefix {
            stored: stored.clone(),
            suffix_start,
        };
    }
    Lookup::Miss
}

fn verifies(
    cp: &CompactionCheckpoint,
    evicted: &[Message],
    origin: u64,
    carry_over: Option<&CompactionCheckpoint>,
    by_id: &HashMap<&str, &CompactionCheckpoint>,
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
        if end > cp.covers_through_seq {
            return false;
        }
        let i0 = (start.saturating_sub(origin)) as usize;
        let i1 = (end.saturating_sub(origin)) as usize;
        if i1 > evicted.len() || i0 >= i1 {
            return false;
        }
        let Ok(hash) = prefix_sha256(&evicted[i0..i1]) else {
            return false;
        };
        if hash != node.source_prefix_sha256 {
            return false;
        }
        cursor = end;
    }
    cursor == cp.covers_through_seq
}

fn lineage<'a>(
    cp: &'a CompactionCheckpoint,
    by_id: &HashMap<&str, &'a CompactionCheckpoint>,
    carry_over: Option<&CompactionCheckpoint>,
) -> Option<Vec<&'a CompactionCheckpoint>> {
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
    node: &CompactionCheckpoint,
    by_id: &HashMap<&str, &CompactionCheckpoint>,
    carry_over: Option<&CompactionCheckpoint>,
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
    if start >= end {
        return None;
    }
    Some((start, end))
}

fn recover_inner<A: FromSummaryText>(checkpoint: &CompactionCheckpoint) -> A {
    A::from_summary_text(strip_summary_label(&message_plain_text(
        &checkpoint.summary_message,
    )))
}

fn labeled_user_summary(artifact: impl Into<Message>) -> Message {
    let text = message_plain_text(&artifact.into());
    let trimmed = text.trim();
    let body = if trimmed.starts_with(HISTORY_SUMMARY_ZH) || trimmed.starts_with(HISTORY_SUMMARY_EN)
    {
        trimmed.to_string()
    } else {
        format!("{HISTORY_SUMMARY_ZH}{trimmed}")
    };
    Message::user(body)
}

fn strip_summary_label(text: &str) -> &str {
    let trimmed = text.trim();
    trimmed
        .strip_prefix(HISTORY_SUMMARY_ZH)
        .or_else(|| trimmed.strip_prefix(HISTORY_SUMMARY_EN))
        .unwrap_or(trimmed)
        .trim()
}

fn valid_summary(message: &Message) -> bool {
    matches!(message, Message::User { content } if !content.is_empty())
        && !message_plain_text(message).trim().is_empty()
}

fn message_plain_text(message: &Message) -> String {
    match message {
        Message::System { content } => content.clone(),
        Message::User { content } => content
            .iter()
            .filter_map(|part| match part {
                UserContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Message::Assistant { content, .. } => content
            .iter()
            .filter_map(|part| match part {
                AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn prefix_sha256(messages: &[Message]) -> Result<String, MemoryError> {
    let bytes = serde_json::to_vec(messages)
        .map_err(|e| MemoryError::Internal(format!("hash compaction prefix: {e}")))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn estimate_summary_tokens(message: &Message) -> u64 {
    HeuristicTokenCounter::openai().count(message) as u64
}

fn is_checkpoint_file(path: &Path) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name == INDEX_FILE {
        return false;
    }
    true
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    let parent = path.parent().ok_or_else(|| {
        MemoryError::Internal(format!("checkpoint path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent).map_err(MemoryError::backend)?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("checkpoint.json"),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(MemoryError::backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::wasm_compat::WasmBoxedFuture;
    use serde_json::Value;

    #[derive(Clone, Debug, PartialEq)]
    struct FakeArtifact {
        text: String,
    }

    impl From<FakeArtifact> for Message {
        fn from(value: FakeArtifact) -> Self {
            Message::system(value.text)
        }
    }

    impl FromSummaryText for FakeArtifact {
        fn from_summary_text(text: &str) -> Self {
            Self {
                text: text.to_string(),
            }
        }
    }

    #[derive(Clone, Debug)]
    struct FakeCall {
        evicted: Vec<Message>,
        carry_over: Option<FakeArtifact>,
    }

    #[derive(Clone)]
    struct FakeCompactor {
        calls: Arc<Mutex<Vec<FakeCall>>>,
        fail_on_call: Arc<Mutex<Option<usize>>>,
    }

    impl FakeCompactor {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_on_call: Arc::new(Mutex::new(None)),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().expect("calls").len()
        }

        fn fail_on(&self, n: usize) {
            *self.fail_on_call.lock().expect("fail_on") = Some(n);
        }

        fn recorded(&self) -> Vec<FakeCall> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl Compactor for FakeCompactor {
        type Artifact = FakeArtifact;

        fn compact<'a>(
            &'a self,
            _conversation_id: &'a str,
            evicted: &'a [Message],
            carry_over: Option<&'a Self::Artifact>,
        ) -> WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
            Box::pin(async move {
                let mut calls = self.calls.lock().expect("calls");
                calls.push(FakeCall {
                    evicted: evicted.to_vec(),
                    carry_over: carry_over.cloned(),
                });
                let n = calls.len();
                drop(calls);
                if *self.fail_on_call.lock().expect("fail_on") == Some(n) {
                    return Err(MemoryError::Internal("inner compact failed".into()));
                }
                let prev = carry_over.map(|a| a.text.as_str()).unwrap_or("none");
                Ok(FakeArtifact {
                    text: format!("inner[{n}|{prev}|{}]", evicted.len()),
                })
            })
        }
    }

    fn msgs(texts: &[&str]) -> Vec<Message> {
        texts.iter().copied().map(Message::user).collect()
    }

    fn summary_text(cp: &CompactionCheckpoint) -> String {
        message_plain_text(&cp.summary_message)
    }

    fn assert_labeled_user(cp: &CompactionCheckpoint) {
        assert!(
            matches!(cp.summary_message, Message::User { .. }),
            "summary must be Message::User, got {:?}",
            cp.summary_message
        );
        let text = summary_text(cp);
        assert!(
            text.starts_with(HISTORY_SUMMARY_ZH) || text.starts_with(HISTORY_SUMMARY_EN),
            "summary must be labeled as history summary: {text}"
        );
        assert!(!text.trim().is_empty());
    }

    fn committed_files(dir: &Path) -> Vec<Value> {
        let root = dir.join("compactions");
        let mut out = Vec::new();
        if !root.exists() {
            return out;
        }
        for entry in std::fs::read_dir(&root).expect("read compactions") {
            let path = entry.expect("entry").path();
            if !is_checkpoint_file(&path) {
                continue;
            }
            let raw = std::fs::read_to_string(&path).expect("read checkpoint");
            out.push(serde_json::from_str(&raw).expect("json"));
        }
        out
    }

    #[tokio::test]
    async fn exact_checkpoint_hit_does_not_call_inner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CheckpointStore::new(dir.path());
        let inner = FakeCompactor::new();
        let compactor =
            CheckpointedCompactor::new(inner.clone(), store).with_fingerprint("model|strategy");
        let evicted = msgs(&["a", "b", "c"]);

        let first = Compactor::compact(&compactor, "conv:epoch-1", &evicted, None)
            .await
            .expect("first compact");
        assert_eq!(inner.call_count(), 1);
        assert_labeled_user(&first);
        assert_eq!(first.covers_through_seq, evicted.len() as u64);

        inner.calls.lock().expect("reset").clear();
        let again = Compactor::compact(&compactor, "conv:epoch-1", &evicted, None)
            .await
            .expect("exact hit");
        assert_eq!(inner.call_count(), 0);
        assert_eq!(again.id, first.id);
        assert_eq!(again.summary_message, first.summary_message);
    }

    #[tokio::test]
    async fn restart_same_prefix_hits_disk_without_inner_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inner1 = FakeCompactor::new();
        let store1 = CheckpointStore::new(dir.path());
        let c1 = CheckpointedCompactor::new(inner1.clone(), store1).with_fingerprint("fp");
        let evicted = msgs(&["one", "two"]);
        let first = Compactor::compact(&c1, "sess:e0", &evicted, None)
            .await
            .expect("seed");
        assert_eq!(inner1.call_count(), 1);
        drop(c1);

        let inner2 = FakeCompactor::new();
        let store2 = CheckpointStore::new(dir.path());
        let c2 = CheckpointedCompactor::new(inner2.clone(), store2).with_fingerprint("fp");
        let hit = Compactor::compact(&c2, "sess:e0", &evicted, None)
            .await
            .expect("restart hit");
        assert_eq!(inner2.call_count(), 0);
        assert_eq!(hit.id, first.id);
        assert_eq!(hit.covers_through_seq, 2);
    }

    #[tokio::test]
    async fn partial_coverage_calls_inner_with_suffix_and_stored_carry_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CheckpointStore::new(dir.path());
        let inner = FakeCompactor::new();
        let compactor = CheckpointedCompactor::new(inner.clone(), store.clone());
        let prefix = msgs(&["m1", "m2"]);
        let full = msgs(&["m1", "m2", "m3", "m4"]);

        let stored = Compactor::compact(&compactor, "c", &prefix, None)
            .await
            .expect("prefix");
        assert_eq!(stored.covers_through_seq, 2);
        assert_eq!(inner.call_count(), 1);

        let combined = Compactor::compact(&compactor, "c", &full, None)
            .await
            .expect("extend");
        assert_eq!(inner.call_count(), 2);
        assert_eq!(combined.covers_through_seq, 4);
        assert_eq!(combined.previous_id.as_deref(), Some(stored.id.as_str()));
        assert_labeled_user(&combined);

        let calls = inner.recorded();
        assert_eq!(calls[1].evicted, msgs(&["m3", "m4"]));
        let carry = calls[1]
            .carry_over
            .as_ref()
            .expect("stored artifact as carry_over");
        assert_eq!(carry.text, strip_summary_label(&summary_text(&stored)));

        inner.calls.lock().expect("reset").clear();
        let restarted = CheckpointedCompactor::new(inner.clone(), CheckpointStore::new(dir.path()));
        let hit = Compactor::compact(&restarted, "c", &full, None)
            .await
            .expect("chained exact after restart");
        assert_eq!(inner.call_count(), 0);
        assert_eq!(hit.id, combined.id);
    }

    #[tokio::test]
    async fn persist_then_drop_wrapper_retries_same_prefix_without_second_inner_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let inner = FakeCompactor::new();
        let store = CheckpointStore::new(dir.path());
        let evicted = msgs(&["p", "q", "r"]);

        let first_wrapper = CheckpointedCompactor::new(inner.clone(), store.clone());
        let _ = Compactor::compact(&first_wrapper, "conv", &evicted, None)
            .await
            .expect("persist");
        assert_eq!(inner.call_count(), 1);
        drop(first_wrapper);

        let retry = CheckpointedCompactor::new(inner.clone(), store);
        let _ = Compactor::compact(&retry, "conv", &evicted, None)
            .await
            .expect("retry hits disk");
        assert_eq!(inner.call_count(), 1);
        assert_eq!(committed_files(dir.path()).len(), 1);
    }

    #[tokio::test]
    async fn hash_mismatch_ignores_old_checkpoint_and_calls_inner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CheckpointStore::new(dir.path());
        let inner = FakeCompactor::new();
        let compactor = CheckpointedCompactor::new(inner.clone(), store);
        let first_msgs = msgs(&["alpha", "beta"]);
        let fork_msgs = msgs(&["gamma", "delta"]);

        let old = Compactor::compact(&compactor, "conv", &first_msgs, None)
            .await
            .expect("old");
        let forked = Compactor::compact(&compactor, "conv", &fork_msgs, None)
            .await
            .expect("fork");
        assert_eq!(inner.call_count(), 2);
        assert_ne!(forked.id, old.id);
        assert_ne!(forked.summary_message, old.summary_message);
        assert_eq!(forked.previous_id, None);

        let files = committed_files(dir.path());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|v| v["id"] == old.id));
        assert!(files.iter().any(|v| v["id"] == forked.id));
        assert!(files.iter().all(|v| v["status"] == STATUS_COMMITTED));
        assert!(files.iter().all(|v| v["summary_message"]["role"] == "user"));
    }

    #[tokio::test]
    async fn inner_error_leaves_previous_committed_checkpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CheckpointStore::new(dir.path());
        let inner = FakeCompactor::new();
        inner.fail_on(2);
        let compactor = CheckpointedCompactor::new(inner.clone(), store.clone());
        let prefix = msgs(&["a", "b"]);
        let longer = msgs(&["a", "b", "c"]);

        let committed = Compactor::compact(&compactor, "conv", &prefix, None)
            .await
            .expect("committed");
        let before = committed_files(dir.path());
        assert_eq!(before.len(), 1);
        assert_eq!(before[0]["id"], committed.id);

        let err = Compactor::compact(&compactor, "conv", &longer, None)
            .await
            .expect_err("inner fails");
        assert!(err.to_string().contains("inner compact failed"), "{err}");
        assert_eq!(inner.call_count(), 2);

        let after = committed_files(dir.path());
        assert_eq!(after.len(), 1);
        assert_eq!(after[0]["id"], committed.id);
        assert_eq!(
            store.active_id("conv").as_deref(),
            Some(committed.id.as_str())
        );

        inner.calls.lock().expect("reset").clear();
        let hit = Compactor::compact(&compactor, "conv", &prefix, None)
            .await
            .expect("previous still hittable");
        assert_eq!(inner.call_count(), 0);
        assert_eq!(hit.id, committed.id);
    }

    #[tokio::test]
    async fn forget_generation_drops_cache_not_messages_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let messages = dir.path().join("messages.jsonl");
        std::fs::write(&messages, "{\"role\":\"user\",\"content\":[]}\n").expect("messages");
        let store = CheckpointStore::new(dir.path());
        let inner = FakeCompactor::new();
        let compactor = CheckpointedCompactor::new(inner.clone(), store.clone());
        let evicted = msgs(&["keep"]);
        let _ = Compactor::compact(&compactor, "conv", &evicted, None)
            .await
            .expect("persist");
        store.forget_generation("conv");
        assert!(messages.exists(), "forget must not touch messages.jsonl");
        inner.calls.lock().expect("reset").clear();
        let _ = Compactor::compact(&compactor, "conv", &evicted, None)
            .await
            .expect("disk still used after cache drop");
        assert_eq!(inner.call_count(), 0);
    }
}

//! CodegMessageMemory: Rig `ConversationMemory` over a per-session directory.
//!
//! Layout:
//! `<root>/<encoded-cwd>/<session-id>/{session.json,messages.jsonl,runtime.jsonl}`
//!
//! `messages.jsonl` is the only conversation body: one full Rig `Message` per
//! line. `runtime.jsonl` records prepare/commit (and run ids / ordinals), not
//! a second transcript. Seq is the 1-based physical line number of a committed
//! message and is never written into the `Message` object.
//!
//! [`ConversationMemory::clear`] wipes this session's messages and cursor. It
//! exists for tests and explicit reset; never use it as compaction.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rig::completion::Message;
use rig::memory::{ConversationMemory, MemoryError};
use rig::wasm_compat::WasmBoxedFuture;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::agent::context::hydrate::encode_session_cwd;

const STORAGE_VERSION: u32 = 2;
const MESSAGE_FORMAT: &str = "rig::completion::Message";
const RIG_VERSION: &str = "0.42.0";
const SESSION_FILE: &str = "session.json";
const SESSION_TMP_FILE: &str = "session.json.tmp";
const MESSAGES_FILE: &str = "messages.jsonl";
const RUNTIME_FILE: &str = "runtime.jsonl";

/// Per-session identity, format version, and committed-message cursor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionMeta {
    pub storage_version: u32,
    pub message_format: String,
    pub rig_version: String,
    pub session_id: String,
    pub cwd: String,
    pub committed_message_count: u64,
    #[serde(default)]
    pub active_compaction_id: Option<String>,
}

impl SessionMeta {
    fn new(session_id: String, cwd: String) -> Self {
        Self {
            storage_version: STORAGE_VERSION,
            message_format: MESSAGE_FORMAT.to_string(),
            rig_version: RIG_VERSION.to_string(),
            session_id,
            cwd,
            committed_message_count: 0,
            active_compaction_id: None,
        }
    }
}

/// JSONL [`ConversationMemory`] bound to one Codeg Agent session directory.
#[derive(Clone)]
pub struct CodegMessageMemory {
    session_dir: PathBuf,
    session_id: String,
    cwd: String,
    write_lock: Arc<tokio::sync::Mutex<()>>,
    /// Exclusive upper bound (1-based seq) applied by [`ConversationMemory::load`].
    history_before_seq: Arc<Mutex<Option<u64>>>,
}

/// Incremental saver for one user-instruction run over [`CodegMessageMemory`].
#[derive(Clone)]
pub struct RunRecorder {
    memory: CodegMessageMemory,
    active: Arc<Mutex<Option<ActiveRun>>>,
}

#[derive(Clone)]
struct ActiveRun {
    run_id: String,
}

/// Handle returned by [`RunRecorder::begin_run`].
#[derive(Clone)]
pub struct RunHandle {
    memory: CodegMessageMemory,
    run_id: String,
    prompt_seq: u64,
}

#[derive(Debug, thiserror::Error)]
enum StoreError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

fn backend(err: impl Into<StoreError>) -> MemoryError {
    MemoryError::backend(err.into())
}

fn other(msg: impl Into<String>) -> MemoryError {
    MemoryError::backend(StoreError::Other(msg.into()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn session_lock(session_dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = locks.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = map.get(session_dir).and_then(Weak::upgrade) {
        return existing;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    map.insert(session_dir.to_path_buf(), Arc::downgrade(&lock));
    lock
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum RuntimeOp {
    Prepare {
        operation_id: String,
        #[serde(default)]
        run_id: Option<String>,
        ordinals: Vec<u64>,
        expected_offset: u64,
        count: u64,
        byte_len: u64,
        content_hashes: Vec<String>,
    },
    Commit {
        operation_id: String,
        #[serde(default)]
        run_id: Option<String>,
        committed_message_count: u64,
        ordinals: Vec<u64>,
        count: u64,
    },
    Abort {
        operation_id: String,
        reason: String,
    },
    BeginRun {
        run_id: String,
        prompt_seq: u64,
    },
}

struct AppendOutcome {
    written: u64,
    first_seq: u64,
}

impl CodegMessageMemory {
    /// Session under [`crate::paths::codeg_agent_sessions_root`].
    pub fn new(session_id: impl Into<String>, cwd: impl AsRef<str>) -> Self {
        Self::with_root(crate::paths::codeg_agent_sessions_root(), session_id, cwd)
    }

    /// Session under an injected root (tests and alternate data dirs).
    pub fn with_root(
        root: impl Into<PathBuf>,
        session_id: impl Into<String>,
        cwd: impl AsRef<str>,
    ) -> Self {
        let session_id = session_id.into();
        let cwd = cwd.as_ref().to_string();
        let session_dir = root.into().join(encode_session_cwd(&cwd)).join(&session_id);
        let write_lock = session_lock(&session_dir);
        Self {
            session_dir,
            session_id,
            cwd,
            write_lock,
            history_before_seq: Arc::new(Mutex::new(None)),
        }
    }

    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Exclude `seq` (1-based) and later from [`ConversationMemory::load`].
    pub async fn bind_history_before(&self, seq: u64) {
        let _guard = self.write_lock.lock().await;
        self.bind_history_before_locked(seq);
    }

    /// All committed messages, including a prompt saved after the history bound.
    pub async fn load_committed(&self) -> Result<Vec<Message>, MemoryError> {
        let _guard = self.write_lock.lock().await;
        self.load_committed_locked()
    }

    pub async fn load_before_seq(&self, seq: u64) -> Result<Vec<Message>, MemoryError> {
        let all = self.load_committed().await?;
        let n = seq.saturating_sub(1) as usize;
        Ok(all.into_iter().take(n).collect())
    }

    pub async fn meta(&self) -> Result<SessionMeta, MemoryError> {
        let _guard = self.write_lock.lock().await;
        self.recover_locked()
    }

    /// `session.json.active_compaction_id`. Compact-llm treats this as the
    /// authoritative live pointer; compaction index files are rebuildable.
    pub async fn active_compaction_id(&self) -> Result<Option<String>, MemoryError> {
        Ok(self.meta().await?.active_compaction_id)
    }

    /// Atomically set `session.json.active_compaction_id`. Does not rewrite
    /// `messages.jsonl` or run ordinals.
    pub async fn set_active_compaction_id(&self, id: Option<String>) -> Result<(), MemoryError> {
        let _guard = self.write_lock.lock().await;
        let mut meta = self.recover_locked()?;
        if meta.active_compaction_id == id {
            return Ok(());
        }
        meta.active_compaction_id = id;
        self.write_session_meta(&meta)
    }

    /// Append `messages` at `start_ordinal` for `run_id`.
    ///
    /// Idempotent on `(run_id, message_ordinal)`: already-committed ordinals
    /// in the batch are skipped. Seq is not stored on the `Message`.
    pub async fn append_run_messages(
        &self,
        run_id: &str,
        start_ordinal: u64,
        messages: Vec<Message>,
    ) -> Result<u64, MemoryError> {
        let _guard = self.write_lock.lock().await;
        let outcome = self.append_locked(Some(run_id), start_ordinal, &messages)?;
        Ok(outcome.written)
    }

    fn bind_history_before_locked(&self, seq: u64) {
        *self
            .history_before_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(seq);
    }

    fn history_before_locked(&self) -> Option<u64> {
        *self
            .history_before_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn session_json_path(&self) -> PathBuf {
        self.session_dir.join(SESSION_FILE)
    }

    fn messages_path(&self) -> PathBuf {
        self.session_dir.join(MESSAGES_FILE)
    }

    fn runtime_path(&self) -> PathBuf {
        self.session_dir.join(RUNTIME_FILE)
    }

    fn ensure_dir(&self) -> Result<(), MemoryError> {
        fs::create_dir_all(&self.session_dir).map_err(backend)
    }

    fn load_committed_locked(&self) -> Result<Vec<Message>, MemoryError> {
        let meta = self.recover_locked()?;
        self.read_committed_messages(meta.committed_message_count)
    }

    fn load_for_runner_locked(&self) -> Result<Vec<Message>, MemoryError> {
        let msgs = self.load_committed_locked()?;
        match self.history_before_locked() {
            Some(seq) if seq > 0 => {
                let n = (seq - 1) as usize;
                Ok(msgs.into_iter().take(n).collect())
            }
            _ => Ok(msgs),
        }
    }

    fn read_or_default_meta(&self) -> Result<SessionMeta, MemoryError> {
        let path = self.session_json_path();
        if !path.exists() {
            return Ok(SessionMeta::new(self.session_id.clone(), self.cwd.clone()));
        }
        let raw = fs::read_to_string(&path).map_err(backend)?;
        let meta: SessionMeta = serde_json::from_str(&raw).map_err(backend)?;
        if meta.storage_version != STORAGE_VERSION {
            return Err(other(format!(
                "unsupported storage_version {} (expected {STORAGE_VERSION}); refusing to rewrite",
                meta.storage_version
            )));
        }
        if meta.message_format != MESSAGE_FORMAT {
            return Err(other(format!(
                "unsupported message_format {} (expected {MESSAGE_FORMAT})",
                meta.message_format
            )));
        }
        if meta.session_id != self.session_id {
            return Err(other(format!(
                "session.json session_id {} does not match {}",
                meta.session_id, self.session_id
            )));
        }
        Ok(meta)
    }

    fn write_session_meta(&self, meta: &SessionMeta) -> Result<(), MemoryError> {
        self.ensure_dir()?;
        let dest = self.session_json_path();
        let tmp = self.session_dir.join(SESSION_TMP_FILE);
        let data = serde_json::to_vec_pretty(meta).map_err(backend)?;
        {
            let mut file = File::create(&tmp).map_err(backend)?;
            file.write_all(&data).map_err(backend)?;
            file.flush().map_err(backend)?;
            file.sync_all().map_err(backend)?;
        }
        fs::rename(&tmp, &dest).map_err(backend)?;
        sync_dir(&self.session_dir);
        Ok(())
    }

    fn write_runtime_op(&self, op: &RuntimeOp) -> Result<(), MemoryError> {
        self.ensure_dir()?;
        let line = serde_json::to_string(op).map_err(backend)?;
        append_synced(&self.runtime_path(), format!("{line}\n").as_bytes())
    }

    fn read_runtime_ops(&self) -> Result<Vec<RuntimeOp>, MemoryError> {
        let path = self.runtime_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&path).map_err(backend)?;
        let reader = BufReader::new(file);
        let mut ops = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(backend)?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(op) = parse_runtime_line(&line)? {
                ops.push(op);
            }
        }
        Ok(ops)
    }

    fn recover_locked(&self) -> Result<SessionMeta, MemoryError> {
        if !self.session_dir.exists()
            && !self.session_json_path().exists()
            && !self.messages_path().exists()
            && !self.runtime_path().exists()
        {
            return Ok(SessionMeta::new(self.session_id.clone(), self.cwd.clone()));
        }
        self.ensure_dir()?;
        let mut meta = self.read_or_default_meta()?;
        let ops = self.read_runtime_ops()?;
        let finished = finished_operation_ids(&ops);
        let unmatched: Vec<RuntimeOp> = ops
            .iter()
            .filter(|op| matches!(op, RuntimeOp::Prepare { operation_id, .. } if !finished.contains(operation_id)))
            .cloned()
            .collect();
        for op in unmatched {
            self.recover_prepare(&mut meta, &op)?;
        }
        let ops = self.read_runtime_ops()?;
        let mut max_commit = meta.committed_message_count;
        for op in &ops {
            if let RuntimeOp::Commit {
                committed_message_count,
                ..
            } = op
            {
                max_commit = max_commit.max(*committed_message_count);
            }
        }
        if max_commit > meta.committed_message_count {
            meta.committed_message_count = max_commit;
            self.write_session_meta(&meta)?;
        }
        self.isolate_bytes_beyond_committed(meta.committed_message_count)?;
        Ok(meta)
    }

    fn recover_prepare(&self, meta: &mut SessionMeta, op: &RuntimeOp) -> Result<(), MemoryError> {
        let RuntimeOp::Prepare {
            operation_id,
            run_id,
            ordinals,
            expected_offset,
            count,
            byte_len,
            content_hashes,
        } = op
        else {
            return Ok(());
        };
        let path = self.messages_path();
        let file_len = file_len_or_zero(&path)?;
        if file_len < *expected_offset {
            self.write_runtime_op(&RuntimeOp::Abort {
                operation_id: operation_id.clone(),
                reason: "messages_shorter_than_prepare_offset".into(),
            })?;
            return Ok(());
        }
        let tail_len = file_len - *expected_offset;
        if tail_len == 0 {
            self.write_runtime_op(&RuntimeOp::Abort {
                operation_id: operation_id.clone(),
                reason: "prepare_not_started".into(),
            })?;
            return Ok(());
        }
        let complete = tail_len == *byte_len
            && hashes_match(&path, *expected_offset, *byte_len, content_hashes)?;
        if complete {
            let new_count = meta.committed_message_count + *count;
            self.write_runtime_op(&RuntimeOp::Commit {
                operation_id: operation_id.clone(),
                run_id: run_id.clone(),
                committed_message_count: new_count,
                ordinals: ordinals.clone(),
                count: *count,
            })?;
            meta.committed_message_count = new_count;
            self.write_session_meta(meta)?;
            tracing::info!(
                operation_id,
                committed_message_count = new_count,
                "recovered complete uncommitted message append"
            );
            return Ok(());
        }
        isolate_tail(&path, *expected_offset, operation_id)?;
        self.write_runtime_op(&RuntimeOp::Abort {
            operation_id: operation_id.clone(),
            reason: "truncated_or_hash_mismatch".into(),
        })?;
        tracing::warn!(operation_id, "isolated truncated uncommitted messages tail");
        Ok(())
    }

    fn isolate_bytes_beyond_committed(&self, committed: u64) -> Result<(), MemoryError> {
        let path = self.messages_path();
        if !path.exists() {
            if committed == 0 {
                return Ok(());
            }
            return Err(other(format!(
                "committed_message_count is {committed} but {MESSAGES_FILE} is missing"
            )));
        }
        let end = committed_byte_end(&path, committed)?;
        let file_len = file_len_or_zero(&path)?;
        if file_len > end {
            isolate_tail(&path, end, "orphan")?;
        }
        Ok(())
    }

    fn read_committed_messages(&self, count: u64) -> Result<Vec<Message>, MemoryError> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let path = self.messages_path();
        if !path.exists() {
            return Err(other(format!(
                "committed_message_count is {count} but {MESSAGES_FILE} is missing"
            )));
        }
        let file = File::open(&path).map_err(backend)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::with_capacity(count as usize);
        for line in reader.lines() {
            if messages.len() as u64 >= count {
                break;
            }
            let line = line.map_err(backend)?;
            let msg: Message = serde_json::from_str(&line).map_err(backend)?;
            messages.push(msg);
        }
        if messages.len() as u64 != count {
            return Err(other(format!(
                "{MESSAGES_FILE} has {} complete messages, expected {count} committed",
                messages.len()
            )));
        }
        Ok(messages)
    }

    fn append_locked(
        &self,
        run_id: Option<&str>,
        start_ordinal: u64,
        messages: &[Message],
    ) -> Result<AppendOutcome, MemoryError> {
        if messages.is_empty() {
            self.recover_locked()?;
            return Ok(AppendOutcome {
                written: 0,
                first_seq: 0,
            });
        }
        let mut meta = self.recover_locked()?;
        let ops = self.read_runtime_ops()?;
        let committed_ords = match run_id {
            Some(id) => committed_ordinals_for(&ops, id),
            None => HashSet::new(),
        };
        let mut remaining = Vec::new();
        let mut ordinals = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            let ordinal = start_ordinal + i as u64;
            if run_id.is_some() && committed_ords.contains(&ordinal) {
                continue;
            }
            remaining.push(msg);
            ordinals.push(ordinal);
        }
        if remaining.is_empty() {
            return Ok(AppendOutcome {
                written: 0,
                first_seq: 0,
            });
        }
        let mut hashes = Vec::with_capacity(remaining.len());
        let mut payload = Vec::new();
        for msg in &remaining {
            let json = serde_json::to_string(msg).map_err(backend)?;
            hashes.push(sha256_hex(json.as_bytes()));
            payload.extend_from_slice(json.as_bytes());
            payload.push(b'\n');
        }
        self.ensure_dir()?;
        let messages_path = self.messages_path();
        let expected_offset = file_len_or_zero(&messages_path)?;
        let operation_id = Uuid::new_v4().to_string();
        let count = remaining.len() as u64;
        self.write_runtime_op(&RuntimeOp::Prepare {
            operation_id: operation_id.clone(),
            run_id: run_id.map(str::to_string),
            ordinals: ordinals.clone(),
            expected_offset,
            count,
            byte_len: payload.len() as u64,
            content_hashes: hashes,
        })?;
        append_synced(&messages_path, &payload)?;
        let new_count = meta.committed_message_count + count;
        self.write_runtime_op(&RuntimeOp::Commit {
            operation_id,
            run_id: run_id.map(str::to_string),
            committed_message_count: new_count,
            ordinals,
            count,
        })?;
        meta.committed_message_count = new_count;
        self.write_session_meta(&meta)?;
        let first_seq = new_count - count + 1;
        Ok(AppendOutcome {
            written: count,
            first_seq,
        })
    }

    fn append_next_locked(
        &self,
        run_id: &str,
        messages: &[Message],
    ) -> Result<AppendOutcome, MemoryError> {
        let meta_ops_start = {
            let _ = self.recover_locked()?;
            self.read_runtime_ops()?
        };
        let start = committed_ordinals_for(&meta_ops_start, run_id)
            .into_iter()
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        self.append_locked(Some(run_id), start, messages)
    }

    fn clear_locked(&self) -> Result<(), MemoryError> {
        self.ensure_dir()?;
        let messages = self.messages_path();
        if messages.exists() {
            fs::remove_file(&messages).map_err(backend)?;
        }
        let runtime = self.runtime_path();
        if runtime.exists() {
            fs::remove_file(&runtime).map_err(backend)?;
        }
        if let Ok(entries) = fs::read_dir(&self.session_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("messages.uncommitted.") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let mut meta = self
            .read_or_default_meta()
            .unwrap_or_else(|_| SessionMeta::new(self.session_id.clone(), self.cwd.clone()));
        meta.committed_message_count = 0;
        meta.active_compaction_id = None;
        self.write_session_meta(&meta)?;
        *self
            .history_before_seq
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}

impl RunRecorder {
    pub fn new(memory: CodegMessageMemory) -> Self {
        Self {
            memory,
            active: Arc::new(Mutex::new(None)),
        }
    }

    pub fn memory(&self) -> &CodegMessageMemory {
        &self.memory
    }

    /// Save the current user prompt once. History for the runner is messages
    /// before this prompt (`prompt_seq` is the prompt's 1-based seq).
    pub async fn begin_run(&self, prompt: &Message) -> Result<RunHandle, MemoryError> {
        let run_id = Uuid::new_v4().to_string();
        let prompt_seq = {
            let _guard = self.memory.write_lock.lock().await;
            let outcome =
                self.memory
                    .append_locked(Some(&run_id), 0, std::slice::from_ref(prompt))?;
            if outcome.written != 1 {
                return Err(other(
                    "begin_run did not persist the user prompt as ordinal 0",
                ));
            }
            let prompt_seq = outcome.first_seq;
            self.memory.bind_history_before_locked(prompt_seq);
            self.memory.write_runtime_op(&RuntimeOp::BeginRun {
                run_id: run_id.clone(),
                prompt_seq,
            })?;
            prompt_seq
        };
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(ActiveRun {
            run_id: run_id.clone(),
        });
        Ok(RunHandle {
            memory: self.memory.clone(),
            run_id,
            prompt_seq,
        })
    }

    /// Append run messages after the saved prompt. Idempotent on
    /// `(run_id, message_ordinal)` via the active run's already-committed
    /// ordinals (prompt is ordinal 0; these start at 1).
    pub async fn append_messages(&self, messages: Vec<Message>) -> Result<(), MemoryError> {
        let run_id = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|r| r.run_id.clone())
            .ok_or_else(|| other("append_messages called before begin_run"))?;
        let _guard = self.memory.write_lock.lock().await;
        self.memory.append_next_locked(&run_id, &messages)?;
        Ok(())
    }
}

impl RunHandle {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn prompt_seq(&self) -> u64 {
        self.prompt_seq
    }

    pub fn memory(&self) -> &CodegMessageMemory {
        &self.memory
    }

    /// Append messages for this run. Ordinals continue after those already
    /// committed for `run_id`. Idempotent if the same ordinals are retried
    /// through [`CodegMessageMemory::append_run_messages`].
    pub async fn append_messages(&self, messages: Vec<Message>) -> Result<(), MemoryError> {
        let _guard = self.memory.write_lock.lock().await;
        self.memory.append_next_locked(&self.run_id, &messages)?;
        Ok(())
    }

    pub async fn load_history_before_prompt(&self) -> Result<Vec<Message>, MemoryError> {
        self.memory.load_before_seq(self.prompt_seq).await
    }
}

impl ConversationMemory for CodegMessageMemory {
    fn load<'a>(
        &'a self,
        _conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<Vec<Message>, MemoryError>> {
        Box::pin(async move {
            let _guard = self.write_lock.lock().await;
            self.load_for_runner_locked()
        })
    }

    fn append<'a>(
        &'a self,
        _conversation_id: &'a str,
        messages: Vec<Message>,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            let _guard = self.write_lock.lock().await;
            self.append_locked(None, 0, &messages)?;
            Ok(())
        })
    }

    fn clear<'a>(
        &'a self,
        _conversation_id: &'a str,
    ) -> WasmBoxedFuture<'a, Result<(), MemoryError>> {
        Box::pin(async move {
            let _guard = self.write_lock.lock().await;
            self.clear_locked()
        })
    }
}

fn parse_runtime_line(line: &str) -> Result<Option<RuntimeOp>, MemoryError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(backend)?;
    let op = value.get("op").and_then(|v| v.as_str()).unwrap_or_default();
    match op {
        "prepare" | "commit" | "abort" | "begin_run" => {
            serde_json::from_value(value).map(Some).map_err(backend)
        }
        _ => Ok(None),
    }
}

fn finished_operation_ids(ops: &[RuntimeOp]) -> HashSet<String> {
    ops.iter()
        .filter_map(|op| match op {
            RuntimeOp::Commit { operation_id, .. } | RuntimeOp::Abort { operation_id, .. } => {
                Some(operation_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn committed_ordinals_for(ops: &[RuntimeOp], run_id: &str) -> HashSet<u64> {
    let mut out = HashSet::new();
    for op in ops {
        if let RuntimeOp::Commit {
            run_id: Some(id),
            ordinals,
            ..
        } = op
        {
            if id == run_id {
                out.extend(ordinals.iter().copied());
            }
        }
    }
    out
}

fn file_len_or_zero(path: &Path) -> Result<u64, MemoryError> {
    match fs::metadata(path) {
        Ok(meta) => Ok(meta.len()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(backend(err)),
    }
}

fn append_synced(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(backend)?;
    file.write_all(bytes).map_err(backend)?;
    file.flush().map_err(backend)?;
    file.sync_all().map_err(backend)?;
    Ok(())
}

fn sync_dir(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

fn committed_byte_end(path: &Path, count: u64) -> Result<u64, MemoryError> {
    if count == 0 {
        return Ok(0);
    }
    let data = fs::read(path).map_err(backend)?;
    let mut seen = 0u64;
    for (i, b) in data.iter().enumerate() {
        if *b == b'\n' {
            seen += 1;
            if seen == count {
                return Ok((i + 1) as u64);
            }
        }
    }
    Err(other(format!(
        "{MESSAGES_FILE} has {seen} complete lines, expected {count} committed"
    )))
}

fn hashes_match(
    path: &Path,
    offset: u64,
    byte_len: u64,
    hashes: &[String],
) -> Result<bool, MemoryError> {
    let mut file = File::open(path).map_err(backend)?;
    file.seek(SeekFrom::Start(offset)).map_err(backend)?;
    let mut buf = vec![0u8; byte_len as usize];
    file.read_exact(&mut buf).map_err(backend)?;
    if buf.last().copied() != Some(b'\n') {
        return Ok(false);
    }
    let mut start = 0usize;
    let mut index = 0usize;
    for (i, b) in buf.iter().enumerate() {
        if *b != b'\n' {
            continue;
        }
        if index >= hashes.len() {
            return Ok(false);
        }
        if sha256_hex(&buf[start..i]) != hashes[index] {
            return Ok(false);
        }
        index += 1;
        start = i + 1;
    }
    Ok(index == hashes.len() && start == buf.len())
}

fn isolate_tail(path: &Path, offset: u64, operation_id: &str) -> Result<(), MemoryError> {
    let file_len = file_len_or_zero(path)?;
    if file_len <= offset {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(backend)?;
    file.seek(SeekFrom::Start(offset)).map_err(backend)?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).map_err(backend)?;
    if !tail.is_empty() {
        let dest = path.with_file_name(format!("messages.uncommitted.{operation_id}.jsonl"));
        fs::write(&dest, &tail).map_err(backend)?;
        if let Ok(orphan) = File::open(&dest) {
            let _ = orphan.sync_all();
        }
    }
    file.set_len(offset).map_err(backend)?;
    file.flush().map_err(backend)?;
    file.sync_all().map_err(backend)?;
    Ok(())
}

#[cfg(test)]
impl CodegMessageMemory {
    async fn inject_uncommitted(
        &self,
        run_id: Option<&str>,
        start_ordinal: u64,
        messages: &[Message],
        truncate_last: bool,
    ) -> Result<(), MemoryError> {
        let _guard = self.write_lock.lock().await;
        self.recover_locked()?;
        let mut payload = Vec::new();
        let mut hashes = Vec::new();
        let mut ordinals = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            let json = serde_json::to_string(msg).map_err(backend)?;
            hashes.push(sha256_hex(json.as_bytes()));
            payload.extend_from_slice(json.as_bytes());
            payload.push(b'\n');
            ordinals.push(start_ordinal + i as u64);
        }
        self.ensure_dir()?;
        let messages_path = self.messages_path();
        let expected_offset = file_len_or_zero(&messages_path)?;
        let byte_len = payload.len() as u64;
        let operation_id = Uuid::new_v4().to_string();
        self.write_runtime_op(&RuntimeOp::Prepare {
            operation_id: operation_id.clone(),
            run_id: run_id.map(str::to_string),
            ordinals,
            expected_offset,
            count: messages.len() as u64,
            byte_len,
            content_hashes: hashes,
        })?;
        let to_write = if truncate_last && payload.len() > 4 {
            &payload[..payload.len() - 4]
        } else {
            payload.as_slice()
        };
        append_synced(&messages_path, to_write)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::{
        AssistantContent, ProviderCallId, Reasoning, ToolCall, ToolCallId, ToolFunction,
        ToolResultContent, UserContent,
    };
    use rig::memory::ConversationMemory;
    use serde_json::Value;

    fn temp_memory() -> (tempfile::TempDir, CodegMessageMemory) {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory = CodegMessageMemory::with_root(dir.path(), "sess-1", "/tmp/demo");
        (dir, memory)
    }

    fn fixture_chain() -> Vec<Message> {
        let provider = ProviderCallId::new("call_status_01")
            .expect("call_id")
            .with_item_id("fc_status_01");
        let call_id = ToolCallId::new("call_status_01").expect("tool id");
        let tool_call = ToolCall::new(
            call_id.clone(),
            ToolFunction::new(
                "bash".into(),
                serde_json::json!({"command": "git status --short"}),
            ),
        )
        .with_provider(provider.clone())
        .with_signature(Some("sig-tool".into()))
        .with_additional_params(Some(serde_json::json!({"region": "us"})));
        let reasoning = Reasoning::new_with_signature("need git status", Some("sig-reason".into()))
            .with_id("reason_01".into());
        vec![
            Message::user("提交当前改动"),
            Message::Assistant {
                id: Some("asst_01".into()),
                content: vec![
                    AssistantContent::Reasoning(reasoning),
                    AssistantContent::ToolCall(tool_call),
                ],
            },
            Message::User {
                content: vec![UserContent::tool_result_for(
                    call_id,
                    Some(provider),
                    "bash",
                    vec![ToolResultContent::text("exit_code: 0\n M README.md\n")],
                )],
            },
            Message::assistant("已确认改动范围。"),
        ]
    }

    fn assert_no_field_loss(original: &Value, round: &Value) {
        match (original, round) {
            (Value::Object(a), Value::Object(b)) => {
                for (key, value) in a {
                    assert!(
                        b.contains_key(key),
                        "dropped field {key} from {original} -> {round}"
                    );
                    assert_no_field_loss(value, &b[key]);
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(
                    a.len(),
                    b.len(),
                    "array length changed {original} -> {round}"
                );
                for (left, right) in a.iter().zip(b.iter()) {
                    assert_no_field_loss(left, right);
                }
            }
            (left, right) => assert_eq!(left, right, "value changed"),
        }
    }

    #[test]
    fn serde_round_trip_preserves_tool_ids_provider_and_function() {
        let chain = fixture_chain();
        let original = serde_json::to_value(&chain).expect("serialize fixture");
        let parsed: Vec<Message> = serde_json::from_value(original.clone()).expect("deserialize");
        let again = serde_json::to_value(&parsed).expect("re-serialize");
        assert_no_field_loss(&original, &again);

        let dumped = serde_json::to_string(&chain).expect("dump");
        assert!(dumped.contains("\"type\":\"toolcall\""), "{dumped}");
        assert!(dumped.contains("\"type\":\"toolresult\""), "{dumped}");
        assert!(!dumped.contains("\"role\":\"tool\""), "{dumped}");

        let assistant = &original[1];
        assert_eq!(assistant["content"][1]["type"], "toolcall");
        assert_eq!(assistant["content"][1]["id"], "call_status_01");
        assert_eq!(
            assistant["content"][1]["provider"]["call_id"],
            "call_status_01"
        );
        assert_eq!(
            assistant["content"][1]["provider"]["item_id"],
            "fc_status_01"
        );
        assert_eq!(assistant["content"][1]["function"]["name"], "bash");
        assert_eq!(assistant["content"][1]["additional_params"]["region"], "us");
        assert_eq!(assistant["content"][0]["type"], "reasoning");
        assert_eq!(assistant["id"], "asst_01");

        let tool_result = &original[2];
        assert_eq!(tool_result["content"][0]["type"], "toolresult");
        assert_eq!(tool_result["content"][0]["call"], "call_status_01");
        assert_eq!(
            tool_result["content"][0]["provider"]["call_id"],
            "call_status_01"
        );
        assert_eq!(
            tool_result["content"][0]["provider"]["item_id"],
            "fc_status_01"
        );
        assert_eq!(tool_result["content"][0]["name"], "bash");
    }

    #[tokio::test]
    async fn serde_chain_survives_jsonl_persist() {
        let (_dir, memory) = temp_memory();
        let chain = fixture_chain();
        let original = serde_json::to_value(&chain).expect("serialize");
        ConversationMemory::append(&memory, memory.session_id(), chain.clone())
            .await
            .expect("append");
        let loaded = memory.load_committed().await.expect("load");
        let again = serde_json::to_value(&loaded).expect("re-serialize loaded");
        assert_no_field_loss(&original, &again);
        assert_eq!(loaded, chain);
    }

    #[tokio::test]
    async fn append_is_idempotent_for_same_run_id_and_ordinal() {
        let (_dir, memory) = temp_memory();
        let msg = Message::assistant("same ordinal");
        memory
            .append_run_messages("run-abc", 0, vec![msg.clone()])
            .await
            .expect("first append");
        memory
            .append_run_messages("run-abc", 0, vec![msg.clone()])
            .await
            .expect("retry same ordinal");
        let loaded = memory.load_committed().await.expect("load");
        assert_eq!(loaded, vec![msg.clone()]);

        memory
            .append_run_messages("run-new", 0, vec![Message::user("same ordinal")])
            .await
            .expect("new run");
        memory
            .append_run_messages("run-new", 0, vec![Message::user("same ordinal")])
            .await
            .expect("new run retry");
        let loaded = memory.load_committed().await.expect("load");
        assert_eq!(
            loaded.len(),
            2,
            "new run_id is a new message even with identical text"
        );
    }

    #[tokio::test]
    async fn load_isolates_truncated_tail_after_prepare_without_commit() {
        let (_dir, memory) = temp_memory();
        let keep = Message::user("keep-me");
        ConversationMemory::append(&memory, memory.session_id(), vec![keep.clone()])
            .await
            .expect("seed");
        memory
            .inject_uncommitted(
                Some("run-crash"),
                1,
                &[Message::assistant("this json will be truncated")],
                true,
            )
            .await
            .expect("inject truncated");

        let loaded = memory.load_committed().await.expect("load after crash");
        assert_eq!(loaded, vec![keep.clone()]);
        let runner = ConversationMemory::load(&memory, memory.session_id())
            .await
            .expect("runner load");
        assert_eq!(runner, vec![keep]);

        let raw = fs::read_to_string(memory.session_dir().join(MESSAGES_FILE)).expect("read jsonl");
        assert!(
            !raw.contains("this json will be truncated"),
            "truncated JSON must not remain in messages.jsonl: {raw}"
        );
        assert_eq!(
            memory.meta().await.expect("meta").committed_message_count,
            1
        );
    }

    #[tokio::test]
    async fn load_recovers_complete_uncommitted_lines_and_commits() {
        let (_dir, memory) = temp_memory();
        let keep = Message::user("keep-me");
        let extra = Message::assistant("fully written but uncommitted");
        ConversationMemory::append(&memory, memory.session_id(), vec![keep.clone()])
            .await
            .expect("seed");
        memory
            .inject_uncommitted(Some("run-crash"), 1, std::slice::from_ref(&extra), false)
            .await
            .expect("inject complete");

        let loaded = memory.load_committed().await.expect("load recovers");
        assert_eq!(loaded, vec![keep, extra]);
        assert_eq!(
            memory.meta().await.expect("meta").committed_message_count,
            2
        );

        let loaded_again = memory.load_committed().await.expect("second load");
        assert_eq!(loaded_again.len(), 2);
    }

    #[tokio::test]
    async fn load_excludes_prompt_saved_after_history_boundary() {
        let (_dir, memory) = temp_memory();
        let prior = Message::user("already committed");
        ConversationMemory::append(&memory, memory.session_id(), vec![prior.clone()])
            .await
            .expect("prior");
        let recorder = RunRecorder::new(memory.clone());
        let prompt = Message::user("current instruction");
        let handle = recorder.begin_run(&prompt).await.expect("begin_run");

        let inner = ConversationMemory::load(&memory, memory.session_id())
            .await
            .expect("inner.load");
        let before = handle
            .load_history_before_prompt()
            .await
            .expect("history before prompt");
        let full = memory.load_committed().await.expect("full committed");

        assert_eq!(inner, vec![prior.clone()]);
        assert_eq!(before, inner);
        assert_eq!(full, vec![prior, prompt.clone()]);
        assert_eq!(handle.prompt_seq(), 2);
        assert!(
            inner.iter().all(|m| m != &prompt),
            "inner.load must not include the saved current prompt"
        );
    }

    #[tokio::test]
    async fn concurrent_appends_on_same_session_are_serialized() {
        let (_dir, memory) = temp_memory();
        let a = memory.clone();
        let b = memory.clone();
        let (r1, r2) = tokio::join!(
            async {
                ConversationMemory::append(&a, a.session_id(), vec![Message::user("one")]).await
            },
            async {
                ConversationMemory::append(&b, b.session_id(), vec![Message::user("two")]).await
            },
        );
        r1.expect("first append");
        r2.expect("second append");
        let loaded = memory.load_committed().await.expect("load");
        assert_eq!(loaded.len(), 2);
        let texts: Vec<String> = loaded.iter().filter_map(Message::rag_text).collect();
        assert!(texts.contains(&"one".to_string()), "{texts:?}");
        assert!(texts.contains(&"two".to_string()), "{texts:?}");
        for line in fs::read_to_string(memory.session_dir().join(MESSAGES_FILE))
            .expect("jsonl")
            .lines()
        {
            let parsed: Message = serde_json::from_str(line).expect("complete json line");
            assert!(matches!(parsed, Message::User { .. }));
        }
        assert_eq!(
            memory.meta().await.expect("meta").committed_message_count,
            2
        );
    }

    #[tokio::test]
    async fn clear_wipes_messages_and_cursor_not_used_as_compaction() {
        let (_dir, memory) = temp_memory();
        ConversationMemory::append(&memory, memory.session_id(), fixture_chain())
            .await
            .expect("append");
        ConversationMemory::clear(&memory, memory.session_id())
            .await
            .expect("clear");
        let loaded = memory.load_committed().await.expect("load");
        assert!(loaded.is_empty());
        assert_eq!(
            memory.meta().await.expect("meta").committed_message_count,
            0
        );
    }

    #[tokio::test]
    async fn with_root_uses_encoded_cwd_and_session_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = "/Users/me/proj";
        let memory = CodegMessageMemory::with_root(dir.path(), "sess-x", cwd);
        assert_eq!(
            memory.session_dir(),
            dir.path().join(encode_session_cwd(cwd)).join("sess-x")
        );
    }

    #[tokio::test]
    async fn set_active_compaction_id_does_not_touch_messages() {
        let (_dir, memory) = temp_memory();
        let keep = Message::user("keep-me");
        ConversationMemory::append(&memory, memory.session_id(), vec![keep.clone()])
            .await
            .expect("seed");
        memory
            .set_active_compaction_id(Some("compact-1".into()))
            .await
            .expect("set pointer");
        assert_eq!(
            memory.active_compaction_id().await.expect("read"),
            Some("compact-1".into())
        );
        let loaded = memory.load_committed().await.expect("messages");
        assert_eq!(loaded, vec![keep]);
        memory
            .set_active_compaction_id(None)
            .await
            .expect("clear pointer");
        assert_eq!(memory.active_compaction_id().await.expect("cleared"), None);
    }
}

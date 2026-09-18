//! Compact-llm pending/committed checkpoints and restart recovery.
//!
//! Layout under `memory.session_dir()`:
//! ```text
//! compactions/
//!   <id>.json          pending or committed checkpoint
//!   <id>.pending/      writable attachment root while pending
//!   <id>/              committed attachment root
//! ```
//!
//! `session.json.active_compaction_id` is the live pointer. This store does
//! not write `messages.jsonl` / `runtime.jsonl`, does not treat
//! `compactions/index.json` as authority, and does not pick a checkpoint by
//! directory order or mtime.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rig::completion::message::UserContent;
use rig::completion::Message;

use super::{
    sha256_hex, CompactLlmCheckpoint, CompactLlmCheckpointStatus, CompactLlmError,
    CompactLlmFileEntry, COMPACT_LLM_MAX_FILE_BYTES, COMPACT_LLM_MAX_FILE_COUNT,
    COMPACT_LLM_MAX_TOTAL_BYTES,
};
use crate::agent::context::message_memory::CodegMessageMemory;

const COMPACTIONS_DIR: &str = "compactions";
const PENDING_SUFFIX: &str = ".pending";

/// Two-phase compact-llm checkpoint persistence bound to one session directory.
#[derive(Clone)]
pub struct CompactLlmCheckpointStore {
    memory: CodegMessageMemory,
}

impl CompactLlmCheckpointStore {
    pub fn new(memory: CodegMessageMemory) -> Self {
        Self { memory }
    }

    pub fn session_dir(&self) -> &Path {
        self.memory.session_dir()
    }

    pub fn compactions_dir(&self) -> PathBuf {
        self.session_dir().join(COMPACTIONS_DIR)
    }

    /// `compactions/<id>.pending`
    pub fn pending_files_dir(&self, id: &str) -> PathBuf {
        self.compactions_dir().join(format!("{id}{PENDING_SUFFIX}"))
    }

    /// `compactions/<id>`
    pub fn committed_files_dir(&self, id: &str) -> PathBuf {
        self.compactions_dir().join(id)
    }

    /// Discard pending json + `*.pending` dirs and orphan attachment dirs.
    /// If a committed record exists but `session.json.active_compaction_id` is
    /// missing/stale, verify lineage then repair the pointer. Never pick "latest
    /// mtime".
    pub async fn recover(&self) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
        let dir = self.compactions_dir();
        if dir.exists() {
            self.complete_committed_renames(&dir)?;
            self.discard_pending_records(&dir)?;
            self.discard_orphan_dirs(&dir)?;
        }

        let committed = self.load_committed_map()?;
        let pointer = self.active_pointer().await?;
        let desired = self.choose_active(&pointer, &committed)?;

        let desired_id = desired.as_ref().map(|cp| cp.id.clone());
        if pointer != desired_id {
            self.memory
                .set_active_compaction_id(desired_id)
                .await
                .map_err(mem_err)?;
        }
        Ok(desired)
    }

    /// Create `<id>.pending/` (flush+sync) and atomically write `<id>.json`
    /// with status=pending. Does not update the active pointer.
    pub async fn write_pending(
        &self,
        checkpoint: &CompactLlmCheckpoint,
    ) -> Result<PathBuf, CompactLlmError> {
        validate_id(&checkpoint.id)?;
        let pending_dir = self.pending_files_dir(&checkpoint.id);
        let committed_dir = self.committed_files_dir(&checkpoint.id);
        let json_path = self.checkpoint_json_path(&checkpoint.id);

        if let Some(existing) = self.read_checkpoint(&checkpoint.id)? {
            if existing.status == CompactLlmCheckpointStatus::Committed {
                return Err(CompactLlmError::checkpoint(format!(
                    "compaction {} is already committed",
                    checkpoint.id
                )));
            }
        }
        if committed_dir.exists() {
            return Err(CompactLlmError::checkpoint(format!(
                "committed attachment dir already exists for {}",
                checkpoint.id
            )));
        }

        if !pending_dir.exists() {
            fs::create_dir_all(&pending_dir).map_err(|e| {
                CompactLlmError::checkpoint(format!(
                    "create pending dir {}: {e}",
                    pending_dir.display()
                ))
            })?;
            sync_dir(&pending_dir);
            sync_dir(&self.compactions_dir());
        }

        let mut pending = checkpoint.clone();
        pending.status = CompactLlmCheckpointStatus::Pending;
        atomic_write_json(&json_path, &pending)?;
        Ok(pending_dir)
    }

    /// After the caller has finished writing attachments into the pending dir:
    /// verify manifest, atomically mark committed, rename pending → committed,
    /// then set the active pointer. On failure the previous pointer is left
    /// untouched.
    pub async fn commit(&self, id: &str) -> Result<CompactLlmCheckpoint, CompactLlmError> {
        validate_id(id)?;
        let mut checkpoint = self
            .read_checkpoint(id)?
            .ok_or_else(|| CompactLlmError::checkpoint(format!("compaction {id} not found")))?;
        if checkpoint.id != id {
            return Err(CompactLlmError::checkpoint(format!(
                "compaction id {} does not match file {id}",
                checkpoint.id
            )));
        }

        let pending_dir = self.pending_files_dir(id);
        let committed_dir = self.committed_files_dir(id);
        let files_root = if pending_dir.exists() {
            pending_dir.clone()
        } else {
            committed_dir.clone()
        };
        verify_files(&files_root, &checkpoint.files)?;

        checkpoint.status = CompactLlmCheckpointStatus::Committed;
        atomic_write_json(&self.checkpoint_json_path(id), &checkpoint)?;

        if pending_dir.exists() {
            if committed_dir.exists() {
                remove_existing(&committed_dir)?;
            }
            fs::rename(&pending_dir, &committed_dir).map_err(|e| {
                CompactLlmError::checkpoint(format!(
                    "rename pending dir {} -> {}: {e}",
                    pending_dir.display(),
                    committed_dir.display()
                ))
            })?;
            sync_dir(&self.compactions_dir());
        }

        self.memory
            .set_active_compaction_id(Some(id.to_string()))
            .await
            .map_err(mem_err)?;
        Ok(checkpoint)
    }

    /// Delete pending json + pending dir. Must not activate the checkpoint.
    pub async fn abort_pending(&self, id: &str) -> Result<(), CompactLlmError> {
        validate_id(id)?;
        if let Some(existing) = self.read_checkpoint(id)? {
            if existing.status == CompactLlmCheckpointStatus::Committed {
                return Err(CompactLlmError::checkpoint(format!(
                    "cannot abort committed compaction {id}"
                )));
            }
        }
        remove_existing(&self.checkpoint_json_path(id))?;
        remove_existing(&self.pending_files_dir(id))?;
        Ok(())
    }

    /// Reuse key: conversation_id + epoch + covers_through_seq +
    /// source_slice_sha256 + fingerprint. Returns matching pending or committed.
    pub async fn find_reusable(
        &self,
        conversation_id: &str,
        epoch: u64,
        covers_through_seq: u64,
        source_slice_sha256: &str,
        fingerprint: &str,
    ) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
        let mut matches: Vec<CompactLlmCheckpoint> = self
            .load_all_checkpoints()?
            .into_iter()
            .filter(|cp| {
                cp.conversation_id == conversation_id
                    && cp.epoch == epoch
                    && cp.covers_through_seq == covers_through_seq
                    && cp.source_slice_sha256 == source_slice_sha256
                    && cp.fingerprint == fingerprint
            })
            .collect();
        matches.sort_by(|a, b| {
            reuse_rank(a)
                .cmp(&reuse_rank(b))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(matches.into_iter().next())
    }

    pub async fn load_active(&self) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
        let Some(id) = self.active_pointer().await? else {
            return Ok(None);
        };
        match self.read_checkpoint(&id)? {
            Some(cp) if cp.status == CompactLlmCheckpointStatus::Committed => Ok(Some(cp)),
            _ => Ok(None),
        }
    }

    /// Committed and pending records on disk. Does not consult the live pointer.
    pub fn load_records(&self) -> Result<Vec<CompactLlmCheckpoint>, CompactLlmError> {
        self.load_all_checkpoints()
    }

    /// Read committed files listed on `files[]`, verify path/size/sha256/utf-8,
    /// reject absolute/`..`/NUL/symlink. First text part is `summary_message`;
    /// each file is a following text part (`## file: <rel>\n<body>`).
    pub fn expand_summary(
        &self,
        checkpoint: &CompactLlmCheckpoint,
    ) -> Result<Message, CompactLlmError> {
        if checkpoint.status != CompactLlmCheckpointStatus::Committed {
            return Err(CompactLlmError::checkpoint(format!(
                "compaction {} is not committed",
                checkpoint.id
            )));
        }
        self.expand_summary_at(checkpoint, &self.committed_files_dir(&checkpoint.id))
    }

    /// Expand attachments from `files_root` (`<id>.pending/` or `<id>/`).
    pub fn expand_summary_at(
        &self,
        checkpoint: &CompactLlmCheckpoint,
        files_root: &Path,
    ) -> Result<Message, CompactLlmError> {
        let Message::User { content: parts } = &checkpoint.summary_message else {
            return Err(CompactLlmError::checkpoint(
                "summary_message must be a user message",
            ));
        };
        if parts.is_empty() {
            return Err(CompactLlmError::checkpoint(
                "summary_message has no content",
            ));
        }

        let bodies = verify_files(files_root, &checkpoint.files)?;
        let mut content = parts.clone();
        for (entry, body) in checkpoint.files.iter().zip(bodies) {
            content.push(UserContent::text(format!(
                "## file: {}\n{body}",
                entry.path
            )));
        }
        Ok(Message::User { content })
    }

    fn checkpoint_json_path(&self, id: &str) -> PathBuf {
        self.compactions_dir().join(format!("{id}.json"))
    }

    async fn active_pointer(&self) -> Result<Option<String>, CompactLlmError> {
        self.memory.active_compaction_id().await.map_err(mem_err)
    }

    fn read_checkpoint(&self, id: &str) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
        validate_id(id)?;
        read_checkpoint_file(&self.checkpoint_json_path(id), Some(id))
    }

    fn load_all_checkpoints(&self) -> Result<Vec<CompactLlmCheckpoint>, CompactLlmError> {
        let dir = self.compactions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for path in checkpoint_json_paths(&dir)? {
            if let Some(cp) = read_checkpoint_file(&path, None)? {
                out.push(cp);
            }
        }
        Ok(out)
    }

    fn load_committed_map(&self) -> Result<HashMap<String, CompactLlmCheckpoint>, CompactLlmError> {
        let mut map = HashMap::new();
        for cp in self.load_all_checkpoints()? {
            if cp.status == CompactLlmCheckpointStatus::Committed {
                map.insert(cp.id.clone(), cp);
            }
        }
        Ok(map)
    }

    fn complete_committed_renames(&self, dir: &Path) -> Result<(), CompactLlmError> {
        for path in checkpoint_json_paths(dir)? {
            let Some(cp) = read_checkpoint_file(&path, None)? else {
                continue;
            };
            if cp.status != CompactLlmCheckpointStatus::Committed {
                continue;
            }
            let pending = self.pending_files_dir(&cp.id);
            let committed = self.committed_files_dir(&cp.id);
            if pending.exists() {
                if committed.exists() {
                    remove_existing(&pending)?;
                } else {
                    fs::rename(&pending, &committed).map_err(|e| {
                        CompactLlmError::checkpoint(format!(
                            "recover rename {} -> {}: {e}",
                            pending.display(),
                            committed.display()
                        ))
                    })?;
                    sync_dir(dir);
                }
            }
        }
        Ok(())
    }

    fn discard_pending_records(&self, dir: &Path) -> Result<(), CompactLlmError> {
        for path in checkpoint_json_paths(dir)? {
            let Some(cp) = read_checkpoint_file(&path, None)? else {
                continue;
            };
            if cp.status != CompactLlmCheckpointStatus::Pending {
                continue;
            }
            remove_existing(&path)?;
            remove_existing(&self.pending_files_dir(&cp.id))?;
        }
        Ok(())
    }

    fn discard_orphan_dirs(&self, dir: &Path) -> Result<(), CompactLlmError> {
        let committed = self.load_committed_map()?;
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(CompactLlmError::checkpoint(format!(
                    "read {}: {err}",
                    dir.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry
                .map_err(|e| CompactLlmError::checkpoint(format!("read {}: {e}", dir.display())))?;
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(CompactLlmError::checkpoint(format!(
                        "stat {}: {err}",
                        path.display()
                    )));
                }
            };
            if !meta.is_dir() || meta.file_type().is_symlink() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.ends_with(PENDING_SUFFIX) {
                remove_existing(&path)?;
                continue;
            }
            if !committed.contains_key(name) {
                remove_existing(&path)?;
            }
        }
        Ok(())
    }

    fn choose_active(
        &self,
        pointer: &Option<String>,
        committed: &HashMap<String, CompactLlmCheckpoint>,
    ) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
        if let Some(id) = pointer {
            if let Some(start) = committed.get(id) {
                if lineage_ok(start, committed)
                    && verify_files(&self.committed_files_dir(&start.id), &start.files).is_ok()
                {
                    return Ok(Some(self.walk_forward(start.clone(), committed)));
                }
            }
        }

        let mut by_generation: HashMap<GenerationKey, Vec<CompactLlmCheckpoint>> = HashMap::new();
        for cp in committed.values() {
            if !lineage_ok(cp, committed) {
                continue;
            }
            if verify_files(&self.committed_files_dir(&cp.id), &cp.files).is_err() {
                continue;
            }
            by_generation
                .entry(GenerationKey::from_checkpoint(cp))
                .or_default()
                .push(cp.clone());
        }
        if by_generation.len() != 1 {
            return Ok(None);
        }
        let group = by_generation.into_values().next().unwrap_or_default();
        Ok(generation_tip(group))
    }

    fn walk_forward(
        &self,
        start: CompactLlmCheckpoint,
        committed: &HashMap<String, CompactLlmCheckpoint>,
    ) -> CompactLlmCheckpoint {
        let mut current = start;
        loop {
            let mut successors: Vec<CompactLlmCheckpoint> = committed
                .values()
                .filter(|cp| {
                    cp.previous_id.as_deref() == Some(current.id.as_str())
                        && same_generation(cp, &current)
                        && cp.covers_through_seq > current.covers_through_seq
                        && lineage_ok(cp, committed)
                        && verify_files(&self.committed_files_dir(&cp.id), &cp.files).is_ok()
                })
                .cloned()
                .collect();
            let Some(next) = successors.pop() else {
                return current;
            };
            if !successors.is_empty() {
                return current;
            }
            current = next;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GenerationKey {
    conversation_id: String,
    epoch: u64,
    fingerprint: String,
}

impl GenerationKey {
    fn from_checkpoint(cp: &CompactLlmCheckpoint) -> Self {
        Self {
            conversation_id: cp.conversation_id.clone(),
            epoch: cp.epoch,
            fingerprint: cp.fingerprint.clone(),
        }
    }
}

fn same_generation(a: &CompactLlmCheckpoint, b: &CompactLlmCheckpoint) -> bool {
    a.conversation_id == b.conversation_id && a.epoch == b.epoch && a.fingerprint == b.fingerprint
}

fn lineage_ok(
    cp: &CompactLlmCheckpoint,
    committed: &HashMap<String, CompactLlmCheckpoint>,
) -> bool {
    match &cp.previous_id {
        None => true,
        Some(prev_id) => {
            let Some(prev) = committed.get(prev_id) else {
                return false;
            };
            prev.status == CompactLlmCheckpointStatus::Committed
                && same_generation(prev, cp)
                && prev.covers_through_seq < cp.covers_through_seq
        }
    }
}

fn generation_tip(group: Vec<CompactLlmCheckpoint>) -> Option<CompactLlmCheckpoint> {
    let referenced: HashSet<String> = group
        .iter()
        .filter_map(|cp| cp.previous_id.clone())
        .collect();
    let mut tips: Vec<CompactLlmCheckpoint> = group
        .into_iter()
        .filter(|cp| !referenced.contains(&cp.id))
        .collect();
    if tips.len() == 1 {
        tips.pop()
    } else {
        None
    }
}

fn reuse_rank(cp: &CompactLlmCheckpoint) -> u8 {
    match cp.status {
        CompactLlmCheckpointStatus::Committed => 0,
        CompactLlmCheckpointStatus::Pending => 1,
    }
}

fn validate_id(id: &str) -> Result<(), CompactLlmError> {
    if id.is_empty() || id.contains('\0') || id.starts_with('.') {
        return Err(CompactLlmError::checkpoint(
            "compaction id must be a single non-hidden path component",
        ));
    }
    if id.ends_with(PENDING_SUFFIX) || id.ends_with(".json") {
        return Err(CompactLlmError::checkpoint(format!(
            "compaction id {id} uses a reserved suffix"
        )));
    }
    let mut comps = Path::new(id).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(name)), None) if name == id => Ok(()),
        _ => Err(CompactLlmError::checkpoint(format!(
            "compaction id {id} must be a single relative path component"
        ))),
    }
}

fn validate_rel_path(rel: &str) -> Result<(), CompactLlmError> {
    if rel.is_empty() {
        return Err(CompactLlmError::checkpoint(
            "attachment path must be non-empty",
        ));
    }
    if rel.contains('\0') {
        return Err(CompactLlmError::checkpoint(format!(
            "attachment path contains NUL: {rel}"
        )));
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(CompactLlmError::checkpoint(format!(
            "attachment path must be relative: {rel}"
        )));
    }
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => saw_normal = true,
            _ => {
                return Err(CompactLlmError::checkpoint(format!(
                    "illegal attachment path {rel}"
                )));
            }
        }
    }
    if !saw_normal {
        return Err(CompactLlmError::checkpoint(format!(
            "illegal attachment path {rel}"
        )));
    }
    Ok(())
}

fn resolve_under(root: &Path, rel: &str) -> Result<PathBuf, CompactLlmError> {
    validate_rel_path(rel)?;
    let mut cur = root.to_path_buf();
    for component in Path::new(rel).components() {
        let Component::Normal(name) = component else {
            return Err(CompactLlmError::checkpoint(format!(
                "illegal attachment path {rel}"
            )));
        };
        cur.push(name);
        if let Ok(meta) = fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                return Err(CompactLlmError::checkpoint(format!(
                    "attachment path is a symlink: {rel}"
                )));
            }
        }
        if !cur.starts_with(root) {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment path escapes compaction dir: {rel}"
            )));
        }
    }
    Ok(cur)
}

fn verify_files(
    root: &Path,
    files: &[CompactLlmFileEntry],
) -> Result<Vec<String>, CompactLlmError> {
    if files.len() > COMPACT_LLM_MAX_FILE_COUNT {
        return Err(CompactLlmError::checkpoint(format!(
            "attachment count {} exceeds max {COMPACT_LLM_MAX_FILE_COUNT}",
            files.len()
        )));
    }
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for entry in files {
        if !seen.insert(entry.path.as_str()) {
            return Err(CompactLlmError::checkpoint(format!(
                "duplicate attachment path {}",
                entry.path
            )));
        }
        if entry.bytes > COMPACT_LLM_MAX_FILE_BYTES {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment {} exceeds max {COMPACT_LLM_MAX_FILE_BYTES} bytes",
                entry.path
            )));
        }
        total = total.saturating_add(entry.bytes);
    }
    if total > COMPACT_LLM_MAX_TOTAL_BYTES {
        return Err(CompactLlmError::checkpoint(format!(
            "attachment total {total} exceeds max {COMPACT_LLM_MAX_TOTAL_BYTES} bytes"
        )));
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    if !root.exists() {
        return Err(CompactLlmError::checkpoint(format!(
            "attachment dir missing: {}",
            root.display()
        )));
    }

    let mut bodies = Vec::with_capacity(files.len());
    for entry in files {
        let path = resolve_under(root, &entry.path)?;
        let meta = fs::symlink_metadata(&path)
            .map_err(|e| CompactLlmError::checkpoint(format!("stat {}: {e}", path.display())))?;
        if meta.file_type().is_symlink() {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment path is a symlink: {}",
                entry.path
            )));
        }
        if !meta.is_file() {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment is not a file: {}",
                entry.path
            )));
        }
        if meta.len() != entry.bytes {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment {} size {} does not match manifest {}",
                entry.path,
                meta.len(),
                entry.bytes
            )));
        }
        let bytes = fs::read(&path)
            .map_err(|e| CompactLlmError::checkpoint(format!("read {}: {e}", path.display())))?;
        if bytes.len() as u64 != entry.bytes {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment {} size {} does not match manifest {}",
                entry.path,
                bytes.len(),
                entry.bytes
            )));
        }
        let digest = sha256_hex(&bytes);
        if digest != entry.sha256 {
            return Err(CompactLlmError::checkpoint(format!(
                "attachment {} sha256 mismatch",
                entry.path
            )));
        }
        let body = String::from_utf8(bytes).map_err(|_| {
            CompactLlmError::checkpoint(format!("attachment {} is not UTF-8", entry.path))
        })?;
        bodies.push(body);
    }
    Ok(bodies)
}

fn checkpoint_json_paths(dir: &Path) -> Result<Vec<PathBuf>, CompactLlmError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(CompactLlmError::checkpoint(format!(
                "read {}: {err}",
                dir.display()
            )));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|e| CompactLlmError::checkpoint(format!("read {}: {e}", dir.display())))?;
        let path = entry.path();
        if is_checkpoint_json(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn is_checkpoint_json(path: &Path) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || name == "index.json" {
        return false;
    }
    true
}

fn read_checkpoint_file(
    path: &Path,
    expected_id: Option<&str>,
) -> Result<Option<CompactLlmCheckpoint>, CompactLlmError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(CompactLlmError::checkpoint(format!(
                "read {}: {err}",
                path.display()
            )));
        }
    };
    let parsed: CompactLlmCheckpoint = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "skip malformed compact-llm checkpoint"
            );
            return Ok(None);
        }
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if parsed.id != stem {
        tracing::warn!(
            path = %path.display(),
            id = %parsed.id,
            "skip compact-llm checkpoint whose id does not match filename"
        );
        return Ok(None);
    }
    if let Some(expected) = expected_id {
        if parsed.id != expected {
            return Ok(None);
        }
    }
    if validate_id(&parsed.id).is_err() {
        tracing::warn!(
            path = %path.display(),
            id = %parsed.id,
            "skip compact-llm checkpoint with invalid id"
        );
        return Ok(None);
    }
    Ok(Some(parsed))
}

fn atomic_write_json(
    path: &Path,
    checkpoint: &CompactLlmCheckpoint,
) -> Result<(), CompactLlmError> {
    let parent = path.parent().ok_or_else(|| {
        CompactLlmError::checkpoint(format!("checkpoint path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)
        .map_err(|e| CompactLlmError::checkpoint(format!("create {}: {e}", parent.display())))?;
    let bytes = serde_json::to_vec_pretty(checkpoint).map_err(|e| {
        CompactLlmError::checkpoint(format!("serialize compaction checkpoint: {e}"))
    })?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("checkpoint.json"),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let write_res = File::create(&tmp)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })
        .and_then(|_| fs::rename(&tmp, path));
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res.map_err(|e| {
        CompactLlmError::checkpoint(format!("atomic write {}: {e}", path.display()))
    })?;
    sync_dir(parent);
    Ok(())
}

fn remove_existing(path: &Path) -> Result<(), CompactLlmError> {
    match fs::symlink_metadata(path) {
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(CompactLlmError::checkpoint(format!(
            "stat {}: {err}",
            path.display()
        ))),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => fs::remove_dir_all(path)
            .map_err(|e| {
                CompactLlmError::checkpoint(format!("remove dir {}: {e}", path.display()))
            }),
        Ok(_) => fs::remove_file(path)
            .map_err(|e| CompactLlmError::checkpoint(format!("remove {}: {e}", path.display()))),
    }
}

fn sync_dir(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

fn mem_err(err: rig::memory::MemoryError) -> CompactLlmError {
    CompactLlmError::checkpoint(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::compact_llm::{fingerprint, HISTORY_SUMMARY_ZH};

    fn temp_store() -> (
        tempfile::TempDir,
        CompactLlmCheckpointStore,
        CodegMessageMemory,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let memory = CodegMessageMemory::with_root(dir.path(), "sess", "/tmp/proj");
        let store = CompactLlmCheckpointStore::new(memory.clone());
        (dir, store, memory)
    }

    fn sample_checkpoint(
        id: &str,
        files: Vec<CompactLlmFileEntry>,
        previous_id: Option<String>,
        covers_through_seq: u64,
    ) -> CompactLlmCheckpoint {
        CompactLlmCheckpoint {
            id: id.to_string(),
            previous_id,
            conversation_id: "sess".into(),
            epoch: 1,
            covers_through_seq,
            source_last_seq: covers_through_seq,
            source_slice_sha256: sha256_hex(id.as_bytes()),
            summary_message: Message::user(format!("{HISTORY_SUMMARY_ZH}kept work")),
            files,
            fingerprint: fingerprint("model", "prompt"),
            status: CompactLlmCheckpointStatus::Pending,
            estimated_summary_tokens: Some(8),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn file_entry(rel: &str, body: &str) -> CompactLlmFileEntry {
        CompactLlmFileEntry {
            path: rel.to_string(),
            bytes: body.len() as u64,
            sha256: sha256_hex(body.as_bytes()),
            media_type: "text/markdown".into(),
        }
    }

    fn write_attachment(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, body).expect("write attachment");
    }

    fn text_parts(message: &Message) -> Vec<String> {
        let Message::User { content } = message else {
            panic!("expected user message");
        };
        content
            .iter()
            .filter_map(|part| match part {
                UserContent::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn assert_no_transcript(store: &CompactLlmCheckpointStore) {
        assert!(
            !store.session_dir().join("messages.jsonl").exists(),
            "store must not write messages.jsonl"
        );
        assert!(
            !store.session_dir().join("runtime.jsonl").exists(),
            "store must not write runtime.jsonl"
        );
    }

    #[tokio::test]
    async fn write_pending_does_not_set_active_compaction_id() {
        let (_dir, store, memory) = temp_store();
        let cp = sample_checkpoint("c1", Vec::new(), None, 2);
        let pending = store.write_pending(&cp).await.expect("write_pending");
        assert_eq!(pending, store.pending_files_dir("c1"));
        assert!(pending.is_dir());
        assert_eq!(memory.active_compaction_id().await.expect("pointer"), None);
        let loaded = store.read_checkpoint("c1").expect("read").expect("json");
        assert_eq!(loaded.status, CompactLlmCheckpointStatus::Pending);
        assert_no_transcript(&store);
    }

    #[tokio::test]
    async fn commit_sets_pointer_and_renames_pending_files_dir() {
        let (_dir, store, memory) = temp_store();
        let body = "notes from compact";
        let cp = sample_checkpoint("c2", vec![file_entry("notes.md", body)], None, 4);
        store.write_pending(&cp).await.expect("write_pending");
        write_attachment(&store.pending_files_dir("c2"), "notes.md", body);
        let committed = store.commit("c2").await.expect("commit");
        assert_eq!(committed.status, CompactLlmCheckpointStatus::Committed);
        assert_eq!(
            memory.active_compaction_id().await.expect("pointer"),
            Some("c2".into())
        );
        let active = store.load_active().await.expect("load_active");
        let active = active.expect("committed active");
        assert_eq!(active.id, "c2");
        assert_eq!(active.status, CompactLlmCheckpointStatus::Committed);
        assert!(store.committed_files_dir("c2").is_dir());
        assert!(!store.pending_files_dir("c2").exists());
        assert!(store.committed_files_dir("c2").join("notes.md").is_file());
        assert_no_transcript(&store);
    }

    #[tokio::test]
    async fn recover_discards_pending_and_keeps_old_active() {
        let (_dir, store, memory) = temp_store();
        let old = sample_checkpoint("old", Vec::new(), None, 2);
        store.write_pending(&old).await.expect("write old");
        store.commit("old").await.expect("commit old");
        assert_eq!(
            memory.active_compaction_id().await.expect("old pointer"),
            Some("old".into())
        );

        let next = sample_checkpoint("new", Vec::new(), Some("old".into()), 5);
        store.write_pending(&next).await.expect("write pending");
        assert!(store.checkpoint_json_path("new").is_file());
        assert!(store.pending_files_dir("new").is_dir());

        let recovered = store.recover().await.expect("recover");
        assert_eq!(recovered.expect("old active").id, "old");
        assert_eq!(
            memory.active_compaction_id().await.expect("pointer"),
            Some("old".into())
        );
        assert!(!store.checkpoint_json_path("new").exists());
        assert!(!store.pending_files_dir("new").exists());
        assert!(store.checkpoint_json_path("old").is_file());
        assert_no_transcript(&store);
    }

    #[tokio::test]
    async fn recover_repairs_missing_pointer_after_lineage_check() {
        let (_dir, store, memory) = temp_store();
        let body = "lineage notes";
        let cp = sample_checkpoint("c4", vec![file_entry("notes.md", body)], None, 3);
        store.write_pending(&cp).await.expect("write_pending");
        write_attachment(&store.pending_files_dir("c4"), "notes.md", body);
        store.commit("c4").await.expect("commit");
        memory
            .set_active_compaction_id(None)
            .await
            .expect("clear pointer");
        assert_eq!(memory.active_compaction_id().await.expect("cleared"), None);

        let recovered = store.recover().await.expect("recover");
        assert_eq!(recovered.expect("repaired").id, "c4");
        assert_eq!(
            memory.active_compaction_id().await.expect("pointer"),
            Some("c4".into())
        );
        let active = store.load_active().await.expect("load_active");
        assert_eq!(active.expect("active").id, "c4");
    }

    #[tokio::test]
    async fn find_reusable_returns_existing_id_for_same_key() {
        let (_dir, store, memory) = temp_store();
        let cp = sample_checkpoint("reuse-1", Vec::new(), None, 7);
        store.write_pending(&cp).await.expect("write_pending");
        let hit = store
            .find_reusable(
                &cp.conversation_id,
                cp.epoch,
                cp.covers_through_seq,
                &cp.source_slice_sha256,
                &cp.fingerprint,
            )
            .await
            .expect("find")
            .expect("hit");
        assert_eq!(hit.id, "reuse-1");
        assert_eq!(hit.status, CompactLlmCheckpointStatus::Pending);
        assert_eq!(memory.active_compaction_id().await.expect("pointer"), None);

        store.commit("reuse-1").await.expect("commit");
        let hit = store
            .find_reusable(
                &cp.conversation_id,
                cp.epoch,
                cp.covers_through_seq,
                &cp.source_slice_sha256,
                &cp.fingerprint,
            )
            .await
            .expect("find committed")
            .expect("hit committed");
        assert_eq!(hit.id, "reuse-1");
        assert_eq!(hit.status, CompactLlmCheckpointStatus::Committed);

        let miss = store
            .find_reusable(
                &cp.conversation_id,
                cp.epoch,
                cp.covers_through_seq,
                &cp.source_slice_sha256,
                "other-fingerprint",
            )
            .await
            .expect("miss lookup");
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn expand_summary_appends_verified_files_and_rejects_tampered_hash() {
        let (_dir, store, _memory) = temp_store();
        let body = "attachment body";
        let cp = sample_checkpoint("c6", vec![file_entry("notes.md", body)], None, 6);
        store.write_pending(&cp).await.expect("write_pending");
        write_attachment(&store.pending_files_dir("c6"), "notes.md", body);
        let committed = store.commit("c6").await.expect("commit");

        let expanded = store.expand_summary(&committed).expect("expand");
        let parts = text_parts(&expanded);
        assert_eq!(
            parts,
            vec![
                format!("{HISTORY_SUMMARY_ZH}kept work"),
                format!("## file: notes.md\n{body}"),
            ]
        );

        fs::write(
            store.committed_files_dir("c6").join("notes.md"),
            "ATTACHMENT BODY",
        )
        .expect("tamper");
        let err = store
            .expand_summary(&committed)
            .expect_err("tampered sha256");
        let CompactLlmError::Checkpoint(msg) = &err else {
            panic!("expected checkpoint error, got {err:?}");
        };
        assert!(msg.contains("sha256"), "{msg}");
    }

    #[tokio::test]
    async fn abort_pending_does_not_set_active_pointer() {
        let (_dir, store, memory) = temp_store();
        let cp = sample_checkpoint("c7", Vec::new(), None, 1);
        store.write_pending(&cp).await.expect("write_pending");
        store.abort_pending("c7").await.expect("abort");
        assert_eq!(memory.active_compaction_id().await.expect("pointer"), None);
        assert!(store.load_active().await.expect("load_active").is_none());
        assert!(!store.checkpoint_json_path("c7").exists());
        assert!(!store.pending_files_dir("c7").exists());
        assert_no_transcript(&store);
    }
}

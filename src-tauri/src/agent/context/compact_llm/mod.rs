//! Isolated LLM compaction branch (`compact-llm`).
//!
//! Default-off adapter: existing [`super::session_memory::SessionMemory`] and
//! [`super::compact::LlmCompactor`] stay on the original path. When enabled,
//! the main session chooses the compact-llm session view at the request
//! boundary. Derived summaries and attachments never enter `messages.jsonl`.
//!
//! Naming: module `compact_llm`, public types `CompactLlm*`, config id
//! `compact-llm`. This crate only wires persisted main-agent sessions.

pub mod branch;
pub mod checkpoint;
pub mod sandbox;
pub mod view;

pub use branch::{CompactLlmBranch, CompactLlmBranchOutput};
pub use checkpoint::CompactLlmCheckpointStore;
pub use sandbox::{CompactLlmSandbox, CompactLlmWriteFileTool};
pub use view::{CompactLlmArtifact, CompactLlmCompactor, CompactLlmSessionView};

use std::time::Duration;

use rig::completion::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

/// Capability / config identifier.
pub const COMPACT_LLM_ID: &str = "compact-llm";
/// Checkpoint + fingerprint format version.
pub const COMPACT_LLM_FORMAT_VERSION: u32 = 1;
pub const COMPACT_LLM_MAX_FILE_BYTES: u64 = 256 * 1024;
pub const COMPACT_LLM_MAX_FILE_COUNT: usize = 16;
pub const COMPACT_LLM_MAX_INPUT_BYTES: usize = 64 * 1024;
pub const COMPACT_LLM_MAX_TOTAL_BYTES: u64 = 1024 * 1024;
pub const COMPACT_LLM_MAX_TURNS: usize = 8;
pub const COMPACT_LLM_DEADLINE_SECS: u64 = 120;
pub const HISTORY_SUMMARY_ZH: &str = "历史摘要：";
pub const HISTORY_SUMMARY_EN: &str = "History summary:";

/// Opt-in compact-llm settings. [`Self::enabled`] defaults to false.
#[derive(Clone, Debug)]
pub struct CompactLlmConfig {
    pub enabled: bool,
    pub model_id: String,
    pub compact_prompt: String,
    pub max_turns: usize,
    pub max_input_bytes: usize,
    pub max_output_tokens: u64,
    pub deadline: Duration,
    pub max_file_bytes: u64,
    pub max_file_count: usize,
    pub max_total_bytes: u64,
}

impl CompactLlmConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            model_id: String::new(),
            compact_prompt: String::new(),
            max_turns: COMPACT_LLM_MAX_TURNS,
            max_input_bytes: COMPACT_LLM_MAX_INPUT_BYTES,
            max_output_tokens: super::compact::L2_MAX_TOKENS,
            deadline: Duration::from_secs(COMPACT_LLM_DEADLINE_SECS),
            max_file_bytes: COMPACT_LLM_MAX_FILE_BYTES,
            max_file_count: COMPACT_LLM_MAX_FILE_COUNT,
            max_total_bytes: COMPACT_LLM_MAX_TOTAL_BYTES,
        }
    }

    pub fn enabled(model_id: impl Into<String>, compact_prompt: impl Into<String>) -> Self {
        Self {
            enabled: true,
            model_id: model_id.into(),
            compact_prompt: compact_prompt.into(),
            ..Self::disabled()
        }
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.model_id, &self.compact_prompt)
    }
}

/// Per-turn cancellation bound to the compact-llm branch runner.
#[derive(Clone, Debug)]
pub struct CompactionControl {
    pub cancel: CancellationToken,
}

impl CompactionControl {
    pub fn new(cancel: CancellationToken) -> Self {
        Self { cancel }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Attachment listed on a compact-llm checkpoint. Paths are relative to
/// `compactions/<id>/`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactLlmFileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub media_type: String,
}

/// Pending or committed compact-llm checkpoint (`compactions/<id>.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactLlmCheckpoint {
    pub id: String,
    pub previous_id: Option<String>,
    pub conversation_id: String,
    pub epoch: u64,
    /// 1-based original prefix length absorbed into this summary.
    pub covers_through_seq: u64,
    pub source_last_seq: u64,
    /// SHA-256 hex of the incremental `new_slice` handed to the inner compactor.
    pub source_slice_sha256: String,
    pub summary_message: Message,
    pub files: Vec<CompactLlmFileEntry>,
    /// `{model}|{compact_prompt_sha256}|heuristic|{format_version}`.
    pub fingerprint: String,
    pub status: CompactLlmCheckpointStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_summary_tokens: Option<u64>,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactLlmCheckpointStatus {
    Pending,
    Committed,
}

#[derive(Debug, thiserror::Error)]
pub enum CompactLlmError {
    #[error("cancelled")]
    Cancelled,
    #[error("sandbox: {0}")]
    Sandbox(String),
    #[error("checkpoint: {0}")]
    Checkpoint(String),
    #[error("branch: {0}")]
    Branch(String),
    #[error("context_budget_exceeded: {0}")]
    Budget(String),
    #[error("{0}")]
    Other(String),
}

impl CompactLlmError {
    pub fn sandbox(msg: impl Into<String>) -> Self {
        Self::Sandbox(msg.into())
    }

    pub fn checkpoint(msg: impl Into<String>) -> Self {
        Self::Checkpoint(msg.into())
    }

    pub fn branch(msg: impl Into<String>) -> Self {
        Self::Branch(msg.into())
    }
}

impl From<CompactLlmError> for rig_memory::MemoryError {
    fn from(err: CompactLlmError) -> Self {
        match err {
            CompactLlmError::Budget(msg) => {
                rig_memory::MemoryError::Policy(format!("context_budget_exceeded: {msg}"))
            }
            CompactLlmError::Cancelled => rig_memory::MemoryError::Internal("cancelled".into()),
            other => rig_memory::MemoryError::Internal(other.to_string()),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn source_slice_sha256(messages: &[Message]) -> String {
    let bytes = serde_json::to_vec(messages).unwrap_or_default();
    sha256_hex(&bytes)
}

pub fn fingerprint(model_id: &str, compact_prompt: &str) -> String {
    format!(
        "{model_id}|{}|heuristic|{COMPACT_LLM_FORMAT_VERSION}",
        sha256_hex(compact_prompt.as_bytes())
    )
}

/// Prefix a summary with the fixed history-summary marker when missing.
pub fn annotate_history_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.starts_with(HISTORY_SUMMARY_ZH)
        || trimmed.starts_with("历史摘要:")
        || trimmed.starts_with("Conversation summary")
        || lower.starts_with("history summary:")
    {
        trimmed.to_string()
    } else {
        format!("{HISTORY_SUMMARY_ZH}{trimmed}")
    }
}

pub fn is_history_summary_message(message: &Message) -> bool {
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
    trimmed.starts_with(HISTORY_SUMMARY_ZH)
        || trimmed.starts_with("历史摘要:")
        || trimmed.starts_with("Conversation summary")
        || lower.starts_with("history summary:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_llm_is_disabled_by_default() {
        let cfg = CompactLlmConfig::disabled();
        assert!(!cfg.enabled);
        assert_eq!(COMPACT_LLM_ID, "compact-llm");
    }

    #[test]
    fn annotate_does_not_double_prefix() {
        let once = annotate_history_summary("keep the tests green");
        assert!(once.starts_with(HISTORY_SUMMARY_ZH), "{once}");
        let twice = annotate_history_summary(&once);
        assert_eq!(once, twice);
    }
}

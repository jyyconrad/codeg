//! ContextStore (canonical facts) + ContextView (per-request projection).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

use super::transcript::{
    acp_status_for, compact_update_payload, tool_call_payload, tool_call_update_payload,
    ModelCommit, NativeMeta, OutputLocator,
};
use crate::acp_transcript::{self, EntryKind};

pub use super::transcript::CompactRecord;

pub use super::transcript::{ToolOutcome, ToolPhase};

const CRITICAL_ACK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    Reported,
    Estimated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionFact {
    pub tool_call_id: String,
    pub function_name: String,
    pub raw_input: Value,
    pub phase: ToolPhase,
    pub outcome: Option<ToolOutcome>,
    pub executed: Option<bool>,
    pub model_presentation: Option<String>,
    pub truncated: bool,
    pub output_locator: Option<OutputLocator>,
    pub reason: Option<String>,
    pub turn_id: String,
}

impl ExecutionFact {
    pub fn pending(
        turn_id: impl Into<String>,
        tool_call_id: impl Into<String>,
        function_name: impl Into<String>,
        raw_input: Value,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            function_name: function_name.into(),
            raw_input,
            phase: ToolPhase::Pending,
            outcome: None,
            executed: Some(false),
            model_presentation: None,
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: turn_id.into(),
        }
    }

    pub fn outcome_feedback(&self) -> String {
        match self.outcome {
            Some(ToolOutcome::Success) => self
                .model_presentation
                .clone()
                .unwrap_or_else(|| "ok".into()),
            Some(ToolOutcome::Rejected) => format!(
                "tool `{}` was rejected and was not executed",
                self.function_name
            ),
            Some(ToolOutcome::Cancelled) => format!(
                "tool `{}` was cancelled before execution",
                self.function_name
            ),
            Some(ToolOutcome::Unknown) => format!(
                "tool `{}` started but completion could not be confirmed; not replayed",
                self.function_name
            ),
            Some(ToolOutcome::Timeout) => format!("tool `{}` timed out", self.function_name),
            Some(ToolOutcome::Error) => self
                .reason
                .clone()
                .unwrap_or_else(|| format!("tool `{}` failed", self.function_name)),
            None => format!("tool `{}` has no terminal result", self.function_name),
        }
    }

    pub fn native_meta(&self) -> NativeMeta {
        let mut meta = NativeMeta::v1();
        meta.turn_id = Some(self.turn_id.clone());
        meta.tool_call_id = Some(self.tool_call_id.clone());
        meta.function_name = Some(self.function_name.clone());
        meta.raw_input = Some(self.raw_input.clone());
        meta.phase = Some(self.phase);
        meta.outcome = self.outcome;
        meta.executed = self.executed;
        meta.reason = self.reason.clone();
        meta.model_presentation = self.model_presentation.clone();
        meta.truncated = Some(self.truncated);
        meta.output_locator = self.output_locator.clone();
        meta
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssistantPart {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssistantRecord {
    pub model_message_id: Option<String>,
    pub committed: bool,
    pub parts: Vec<AssistantPart>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalTurn {
    pub turn_id: String,
    pub user_text: String,
    pub assistant: Option<AssistantRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextView {
    pub messages: Vec<rig::completion::Message>,
    pub omitted_turns: usize,
    pub estimated_tokens: u64,
    pub window: u64,
    pub input_budget: u64,
    pub source: UsageSource,
    pub compact_level: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextStore {
    session_id: String,
    turns: Vec<CanonicalTurn>,
    facts: HashMap<String, ExecutionFact>,
    compact: Option<CompactRecord>,
}

impl ContextStore {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            turns: Vec::new(),
            facts: HashMap::new(),
            compact: None,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn turns(&self) -> &[CanonicalTurn] {
        &self.turns
    }

    pub fn fact(&self, tool_call_id: &str) -> Option<&ExecutionFact> {
        self.facts.get(tool_call_id)
    }

    pub fn facts(&self) -> impl Iterator<Item = &ExecutionFact> {
        self.facts.values()
    }

    pub fn compact(&self) -> Option<&CompactRecord> {
        self.compact.as_ref()
    }

    pub fn set_compact(&mut self, record: CompactRecord) {
        self.compact = Some(record);
    }

    pub fn append_user(&mut self, turn_id: String, user_text: String) {
        self.turns.push(CanonicalTurn {
            turn_id,
            user_text,
            assistant: None,
        });
    }

    pub fn prompt_index(&self) -> u64 {
        self.turns.len() as u64
    }

    pub fn turn_id_for_prompt(&self, prompt_index: u64) -> String {
        format!("{}:{prompt_index}", self.session_id)
    }

    pub fn record_fact(&mut self, fact: ExecutionFact) {
        self.facts.insert(fact.tool_call_id.clone(), fact);
    }

    pub fn upsert_fact<F>(&mut self, tool_call_id: &str, update: F)
    where
        F: FnOnce(&mut ExecutionFact),
    {
        if let Some(fact) = self.facts.get_mut(tool_call_id) {
            update(fact);
        }
    }

    pub fn commit_assistant(&mut self, turn_id: &str, assistant: AssistantRecord) {
        if let Some(turn) = self.turns.iter_mut().rev().find(|t| t.turn_id == turn_id) {
            turn.assistant = Some(assistant);
        }
    }

    /// v1 never auto-replays tools. Unknown / cancelled / rejected stay recorded.
    pub fn auto_replay_ids(&self) -> Vec<String> {
        Vec::new()
    }

    /// Close an in-flight tool batch: keep confirmed successes; mark unstarted
    /// calls cancelled; mark started-without-terminal unknown. Rig history that
    /// omits A cannot retract A's fact.
    pub fn settle_cancel(&mut self, turn_id: &str) {
        for fact in self.facts.values_mut() {
            if fact.turn_id != turn_id {
                continue;
            }
            match (fact.phase, fact.outcome) {
                (ToolPhase::Terminal, Some(_)) => {}
                (ToolPhase::Started, _) => {
                    fact.phase = ToolPhase::Terminal;
                    fact.outcome = Some(ToolOutcome::Unknown);
                    fact.executed = None;
                    fact.reason = Some("started but cancelled before a terminal ack".into());
                    fact.model_presentation = Some(fact.outcome_feedback());
                }
                _ => {
                    fact.phase = ToolPhase::Terminal;
                    fact.outcome = Some(ToolOutcome::Cancelled);
                    fact.executed = Some(false);
                    fact.reason = Some("cancelled before execution".into());
                    fact.model_presentation = Some(fact.outcome_feedback());
                }
            }
        }
    }

    pub fn unknown_ids(&self) -> Vec<String> {
        self.facts
            .values()
            .filter(|f| f.outcome == Some(ToolOutcome::Unknown))
            .map(|f| f.tool_call_id.clone())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CallIdentity {
    pub turn_id: u64,
    pub turn_key: String,
    pub tool_call_id: String,
    pub function_name: String,
}

#[derive(Clone, Default)]
pub struct CallIdentityBridge {
    inner: Arc<Mutex<Option<CallIdentity>>>,
}

impl CallIdentityBridge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, identity: CallIdentity) {
        *self.inner.lock().expect("identity") = Some(identity);
    }

    pub fn clear(&self) {
        *self.inner.lock().expect("identity") = None;
    }

    pub fn require(&self, turn_id: u64) -> Result<CallIdentity, String> {
        let guard = self.inner.lock().expect("identity");
        let Some(id) = guard.as_ref() else {
            return Err("missing call identity".into());
        };
        if id.turn_id != turn_id {
            return Err(format!(
                "call identity turn {} does not match current {turn_id}",
                id.turn_id
            ));
        }
        Ok(id.clone())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FactWriteError {
    #[error("critical transcript write was not acknowledged")]
    AckFailed,
}

#[derive(Clone)]
pub struct FactRecorder {
    agent_dir: String,
    session_id: String,
    root: PathBuf,
    store: Arc<Mutex<ContextStore>>,
    fail_started: Arc<AtomicBool>,
    fail_turn_end: Arc<AtomicBool>,
    memory_only: bool,
    writes: Arc<Mutex<Vec<String>>>,
}

impl FactRecorder {
    pub fn transcript(
        root: PathBuf,
        agent_dir: impl Into<String>,
        session_id: impl Into<String>,
        store: Arc<Mutex<ContextStore>>,
    ) -> Self {
        Self {
            agent_dir: agent_dir.into(),
            session_id: session_id.into(),
            root,
            store,
            fail_started: Arc::new(AtomicBool::new(false)),
            fail_turn_end: Arc::new(AtomicBool::new(false)),
            memory_only: false,
            writes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn memory(store: Arc<Mutex<ContextStore>>) -> Self {
        let session_id = store.lock().expect("store").session_id().to_string();
        Self {
            agent_dir: "mem".into(),
            session_id,
            root: PathBuf::from("/tmp"),
            store,
            fail_started: Arc::new(AtomicBool::new(false)),
            fail_turn_end: Arc::new(AtomicBool::new(false)),
            memory_only: true,
            writes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn fail_next_started(&self) {
        self.fail_started.store(true, Ordering::SeqCst);
    }

    pub fn fail_next_turn_end(&self) {
        self.fail_turn_end.store(true, Ordering::SeqCst);
    }

    pub fn writes(&self) -> Vec<String> {
        self.writes.lock().expect("writes").clone()
    }

    pub fn store(&self) -> Arc<Mutex<ContextStore>> {
        Arc::clone(&self.store)
    }

    pub fn spill_dir(&self) -> PathBuf {
        super::spill::spill_dir(&self.root, &self.agent_dir, &self.session_id)
    }

    pub async fn record_fact_update(&self, fact: &ExecutionFact) -> Result<(), FactWriteError> {
        self.write_tool(fact, fact.phase).await
    }

    pub async fn record_started(&self, fact: &ExecutionFact) -> Result<(), FactWriteError> {
        if self.fail_started.swap(false, Ordering::SeqCst) {
            return Err(FactWriteError::AckFailed);
        }
        self.write_tool(fact, ToolPhase::Started).await?;
        self.store.lock().expect("store").record_fact(fact.clone());
        self.writes
            .lock()
            .expect("writes")
            .push(format!("started:{}", fact.tool_call_id));
        Ok(())
    }

    pub async fn record_terminal(&self, fact: &ExecutionFact) -> Result<(), FactWriteError> {
        self.write_tool(fact, ToolPhase::Terminal).await?;
        self.store.lock().expect("store").record_fact(fact.clone());
        self.writes
            .lock()
            .expect("writes")
            .push(format!("terminal:{}", fact.tool_call_id));
        Ok(())
    }

    pub async fn record_prompt(&self, blocks: Value) -> Result<(), FactWriteError> {
        self.ack(acp_transcript::record_entry_critical_in(
            &self.root,
            &self.agent_dir,
            &self.session_id,
            EntryKind::Prompt,
            blocks,
        ))
        .await
    }

    pub async fn record_update(&self, payload: Value) -> Result<(), FactWriteError> {
        self.ack(acp_transcript::record_entry_critical_in(
            &self.root,
            &self.agent_dir,
            &self.session_id,
            EntryKind::Update,
            payload,
        ))
        .await
    }

    pub async fn record_turn_end(&self, stop_reason: &str) -> Result<(), FactWriteError> {
        if self.fail_turn_end.swap(false, Ordering::SeqCst) {
            return Err(FactWriteError::AckFailed);
        }
        self.ack(acp_transcript::record_entry_critical_in(
            &self.root,
            &self.agent_dir,
            &self.session_id,
            EntryKind::TurnEnd,
            serde_json::json!({ "stopReason": stop_reason }),
        ))
        .await
    }

    pub async fn record_model_commit(
        &self,
        turn_id: &str,
        commit: &ModelCommit,
        last_payload: Option<Value>,
    ) -> Result<(), FactWriteError> {
        let mut native = NativeMeta::v1();
        native.turn_id = Some(turn_id.to_string());
        native.model_commit = Some(commit.clone());
        let payload = match last_payload {
            Some(payload) => super::transcript::attach_native_meta(payload, &native),
            None => super::transcript::agent_message_chunk("", &native),
        };
        self.record_update(payload).await
    }

    pub async fn record_compact(&self, record: &CompactRecord) -> Result<(), FactWriteError> {
        self.writes
            .lock()
            .expect("writes")
            .push(format!("compact:{}", record.level));
        if self.memory_only {
            return Ok(());
        }
        self.record_update(compact_update_payload(record)).await
    }

    async fn write_tool(
        &self,
        fact: &ExecutionFact,
        phase: ToolPhase,
    ) -> Result<(), FactWriteError> {
        if self.memory_only {
            return Ok(());
        }
        let mut fact = fact.clone();
        fact.phase = phase;
        let status = acp_status_for(fact.outcome, phase);
        let payload = if phase == ToolPhase::Pending {
            tool_call_payload(
                &fact.tool_call_id,
                &fact.function_name,
                status,
                &fact.raw_input,
                &fact.native_meta(),
            )
        } else {
            tool_call_update_payload(&fact.tool_call_id, status, &fact.native_meta())
        };
        self.record_update(payload).await
    }

    async fn ack(&self, rx: oneshot::Receiver<()>) -> Result<(), FactWriteError> {
        if self.memory_only {
            return Ok(());
        }
        match tokio::time::timeout(CRITICAL_ACK, rx).await {
            Ok(Ok(())) => Ok(()),
            _ => Err(FactWriteError::AckFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_keeps_confirmed_success_and_marks_unstarted_cancelled() {
        let mut store = ContextStore::new("sess");
        store.append_user("sess:1".into(), "do both".into());
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
            turn_id: "sess:1".into(),
        });
        store.record_fact(ExecutionFact::pending(
            "sess:1",
            "call_b",
            "write_mem",
            serde_json::json!({"text": "B"}),
        ));
        store.settle_cancel("sess:1");
        let a = store.fact("call_a").unwrap();
        assert_eq!(a.outcome, Some(ToolOutcome::Success));
        assert_eq!(a.executed, Some(true));
        let b = store.fact("call_b").unwrap();
        assert_eq!(b.outcome, Some(ToolOutcome::Cancelled));
        assert_eq!(b.executed, Some(false));
        assert!(store.auto_replay_ids().is_empty());
    }

    #[test]
    fn started_without_terminal_becomes_unknown_and_is_not_replayed() {
        let mut store = ContextStore::new("sess");
        store.append_user("sess:1".into(), "write".into());
        store.record_fact(ExecutionFact {
            tool_call_id: "call_u".into(),
            function_name: "write_mem".into(),
            raw_input: serde_json::json!({}),
            phase: ToolPhase::Started,
            outcome: None,
            executed: None,
            model_presentation: None,
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: "sess:1".into(),
        });
        store.settle_cancel("sess:1");
        let u = store.fact("call_u").unwrap();
        assert_eq!(u.outcome, Some(ToolOutcome::Unknown));
        assert!(u.executed.is_none());
        assert!(store.auto_replay_ids().is_empty());
        assert_eq!(store.unknown_ids(), vec!["call_u".to_string()]);
    }

    #[test]
    fn rig_history_without_a_cannot_drop_a_fact() {
        let mut store = ContextStore::new("sess");
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
            turn_id: "sess:1".into(),
        });
        // Simulate a cancelled runner history that never mentioned A.
        store.settle_cancel("sess:1");
        assert_eq!(
            store.fact("call_a").unwrap().outcome,
            Some(ToolOutcome::Success)
        );
    }

    #[tokio::test]
    async fn started_ack_failure_does_not_commit_the_fact() {
        let store = Arc::new(Mutex::new(ContextStore::new("sess")));
        let recorder = FactRecorder::memory(Arc::clone(&store));
        recorder.fail_next_started();
        let fact = ExecutionFact::pending("sess:1", "call_x", "write_mem", serde_json::json!({}));
        let mut started = fact.clone();
        started.phase = ToolPhase::Started;
        assert!(recorder.record_started(&started).await.is_err());
        assert!(store.lock().unwrap().fact("call_x").is_none());
    }
}

//! Convert an ACP-native JSONL transcript into `Vec<rig::completion::Message>`.
//!
//! Used later by hydrate. Does not switch `OpenedSession` / `ContextStore`.
//! Old `batch_id` values collide and are never used as keys.

use std::collections::{HashMap, HashSet};

use rig::completion::message::{
    AssistantContent, ToolCall, ToolCallId, ToolFunction, ToolResultContent, UserContent,
};
use rig::completion::Message;
use serde_json::Value;

use super::hydrate::HydrateError;
use super::store::ExecutionFact;
use super::transcript::{
    extract_native_meta, session_update_kind, tool_call_id_of, CompactRecord, ModelCommit,
    NativeMeta, NativeMetaError, ToolOutcome, ToolPhase,
};
use crate::acp_transcript::{EntryKind, TranscriptEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MigratedMessages {
    pub messages: Vec<Message>,
    pub warnings: Vec<MigrateWarning>,
    pub complete: bool,
}

pub fn migrate_transcript_entries(
    session_id: &str,
    entries: &[TranscriptEntry],
) -> Result<MigratedMessages, HydrateError> {
    let mut migrator = Migrator::new(session_id);
    for entry in entries {
        migrator.apply_entry(entry)?;
    }
    migrator.finish();
    Ok(migrator.into_result())
}

struct PendingAssistant {
    model_message_id: Option<String>,
    text: String,
    call_ids: Vec<String>,
    parts_hint: usize,
}

struct Migrator<'a> {
    session_id: &'a str,
    messages: Vec<Message>,
    warnings: Vec<MigrateWarning>,
    complete: bool,
    warned: HashSet<&'static str>,
    open_text: String,
    open_model_message_id: Option<String>,
    facts: HashMap<String, ExecutionFact>,
    pending: Option<PendingAssistant>,
    compact: Option<CompactRecord>,
    prompt_index: u64,
    current_turn: Option<String>,
    emitted_calls: HashSet<String>,
}

impl<'a> Migrator<'a> {
    fn new(session_id: &'a str) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            warnings: Vec::new(),
            complete: true,
            warned: HashSet::new(),
            open_text: String::new(),
            open_model_message_id: None,
            facts: HashMap::new(),
            pending: None,
            compact: None,
            prompt_index: 0,
            current_turn: None,
            emitted_calls: HashSet::new(),
        }
    }

    fn failed(&self, message: impl Into<String>) -> HydrateError {
        HydrateError::LoadFailed {
            session_id: self.session_id.to_string(),
            code: "session_unavailable",
            message: message.into(),
        }
    }

    fn warn(&mut self, code: &'static str, message: impl Into<String>) {
        if self.warned.insert(code) {
            self.warnings.push(MigrateWarning {
                code,
                message: message.into(),
            });
        }
    }

    fn apply_entry(&mut self, entry: &TranscriptEntry) -> Result<(), HydrateError> {
        match entry.k {
            EntryKind::Prompt => {
                self.flush_pending();
                self.flush_uncommitted_text();
                self.prompt_index += 1;
                let turn_id = format!("{}:{}", self.session_id, self.prompt_index);
                let user_text = prompt_text(&entry.p);
                if !user_text.is_empty() {
                    self.messages.push(Message::user(user_text));
                }
                self.current_turn = Some(turn_id);
            }
            EntryKind::TurnEnd => {
                self.flush_pending();
                self.flush_uncommitted_text();
            }
            EntryKind::Update => match extract_native_meta(&entry.p) {
                Err(NativeMetaError::UnsupportedVersion(v)) => {
                    return Err(
                        self.failed(format!("unsupported codeg_native metadata version {v}"))
                    );
                }
                Err(NativeMetaError::Malformed) => {
                    return Err(self.failed("malformed codeg_native metadata"));
                }
                Ok(None) => {
                    if matches!(
                        session_update_kind(&entry.p),
                        Some("tool_call" | "tool_call_update")
                    ) {
                        return Err(self.failed(
                            "tool call is missing required codeg_native execution metadata",
                        ));
                    }
                    if session_update_kind(&entry.p) == Some("agent_message_chunk") {
                        self.push_chunk_text(&entry.p);
                    }
                }
                Ok(Some(meta)) => self.apply_native_update(&entry.p, meta)?,
            },
        }
        Ok(())
    }

    fn apply_native_update(
        &mut self,
        payload: &Value,
        meta: NativeMeta,
    ) -> Result<(), HydrateError> {
        self.push_chunk_text(payload);
        if meta.model_message_id.is_some() {
            self.open_model_message_id = meta.model_message_id.clone();
        }
        let kind = session_update_kind(payload).unwrap_or("");
        if matches!(kind, "tool_call" | "tool_call_update") {
            self.apply_tool(payload, &meta)?;
        }
        if let Some(commit) = meta.model_commit {
            self.apply_commit(commit);
        }
        if let Some(compact) = meta.compact {
            self.compact = Some(compact);
        }
        Ok(())
    }

    fn push_chunk_text(&mut self, payload: &Value) {
        if let Some(text) = chunk_text(payload) {
            if !text.is_empty() {
                self.open_text.push_str(&text);
            }
        }
    }

    fn apply_tool(&mut self, payload: &Value, meta: &NativeMeta) -> Result<(), HydrateError> {
        let tool_call_id = meta
            .tool_call_id
            .clone()
            .or_else(|| tool_call_id_of(payload).map(str::to_string))
            .ok_or_else(|| self.failed("tool call missing id"))?;
        if meta.function_name.is_none() && !self.facts.contains_key(&tool_call_id) {
            // Commit is often attached to the last tool card and overwrites native
            // execution fields. Subsequent started/terminal records carry the fact.
            if meta.model_commit.is_some() {
                return Ok(());
            }
            return Err(
                self.failed("tool call missing function_name; refusing to infer from title")
            );
        }
        let raw_input = meta
            .raw_input
            .clone()
            .or_else(|| payload.get("rawInput").cloned())
            .unwrap_or(Value::Null);
        let mut fact = if let Some(existing) = self.facts.remove(&tool_call_id) {
            existing
        } else {
            let function_name = meta.function_name.clone().ok_or_else(|| {
                self.failed("tool call missing function_name; refusing to infer from title")
            })?;
            let turn_id = meta
                .turn_id
                .clone()
                .or_else(|| self.current_turn.clone())
                .ok_or_else(|| self.failed("tool call is missing a model message boundary"))?;
            ExecutionFact::pending(turn_id, &tool_call_id, function_name, raw_input.clone())
        };
        if let Some(name) = meta.function_name.clone() {
            fact.function_name = name;
        }
        if meta.raw_input.is_some() || payload.get("rawInput").is_some() {
            fact.raw_input = raw_input;
        }
        if let Some(phase) = meta.phase {
            fact.phase = phase;
        }
        if meta.outcome.is_some() {
            fact.outcome = meta.outcome;
        }
        if meta.executed.is_some() {
            fact.executed = meta.executed;
        }
        if meta.model_presentation.is_some() {
            fact.model_presentation = meta.model_presentation.clone();
        }
        if let Some(reason) = meta.reason.clone() {
            fact.reason = Some(reason);
        }
        if meta.truncated == Some(true) {
            fact.truncated = true;
        }
        if meta.output_locator.is_some() {
            fact.output_locator = meta.output_locator.clone();
        }
        self.facts.insert(tool_call_id, fact);
        Ok(())
    }

    fn apply_commit(&mut self, commit: ModelCommit) {
        // batch_id collides across model rounds; associate only by call_ids order.
        let ModelCommit {
            batch_id: _,
            parts,
            call_ids,
        } = commit;
        self.flush_pending();
        let text = std::mem::take(&mut self.open_text);
        let model_message_id = self.open_model_message_id.take();
        if text.is_empty() && call_ids.is_empty() {
            return;
        }
        self.pending = Some(PendingAssistant {
            model_message_id,
            text,
            call_ids,
            parts_hint: parts.len(),
        });
    }

    fn flush_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        for id in &pending.call_ids {
            if let Some(fact) = self.facts.get_mut(id) {
                settle_fact(fact);
            }
        }
        let mut content = Vec::new();
        if !pending.text.is_empty() {
            content.push(AssistantContent::text(pending.text));
        }
        let mut results = Vec::new();
        let mut missing = Vec::new();
        for id in &pending.call_ids {
            let Some(fact) = self.facts.get(id) else {
                missing.push(id.clone());
                continue;
            };
            let name = fact.function_name.clone();
            let args = fact.raw_input.clone();
            let call = ToolCall::new(
                ToolCallId::new_or_mint(id.clone()),
                ToolFunction::new(name.clone(), args),
            );
            content.push(AssistantContent::ToolCall(call));
            results.push(UserContent::tool_result(
                id.clone(),
                name,
                vec![ToolResultContent::text(tool_result_body(fact))],
            ));
            self.emitted_calls.insert(id.clone());
        }
        if !missing.is_empty() {
            self.complete = false;
            self.warn(
                "incomplete_tool_chain",
                format!(
                    "model commit call_ids had no recoverable native execution metadata: {}",
                    missing.join(", ")
                ),
            );
        }
        if !pending.call_ids.is_empty() {
            self.warn(
                "missing_provider_ids",
                "old JSONL has no provider dual-ids or reasoning; not fabricated",
            );
        }
        if pending.parts_hint > 0 && content.len() < pending.parts_hint {
            self.warn(
                "missing_assistant_parts",
                "old JSONL assistant parts are incomplete; missing text or reasoning was not fabricated",
            );
        }
        if !content.is_empty() {
            self.messages.push(Message::Assistant {
                id: pending.model_message_id,
                content,
            });
        }
        if !results.is_empty() {
            self.messages.push(Message::User { content: results });
        }
    }

    fn flush_uncommitted_text(&mut self) {
        if self.open_text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.open_text);
        self.messages.push(Message::Assistant {
            id: self.open_model_message_id.take(),
            content: vec![AssistantContent::text(text)],
        });
        self.warn(
            "uncommitted_assistant_text",
            "assistant text had no model_commit boundary; recovered as a text message",
        );
    }

    fn finish(&mut self) {
        self.flush_pending();
        self.flush_uncommitted_text();
        let unpaired: Vec<String> = self
            .facts
            .keys()
            .filter(|id| !self.emitted_calls.contains(*id))
            .cloned()
            .collect();
        if !unpaired.is_empty() {
            self.complete = false;
            self.warn(
                "unpaired_tool_facts",
                format!(
                    "tool execution records were not paired with a model_commit call_id: {}",
                    unpaired.join(", ")
                ),
            );
        }
        if self.compact.is_some() {
            // through_turn is a legacy turn id, not a message seq. One user
            // turn can contain several assistant/tool-result messages (2+4+3).
            self.warn(
                "compact_not_applied",
                "old CompactRecord through_turn could not be mapped to a legal message seq; keeping full recovered messages",
            );
        }
    }

    fn into_result(self) -> MigratedMessages {
        MigratedMessages {
            messages: self.messages,
            warnings: self.warnings,
            complete: self.complete,
        }
    }
}

fn settle_fact(fact: &mut ExecutionFact) {
    match (fact.phase, fact.outcome) {
        (_, Some(_)) => {}
        (ToolPhase::Started, None) => {
            fact.phase = ToolPhase::Terminal;
            fact.outcome = Some(ToolOutcome::Unknown);
            fact.executed = None;
            fact.reason = Some("started without a terminal record".into());
            fact.model_presentation = Some(fact.outcome_feedback());
        }
        (ToolPhase::Pending, None) => {
            fact.phase = ToolPhase::Terminal;
            fact.outcome = Some(ToolOutcome::Cancelled);
            fact.executed = Some(false);
            fact.reason = Some("never started".into());
            fact.model_presentation = Some(fact.outcome_feedback());
        }
        (ToolPhase::Terminal, None) => {
            fact.outcome = Some(ToolOutcome::Unknown);
            fact.model_presentation = Some(fact.outcome_feedback());
        }
    }
}

fn tool_result_body(fact: &ExecutionFact) -> String {
    match fact.outcome {
        Some(ToolOutcome::Success) => fact
            .model_presentation
            .clone()
            .unwrap_or_else(|| fact.outcome_feedback()),
        _ => fact.outcome_feedback(),
    }
}

fn prompt_text(payload: &Value) -> String {
    let Some(items) = payload.as_array() else {
        return payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    };
    items
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn chunk_text(payload: &Value) -> Option<String> {
    payload
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::transcript::{
        agent_message_chunk, attach_native_meta, compact_update_payload, tool_call_payload,
        tool_call_update_payload,
    };
    use serde_json::json;

    fn prompt(text: &str) -> TranscriptEntry {
        TranscriptEntry {
            t: 1,
            k: EntryKind::Prompt,
            p: json!([{"type": "text", "text": text}]),
        }
    }

    fn text_chunk(text: &str) -> TranscriptEntry {
        TranscriptEntry {
            t: 1,
            k: EntryKind::Update,
            p: agent_message_chunk(text, &NativeMeta::v1()),
        }
    }

    fn commit(
        batch_id: &str,
        call_ids: &[&str],
        model_message_id: Option<&str>,
    ) -> TranscriptEntry {
        let mut meta = NativeMeta::v1();
        meta.model_message_id = model_message_id.map(str::to_string);
        meta.model_commit = Some(ModelCommit {
            batch_id: batch_id.into(),
            parts: call_ids.iter().map(|id| (*id).to_string()).collect(),
            call_ids: call_ids.iter().map(|id| (*id).to_string()).collect(),
        });
        TranscriptEntry {
            t: 1,
            k: EntryKind::Update,
            p: agent_message_chunk("", &meta),
        }
    }

    fn native_tool(
        id: &str,
        name: &str,
        phase: ToolPhase,
        outcome: Option<ToolOutcome>,
        input: Value,
    ) -> NativeMeta {
        let mut meta = NativeMeta::v1();
        meta.turn_id = Some("s1:1".into());
        meta.tool_call_id = Some(id.into());
        meta.function_name = Some(name.into());
        meta.raw_input = Some(input);
        meta.phase = Some(phase);
        meta.outcome = outcome;
        meta.executed = match outcome {
            Some(ToolOutcome::Success) => Some(true),
            Some(ToolOutcome::Cancelled | ToolOutcome::Rejected) => Some(false),
            _ => None,
        };
        meta
    }

    fn tool_started(id: &str, name: &str, title: &str, input: Value) -> TranscriptEntry {
        let meta = native_tool(id, name, ToolPhase::Started, None, input.clone());
        TranscriptEntry {
            t: 1,
            k: EntryKind::Update,
            p: tool_call_payload(id, title, "pending", &input, &meta),
        }
    }

    fn tool_terminal(
        id: &str,
        name: &str,
        presentation: &str,
        outcome: ToolOutcome,
        input: Value,
    ) -> TranscriptEntry {
        let mut meta = native_tool(id, name, ToolPhase::Terminal, Some(outcome), input);
        meta.model_presentation = Some(presentation.into());
        TranscriptEntry {
            t: 1,
            k: EntryKind::Update,
            p: tool_call_update_payload(id, "completed", &meta),
        }
    }

    fn assistant_calls(message: &Message) -> Vec<(String, String, Value)> {
        let Message::Assistant { content, .. } = message else {
            return Vec::new();
        };
        content
            .iter()
            .filter_map(|part| match part {
                AssistantContent::ToolCall(tc) => Some((
                    tc.id.as_str().to_string(),
                    tc.function.name.clone(),
                    tc.function.arguments.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn assistant_text(message: &Message) -> String {
        let Message::Assistant { content, .. } = message else {
            return String::new();
        };
        content
            .iter()
            .filter_map(|part| match part {
                AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn tool_result_parts(message: &Message) -> Vec<(String, String, String)> {
        let Message::User { content } = message else {
            return Vec::new();
        };
        content
            .iter()
            .filter_map(|part| match part {
                UserContent::ToolResult(result) => {
                    let body = result
                        .content
                        .iter()
                        .filter_map(ToolResultContent::as_text)
                        .collect::<Vec<_>>()
                        .join("");
                    Some((result.call.as_str().to_string(), result.name.clone(), body))
                }
                _ => None,
            })
            .collect()
    }

    fn user_text(message: &Message) -> Option<&str> {
        let Message::User { content } = message else {
            return None;
        };
        content.iter().find_map(|part| match part {
            UserContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
    }

    fn is_assistant_tools(message: &Message) -> bool {
        !assistant_calls(message).is_empty()
    }

    fn is_tool_results(message: &Message) -> bool {
        !tool_result_parts(message).is_empty()
    }

    #[test]
    fn three_commits_2_4_3_keep_all_calls_despite_colliding_batch_id() {
        const BATCH: &str = "batch-collide";
        let round1 = ["c1", "c2"];
        let round2 = ["c3", "c4", "c5", "c6"];
        let round3 = ["c7", "c8", "c9"];
        let mut entries = vec![prompt("do the work")];
        for (text, msg_id, ids) in [
            ("round-1", "msg-1", round1.as_slice()),
            ("round-2", "msg-2", round2.as_slice()),
            ("round-3", "msg-3", round3.as_slice()),
        ] {
            entries.push(text_chunk(text));
            entries.push(commit(BATCH, ids, Some(msg_id)));
            for id in ids {
                let input = json!({"id": id});
                entries.push(tool_started(id, "write_mem", "Write memory", input.clone()));
                entries.push(tool_terminal(
                    id,
                    "write_mem",
                    &format!("out-{id}"),
                    ToolOutcome::Success,
                    input,
                ));
            }
        }

        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        assert!(out.complete, "warnings: {:?}", out.warnings);

        let assistant_tool_msgs: Vec<_> = out
            .messages
            .iter()
            .filter(|m| is_assistant_tools(m))
            .collect();
        let result_msgs: Vec<_> = out.messages.iter().filter(|m| is_tool_results(m)).collect();
        assert_eq!(assistant_tool_msgs.len(), 3);
        assert_eq!(result_msgs.len(), 3);

        let expected = [round1.as_slice(), round2.as_slice(), round3.as_slice()];
        for (i, ids) in expected.into_iter().enumerate() {
            let calls = assistant_calls(assistant_tool_msgs[i]);
            let got: Vec<&str> = calls.iter().map(|(id, _, _)| id.as_str()).collect();
            assert_eq!(got, ids, "commit {i} call order");
            let results = tool_result_parts(result_msgs[i]);
            let got_results: Vec<&str> = results.iter().map(|(id, _, _)| id.as_str()).collect();
            assert_eq!(got_results, ids, "commit {i} result order");
            assert_eq!(results.len(), ids.len());
        }

        let all_ids: Vec<_> = assistant_tool_msgs
            .iter()
            .flat_map(|m| assistant_calls(m))
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(all_ids.len(), 9);

        match &out.messages[1] {
            Message::Assistant { id, .. } => assert_eq!(id.as_deref(), Some("msg-1")),
            other => panic!("expected assistant, got {other:?}"),
        }
        assert_eq!(user_text(&out.messages[0]), Some("do the work"));
    }

    #[test]
    fn function_names_come_from_native_meta_not_titles() {
        let input = json!({"path": "a.txt"});
        let entries = vec![
            prompt("write"),
            commit("batch-collide", &["call_a"], Some("msg-1")),
            tool_started("call_a", "write_mem", "Write memory", input.clone()),
            tool_terminal(
                "call_a",
                "write_mem",
                "wrote A",
                ToolOutcome::Success,
                input.clone(),
            ),
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        let calls = assistant_calls(&out.messages[1]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "write_mem");
        assert_eq!(calls[0].2, input);
        assert_ne!(calls[0].1, "Write memory");
        let results = tool_result_parts(&out.messages[2]);
        assert_eq!(results[0].1, "write_mem");
    }

    #[test]
    fn duplicate_terminal_updates_collapse_to_one_final_result() {
        let input = json!({"text": "A"});
        let first = tool_terminal(
            "call_a",
            "echo",
            "first",
            ToolOutcome::Success,
            input.clone(),
        );
        let second = tool_terminal(
            "call_a",
            "echo",
            "second",
            ToolOutcome::Success,
            input.clone(),
        );
        let entries = vec![
            prompt("echo"),
            commit("b", &["call_a"], None),
            tool_started("call_a", "echo", "Echo", input),
            first,
            second,
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        let result_msgs: Vec<_> = out.messages.iter().filter(|m| is_tool_results(m)).collect();
        assert_eq!(result_msgs.len(), 1);
        let parts = tool_result_parts(result_msgs[0]);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].2, "second");
    }

    #[test]
    fn missing_native_meta_on_tool_call_fails_closed() {
        let entries = vec![
            prompt("hi"),
            TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "c1",
                    "title": "Write memory"
                }),
            },
        ];
        let err = migrate_transcript_entries("s1", &entries).unwrap_err();
        match err {
            HydrateError::LoadFailed { message, .. } => {
                assert!(
                    message.contains("codeg_native") || message.contains("function_name"),
                    "{message}"
                );
            }
        }
    }

    #[test]
    fn cancelled_and_unknown_facts_are_honest_tool_results() {
        let mut cancelled = native_tool(
            "call_c",
            "bash",
            ToolPhase::Terminal,
            Some(ToolOutcome::Cancelled),
            json!({}),
        );
        cancelled.model_presentation = Some("ok".into());
        let mut unknown = native_tool(
            "call_u",
            "bash",
            ToolPhase::Terminal,
            Some(ToolOutcome::Unknown),
            json!({}),
        );
        unknown.model_presentation = Some("ok".into());
        let entries = vec![
            prompt("try"),
            commit("b", &["call_c", "call_u"], None),
            TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: tool_call_payload("call_c", "Run", "failed", &json!({}), &cancelled),
            },
            TranscriptEntry {
                t: 3,
                k: EntryKind::Update,
                p: tool_call_update_payload("call_u", "failed", &unknown),
            },
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        assert!(out.complete, "{:?}", out.warnings);
        let results = tool_result_parts(&out.messages[2]);
        assert_eq!(results.len(), 2);
        assert!(
            results[0].2.contains("cancelled"),
            "cancelled body: {}",
            results[0].2
        );
        assert!(
            results[1].2.contains("not replayed")
                || results[1].2.contains("could not be confirmed"),
            "unknown body: {}",
            results[1].2
        );
        assert_ne!(results[0].2, "ok");
        assert_ne!(results[1].2, "ok");
    }

    #[test]
    fn prompt_and_text_only_assistant_round_trips() {
        let entries = vec![
            prompt("hello"),
            text_chunk("visible reply"),
            commit("b", &[], None),
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        assert!(out.complete, "{:?}", out.warnings);
        assert_eq!(out.messages.len(), 2);
        assert_eq!(user_text(&out.messages[0]), Some("hello"));
        assert_eq!(assistant_text(&out.messages[1]), "visible reply");
        assert!(assistant_calls(&out.messages[1]).is_empty());
        assert!(!is_tool_results(&out.messages[1]));
    }

    #[test]
    fn old_compact_record_is_not_applied_as_a_cut() {
        let compact = CompactRecord {
            level: 2,
            through_turn: "s1:1".into(),
            summary: "L2-RESUME-SUMMARY".into(),
            files: Vec::new(),
            created_at_ms: 1,
        };
        let entries = vec![
            prompt("first"),
            text_chunk("one"),
            commit("b1", &[], None),
            TranscriptEntry {
                t: 4,
                k: EntryKind::Update,
                p: compact_update_payload(&compact),
            },
            prompt("second"),
            text_chunk("two"),
            commit("b2", &[], None),
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        let dumped = format!("{:?}", out.messages);
        assert!(dumped.contains("first"), "{dumped}");
        assert!(dumped.contains("second"), "{dumped}");
        assert!(
            !dumped.contains("L2-RESUME-SUMMARY"),
            "old summary must not replace recovered prefix: {dumped}"
        );
        assert!(
            out.warnings.iter().any(|w| w.code == "compact_not_applied"),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn commit_attached_to_tool_card_uses_subsequent_native_meta() {
        let mut commit_only = NativeMeta::v1();
        commit_only.turn_id = Some("s1:1".into());
        commit_only.model_commit = Some(ModelCommit {
            batch_id: "batch-collide".into(),
            parts: vec!["c1".into()],
            call_ids: vec!["c1".into()],
        });
        let stripped = attach_native_meta(
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "c1",
                "title": "Write memory",
                "rawInput": {"path": "a.txt"}
            }),
            &commit_only,
        );
        let input = json!({"path": "a.txt"});
        let entries = vec![
            prompt("write"),
            TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: stripped,
            },
            tool_started("c1", "write_mem", "Write memory", input.clone()),
            tool_terminal("c1", "write_mem", "wrote", ToolOutcome::Success, input),
        ];
        let out = migrate_transcript_entries("s1", &entries).expect("migrate");
        assert!(out.complete, "{:?}", out.warnings);
        let calls = assistant_calls(&out.messages[1]);
        assert_eq!(calls[0].0, "c1");
        assert_eq!(calls[0].1, "write_mem");
    }
}

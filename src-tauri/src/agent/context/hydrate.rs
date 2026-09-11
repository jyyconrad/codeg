//! Resume from model_commit + tool outcomes. Never auto-replays tools.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::store::{AssistantPart, AssistantRecord, ContextStore, ExecutionFact};
use super::transcript::{
    extract_native_meta, session_update_kind, tool_call_id_of, ModelCommit, NativeMetaError,
    ToolOutcome, ToolPhase,
};
use crate::acp_transcript::{
    acquire_write_lease_in, copy_transcript_in, find_grouped_session_in, find_session_in_roots,
    read_chain_in, read_header_in, repair_eof_in, transcript_stat_in, EntryKind, TranscriptHeader,
    TranscriptLease, TranscriptLeaseError, TranscriptRepairError, MAX_CONTINUATION_DEPTH,
    TRANSCRIPT_SCHEMA_VERSION,
};

#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    #[error("{message}")]
    LoadFailed {
        session_id: String,
        message: String,
        code: &'static str,
    },
}

impl HydrateError {
    fn failed(
        session_id: impl Into<String>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::LoadFailed {
            session_id: session_id.into(),
            code,
            message: message.into(),
        }
    }
}

pub struct OpenedSession {
    pub session_id: String,
    pub continues_from: Option<String>,
    pub store: ContextStore,
    pub lease: TranscriptLease,
    pub header: Option<TranscriptHeader>,
    /// Transcript root subsequent writes must use.
    pub write_root: PathBuf,
    /// Group directory under `write_root` (registry id or percent-encoded cwd).
    pub write_group: String,
}

const MAX_ENCODED_CWD_LEN: usize = 255;

/// Percent-encode a working directory as a single sessions subdirectory name.
pub fn encode_session_cwd(cwd: &str) -> String {
    let normalized = normalize_session_cwd(cwd);
    if normalized.is_empty() {
        return "_".to_string();
    }
    let encoded = urlencoding::encode(&normalized).into_owned();
    if encoded.len() <= MAX_ENCODED_CWD_LEN && !encoded.starts_with('.') && !encoded.is_empty() {
        encoded
    } else {
        hashed_session_cwd(&normalized)
    }
}

fn normalize_session_cwd(cwd: &str) -> String {
    let s = cwd.trim().replace('\\', "/");
    if s.is_empty() {
        return String::new();
    }
    if s.chars().all(|c| c == '/') {
        return "/".to_string();
    }
    s.trim_end_matches('/').to_string()
}

fn hashed_session_cwd(normalized: &str) -> String {
    let digest = Sha256::digest(normalized.as_bytes());
    let hash = digest
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let slug = Path::new(normalized)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cwd");
    let slug: String = slug
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    let slug = if slug.is_empty() {
        "cwd".to_string()
    } else {
        slug
    };
    format!("{slug}-{hash}")
}

fn codeg_agent_legacy_dir() -> &'static str {
    crate::acp::registry::registry_id_for(crate::models::agent::AgentType::CodegAgent)
}

/// Open or resume a Codeg Agent transcript under
/// `<sessions_root>/<percent-encoded-cwd>/<id>.jsonl`.
///
/// `fallback_root` is the old `acp-transcripts` tree (`…/codeg-agent/<id>.jsonl`).
/// It is read-only: a hit is copied to the new path (old file kept) so the next
/// write uses the new location.
pub fn open_codeg_agent_session(
    sessions_root: &Path,
    fallback_root: Option<&Path>,
    cwd: &str,
    agent_wire: &str,
    requested: Option<&str>,
) -> Result<OpenedSession, HydrateError> {
    let dest_group = encode_session_cwd(cwd);
    let legacy_dir = codeg_agent_legacy_dir();
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        None => fresh(sessions_root, &dest_group, None),
        Some(session_id) => match transcript_stat_in(sessions_root, &dest_group, session_id) {
            Ok(Some(_)) => load_existing(sessions_root, &dest_group, agent_wire, session_id),
            Err(err) => Err(HydrateError::failed(
                session_id,
                "session_unavailable",
                format!("transcript could not be read: {err}"),
            )),
            Ok(None) => {
                if let Some(group) = find_grouped_session_in(sessions_root, session_id) {
                    return load_existing(sessions_root, &group, agent_wire, session_id);
                }
                let Some(fallback) = fallback_root else {
                    return fresh(sessions_root, &dest_group, Some(session_id.to_string()));
                };
                match transcript_stat_in(fallback, legacy_dir, session_id) {
                    Ok(None) => fresh(sessions_root, &dest_group, Some(session_id.to_string())),
                    Err(err) => Err(HydrateError::failed(
                        session_id,
                        "session_unavailable",
                        format!("transcript could not be read: {err}"),
                    )),
                    Ok(Some(_)) => {
                        let header = read_header_in(fallback, legacy_dir, session_id);
                        let migrate_group = header
                            .as_ref()
                            .map(|h| h.cwd.as_str())
                            .filter(|c| !c.is_empty())
                            .map(encode_session_cwd)
                            .unwrap_or_else(|| dest_group.clone());
                        migrate_fallback_chain(
                            fallback,
                            legacy_dir,
                            sessions_root,
                            &migrate_group,
                            session_id,
                        )
                        .map_err(|err| {
                            HydrateError::failed(
                                session_id,
                                "session_unavailable",
                                format!("failed to migrate transcript: {err}"),
                            )
                        })?;
                        load_existing(sessions_root, &migrate_group, agent_wire, session_id)
                    }
                }
            }
        },
    }
}

fn migrate_fallback_chain(
    src_root: &Path,
    src_dir: &str,
    dst_root: &Path,
    dst_dir: &str,
    session_id: &str,
) -> std::io::Result<()> {
    let mut cursor = Some(session_id.to_string());
    let mut visited = HashSet::new();
    while let Some(id) = cursor.take() {
        if !visited.insert(id.clone()) {
            break;
        }
        if visited.len() > MAX_CONTINUATION_DEPTH {
            break;
        }
        if find_session_in_roots(&[dst_root], &[], &id).is_some() {
            let group =
                find_grouped_session_in(dst_root, &id).unwrap_or_else(|| dst_dir.to_string());
            cursor = read_header_in(dst_root, &group, &id).and_then(|h| h.continues_from);
            continue;
        }
        match transcript_stat_in(src_root, src_dir, &id) {
            Ok(Some(_)) => {
                copy_transcript_in(src_root, src_dir, dst_root, dst_dir, &id)?;
                cursor = read_header_in(dst_root, dst_dir, &id).and_then(|h| h.continues_from);
            }
            Ok(None) => break,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Open or resume a native transcript. Missing files start a new id with
/// `continues_from`; corrupt / busy / unsupported native meta fail closed.
pub fn open_native_session(
    root: &Path,
    agent_dir: &str,
    agent_wire: &str,
    requested: Option<&str>,
) -> Result<OpenedSession, HydrateError> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        None => fresh(root, agent_dir, None),
        Some(session_id) => match transcript_stat_in(root, agent_dir, session_id) {
            Ok(None) => fresh(root, agent_dir, Some(session_id.to_string())),
            Ok(Some(_)) => load_existing(root, agent_dir, agent_wire, session_id),
            Err(err) => Err(HydrateError::failed(
                session_id,
                "session_unavailable",
                format!("transcript could not be read: {err}"),
            )),
        },
    }
}

fn fresh(
    root: &Path,
    agent_dir: &str,
    continues_from: Option<String>,
) -> Result<OpenedSession, HydrateError> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let lease = acquire_write_lease_in(root, agent_dir, &session_id)
        .map_err(|err| lease_error(&session_id, err))?;
    Ok(OpenedSession {
        store: ContextStore::new(&session_id),
        session_id,
        continues_from,
        lease,
        header: None,
        write_root: root.to_path_buf(),
        write_group: agent_dir.to_string(),
    })
}

fn load_existing(
    root: &Path,
    agent_dir: &str,
    agent_wire: &str,
    session_id: &str,
) -> Result<OpenedSession, HydrateError> {
    let lease = acquire_write_lease_in(root, agent_dir, session_id)
        .map_err(|err| lease_error(session_id, err))?;
    repair_eof_in(root, agent_dir, session_id, &lease).map_err(|err| match err {
        TranscriptRepairError::MidFileCorruption => HydrateError::failed(
            session_id,
            "session_unavailable",
            "transcript has mid-file corruption and was not truncated",
        ),
        other => HydrateError::failed(
            session_id,
            "session_unavailable",
            format!("transcript tail repair failed: {other}"),
        ),
    })?;
    let header = read_header_in(root, agent_dir, session_id).ok_or_else(|| {
        HydrateError::failed(
            session_id,
            "session_unavailable",
            "transcript header missing",
        )
    })?;
    if header.v != TRANSCRIPT_SCHEMA_VERSION || header.kind != "header" {
        return Err(HydrateError::failed(
            session_id,
            "session_unavailable",
            "transcript header schema is not supported",
        ));
    }
    if header.session_id != session_id {
        return Err(HydrateError::failed(
            session_id,
            "session_unavailable",
            "transcript session id does not match the file",
        ));
    }
    if header.agent != agent_wire {
        return Err(HydrateError::failed(
            session_id,
            "session_unavailable",
            format!(
                "transcript agent `{}` does not match `{agent_wire}`",
                header.agent
            ),
        ));
    }
    let chain = read_chain_in(root, agent_dir, session_id);
    let store = hydrate_store(session_id, &chain.entries)?;
    Ok(OpenedSession {
        session_id: session_id.to_string(),
        continues_from: None,
        store,
        lease,
        header: Some(header),
        write_root: root.to_path_buf(),
        write_group: agent_dir.to_string(),
    })
}

fn lease_error(session_id: &str, err: TranscriptLeaseError) -> HydrateError {
    match err {
        TranscriptLeaseError::Busy => HydrateError::failed(
            session_id,
            "session_busy",
            "another session still holds the transcript write lease",
        ),
        other => HydrateError::failed(
            session_id,
            "session_unavailable",
            format!("transcript lease failed: {other}"),
        ),
    }
}

pub fn hydrate_store(
    session_id: &str,
    entries: &[crate::acp_transcript::TranscriptEntry],
) -> Result<ContextStore, HydrateError> {
    let mut store = ContextStore::new(session_id);
    let mut prompt_index = 0u64;
    let mut current_turn: Option<String> = None;
    let mut open_assistant: Option<AssistantRecord> = None;
    let mut open_text = String::new();

    let flush_text = |open_assistant: &mut Option<AssistantRecord>, open_text: &mut String| {
        if !open_text.is_empty() {
            open_assistant
                .get_or_insert_with(AssistantRecord::default)
                .parts
                .push(AssistantPart::Text(std::mem::take(open_text)));
        }
    };

    for entry in entries {
        match entry.k {
            EntryKind::Prompt => {
                flush_text(&mut open_assistant, &mut open_text);
                if let (Some(turn_id), Some(asst)) = (current_turn.take(), open_assistant.take()) {
                    store.commit_assistant(&turn_id, asst);
                }
                prompt_index += 1;
                let turn_id = store.turn_id_for_prompt(prompt_index);
                let user_text = prompt_text(&entry.p);
                store.append_user(turn_id.clone(), user_text);
                current_turn = Some(turn_id);
            }
            EntryKind::TurnEnd => {
                flush_text(&mut open_assistant, &mut open_text);
                if let (Some(turn_id), Some(asst)) = (current_turn.as_ref(), open_assistant.take())
                {
                    store.commit_assistant(turn_id, asst);
                }
            }
            EntryKind::Update => {
                match extract_native_meta(&entry.p) {
                    Err(NativeMetaError::UnsupportedVersion(v)) => {
                        return Err(HydrateError::failed(
                            session_id,
                            "session_unavailable",
                            format!("unsupported codeg_native metadata version {v}"),
                        ));
                    }
                    Err(NativeMetaError::Malformed) => {
                        return Err(HydrateError::failed(
                            session_id,
                            "session_unavailable",
                            "malformed codeg_native metadata",
                        ));
                    }
                    Ok(None) => {
                        if matches!(
                            session_update_kind(&entry.p),
                            Some("tool_call" | "tool_call_update")
                        ) {
                            return Err(HydrateError::failed(
                                session_id,
                                "session_unavailable",
                                "tool call is missing required codeg_native execution metadata",
                            ));
                        }
                        // Display-only ACP updates (usage, thought) are ignored.
                        if session_update_kind(&entry.p) == Some("agent_message_chunk") {
                            if let Some(text) = chunk_text(&entry.p) {
                                if !text.is_empty() {
                                    open_text.push_str(&text);
                                }
                            }
                        }
                    }
                    Ok(Some(meta)) => {
                        apply_native_update(
                            &mut store,
                            current_turn.as_deref(),
                            &mut open_assistant,
                            &mut open_text,
                            &entry.p,
                            meta,
                            session_id,
                        )?;
                    }
                }
            }
        }
    }
    flush_text(&mut open_assistant, &mut open_text);
    if let (Some(turn_id), Some(asst)) = (current_turn, open_assistant) {
        store.commit_assistant(&turn_id, asst);
    }
    finalize_open_facts(&mut store);
    Ok(store)
}

fn apply_native_update(
    store: &mut ContextStore,
    current_turn: Option<&str>,
    open_assistant: &mut Option<AssistantRecord>,
    open_text: &mut String,
    payload: &Value,
    meta: super::transcript::NativeMeta,
    session_id: &str,
) -> Result<(), HydrateError> {
    let kind = session_update_kind(payload).unwrap_or("");
    if let Some(text) = chunk_text(payload) {
        if !text.is_empty() {
            open_text.push_str(&text);
        }
    }
    if matches!(kind, "tool_call" | "tool_call_update") {
        if !open_text.is_empty() {
            open_assistant
                .get_or_insert_with(AssistantRecord::default)
                .parts
                .push(AssistantPart::Text(std::mem::take(open_text)));
        }
        let tool_call_id = meta
            .tool_call_id
            .clone()
            .or_else(|| tool_call_id_of(payload).map(str::to_string))
            .ok_or_else(|| {
                HydrateError::failed(session_id, "session_unavailable", "tool call missing id")
            })?;
        let function_name = meta.function_name.clone().ok_or_else(|| {
            HydrateError::failed(
                session_id,
                "session_unavailable",
                "tool call missing function_name; refusing to infer from title",
            )
        })?;
        let turn_id = meta
            .turn_id
            .clone()
            .or_else(|| current_turn.map(str::to_string))
            .ok_or_else(|| {
                HydrateError::failed(
                    session_id,
                    "session_unavailable",
                    "tool call is missing a model message boundary",
                )
            })?;
        let raw_input = meta
            .raw_input
            .clone()
            .or_else(|| payload.get("rawInput").cloned())
            .unwrap_or(Value::Null);
        if kind == "tool_call" {
            let asst = open_assistant.get_or_insert_with(AssistantRecord::default);
            if meta.model_message_id.is_some() {
                asst.model_message_id = meta.model_message_id.clone();
            }
            if !asst
                .parts
                .iter()
                .any(|p| matches!(p, AssistantPart::ToolCall { id, .. } if *id == tool_call_id))
            {
                asst.parts.push(AssistantPart::ToolCall {
                    id: tool_call_id.clone(),
                    name: function_name.clone(),
                    args: raw_input.clone(),
                });
            }
        }
        let mut fact = store.fact(&tool_call_id).cloned().unwrap_or_else(|| {
            ExecutionFact::pending(&turn_id, &tool_call_id, &function_name, raw_input)
        });
        fact.function_name = function_name;
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
        store.record_fact(fact);
    }
    if let Some(commit) = meta.model_commit {
        apply_commit(open_assistant, open_text, &commit, meta.model_message_id);
    }
    if let Some(compact) = meta.compact {
        store.set_compact(compact);
    }
    Ok(())
}

fn apply_commit(
    open_assistant: &mut Option<AssistantRecord>,
    open_text: &mut String,
    commit: &ModelCommit,
    model_message_id: Option<String>,
) {
    if !open_text.is_empty() {
        open_assistant
            .get_or_insert_with(AssistantRecord::default)
            .parts
            .push(AssistantPart::Text(std::mem::take(open_text)));
    }
    let asst = open_assistant.get_or_insert_with(AssistantRecord::default);
    asst.committed = true;
    if asst.model_message_id.is_none() {
        asst.model_message_id = model_message_id.or_else(|| Some(commit.batch_id.clone()));
    }
    let _ = commit;
}

fn finalize_open_facts(store: &mut ContextStore) {
    let ids: Vec<String> = store.facts().map(|f| f.tool_call_id.clone()).collect();
    for id in ids {
        store.upsert_fact(&id, |fact| match (fact.phase, fact.outcome) {
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
        });
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
    use crate::acp_transcript::{
        append_line_in, transcript_path_in, TranscriptEntry, TranscriptHeader,
    };
    use crate::agent::context::transcript::{
        agent_message_chunk, attach_native_meta, compact_update_payload, tool_call_payload,
        tool_call_update_payload, CompactRecord, NativeMeta,
    };
    use crate::models::agent::AgentType;
    use crate::parsers::acp_native::AcpNativeParser;
    use crate::parsers::AgentParser;
    use serde_json::json;

    fn temp_root() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "codeg-native-hydrate-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_line(root: &std::path::Path, session: &str, line: &str) {
        append_line_in(root, "codeg-agent", session, line);
    }

    fn header_line(session: &str) -> String {
        serde_json::to_string(&TranscriptHeader::new("codeg_agent", session, "/tmp", 1)).unwrap()
    }

    fn entry(kind: EntryKind, payload: Value) -> String {
        serde_json::to_string(&TranscriptEntry {
            t: 2,
            k: kind,
            p: payload,
        })
        .unwrap()
    }

    fn native_tool(
        id: &str,
        name: &str,
        phase: ToolPhase,
        outcome: Option<ToolOutcome>,
    ) -> NativeMeta {
        let mut meta = NativeMeta::v1();
        meta.turn_id = Some("s1:1".into());
        meta.tool_call_id = Some(id.into());
        meta.function_name = Some(name.into());
        meta.raw_input = Some(json!({"text": id}));
        meta.phase = Some(phase);
        meta.outcome = outcome;
        meta.executed = match outcome {
            Some(ToolOutcome::Success) => Some(true),
            Some(ToolOutcome::Cancelled | ToolOutcome::Rejected) => Some(false),
            _ => None,
        };
        meta.model_message_id = Some("msg-1".into());
        meta
    }

    #[test]
    fn missing_file_starts_new_id_with_continuation() {
        let root = temp_root();
        let opened = open_native_session(&root, "codeg-agent", "codeg_agent", Some("old-sess"))
            .expect("fresh");
        assert_ne!(opened.session_id, "old-sess");
        assert_eq!(opened.continues_from.as_deref(), Some("old-sess"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn groups_two_calls_of_one_model_message_and_keeps_function_names() {
        let mut store = hydrate_fixture_two_calls(ToolOutcome::Success, None);
        assert_eq!(store.turns().len(), 1);
        let asst = store.turns()[0].assistant.as_ref().unwrap();
        assert!(asst.committed);
        assert_eq!(asst.model_message_id.as_deref(), Some("msg-1"));
        let names: Vec<_> = asst
            .parts
            .iter()
            .filter_map(|p| match p {
                AssistantPart::ToolCall { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["write_mem", "write_mem"]);
        assert_eq!(
            store.fact("call_a").unwrap().outcome,
            Some(ToolOutcome::Success)
        );
        store.settle_cancel("s1:1");
        assert_eq!(
            store.fact("call_a").unwrap().outcome,
            Some(ToolOutcome::Success)
        );
        assert_eq!(
            store.fact("call_b").unwrap().outcome,
            Some(ToolOutcome::Cancelled)
        );
        assert!(store.auto_replay_ids().is_empty());
    }

    fn hydrate_fixture_two_calls(
        a_outcome: ToolOutcome,
        b_outcome: Option<ToolOutcome>,
    ) -> ContextStore {
        let a_start = native_tool("call_a", "write_mem", ToolPhase::Started, None);
        let mut a_term = native_tool("call_a", "write_mem", ToolPhase::Terminal, Some(a_outcome));
        a_term.model_presentation = Some("wrote A".into());
        let b_pending = native_tool("call_b", "write_mem", ToolPhase::Pending, b_outcome);
        let mut commit = NativeMeta::v1();
        commit.turn_id = Some("s1:1".into());
        commit.model_message_id = Some("msg-1".into());
        commit.model_commit = Some(ModelCommit {
            batch_id: "batch-1".into(),
            parts: vec!["t0".into(), "cA".into(), "cB".into()],
            call_ids: vec!["call_a".into(), "call_b".into()],
        });
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"do A then B"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: agent_message_chunk("working", &{
                    let mut m = NativeMeta::v1();
                    m.turn_id = Some("s1:1".into());
                    m.model_message_id = Some("msg-1".into());
                    m.part_index = Some(0);
                    m
                }),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 3,
                k: EntryKind::Update,
                p: tool_call_payload(
                    "call_a",
                    "Write memory",
                    "pending",
                    &json!({"text":"A"}),
                    &a_start,
                ),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 4,
                k: EntryKind::Update,
                p: tool_call_update_payload("call_a", "completed", &a_term),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 5,
                k: EntryKind::Update,
                p: tool_call_payload(
                    "call_b",
                    "Write memory",
                    "pending",
                    &json!({"text":"B"}),
                    &b_pending,
                ),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 6,
                k: EntryKind::Update,
                p: agent_message_chunk("", &commit),
            },
        ];
        hydrate_store("s1", &entries).expect("hydrate")
    }

    #[test]
    fn started_without_terminal_hydrates_unknown_and_is_not_replayed() {
        let mut start = native_tool("call_u", "write_mem", ToolPhase::Started, None);
        start.model_commit = Some(ModelCommit {
            batch_id: "b".into(),
            parts: vec!["cU".into()],
            call_ids: vec!["call_u".into()],
        });
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"write"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: tool_call_payload("call_u", "Write", "in_progress", &json!({}), &start),
            },
        ];
        let store = hydrate_store("s1", &entries).expect("hydrate");
        assert_eq!(
            store.fact("call_u").unwrap().outcome,
            Some(ToolOutcome::Unknown)
        );
        assert!(store.auto_replay_ids().is_empty());
    }

    #[test]
    fn unknown_native_version_fails_closed() {
        let payload = attach_native_meta(
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hi" }
            }),
            &NativeMeta {
                v: 2,
                turn_id: Some("s:1".into()),
                ..NativeMeta::default()
            },
        );
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"hi"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: payload,
            },
        ];
        let err = hydrate_store("s1", &entries).unwrap_err();
        match err {
            HydrateError::LoadFailed { message, .. } => {
                assert!(message.contains("unsupported"), "{message}");
            }
        }
    }

    #[test]
    fn tool_call_without_native_meta_fails() {
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"hi"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "c1",
                    "title": "Write memory"
                }),
            },
        ];
        assert!(hydrate_store("s1", &entries).is_err());
    }

    #[test]
    fn rejected_and_error_outcomes_stay_failed_not_success() {
        let mut rejected = native_tool(
            "call_r",
            "write_mem",
            ToolPhase::Terminal,
            Some(ToolOutcome::Rejected),
        );
        rejected.executed = Some(false);
        let mut errored = native_tool(
            "call_e",
            "write_mem",
            ToolPhase::Terminal,
            Some(ToolOutcome::Error),
        );
        errored.reason = Some("file does not exist".into());
        errored.model_presentation = Some("file does not exist".into());
        let mut commit = NativeMeta::v1();
        commit.model_commit = Some(ModelCommit {
            batch_id: "b".into(),
            parts: vec![],
            call_ids: vec!["call_r".into(), "call_e".into()],
        });
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"try"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: tool_call_payload("call_r", "Write", "failed", &json!({}), &rejected),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 3,
                k: EntryKind::Update,
                p: tool_call_payload("call_e", "Write", "failed", &json!({}), &errored),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 4,
                k: EntryKind::Update,
                p: agent_message_chunk("", &commit),
            },
        ];
        let store = hydrate_store("s1", &entries).expect("hydrate");
        assert_eq!(
            store.fact("call_r").unwrap().outcome,
            Some(ToolOutcome::Rejected)
        );
        assert_eq!(
            store.fact("call_e").unwrap().outcome,
            Some(ToolOutcome::Error)
        );
    }

    #[test]
    fn display_parser_ignores_empty_commit_marker() {
        let root = temp_root();
        let session = "disp-1";
        write_line(&root, session, &header_line(session));
        write_line(
            &root,
            session,
            &entry(EntryKind::Prompt, json!([{"type":"text","text":"hello"}])),
        );
        let mut commit = NativeMeta::v1();
        commit.model_commit = Some(ModelCommit {
            batch_id: "b".into(),
            parts: vec![],
            call_ids: vec![],
        });
        write_line(
            &root,
            session,
            &entry(
                EntryKind::Update,
                agent_message_chunk("visible", &{
                    let mut m = NativeMeta::v1();
                    m.part_index = Some(0);
                    m
                }),
            ),
        );
        write_line(
            &root,
            session,
            &entry(EntryKind::Update, agent_message_chunk("", &commit)),
        );
        let parser = AcpNativeParser::new_in(AgentType::CodegAgent, root.clone());
        let detail = parser.get_conversation(session).expect("parsed");
        let dumped = format!("{:?}", detail.turns);
        assert!(dumped.contains("visible"), "{dumped}");
        let empty_text = detail.turns.iter().any(|turn| {
            turn.blocks.iter().any(|block| matches!(block, crate::models::message::ContentBlock::Text { text } if text.is_empty()))
        });
        assert!(
            !empty_text,
            "empty commit marker must not become a display block: {dumped}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn text_and_tools_interleave_in_one_assistant_message() {
        let mut text_meta = NativeMeta::v1();
        text_meta.model_message_id = Some("msg".into());
        text_meta.part_index = Some(0);
        let call = native_tool(
            "call_a",
            "echo",
            ToolPhase::Terminal,
            Some(ToolOutcome::Success),
        );
        let mut after = NativeMeta::v1();
        after.model_message_id = Some("msg".into());
        after.part_index = Some(2);
        after.model_commit = Some(ModelCommit {
            batch_id: "b".into(),
            parts: vec!["t0".into(), "cA".into(), "t2".into()],
            call_ids: vec!["call_a".into()],
        });
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"mix"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: agent_message_chunk("before ", &text_meta),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 3,
                k: EntryKind::Update,
                p: tool_call_payload("call_a", "echo", "completed", &json!({}), &call),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 4,
                k: EntryKind::Update,
                p: agent_message_chunk("after", &after),
            },
        ];
        let store = hydrate_store("s1", &entries).expect("hydrate");
        let parts = &store.turns()[0].assistant.as_ref().unwrap().parts;
        assert!(matches!(&parts[0], AssistantPart::Text(t) if t == "before "));
        assert!(matches!(&parts[1], AssistantPart::ToolCall { id, .. } if id == "call_a"));
        assert!(matches!(&parts[2], AssistantPart::Text(t) if t == "after"));
    }

    #[test]
    fn hydrate_reuses_l2_compact_record_and_keeps_original_turns() {
        let compact = CompactRecord {
            level: 2,
            through_turn: "s1:1".into(),
            summary: "L2-RESUME-SUMMARY".into(),
            created_at_ms: 1,
        };
        let entries = vec![
            crate::acp_transcript::TranscriptEntry {
                t: 1,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"first"}]),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 2,
                k: EntryKind::Update,
                p: compact_update_payload(&compact),
            },
            crate::acp_transcript::TranscriptEntry {
                t: 3,
                k: EntryKind::Prompt,
                p: json!([{"type":"text","text":"second"}]),
            },
        ];
        let store = hydrate_store("s1", &entries).expect("hydrate");
        assert_eq!(store.turns().len(), 2, "original turns stay");
        let rec = store.compact().expect("compact");
        assert_eq!(rec.level, 2);
        assert_eq!(rec.summary, "L2-RESUME-SUMMARY");
        assert_eq!(rec.through_turn, "s1:1");
    }

    #[test]
    fn corrupt_existing_file_does_not_invent_a_new_uuid() {
        let root = temp_root();
        let session = "bad-id";
        let path = transcript_path_in(&root, "codeg-agent", session).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not a header\n").unwrap();
        let err = match open_native_session(&root, "codeg-agent", "codeg_agent", Some(session)) {
            Err(err) => err,
            Ok(_) => panic!("corrupt file must not hydrate"),
        };
        match err {
            HydrateError::LoadFailed { session_id, .. } => {
                assert_eq!(session_id, session);
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn encode_session_cwd_percent_encodes_paths() {
        assert_eq!(encode_session_cwd("/Users/me/proj"), "%2FUsers%2Fme%2Fproj");
        assert_eq!(encode_session_cwd("/tmp/"), "%2Ftmp");
        assert_eq!(encode_session_cwd(""), "_");
        assert_eq!(encode_session_cwd(r"C:\Users\me"), "C%3A%2FUsers%2Fme");
        let long = format!("/{}", "a".repeat(300));
        let hashed = encode_session_cwd(&long);
        assert!(hashed.len() <= 255, "{hashed}");
        assert!(hashed.contains('-'), "{hashed}");
        assert!(!hashed.contains('%'), "{hashed}");
    }

    #[test]
    fn open_codeg_agent_session_uses_encoded_cwd_group() {
        let root = temp_root();
        let opened =
            open_codeg_agent_session(&root, None, "/tmp/demo", "codeg_agent", None).expect("fresh");
        assert_eq!(opened.write_root, root);
        assert_eq!(opened.write_group, encode_session_cwd("/tmp/demo"));
        assert!(opened.header.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_codeg_agent_session_hydrates_new_path() {
        let root = temp_root();
        let session = "sess-new-path";
        let group = encode_session_cwd("/tmp/demo");
        append_line_in(&root, &group, session, &header_line(session));
        append_line_in(
            &root,
            &group,
            session,
            &entry(EntryKind::Prompt, json!([{"type":"text","text":"hello"}])),
        );
        let opened =
            open_codeg_agent_session(&root, None, "/tmp/demo", "codeg_agent", Some(session))
                .expect("hydrate");
        assert_eq!(opened.session_id, session);
        assert_eq!(opened.write_group, group);
        assert_eq!(opened.store.turns().len(), 1);
        assert_eq!(opened.store.turns()[0].user_text, "hello");
        let parser = AcpNativeParser::new_in(AgentType::CodegAgent, root.clone());
        let detail = parser.get_conversation(session).expect("parsed");
        assert_eq!(detail.summary.title.as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_codeg_agent_session_migrates_legacy_once_and_keeps_old_file() {
        let sessions = temp_root();
        let fallback = temp_root();
        let session = "legacy-sess";
        append_line_in(&fallback, "codeg-agent", session, &header_line(session));
        append_line_in(
            &fallback,
            "codeg-agent",
            session,
            &entry(
                EntryKind::Prompt,
                json!([{"type":"text","text":"from legacy"}]),
            ),
        );
        let old_path = transcript_path_in(&fallback, "codeg-agent", session).unwrap();
        let old_bytes = std::fs::read(&old_path).unwrap();

        let opened = open_codeg_agent_session(
            &sessions,
            Some(&fallback),
            "/tmp/demo",
            "codeg_agent",
            Some(session),
        )
        .expect("migrate");
        assert_eq!(opened.session_id, session);
        assert_eq!(opened.write_group, encode_session_cwd("/tmp"));
        assert_eq!(opened.store.turns()[0].user_text, "from legacy");

        let new_path = transcript_path_in(&sessions, &opened.write_group, session).unwrap();
        assert!(new_path.exists(), "migrated file must exist at new path");
        assert_eq!(
            std::fs::read(&old_path).unwrap(),
            old_bytes,
            "legacy file must not be deleted or rewritten"
        );

        let extra = entry(
            EntryKind::Prompt,
            json!([{"type":"text","text":"after migrate"}]),
        );
        append_line_in(&sessions, &opened.write_group, session, &extra);
        assert_eq!(
            std::fs::read(&old_path).unwrap(),
            old_bytes,
            "writes after migrate must not touch the legacy file"
        );
        let new_text = std::fs::read_to_string(&new_path).unwrap();
        assert!(new_text.contains("after migrate"), "{new_text}");

        let parser = AcpNativeParser::new_in_with_fallback(
            AgentType::CodegAgent,
            sessions.clone(),
            fallback.clone(),
        );
        let detail = parser.get_conversation(session).expect("parsed");
        assert_eq!(detail.summary.message_count, 2);

        let _ = std::fs::remove_dir_all(&sessions);
        let _ = std::fs::remove_dir_all(&fallback);
    }
}

//! `_meta.codeg_native` encode / decode. Host-owned; never overwritten by MCP `_meta`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const NATIVE_META_VERSION: u32 = 1;
pub const NATIVE_META_NS: &str = "codeg_native";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Pending,
    Started,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Success,
    Error,
    Timeout,
    Rejected,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutputLocator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCommit {
    pub batch_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub call_ids: Vec<String>,
}

/// Derived two-level compression artifact. JSONL original turns are never deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CompactRecord {
    pub level: u8,
    pub through_turn: String,
    pub summary: String,
    #[serde(default)]
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NativeMeta {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ToolPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ToolOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_presentation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_locator: Option<OutputLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_commit: Option<ModelCommit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_remote_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<CompactRecord>,
}

impl NativeMeta {
    pub fn v1() -> Self {
        Self {
            v: NATIVE_META_VERSION,
            ..Self::default()
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NativeMetaError {
    #[error("unsupported codeg_native metadata version {0}")]
    UnsupportedVersion(u32),
    #[error("malformed codeg_native metadata")]
    Malformed,
}

/// Read `_meta.codeg_native` from an ACP SessionUpdate payload.
pub fn extract_native_meta(payload: &Value) -> Result<Option<NativeMeta>, NativeMetaError> {
    let Some(meta) = payload.get("_meta") else {
        return Ok(None);
    };
    let Some(native) = meta.get(NATIVE_META_NS) else {
        return Ok(None);
    };
    let parsed: NativeMeta =
        serde_json::from_value(native.clone()).map_err(|_| NativeMetaError::Malformed)?;
    if parsed.v != NATIVE_META_VERSION {
        return Err(NativeMetaError::UnsupportedVersion(parsed.v));
    }
    Ok(Some(parsed))
}

/// Embed `codeg_native` under `_meta` without dropping sibling keys (MCP `_meta`).
pub fn attach_native_meta(mut payload: Value, native: &NativeMeta) -> Value {
    let native_val = serde_json::to_value(native).unwrap_or(Value::Null);
    let obj = payload.as_object_mut();
    let Some(obj) = obj else {
        return payload;
    };
    match obj.get_mut("_meta") {
        Some(Value::Object(existing)) => {
            existing.insert(NATIVE_META_NS.to_string(), native_val);
        }
        _ => {
            let mut map = Map::new();
            map.insert(NATIVE_META_NS.to_string(), native_val);
            obj.insert("_meta".into(), Value::Object(map));
        }
    }
    payload
}

pub fn compact_update_payload(record: &CompactRecord) -> Value {
    let mut native = NativeMeta::v1();
    native.compact = Some(record.clone());
    agent_message_chunk("", &native)
}

pub fn agent_message_chunk(text: &str, native: &NativeMeta) -> Value {
    attach_native_meta(
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }),
        native,
    )
}

pub fn tool_call_payload(
    tool_call_id: &str,
    title: &str,
    status: &str,
    raw_input: &Value,
    native: &NativeMeta,
) -> Value {
    attach_native_meta(
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": tool_call_id,
            "title": title,
            "kind": "other",
            "status": status,
            "rawInput": raw_input,
        }),
        native,
    )
}

pub fn tool_call_update_payload(tool_call_id: &str, status: &str, native: &NativeMeta) -> Value {
    attach_native_meta(
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": tool_call_id,
            "status": status,
        }),
        native,
    )
}

pub fn acp_status_for(outcome: Option<ToolOutcome>, phase: ToolPhase) -> &'static str {
    match (phase, outcome) {
        (ToolPhase::Pending, _) => "pending",
        (ToolPhase::Started, _) => "in_progress",
        (ToolPhase::Terminal, Some(ToolOutcome::Success)) => "completed",
        (ToolPhase::Terminal, _) => "failed",
    }
}

pub fn session_update_kind(payload: &Value) -> Option<&str> {
    payload.get("sessionUpdate").and_then(Value::as_str)
}

pub fn tool_call_id_of(payload: &Value) -> Option<&str> {
    payload
        .get("toolCallId")
        .or_else(|| payload.get("tool_call_id"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_namespace_does_not_clobber_sibling_meta() {
        let payload = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "c1",
            "_meta": { "mcp": { "keep": true } }
        });
        let mut native = NativeMeta::v1();
        native.tool_call_id = Some("c1".into());
        native.function_name = Some("echo".into());
        let out = attach_native_meta(payload, &native);
        assert_eq!(out["_meta"]["mcp"]["keep"], json!(true));
        let parsed = extract_native_meta(&out).unwrap().unwrap();
        assert_eq!(parsed.function_name.as_deref(), Some("echo"));
    }

    #[test]
    fn unknown_native_version_is_an_error() {
        let payload = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "" },
            "_meta": { "codeg_native": { "v": 2, "turn_id": "t1" } }
        });
        assert!(matches!(
            extract_native_meta(&payload),
            Err(NativeMetaError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn compact_meta_round_trips_without_clobbering_turns() {
        let record = CompactRecord {
            level: 2,
            through_turn: "s:3".into(),
            summary: "folded".into(),
            created_at_ms: 9,
        };
        let payload = compact_update_payload(&record);
        assert_eq!(payload["content"]["text"], json!(""));
        let parsed = extract_native_meta(&payload).unwrap().unwrap();
        assert_eq!(parsed.compact.as_ref().unwrap().level, 2);
        assert_eq!(parsed.compact.unwrap().summary, "folded");
    }

    #[test]
    fn empty_commit_marker_round_trips() {
        let mut native = NativeMeta::v1();
        native.turn_id = Some("s:1".into());
        native.model_commit = Some(ModelCommit {
            batch_id: "b1".into(),
            parts: vec!["p0".into()],
            call_ids: vec!["c1".into()],
        });
        let payload = agent_message_chunk("", &native);
        assert_eq!(payload["content"]["text"], json!(""));
        let parsed = extract_native_meta(&payload).unwrap().unwrap();
        assert_eq!(
            parsed.model_commit.unwrap().call_ids,
            vec!["c1".to_string()]
        );
    }
}

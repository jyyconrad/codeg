//! One-shot LLM summarizer implementing rig-memory [`Compactor`].
//!
//! Windowing lives in [`super::policy`] + Rig `TokenWindowMemory`. This module
//! does not project request history from execution facts.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use rig::agent::{
    AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, InvalidToolCallAction,
    InvalidToolCallContext, MultiTurnStreamItem, ObservationAction, TextDelta, ToolCall,
    ToolCallAction,
};
use rig::client::AgentClientExt;
use rig::completion::Message;
use rig_memory::{Compactor, MemoryError};
use tokio_util::sync::CancellationToken;

use crate::agent::mode::compact_context_dir;
use crate::agent::model::{CodegLlmClient, DEFAULT_INVALID_TOOL_CALL_RETRIES};

/// Cap on compact `max_tokens` (min of this and the session setting).
pub const L2_MAX_TOKENS: u64 = 2048;
/// Hard cap on inner dedicated compact completions when eviction is chunked.
pub const COMPACT_MAX_TURNS: usize = 8;
/// Serialized-message budget per compact completion. Whole messages only.
const COMPACT_CHUNK_MAX_BYTES: usize = 64 * 1024;

/// Artifact produced by [`LlmCompactor`].
#[derive(Clone, Debug)]
pub struct CompactFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct CompactArtifact {
    pub summary: String,
    pub files: Vec<CompactFile>,
}

impl From<CompactArtifact> for Message {
    fn from(value: CompactArtifact) -> Self {
        Message::user(value.summary)
    }
}

/// Dedicated no-tool compact completion. Same session client/protocol as the main turn.
#[derive(Clone)]
pub struct LlmCompactor {
    client: CodegLlmClient,
    model_id: String,
    compact_prompt: String,
    max_tokens: u64,
    #[allow(dead_code)]
    artifacts_dir: Option<PathBuf>,
    cancel: CancellationToken,
}

impl LlmCompactor {
    pub fn new(
        client: CodegLlmClient,
        model_id: impl Into<String>,
        compact_prompt: impl Into<String>,
        max_tokens: u64,
    ) -> Self {
        Self {
            client,
            model_id: model_id.into(),
            compact_prompt: compact_prompt.into(),
            max_tokens: max_tokens.clamp(1, L2_MAX_TOKENS),
            artifacts_dir: None,
            cancel: CancellationToken::new(),
        }
    }

    pub fn with_artifacts(mut self, artifacts_dir: impl Into<PathBuf>) -> Self {
        self.artifacts_dir = Some(artifacts_dir.into());
        self
    }

    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn max_tokens(&self) -> u64 {
        self.max_tokens
    }

    pub fn compact_prompt(&self) -> &str {
        &self.compact_prompt
    }

    async fn summarize(
        &self,
        conversation_id: &str,
        evicted: &[Message],
        carry_over: Option<&str>,
    ) -> Result<CompactArtifact, String> {
        if self.cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let chunks = chunk_evicted_messages(evicted);
        if chunks.len() > COMPACT_MAX_TURNS {
            return Err(format!(
                "compact needs {} completions; cap is {COMPACT_MAX_TURNS}",
                chunks.len()
            ));
        }
        let mut carry = carry_over
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mut summary = String::new();
        for chunk in &chunks {
            if self.cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            summary = self
                .summarize_chunk(conversation_id, chunk, carry.as_deref())
                .await?;
            carry = Some(summary.clone());
        }
        Ok(CompactArtifact {
            summary: annotate_history_summary(summary),
            files: Vec::new(),
        })
    }

    async fn summarize_chunk(
        &self,
        conversation_id: &str,
        evicted: &[Message],
        carry_over: Option<&str>,
    ) -> Result<String, String> {
        let _ = conversation_id;
        let prompt = Message::user(compact_prompt_body(evicted, carry_over));
        tracing::info!("codeg agent compacting context with dedicated completion");
        let summary = match &self.client {
            CodegLlmClient::Completions(client) => {
                run_compact_agent(
                    client.clone(),
                    &self.model_id,
                    &self.compact_prompt,
                    prompt,
                    self.max_tokens,
                    self.cancel.clone(),
                )
                .await?
            }
            CodegLlmClient::Responses(client) => {
                run_compact_agent(
                    client.clone(),
                    &self.model_id,
                    &self.compact_prompt,
                    prompt,
                    self.max_tokens,
                    self.cancel.clone(),
                )
                .await?
            }
        };
        if summary.trim().is_empty() {
            return Err("empty compact summary".into());
        }
        Ok(summary)
    }

    #[allow(dead_code)]
    fn prepare_context_dir(&self) -> Result<Option<PathBuf>, String> {
        let Some(artifacts) = &self.artifacts_dir else {
            return Ok(None);
        };
        let dir = compact_context_dir(artifacts);
        std::fs::create_dir_all(&dir).map_err(|e| format!("create compact context dir: {e}"))?;
        Ok(Some(dir))
    }
}

impl Compactor for LlmCompactor {
    type Artifact = CompactArtifact;

    fn compact<'a>(
        &'a self,
        conversation_id: &'a str,
        evicted: &'a [Message],
        carry_over: Option<&'a Self::Artifact>,
    ) -> rig::wasm_compat::WasmBoxedFuture<'a, Result<Self::Artifact, MemoryError>> {
        Box::pin(async move {
            self.summarize(
                conversation_id,
                evicted,
                carry_over.map(|a| a.summary.as_str()),
            )
            .await
            .map_err(MemoryError::Internal)
        })
    }
}

struct CompactHook {
    cancel: CancellationToken,
    text: Arc<Mutex<String>>,
}

impl AgentHook for CompactHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        if self.cancel.is_cancelled() {
            return CompletionCallAction::stop("cancelled");
        }
        CompletionCallAction::continue_run()
    }

    async fn on_invalid_tool_call(
        &self,
        _ctx: &HookContext,
        event: &InvalidToolCallContext,
    ) -> Option<InvalidToolCallAction> {
        if self.cancel.is_cancelled() {
            return Some(InvalidToolCallAction::stop("cancelled"));
        }
        Some(InvalidToolCallAction::retry(format!(
            "unknown or disallowed tool `{}`; compact is a tool-free completion",
            event.tool_name
        )))
    }

    async fn on_tool_call(&self, _ctx: &HookContext, _event: ToolCall<'_>) -> ToolCallAction {
        if self.cancel.is_cancelled() {
            return ToolCallAction::stop("cancelled");
        }
        ToolCallAction::skip("compact is a tool-free completion")
    }

    async fn on_text_delta(&self, _ctx: &HookContext, event: TextDelta<'_>) -> ObservationAction {
        *self.text.lock().expect("compact text") = event.aggregated.to_string();
        ObservationAction::continue_run()
    }
}

async fn run_compact_agent<C>(
    client: C,
    model_id: &str,
    preamble: &str,
    prompt: Message,
    max_tokens: u64,
    cancel: CancellationToken,
) -> Result<String, String>
where
    C: AgentClientExt + Send,
    C::CompletionModel: 'static,
{
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    let text = Arc::new(Mutex::new(String::new()));
    let hook = CompactHook {
        cancel: cancel.clone(),
        text: Arc::clone(&text),
    };
    let stream = client
        .agent(model_id)
        .preamble(preamble)
        .max_tokens(max_tokens)
        .default_max_turns(1)
        .build()
        .runner(prompt)
        .history(Vec::<Message>::new())
        .max_turns(1)
        .tool_concurrency(1)
        .max_invalid_tool_call_retries(DEFAULT_INVALID_TOOL_CALL_RETRIES)
        .add_hook(hook)
        .stream()
        .await;
    drain_compact_stream(stream, cancel, text).await
}

async fn drain_compact_stream(
    mut stream: rig::agent::StreamingResult,
    cancel: CancellationToken,
    text: Arc<Mutex<String>>,
) -> Result<String, String> {
    let mut output = String::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err("cancelled".into()),
            item = stream.next() => match item {
                None => break,
                Some(Ok(MultiTurnStreamItem::FinalResponse(response))) => {
                    output = response.output;
                    break;
                }
                Some(Err(err)) => {
                    let message = err.to_string();
                    if cancel.is_cancelled() || message.to_ascii_lowercase().contains("cancel") {
                        return Err("cancelled".into());
                    }
                    return Err(message);
                }
                Some(Ok(_)) => {}
            }
        }
    }
    if output.trim().is_empty() {
        output = text.lock().expect("compact text").clone();
    }
    Ok(output)
}

#[allow(dead_code)]
fn collect_markdown_files(root: &Path) -> Result<Vec<CompactFile>, String> {
    let mut files = Vec::new();
    collect_markdown_files_inner(root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[allow(dead_code)]
fn collect_markdown_files_inner(dir: &Path, out: &mut Vec<CompactFile>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let path = entry
            .map_err(|e| format!("read compact context dir: {e}"))?
            .path();
        if path.is_dir() {
            collect_markdown_files_inner(&path, out)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        out.push(CompactFile {
            path: path.to_string_lossy().to_string(),
            content,
        });
    }
    Ok(())
}

fn ensure_summary_lists_files(summary: &mut String, files: &[CompactFile]) {
    let missing: Vec<&str> = files
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| !summary.contains(path))
        .collect();
    if missing.is_empty() {
        return;
    }
    if !summary.is_empty() && !summary.ends_with('\n') {
        summary.push('\n');
    }
    summary.push_str("\nContext files:\n");
    for path in missing {
        summary.push_str("- ");
        summary.push_str(path);
        summary.push('\n');
    }
}

fn messages_as_text(messages: &[Message]) -> String {
    serde_json::to_string(messages).unwrap_or_else(|_| format!("{messages:?}"))
}

fn compact_prompt_body(evicted: &[Message], carry_over: Option<&str>) -> String {
    let mut body = String::new();
    if let Some(prev) = carry_over.map(str::trim).filter(|s| !s.is_empty()) {
        body.push_str("Previous summary:\n");
        body.push_str(prev);
        body.push_str("\n\n");
    }
    body.push_str("Evicted turns:\n");
    body.push_str(&messages_as_text(evicted));
    body
}

fn annotate_history_summary(summary: String) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return summary;
    }
    let already_marked = trimmed.starts_with("Conversation summary")
        || trimmed.starts_with("历史摘要")
        || trimmed.to_ascii_lowercase().starts_with("history summary");
    if already_marked {
        summary
    } else {
        format!("Conversation summary:\n{summary}")
    }
}

fn message_json_len(message: &Message) -> usize {
    serde_json::to_vec(message)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn chunk_evicted_messages(messages: &[Message]) -> Vec<&[Message]> {
    if messages.is_empty() {
        return vec![messages];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut bytes: usize = 0;
    for (index, message) in messages.iter().enumerate() {
        let size = message_json_len(message);
        if index > start && bytes.saturating_add(size) > COMPACT_CHUNK_MAX_BYTES {
            chunks.push(&messages[start..index]);
            start = index;
            bytes = 0;
        }
        bytes = bytes.saturating_add(size);
    }
    if start < messages.len() {
        chunks.push(&messages[start..]);
    }
    chunks
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
    use rig::completion::Message;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[test]
    fn compact_context_dir_nests_under_session_artifacts() {
        let artifacts = PathBuf::from("/tmp/artifacts/sess");
        assert_eq!(
            compact_context_dir(&artifacts),
            PathBuf::from("/tmp/artifacts/sess/context")
        );
    }

    #[test]
    fn summary_appends_missing_session_file_paths() {
        let mut summary = "goal and next step".to_string();
        ensure_summary_lists_files(
            &mut summary,
            &[CompactFile {
                path: "/tmp/artifacts/sess/context/api.md".into(),
                content: "# API".into(),
            }],
        );
        assert!(summary.contains("goal and next step"), "{summary}");
        assert!(
            summary.contains("/tmp/artifacts/sess/context/api.md"),
            "{summary}"
        );
        assert!(!summary.contains("{\"summary\""), "{summary}");
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
                if next.get("fail").and_then(Value::as_bool) == Some(true) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        [(header::CONTENT_TYPE, "application/json")],
                        r#"{"error":"compact failed"}"#.to_string(),
                    )
                        .into_response();
                }
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
        compact_sse_frames(&[
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
                }],
                "usage": { "prompt_tokens": 20, "completion_tokens": 8, "total_tokens": 28 }
            })
            .to_string(),
            "[DONE]".to_string(),
        ])
    }

    fn compact_sse_frames(payloads: &[String]) -> String {
        let mut out = String::new();
        for payload in payloads {
            out.push_str("data: ");
            out.push_str(payload);
            out.push_str("\n\n");
        }
        out
    }

    fn llm(base: &str, prompt: &str, max_tokens: u64) -> LlmCompactor {
        let client = completions_client("sk-test", base).expect("client");
        LlmCompactor::new(CodegLlmClient::Completions(client), "m", prompt, max_tokens)
    }

    fn request_has_no_tools(body: &Value) -> bool {
        match body.get("tools") {
            None => true,
            Some(Value::Null) => true,
            Some(Value::Array(tools)) => tools.is_empty(),
            Some(_) => false,
        }
    }

    #[tokio::test]
    async fn llm_compactor_small_evicted_is_one_tool_free_completion() {
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "L2-SUMMARY-BODY"})]).await;
        let compact = llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 2048);
        let evicted = vec![
            Message::user("hello-evicted"),
            Message::assistant("reply-evicted"),
        ];
        let artifact = Compactor::compact(&compact, "s", &evicted, None)
            .await
            .expect("compact");
        assert!(
            artifact.summary.contains("L2-SUMMARY-BODY"),
            "{}",
            artifact.summary
        );
        assert!(artifact.files.is_empty(), "{:?}", artifact.files);
        let captured = bodies.lock().expect("bodies").clone();
        assert_eq!(captured.len(), 1, "{captured:?}");
        assert!(request_has_no_tools(&captured[0]), "{:?}", captured[0]);
        let body = captured[0].to_string();
        assert!(body.contains("hello-evicted"), "{body}");
        assert!(body.contains("CODEG-COMPACT-PROMPT-MARKER"), "{body}");
    }

    #[tokio::test]
    async fn llm_compactor_includes_carry_over_and_new_evicted_text() {
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "NEW-SUMMARY"})]).await;
        let compact = llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 2048);
        let carry = CompactArtifact {
            summary: "PREV-SUMMARY-UNIQUE".into(),
            files: Vec::new(),
        };
        let evicted = vec![Message::user("NEW-EVICTED-UNIQUE")];
        let _ = Compactor::compact(&compact, "s", &evicted, Some(&carry))
            .await
            .expect("compact");
        let captured = bodies.lock().expect("bodies").clone();
        assert_eq!(captured.len(), 1, "{captured:?}");
        let body = captured[0].to_string();
        assert!(body.contains("Previous summary:"), "{body}");
        assert!(body.contains("PREV-SUMMARY-UNIQUE"), "{body}");
        assert!(body.contains("NEW-EVICTED-UNIQUE"), "{body}");
        assert!(body.contains("Evicted turns:"), "{body}");
    }

    #[tokio::test]
    async fn llm_compactor_empty_model_output_is_error() {
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "   "})]).await;
        let compact = llm(&base, "marker", 2048);
        let evicted = vec![Message::user("x")];
        let err = Compactor::compact(&compact, "s", &evicted, None).await;
        assert!(err.is_err(), "{err:?}");
        assert_eq!(bodies.lock().expect("bodies").len(), 1);
    }

    #[tokio::test]
    async fn llm_compactor_cancelled_token_errors_without_hanging() {
        let (base, bodies) = spawn_json_completions(vec![json!({"text": "SHOULD-NOT-RUN"})]).await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let compact = llm(&base, "marker", 2048).with_cancel(cancel);
        let evicted = vec![Message::user("x")];
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            Compactor::compact(&compact, "s", &evicted, None),
        )
        .await
        .expect("cancelled compact must not hang");
        assert!(result.is_err(), "{result:?}");
        assert!(
            bodies.lock().expect("bodies").is_empty(),
            "cancelled compact must not call Completions: {:?}",
            bodies.lock().expect("bodies")
        );
    }

    #[tokio::test]
    async fn l2_with_artifacts_is_one_tool_free_completion() {
        let artifacts = tempfile::tempdir().expect("artifacts");
        let (base, bodies) = spawn_json_completions(vec![json!({
            "text": "## Handoff\n\nKeep the current work goal."
        })])
        .await;
        let compact =
            llm(&base, "CODEG-COMPACT-PROMPT-MARKER", 4096).with_artifacts(artifacts.path());
        let evicted = vec![
            Message::user("hello-evicted"),
            Message::assistant("reply-evicted"),
        ];
        let artifact = Compactor::compact(&compact, "s", &evicted, None)
            .await
            .expect("compact");
        assert!(
            artifact.summary.contains("## Handoff"),
            "{}",
            artifact.summary
        );
        assert!(artifact.files.is_empty(), "{:?}", artifact.files);
        assert!(!compact_context_dir(artifacts.path())
            .join("api.md")
            .exists());
        let captured = bodies.lock().expect("bodies").clone();
        assert_eq!(captured.len(), 1, "{captured:?}");
        let body = captured[0].to_string();
        assert!(request_has_no_tools(&captured[0]), "{body}");
        assert!(!body.contains("write_file"), "{body}");
        assert!(body.contains("CODEG-COMPACT-PROMPT-MARKER"), "{body}");
        assert!(body.contains("hello-evicted"), "{body}");
    }

    #[test]
    fn compact_chunks_keep_whole_messages() {
        let messages = vec![
            Message::user("H".repeat(COMPACT_CHUNK_MAX_BYTES + 8)),
            Message::user("tail-message"),
        ];
        let chunks = chunk_evicted_messages(&messages);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1);
        assert_eq!(chunks[1].len(), 1);
        let packed = vec![Message::user("a"), Message::user("b")];
        let one = chunk_evicted_messages(&packed);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].len(), 2);
    }
}

//! Recover one rejected provider request without restarting the Rig tool loop.
//!
//! Provider decoding and normalization stay in Rig. Only a context-length
//! rejection before the first raw event can trigger one compact-and-retry.

use std::sync::Arc;

use futures::StreamExt;
use rig::agent::ModelHandle;
use rig::client::CompletionClient;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, Message,
    ProviderCapabilities,
};
use rig::streaming::{normalize_stream, StreamingCompletionResponse, StreamingResult};
use tokio_util::sync::CancellationToken;

use crate::agent::context::{BudgetConfig, SessionMemory};

use super::CodegLlmClient;

type RecoveryContext = (Arc<SessionMemory>, BudgetConfig, CancellationToken);

pub(crate) fn model_with_recovery(
    client: CodegLlmClient,
    model_id: String,
    recovery: Option<RecoveryContext>,
) -> ModelHandle {
    let base = match &client {
        CodegLlmClient::Completions(client) => ModelHandle::new(client.completion_model(&model_id)),
        CodegLlmClient::Responses(client) => ModelHandle::new(client.completion_model(&model_id)),
    };
    match recovery {
        Some((memory, budget, cancel)) => ModelHandle::new(RecoveringModel {
            client,
            model_id,
            base,
            recovery: Some((memory, budget)),
            cancel,
        }),
        None => base,
    }
}

/// Cancellation at the provider boundary for a Rig runner without persisted
/// session memory (for example a child agent). In-flight tools still drain.
pub(crate) fn model_with_cancel(
    client: CodegLlmClient,
    model_id: String,
    cancel: CancellationToken,
) -> ModelHandle {
    let base = model_with_recovery(client.clone(), model_id.clone(), None);
    ModelHandle::new(RecoveringModel {
        client,
        model_id,
        base,
        recovery: None,
        cancel,
    })
}

struct RecoveringModel {
    client: CodegLlmClient,
    model_id: String,
    base: ModelHandle,
    recovery: Option<(Arc<SessionMemory>, BudgetConfig)>,
    cancel: CancellationToken,
}

impl RecoveringModel {
    async fn raw_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingResult, CompletionError> {
        let stream = match &self.client {
            CodegLlmClient::Completions(client) => normalize_stream(
                client
                    .completion_model(&self.model_id)
                    .raw_stream(request)
                    .await?,
                |response| Ok(("openai", response).into()),
            ),
            CodegLlmClient::Responses(client) => normalize_stream(
                client
                    .completion_model(&self.model_id)
                    .raw_stream(request)
                    .await?,
                |response| Ok(("openai", response).into()),
            ),
        };
        Ok(cancelable_stream(stream, self.cancel.clone()))
    }

    async fn compact_request(
        &self,
        mut request: CompletionRequest,
        error: CompletionError,
    ) -> Result<CompletionRequest, CompletionError> {
        let Some((memory, mut budget)) = self.recovery.clone() else {
            return Err(error);
        };
        let cancel = self.cancel.clone();
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if !is_context_overflow(&error) {
            return Err(error);
        }
        let Some(pending) = request.chat_history.last().cloned() else {
            return Err(error);
        };
        let systems = request
            .chat_history
            .iter()
            .take_while(|m| matches!(m, Message::System { .. }))
            .cloned()
            .collect::<Vec<_>>();
        // Count the actual fixed context of this attempt. Keep the original
        // fields on the request; this string is only a budgeting input.
        let mut fixed = request.preamble.clone().unwrap_or_default();
        for message in &systems {
            fixed.push_str(&serde_json::to_string(message)?);
        }
        if let Some(documents) = request.normalized_documents() {
            fixed.push_str(&serde_json::to_string(&documents)?);
        }
        let mut schemas = request
            .tools
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(schema) = &request.output_schema {
            schemas.push(serde_json::to_value(schema)?);
        }
        budget.max_output = request.max_tokens.unwrap_or(budget.max_output);
        let loaded = match memory
            .force_compact(&pending, &fixed, &schemas, budget, cancel.clone())
            .await
        {
            Ok(loaded) => loaded,
            Err(_) if cancel.is_cancelled() => return Err(cancelled()),
            Err(compact_error) => {
                tracing::warn!(%compact_error, "context overflow compaction failed");
                return Err(error);
            }
        };
        let mut history = systems;
        history.extend(loaded.messages);
        history.push(pending);
        // A forced load that could not reduce the request is not a retry.
        if !loaded.compacted || history == request.chat_history {
            return Err(error);
        }
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        request.chat_history = history;
        Ok(request)
    }
}

impl CompletionModel for RecoveringModel {
    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        let first = tokio::select! {
            _ = self.cancel.cancelled() => return Err(cancelled()),
            response = self.base.completion(request.clone()) => response,
        };
        match first {
            Ok(response) => Ok(response),
            Err(error) => {
                let retry = self.compact_request(request, error).await?;
                tokio::select! {
                    _ = self.cancel.cancelled() => Err(cancelled()),
                    response = self.base.completion(retry) => response,
                }
            }
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        let opened = tokio::select! {
            _ = self.cancel.cancelled() => return Err(cancelled()),
            stream = self.raw_stream(request.clone()) => stream,
        };
        let error = match opened {
            Ok(mut raw) => match raw.next().await {
                Some(Err(error)) => error,
                first => {
                    // Even a metadata event closes the recovery window. No
                    // emitted partial result or tool call is replayed.
                    let stream = futures::stream::iter(first).chain(raw);
                    return Ok(StreamingCompletionResponse::stream(
                        "openai",
                        Box::pin(stream),
                    ));
                }
            },
            Err(error) => error,
        };
        let retry = self.compact_request(request, error).await?;
        let raw = tokio::select! {
            _ = self.cancel.cancelled() => return Err(cancelled()),
            stream = self.raw_stream(retry) => stream?,
        };
        // Deliberately no recursion: the second provider failure terminates
        // this model request and leaves the existing Rig run in control.
        Ok(StreamingCompletionResponse::stream("openai", raw))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.base.capabilities()
    }
}

fn cancelable_stream(stream: StreamingResult, cancel: CancellationToken) -> StreamingResult {
    Box::pin(futures::stream::unfold(
        (stream, cancel, false),
        |(mut stream, cancel, done)| async move {
            if done {
                return None;
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Some((Err(cancelled()), (stream, cancel, true))),
                item = stream.next() => item.map(|item| (item, (stream, cancel, false))),
            }
        },
    ))
}

fn cancelled() -> CompletionError {
    CompletionError::ResponseError("cancelled".into())
}

fn is_context_overflow(error: &CompletionError) -> bool {
    let json = error.provider_response_json().ok().flatten();
    let detail = json
        .as_ref()
        .and_then(|value| value.get("error"))
        .or(json.as_ref());
    if let Some(code) = detail
        .and_then(|value| value.get("code"))
        .and_then(|v| v.as_str())
    {
        if matches!(
            code,
            "context_length_exceeded"
                | "context_window_exceeded"
                | "max_context_length"
                | "prompt_too_long"
                | "input_too_long"
        ) {
            return true;
        }
    }
    let message = detail
        .and_then(|value| value.get("message"))
        .and_then(|v| v.as_str())
        .or_else(|| error.provider_response_body())
        .unwrap_or_default()
        .to_ascii_lowercase();
    message.contains("maximum context length") && message.contains("exceed")
        || message.contains("exceeds the context window")
        || message.contains("prompt is too long")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{context::LlmCompactor, model::completions_client};
    use axum::{extract::Json, http::StatusCode, response::IntoResponse, routing::post, Router};
    use futures::StreamExt;
    use rig::{agent::AgentBuilder, memory::ConversationMemory};
    use serde_json::{json, Value};
    use std::sync::Mutex;

    async fn gateway(script: Vec<Value>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let steps = Arc::new(Mutex::new(std::collections::VecDeque::from(script)));
        let router = Router::new().fallback(post(move |Json(body): Json<Value>| {
            let steps = steps.clone();
            captured.lock().unwrap().push(body.clone());
            async move {
                let is_compact = body.to_string().contains("SUMMARIZE_OVERFLOW_FIXTURE");
                let step = if is_compact { json!({"text":"Preserved objective and results."}) }
                    else { steps.lock().unwrap().pop_front().expect("bounded requests") };
                if step["non_overflow"] == true {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error": {
                        "code":"invalid_request_error", "message":"invalid tool schema"
                    }}))).into_response();
                }
                if step["tool"] == true {
                    let frames = [
                        json!({"id":"tool-response", "object":"chat.completion.chunk", "choices":[{"index":0,"delta":{
                            "role":"assistant", "tool_calls":[{"index":0,"id":"write-once","type":"function",
                            "function":{"name":"write_once","arguments":"{}"}}]}}]}),
                        json!({"id":"tool-response", "object":"chat.completion.chunk", "choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
                    ];
                    let mut sse = frames.iter().map(|frame| format!("data: {frame}\n\n")).collect::<String>();
                    sse.push_str("data: [DONE]\n\n");
                    return ([("content-type", "text/event-stream")], sse).into_response();
                }
                if step["overflow"] == true {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error": {
                        "code":"context_length_exceeded", "message":"maximum context length exceeded"
                    }}))).into_response();
                }
                let mut sse = format!("data: {}\n\n", json!({
                    "id":"response-id", "object":"chat.completion.chunk",
                    "choices":[{"index":0,"delta":{"role":"assistant","content":step["text"].as_str().unwrap_or("ok")}}]
                }));
                if step["late_overflow"] == true {
                    sse.push_str(&format!("data: {}\n\n", json!({"error":{
                        "code":"context_length_exceeded", "message":"maximum context length exceeded"
                    }})));
                } else {
                    sse.push_str(&format!("data: {}\n\ndata: [DONE]\n\n", json!({
                        "id":"response-id", "object":"chat.completion.chunk",
                        "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                        "usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}
                    })));
                }
                ([("content-type", "text/event-stream")], sse).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{addr}/v1"), requests)
    }

    async fn run(script: Vec<Value>) -> (bool, Vec<Value>, Vec<Message>) {
        let dir = tempfile::tempdir().unwrap();
        let (base, requests) = gateway(script).await;
        let client = CodegLlmClient::Completions(completions_client("test", &base).unwrap());
        let budget = BudgetConfig::new(128_000, 2048);
        let memory = Arc::new(SessionMemory::open_in(
            dir.path(),
            "overflow",
            "/tmp/overflow",
            80_000,
            "m",
            LlmCompactor::new(client.clone(), "m", "SUMMARIZE_OVERFLOW_FIXTURE", 512),
        ));
        let history = (0..8)
            .flat_map(|i| {
                [
                    Message::user(format!(
                        "old request {i}: {}",
                        "recorded detail ".repeat(100)
                    )),
                    Message::assistant(format!("old answer {i}")),
                ]
            })
            .collect::<Vec<_>>();
        memory
            .inner()
            .append("overflow", history.clone())
            .await
            .unwrap();
        let prompt = Message::user("finish current request");
        memory.begin_run(&prompt).await.unwrap();
        let model = model_with_recovery(
            client,
            "m".into(),
            Some((memory.clone(), budget, CancellationToken::new())),
        );
        let mut stream = AgentBuilder::from_model_handle(model)
            .preamble("original system")
            .context("original document")
            .build()
            .runner(prompt)
            .history(history)
            .max_turns(1)
            .stream()
            .await;
        let mut ok = true;
        while let Some(item) = stream.next().await {
            if item.is_err() {
                ok = false;
            }
        }
        let messages = memory.inner().load_committed().await.unwrap();
        let captured = requests.lock().unwrap().clone();
        (ok, captured, messages)
    }

    #[tokio::test]
    async fn rejected_request_compacts_and_retries_once_with_original_prompt_and_context() {
        let (ok, requests, originals) =
            run(vec![json!({"overflow":true}), json!({"text":"done"})]).await;
        assert!(ok, "provider overflow before output should be recoverable");
        let main = requests
            .iter()
            .filter(|r| !r.to_string().contains("SUMMARIZE_OVERFLOW_FIXTURE"))
            .collect::<Vec<_>>();
        assert_eq!(main.len(), 2);
        assert!(main[1].to_string().len() < main[0].to_string().len());
        let retried = main[1].to_string();
        assert!(retried.contains("original system"));
        assert!(retried.contains("original document"));
        assert_eq!(retried.matches("finish current request").count(), 1);
        assert_eq!(originals.len(), 17, "compaction must not rewrite originals");
    }

    #[tokio::test]
    async fn second_overflow_stops_without_unbounded_retry() {
        let (ok, requests, _) = run(vec![json!({"overflow":true}), json!({"overflow":true})]).await;
        assert!(!ok);
        assert_eq!(
            requests
                .iter()
                .filter(|r| !r.to_string().contains("SUMMARIZE_OVERFLOW_FIXTURE"))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn overflow_after_streamed_content_is_not_retried() {
        let (ok, requests, _) =
            run(vec![json!({"text":"partial result", "late_overflow":true})]).await;
        assert!(!ok);
        assert_eq!(requests.len(), 1);
    }
    #[tokio::test]
    async fn unrelated_provider_rejection_does_not_compact_or_retry() {
        let (ok, requests, _) = run(vec![json!({"non_overflow":true})]).await;
        assert!(!ok);
        assert_eq!(requests.len(), 1);
    }

    struct SaveCommitted(Arc<SessionMemory>);

    impl rig::agent::AgentHook for SaveCommitted {
        async fn on_messages_committed(
            &self,
            _ctx: &rig::agent::HookContext,
            event: rig::agent::CommittedMessages<'_>,
        ) -> rig::agent::CommittedMessagesAction {
            self.0
                .recorder()
                .persist_committed(event.messages)
                .await
                .expect("confirmed messages");
            rig::agent::CommittedMessagesAction::continue_run()
        }
    }

    struct WriteOnce(Arc<std::sync::atomic::AtomicUsize>);

    impl rig::tool::Tool for WriteOnce {
        const NAME: &'static str = "write_once";
        type Args = Value;
        type Output = String;
        type Error = rig::tool::ToolExecutionError;
        fn description(&self) -> String {
            "Record a side effect".into()
        }
        fn parameters(&self) -> Value {
            json!({"type":"object","properties":{}})
        }
        async fn call(
            &self,
            _ctx: &mut rig::tool::ToolContext,
            _args: Value,
        ) -> Result<String, Self::Error> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("confirmed write receipt".into())
        }
    }

    #[tokio::test]
    async fn overflow_after_completed_tool_retries_model_without_replaying_side_effect() {
        let dir = tempfile::tempdir().unwrap();
        let (base, requests) = gateway(vec![
            json!({"tool":true}),
            json!({"overflow":true}),
            json!({"text":"done"}),
        ])
        .await;
        let client = CodegLlmClient::Completions(completions_client("test", &base).unwrap());
        let budget = BudgetConfig::new(128_000, 2048);
        let memory = Arc::new(SessionMemory::open_in(
            dir.path(),
            "tool-overflow",
            "/tmp/overflow",
            80_000,
            "m",
            LlmCompactor::new(client.clone(), "m", "SUMMARIZE_OVERFLOW_FIXTURE", 512),
        ));
        let history = (0..8)
            .flat_map(|i| {
                [
                    Message::user(format!(
                        "old request {i}: {}",
                        "recorded detail ".repeat(100)
                    )),
                    Message::assistant(format!("old answer {i}")),
                ]
            })
            .collect::<Vec<_>>();
        memory
            .inner()
            .append("tool-overflow", history.clone())
            .await
            .unwrap();
        let prompt = Message::user("write once and finish");
        memory.begin_run(&prompt).await.unwrap();
        let side_effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let model = model_with_recovery(
            client,
            "m".into(),
            Some((memory.clone(), budget, CancellationToken::new())),
        );
        let mut stream = AgentBuilder::from_model_handle(model)
            .tool(WriteOnce(side_effects.clone()))
            .build()
            .runner(prompt)
            .history(history)
            .add_hook(SaveCommitted(memory.clone()))
            .max_turns(2)
            .stream()
            .await;
        while let Some(item) = stream.next().await {
            item.expect("retry must continue the same Rig run");
        }
        assert_eq!(side_effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        let originals = memory.inner().load_committed().await.unwrap();
        assert_eq!(originals.len(), 20);
        let captured = requests.lock().unwrap();
        let main = captured
            .iter()
            .filter(|r| !r.to_string().contains("SUMMARIZE_OVERFLOW_FIXTURE"))
            .collect::<Vec<_>>();
        assert_eq!(main.len(), 3);
        assert!(main[2].to_string().contains("confirmed write receipt"));
    }
}

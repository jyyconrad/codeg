//! PR2 contract tests: Chat Completions path, Hook, tools, MCP names, RequestPatch.

use std::sync::{Arc, Mutex};

use axum::extract::Json;
use axum::http::{header, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use futures::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::client::AgentClientExt;
use rig::completion::Message;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::hook::{CodegHook, HookTrace, PendingPermission};
use super::model::{
    completions_client, DEFAULT_INVALID_TOOL_CALL_RETRIES, DEFAULT_MAX_TURNS,
    DEFAULT_TOOL_CONCURRENCY,
};
use super::tools::{EchoTool, FailTool};

#[derive(Clone)]
struct ScriptedCompletions {
    paths: Arc<Mutex<Vec<String>>>,
    bodies: Arc<Mutex<Vec<Value>>>,
    script: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedCompletions {
    fn new(script: Vec<Value>) -> Self {
        Self {
            paths: Arc::new(Mutex::new(Vec::new())),
            bodies: Arc::new(Mutex::new(Vec::new())),
            script: Arc::new(Mutex::new(script)),
        }
    }

    fn paths(&self) -> Vec<String> {
        self.paths.lock().expect("paths").clone()
    }

    fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().expect("bodies").clone()
    }
}

async fn spawn_completions(script: Vec<Value>) -> (String, ScriptedCompletions) {
    let capture = ScriptedCompletions::new(script);
    let state = capture.clone();
    let app = Router::new().fallback(post(move |uri: Uri, Json(body): Json<Value>| {
        let state = state.clone();
        async move {
            state
                .paths
                .lock()
                .expect("paths")
                .push(uri.path().to_string());
            state.bodies.lock().expect("bodies").push(body);
            let next = {
                let mut script = state.script.lock().expect("script");
                if script.is_empty() {
                    sse_text("ok")
                } else {
                    script.remove(0)
                }
            };
            let sse = value_to_sse(&next);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/event-stream")],
                sse,
            )
                .into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock completions");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/v1"), capture)
}

fn sse_text(text: &str) -> Value {
    json!({
        "kind": "text",
        "text": text,
    })
}

fn sse_tool(name: &str, id: &str, arguments: Value) -> Value {
    json!({
        "kind": "tools",
        "calls": [{ "name": name, "id": id, "arguments": arguments }]
    })
}

fn sse_tools(calls: Vec<Value>) -> Value {
    json!({ "kind": "tools", "calls": calls })
}

fn script_usage(script: &Value, prompt_tokens: u64) -> Value {
    script.get("usage").cloned().unwrap_or(json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": 4,
        "total_tokens": prompt_tokens + 4
    }))
}

fn value_to_sse(script: &Value) -> String {
    match script.get("kind").and_then(Value::as_str) {
        Some("tools") => {
            let calls = script
                .get("calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let tool_calls: Vec<Value> = calls
                .iter()
                .enumerate()
                .map(|(index, call)| {
                    json!({
                        "index": index,
                        "id": call.get("id").and_then(Value::as_str).unwrap_or("call"),
                        "type": "function",
                        "function": {
                            "name": call.get("name").and_then(Value::as_str).unwrap_or("echo"),
                            "arguments": call.get("arguments").cloned().unwrap_or(json!({})).to_string()
                        }
                    })
                })
                .collect();
            sse_frames(&[
                json!({
                    "id": "chatcmpl-1",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": null, "tool_calls": tool_calls },
                        "finish_reason": null
                    }]
                })
                .to_string(),
                json!({
                    "id": "chatcmpl-1",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "tool_calls"
                    }],
                    "usage": script_usage(script, 8)
                })
                .to_string(),
                "[DONE]".to_string(),
            ])
        }
        _ => {
            let text = script.get("text").and_then(Value::as_str).unwrap_or("ok");
            sse_frames(&[
                json!({
                    "id": "chatcmpl-1",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": text },
                        "finish_reason": null
                    }]
                })
                .to_string(),
                json!({
                    "id": "chatcmpl-1",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": script_usage(script, 4)
                })
                .to_string(),
                "[DONE]".to_string(),
            ])
        }
    }
}

fn sse_frames(payloads: &[String]) -> String {
    let mut out = String::new();
    for payload in payloads {
        out.push_str("data: ");
        out.push_str(payload);
        out.push_str("\n\n");
    }
    out
}

struct Drain {
    output: String,
    error: Option<String>,
}

async fn drain_stream(mut stream: rig::agent::StreamingResult) -> Drain {
    let mut output = String::new();
    let mut error = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                output = response.output.clone();
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(item)) => {
                if let Some(text) = assistant_text(&item) {
                    output.push_str(&text);
                }
            }
            Err(err) => {
                error = Some(err.to_string());
                break;
            }
            Ok(_) => {}
        }
    }
    Drain { output, error }
}

fn assistant_text(item: &rig::streaming::StreamedAssistantContent) -> Option<String> {
    let value = serde_json::to_value(item).ok()?;
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .pointer("/delta")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

#[tokio::test]
async fn chat_completions_path_one_delta_one_output() {
    let (base, capture) = spawn_completions(vec![sse_text("hello-delta")]).await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .preamble("you are tests")
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let trace = HookTrace::new();
    let stream = agent
        .runner("say hello")
        .history(Vec::<Message>::new())
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(DEFAULT_TOOL_CONCURRENCY)
        .max_invalid_tool_call_retries(DEFAULT_INVALID_TOOL_CALL_RETRIES)
        .add_hook(CodegHook::auto_allow(trace.clone()))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert!(
        drain.output.contains("hello-delta") || trace.aggregated_text().contains("hello-delta"),
        "output={} aggregated={}",
        drain.output,
        trace.aggregated_text()
    );
    assert_eq!(trace.text_deltas().len(), 1, "{:?}", trace.text_deltas());
    assert_eq!(trace.completion_calls(), 1);
    let paths = capture.paths();
    assert!(
        paths.iter().all(|p| p.ends_with("/chat/completions")),
        "expected chat/completions, got {paths:?}"
    );
    assert!(
        paths.iter().all(|p| !p.contains("/responses")),
        "must not use Responses path: {paths:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_token_stops_completion_call_before_http() {
    let (base, capture) = spawn_completions(vec![sse_text("nope")]).await;
    let client = completions_client("sk-test", &base).expect("client");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let (tx, _rx) = mpsc::channel::<PendingPermission>(1);
    let trace = HookTrace::new();
    let agent = client.agent("codeg-test").build();
    let stream = agent
        .runner("hi")
        .add_hook(CodegHook::waiting(trace, tx, cancel))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(
        drain
            .error
            .as_deref()
            .is_some_and(|err| err.to_ascii_lowercase().contains("cancel"))
            || drain.output.is_empty(),
        "cancelled completion call must stop the run: {:?}",
        drain.error
    );
    assert!(
        capture.bodies().is_empty(),
        "cancelled pre-HTTP hook must not issue a completion request: {:?}",
        capture.bodies()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_tool_run_skip_and_stop() {
    let echo_run = EchoTool::new();
    let (base, _) = spawn_completions(vec![
        sse_tool("echo", "call_run", json!({"text": "ran"})),
        sse_text("after-run"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .preamble("tools")
        .default_max_turns(DEFAULT_MAX_TURNS)
        .tool(echo_run.clone())
        .build();
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let cancel = tokio_util::sync::CancellationToken::new();
    let trace = HookTrace::new();
    let hook = CodegHook::waiting(trace.clone(), tx, cancel);
    let stream = agent
        .runner("use echo")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .max_invalid_tool_call_retries(2)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let pending = rx.recv().await.expect("one permission card");
    assert_eq!(pending.tool_name, "echo");
    pending.allow();
    let drain = drain_task.await.expect("join run");
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert_eq!(trace.tool_calls(), vec!["echo".to_string()]);
    assert_eq!(echo_run.calls(), vec!["ran".to_string()]);
    assert_eq!(trace.tool_results()[0].status, "success");

    let echo_skip = EchoTool::new();
    let (base, _) = spawn_completions(vec![
        sse_tool("echo", "call_skip", json!({"text": "nope"})),
        sse_text("after-skip"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(echo_skip.clone())
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let trace = HookTrace::new();
    let hook = CodegHook::waiting(
        trace.clone(),
        tx,
        tokio_util::sync::CancellationToken::new(),
    );
    let stream = agent
        .runner("skip echo")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let pending = rx.recv().await.expect("skip permission");
    pending.reject("user denied");
    let drain = drain_task.await.expect("join skip");
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert!(echo_skip.calls().is_empty(), "skip must not run the body");
    assert_eq!(trace.tool_results()[0].status, "skipped");

    let echo_stop = EchoTool::new();
    let (base, _) =
        spawn_completions(vec![sse_tool("echo", "call_stop", json!({"text": "halt"}))]).await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(echo_stop.clone())
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let trace = HookTrace::new();
    let hook = CodegHook::waiting(
        trace.clone(),
        tx,
        tokio_util::sync::CancellationToken::new(),
    );
    let stream = agent
        .runner("stop echo")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let pending = rx.recv().await.expect("stop permission");
    pending.cancel("user cancelled");
    let drain = drain_task.await.expect("join stop");
    assert!(drain.error.is_some(), "stop should terminate the run");
    assert!(echo_stop.calls().is_empty(), "stop must not run the body");
}

#[tokio::test]
async fn tool_error_feeds_model_and_records_error_status() {
    let fail = FailTool::new();
    let (base, capture) = spawn_completions(vec![
        sse_tool("fail_tool", "call_fail", json!({})),
        sse_text("noted-error"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(fail.clone())
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let trace = HookTrace::new();
    let stream = agent
        .runner("fail please")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(CodegHook::auto_allow(trace.clone()))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert_eq!(fail.call_count(), 1);
    assert_eq!(trace.tool_results()[0].status, "error");
    assert!(
        trace.tool_results()[0]
            .presentation
            .contains("file does not exist: demo.txt"),
        "{:?}",
        trace.tool_results()
    );
    let second = capture.bodies().get(1).cloned().unwrap_or(json!({}));
    let dumped = second.to_string();
    assert!(
        dumped.contains("file does not exist: demo.txt"),
        "model-visible feedback missing from follow-up request: {dumped}"
    );
}

#[tokio::test]
async fn invalid_retry_discards_legal_siblings() {
    let echo = EchoTool::new();
    let (base, _) = spawn_completions(vec![
        sse_tools(vec![
            json!({"name": "not_a_tool", "id": "call_bad", "arguments": {}}),
            json!({"name": "echo", "id": "call_sib", "arguments": {"text": "sibling"}}),
        ]),
        sse_text("recovered"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(echo.clone())
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let trace = HookTrace::new();
    let stream = agent
        .runner("mixed tools")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .max_invalid_tool_call_retries(2)
        .add_hook(CodegHook::auto_allow(trace.clone()))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert_eq!(trace.invalid_calls(), vec!["not_a_tool".to_string()]);
    assert!(
        echo.calls().is_empty(),
        "legal sibling must not execute after invalid Retry: {:?}",
        echo.calls()
    );
    assert!(
        drain.output.contains("recovered") || trace.aggregated_text().contains("recovered"),
        "output={}",
        drain.output
    );
}

#[tokio::test]
async fn request_patch_hits_every_http_call_without_rewriting_canonical_history() {
    let echo = EchoTool::new();
    let (base, capture) = spawn_completions(vec![
        sse_tool("echo", "call_p", json!({"text": "ok"})),
        sse_text("done"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(echo)
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let canonical = vec![
        Message::user("remember ALPHA-TOKEN"),
        Message::assistant("noted"),
    ];
    let trace = HookTrace::new();
    let hook = CodegHook::auto_allow(trace.clone()).with_request_patch(Vec::new(), Some(256));
    let stream = agent
        .runner("use echo")
        .history(canonical)
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert!(
        trace.completion_calls() >= 2,
        "{}",
        trace.completion_calls()
    );
    let bodies = capture.bodies();
    assert!(bodies.len() >= 2, "{bodies:?}");
    for body in &bodies {
        let dumped = body.to_string();
        assert!(
            !dumped.contains("ALPHA-TOKEN"),
            "patched request must not carry dropped history: {dumped}"
        );
        let cap = body
            .get("max_tokens")
            .or_else(|| body.get("max_completion_tokens"));
        assert_eq!(cap, Some(&json!(256)), "{body}");
    }
    let canonical_dump = trace.completion_histories().join("\n");
    assert!(
        canonical_dump.contains("ALPHA-TOKEN"),
        "RequestPatch must not overwrite Runner history seen by later completion-call hooks: {canonical_dump}"
    );
}

fn native_state(
    store: std::sync::Arc<std::sync::Mutex<crate::agent::context::ContextStore>>,
    recorder: std::sync::Arc<crate::agent::context::FactRecorder>,
    identity: std::sync::Arc<crate::agent::context::CallIdentityBridge>,
    window: u64,
    max_output: u64,
) -> crate::agent::hook::NativeRunState {
    crate::agent::hook::NativeRunState {
        turn_id: 1,
        turn_key: "s:1".into(),
        budget: crate::agent::context::BudgetConfig { window, max_output },
        preamble: "short preamble".into(),
        tool_schemas: Vec::new(),
        store,
        identity,
        recorder,
        last_estimate: std::sync::Arc::new(std::sync::Mutex::new(0)),
        last_usage_input: std::sync::Arc::new(std::sync::Mutex::new(None)),
        feedback: None,
        mcp_readonly: std::sync::Arc::new(std::collections::HashSet::new()),
        compact: None,
    }
}

#[tokio::test]
async fn last_request_usage_is_not_the_run_aggregate() {
    use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder};

    let echo = EchoTool::new();
    let (base, _) = spawn_completions(vec![
        json!({
            "kind": "tools",
            "calls": [{ "name": "echo", "id": "c1", "arguments": { "text": "ok" } }],
            "usage": { "prompt_tokens": 30000, "completion_tokens": 10, "total_tokens": 30010 }
        }),
        json!({
            "kind": "text",
            "text": "done",
            "usage": { "prompt_tokens": 32000, "completion_tokens": 5, "total_tokens": 32005 }
        }),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
    let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
    let identity = std::sync::Arc::new(CallIdentityBridge::new());
    let native = native_state(
        std::sync::Arc::clone(&store),
        std::sync::Arc::clone(&recorder),
        identity,
        128_000,
        4096,
    );
    let last_usage = std::sync::Arc::clone(&native.last_usage_input);
    let agent = client
        .agent("codeg-test")
        .tool(echo)
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let stream = agent
        .runner("use echo")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(CodegHook::auto_allow(HookTrace::new()).with_native(native))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_none(), "{:?}", drain.error);
    let used = last_usage.lock().expect("usage").unwrap_or(0);
    assert!(
        used < 60_000,
        "occupancy must be last-request input, not 30k+32k aggregate; got {used}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_a_success_b_cancel_keeps_a_and_does_not_replay() {
    use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder, ToolOutcome};
    use crate::agent::tools::SideEffectTool;

    let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
    let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
    let identity = std::sync::Arc::new(CallIdentityBridge::new());
    let tool = SideEffectTool::new(
        std::sync::Arc::clone(&identity),
        std::sync::Arc::clone(&recorder),
        1,
    );
    let native = native_state(
        std::sync::Arc::clone(&store),
        std::sync::Arc::clone(&recorder),
        std::sync::Arc::clone(&identity),
        128_000,
        4096,
    );
    let (base, _) = spawn_completions(vec![sse_tools(vec![
        json!({"name": "write_mem", "id": "call_a", "arguments": {"text": "A"}}),
        json!({"name": "write_mem", "id": "call_b", "arguments": {"text": "B"}}),
    ])])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(tool.clone())
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let cancel = tokio_util::sync::CancellationToken::new();
    let hook = CodegHook::waiting(HookTrace::new(), tx, cancel.clone()).with_native(native);
    let stream = agent
        .runner("write A then B")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let first = rx.recv().await.expect("permission A");
    first.allow();
    let second = rx.recv().await.expect("permission B");
    second.cancel("user cancelled");
    let _ = drain_task.await;
    store.lock().expect("store").settle_cancel("s:1");
    assert_eq!(tool.effects(), vec!["A".to_string()]);
    let a = store.lock().expect("store").fact("call_a").cloned();
    let b = store.lock().expect("store").fact("call_b").cloned();
    assert_eq!(
        a.as_ref().and_then(|f| f.outcome),
        Some(ToolOutcome::Success)
    );
    assert_eq!(
        b.as_ref().and_then(|f| f.outcome),
        Some(ToolOutcome::Cancelled)
    );
    assert!(store.lock().expect("store").auto_replay_ids().is_empty());
}

#[tokio::test]
async fn over_budget_prompt_stops_before_http() {
    use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder};

    let (base, capture) = spawn_completions(vec![sse_text("should-not-run")]).await;
    let client = completions_client("sk-test", &base).expect("client");
    let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
    let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
    let identity = std::sync::Arc::new(CallIdentityBridge::new());
    let native = native_state(store, recorder, identity, 4096, 2048);
    let agent = client
        .agent("codeg-test")
        .preamble("p")
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let stream = agent
        .runner("x".repeat(8000))
        .max_turns(DEFAULT_MAX_TURNS)
        .add_hook(CodegHook::auto_allow(HookTrace::new()).with_native(native))
        .stream()
        .await;
    let drain = drain_stream(stream).await;
    assert!(drain.error.is_some(), "over-budget must stop the turn");
    assert!(
        capture.bodies().is_empty(),
        "must not send a locally over-budget request: {:?}",
        capture.bodies()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_file_reject_is_failed_and_does_not_write() {
    use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
    use crate::acp::session_state::{SessionState, ToolCallStatus};
    use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder, ToolOutcome};
    use crate::agent::session::TurnCoordinator;
    use crate::agent::tools::{acp_card_status_from_rig, NativeToolCtx, WriteFileTool};
    use crate::models::agent::AgentType;
    use crate::web::event_bridge::EventEmitter;
    use std::path::PathBuf;
    use tokio::sync::RwLock;

    assert_eq!(acp_card_status_from_rig("skipped"), "failed");
    assert_eq!(acp_card_status_from_rig("error"), "failed");
    assert_eq!(acp_card_status_from_rig("success"), "completed");

    let dir = tempfile::tempdir().expect("dir");
    let target = dir.path().join("denied.txt");
    let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
    let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
    let identity = std::sync::Arc::new(CallIdentityBridge::new());
    let native = native_state(
        std::sync::Arc::clone(&store),
        std::sync::Arc::clone(&recorder),
        std::sync::Arc::clone(&identity),
        128_000,
        4096,
    );
    let ctx = NativeToolCtx {
        turn_id: 1,
        identity: std::sync::Arc::clone(&identity),
        recorder: std::sync::Arc::clone(&recorder),
        cancel: tokio_util::sync::CancellationToken::new(),
        launch_cwd: dir.path().to_path_buf(),
        fs: std::sync::Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(
            dir.path(),
        ))),
        session_id: "s".into(),
    };
    let tool = WriteFileTool::new(ctx);
    let (base, _) = spawn_completions(vec![
        sse_tool(
            "write_file",
            "call_deny",
            json!({"path": "denied.txt", "content": "nope"}),
        ),
        sse_text("after-deny"),
    ])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(tool)
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();

    let connection_id = "write-deny".to_string();
    let state = SessionState::new(
        connection_id,
        AgentType::CodegAgent,
        Some(PathBuf::from("/tmp")),
        "main".into(),
        None,
    );
    let state = std::sync::Arc::new(RwLock::new(state));
    let turn = std::sync::Arc::new(TurnCoordinator::new());
    let (turn_id, _) = turn.begin();
    let host = crate::agent::hook::HostBridge {
        emitter: EventEmitter::Noop,
        session_state: std::sync::Arc::clone(&state),
        turn,
        turn_id,
    };
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let cancel = tokio_util::sync::CancellationToken::new();
    let trace = HookTrace::new();
    let hook = CodegHook::for_session(trace.clone(), tx, cancel, host).with_native(native);
    let stream = agent
        .runner("write denied")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let pending = rx.recv().await.expect("permission");
    pending.reject("user rejected");
    let drain = drain_task.await.expect("join");
    assert!(drain.error.is_none(), "{:?}", drain.error);
    assert!(!target.exists(), "reject must not write");
    assert_eq!(trace.tool_results()[0].status, "skipped");
    let fact = store.lock().expect("store").fact("call_deny").cloned();
    assert_eq!(fact.and_then(|f| f.outcome), Some(ToolOutcome::Rejected));
    let snapshot = state.read().await;
    let card = snapshot
        .active_tool_calls
        .get("call_deny")
        .expect("tool card");
    assert_eq!(
        card.status,
        ToolCallStatus::Failed,
        "reject/cancel cards must be failed, not completed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn write_a_cancel_b_keeps_file_and_does_not_replay() {
    use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
    use crate::agent::context::{CallIdentityBridge, ContextStore, FactRecorder, ToolOutcome};
    use crate::agent::tools::{NativeToolCtx, WriteFileTool};

    let dir = tempfile::tempdir().expect("dir");
    let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
    let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
    let identity = std::sync::Arc::new(CallIdentityBridge::new());
    let native = native_state(
        std::sync::Arc::clone(&store),
        std::sync::Arc::clone(&recorder),
        std::sync::Arc::clone(&identity),
        128_000,
        4096,
    );
    let ctx = NativeToolCtx {
        turn_id: 1,
        identity: std::sync::Arc::clone(&identity),
        recorder: std::sync::Arc::clone(&recorder),
        cancel: tokio_util::sync::CancellationToken::new(),
        launch_cwd: dir.path().to_path_buf(),
        fs: std::sync::Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(
            dir.path(),
        ))),
        session_id: "s".into(),
    };
    let tool = WriteFileTool::new(ctx);
    let (base, _) = spawn_completions(vec![sse_tools(vec![
        json!({"name": "write_file", "id": "call_a", "arguments": {"path": "a.txt", "content": "A"}}),
        json!({"name": "write_file", "id": "call_b", "arguments": {"path": "b.txt", "content": "B"}}),
    ])])
    .await;
    let client = completions_client("sk-test", &base).expect("client");
    let agent = client
        .agent("codeg-test")
        .tool(tool)
        .default_max_turns(DEFAULT_MAX_TURNS)
        .build();
    let (tx, mut rx) = mpsc::channel::<PendingPermission>(4);
    let cancel = tokio_util::sync::CancellationToken::new();
    let hook = CodegHook::waiting(HookTrace::new(), tx, cancel).with_native(native);
    let stream = agent
        .runner("write A then B")
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(1)
        .add_hook(hook)
        .stream()
        .await;
    let drain_task = tokio::spawn(drain_stream(stream));
    let first = rx.recv().await.expect("permission A");
    first.allow();
    let second = rx.recv().await.expect("permission B");
    second.cancel("user cancelled");
    let _ = drain_task.await;
    store.lock().expect("store").settle_cancel("s:1");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "A"
    );
    assert!(!dir.path().join("b.txt").exists(), "B must not be written");
    let a = store.lock().expect("store").fact("call_a").cloned();
    let b = store.lock().expect("store").fact("call_b").cloned();
    assert_eq!(a.and_then(|f| f.outcome), Some(ToolOutcome::Success));
    assert_eq!(b.and_then(|f| f.outcome), Some(ToolOutcome::Cancelled));
    assert!(store.lock().expect("store").auto_replay_ids().is_empty());
}

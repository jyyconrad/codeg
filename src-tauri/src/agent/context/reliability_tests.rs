//! Regression coverage for session compaction transactions.
//!
//! The parent module wires this file under `#[cfg(test)]`; keeping the tests
//! here avoids changing the production context module surface.

use std::sync::{Arc, Mutex};

use axum::extract::Json;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use rig::completion::Message;
use rig::memory::ConversationMemory;
use serde_json::{json, Value};

use super::{BudgetConfig, LlmCompactor, SessionMemory};
use crate::agent::model::CodegLlmClient;

#[derive(Clone)]
struct CompactFixture {
    requests: Arc<Mutex<Vec<Value>>>,
    summary: String,
}

async fn spawn_compact_fixture(summary: &str) -> (String, CompactFixture) {
    let fixture = CompactFixture {
        requests: Arc::new(Mutex::new(Vec::new())),
        summary: summary.to_string(),
    };
    let state = fixture.clone();
    let app = Router::new().fallback(post(move |Json(body): Json<Value>| {
        let state = state.clone();
        async move {
            state.requests.lock().expect("fixture requests").push(body);
            let response = compact_sse(&state.summary);
            ([(header::CONTENT_TYPE, "text/event-stream")], response).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind compact fixture");
    let addr = listener.local_addr().expect("compact fixture addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/v1"), fixture)
}

fn compact_sse(summary: &str) -> String {
    let first = json!({
        "id": "chatcmpl-compact-reliability",
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": summary},
            "finish_reason": null
        }]
    });
    let last = json!({
        "id": "chatcmpl-compact-reliability",
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 20, "completion_tokens": 8, "total_tokens": 28}
    });
    format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n")
}

fn session(base: &str, root: &std::path::Path, id: &str) -> SessionMemory {
    let client = crate::agent::model::completions_client("sk-test", base).expect("client");
    SessionMemory::open_in(
        root,
        id,
        "/tmp/reliability",
        usize::MAX / 4,
        "m",
        LlmCompactor::new(
            CodegLlmClient::Completions(client),
            "m",
            "RELIABILITY_COMPACT_PROMPT",
            512,
        ),
    )
}

fn long_turn(label: &str, index: usize) -> [Message; 2] {
    [
        Message::user(format!(
            "{label}-user-{index} {}",
            "historical detail ".repeat(220)
        )),
        Message::assistant(format!(
            "{label}-assistant-{index} {}",
            "historical result ".repeat(220)
        )),
    ]
}

async fn append_turns(session: &SessionMemory, label: &str, count: usize) {
    let mut messages = Vec::with_capacity(count * 2);
    for index in 0..count {
        messages.extend(long_turn(label, index));
    }
    ConversationMemory::append(session.inner(), session.session_id(), messages)
        .await
        .expect("append history");
}

fn compact_budget() -> BudgetConfig {
    BudgetConfig::new(9000, 128).with_compact(80, 2)
}

#[tokio::test]
async fn soft_threshold_skips_compaction_and_summary_watermark_is_reused() {
    let (base, fixture) = spawn_compact_fixture("SUMMARY-RELIABILITY-TWO").await;
    let root = tempfile::tempdir().expect("temp root");
    let small = session(&base, root.path(), "soft-threshold");
    ConversationMemory::append(
        small.inner(),
        small.session_id(),
        vec![
            Message::user("short history"),
            Message::assistant("short reply"),
        ],
    )
    .await
    .expect("append short history");
    let small_prompt = Message::user("short pending prompt");
    small
        .begin_run(&small_prompt)
        .await
        .expect("begin short run");
    let loaded = small
        .load_history(&small_prompt, "preamble", &[], BudgetConfig::new(8192, 256))
        .await
        .expect("under soft threshold");
    assert!(!loaded.compacted);
    assert!(fixture.requests.lock().expect("requests").is_empty());

    let compacted = session(&base, root.path(), "summary-watermark");
    append_turns(&compacted, "summary", 8).await;
    let prompt = Message::user("summary pending prompt");
    compacted
        .begin_run(&prompt)
        .await
        .expect("begin summary run");
    compacted
        .load_history(&prompt, "preamble", &[], compact_budget())
        .await
        .expect("compact summary");
    let calls_after_compact = fixture.requests.lock().expect("requests").len();
    assert_eq!(calls_after_compact, 1);
    compacted
        .load_history(&prompt, "preamble", &[], compact_budget())
        .await
        .expect("reuse summary");
    assert_eq!(
        fixture.requests.lock().expect("requests").len(),
        calls_after_compact,
        "the same demoted prefix must not invoke the LLM again"
    );
}

#[tokio::test]
async fn compact_recent_turns_keeps_the_requested_user_suffix() {
    let (base, fixture) = spawn_compact_fixture("SUMMARY-RELIABILITY-THREE").await;
    let root = tempfile::tempdir().expect("temp root");
    let session = session(&base, root.path(), "recent-turns");
    append_turns(&session, "recent", 8).await;
    let prompt = Message::user("recent pending prompt");
    session.begin_run(&prompt).await.expect("begin recent run");
    let loaded = session
        .load_history(
            &prompt,
            "preamble",
            &[],
            BudgetConfig::new(8192, 256).with_compact(80, 2),
        )
        .await
        .expect("recent-turn compaction");
    assert!(loaded.compacted);
    let dump = serde_json::to_string(&loaded.messages).expect("serialize loaded");
    assert!(dump.contains("SUMMARY-RELIABILITY-THREE"), "{dump}");
    assert!(
        dump.contains("recent-user-6"),
        "last two user turns must remain: {dump}"
    );
    assert!(
        dump.contains("recent-user-7"),
        "last user turn must remain: {dump}"
    );
    assert!(
        !dump.contains("recent-user-0"),
        "old user turn should be summarized: {dump}"
    );
    assert_eq!(fixture.requests.lock().expect("requests").len(), 1);
}

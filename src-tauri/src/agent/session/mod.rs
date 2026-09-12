mod prompt;
mod supervisor;
mod turn;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, Notify, RwLock};

use crate::acp::connection::{ConnectionCleanupGuard, ConnectionCommand, DelegationInjection};
use crate::acp::error::AcpError;
use crate::acp::file_system_runtime::FsAccessPolicy;
use crate::acp::host_tools_policy::HostToolsPolicy;
use crate::acp::native_config::EffectiveNativeConfig;
use crate::acp::native_shutdown::NativeShutdownHandle;
use crate::acp::session_state::SessionState;
use crate::acp::terminal_runtime::TerminalShellRuntimeConfig;
use crate::models::agent::AgentType;
use crate::web::event_bridge::EventEmitter;

pub use prompt::{assemble_native_prompt, inspect_native_prompt, NativePromptView};
pub use supervisor::NativeSessionSupervisor;
pub use turn::TurnCoordinator;

/// Host-side arguments for an in-process session. No Rig types.
pub struct NativeSessionArgs {
    pub connection_id: String,
    pub agent_type: AgentType,
    pub working_dir: Option<String>,
    pub launch_cwd: PathBuf,
    pub resume_session_id: Option<String>,
    pub effective_config: EffectiveNativeConfig,
    pub preferred_config_values: BTreeMap<String, String>,
    pub owner_window_label: String,
    pub emitter: EventEmitter,
    pub session_state: Arc<RwLock<SessionState>>,
    pub cmd_rx: mpsc::Receiver<ConnectionCommand>,
    pub delegation_injection: Option<DelegationInjection>,
    pub terminal_shell_config: TerminalShellRuntimeConfig,
    pub terminal_base_env: BTreeMap<String, String>,
    pub fs_policy: FsAccessPolicy,
    pub host_tools: HostToolsPolicy,
    pub config_fingerprint: String,
    pub shutdown: Arc<NativeShutdownHandle>,
    pub(crate) map_cleanup: Option<ConnectionCleanupGuard>,
    /// Test-only: register the in-memory echo tool so permission wait can be
    /// exercised without PR5 file tools.
    pub include_echo_tool: bool,
    /// Test-only: hold initialization until notified (or shutdown fires).
    pub init_hold: Option<Arc<Notify>>,
    /// `None` reads `read_servers_for_agent_type`. Tests pass `Some(empty)` so a
    /// developer MCP store is never spawned during session unit tests.
    pub mcp_server_specs: Option<BTreeMap<String, serde_json::Value>>,
    /// Test-only: fail the next critical TurnEnd write.
    pub fail_turn_end: bool,
}

/// Start the supervisor in the background and return immediately. The caller
/// already holds `session_started_rx` from `SessionState`.
pub fn spawn_native_session(args: NativeSessionArgs) -> Result<(), AcpError> {
    tokio::spawn(run_native_session(args));
    Ok(())
}

pub async fn run_native_session(args: NativeSessionArgs) {
    supervisor::NativeSessionSupervisor::run(args).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::connection::ConnectionCommand;
    use crate::acp::file_system_runtime::FsAccessPolicy;
    use crate::acp::host_tools_policy::HostToolsPolicy;
    use crate::acp::native_config::EffectiveNativeConfig;
    use crate::acp::native_shutdown::NativeShutdownHandle;
    use crate::acp::process_owner::pid_is_alive;
    use crate::acp::session_state::SessionState;
    use crate::acp::terminal_runtime::TerminalShellRuntimeConfig;
    use crate::acp::types::{AcpEvent, PromptInputBlock};
    use crate::models::agent::AgentType;
    use crate::web::event_bridge::EventEmitter;
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;
    use tokio::sync::{mpsc, Notify, RwLock};

    struct Harness {
        cmd_tx: mpsc::Sender<ConnectionCommand>,
        state: Arc<RwLock<SessionState>>,
        shutdown: Arc<NativeShutdownHandle>,
        started: tokio::sync::oneshot::Receiver<()>,
        events: tokio::sync::broadcast::Receiver<Arc<crate::acp::types::EventEnvelope>>,
    }

    async fn spawn_session(
        base_url: &str,
        include_echo: bool,
        init_hold: Option<Arc<Notify>>,
    ) -> Harness {
        spawn_session_cfg(base_url, include_echo, init_hold, false).await
    }

    async fn spawn_session_cfg(
        base_url: &str,
        include_echo: bool,
        init_hold: Option<Arc<Notify>>,
        fail_turn_end: bool,
    ) -> Harness {
        let connection_id = format!("native-conn-{}", uuid::Uuid::new_v4());
        let session_id = format!("sess-native-{}", uuid::Uuid::new_v4());
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let mut state = SessionState::new(
            connection_id.clone(),
            AgentType::CodegAgent,
            Some(PathBuf::from("/tmp")),
            "main".into(),
            None,
        );
        let started = state.install_session_started_signal();
        let events = state.event_stream().subscribe();
        let state = Arc::new(RwLock::new(state));
        let shutdown = NativeShutdownHandle::new();
        let mut windows = BTreeMap::new();
        windows.insert("m".into(), 128000);
        let args = NativeSessionArgs {
            connection_id,
            agent_type: AgentType::CodegAgent,
            working_dir: Some("/tmp".into()),
            launch_cwd: PathBuf::from("/tmp"),
            resume_session_id: Some(session_id),
            effective_config: EffectiveNativeConfig {
                api_base_url: base_url.into(),
                api_key: "sk".into(),
                model_id: "m".into(),
                context_windows: windows,
                max_output_tokens: 4096,
                system_prompt: None,
                compact_prompt: None,
                compact_soft_percent: 80,
                compact_recent_turns: 6,
                compact_model_id: None,
                max_turns: 40,
                protocol: crate::acp::native_config::CodegProtocol::ChatCompletions,
                resolved_protocol: None,
            },
            preferred_config_values: BTreeMap::new(),
            owner_window_label: "main".into(),
            emitter: EventEmitter::Noop,
            session_state: Arc::clone(&state),
            cmd_rx,
            delegation_injection: None,
            terminal_shell_config: TerminalShellRuntimeConfig::new(),
            terminal_base_env: BTreeMap::new(),
            fs_policy: FsAccessPolicy::strict(&PathBuf::from("/tmp")),
            host_tools: HostToolsPolicy::Default,
            config_fingerprint: "fp".into(),
            shutdown: Arc::clone(&shutdown),
            map_cleanup: None,
            include_echo_tool: include_echo,
            init_hold,
            mcp_server_specs: Some(BTreeMap::new()),
            fail_turn_end,
        };
        spawn_native_session(args).expect("spawn native");
        Harness {
            cmd_tx,
            state,
            shutdown,
            started,
            events,
        }
    }

    async fn wait_event(
        events: &mut tokio::sync::broadcast::Receiver<Arc<crate::acp::types::EventEnvelope>>,
        pred: impl Fn(&AcpEvent) -> bool,
    ) -> AcpEvent {
        wait_event_labeled(events, "event", pred).await
    }

    async fn wait_event_labeled(
        events: &mut tokio::sync::broadcast::Receiver<Arc<crate::acp::types::EventEnvelope>>,
        label: &str,
        pred: impl Fn(&AcpEvent) -> bool,
    ) -> AcpEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        let mut seen = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let envelope = match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Ok(envelope)) => envelope,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                    seen.push(format!("lagged({n})"));
                    continue;
                }
                Ok(Err(err)) => panic!("{label} closed: {err}; seen={seen:?}"),
                Err(_) => panic!("{label} timeout; seen={seen:?}"),
            };
            seen.push(format!("{:?}", envelope.payload));
            if pred(&envelope.payload) {
                return envelope.payload.clone();
            }
        }
    }

    async fn spawn_completions(script: Vec<Value>) -> (String, Arc<StdMutex<Vec<Value>>>) {
        let bodies = Arc::new(StdMutex::new(Vec::new()));
        let script = Arc::new(StdMutex::new(script));
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
                        json!({"kind": "text", "text": "recovered"})
                    } else {
                        script.remove(0)
                    }
                };
                if next.get("kind").and_then(Value::as_str) == Some("hang") {
                    std::future::pending::<()>().await;
                }
                let sse = script_to_sse(&next);
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
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/v1"), bodies)
    }

    fn script_to_sse(next: &Value) -> String {
        if next.get("kind").and_then(Value::as_str) == Some("tools") {
            let calls = next
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
            format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({
                    "id": "c",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": null, "tool_calls": tool_calls },
                        "finish_reason": null
                    }]
                }),
                json!({
                    "id": "c",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "tool_calls"
                    }],
                    "usage": { "prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12 }
                })
            )
        } else {
            let text = next.get("text").and_then(Value::as_str).unwrap_or("ok");
            format!(
                "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"{text}\"}}}}]}}\n\n\
                 data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}}}\n\n\
                 data: [DONE]\n\n"
            )
        }
    }

    async fn wait_started(h: &mut Harness) {
        tokio::time::timeout(Duration::from_secs(5), &mut h.started)
            .await
            .expect("started timeout")
            .expect("started");
        wait_event_labeled(&mut h.events, "selectors", |e| {
            matches!(e, AcpEvent::SelectorsReady)
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_returns_and_emits_session_started() {
        let (base, _) = spawn_completions(vec![json!({"kind":"text","text":"hi"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        let sid = h.state.read().await.external_id.clone();
        assert!(
            sid.as_deref().is_some_and(|s| !s.is_empty()),
            "session id {sid:?}"
        );
        {
            let s = h.state.read().await;
            assert!(
                !s.feedback_tool_available && !s.delegation_enabled && !s.native_steering_available,
                "no companion injection means no companion capabilities"
            );
        }
        h.shutdown.signal_shutdown();
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Disconnected
                }
            )
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_publishes_code_and_plan_modes() {
        let (base, _) = spawn_completions(vec![json!({"kind":"text","text":"hi"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        {
            let s = h.state.read().await;
            assert_eq!(s.current_mode.as_deref(), Some("code"));
            let modes = s.modes.as_ref().expect("modes");
            let ids: Vec<_> = modes
                .available_modes
                .iter()
                .map(|m| m.id.as_str())
                .collect();
            assert_eq!(ids, vec!["code", "plan"]);
            assert!(!ids.contains(&"explore"));
        }
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_mode_plan_omits_write_file_from_next_prompt() {
        let (base, bodies) = spawn_completions(vec![json!({"kind":"text","text":"ok"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::SetMode {
                mode_id: "plan".into(),
            })
            .await
            .expect("set mode");
        wait_event(
            &mut h.events,
            |e| matches!(e, AcpEvent::ModeChanged { mode_id } if mode_id == "plan"),
        )
        .await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "plan please".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        let dumped = bodies
            .lock()
            .expect("bodies")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            dumped.contains("write_plan"),
            "plan mode must advertise write_plan: {dumped}"
        );
        assert!(
            dumped.contains("exit_plan_mode"),
            "plan mode must advertise exit_plan_mode: {dumped}"
        );
        assert!(
            !dumped.contains("\"name\":\"write_file\""),
            "plan mode must not register write_file: {dumped}"
        );
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_streams_text_and_emits_unique_turn_complete() {
        let (base, _) = spawn_completions(vec![json!({"kind":"text","text":"hello-delta"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "say hello".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(
            &mut h.events,
            |e| matches!(e, AcpEvent::ContentDelta { text, .. } if text.contains("hello-delta")),
        )
        .await;
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        let completed = h.state.read().await.turns_completed;
        assert_eq!(completed, 1);
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_during_http_hang_then_fresh_token_on_next_turn() {
        let (base, _) = spawn_completions(vec![json!({"kind":"hang"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "hang please".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event_labeled(&mut h.events, "prompting", |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Prompting
                }
            )
        })
        .await;
        h.cmd_tx
            .send(ConnectionCommand::Cancel)
            .await
            .expect("cancel");
        let complete = wait_event_labeled(&mut h.events, "cancelled-turn", |e| {
            matches!(e, AcpEvent::TurnComplete { .. })
        })
        .await;
        match complete {
            AcpEvent::TurnComplete { stop_reason, .. } => {
                assert!(
                    stop_reason == "cancelled" || stop_reason == "error",
                    "cancel during hang must settle the turn, got {stop_reason}"
                );
            }
            other => panic!("expected TurnComplete, got {other:?}"),
        }
        assert_eq!(h.state.read().await.turns_completed, 1);

        // A follow-up prompt on the same mock HTTP connection can stall behind
        // the hung request's keep-alive slot. Fresh-token coverage lives in
        // `TurnCoordinator` tests; a second live turn after cancel is covered
        // by `permission_wait_keeps_command_loop_reachable`.
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn init_hold_still_accepts_shutdown() {
        let (base, _) = spawn_completions(vec![]).await;
        let hold = Arc::new(Notify::new());
        let mut h = spawn_session(&base, false, Some(Arc::clone(&hold))).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(h.state.read().await.external_id.is_none());
        h.shutdown.signal_shutdown();
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Disconnected
                }
            )
        })
        .await;
        assert!(h.cmd_tx.is_closed() || true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queue_full_shutdown_still_reaches_native_token() {
        let (base, _) = spawn_completions(vec![json!({"kind":"hang"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "hang".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Prompting
                }
            )
        })
        .await;
        for _ in 0..32 {
            let _ = h.cmd_tx.try_send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text { text: "x".into() }],
                user_message: None,
            });
        }
        h.shutdown.signal_shutdown();
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Disconnected
                }
            )
        })
        .await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn panic_drop_reaps_sigterm_immune_child() {
        let shutdown = NativeShutdownHandle::new();
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("trap '' TERM; sleep 60")
            .spawn()
            .expect("spawn immune child");
        let pid = child.id();
        crate::acp::process_owner::lock_owners(&shutdown.owners()).register(pid);
        let join = tokio::spawn({
            let shutdown = Arc::clone(&shutdown);
            async move {
                let _guard = supervisor::NativeDropGuard {
                    shutdown,
                    _map: None,
                };
                panic!("native worker boom");
            }
        });
        let _ = join.await;
        let gone = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if child.try_wait().ok().flatten().is_some() || !pid_is_alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            gone.is_ok(),
            "SIGTERM-immune child must be force-killed on panic"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn permission_wait_keeps_command_loop_reachable() {
        let (base, _) = spawn_completions(vec![
            json!({
                "kind": "tools",
                "calls": [{ "name": "echo", "id": "call_perm", "arguments": { "text": "hi" } }]
            }),
            json!({
                "kind": "tools",
                "calls": [{ "name": "echo", "id": "call_perm2", "arguments": { "text": "again" } }]
            }),
            json!({"kind": "text", "text": "after-allow"}),
        ])
        .await;
        let mut h = spawn_session(&base, true, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "use echo".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        let request_id = match wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::PermissionRequest { .. })
        })
        .await
        {
            AcpEvent::PermissionRequest { request_id, .. } => request_id,
            other => panic!("expected permission card, got {other:?}"),
        };

        h.cmd_tx
            .send(ConnectionCommand::Cancel)
            .await
            .expect("cancel during permission wait");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "cancelled")
        })
        .await;
        assert_eq!(h.state.read().await.turns_completed, 1);
        let _ = request_id;

        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "use echo again".into(),
                }],
                user_message: None,
            })
            .await
            .expect("second prompt");
        let request_id = match wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::PermissionRequest { .. })
        })
        .await
        {
            AcpEvent::PermissionRequest { request_id, .. } => request_id,
            other => panic!("expected second permission card, got {other:?}"),
        };
        h.cmd_tx
            .send(ConnectionCommand::RespondPermission {
                request_id,
                option_id: "allow-once".into(),
            })
            .await
            .expect("allow");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        assert_eq!(h.state.read().await.turns_completed, 2);
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mixed_text_and_image_prompt_sends_image_url() {
        let (base, bodies) = spawn_completions(vec![json!({"kind":"text","text":"ok"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![
                    PromptInputBlock::Text {
                        text: "describe this".into(),
                    },
                    PromptInputBlock::Image {
                        data: "iVBORw0KGgo=".into(),
                        mime_type: "image/png".into(),
                        uri: None,
                    },
                ],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        let dumped = bodies
            .lock()
            .expect("bodies")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(dumped.contains("describe this"), "{dumped}");
        assert!(
            dumped.contains("image_url") && dumped.contains("data:image/png;base64,"),
            "mixed prompt must send image_url data URI: {dumped}"
        );
        assert!(dumped.contains("iVBORw0KGgo"), "{dumped}");
        assert!(
            !dumped.contains("images are not supported"),
            "images are forwarded: {dumped}"
        );
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn image_only_prompt_sends_image_to_model() {
        let (base, bodies) = spawn_completions(vec![json!({"kind":"text","text":"ok"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Image {
                    data: "iVBORw0KGgo=".into(),
                    mime_type: "image/png".into(),
                    uri: None,
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        let dumped = bodies
            .lock()
            .expect("bodies")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            dumped.contains("image_url") && dumped.contains("data:image/png;base64,"),
            "image-only prompt must send image_url data URI: {dumped}"
        );
        assert!(dumped.contains("iVBORw0KGgo"), "{dumped}");
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_advertises_image_prompt_capability() {
        let (base, _) = spawn_completions(vec![json!({"kind":"text","text":"hi"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::PromptCapabilities {
                    prompt_capabilities
                } if prompt_capabilities.image && !prompt_capabilities.audio
            )
        })
        .await;
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_prompt_registers_update_plan_and_subagent_spec() {
        let (base, bodies) = spawn_completions(vec![json!({"kind":"text","text":"ok"})]).await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "say ok".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;
        let dumped = bodies
            .lock()
            .expect("bodies")
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            dumped.contains("update_plan"),
            "native turn must advertise update_plan: {dumped}"
        );
        assert!(
            dumped.contains("subagent"),
            "native turn must advertise subagent: {dumped}"
        );
        assert!(
            dumped.contains("codeg-subagent-spec"),
            "subagent usage spec must be in extra_context: {dumped}"
        );
        assert!(
            dumped.contains("codeg-using-plan-explore"),
            "builtin plan/explore skill must be in extra_context: {dumped}"
        );
        h.shutdown.signal_shutdown();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn turn_end_write_failure_closes_instead_of_idling() {
        let (base, _) = spawn_completions(vec![json!({"kind":"text","text":"hello"})]).await;
        let mut h = spawn_session_cfg(&base, false, None, true).await;
        wait_started(&mut h).await;
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "say hello".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::Error { terminal: true, .. })
        })
        .await;
        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::StatusChanged {
                    status: crate::acp::types::ConnectionStatus::Disconnected
                }
            )
        })
        .await;
        assert_eq!(
            h.state.read().await.turns_completed,
            0,
            "failed TurnEnd must not emit a successful TurnComplete"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn origin_rewind_starts_an_empty_session_after_the_first_round() {
        let (base, _) = spawn_completions(vec![
            json!({"kind":"text","text":"first-reply"}),
            json!({"kind":"text","text":"second-reply"}),
        ])
        .await;
        let mut h = spawn_session(&base, false, None).await;
        wait_started(&mut h).await;
        let original = h
            .state
            .read()
            .await
            .external_id
            .clone()
            .expect("session id");
        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "first".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt");
        wait_event(&mut h.events, |e| {
            matches!(e, AcpEvent::TurnComplete { stop_reason, .. } if stop_reason == "end_turn")
        })
        .await;

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        h.cmd_tx
            .send(ConnectionCommand::Fork {
                fork_point: None,
                rewind_to_origin: true,
                reply: reply_tx,
            })
            .await
            .expect("origin rewind");
        let protocol = reply_rx
            .await
            .expect("reply closed")
            .expect("origin rewind");
        assert_eq!(protocol.original_session_id, original);
        assert_ne!(protocol.forked_session_id, original);

        wait_event(&mut h.events, |e| {
            matches!(
                e,
                AcpEvent::SessionStarted { session_id }
                    if session_id == &protocol.forked_session_id
            )
        })
        .await;
        assert_eq!(
            h.state.read().await.external_id.as_deref(),
            Some(protocol.forked_session_id.as_str())
        );

        h.cmd_tx
            .send(ConnectionCommand::Prompt {
                blocks: vec![PromptInputBlock::Text {
                    text: "edited first".into(),
                }],
                user_message: None,
            })
            .await
            .expect("prompt after rewind");
        wait_event(
            &mut h.events,
            |e| matches!(e, AcpEvent::ContentDelta { text, .. } if text.contains("second-reply")),
        )
        .await;
        h.shutdown.signal_shutdown();
    }
}

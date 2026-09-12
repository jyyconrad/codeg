//! Command ring: `cmd_rx` + runner stream + shutdown select.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use super::prompt::native_prompt_store_text;
use super::{assemble_native_prompt, inspect_native_prompt, NativeSessionArgs, TurnCoordinator};
use crate::acp::agent_mentions::append_agent_routes;
use crate::acp::connection::{snapshot_companion_features, ConnectionCommand, DelegationInjection};
use crate::acp::file_system_runtime::FileSystemRuntime;
use crate::acp::native_config::{effective_compact_prompt, EffectiveNativeConfig};
use crate::acp::native_shutdown::{NativeCleanupState, NativeShutdownHandle};
use crate::acp::session_state::SessionState;
use crate::acp::terminal_runtime::TerminalRuntime;
use crate::acp::types::{
    AcpEvent, ConnectionStatus, PermissionOptionInfo, PromptCapabilitiesInfo, PromptInputBlock,
    SessionConfigKindInfo, SessionConfigOptionInfo, SessionConfigSelectInfo,
    SessionConfigSelectOptionInfo, UserMessageBlock,
};
use crate::acp_transcript::{now_epoch_ms, record_header_critical_in, TranscriptHeader};
use crate::agent::context::transcript::tool_call_update_payload;
use crate::agent::context::{
    agent_message_chunk, open_codeg_agent_session, BudgetConfig, CallIdentityBridge, ContextStore,
    FactRecorder, HydrateError, LlmCompactor, NativeMeta, ToolOutcome, L2_MAX_TOKENS,
};
use crate::agent::hook::{CodegHook, HookTrace, HostBridge, NativeRunState, PendingPermission};
use crate::agent::mode::{self, MODE_PLAN};
use crate::agent::model::{
    resolve_session_wire_protocol, run_native_turn, session_preamble, CodegLlmClient,
    NativeTurnOutcome, NativeTurnRequest, NativeTurnTools,
};
use crate::agent::tools::{
    build_companion_tools, companion_plan_from_injection, schema_for, schema_for_companion_def,
    tool_kind, BashTool, CompanionPlan, CompanionRuntime, EchoTool, EditFileTool,
    EnterPlanModeTool, ExitPlanModeTool, FeedbackDelivery, GlobTool, GrepTool, McpSession,
    McpTimeouts, NativeInject, NativeToolCtx, ReadFileTool, RecallTool, SkillCatalog, SkillTool,
    SubagentTable, SubagentTool, UpdatePlanTool, WriteFileTool, WritePlanTool,
};
use crate::web::event_bridge::{emit_with_state, EventEmitter};

const HEADER_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const WORKER_CANCEL_WAIT: Duration = Duration::from_secs(5);
const NATIVE_FORCE_REAP: Duration = Duration::from_secs(2);
const TURN_END_WRITE_FAILED: &str = "failed to confirm transcript turn end";
const TURN_DID_NOT_CONVERGE: &str = "turn did not converge after cancel";

fn companion_ok_in_mode(mode: &str, name: &str) -> bool {
    if mode != MODE_PLAN {
        return true;
    }
    matches!(
        name,
        "ask_user_question" | "get_session_info" | "check_user_feedback"
    )
}

pub struct NativeSessionSupervisor;

impl NativeSessionSupervisor {
    pub async fn run(mut args: NativeSessionArgs) {
        let map_cleanup = args.map_cleanup.take();
        let shutdown = Arc::clone(&args.shutdown);
        let _panic_guard = NativeDropGuard {
            shutdown: Arc::clone(&shutdown),
            _map: map_cleanup,
        };
        let emitter = args.emitter.clone();
        let state = Arc::clone(&args.session_state);
        let agent_type = args.agent_type;
        let terminals = Arc::new(
            TerminalRuntime::with_base_env(args.terminal_base_env.clone())
                .with_default_cwd(Some(args.launch_cwd.clone()))
                .with_default_shell_config(args.terminal_shell_config.clone())
                .with_process_owners(shutdown.owners()),
        );
        let outcome = run_session(&mut args, Arc::clone(&terminals)).await;

        let leftover = terminals
            .force_kill_all_and_wait_reaped(NATIVE_FORCE_REAP)
            .await;
        let leftover2 = shutdown.force_kill_owners(NATIVE_FORCE_REAP).await;
        let failed = !leftover.is_empty() || !leftover2.is_empty();
        if failed {
            tracing::error!(
                "[ACP] native cleanup_failed terminals={leftover:?} owners={leftover2:?}"
            );
            shutdown.mark_failed();
        } else if matches!(shutdown.cleanup_state(), NativeCleanupState::Pending) {
            shutdown.mark_complete();
        }

        if let Some(message) = outcome.err {
            emit_with_state(
                &state,
                &emitter,
                AcpEvent::Error {
                    message,
                    agent_type: agent_type.to_string(),
                    code: None,
                    details: None,
                    terminal: true,
                },
            )
            .await;
            emit_with_state(
                &state,
                &emitter,
                AcpEvent::StatusChanged {
                    status: ConnectionStatus::Error,
                },
            )
            .await;
        }
        emit_with_state(
            &state,
            &emitter,
            AcpEvent::StatusChanged {
                status: ConnectionStatus::Disconnected,
            },
        )
        .await;
    }
}

struct SessionOutcome {
    err: Option<String>,
}

async fn run_session(
    args: &mut NativeSessionArgs,
    terminals: Arc<TerminalRuntime>,
) -> SessionOutcome {
    let shutdown = Arc::clone(&args.shutdown);
    if let Some(hold) = args.init_hold.clone() {
        tokio::select! {
            _ = shutdown.cancelled() => {
                return SessionOutcome { err: Some("native session cancelled during init".into()) };
            }
            _ = hold.notified() => {}
        }
    }

    let mut model_id =
        match resolve_session_model(&args.effective_config, &args.preferred_config_values) {
            Ok(id) => id,
            Err(message) => return SessionOutcome { err: Some(message) },
        };

    let companion = if let Some(inj) = args.delegation_injection.as_ref() {
        let flags = snapshot_companion_features(
            inj,
            args.host_tools,
            args.owner_window_label == "work_task",
        )
        .await;
        companion_plan_from_injection(Some(inj), flags).await
    } else {
        CompanionPlan::empty()
    };
    write_capability_snapshot(&args.session_state, &companion).await;
    let feedback_delivery = args.delegation_injection.as_ref().map(|inj| {
        Arc::new(FeedbackDelivery::new(
            Arc::clone(&inj.feedback_access),
            args.connection_id.clone(),
        ))
    });

    let sessions_root = crate::paths::codeg_agent_sessions_root();
    let fallback_root = crate::paths::codeg_acp_transcripts_root();
    let opened = match open_codeg_agent_session(
        &sessions_root,
        Some(&fallback_root),
        &args.launch_cwd.to_string_lossy(),
        args.agent_type.as_wire().as_ref(),
        args.resume_session_id.as_deref(),
    ) {
        Ok(opened) => opened,
        Err(HydrateError::LoadFailed {
            session_id,
            message,
            code,
        }) => {
            emit_with_state(
                &args.session_state,
                &args.emitter,
                AcpEvent::SessionLoadFailed {
                    session_id,
                    message: message.clone(),
                    code: code.to_string(),
                },
            )
            .await;
            return SessionOutcome { err: Some(message) };
        }
    };
    let mut session_id = opened.session_id.clone();
    let write_root = opened.write_root.clone();
    let write_group = opened.write_group.clone();
    let mut _lease = opened.lease;
    let store = Arc::new(Mutex::new(opened.store));
    let mut recorder = Arc::new(FactRecorder::transcript(
        write_root.clone(),
        write_group.clone(),
        session_id.clone(),
        Arc::clone(&store),
    ));
    if args.fail_turn_end {
        recorder.fail_next_turn_end();
    }
    if opened.header.is_none() {
        let mut header = TranscriptHeader::new(
            args.agent_type.as_wire().as_ref(),
            &session_id,
            &args.launch_cwd.to_string_lossy(),
            now_epoch_ms(),
        );
        if let Some(previous) = opened.continues_from.as_deref() {
            header = header.continuing(previous);
        }
        let ack = record_header_critical_in(&write_root, &write_group, &header);
        match tokio::time::timeout(HEADER_ACK_TIMEOUT, ack).await {
            Ok(Ok(())) => {}
            _ => {
                return SessionOutcome {
                    err: Some("failed to confirm transcript header".into()),
                };
            }
        }
    }

    let mut artifacts_dir =
        crate::agent::mode::artifacts_dir(&args.launch_cwd.to_string_lossy(), &session_id);
    let _ = std::fs::create_dir_all(&artifacts_dir);
    let initial_mode = crate::agent::mode::load_persisted_mode(&artifacts_dir);
    let session_mode = Arc::new(tokio::sync::RwLock::new(initial_mode.clone()));
    let pending_continue = Arc::new(Mutex::new(None::<String>));
    let fs = Arc::new(FileSystemRuntime::with_policy(
        args.fs_policy.clone().with_extra_read_root(&artifacts_dir),
    ));

    let mcp = match args.mcp_server_specs.clone() {
        Some(specs) => {
            McpSession::connect_specs(
                specs,
                Some(shutdown.owners()),
                shutdown.token(),
                McpTimeouts::default(),
            )
            .await
        }
        None => {
            McpSession::connect_for_agent(
                args.agent_type,
                Some(shutdown.owners()),
                shutdown.token(),
                McpTimeouts::default(),
            )
            .await
        }
    };
    let mcp = Arc::new(mcp);
    if shutdown.is_cancelled() {
        mcp.close().await;
        return SessionOutcome {
            err: Some("native session cancelled during MCP init".into()),
        };
    }

    emit_with_state(
        &args.session_state,
        &args.emitter,
        AcpEvent::SessionStarted {
            session_id: session_id.clone(),
        },
    )
    .await;
    emit_with_state(
        &args.session_state,
        &args.emitter,
        AcpEvent::StatusChanged {
            status: ConnectionStatus::Connected,
        },
    )
    .await;
    emit_session_config(
        &args.session_state,
        &args.emitter,
        &args.effective_config,
        &model_id,
    )
    .await;
    emit_with_state(
        &args.session_state,
        &args.emitter,
        AcpEvent::PromptCapabilities {
            prompt_capabilities: PromptCapabilitiesInfo {
                image: true,
                audio: false,
                embedded_context: false,
            },
        },
    )
    .await;
    emit_with_state(
        &args.session_state,
        &args.emitter,
        AcpEvent::ForkSupported { supported: false },
    )
    .await;
    crate::agent::mode::emit_modes(&args.session_state, &args.emitter, &initial_mode).await;
    emit_with_state(&args.session_state, &args.emitter, AcpEvent::SelectorsReady).await;

    let wire = resolve_session_wire_protocol(&args.effective_config).await;
    let client = match CodegLlmClient::build(
        args.effective_config.api_key.clone(),
        &args.effective_config.api_base_url,
        wire,
    ) {
        Ok(client) => client,
        Err(err) => {
            return SessionOutcome {
                err: Some(err.to_string()),
            }
        }
    };
    let workspace = args
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            args.launch_cwd
                .to_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
    let _ = crate::agent::builtin_skills::ensure_installed();
    let catalog = SkillCatalog::load(args.agent_type, workspace.as_deref());
    let preamble = session_preamble(
        &args.launch_cwd,
        catalog.preamble_section().as_deref(),
        args.effective_config.system_prompt.as_deref(),
    );
    let coordinator = Arc::new(TurnCoordinator::new());
    let mut cmd_rx = std::mem::replace(&mut args.cmd_rx, mpsc::channel(1).1);
    let (inject_tx, mut inject_rx) = mpsc::channel::<NativeInject>(8);
    let subagents = Arc::new(Mutex::new(SubagentTable::default()));
    let mut pending_injects: VecDeque<String> = VecDeque::new();

    let mut running: Option<RunningTurn> = None;
    let mut closing_err: Option<String> = None;
    loop {
        if shutdown.is_cancelled() {
            shutdown_subagents(&subagents, &mut inject_rx, &mut pending_injects);
            if let SettleOutcome::Fatal(message) = settle_running(
                running.take(),
                &coordinator,
                &args.session_state,
                &args.emitter,
                &session_id,
                args.agent_type.to_string(),
                "cancelled",
                args.delegation_injection.as_ref(),
                &args.connection_id,
                &terminals,
                &store,
            )
            .await
            {
                closing_err = Some(message);
            }
            break;
        }

        if let Some(active) = running.as_mut() {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    shutdown_subagents(&subagents, &mut inject_rx, &mut pending_injects);
                    let taken = running.take();
                    if let SettleOutcome::Fatal(message) = settle_running(
                        taken,
                        &coordinator,
                        &args.session_state,
                        &args.emitter,
                        &session_id,
                        args.agent_type.to_string(),
                        "cancelled",
                        args.delegation_injection.as_ref(),
                        &args.connection_id,
                        &terminals,
                        &store,
                    )
                    .await
                    {
                        closing_err = Some(message);
                    }
                    break;
                }
                inject = inject_rx.recv() => {
                    if let Some(inject) = inject {
                        let text = apply_subagent_finished(
                            inject,
                            &subagents,
                            &recorder,
                            &args.session_state,
                            &args.emitter,
                        )
                        .await;
                        pending_injects.push_back(text);
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        None | Some(ConnectionCommand::Disconnect) => {
                            shutdown_subagents(&subagents, &mut inject_rx, &mut pending_injects);
                            let taken = running.take();
                            if let SettleOutcome::Fatal(message) = settle_running(
                                taken,
                                &coordinator,
                                &args.session_state,
                                &args.emitter,
                                &session_id,
                                args.agent_type.to_string(),
                                "cancelled",
                                args.delegation_injection.as_ref(),
                                &args.connection_id,
                                &terminals,
                                &store,
                            )
                            .await
                            {
                                closing_err = Some(message);
                            }
                            break;
                        }
                        Some(ConnectionCommand::Cancel) => {
                            cancel_turn_subagents(&subagents, &mut inject_rx, &mut pending_injects);
                            let taken = running.take();
                            if let SettleOutcome::Fatal(message) = settle_running(
                                taken,
                                &coordinator,
                                &args.session_state,
                                &args.emitter,
                                &session_id,
                                args.agent_type.to_string(),
                                "cancelled",
                                args.delegation_injection.as_ref(),
                                &args.connection_id,
                                &terminals,
                                &store,
                            )
                            .await
                            {
                                closing_err = Some(message);
                                break;
                            }
                        }
                        Some(ConnectionCommand::RespondPermission { request_id, option_id }) => {
                            if coordinator.resolve(&request_id, &option_id) {
                                emit_with_state(
                                    &args.session_state,
                                    &args.emitter,
                                    AcpEvent::PermissionResolved { request_id },
                                )
                                .await;
                            }
                        }
                        Some(ConnectionCommand::Fork { reply, .. }) => {
                            let _ = reply.send(Err(crate::acp::error::AcpError::protocol(
                                "Cannot rewind a session during a turn".to_string(),
                            )));
                        }
                        Some(other) => handle_control_command(
                            other,
                            true,
                            &mut model_id,
                            args,
                            &session_id,
                            &session_mode,
                            &artifacts_dir,
                        )
                        .await,
                    }
                }
                pending = active.permissions.recv() => {
                    if let Some(pending) = pending {
                        publish_permission(&args.session_state, &args.emitter, &coordinator, pending)
                            .await;
                    }
                }
                joined = &mut active.worker => {
                    let turn_id = active.turn_id;
                    let recorder = Arc::clone(&active.recorder);
                    running = None;
                    let stop = match joined {
                        Ok(NativeTurnOutcome::Complete) => "end_turn",
                        Ok(NativeTurnOutcome::Cancelled) => "cancelled",
                        Ok(NativeTurnOutcome::Failed(message)) => {
                            emit_with_state(
                                &args.session_state,
                                &args.emitter,
                                AcpEvent::Error {
                                    message,
                                    agent_type: args.agent_type.to_string(),
                                    code: None,
                                    details: None,
                                    terminal: false,
                                },
                            )
                            .await;
                            "error"
                        }
                        Err(_) => {
                            emit_with_state(
                                &args.session_state,
                                &args.emitter,
                                AcpEvent::Error {
                                    message: "native turn worker panicked".into(),
                                    agent_type: args.agent_type.to_string(),
                                    code: None,
                                    details: None,
                                    terminal: false,
                                },
                            )
                            .await;
                            "error"
                        }
                    };
                    if recorder.record_turn_end(stop).await.is_err() {
                        closing_err = Some(TURN_END_WRITE_FAILED.into());
                        break;
                    }
                    finish_turn(
                        &coordinator,
                        turn_id,
                        &args.session_state,
                        &args.emitter,
                        &session_id,
                        args.agent_type.to_string(),
                        stop,
                    )
                    .await;
                    if let Some(text) = pending_continue.lock().expect("pending continue").take() {
                        pending_injects.push_back(text);
                    }
                }
            }
        } else if let Some(text) = pending_injects.pop_front() {
            let (blocks, user_message) = inject_prompt(text);
            match start_prompt(
                &client,
                &preamble,
                &model_id,
                args,
                &session_id,
                blocks,
                user_message,
                Arc::clone(&coordinator),
                Arc::clone(&store),
                Arc::clone(&recorder),
                Arc::clone(&fs),
                Arc::clone(&terminals),
                catalog.clone(),
                companion.clone(),
                feedback_delivery.clone(),
                Arc::clone(&mcp),
                Arc::clone(&subagents),
                inject_tx.clone(),
                Arc::clone(&session_mode),
                artifacts_dir.clone(),
                Arc::clone(&pending_continue),
            )
            .await
            {
                Ok(turn) => running = Some(turn),
                Err(message) => {
                    emit_prompt_start_error(
                        &args.session_state,
                        &args.emitter,
                        args.agent_type.to_string(),
                        &session_id,
                        message,
                    )
                    .await;
                }
            }
        } else {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    shutdown_subagents(&subagents, &mut inject_rx, &mut pending_injects);
                    break;
                }
                inject = inject_rx.recv() => {
                    if let Some(inject) = inject {
                        let text = apply_subagent_finished(
                            inject,
                            &subagents,
                            &recorder,
                            &args.session_state,
                            &args.emitter,
                        )
                        .await;
                        pending_injects.push_back(text);
                    }
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        None | Some(ConnectionCommand::Disconnect) => {
                            shutdown_subagents(&subagents, &mut inject_rx, &mut pending_injects);
                            break;
                        }
                        Some(ConnectionCommand::Cancel) => {
                            // Idle cancel is a no-op besides draining leftover cards.
                            for request_id in coordinator.drain_cancelled() {
                                emit_with_state(
                                    &args.session_state,
                                    &args.emitter,
                                    AcpEvent::PermissionResolved { request_id },
                                )
                                .await;
                            }
                        }
                        Some(ConnectionCommand::Prompt { blocks, user_message }) => {
                            match start_prompt(
                                &client,
                                &preamble,
                                &model_id,
                                args,
                                &session_id,
                                blocks,
                                user_message,
                                Arc::clone(&coordinator),
                                Arc::clone(&store),
                                Arc::clone(&recorder),
                                Arc::clone(&fs),
                                Arc::clone(&terminals),
                                catalog.clone(),
                                companion.clone(),
                                feedback_delivery.clone(),
                                Arc::clone(&mcp),
                                Arc::clone(&subagents),
                                inject_tx.clone(),
                                Arc::clone(&session_mode),
                                artifacts_dir.clone(),
                                Arc::clone(&pending_continue),
                            )
                            .await
                            {
                                Ok(turn) => running = Some(turn),
                                Err(message) => {
                                    emit_prompt_start_error(
                                        &args.session_state,
                                        &args.emitter,
                                        args.agent_type.to_string(),
                                        &session_id,
                                        message,
                                    )
                                    .await;
                                }
                            }
                        }
                        Some(ConnectionCommand::RespondPermission { request_id, option_id }) => {
                            if coordinator.resolve(&request_id, &option_id) {
                                emit_with_state(
                                    &args.session_state,
                                    &args.emitter,
                                    AcpEvent::PermissionResolved { request_id },
                                )
                                .await;
                            }
                        }
                        Some(ConnectionCommand::Fork {
                            rewind_to_origin,
                            reply,
                            ..
                        }) => {
                            if !rewind_to_origin {
                                let _ = reply.send(Err(crate::acp::error::AcpError::protocol(
                                    "This agent does not support session/fork".to_string(),
                                )));
                                continue;
                            }
                            match rewind_native_to_origin(args, &session_id, &store).await
                            {
                                Ok(rewound) => {
                                    let original = session_id.clone();
                                    session_id = rewound.session_id;
                                    _lease = rewound.lease;
                                    recorder = Arc::new(rewound.recorder);
                                    artifacts_dir = rewound.artifacts_dir;
                                    emit_with_state(
                                        &args.session_state,
                                        &args.emitter,
                                        AcpEvent::SessionStarted {
                                            session_id: session_id.clone(),
                                        },
                                    )
                                    .await;
                                    let _ = reply.send(Ok(
                                        crate::acp::types::ForkProtocolResult {
                                            forked_session_id: session_id.clone(),
                                            original_session_id: original,
                                        },
                                    ));
                                }
                                Err(err) => {
                                    let _ = reply.send(Err(err));
                                }
                            }
                        }
                        Some(other) => handle_control_command(
                            other,
                            false,
                            &mut model_id,
                            args,
                            &session_id,
                            &session_mode,
                            &artifacts_dir,
                        )
                        .await,
                    }
                }
            }
        }
    }

    subagents.lock().expect("subagent table").shutdown();
    mcp.close().await;
    SessionOutcome { err: closing_err }
}

struct RunningTurn {
    turn_id: u64,
    turn_key: String,
    worker: tokio::task::JoinHandle<NativeTurnOutcome>,
    permissions: mpsc::Receiver<PendingPermission>,
    cancel: CancellationToken,
    recorder: Arc<FactRecorder>,
}

enum SettleOutcome {
    Settled,
    Fatal(String),
}

fn drain_injects(rx: &mut mpsc::Receiver<NativeInject>) {
    while rx.try_recv().is_ok() {}
}

fn shutdown_subagents(
    table: &Arc<Mutex<SubagentTable>>,
    inject_rx: &mut mpsc::Receiver<NativeInject>,
    pending_injects: &mut VecDeque<String>,
) {
    table.lock().expect("subagent table").shutdown();
    pending_injects.clear();
    drain_injects(inject_rx);
}

fn cancel_turn_subagents(
    table: &Arc<Mutex<SubagentTable>>,
    inject_rx: &mut mpsc::Receiver<NativeInject>,
    pending_injects: &mut VecDeque<String>,
) {
    table.lock().expect("subagent table").cancel_inflight();
    pending_injects.clear();
    drain_injects(inject_rx);
}

fn inject_prompt(
    text: String,
) -> (
    Vec<PromptInputBlock>,
    Option<(String, Vec<UserMessageBlock>)>,
) {
    (
        vec![PromptInputBlock::Text { text: text.clone() }],
        Some((
            uuid::Uuid::new_v4().to_string(),
            vec![UserMessageBlock::Text { text }],
        )),
    )
}

async fn emit_prompt_start_error(
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
    agent_type: String,
    session_id: &str,
    message: String,
) {
    emit_with_state(
        state,
        emitter,
        AcpEvent::Error {
            message,
            agent_type: agent_type.clone(),
            code: None,
            details: None,
            terminal: false,
        },
    )
    .await;
    emit_with_state(
        state,
        emitter,
        AcpEvent::TurnComplete {
            session_id: session_id.to_string(),
            stop_reason: "error".into(),
            agent_type,
        },
    )
    .await;
}

async fn apply_subagent_finished(
    inject: NativeInject,
    table: &Arc<Mutex<SubagentTable>>,
    recorder: &FactRecorder,
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
) -> String {
    let NativeInject::SubagentFinished {
        id,
        ok,
        output,
        tool_call_id,
    } = inject;
    table.lock().expect("subagent table").clear_if(&id);
    let status = if ok { "completed" } else { "failed" };
    let note = output.clone();
    let mut meta = NativeMeta::v1();
    meta.function_name = Some("subagent".into());
    meta.tool_call_id = Some(tool_call_id.clone());
    meta.outcome = Some(if ok {
        ToolOutcome::Success
    } else {
        ToolOutcome::Error
    });
    emit_with_state(
        state,
        emitter,
        AcpEvent::ToolCallUpdate {
            tool_call_id: tool_call_id.clone(),
            title: None,
            status: Some(status.into()),
            content: Some(output.clone()),
            raw_input: None,
            raw_output: Some(output),
            raw_output_append: None,
            locations: None,
            meta: None,
            images: None,
        },
    )
    .await;
    let _ = recorder
        .record_update(tool_call_update_payload(&tool_call_id, status, &meta))
        .await;
    let _ = recorder
        .record_update(agent_message_chunk(&note, &meta))
        .await;
    note
}

#[allow(clippy::too_many_arguments)]
async fn start_prompt(
    client: &CodegLlmClient,
    preamble: &str,
    model_id: &str,
    args: &NativeSessionArgs,
    session_id: &str,
    blocks: Vec<PromptInputBlock>,
    user_message: Option<(String, Vec<crate::acp::types::UserMessageBlock>)>,
    coordinator: Arc<TurnCoordinator>,
    store: Arc<Mutex<ContextStore>>,
    recorder: Arc<FactRecorder>,
    fs: Arc<FileSystemRuntime>,
    terminals: Arc<TerminalRuntime>,
    catalog: SkillCatalog,
    companion: CompanionPlan,
    feedback: Option<Arc<FeedbackDelivery>>,
    mcp: Arc<McpSession>,
    subagents: Arc<Mutex<SubagentTable>>,
    inject_tx: mpsc::Sender<NativeInject>,
    session_mode: Arc<tokio::sync::RwLock<String>>,
    artifacts_dir: std::path::PathBuf,
    pending_continue: Arc<Mutex<Option<String>>>,
) -> Result<RunningTurn, String> {
    let inspected = inspect_native_prompt(&blocks);
    if let Some(reason) = inspected.reject_reason() {
        return Err(reason.to_string());
    }
    let text = inspected.text.clone();
    let mut model_blocks = blocks.clone();
    append_agent_routes(&mut model_blocks, companion.delegation_registered());
    let prompt = assemble_native_prompt(&model_blocks);
    let model_text = native_prompt_store_text(&prompt);

    if let Some((message_id, blocks)) = user_message {
        emit_with_state(
            &args.session_state,
            &args.emitter,
            AcpEvent::UserMessage { message_id, blocks },
        )
        .await;
    }
    emit_with_state(
        &args.session_state,
        &args.emitter,
        AcpEvent::StatusChanged {
            status: ConnectionStatus::Prompting,
        },
    )
    .await;

    let (turn_id, cancel) = coordinator.begin();
    let turn_key = {
        let mut store = store.lock().expect("store");
        let idx = store.prompt_index() + 1;
        let key = store.turn_id_for_prompt(idx);
        store.append_user(key.clone(), model_text);
        key
    };
    let prompt_blocks = serde_json::json!([{ "type": "text", "text": text }]);
    recorder
        .record_prompt(prompt_blocks)
        .await
        .map_err(|_| "failed to confirm prompt write".to_string())?;

    let window = u64::from(
        args.effective_config
            .context_windows
            .get(model_id)
            .copied()
            .unwrap_or(0),
    );
    let identity = Arc::new(CallIdentityBridge::new());
    let tool_ctx = NativeToolCtx {
        turn_id,
        identity: Arc::clone(&identity),
        recorder: Arc::clone(&recorder),
        cancel: cancel.clone(),
        launch_cwd: args.launch_cwd.clone(),
        fs,
        session_id: session_id.to_string(),
        spill_dir: recorder.spill_dir(),
    };
    let mode = session_mode.read().await.clone();
    let in_plan = mode == MODE_PLAN;
    let turn_preamble = mode::with_mode_attachment(preamble, &mode);
    let read = ReadFileTool::new(tool_ctx.clone());
    let recall = RecallTool::new(tool_ctx.clone());
    let glob = GlobTool::new(tool_ctx.clone());
    let grep = GrepTool::new(tool_ctx.clone());
    let skill = SkillTool::new(tool_ctx.clone(), catalog.clone());
    let max_output = u64::from(args.effective_config.max_output_tokens);
    let write = (!in_plan).then(|| WriteFileTool::new(tool_ctx.clone()));
    let edit = (!in_plan).then(|| EditFileTool::new(tool_ctx.clone()));
    let bash = (!in_plan).then(|| BashTool::new(tool_ctx.clone(), terminals));
    let plan = (!in_plan).then(|| {
        UpdatePlanTool::new(
            tool_ctx.clone(),
            args.emitter.clone(),
            Arc::clone(&args.session_state),
        )
    });
    let write_plan = in_plan.then(|| WritePlanTool::new(tool_ctx.clone(), artifacts_dir.clone()));
    let enter_plan = (!in_plan).then(|| {
        EnterPlanModeTool::new(
            tool_ctx.clone(),
            Arc::clone(&session_mode),
            Arc::clone(&args.session_state),
            args.emitter.clone(),
            artifacts_dir.clone(),
        )
    });
    let exit_plan = in_plan.then(|| {
        ExitPlanModeTool::new(
            tool_ctx.clone(),
            Arc::clone(&session_mode),
            Arc::clone(&args.session_state),
            args.emitter.clone(),
            artifacts_dir.clone(),
            args.connection_id.clone(),
            args.delegation_injection
                .as_ref()
                .map(|inj| Arc::clone(&inj.plan_approvals)),
            Arc::clone(&pending_continue),
        )
    });
    let subagent = Some(SubagentTool::new(
        tool_ctx.clone(),
        client.clone(),
        model_id.to_string(),
        turn_preamble.clone(),
        catalog,
        BudgetConfig::new(window, max_output).with_compact(
            args.effective_config.compact_soft_percent,
            args.effective_config.compact_recent_turns as usize,
        ),
        subagents,
        inject_tx,
        artifacts_dir.clone(),
    ));
    let mcp_tools = {
        let tools = mcp.dynamic_tools(tool_ctx.clone());
        if in_plan {
            let readonly = mcp.readonly_local_names();
            tools
                .into_iter()
                .filter(|tool| readonly.contains(tool.name()))
                .collect()
        } else {
            tools
        }
    };
    let companion_tools = if let Some(inj) = args.delegation_injection.clone() {
        let runtime = CompanionRuntime {
            tool_ctx,
            connection_id: args.connection_id.clone(),
            launch_cwd: args.launch_cwd.clone(),
            injection: inj,
            session_state: Arc::clone(&args.session_state),
            feedback: feedback.clone(),
        };
        let defs: Vec<_> = companion
            .defs
            .iter()
            .filter(|def| companion_ok_in_mode(&mode, &def.name))
            .cloned()
            .collect();
        let defs = Arc::new(defs);
        build_companion_tools(runtime, &defs)
    } else {
        Vec::new()
    };
    let mut dynamic_tools = companion_tools;
    dynamic_tools.extend(mcp_tools);
    let mut tool_schemas = vec![
        schema_for(&read),
        schema_for(&recall),
        schema_for(&glob),
        schema_for(&grep),
        schema_for(&skill),
    ];
    if let Some(tool) = write.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = edit.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = bash.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = plan.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = write_plan.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = enter_plan.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = exit_plan.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    if let Some(tool) = subagent.as_ref() {
        tool_schemas.push(schema_for(tool));
    }
    let include_echo = args.include_echo_tool;
    if include_echo {
        tool_schemas.push(schema_for(&EchoTool::new()));
    }
    for def in companion
        .defs
        .iter()
        .filter(|def| companion_ok_in_mode(&mode, &def.name))
    {
        tool_schemas.push(schema_for_companion_def(def));
    }
    let mcp_readonly = mcp.readonly_local_names();
    for binding in mcp.bindings() {
        if !in_plan || mcp_readonly.contains(&binding.local_name) {
            tool_schemas.push(binding.schema());
        }
    }
    let compact_prompt = effective_compact_prompt(args.effective_config.compact_prompt.as_deref());
    let compact_prompt = if in_plan {
        mode::plan_compact_prompt(compact_prompt)
    } else {
        compact_prompt.to_string()
    };
    let native = NativeRunState {
        turn_id,
        turn_key: turn_key.clone(),
        budget: BudgetConfig::new(window, max_output).with_compact(
            args.effective_config.compact_soft_percent,
            args.effective_config.compact_recent_turns as usize,
        ),
        preamble: turn_preamble.clone(),
        tool_schemas,
        store: Arc::clone(&store),
        identity,
        recorder: Arc::clone(&recorder),
        last_estimate: Arc::new(Mutex::new(0)),
        last_usage_input: Arc::new(Mutex::new(None)),
        feedback,
        mcp_readonly: Arc::new(mcp_readonly),
        compact: Some(LlmCompactor::new(
            client.clone(),
            args.effective_config
                .compact_model_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or(model_id)
                .to_string(),
            compact_prompt,
            max_output.min(L2_MAX_TOKENS),
        )),
    };
    let (perm_tx, perm_rx) = mpsc::channel(8);
    let host = HostBridge {
        emitter: args.emitter.clone(),
        session_state: Arc::clone(&args.session_state),
        turn: Arc::clone(&coordinator),
        turn_id,
    };
    let hook =
        CodegHook::for_session(HookTrace::new(), perm_tx, cancel.clone(), host).with_native(native);

    // Build the runner on the worker so the command loop keeps polling
    // `cmd_rx` during the first HTTP round-trip (K25).
    let client = client.clone();
    let preamble = turn_preamble;
    let model_id = model_id.to_string();
    let worker_cancel = cancel.clone();
    let echo = include_echo.then(EchoTool::new);
    let max_turns = args.effective_config.max_turns.max(1) as usize;
    let worker = tokio::spawn(async move {
        run_native_turn(NativeTurnRequest {
            client,
            model_id,
            preamble,
            prompt,
            tools: NativeTurnTools {
                read,
                recall,
                write,
                edit,
                glob,
                grep,
                bash,
                skill,
                plan,
                write_plan,
                enter_plan,
                exit_plan,
                write_explore: None,
                subagent,
                echo,
                dynamic: dynamic_tools,
            },
            hook,
            cancel: worker_cancel,
            max_turns,
        })
        .await
    });
    Ok(RunningTurn {
        turn_id,
        turn_key,
        worker,
        permissions: perm_rx,
        cancel,
        recorder,
    })
}

#[allow(clippy::too_many_arguments)]
async fn settle_running(
    running: Option<RunningTurn>,
    coordinator: &TurnCoordinator,
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
    session_id: &str,
    agent_type: String,
    stop_reason: &str,
    injection: Option<&DelegationInjection>,
    connection_id: &str,
    terminals: &TerminalRuntime,
    store: &Arc<Mutex<ContextStore>>,
) -> SettleOutcome {
    let Some(mut running) = running else {
        return SettleOutcome::Settled;
    };
    running.cancel.cancel();
    coordinator.cancel_current();
    for request_id in coordinator.drain_cancelled() {
        emit_with_state(state, emitter, AcpEvent::PermissionResolved { request_id }).await;
    }
    if let Some(inj) = injection {
        inj.broker.cancel_by_parent_turn(connection_id).await;
        inj.questions
            .cancel_questions_by_parent(connection_id)
            .await;
        inj.plan_approvals
            .cancel_plan_approvals_by_parent(connection_id)
            .await;
    }
    terminals.release_all_for_session(session_id).await;
    let worker_done = tokio::time::timeout(WORKER_CANCEL_WAIT, &mut running.worker).await;
    if worker_done.is_err() {
        tracing::warn!("[ACP] native turn worker did not stop after cancel");
        running.worker.abort();
        let _ = tokio::time::timeout(WORKER_CANCEL_WAIT, &mut running.worker).await;
        store
            .lock()
            .expect("store")
            .settle_cancel(&running.turn_key);
        return SettleOutcome::Fatal(TURN_DID_NOT_CONVERGE.into());
    }
    store
        .lock()
        .expect("store")
        .settle_cancel(&running.turn_key);
    if running.recorder.record_turn_end(stop_reason).await.is_err() {
        return SettleOutcome::Fatal(TURN_END_WRITE_FAILED.into());
    }
    finish_turn(
        coordinator,
        running.turn_id,
        state,
        emitter,
        session_id,
        agent_type,
        stop_reason,
    )
    .await;
    SettleOutcome::Settled
}

async fn finish_turn(
    coordinator: &TurnCoordinator,
    turn_id: u64,
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
    session_id: &str,
    agent_type: String,
    stop_reason: &str,
) {
    if !coordinator.try_finish(turn_id) {
        return;
    }
    emit_with_state(
        state,
        emitter,
        AcpEvent::TurnComplete {
            session_id: session_id.to_string(),
            stop_reason: stop_reason.to_string(),
            agent_type,
        },
    )
    .await;
    emit_with_state(
        state,
        emitter,
        AcpEvent::StatusChanged {
            status: ConnectionStatus::Connected,
        },
    )
    .await;
}

async fn publish_permission(
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
    coordinator: &TurnCoordinator,
    pending: PendingPermission,
) {
    let request_id = uuid::Uuid::new_v4().to_string();
    let tool_call = json!({
        "toolCallId": pending.tool_call_id.clone().unwrap_or_else(|| request_id.clone()),
        "title": pending.tool_name,
        "kind": tool_kind(&pending.tool_name),
        "status": "pending",
        "rawInput": pending.args,
    });
    coordinator.register_permission(request_id.clone(), pending.reply);
    emit_with_state(
        state,
        emitter,
        AcpEvent::PermissionRequest {
            request_id,
            tool_call,
            options: vec![
                PermissionOptionInfo {
                    option_id: "allow-once".into(),
                    name: "Allow once".into(),
                    kind: "allow_once".into(),
                    meta: None,
                },
                PermissionOptionInfo {
                    option_id: "reject-once".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                    meta: None,
                },
            ],
            queued: 0,
        },
    )
    .await;
}

struct NativeOriginRewind {
    session_id: String,
    lease: crate::acp_transcript::TranscriptLease,
    recorder: FactRecorder,
    artifacts_dir: std::path::PathBuf,
}

async fn rewind_native_to_origin(
    args: &NativeSessionArgs,
    old_session_id: &str,
    store: &Arc<Mutex<ContextStore>>,
) -> Result<NativeOriginRewind, crate::acp::error::AcpError> {
    let sessions_root = crate::paths::codeg_agent_sessions_root();
    let fallback_root = crate::paths::codeg_acp_transcripts_root();
    let opened = open_codeg_agent_session(
        &sessions_root,
        Some(&fallback_root),
        &args.launch_cwd.to_string_lossy(),
        args.agent_type.as_wire().as_ref(),
        None,
    )
    .map_err(|err| crate::acp::error::AcpError::protocol(err.to_string()))?;

    let session_id = opened.session_id.clone();
    {
        let mut inner = store.lock().expect("store");
        *inner = opened.store;
    }
    let recorder = FactRecorder::transcript(
        opened.write_root.clone(),
        opened.write_group.clone(),
        session_id.clone(),
        Arc::clone(store),
    );
    let header = TranscriptHeader::new(
        args.agent_type.as_wire().as_ref(),
        &session_id,
        &args.launch_cwd.to_string_lossy(),
        now_epoch_ms(),
    )
    .continuing(old_session_id);
    let ack = record_header_critical_in(&opened.write_root, &opened.write_group, &header);
    match tokio::time::timeout(HEADER_ACK_TIMEOUT, ack).await {
        Ok(Ok(())) => {}
        _ => {
            return Err(crate::acp::error::AcpError::protocol(
                "failed to confirm transcript header",
            ));
        }
    }
    let artifacts_dir =
        crate::agent::mode::artifacts_dir(&args.launch_cwd.to_string_lossy(), &session_id);
    let _ = std::fs::create_dir_all(&artifacts_dir);
    Ok(NativeOriginRewind {
        session_id,
        lease: opened.lease,
        recorder,
        artifacts_dir,
    })
}

async fn handle_control_command(
    cmd: ConnectionCommand,
    in_turn: bool,
    model_id: &mut String,
    args: &mut NativeSessionArgs,
    _session_id: &str,
    session_mode: &Arc<tokio::sync::RwLock<String>>,
    artifacts_dir: &std::path::Path,
) {
    match cmd {
        ConnectionCommand::SetConfigOption {
            config_id,
            value_id,
        } => {
            if config_id != "model" {
                emit_with_state(
                    &args.session_state,
                    &args.emitter,
                    AcpEvent::Error {
                        message: format!("Unsupported config option `{config_id}`"),
                        agent_type: args.agent_type.to_string(),
                        code: None,
                        details: None,
                        terminal: false,
                    },
                )
                .await;
                return;
            }
            if in_turn {
                emit_with_state(
                    &args.session_state,
                    &args.emitter,
                    AcpEvent::ConfigOptionRejected {
                        config_id,
                        option_name: "Model".into(),
                        requested: value_id,
                        actual: model_id.clone(),
                    },
                )
                .await;
                return;
            }
            match resolve_session_model(
                &args.effective_config,
                &BTreeMap::from([("model".into(), value_id.clone())]),
            ) {
                Ok(next) => {
                    *model_id = next;
                    emit_session_config(
                        &args.session_state,
                        &args.emitter,
                        &args.effective_config,
                        model_id,
                    )
                    .await;
                }
                Err(_) => {
                    emit_with_state(
                        &args.session_state,
                        &args.emitter,
                        AcpEvent::ConfigOptionRejected {
                            config_id,
                            option_name: "Model".into(),
                            requested: value_id,
                            actual: model_id.clone(),
                        },
                    )
                    .await;
                }
            }
        }
        ConnectionCommand::SetMode { mode_id } => {
            if in_turn {
                emit_with_state(
                    &args.session_state,
                    &args.emitter,
                    AcpEvent::Error {
                        message: "Cannot change session mode during a turn".into(),
                        agent_type: args.agent_type.to_string(),
                        code: None,
                        details: None,
                        terminal: false,
                    },
                )
                .await;
                return;
            }
            let Some(mode) = mode::parse_mode(&mode_id) else {
                emit_with_state(
                    &args.session_state,
                    &args.emitter,
                    AcpEvent::Error {
                        message: format!("Unknown session mode `{mode_id}`"),
                        agent_type: args.agent_type.to_string(),
                        code: None,
                        details: None,
                        terminal: false,
                    },
                )
                .await;
                return;
            };
            mode::apply_mode(
                &args.session_state,
                &args.emitter,
                session_mode,
                artifacts_dir,
                mode,
            )
            .await;
        }
        ConnectionCommand::GoalControl {
            reply: Some(reply), ..
        } => {
            let _ = reply.send(false);
        }
        ConnectionCommand::Steer { reply, .. } => {
            let _ = reply.send(Err(crate::acp::error::AcpError::protocol(
                "Codeg Agent does not support steering",
            )));
        }
        ConnectionCommand::Fork { reply, .. } => {
            let _ = reply.send(Err(crate::acp::error::AcpError::protocol(
                "This agent does not support session/fork",
            )));
        }
        ConnectionCommand::StopAsyncTask { reply, .. } => {
            let _ = reply.send(Ok(false));
        }
        _ => {}
    }
}

async fn write_capability_snapshot(state: &Arc<RwLock<SessionState>>, plan: &CompanionPlan) {
    let mut s = state.write().await;
    s.feedback_tool_available = plan.feedback_registered();
    s.delegation_enabled = plan.delegation_registered();
    s.native_steering_available = false;
}

fn resolve_session_model(
    config: &EffectiveNativeConfig,
    preferred: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    if let Some(model) = preferred
        .get("model")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if config.context_windows.contains_key(model) {
            return Ok(model.to_string());
        }
        return Err(format!(
            "Unknown model `{model}` is not in CODEG_AGENT_CONTEXT_WINDOWS"
        ));
    }
    Ok(config.model_id.clone())
}

async fn emit_session_config(
    state: &Arc<RwLock<SessionState>>,
    emitter: &EventEmitter,
    config: &EffectiveNativeConfig,
    model_id: &str,
) {
    emit_with_state(
        state,
        emitter,
        AcpEvent::SessionConfigOptions {
            config_options: vec![native_session_model_option(config, model_id)],
        },
    )
    .await;
}

/// Session model picker: only ids with an explicit window. Unknown ids are
/// rejected at SetConfigOption; windows are never assumed to be 128k.
pub(crate) fn native_session_model_option(
    config: &EffectiveNativeConfig,
    model_id: &str,
) -> SessionConfigOptionInfo {
    let options: Vec<SessionConfigSelectOptionInfo> = config
        .context_windows
        .iter()
        .map(|(id, window)| SessionConfigSelectOptionInfo {
            value: id.clone(),
            name: id.clone(),
            description: Some(format!("{window}-token window")),
        })
        .collect();
    SessionConfigOptionInfo {
        id: "model".into(),
        name: "Model".into(),
        description: Some(
            "Ids from CODEG_AGENT_CONTEXT_WINDOWS. Unknown models cannot start; \
             windows are not assumed to be 128k."
                .into(),
        ),
        category: None,
        kind: SessionConfigKindInfo::Select(SessionConfigSelectInfo {
            current_value: model_id.to_string(),
            options,
            groups: Vec::new(),
        }),
    }
}

pub(crate) struct NativeDropGuard {
    pub(crate) shutdown: Arc<NativeShutdownHandle>,
    pub(crate) _map: Option<crate::acp::connection::ConnectionCleanupGuard>,
}

impl Drop for NativeDropGuard {
    fn drop(&mut self) {
        self.shutdown.signal_shutdown();
        let had_pids = self.shutdown.kill_owners_now();
        if matches!(self.shutdown.cleanup_state(), NativeCleanupState::Pending) {
            if had_pids {
                self.shutdown.mark_failed();
            } else {
                self.shutdown.mark_complete();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::native_config::EffectiveNativeConfig;

    #[test]
    fn model_picker_lists_configured_windows_only() {
        let mut windows = BTreeMap::new();
        windows.insert("gateway-model".into(), 128000);
        windows.insert("small".into(), 8192);
        let config = EffectiveNativeConfig {
            api_base_url: "https://example.test/v1".into(),
            api_key: "sk".into(),
            model_id: "gateway-model".into(),
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
        };
        let option = native_session_model_option(&config, "gateway-model");
        assert_eq!(option.id, "model");
        let description = option.description.expect("window copy");
        assert!(description.contains("CODEG_AGENT_CONTEXT_WINDOWS"));
        assert!(description.contains("128k"));
        match option.kind {
            SessionConfigKindInfo::Select(select) => {
                assert_eq!(select.current_value, "gateway-model");
                assert_eq!(select.options.len(), 2);
                let small = select
                    .options
                    .iter()
                    .find(|o| o.value == "small")
                    .expect("small");
                assert_eq!(small.description.as_deref(), Some("8192-token window"));
                let large = select
                    .options
                    .iter()
                    .find(|o| o.value == "gateway-model")
                    .expect("gateway");
                assert_eq!(large.description.as_deref(), Some("128000-token window"));
            }
            other => panic!("expected select, got {other:?}"),
        }
    }
}

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::web::event_bridge::{emit_event, EventEmitter};

const HOOK_EVENT_NAME: &str = "agent-terminal://event";
const HOOK_MAX_BYTES: u64 = 64 * 1024;
const NATIVE_PAYLOAD_MAX_BYTES: u64 = 4 * 1024 * 1024;
const INACTIVE_RUNS_MAX: usize = 256;
const CALLBACK_ENV: &str = "CODEG_AGENT_HOOK_URL";
const TOKEN_ENV: &str = "CODEG_AGENT_HOOK_TOKEN";
const RUN_ENV: &str = "CODEG_AGENT_HOOK_RUN_ID";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEventType {
    TurnStarted,
    TurnStopped,
    SessionStarted,
    SessionEnded,
}

impl TurnEventType {
    fn native_name(self) -> &'static str {
        match self {
            Self::TurnStarted => "UserPromptSubmit",
            Self::TurnStopped => "Stop",
            Self::SessionStarted => "SessionStart",
            Self::SessionEnded => "SessionEnd",
        }
    }

    fn from_helper_action(action: &str) -> Option<Self> {
        match action {
            "turn-start" => Some(Self::TurnStarted),
            "turn-stop" => Some(Self::TurnStopped),
            "session-start" => Some(Self::SessionStarted),
            "session-end" => Some(Self::SessionEnded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookSupport {
    Available,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentTerminalEvent {
    pub run_id: String,
    pub terminal_id: String,
    pub agent_type: String,
    pub event_id: String,
    #[serde(rename = "type")]
    pub kind: TurnEventType,
    pub native_event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_turn_id: Option<String>,
    /// Agent-supplied session name. Applied with `refresh_auto_title`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Truncated first-prompt seed. Applied only while the row title is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_seed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Prompt or last assistant text for the chat card. Capped at 2000 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice_body: Option<String>,
    pub received_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentTerminalState {
    pub run_id: String,
    pub terminal_id: String,
    pub agent_type: String,
    pub hook_support: HookSupport,
    pub active: bool,
    pub latest_event: Option<AgentTerminalEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HookReport {
    run_id: String,
    event_id: String,
    #[serde(rename = "type")]
    kind: TurnEventType,
    native_event: String,
    native_session_id: Option<String>,
    native_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title_seed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Prompt on turn start, last assistant text on turn stop. Capped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notice_body: Option<String>,
}

struct RunRecord {
    state: AgentTerminalState,
    token: Option<String>,
    seen: HashSet<String>,
    /// Conversation a channel-started CLI should bind when its session id arrives.
    preferred_conversation: Option<(i32, i32)>,
    session_bound: bool,
}

#[derive(Default)]
struct RunStore {
    by_terminal: HashMap<String, RunRecord>,
    by_token: HashMap<String, String>,
    /// Native session id → terminal id, including resumes that have not hooked yet.
    by_session: HashMap<String, String>,
    /// Claimed before `register` so the first hook already knows the conversation.
    reserved_conversation: HashMap<String, (i32, i32)>,
    inactive: VecDeque<(String, String)>,
}

impl RunStore {
    fn register(&mut self, terminal_id: &str, agent_type: &str, token: &str) -> Option<String> {
        if self
            .by_terminal
            .get(terminal_id)
            .is_some_and(|record| record.state.active)
        {
            return None;
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        let token = (!token.is_empty()).then(|| token.to_string());
        let hook_support = if token.is_some() {
            HookSupport::Available
        } else {
            HookSupport::Unsupported
        };
        if let Some(token) = &token {
            self.by_token.insert(token.clone(), terminal_id.to_string());
        }
        let preferred_conversation = self.reserved_conversation.remove(terminal_id);
        self.by_terminal.insert(
            terminal_id.to_string(),
            RunRecord {
                state: AgentTerminalState {
                    run_id: run_id.clone(),
                    terminal_id: terminal_id.to_string(),
                    agent_type: agent_type.to_string(),
                    hook_support,
                    active: true,
                    latest_event: None,
                },
                token,
                seen: HashSet::new(),
                preferred_conversation,
                session_bound: false,
            },
        );
        Some(run_id)
    }

    fn remember_session(&mut self, terminal_id: &str, session_id: &str) {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return;
        }
        self.by_session
            .insert(session_id.to_string(), terminal_id.to_string());
    }

    fn forget_session(&mut self, terminal_id: &str) {
        self.by_session.retain(|_, id| id != terminal_id);
    }

    fn record(&mut self, token: &str, run_id: &str, event_id: &str) -> bool {
        let Some(terminal_id) = self.by_token.get(token) else {
            return false;
        };
        let Some(record) = self.by_terminal.get_mut(terminal_id) else {
            return false;
        };
        record.state.active
            && record.state.run_id == run_id
            && record.seen.insert(event_id.to_string())
    }

    fn revoke(&mut self, terminal_id: &str, run_id: &str) {
        if let Some(record) = self.by_terminal.get_mut(terminal_id) {
            if record.state.run_id == run_id && record.state.active {
                record.state.active = false;
                record.seen.clear();
                if let Some(token) = record.token.take() {
                    self.by_token.remove(&token);
                }
                self.forget_session(terminal_id);
                self.inactive
                    .push_back((terminal_id.to_string(), run_id.to_string()));
                while self.inactive.len() > INACTIVE_RUNS_MAX {
                    if let Some((old_terminal, old_run)) = self.inactive.pop_front() {
                        if self
                            .by_terminal
                            .get(&old_terminal)
                            .is_some_and(|old| !old.state.active && old.state.run_id == old_run)
                        {
                            self.by_terminal.remove(&old_terminal);
                        }
                    }
                }
            }
        }
    }
}

pub struct AgentHookRegistry {
    store: Mutex<RunStore>,
    listener_url: tokio::sync::Mutex<Option<String>>,
    emitter: EventEmitter,
    db: Mutex<Option<sea_orm::DatabaseConnection>>,
}

impl AgentHookRegistry {
    pub fn new(emitter: EventEmitter) -> Self {
        Self {
            store: Mutex::new(RunStore::default()),
            listener_url: tokio::sync::Mutex::new(None),
            emitter,
            db: Mutex::new(None),
        }
    }

    pub fn set_database(&self, conn: sea_orm::DatabaseConnection) {
        *self.db.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(conn);
    }

    /// Remember which conversation a terminal id belongs to before the process starts.
    pub fn reserve_conversation(&self, terminal_id: &str, conversation_id: i32, folder_id: i32) {
        self.store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reserved_conversation
            .insert(terminal_id.to_string(), (conversation_id, folder_id));
    }

    pub fn clear_reservation(&self, terminal_id: &str) {
        self.store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reserved_conversation
            .remove(terminal_id);
    }

    pub fn remember_native_session(&self, terminal_id: &str, session_id: &str) {
        self.store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remember_session(terminal_id, session_id);
    }

    pub fn active_terminal_for_session(&self, session_id: &str) -> Option<String> {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let terminal_id = store.by_session.get(session_id.trim())?.clone();
        store
            .by_terminal
            .get(&terminal_id)
            .is_some_and(|record| record.state.active)
            .then_some(terminal_id)
    }

    pub async fn ensure_listener(self: &Arc<Self>) -> std::io::Result<String> {
        let mut current = self.listener_url.lock().await;
        if let Some(url) = current.as_ref() {
            return Ok(url.clone());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/agent-hook", listener.local_addr()?);
        let router = Router::new()
            .route("/agent-hook", post(receive_hook))
            .layer(DefaultBodyLimit::max(HOOK_MAX_BYTES as usize))
            .with_state(self.clone());
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::error!("[agent-hook] listener stopped: {error}");
            }
        });
        *current = Some(url.clone());
        Ok(url)
    }

    pub fn register_run(
        &self,
        terminal_id: &str,
        agent_type: &str,
        hook_enabled: bool,
    ) -> Option<(AgentTerminalState, Option<String>)> {
        let token = hook_enabled.then(|| uuid::Uuid::new_v4().to_string());
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        store.register(terminal_id, agent_type, token.as_deref().unwrap_or(""))?;
        let state = store.by_terminal.get(terminal_id)?.state.clone();
        Some((state, token))
    }

    pub fn state(&self, terminal_id: &str) -> Option<AgentTerminalState> {
        self.store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .by_terminal
            .get(terminal_id)
            .map(|record| record.state.clone())
    }

    pub fn revoke(&self, terminal_id: &str, run_id: &str) {
        self.store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .revoke(terminal_id, run_id);
    }

    /// PTY exit is a session end, not a successful turn. The native name stays
    /// `process_exit` so it is not reported as Claude's SessionEnd hook.
    /// `latest_event` keeps the session id for a remount, then the run is revoked.
    pub fn note_process_exit(&self, terminal_id: &str, run_id: &str) {
        let event = {
            let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            let Some(record) = store.by_terminal.get_mut(terminal_id) else {
                return;
            };
            if !record.state.active || record.state.run_id != run_id {
                return;
            }
            let native_session_id = record
                .state
                .latest_event
                .as_ref()
                .and_then(|event| event.native_session_id.clone());
            let event = AgentTerminalEvent {
                run_id: run_id.to_string(),
                terminal_id: terminal_id.to_string(),
                agent_type: record.state.agent_type.clone(),
                event_id: uuid::Uuid::new_v4().to_string(),
                kind: TurnEventType::SessionEnded,
                native_event: "process_exit".to_string(),
                native_session_id,
                native_turn_id: None,
                title: None,
                title_seed: None,
                model: None,
                notice_body: None,
                received_at: chrono::Utc::now().to_rfc3339(),
            };
            record.state.latest_event = Some(event.clone());
            event
        };
        emit_event(&self.emitter, HOOK_EVENT_NAME, event);
        self.revoke(terminal_id, run_id);
    }

    fn accept(&self, token: &str, report: HookReport) -> bool {
        if report.native_event != report.kind.native_name() || report.event_id.is_empty() {
            return false;
        }
        let (event, bind) = {
            let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
            if !store.record(token, &report.run_id, &report.event_id) {
                return false;
            }
            let terminal_id = store.by_token.get(token).cloned().unwrap_or_default();
            let prepared = {
                let Some(record) = store.by_terminal.get_mut(&terminal_id) else {
                    return false;
                };
                let event = AgentTerminalEvent {
                    run_id: report.run_id,
                    terminal_id: terminal_id.clone(),
                    agent_type: record.state.agent_type.clone(),
                    event_id: report.event_id,
                    kind: report.kind,
                    native_event: report.native_event,
                    native_session_id: report.native_session_id,
                    native_turn_id: report.native_turn_id,
                    title: report.title,
                    title_seed: report.title_seed,
                    model: report.model,
                    notice_body: report.notice_body,
                    received_at: chrono::Utc::now().to_rfc3339(),
                };
                record.state.latest_event = Some(event.clone());
                let session_id = event.native_session_id.clone();
                let bind = session_id.as_ref().and_then(|_| {
                    let preferred = record.preferred_conversation?;
                    if record.session_bound {
                        return None;
                    }
                    record.session_bound = true;
                    Some(preferred)
                });
                (event, bind, session_id)
            };
            if let Some(session_id) = prepared.2.as_deref() {
                store.remember_session(&terminal_id, session_id);
            }
            (prepared.0, prepared.1)
        };
        emit_event(&self.emitter, HOOK_EVENT_NAME, event.clone());
        if let Some((conversation_id, folder_id)) = bind {
            self.bind_preferred_session(&event, conversation_id, folder_id);
        }
        true
    }

    fn bind_preferred_session(
        &self,
        event: &AgentTerminalEvent,
        conversation_id: i32,
        folder_id: i32,
    ) {
        let Some(external_id) = event.native_session_id.clone() else {
            return;
        };
        let Some(db) = self
            .db
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
        else {
            return;
        };
        let agent_type = event.agent_type.clone();
        let title = event.title.clone();
        let title_seed = event.title_seed.clone();
        let model = event.model.clone();
        tokio::spawn(async move {
            let Some(agent_type) = crate::models::agent::AgentType::from_wire(&agent_type) else {
                return;
            };
            let request =
                crate::db::service::conversation_service::EstablishTerminalSessionRequest {
                    agent_type,
                    external_id,
                    preferred_conversation_id: Some(conversation_id),
                    folder_id: Some(folder_id),
                    title,
                    title_seed,
                    model,
                };
            if let Err(error) =
                crate::db::service::conversation_service::establish_terminal_session(&db, request)
                    .await
            {
                tracing::warn!(
                    conversation_id,
                    error = %error,
                    "[agent-hook] failed to bind channel terminal session"
                );
            }
        });
    }
}

async fn receive_hook(
    State(registry): State<Arc<AgentHookRegistry>>,
    headers: HeaderMap,
    Json(report): Json<HookReport>,
) -> StatusCode {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if registry.accept(token, report) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::UNAUTHORIZED
    }
}

pub fn hook_environment(url: &str, run_id: &str, token: &str) -> HashMap<String, String> {
    HashMap::from([
        (CALLBACK_ENV.to_string(), url.to_string()),
        (RUN_ENV.to_string(), run_id.to_string()),
        (TOKEN_ENV.to_string(), token.to_string()),
    ])
}

/// Serde aliases reject an object that contains both `session_id` and `sessionId`.
fn report_from_payload(kind: TurnEventType, run_id: String, payload: &[u8]) -> Option<HookReport> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let object = value.as_object()?;
    if is_subagent_payload(object) {
        return None;
    }
    let title = ["session_title", "sessionTitle", "title"]
        .into_iter()
        .find_map(|key| {
            bounded_title(
                object
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            )
        });
    let title_seed = bounded_title(
        object
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    );
    let model = first_nonblank(object, &["model", "modelId"])
        .map(|value| crate::parsers::truncate_str(value.trim(), 200));
    let notice_body = match kind {
        TurnEventType::TurnStarted => bounded_card_body(first_nonblank(object, &["prompt"])),
        TurnEventType::TurnStopped => bounded_card_body(first_nonblank(
            object,
            &["last_assistant_message", "lastAssistantMessage"],
        )),
        TurnEventType::SessionStarted | TurnEventType::SessionEnded => None,
    };
    Some(HookReport {
        run_id,
        event_id: first_nonblank(object, &["event_id", "eventId"])
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        kind,
        native_event: kind.native_name().to_string(),
        native_session_id: first_nonblank(object, &["session_id", "sessionId"]),
        native_turn_id: first_nonblank(object, &["turn_id", "promptId", "turnId"]),
        title,
        title_seed,
        model,
        notice_body,
    })
}

/// A present non-null id, or a non-blank subagent type, belongs to a child run.
/// JSON null and a missing key stay on the parent. A blank string id still drops
/// the event.
fn is_subagent_payload(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    const ID_KEYS: [&str; 8] = [
        "agent_id",
        "agentId",
        "subagent_id",
        "subagentId",
        "parent_tool_use_id",
        "parentToolUseId",
        // A child run reports the parent session next to its own session id.
        // Treating that as the parent CLI minted a second untitled conversation
        // for every subagent the terminal spawned.
        "parent_session_id",
        "parentSessionId",
    ];
    if ID_KEYS
        .iter()
        .any(|key| object.get(*key).is_some_and(|value| !value.is_null()))
    {
        return true;
    }
    ["subagent_type", "subagentType"]
        .iter()
        .any(|key| match object.get(*key) {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::String(text)) => !text.trim().is_empty(),
            Some(_) => true,
        })
}

/// First non-blank string among `keys`. Snake_case comes before camelCase.
fn first_nonblank(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        let text = object.get(*key)?.as_str()?;
        if text.trim().is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    })
}

/// Title text that may be stored on a conversation row. The chat card keeps a
/// separate 2000-character `notice_body`; this fold is only the 100-character
/// title seed.
fn bounded_title(value: Option<String>) -> Option<String> {
    let text = value?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let title = crate::parsers::title_from_user_text(&text);
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

const CARD_BODY_CHARS: usize = 2000;

/// Prompt or assistant text for the chat card. Empty becomes absent.
/// `truncate_str` keeps `CARD_BODY_CHARS` and appends "..." when it cuts.
fn bounded_card_body(value: Option<String>) -> Option<String> {
    let text = value?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(crate::parsers::truncate_str(&text, CARD_BODY_CHARS))
}

/// Executed by the current binary before the desktop or server runtime starts.
/// Native hooks are advisory; delivery failures never change the CLI decision.
pub fn run_agent_hook_helper() {
    let mut args = std::env::args();
    let Some(kind) = args
        .find(|arg| arg == "--agent-hook")
        .and_then(|_| args.next())
        .as_deref()
        .and_then(TurnEventType::from_helper_action)
    else {
        return;
    };
    let (Ok(url), Ok(token), Ok(run_id)) = (
        std::env::var(CALLBACK_ENV),
        std::env::var(TOKEN_ENV),
        std::env::var(RUN_ENV),
    ) else {
        return;
    };
    let Ok(callback) = url::Url::parse(&url) else {
        return;
    };
    if callback.scheme() != "http"
        || callback.host_str() != Some("127.0.0.1")
        || callback.port().is_none()
        || callback.path() != "/agent-hook"
    {
        return;
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if std::thread::Builder::new()
        .spawn(move || {
            let mut payload = Vec::new();
            let result = std::io::stdin()
                .take(NATIVE_PAYLOAD_MAX_BYTES + 1)
                .read_to_end(&mut payload)
                .ok()
                .filter(|_| payload.len() as u64 <= NATIVE_PAYLOAD_MAX_BYTES)
                .map(|_| payload);
            let _ = tx.send(result);
        })
        .is_err()
    {
        return;
    }
    let Ok(Some(payload)) = rx.recv_timeout(Duration::from_millis(700)) else {
        return;
    };
    let Some(report) = report_from_payload(kind, run_id, &payload) else {
        return;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    runtime.block_on(async {
        let Ok(client) = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(800))
            .build()
        else {
            return;
        };
        let _ = client
            .post(url)
            .bearer_auth(token)
            .json(&report)
            .send()
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_run_rejects_wrong_token_and_duplicate_event() {
        let mut store = RunStore::default();
        let run = store.register("terminal-1", "codex", "token-1").unwrap();
        assert!(!store.record("wrong-token", &run, "event-1"));
        assert!(store.record("token-1", &run, "event-1"));
        assert!(!store.record("token-1", &run, "event-1"));
        store.revoke("terminal-1", &run);
        assert!(!store.record("token-1", &run, "event-2"));
    }

    #[test]
    fn hook_run_rejects_parallel_spawn_and_stale_run() {
        let mut store = RunStore::default();
        let first = store.register("terminal-1", "codex", "token-1").unwrap();
        assert!(store.register("terminal-1", "codex", "token-2").is_none());
        store.revoke("terminal-1", &first);
        let second = store.register("terminal-1", "codex", "token-2").unwrap();
        assert_ne!(first, second);
        assert!(!store.record("token-1", &first, "event-2"));
        assert!(!store.record("token-2", &first, "event-3"));
        assert!(store.record("token-2", &second, "event-4"));
    }

    #[test]
    fn inactive_run_cache_is_bounded_and_keeps_recent_state() {
        let mut store = RunStore::default();
        for index in 0..(INACTIVE_RUNS_MAX + 8) {
            let terminal_id = format!("terminal-{index}");
            let run = store.register(&terminal_id, "codex", "token").unwrap();
            store.revoke(&terminal_id, &run);
        }
        assert_eq!(store.by_terminal.len(), INACTIVE_RUNS_MAX);
        assert!(!store.by_terminal.contains_key("terminal-0"));
        assert!(store
            .by_terminal
            .contains_key(&format!("terminal-{}", INACTIVE_RUNS_MAX + 7)));
    }

    #[test]
    fn one_long_run_still_rejects_old_duplicate_events() {
        let mut store = RunStore::default();
        let run = store.register("terminal-1", "codex", "token-1").unwrap();
        for index in 0..1100 {
            assert!(store.record("token-1", &run, &index.to_string()));
        }
        let record = store.by_terminal.get("terminal-1").unwrap();
        assert_eq!(record.seen.len(), 1100);
        assert!(!store.record("token-1", &run, "0"));
        assert!(!store.record("token-1", &run, "1099"));
    }

    #[test]
    fn native_payload_ignores_subagent_events_and_prompt_body() {
        let payload = br#"{"session_id":"session-1","turn_id":"turn-1","prompt":"private"}"#;
        let report =
            report_from_payload(TurnEventType::TurnStarted, "run".into(), payload).unwrap();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["native_session_id"], "session-1");
        assert_eq!(value["native_turn_id"], "turn-1");
        assert!(value.get("prompt").is_none());
        assert_eq!(value["notice_body"], "private");
        assert_eq!(value["title_seed"], "private");
        assert!(value.get("title").is_none());
        let child = br#"{"agent_id":"child-1","session_id":"session-1"}"#;
        assert!(report_from_payload(TurnEventType::TurnStopped, "run".into(), child).is_none());
    }

    #[test]
    fn native_payload_can_discard_a_large_message_body() {
        let payload = serde_json::json!({
            "session_id": "session-1",
            "last_assistant_message": "x".repeat(80 * 1024),
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let report = report_from_payload(TurnEventType::TurnStopped, "run".into(), &bytes).unwrap();
        assert_eq!(report.native_session_id.as_deref(), Some("session-1"));
        let body = report.notice_body.as_deref().unwrap();
        assert!(body.ends_with("..."));
        assert_eq!(body.chars().count(), CARD_BODY_CHARS + 3);
        assert!(serde_json::to_vec(&report).unwrap().len() < 8 * 1024);
    }

    #[test]
    fn session_start_keeps_a_title_and_a_huge_prompt_becomes_a_short_seed() {
        let started = report_from_payload(
            TurnEventType::SessionStarted,
            "run".into(),
            br#"{"session_id":"session-1","session_title":"  Plan the release  ","model":"claude-opus"}"#,
        )
        .unwrap();
        assert_eq!(started.kind, TurnEventType::SessionStarted);
        assert_eq!(started.native_event, "SessionStart");
        assert_eq!(started.title.as_deref(), Some("Plan the release"));
        assert!(started.title_seed.is_none());
        assert!(started.notice_body.is_none());
        assert_eq!(started.model.as_deref(), Some("claude-opus"));

        let prompt = format!("hello {}", "x".repeat(80 * 1024));
        let bytes = serde_json::to_vec(&serde_json::json!({
            "session_id": "session-1",
            "prompt": prompt,
        }))
        .unwrap();
        let seeded = report_from_payload(TurnEventType::TurnStarted, "run".into(), &bytes).unwrap();
        let value = serde_json::to_value(&seeded).unwrap();
        assert!(value.get("prompt").is_none());
        let seed = value["title_seed"].as_str().unwrap();
        assert_eq!(seed, crate::parsers::title_from_user_text(&prompt));
        assert!(seed.starts_with("hello "));
        assert!(seed.ends_with("..."));
        // `title_from_user_text` keeps 100 characters and then appends "...".
        assert_eq!(seed.chars().count(), 103);
        let body = seeded.notice_body.as_deref().unwrap();
        assert!(body.ends_with("..."));
        assert_eq!(body.chars().count(), CARD_BODY_CHARS + 3);
        assert!(serde_json::to_vec(&seeded).unwrap().len() < 8 * 1024);
        assert_eq!(
            TurnEventType::from_helper_action("session-start"),
            Some(TurnEventType::SessionStarted)
        );
        assert_eq!(
            TurnEventType::from_helper_action("session-end"),
            Some(TurnEventType::SessionEnded)
        );
        assert_eq!(TurnEventType::SessionEnded.native_name(), "SessionEnd");
    }

    #[test]
    fn camel_case_grok_payload_keeps_session_turn_title_and_model() {
        let payload = br#"{"sessionId":"sid-9","promptId":"turn-9","prompt":"fix the pane","modelId":"grok-4","title":"Pane"}"#;
        let report =
            report_from_payload(TurnEventType::TurnStarted, "run".into(), payload).unwrap();
        assert_eq!(report.kind, TurnEventType::TurnStarted);
        assert_eq!(report.native_session_id.as_deref(), Some("sid-9"));
        assert_eq!(report.native_turn_id.as_deref(), Some("turn-9"));
        assert_eq!(report.title_seed.as_deref(), Some("fix the pane"));
        assert_eq!(report.model.as_deref(), Some("grok-4"));
        assert_eq!(report.title.as_deref(), Some("Pane"));
        assert_eq!(report.notice_body.as_deref(), Some("fix the pane"));
        let value = serde_json::to_value(&report).unwrap();
        assert!(value.get("prompt").is_none());
    }

    #[test]
    fn camel_case_subagent_markers_drop_the_event() {
        for payload in [
            br#"{"subagentId":"child","sessionId":"sid"}"#.as_slice(),
            br#"{"subagentType":"explore","sessionId":"sid"}"#.as_slice(),
            br#"{"parentToolUseId":"tool-1","sessionId":"sid"}"#.as_slice(),
            br#"{"parentSessionId":"parent","sessionId":"child"}"#.as_slice(),
            br#"{"parent_session_id":"parent","session_id":"child"}"#.as_slice(),
        ] {
            assert!(
                report_from_payload(TurnEventType::TurnStarted, "run".into(), payload).is_none()
            );
        }
    }

    #[test]
    fn blank_session_id_is_absent_and_empty_subagent_type_stays_parent() {
        for payload in [
            br#"{"session_id":"","turn_id":"t"}"#.as_slice(),
            br#"{"sessionId":"   ","promptId":"t"}"#.as_slice(),
            br#"{"session_id":"  \n"}"#.as_slice(),
        ] {
            let report =
                report_from_payload(TurnEventType::TurnStarted, "run".into(), payload).unwrap();
            assert!(report.native_session_id.is_none());
        }

        let parent = report_from_payload(
            TurnEventType::SessionStarted,
            "run".into(),
            br#"{"subagentType":"","sessionId":"sid","turnId":"turn-a","sessionTitle":"Hello","eventId":"evt-1","model":"m"}"#,
        )
        .unwrap();
        assert_eq!(parent.native_session_id.as_deref(), Some("sid"));
        assert_eq!(parent.native_turn_id.as_deref(), Some("turn-a"));
        assert_eq!(parent.title.as_deref(), Some("Hello"));
        assert_eq!(parent.event_id, "evt-1");
        assert_eq!(parent.model.as_deref(), Some("m"));

        let spaced = report_from_payload(
            TurnEventType::TurnStarted,
            "run".into(),
            br#"{"subagent_type":"   ","session_id":"sid"}"#,
        )
        .unwrap();
        assert_eq!(spaced.native_session_id.as_deref(), Some("sid"));

        assert!(report_from_payload(
            TurnEventType::TurnStarted,
            "run".into(),
            br#"{"agentId":"child","sessionId":"sid"}"#,
        )
        .is_none());
        assert!(report_from_payload(
            TurnEventType::TurnStopped,
            "run".into(),
            br#"{"subagent_type":"explore","session_id":"sid"}"#,
        )
        .is_none());
    }

    #[test]
    fn dual_key_grok_payload_keeps_session_turn_and_prompt_seed() {
        // grok 1.0.44 emits both spellings in one object. Aliases reject that.
        let payload = br#"{
            "hookEventName":"user_prompt_submit",
            "sessionId":"sid-dual",
            "cwd":"/tmp/work",
            "workspaceRoot":"/tmp/work",
            "timestamp":"2026-10-01T00:00:00Z",
            "promptId":"turn-dual",
            "permissionMode":"default",
            "prompt":"reply with the single word pong",
            "hook_event_name":"UserPromptSubmit",
            "session_id":"sid-dual",
            "permission_mode":"default",
            "agent_id":null,
            "agentId":null
        }"#;
        let report =
            report_from_payload(TurnEventType::TurnStarted, "run".into(), payload).unwrap();
        assert_eq!(report.native_session_id.as_deref(), Some("sid-dual"));
        assert_eq!(report.native_turn_id.as_deref(), Some("turn-dual"));
        assert_eq!(
            report.title_seed.as_deref(),
            Some("reply with the single word pong")
        );
        assert!(report.title.is_none());
        assert!(report.model.is_none());

        let snake_wins = report_from_payload(
            TurnEventType::TurnStarted,
            "run".into(),
            br#"{"session_id":"from-snake","sessionId":"from-camel","turn_id":"from-snake-turn","promptId":"from-camel-turn","model":"snake-model","modelId":"camel-model"}"#,
        )
        .unwrap();
        assert_eq!(snake_wins.native_session_id.as_deref(), Some("from-snake"));
        assert_eq!(
            snake_wins.native_turn_id.as_deref(),
            Some("from-snake-turn")
        );
        assert_eq!(snake_wins.model.as_deref(), Some("snake-model"));

        let camel_fills_blank = report_from_payload(
            TurnEventType::SessionStarted,
            "run".into(),
            br#"{"session_id":"","sessionId":"from-camel","turn_id":"   ","promptId":"from-camel-turn","session_title":"  ","sessionTitle":"From camel"}"#,
        )
        .unwrap();
        assert_eq!(
            camel_fills_blank.native_session_id.as_deref(),
            Some("from-camel")
        );
        assert_eq!(
            camel_fills_blank.native_turn_id.as_deref(),
            Some("from-camel-turn")
        );
        assert_eq!(camel_fills_blank.title.as_deref(), Some("From camel"));

        let stop = report_from_payload(
            TurnEventType::TurnStopped,
            "run".into(),
            br#"{"session_id":"sid-stop","sessionId":"sid-stop","last_assistant_message":"done","permissionMode":"default","permission_mode":"default","hookEventName":"stop","hook_event_name":"Stop"}"#,
        )
        .unwrap();
        assert_eq!(stop.native_session_id.as_deref(), Some("sid-stop"));
        assert!(stop.title.is_none());
        assert_eq!(stop.notice_body.as_deref(), Some("done"));

        let camel_stop = report_from_payload(
            TurnEventType::TurnStopped,
            "run".into(),
            br#"{"sessionId":"sid-stop","lastAssistantMessage":"camel done"}"#,
        )
        .unwrap();
        assert_eq!(camel_stop.notice_body.as_deref(), Some("camel done"));

        assert!(report_from_payload(
            TurnEventType::TurnStarted,
            "run".into(),
            br#"{"agent_id":null,"agentId":"","session_id":"sid","sessionId":"sid"}"#,
        )
        .is_none());
    }

    #[test]
    fn process_exit_keeps_the_session_id_and_then_revokes() {
        let registry = AgentHookRegistry::new(EventEmitter::Noop);
        let (state, token) = registry
            .register_run("term-1", "claude_code", true)
            .unwrap();
        let token = token.unwrap();
        assert!(registry.accept(
            &token,
            HookReport {
                run_id: state.run_id.clone(),
                event_id: "event-1".into(),
                kind: TurnEventType::SessionStarted,
                native_event: "SessionStart".into(),
                native_session_id: Some("sid-1".into()),
                native_turn_id: None,
                title: Some("Plan".into()),
                title_seed: None,
                model: None,
                notice_body: None,
            },
        ));
        registry.note_process_exit("term-1", &state.run_id);
        let after = registry.state("term-1").unwrap();
        assert!(!after.active);
        let latest = after.latest_event.unwrap();
        assert_eq!(latest.kind, TurnEventType::SessionEnded);
        assert_eq!(latest.native_event, "process_exit");
        assert_eq!(latest.native_session_id.as_deref(), Some("sid-1"));
        let event_id = latest.event_id.clone();
        registry.note_process_exit("term-1", &state.run_id);
        registry.note_process_exit("missing", &state.run_id);
        assert_eq!(
            registry
                .state("term-1")
                .unwrap()
                .latest_event
                .unwrap()
                .event_id,
            event_id
        );
    }

    #[tokio::test]
    async fn loopback_receiver_authenticates_and_keeps_latest_event() {
        let registry = Arc::new(AgentHookRegistry::new(EventEmitter::Noop));
        let url = registry.ensure_listener().await.unwrap();
        let (state, token) = registry.register_run("term-a", "codex", true).unwrap();
        let token = token.unwrap();
        let report = HookReport {
            run_id: state.run_id.clone(),
            event_id: "event-1".into(),
            kind: TurnEventType::TurnStarted,
            native_event: "UserPromptSubmit".into(),
            native_session_id: Some("session-1".into()),
            native_turn_id: Some("turn-1".into()),
            title: None,
            title_seed: None,
            model: None,
            notice_body: None,
        };
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let denied = client
            .post(&url)
            .bearer_auth("wrong-token")
            .json(&report)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(registry.state("term-a").unwrap().latest_event.is_none());
        let accepted = client
            .post(&url)
            .bearer_auth(&token)
            .json(&report)
            .send()
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
        let latest = registry.state("term-a").unwrap().latest_event.unwrap();
        assert_eq!(latest.native_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(latest.kind, TurnEventType::TurnStarted);
        registry.revoke("term-a", &state.run_id);
        let stale = client
            .post(&url)
            .bearer_auth(&token)
            .json(&HookReport {
                event_id: "event-2".into(),
                ..report
            })
            .send()
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            registry
                .state("term-a")
                .unwrap()
                .latest_event
                .unwrap()
                .event_id,
            "event-1"
        );
    }
}

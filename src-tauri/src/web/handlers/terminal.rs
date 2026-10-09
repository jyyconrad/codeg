use std::sync::Arc;

use axum::{extract::Extension, Json};
use serde::Deserialize;

use crate::app_error::AppCommandError;
use crate::app_state::AppState;
use crate::commands::terminal::prepare_credential_env;
use crate::models::agent::AgentType;
use crate::terminal::agent_hooks::AgentTerminalState;
use crate::terminal::agent_launch::{spawn_agent, SpawnAgentResponse};
use crate::terminal::manager::SpawnOptions;
use crate::terminal::types::{TerminalInfo, TerminalSnapshot};

// ---------------------------------------------------------------------------
// Param structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSpawnParams {
    pub working_dir: String,
    pub shell: Option<String>,
    pub initial_command: Option<String>,
    pub terminal_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSpawnAgentParams {
    pub agent_type: AgentType,
    pub working_dir: String,
    pub prompt: String,
    pub terminal_id: String,
    pub shell: Option<String>,
    /// Native session id to resume. Absent on a fresh launch.
    #[serde(default)]
    pub resume_session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalIdParams {
    pub terminal_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalConversationLookupParams {
    pub conversation_id: Option<i32>,
    pub native_session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalWriteParams {
    pub terminal_id: String,
    pub data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResizeParams {
    pub terminal_id: String,
    pub cols: u16,
    pub rows: u16,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn terminal_spawn(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalSpawnParams>,
) -> Result<Json<String>, AppCommandError> {
    let manager = &state.terminal_manager;
    let terminal_id = params
        .terminal_id
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let extra_env = prepare_credential_env(&state.data_dir);

    let id = manager
        .spawn_with_id(
            SpawnOptions {
                terminal_id,
                working_dir: params.working_dir,
                owner_window_label: "web".to_string(),
                shell: params.shell,
                initial_command: params.initial_command,
                extra_env,
                temp_files: vec![],
                on_exit: None,
            },
            state.emitter.clone(),
        )
        .map_err(|e| AppCommandError::task_execution_failed(e.to_string()))?;

    Ok(Json(id))
}

pub async fn terminal_spawn_agent(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalSpawnAgentParams>,
) -> Result<Json<SpawnAgentResponse>, AppCommandError> {
    let runtime_env = crate::commands::acp::build_terminal_runtime_env(
        &state.db,
        params.agent_type,
        params.resume_session_id.as_deref(),
        &state.data_dir,
    )
    .await
    .map_err(|error| AppCommandError::task_execution_failed(error.to_string()))?;
    let response = spawn_agent(
        &state.terminal_manager,
        state.agent_hooks.clone(),
        state.emitter.clone(),
        params.agent_type,
        params.working_dir,
        params.prompt,
        params.resume_session_id,
        params.terminal_id,
        params.shell,
        "web".to_string(),
        &state.data_dir,
        runtime_env,
        false,
    )
    .await
    .map_err(|error| AppCommandError::task_execution_failed(error.to_string()))?;
    Ok(Json(response))
}

pub async fn terminal_id_for_conversation(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalConversationLookupParams>,
) -> Result<Json<Option<String>>, AppCommandError> {
    let mut session_id = params.native_session_id;
    if session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .is_none()
    {
        if let Some(conversation_id) = params.conversation_id.filter(|id| *id > 0) {
            if let Ok(conv) =
                crate::db::service::conversation_service::get_by_id(&state.db.conn, conversation_id)
                    .await
            {
                session_id = conv.external_id;
            }
        }
    }
    Ok(Json(
        crate::chat_channel::terminal_followup::live_terminal_id(
            &state.terminal_manager,
            &state.agent_hooks,
            session_id.as_deref(),
            params.conversation_id,
        ),
    ))
}

pub async fn terminal_agent_state(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalIdParams>,
) -> Result<Json<Option<AgentTerminalState>>, AppCommandError> {
    Ok(Json(state.agent_hooks.state(&params.terminal_id)))
}

pub async fn terminal_write(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalWriteParams>,
) -> Result<Json<()>, AppCommandError> {
    let manager = &state.terminal_manager;
    manager
        .write(&params.terminal_id, params.data.as_bytes())
        .map_err(|e| AppCommandError::task_execution_failed(e.to_string()))?;
    Ok(Json(()))
}

pub async fn terminal_resize(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalResizeParams>,
) -> Result<Json<()>, AppCommandError> {
    let manager = &state.terminal_manager;
    manager
        .resize(&params.terminal_id, params.cols, params.rows)
        .map_err(|e| AppCommandError::task_execution_failed(e.to_string()))?;
    Ok(Json(()))
}

/// Recent output of an already-running terminal — the re-attach half of
/// `terminal_spawn` (see the Tauri command of the same name).
pub async fn terminal_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalIdParams>,
) -> Result<Json<TerminalSnapshot>, AppCommandError> {
    let manager = &state.terminal_manager;
    Ok(Json(manager.snapshot(&params.terminal_id)))
}

pub async fn terminal_kill(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<TerminalIdParams>,
) -> Result<Json<()>, AppCommandError> {
    let manager = &state.terminal_manager;
    manager
        .kill(&params.terminal_id)
        .map_err(|e| AppCommandError::task_execution_failed(e.to_string()))?;
    Ok(Json(()))
}

pub async fn terminal_list(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<Vec<TerminalInfo>>, AppCommandError> {
    let manager = &state.terminal_manager;
    let result = manager.list_with_exit_check(Some(&state.emitter));
    Ok(Json(result))
}

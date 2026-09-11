//! rmcp client + DynamicTool local/remote names.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rig::tool::{DynamicTool, ToolContext, ToolExecutionError, ToolOutput};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, ContentBlock, ServerResult, Tool as RmcpTool,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService, ServiceError};
use rmcp::transport::TokioChildProcess;
use rmcp::{Peer, ServiceExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::NativeToolCtx;
use crate::acp::process_owner::{
    force_kill_and_reap, kill_tree_signal, lock_owners, ProcessOwnerRegistry,
};
use crate::models::agent::AgentType;

const LOCAL_NAME_MAX: usize = 64;

/// Per-server initialize + tools/list budget.
pub const MCP_SERVER_INIT_BUDGET: Duration = Duration::from_secs(10);
/// Whole-session MCP handshake budget. Unfinished servers are cancelled.
pub const MCP_SESSION_INIT_BUDGET: Duration = Duration::from_secs(20);
/// Cap on simultaneous stdio handshakes.
pub const MCP_MAX_CONCURRENT_INIT: usize = 4;
/// Default tools/call budget.
pub const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Hard cap on tools/call.
pub const MCP_CALL_TIMEOUT_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum McpNameError {
    #[error(
        "MCP local name `{local}` collides ({server_key}/{remote_name} vs existing {existing_server}/{existing_remote}); refusing to overwrite"
    )]
    Collision {
        local: String,
        server_key: String,
        remote_name: String,
        existing_server: String,
        existing_remote: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum McpConnectError {
    #[error("mcp child spawn: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("mcp initialize: {0}")]
    Initialize(String),
}

/// Handshake and call budgets for one native session.
#[derive(Debug, Clone, Copy)]
pub struct McpTimeouts {
    pub per_server_init: Duration,
    pub session_init: Duration,
    pub max_concurrent_init: usize,
    pub call: Duration,
}

impl Default for McpTimeouts {
    fn default() -> Self {
        Self {
            per_server_init: MCP_SERVER_INIT_BUDGET,
            session_init: MCP_SESSION_INIT_BUDGET,
            max_concurrent_init: MCP_MAX_CONCURRENT_INIT,
            call: MCP_CALL_TIMEOUT.min(MCP_CALL_TIMEOUT_MAX),
        }
    }
}

impl McpTimeouts {
    pub fn capped_call(self) -> Duration {
        self.call.min(MCP_CALL_TIMEOUT_MAX)
    }
}

/// Local Rig name ↔ remote MCP identity. Schema/annotations are the peer's.
#[derive(Debug, Clone)]
pub struct McpToolBinding {
    pub local_name: String,
    pub server_key: String,
    pub remote_name: String,
    pub description: String,
    pub parameters: Value,
    pub annotations: Option<Value>,
    pub read_only: bool,
}

impl McpToolBinding {
    pub fn schema(&self) -> Value {
        json!({
            "name": self.local_name,
            "description": self.description,
            "parameters": self.parameters,
        })
    }
}

/// `readOnlyHint == true` is the only MCP skip; missing/false needs a card.
pub fn mcp_tool_requires_permission(read_only_hint: Option<bool>) -> bool {
    read_only_hint != Some(true)
}

/// Allocates unique local Rig names for MCP tools. Never overwrites a collision.
#[derive(Debug, Default)]
pub struct McpNameAllocator {
    used: BTreeMap<String, (String, String)>,
}

impl McpNameAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(
        &mut self,
        server_key: &str,
        remote_name: &str,
    ) -> Result<String, McpNameError> {
        let local = canonical_local_name(server_key, remote_name);
        if let Some((existing_server, existing_remote)) = self.used.get(&local) {
            if existing_server == server_key && existing_remote == remote_name {
                return Ok(local);
            }
            tracing::warn!(
                local,
                server_key,
                remote_name,
                existing_server,
                existing_remote,
                "MCP local tool name collision; refusing to overwrite"
            );
            return Err(McpNameError::Collision {
                local,
                server_key: server_key.to_string(),
                remote_name: remote_name.to_string(),
                existing_server: existing_server.clone(),
                existing_remote: existing_remote.clone(),
            });
        }
        self.used.insert(
            local.clone(),
            (server_key.to_string(), remote_name.to_string()),
        );
        Ok(local)
    }
}

/// `{server_key}__{tool}` when that is ASCII alnum/_/- and ≤64; otherwise a
/// digest-suffixed sanitized form. Collisions are the allocator's job.
pub fn canonical_local_name(server_key: &str, remote_name: &str) -> String {
    let candidate = format!("{server_key}__{remote_name}");
    if is_legal_name(&candidate) {
        return candidate;
    }
    let digest = name_digest(server_key, remote_name);
    let mut base: String = candidate
        .chars()
        .map(|c| if is_legal_char(c) { c } else { '_' })
        .collect();
    if base.is_empty() {
        base.push('_');
    }
    let budget = LOCAL_NAME_MAX.saturating_sub(1 + digest.len());
    let mut base: String = base.chars().take(budget).collect();
    if base.is_empty() {
        base.push('_');
    }
    format!("{base}_{digest}")
}

fn is_legal_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn is_legal_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= LOCAL_NAME_MAX && s.chars().all(is_legal_char)
}

fn name_digest(server_key: &str, remote_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(server_key.as_bytes());
    hasher.update([0u8]);
    hasher.update(remote_name.as_bytes());
    let out = hasher.finalize();
    format!(
        "{:08x}",
        u32::from_be_bytes(out[0..4].try_into().expect("sha256 prefix"))
    )
}

/// Live stdio MCP session. Keep this value so the child is not dropped while
/// cloned [`Peer`]s are used from DynamicTool callbacks.
pub struct McpClient {
    service: RunningService<RoleClient, ()>,
    pid: u32,
}

impl McpClient {
    pub async fn connect_child(command: tokio::process::Command) -> Result<Self, McpConnectError> {
        Self::connect_child_with_owners(command, None).await
    }

    /// Spawn the stdio child, register its pid with the native owner set, then
    /// handshake. Registration happens before initialize so Disconnect can reap
    /// a server that never finished `tools/list`.
    pub async fn connect_child_with_owners(
        command: tokio::process::Command,
        owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    ) -> Result<Self, McpConnectError> {
        Self::connect_child_with_owners_pid(command, owners, None).await
    }

    async fn connect_child_with_owners_pid(
        command: tokio::process::Command,
        owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
        spawned: Option<Arc<std::sync::atomic::AtomicU32>>,
    ) -> Result<Self, McpConnectError> {
        let transport = TokioChildProcess::new(command)?;
        let pid = transport.id().unwrap_or(0);
        if let Some(spawned) = &spawned {
            spawned.store(pid, Ordering::SeqCst);
        }
        if pid != 0 {
            if let Some(owners) = &owners {
                lock_owners(owners).register(pid);
            }
        }
        match ().serve(transport).await {
            Ok(service) => Ok(Self { service, pid }),
            Err(err) => {
                if pid != 0 {
                    kill_tree_signal(pid, "SIGKILL");
                    if let Some(owners) = &owners {
                        lock_owners(owners).unregister(pid);
                    }
                }
                Err(McpConnectError::Initialize(err.to_string()))
            }
        }
    }

    pub fn peer(&self) -> Peer<RoleClient> {
        self.service.peer().clone()
    }

    pub fn child_pid(&self) -> u32 {
        self.pid
    }
}

struct McpServerSlot {
    key: String,
    peer: Peer<RoleClient>,
    pid: u32,
    live: AtomicBool,
    client: Mutex<Option<McpClient>>,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
}

impl McpServerSlot {
    fn is_live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    async fn retire(&self) {
        if !self.live.swap(false, Ordering::SeqCst) {
            return;
        }
        let pid = {
            let mut guard = match self.client.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(client) = guard.take() {
                let pid = client.pid;
                drop(client);
                pid
            } else {
                0
            }
        };
        if pid == 0 {
            return;
        }
        // Keep the pid registered until the OS has reaped it so shutdown
        // `force_kill_owners` can still find a live child.
        let leftover = force_kill_and_reap(&[pid], Duration::from_secs(2)).await;
        if leftover.is_empty() {
            if let Some(owners) = &self.owners {
                lock_owners(owners).unregister(pid);
            }
        }
    }
}

/// Session-scoped stdio MCP clients and the DynamicTools they back.
pub struct McpSession {
    slots: HashMap<String, Arc<McpServerSlot>>,
    bindings: Vec<McpToolBinding>,
    timeouts: McpTimeouts,
    pub warnings: Vec<String>,
}

impl McpSession {
    pub fn empty() -> Self {
        Self {
            slots: HashMap::new(),
            bindings: Vec::new(),
            timeouts: McpTimeouts::default(),
            warnings: Vec::new(),
        }
    }

    pub fn bindings(&self) -> &[McpToolBinding] {
        &self.bindings
    }

    pub fn readonly_local_names(&self) -> HashSet<String> {
        self.bindings
            .iter()
            .filter(|binding| binding.read_only)
            .map(|binding| binding.local_name.clone())
            .collect()
    }

    pub fn live_pids(&self) -> Vec<u32> {
        self.slots
            .values()
            .map(|slot| slot.pid)
            .filter(|pid| *pid != 0)
            .collect()
    }

    pub fn dynamic_tools(&self, tool_ctx: NativeToolCtx) -> Vec<DynamicTool> {
        let timeout = self.timeouts.capped_call();
        self.bindings
            .iter()
            .filter_map(|binding| {
                let slot = self.slots.get(&binding.server_key)?.clone();
                Some(session_mcp_tool(
                    binding.clone(),
                    slot,
                    tool_ctx.clone(),
                    timeout,
                ))
            })
            .collect()
    }

    /// Load `read_servers_for_agent_type` and handshake stdio servers.
    pub async fn connect_for_agent(
        agent_type: AgentType,
        owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
        shutdown: CancellationToken,
        timeouts: McpTimeouts,
    ) -> Self {
        let specs = match crate::commands::mcp::read_servers_for_agent_type(agent_type) {
            Ok(specs) => specs,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "[ACP] failed to read MCP servers for {agent_type}; continuing without them"
                );
                BTreeMap::new()
            }
        };
        Self::connect_specs(specs, owners, shutdown, timeouts).await
    }

    /// Handshake the given specs (stdio only). HTTP/SSE are warned and skipped.
    pub async fn connect_specs(
        specs: BTreeMap<String, Value>,
        owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
        shutdown: CancellationToken,
        timeouts: McpTimeouts,
    ) -> Self {
        let mut warnings = Vec::new();
        let mut stdio = Vec::new();
        for (key, spec) in specs {
            match stdio_command_from_spec(&key, &spec) {
                Ok(cmd) => stdio.push((key, cmd)),
                Err(reason) => {
                    tracing::warn!(server_key = %key, "{reason}");
                    warnings.push(format!("{key}: {reason}"));
                }
            }
        }

        let session_deadline = tokio::time::Instant::now() + timeouts.session_init;
        let limiter = Arc::new(Semaphore::new(timeouts.max_concurrent_init.max(1)));
        let names = Arc::new(Mutex::new(McpNameAllocator::new()));
        let mut join = tokio::task::JoinSet::new();
        for (key, cmd) in stdio {
            let owners = owners.clone();
            let limiter = Arc::clone(&limiter);
            let names = Arc::clone(&names);
            let per_server = timeouts.per_server_init;
            join.spawn(async move {
                let _permit = limiter
                    .acquire_owned()
                    .await
                    .map_err(|_| format!("{key}: MCP init cancelled"))?;
                timeout_connect_one(key, cmd, owners, names, per_server).await
            });
        }

        let mut slots = HashMap::new();
        let mut bindings = Vec::new();
        loop {
            if shutdown.is_cancelled() || tokio::time::Instant::now() >= session_deadline {
                join.abort_all();
                warnings.push("MCP init cancelled or exceeded session budget".into());
                break;
            }
            let remaining = session_deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::select! {
                _ = shutdown.cancelled() => {
                    join.abort_all();
                    warnings.push("MCP init cancelled".into());
                    break;
                }
                _ = tokio::time::sleep(remaining) => {
                    join.abort_all();
                    warnings.push("MCP init exceeded session budget".into());
                    break;
                }
                joined = join.join_next() => {
                    match joined {
                        None => break,
                        Some(Ok(Ok(ready))) => {
                            slots.insert(ready.slot.key.clone(), Arc::clone(&ready.slot));
                            bindings.extend(ready.bindings);
                        }
                        Some(Ok(Err(warn))) => {
                            tracing::warn!("{warn}");
                            warnings.push(warn);
                        }
                        Some(Err(err)) => {
                            if !err.is_cancelled() {
                                tracing::warn!("MCP init task failed: {err}");
                                warnings.push(format!("MCP init task failed: {err}"));
                            }
                        }
                    }
                }
            }
        }

        reap_unowned_mcp_pids(&owners, &slots).await;

        Self {
            slots,
            bindings,
            timeouts,
            warnings,
        }
    }

    pub async fn call(
        &self,
        local_name: &str,
        arguments: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolExecutionError> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.local_name == local_name)
            .ok_or_else(|| {
                ToolExecutionError::invalid_args(format!("unknown MCP tool '{local_name}'"))
                    .with_model_feedback(format!("unknown MCP tool '{local_name}'"))
            })?;
        let slot = self
            .slots
            .get(&binding.server_key)
            .ok_or_else(|| unavailable(&binding.server_key))?;
        call_on_slot(
            Arc::clone(slot),
            binding.remote_name.clone(),
            arguments,
            cancel,
            self.timeouts.capped_call(),
        )
        .await
        .and_then(|result| mcp_result_to_output(binding.remote_name.clone(), result))
    }

    pub async fn close(&self) {
        for slot in self.slots.values() {
            slot.retire().await;
        }
        let pids: Vec<u32> = self
            .slots
            .values()
            .map(|slot| slot.pid)
            .filter(|pid| *pid != 0)
            .collect();
        let leftover = force_kill_and_reap(&pids, Duration::from_secs(2)).await;
        if leftover.is_empty() {
            if let Some(owners) = self.slots.values().find_map(|slot| slot.owners.clone()) {
                let mut owners = lock_owners(&owners);
                for pid in pids {
                    owners.unregister(pid);
                }
            }
        }
    }
}

struct ReadyServer {
    slot: Arc<McpServerSlot>,
    bindings: Vec<McpToolBinding>,
}

async fn timeout_connect_one(
    key: String,
    cmd: tokio::process::Command,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    names: Arc<Mutex<McpNameAllocator>>,
    budget: Duration,
) -> Result<ReadyServer, String> {
    let spawned = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let owners_for_connect = owners.clone();
    match tokio::time::timeout(
        budget,
        connect_and_list(key.clone(), cmd, owners_for_connect, Arc::clone(&spawned)),
    )
    .await
    {
        Ok(Ok(ready)) => Ok(ready_with_names(ready, names.as_ref())),
        Ok(Err(err)) => Err(format!("{key}: {err}")),
        Err(_) => {
            let pid = spawned.load(Ordering::SeqCst);
            if pid != 0 {
                kill_tree_signal(pid, "SIGKILL");
                let _ = force_kill_and_reap(&[pid], Duration::from_secs(2)).await;
                if let Some(owners) = &owners {
                    lock_owners(owners).unregister(pid);
                }
            }
            Err(format!("{key}: initialize timed out"))
        }
    }
}

struct ListedServer {
    key: String,
    client: McpClient,
    tools: Vec<RmcpTool>,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
}

async fn connect_and_list(
    key: String,
    cmd: tokio::process::Command,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    spawned: Arc<std::sync::atomic::AtomicU32>,
) -> Result<ListedServer, String> {
    let client = McpClient::connect_child_with_owners_pid(cmd, owners.clone(), Some(spawned))
        .await
        .map_err(|err| err.to_string())?;
    let tools = client.peer().list_all_tools().await.map_err(|err| {
        let pid = client.child_pid();
        if pid != 0 {
            kill_tree_signal(pid, "SIGKILL");
            if let Some(owners) = &owners {
                lock_owners(owners).unregister(pid);
            }
        }
        err.to_string()
    })?;
    Ok(ListedServer {
        key,
        client,
        tools,
        owners,
    })
}

fn ready_with_names(listed: ListedServer, names: &Mutex<McpNameAllocator>) -> ReadyServer {
    let peer = listed.client.peer();
    let pid = listed.client.child_pid();
    let slot = Arc::new(McpServerSlot {
        key: listed.key.clone(),
        peer,
        pid,
        live: AtomicBool::new(true),
        client: Mutex::new(Some(listed.client)),
        owners: listed.owners,
    });
    let mut bindings = Vec::new();
    let mut allocator = names.lock().expect("mcp name allocator");
    for tool in listed.tools {
        let remote_name = tool.name.clone().into_owned();
        let local = match allocator.allocate(&listed.key, &remote_name) {
            Ok(local) => local,
            Err(err) => {
                tracing::warn!("{err}");
                continue;
            }
        };
        let read_only = !mcp_tool_requires_permission(
            tool.annotations.as_ref().and_then(|ann| ann.read_only_hint),
        );
        let parameters = Value::Object((*tool.input_schema).clone());
        let description = tool
            .description
            .clone()
            .map(|d| d.into_owned())
            .unwrap_or_default();
        let annotations = tool
            .annotations
            .as_ref()
            .and_then(|ann| serde_json::to_value(ann).ok());
        bindings.push(McpToolBinding {
            local_name: local,
            server_key: listed.key.clone(),
            remote_name,
            description,
            parameters,
            annotations,
            read_only,
        });
    }
    ReadyServer { slot, bindings }
}

fn stdio_command_from_spec(key: &str, spec: &Value) -> Result<tokio::process::Command, String> {
    let ty = spec.get("type").and_then(Value::as_str).unwrap_or("stdio");
    if ty != "stdio" {
        return Err(format!(
            "skipping {ty} MCP server '{key}' (Codeg Agent v1 stdio only)"
        ));
    }
    let command = spec
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("stdio MCP server '{key}' is missing command"))?;
    let mut cmd = tokio::process::Command::new(command);
    if let Some(args) = spec.get("args").and_then(Value::as_array) {
        for arg in args {
            if let Some(value) = arg.as_str() {
                cmd.arg(value);
            }
        }
    }
    if let Some(env) = spec.get("env").and_then(Value::as_object) {
        for (name, value) in env {
            if let Some(value) = value.as_str() {
                cmd.env(name, value);
            }
        }
    }
    if let Some(cwd) = spec
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        cmd.current_dir(cwd);
    }
    cmd.kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    Ok(cmd)
}

async fn reap_unowned_mcp_pids(
    owners: &Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    slots: &HashMap<String, Arc<McpServerSlot>>,
) {
    let keep: HashSet<u32> = slots
        .values()
        .map(|slot| slot.pid)
        .filter(|pid| *pid != 0)
        .collect();
    reap_new_pids(owners, &keep.into_iter().collect::<Vec<_>>()).await;
}

async fn reap_new_pids(owners: &Option<Arc<Mutex<ProcessOwnerRegistry>>>, keep: &[u32]) {
    let Some(owners) = owners else {
        return;
    };
    let extras: Vec<u32> = lock_owners(owners)
        .pids()
        .into_iter()
        .filter(|pid| !keep.contains(pid))
        .collect();
    if extras.is_empty() {
        return;
    }
    let leftover = force_kill_and_reap(&extras, Duration::from_secs(2)).await;
    let mut owners = lock_owners(owners);
    for pid in extras {
        if !leftover.contains(&pid) {
            owners.unregister(pid);
        }
    }
}

async fn call_on_slot(
    slot: Arc<McpServerSlot>,
    remote_name: String,
    arguments: Value,
    cancel: CancellationToken,
    timeout: Duration,
) -> Result<CallToolResult, ToolExecutionError> {
    if !slot.is_live() {
        return Err(unavailable(&slot.key));
    }
    if cancel.is_cancelled() {
        return Err(ToolExecutionError::cancelled("MCP tool was cancelled")
            .with_model_feedback("MCP tool was cancelled"));
    }
    let mut request = CallToolRequestParams::new(remote_name.clone());
    match arguments {
        Value::Null => {}
        Value::Object(map) => {
            request = request.with_arguments(map);
        }
        other => {
            return Err(ToolExecutionError::invalid_args(format!(
                "MCP tool '{remote_name}' expects a JSON object, got {other}"
            ))
            .with_model_feedback(format!("MCP tool '{remote_name}' expects a JSON object")));
        }
    }
    let handle = match slot
        .peer
        .send_cancellable_request(
            ClientRequest::CallToolRequest(CallToolRequest::new(request)),
            PeerRequestOptions::with_timeout(timeout).with_max_total_timeout(timeout),
        )
        .await
    {
        Ok(handle) => handle,
        Err(err) => {
            slot.retire().await;
            return Err(ToolExecutionError::provider(format!(
                "MCP tool '{remote_name}' request failed: {err}"
            ))
            .with_model_feedback(format!("MCP tool '{remote_name}' failed")));
        }
    };
    let request_id = handle.id.clone();
    let peer = handle.peer.clone();
    tokio::select! {
        result = handle.await_response() => match result {
            Ok(ServerResult::CallToolResult(result)) => Ok(result),
            Ok(_) => Err(
                ToolExecutionError::provider(format!(
                    "MCP tool '{remote_name}' returned an unexpected response"
                ))
                .with_model_feedback(format!("MCP tool '{remote_name}' failed")),
            ),
            Err(ServiceError::Timeout { .. }) => {
                slot.retire().await;
                Err(
                    ToolExecutionError::timeout(format!(
                        "MCP tool '{remote_name}' timed out"
                    ))
                    .with_model_feedback(format!("MCP tool '{remote_name}' timed out")),
                )
            }
            Err(ServiceError::Cancelled { .. }) => {
                slot.retire().await;
                Err(
                    ToolExecutionError::cancelled(format!(
                        "MCP tool '{remote_name}' was cancelled"
                    ))
                    .with_model_feedback(format!("MCP tool '{remote_name}' was cancelled")),
                )
            }
            Err(ServiceError::TransportClosed) => {
                slot.retire().await;
                Err(unavailable(&slot.key))
            }
            Err(err) => {
                slot.retire().await;
                Err(
                    ToolExecutionError::provider(format!(
                        "MCP tool '{remote_name}' request failed: {err}"
                    ))
                    .with_model_feedback(format!("MCP tool '{remote_name}' failed")),
                )
            }
        },
        _ = cancel.cancelled() => {
            let _ = peer
                .notify_cancelled(CancelledNotificationParam::new(
                    Some(request_id),
                    Some("cancelled".into()),
                ))
                .await;
            slot.retire().await;
            Err(
                ToolExecutionError::cancelled(format!(
                    "MCP tool '{remote_name}' was cancelled"
                ))
                .with_model_feedback(format!("MCP tool '{remote_name}' was cancelled")),
            )
        }
    }
}

fn session_mcp_tool(
    binding: McpToolBinding,
    slot: Arc<McpServerSlot>,
    tool_ctx: NativeToolCtx,
    timeout: Duration,
) -> DynamicTool {
    let local_name = binding.local_name.clone();
    let remote_name = binding.remote_name.clone();
    DynamicTool::new(
        binding.local_name.clone(),
        binding.description.clone(),
        binding.parameters.clone(),
        move |_context: &mut ToolContext, arguments: Value| {
            let slot = Arc::clone(&slot);
            let tool_ctx = tool_ctx.clone();
            let local_name = local_name.clone();
            let remote_name = remote_name.clone();
            Box::pin(async move {
                let fact = tool_ctx.begin(&local_name, arguments.clone()).await?;
                let result = call_on_slot(
                    slot,
                    remote_name.clone(),
                    arguments,
                    tool_ctx.cancel.clone(),
                    timeout,
                )
                .await;
                match result {
                    Ok(result) => match mcp_result_to_output(remote_name, result) {
                        Ok(output) => {
                            let text = output.as_text().unwrap_or("").to_string();
                            tool_ctx.finish_ok(fact, text.clone()).await?;
                            Ok(ToolOutput::text(text))
                        }
                        Err(err) => Err(tool_ctx.finish_err(fact, err).await),
                    },
                    Err(err) => Err(tool_ctx.finish_err(fact, err).await),
                }
            })
        },
    )
}

fn unavailable(server_key: &str) -> ToolExecutionError {
    ToolExecutionError::provider(format!(
        "MCP server '{server_key}' is unavailable after a cancelled or timed-out call"
    ))
    .with_model_feedback(format!(
        "MCP server '{server_key}' is unavailable; the previous call was not confirmed stopped"
    ))
}

/// Call MCP using the **original remote name**, never the local Rig name.
pub fn call_remote_mcp_tool(
    peer: Peer<RoleClient>,
    original_remote_name: String,
    arguments: Value,
) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolExecutionError>> + Send>> {
    Box::pin(async move {
        let mut request = CallToolRequestParams::new(original_remote_name.clone());
        match arguments {
            Value::Null => {}
            Value::Object(map) => {
                request = request.with_arguments(map);
            }
            other => {
                return Err(ToolExecutionError::invalid_args(format!(
                    "MCP tool '{original_remote_name}' expects a JSON object, got {other}"
                )));
            }
        }
        let result = peer.call_tool(request).await.map_err(|err| {
            ToolExecutionError::provider(format!(
                "MCP tool '{original_remote_name}' request failed: {err}"
            ))
            .with_model_feedback(format!("MCP tool '{original_remote_name}' failed"))
        })?;
        mcp_result_to_output(original_remote_name, result)
    })
}

/// DynamicTool whose Rig registry name is `local_name` while the MCP wire
/// name stays `original_remote_name`.
pub fn mcp_dynamic_tool(
    local_name: impl Into<String>,
    description: impl Into<String>,
    parameters: Value,
    peer: Peer<RoleClient>,
    original_remote_name: impl Into<String>,
) -> DynamicTool {
    let original_remote_name = original_remote_name.into();
    DynamicTool::new(
        local_name,
        description,
        parameters,
        move |_context: &mut ToolContext, arguments: Value| {
            call_remote_mcp_tool(peer.clone(), original_remote_name.clone(), arguments)
        },
    )
}

fn mcp_result_to_output(
    remote_name: String,
    result: CallToolResult,
) -> Result<ToolOutput, ToolExecutionError> {
    let text = mcp_result_text(&result);
    if result.is_error.unwrap_or(false) {
        return Err(ToolExecutionError::provider(format!(
            "MCP tool '{remote_name}' returned is_error"
        ))
        .with_model_feedback(text));
    }
    Ok(ToolOutput::text(text))
}

fn mcp_result_text(result: &CallToolResult) -> String {
    let mut parts = Vec::new();
    for block in &result.content {
        parts.push(content_block_text(block));
    }
    if let Some(value) = &result.structured_content {
        parts.push(value.to_string());
    }
    parts.retain(|part| !part.is_empty());
    parts.join("\n")
}

fn content_block_text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(text) => text.text.clone(),
        ContentBlock::Image(_) => "[omitted: image content]".to_string(),
        ContentBlock::Audio(_) => "[omitted: audio content]".to_string(),
        ContentBlock::Resource(resource) => {
            let text = resource.get_text();
            if text.is_empty() {
                "[omitted: embedded resource]".to_string()
            } else {
                text
            }
        }
        ContentBlock::ResourceLink(link) => format!("[resource {}]", link.uri),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legal_server_tool_keeps_readable_name() {
        assert_eq!(canonical_local_name("alpha", "echo"), "alpha__echo");
    }

    #[test]
    fn illegal_or_long_names_get_digest_suffix() {
        let local = canonical_local_name("weird server", "echo tool");
        assert!(is_legal_name(&local), "{local}");
        assert!(local.len() <= LOCAL_NAME_MAX);
        assert_ne!(local, "weird server__echo tool");

        let long_tool = "x".repeat(80);
        let local = canonical_local_name("alpha", &long_tool);
        assert!(is_legal_name(&local));
        assert!(local.len() <= LOCAL_NAME_MAX);
    }

    #[test]
    fn two_servers_same_remote_stay_distinct_and_idempotent() {
        let mut names = McpNameAllocator::new();
        assert_eq!(
            names.allocate("alpha", "echo").expect("alpha"),
            "alpha__echo"
        );
        assert_eq!(
            names.allocate("bravo", "echo").expect("bravo"),
            "bravo__echo"
        );
        assert_eq!(
            names.allocate("alpha", "echo").expect("idempotent"),
            "alpha__echo"
        );
    }

    #[test]
    fn normalized_name_collision_refuses_overwrite() {
        let mut names = McpNameAllocator::new();
        assert_eq!(names.allocate("a", "b__c").expect("first"), "a__b__c");
        let err = names.allocate("a__b", "c").expect_err("collision");
        match err {
            McpNameError::Collision { local, .. } => assert_eq!(local, "a__b__c"),
        }
    }

    #[test]
    fn read_only_hint_is_the_only_mcp_permission_skip() {
        assert!(!mcp_tool_requires_permission(Some(true)));
        assert!(mcp_tool_requires_permission(Some(false)));
        assert!(mcp_tool_requires_permission(None));
    }

    const MCP_FIXTURE: &str = r#"
import json, sys, time

mode = sys.argv[1] if len(sys.argv) > 1 else "echo"
server = sys.argv[2] if len(sys.argv) > 2 else mode

def read_msg():
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    return json.loads(line.decode("utf-8"))

def send_msg(obj):
    sys.stdout.buffer.write(json.dumps(obj).encode("utf-8") + b"\n")
    sys.stdout.buffer.flush()

if mode == "crash":
    sys.exit(1)

echo_schema = {
    "type": "object",
    "properties": {"text": {"type": "string"}},
    "required": ["text"],
}

while True:
    msg = read_msg()
    if msg is None:
        break
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        if mode == "hang-init":
            time.sleep(3600)
        version = (msg.get("params") or {}).get("protocolVersion", "2024-11-05")
        send_msg({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": server, "version": "0.0.1"},
            },
        })
    elif method == "notifications/initialized":
        pass
    elif method == "ping":
        send_msg({"jsonrpc": "2.0", "id": req_id, "result": {}})
    elif method == "tools/list":
        tools = [{
            "name": "echo",
            "description": "echo text",
            "inputSchema": echo_schema,
        }]
        if mode == "annotated":
            tools = [
                {
                    "name": "echo",
                    "description": "echo text",
                    "inputSchema": echo_schema,
                    "annotations": {"readOnlyHint": True, "title": "Echo"},
                },
                {
                    "name": "write",
                    "description": "write text",
                    "inputSchema": echo_schema,
                    "annotations": {"readOnlyHint": False},
                },
            ]
        send_msg({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"tools": tools},
        })
    elif method == "tools/call":
        if mode == "hang-call":
            time.sleep(3600)
        params = msg.get("params") or {}
        name = params.get("name")
        args = params.get("arguments") or {}
        text = args.get("text", "")
        if mode == "error":
            send_msg({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "isError": True,
                    "content": [{"type": "text", "text": f"{server}:{name}:boom"}],
                },
            })
        elif mode == "image":
            send_msg({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [
                        {"type": "text", "text": f"{server}:{name}:{text}"},
                        {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
                    ]
                },
            })
        else:
            send_msg({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": f"{server}:{name}:{text}"}]
                },
            })
    elif method == "shutdown":
        send_msg({"jsonrpc": "2.0", "id": req_id, "result": None})
        break
    elif req_id is not None:
        send_msg({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {"code": -32601, "message": f"unknown method {method}"},
        })
"#;

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_stdio_mcp_keeps_original_remote_names() {
        use std::io::Write;
        use std::process::Stdio;
        use std::time::Duration;

        let python = ["python3", "python"]
            .into_iter()
            .find_map(|bin| which::which(bin).ok())
            .expect("python3 is required for the MCP stdio fixture");
        let mut script = tempfile::NamedTempFile::new().expect("mcp fixture");
        script
            .write_all(MCP_FIXTURE.as_bytes())
            .expect("write fixture");
        script.flush().expect("flush fixture");

        async fn spawn_server(
            python: &std::path::Path,
            script: &std::path::Path,
            server: &str,
            owners: Arc<Mutex<ProcessOwnerRegistry>>,
        ) -> McpClient {
            let mut cmd = tokio::process::Command::new(python);
            cmd.arg("-u")
                .arg(script)
                .arg("echo")
                .arg(server)
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            tokio::time::timeout(
                Duration::from_secs(10),
                McpClient::connect_child_with_owners(cmd, Some(owners)),
            )
            .await
            .unwrap_or_else(|_| panic!("timed out connecting MCP server {server}"))
            .unwrap_or_else(|err| panic!("connect {server}: {err}"))
        }

        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let alpha = spawn_server(&python, script.path(), "alpha", Arc::clone(&owners)).await;
        let bravo = spawn_server(&python, script.path(), "bravo", Arc::clone(&owners)).await;
        assert!(
            alpha.child_pid() != 0 && bravo.child_pid() != 0,
            "MCP stdio children must publish pids"
        );
        let registered = lock_owners(&owners).pids();
        assert!(
            registered.contains(&alpha.child_pid()) && registered.contains(&bravo.child_pid()),
            "MCP pids must be registered before initialize returns: {registered:?}"
        );
        let mut names = McpNameAllocator::new();
        let alpha_local = names.allocate("alpha", "echo").expect("alpha local");
        let bravo_local = names.allocate("bravo", "echo").expect("bravo local");
        assert_eq!(alpha_local, "alpha__echo");
        assert_eq!(bravo_local, "bravo__echo");
        assert_ne!(alpha_local, bravo_local);

        let schema = serde_json::json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"]
        });
        let _alpha_tool = mcp_dynamic_tool(
            alpha_local,
            "echo text",
            schema.clone(),
            alpha.peer(),
            "echo",
        );
        let _bravo_tool = mcp_dynamic_tool(bravo_local, "echo text", schema, bravo.peer(), "echo");

        let alpha_out = tokio::time::timeout(
            Duration::from_secs(10),
            call_remote_mcp_tool(
                alpha.peer(),
                "echo".into(),
                serde_json::json!({"text": "hi-a"}),
            ),
        )
        .await
        .expect("alpha call timeout")
        .expect("alpha call");
        let bravo_out = tokio::time::timeout(
            Duration::from_secs(10),
            call_remote_mcp_tool(
                bravo.peer(),
                "echo".into(),
                serde_json::json!({"text": "hi-b"}),
            ),
        )
        .await
        .expect("bravo call timeout")
        .expect("bravo call");

        assert_eq!(alpha_out.as_text(), Some("alpha:echo:hi-a"));
        assert_eq!(bravo_out.as_text(), Some("bravo:echo:hi-b"));

        let _ = alpha;
        let _ = bravo;
        let _ = script;
    }

    fn python_bin() -> std::path::PathBuf {
        ["python3", "python"]
            .into_iter()
            .find_map(|bin| which::which(bin).ok())
            .expect("python3 is required for the MCP stdio fixture")
    }

    fn write_fixture() -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut script = tempfile::NamedTempFile::new().expect("mcp fixture");
        script
            .write_all(MCP_FIXTURE.as_bytes())
            .expect("write fixture");
        script.flush().expect("flush fixture");
        script
    }

    fn stdio_spec(
        python: &std::path::Path,
        script: &std::path::Path,
        mode: &str,
        server: &str,
    ) -> Value {
        json!({
            "type": "stdio",
            "command": python.to_string_lossy(),
            "args": ["-u", script.to_string_lossy(), mode, server],
        })
    }

    fn test_timeouts() -> McpTimeouts {
        McpTimeouts {
            per_server_init: Duration::from_millis(800),
            session_init: Duration::from_millis(1500),
            max_concurrent_init: 4,
            call: Duration::from_millis(800),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_and_sse_specs_are_skipped() {
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let mut specs = BTreeMap::new();
        specs.insert(
            "remote".into(),
            json!({"type": "http", "url": "https://example.test/mcp"}),
        );
        specs.insert(
            "events".into(),
            json!({"type": "sse", "url": "https://example.test/sse"}),
        );
        let session = McpSession::connect_specs(
            specs,
            Some(owners),
            CancellationToken::new(),
            test_timeouts(),
        )
        .await;
        assert!(session.bindings().is_empty());
        assert!(
            session.warnings.iter().any(|w| w.contains("http")),
            "{:?}",
            session.warnings
        );
        assert!(
            session.warnings.iter().any(|w| w.contains("sse")),
            "{:?}",
            session.warnings
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bad_and_hung_servers_are_skipped_and_reaped() {
        use crate::acp::process_owner::pid_is_alive;

        let python = python_bin();
        let script = write_fixture();
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let mut specs = BTreeMap::new();
        specs.insert(
            "good".into(),
            stdio_spec(&python, script.path(), "echo", "good"),
        );
        specs.insert(
            "hung".into(),
            stdio_spec(&python, script.path(), "hang-init", "hung"),
        );
        specs.insert(
            "crash".into(),
            stdio_spec(&python, script.path(), "crash", "crash"),
        );
        let session = McpSession::connect_specs(
            specs,
            Some(Arc::clone(&owners)),
            CancellationToken::new(),
            test_timeouts(),
        )
        .await;
        assert!(
            session
                .bindings()
                .iter()
                .any(|b| b.local_name == "good__echo"),
            "good server must still register: {:?}",
            session
                .bindings()
                .iter()
                .map(|b| &b.local_name)
                .collect::<Vec<_>>()
        );
        assert!(
            session.warnings.iter().any(|w| w.contains("hung")),
            "hung server must be skipped: {:?}",
            session.warnings
        );
        let keep = session.live_pids();
        for pid in lock_owners(&owners).pids() {
            if keep.contains(&pid) {
                continue;
            }
            assert!(!pid_is_alive(pid), "failed MCP pid {pid} must be reaped");
        }
        session.close().await;
        let _ = script;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn annotations_and_read_only_are_preserved() {
        let python = python_bin();
        let script = write_fixture();
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let mut specs = BTreeMap::new();
        specs.insert(
            "ann".into(),
            stdio_spec(&python, script.path(), "annotated", "ann"),
        );
        let session = McpSession::connect_specs(
            specs,
            Some(owners),
            CancellationToken::new(),
            test_timeouts(),
        )
        .await;
        let echo = session
            .bindings()
            .iter()
            .find(|b| b.remote_name == "echo")
            .expect("echo");
        let write = session
            .bindings()
            .iter()
            .find(|b| b.remote_name == "write")
            .expect("write");
        assert!(echo.read_only);
        assert!(!mcp_tool_requires_permission(Some(true)));
        assert!(!write.read_only);
        assert_eq!(
            echo.parameters["properties"]["text"]["type"],
            json!("string")
        );
        assert_eq!(
            echo.annotations.as_ref().unwrap()["readOnlyHint"],
            json!(true)
        );
        let expected: HashSet<String> = ["ann__echo".to_string()].into_iter().collect();
        assert_eq!(session.readonly_local_names(), expected);
        session.close().await;
        let _ = script;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn is_error_and_image_omission() {
        let python = python_bin();
        let script = write_fixture();
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let mut specs = BTreeMap::new();
        specs.insert(
            "err".into(),
            stdio_spec(&python, script.path(), "error", "err"),
        );
        specs.insert(
            "img".into(),
            stdio_spec(&python, script.path(), "image", "img"),
        );
        let session = McpSession::connect_specs(
            specs,
            Some(owners),
            CancellationToken::new(),
            test_timeouts(),
        )
        .await;
        let err = session
            .call("err__echo", json!({"text": "x"}), CancellationToken::new())
            .await
            .expect_err("is_error must not succeed");
        assert!(
            err.model_feedback().unwrap_or("").contains("boom"),
            "{:?}",
            err.model_feedback()
        );
        let img = session
            .call("img__echo", json!({"text": "hi"}), CancellationToken::new())
            .await
            .expect("image call");
        let text = img.as_text().unwrap_or("");
        assert!(text.contains("img:echo:hi"), "{text}");
        assert!(text.contains("[omitted: image content]"), "{text}");
        assert!(!text.contains("iVBORw0KGgo="), "{text}");
        session.close().await;
        let _ = script;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_does_not_reuse_a_live_transport() {
        use crate::acp::process_owner::pid_is_alive;

        let python = python_bin();
        let script = write_fixture();
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let mut specs = BTreeMap::new();
        specs.insert(
            "slow".into(),
            stdio_spec(&python, script.path(), "hang-call", "slow"),
        );
        let session = McpSession::connect_specs(
            specs,
            Some(Arc::clone(&owners)),
            CancellationToken::new(),
            McpTimeouts {
                per_server_init: Duration::from_secs(5),
                session_init: Duration::from_secs(8),
                max_concurrent_init: 4,
                call: Duration::from_millis(400),
            },
        )
        .await;
        assert!(
            session
                .bindings()
                .iter()
                .any(|b| b.local_name == "slow__echo"),
            "{:?}",
            session
                .bindings()
                .iter()
                .map(|b| &b.local_name)
                .collect::<Vec<_>>()
        );
        let pid = *session.live_pids().first().expect("hang-call server pid");
        let err = session
            .call("slow__echo", json!({"text": "x"}), CancellationToken::new())
            .await
            .expect_err("hung call must time out");
        assert!(
            err.message().to_ascii_lowercase().contains("timed out")
                || err.model_feedback().unwrap_or("").contains("timed out"),
            "{} / {:?}",
            err.message(),
            err.model_feedback()
        );
        let again = session
            .call("slow__echo", json!({"text": "y"}), CancellationToken::new())
            .await
            .expect_err("retired transport must not be reused");
        assert!(
            again.model_feedback().unwrap_or("").contains("unavailable")
                || again.message().contains("unavailable"),
            "{} / {:?}",
            again.message(),
            again.model_feedback()
        );
        assert!(
            !pid_is_alive(pid),
            "retire must wait for OS reap before returning"
        );
        assert!(
            !lock_owners(&owners).pids().contains(&pid),
            "reaped MCP pid must be unregistered only after it is gone"
        );
        session.close().await;
        let _ = script;
    }
}

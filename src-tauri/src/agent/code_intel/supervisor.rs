//! One supervisor per canonical workspace.
//!
//! It owns the Serena and CodeGraph child MCP processes, optional CodeGraph
//! host `init`/`sync`, and a single Streamable HTTP adapter named
//! `codeg-code-intel`. Sessions attach; they do not start a second server.
//! LSP has no official MCP provider, so no language server is launched.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use rmcp::ServiceExt;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
use crate::acp::process_owner::{lock_owners, ProcessOwnerRegistry};

use super::argv::{plan_codegraph, plan_serena_binary, plan_serena_uvx, PlannedProcess};
use super::codegraph::{should_run_host_index, spawn_host_index, HostIndexAction};
use super::discover::{
    code_tools_managed_root, discover_codegraph, resolve_serena, SerenaResolution,
};
use super::filter::ToolProvider;
use super::lsp_select::{official_lsp_provider_ids, plan_lsp_starts, LspStartInput};
use super::mcp_adapter::CodeIntelMcpAdapter;
use super::workspace::canonical_workspace;
use super::{load_code_intel_config, normalize_config, CodeIntelConfig, MCP_SERVER_NAME};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRuntimeSnapshot {
    pub runtime: String,
    pub install: String,
    pub last_error: Option<String>,
    pub last_started_at: Option<String>,
    pub resolved_command: Option<String>,
}

impl ProviderRuntimeSnapshot {
    fn stopped() -> Self {
        Self {
            runtime: "stopped".into(),
            install: "idle".into(),
            last_error: None,
            last_started_at: None,
            resolved_command: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeIntelRuntimeSnapshot {
    pub lsp: ProviderRuntimeSnapshot,
    pub codegraph: ProviderRuntimeSnapshot,
    pub serena: ProviderRuntimeSnapshot,
}

impl CodeIntelRuntimeSnapshot {
    fn initial() -> Self {
        Self {
            lsp: ProviderRuntimeSnapshot::stopped(),
            codegraph: ProviderRuntimeSnapshot::stopped(),
            serena: ProviderRuntimeSnapshot::stopped(),
        }
    }
}

#[derive(Clone, Copy)]
enum ProviderSlot {
    Lsp,
    Codegraph,
    Serena,
}

fn slot_mut<'a>(
    board: &'a mut CodeIntelRuntimeSnapshot,
    slot: ProviderSlot,
) -> &'a mut ProviderRuntimeSnapshot {
    match slot {
        ProviderSlot::Lsp => &mut board.lsp,
        ProviderSlot::Codegraph => &mut board.codegraph,
        ProviderSlot::Serena => &mut board.serena,
    }
}

fn provider_failed(slot: &ProviderRuntimeSnapshot) -> bool {
    matches!(slot.runtime.as_str(), "handshake_failed" | "exited")
}

struct RegistryEntry {
    cfg: CodeIntelConfig,
    inner: Weak<ProjectCodeIntelInner>,
}

static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, RegistryEntry>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<PathBuf, RegistryEntry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct ProjectCodeIntelLease {
    inner: Arc<ProjectCodeIntelInner>,
}

struct ProjectCodeIntelInner {
    workspace: PathBuf,
    cfg: CodeIntelConfig,
    owners: Arc<Mutex<ProcessOwnerRegistry>>,
    cancel: CancellationToken,
    adapter: CodeIntelMcpAdapter,
    http_url: tokio::sync::OnceCell<Option<String>>,
    startup: tokio::sync::OnceCell<()>,
    board: Arc<Mutex<CodeIntelRuntimeSnapshot>>,
}

impl ProjectCodeIntelLease {
    pub fn workspace(&self) -> &Path {
        &self.inner.workspace
    }

    pub fn adapter(&self) -> CodeIntelMcpAdapter {
        self.inner.adapter.clone()
    }

    pub fn mcp_http_url(&self) -> Option<String> {
        self.inner.http_url.get().cloned().flatten()
    }

    pub fn runtime_snapshot(&self) -> CodeIntelRuntimeSnapshot {
        self.inner
            .board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn mcp_server_name() -> &'static str {
        MCP_SERVER_NAME
    }
}

impl Drop for ProjectCodeIntelInner {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub struct ProjectCodeIntelSupervisor;

impl ProjectCodeIntelSupervisor {
    pub async fn acquire(
        cwd: PathBuf,
        fs: Arc<FileSystemRuntime>,
    ) -> Option<ProjectCodeIntelLease> {
        let cfg = load_code_intel_config();
        Self::acquire_started_with_config(cwd, fs, cfg).await
    }

    pub async fn acquire_started_with_config(
        cwd: PathBuf,
        fs: Arc<FileSystemRuntime>,
        cfg: CodeIntelConfig,
    ) -> Option<ProjectCodeIntelLease> {
        let lease = Self::acquire_with_config(cwd, fs, cfg)?;
        lease.inner.ensure_started().await;
        Some(lease)
    }

    pub fn acquire_with_config(
        cwd: PathBuf,
        _fs: Arc<FileSystemRuntime>,
        cfg: CodeIntelConfig,
    ) -> Option<ProjectCodeIntelLease> {
        let cfg = normalize_config(cfg);
        if !cfg.enabled {
            return None;
        }
        let canonical = canonical_workspace(&cwd);
        let mut guard = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = guard.get(&canonical) {
            if entry.cfg == cfg {
                if let Some(inner) = entry.inner.upgrade() {
                    return Some(ProjectCodeIntelLease { inner });
                }
            }
        }
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let cancel = CancellationToken::new();
        let adapter = CodeIntelMcpAdapter::new();
        let inner = Arc::new(ProjectCodeIntelInner {
            workspace: canonical.clone(),
            cfg: cfg.clone(),
            owners,
            cancel,
            adapter,
            http_url: tokio::sync::OnceCell::new(),
            startup: tokio::sync::OnceCell::new(),
            board: Arc::new(Mutex::new(CodeIntelRuntimeSnapshot::initial())),
        });
        guard.insert(
            canonical,
            RegistryEntry {
                cfg,
                inner: Arc::downgrade(&inner),
            },
        );
        Some(ProjectCodeIntelLease { inner })
    }

    /// Restart providers that already failed inside a live workspace lease.
    /// A settings read does not call this, and a missing lease does not spawn.
    pub async fn retry_failed(cwd: &Path) -> bool {
        let canonical = canonical_workspace(cwd);
        let inner = {
            let guard = registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .get(&canonical)
                .and_then(|entry| entry.inner.upgrade())
        };
        let Some(inner) = inner else {
            return false;
        };
        let snap = inner
            .board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if provider_failed(&snap.codegraph) {
            inner.start_codegraph().await;
        }
        if provider_failed(&snap.serena) {
            inner.start_serena().await;
        }
        true
    }
}

pub fn runtime_snapshot_for(cwd: &Path) -> Option<CodeIntelRuntimeSnapshot> {
    let canonical = canonical_workspace(cwd);
    let guard = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get(&canonical)?;
    let inner = entry.inner.upgrade()?;
    let snapshot = inner
        .board
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Some(snapshot)
}

impl ProjectCodeIntelInner {
    async fn ensure_started(&self) {
        self.startup
            .get_or_init(|| async {
                self.note_lsp();
                let _ = self.ensure_http().await;
                tokio::join!(self.start_codegraph(), self.start_serena());
            })
            .await;
    }

    fn note_lsp(&self) {
        let fs = FileSystemRuntime::with_policy(FsAccessPolicy::strict(&self.workspace));
        let detected = super::detect_languages(&self.workspace, &fs, super::preset_lsp_servers())
            .into_iter()
            .filter_map(|hit| super::language_key_for_preset_id(&hit.server_id).map(str::to_string))
            .collect();
        let official = official_lsp_provider_ids()
            .iter()
            .map(|key| (*key).to_string())
            .collect();
        let plan = plan_lsp_starts(&LspStartInput {
            master_enabled: self.cfg.enabled,
            lsp_enabled: self.cfg.lsp.enabled,
            checked: self.cfg.lsp.languages.clone(),
            detected,
            official,
            running: Vec::new(),
            max_concurrent: self.cfg.lsp.max_concurrent,
        });
        let runtime = if plan.waiting.is_empty() {
            "stopped"
        } else {
            "waiting"
        };
        self.update(ProviderSlot::Lsp, |slot| {
            *slot = ProviderRuntimeSnapshot::stopped();
            slot.runtime = runtime.to_string();
        });
    }

    async fn start_codegraph(&self) {
        if !self.cfg.codegraph.enabled {
            self.update(ProviderSlot::Codegraph, |slot| {
                *slot = ProviderRuntimeSnapshot::stopped();
            });
            return;
        }
        let discovered =
            discover_codegraph(&self.cfg.codegraph, &code_tools_managed_root(), &|name| {
                which::which(name).ok()
            });
        let Some(program) = discovered.path else {
            self.update(ProviderSlot::Codegraph, |slot| {
                *slot = ProviderRuntimeSnapshot::stopped();
                slot.last_error = discovered.error.clone();
            });
            return;
        };
        let plan = plan_codegraph(program.clone(), &self.workspace);
        self.mark_starting(ProviderSlot::Codegraph, &program);
        let action = should_run_host_index(&self.cfg, Some(program.as_path()), &self.workspace);
        if !matches!(action, HostIndexAction::Skip) {
            let binary = program;
            let cwd = self.workspace.clone();
            let owners = Arc::clone(&self.owners);
            let cancel = self.cancel.clone();
            tokio::spawn(async move {
                spawn_host_index(binary, cwd, action, owners, cancel).await;
            });
        }
        self.spawn_mcp(
            "codegraph",
            ProviderSlot::Codegraph,
            ToolProvider::Codegraph,
            &plan,
        )
        .await;
    }

    async fn start_serena(&self) {
        if !self.cfg.serena.enabled {
            self.update(ProviderSlot::Serena, |slot| {
                *slot = ProviderRuntimeSnapshot::stopped();
            });
            return;
        }
        let resolved = resolve_serena(&self.cfg.serena, &code_tools_managed_root(), &|name| {
            which::which(name).ok()
        });
        let plan = match resolved {
            SerenaResolution::Direct(found) => {
                let Some(program) = found.path else {
                    self.update(ProviderSlot::Serena, |slot| {
                        *slot = ProviderRuntimeSnapshot::stopped();
                        slot.last_error = found.error.clone();
                    });
                    return;
                };
                plan_serena_binary(
                    program,
                    &self.cfg.serena.context,
                    &self.workspace,
                    &self.cfg.serena.modes,
                )
            }
            SerenaResolution::Uvx { uvx } => plan_serena_uvx(
                uvx,
                &self.cfg.serena.context,
                &self.workspace,
                &self.cfg.serena.modes,
            ),
            SerenaResolution::Missing { error } => {
                tracing::warn!(
                    workspace = %self.workspace.display(),
                    provider = "serena",
                    error = %error,
                    "code-intel provider missing runtime"
                );
                self.update(ProviderSlot::Serena, |slot| {
                    *slot = ProviderRuntimeSnapshot::stopped();
                    slot.last_error = Some(error);
                });
                return;
            }
        };
        self.mark_starting(ProviderSlot::Serena, &plan.program);
        self.spawn_mcp("serena", ProviderSlot::Serena, ToolProvider::Serena, &plan)
            .await;
    }

    fn mark_starting(&self, slot: ProviderSlot, program: &Path) {
        let stamp = chrono::Utc::now().to_rfc3339();
        let command = program.display().to_string();
        self.update(slot, |state| {
            state.runtime = "starting".into();
            state.install = "installed".into();
            state.resolved_command = Some(command);
            state.last_started_at = Some(stamp);
            state.last_error = None;
        });
    }

    async fn spawn_mcp(
        &self,
        provider_id: &str,
        slot: ProviderSlot,
        provider: ToolProvider,
        plan: &PlannedProcess,
    ) {
        let basename = plan
            .program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| provider_id.to_string());
        let mut cmd = tokio::process::Command::new(&plan.program);
        cmd.args(&plan.args)
            .current_dir(&plan.cwd)
            .kill_on_drop(true);
        for (key, value) in &plan.env {
            cmd.env(key, value);
        }
        match rmcp::transport::TokioChildProcess::new(cmd) {
            Ok(transport) => {
                let pid = transport.id().unwrap_or(0);
                if pid != 0 {
                    lock_owners(&self.owners).register(pid);
                }
                match tokio::time::timeout(HANDSHAKE_TIMEOUT, ().serve(transport)).await {
                    Ok(Ok(running)) => {
                        self.adapter
                            .attach_provider(provider, running.peer().clone())
                            .await;
                        self.update(slot, |state| {
                            state.runtime = "connected".into();
                            state.last_error = None;
                        });
                        let started_at = self.slot_snapshot(slot).last_started_at;
                        let cancel = self.cancel.clone();
                        let owners = Arc::clone(&self.owners);
                        let board = Arc::clone(&self.board);
                        tokio::spawn(async move {
                            tokio::select! {
                                _ = cancel.cancelled() => {}
                                result = running.waiting() => {
                                    let mut message = match result {
                                        Ok(reason) => format!("{reason:?}"),
                                        Err(err) => err.to_string(),
                                    };
                                    message.truncate(300);
                                    let mut guard = board
                                        .lock()
                                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                                    let state = slot_mut(&mut guard, slot);
                                    if state.last_started_at == started_at
                                        && state.runtime == "connected"
                                    {
                                        state.runtime = "exited".into();
                                        state.last_error = Some(message);
                                    }
                                }
                            }
                            if pid != 0 {
                                lock_owners(&owners).unregister(pid);
                            }
                        });
                    }
                    Ok(Err(err)) => {
                        if pid != 0 {
                            lock_owners(&self.owners).unregister(pid);
                        }
                        tracing::warn!(
                            workspace = %self.workspace.display(),
                            provider = provider_id,
                            command = %basename,
                            error = %err,
                            "code-intel MCP handshake failed"
                        );
                        self.update(slot, |state| {
                            state.runtime = "handshake_failed".into();
                            state.last_error = Some(err.to_string());
                        });
                    }
                    Err(_) => {
                        if pid != 0 {
                            lock_owners(&self.owners).unregister(pid);
                        }
                        tracing::warn!(
                            workspace = %self.workspace.display(),
                            provider = provider_id,
                            command = %basename,
                            "code-intel MCP handshake timed out"
                        );
                        self.update(slot, |state| {
                            state.runtime = "handshake_failed".into();
                            state.last_error = Some("MCP handshake timed out".into());
                        });
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    workspace = %self.workspace.display(),
                    provider = provider_id,
                    command = %basename,
                    error = %err,
                    "code-intel provider spawn failed"
                );
                self.update(slot, |state| {
                    state.runtime = "exited".into();
                    state.last_error = Some(err.to_string());
                });
            }
        }
    }

    fn slot_snapshot(&self, slot: ProviderSlot) -> ProviderRuntimeSnapshot {
        let board = self
            .board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot {
            ProviderSlot::Lsp => board.lsp.clone(),
            ProviderSlot::Codegraph => board.codegraph.clone(),
            ProviderSlot::Serena => board.serena.clone(),
        }
    }

    fn update(&self, slot: ProviderSlot, f: impl FnOnce(&mut ProviderRuntimeSnapshot)) {
        let mut board = self
            .board
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(slot_mut(&mut board, slot));
    }

    async fn ensure_http(&self) -> Option<String> {
        self.http_url
            .get_or_init(|| async {
                match bind_http_adapter(self.adapter.clone(), self.cancel.clone()).await {
                    Ok(url) => Some(url),
                    Err(err) => {
                        tracing::warn!(error = %err, "code-intel MCP HTTP bind failed");
                        None
                    }
                }
            })
            .await
            .clone()
    }
}

async fn bind_http_adapter(
    adapter: CodeIntelMcpAdapter,
    cancel: CancellationToken,
) -> Result<String, String> {
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|err| err.to_string())?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    let url = format!("http://{addr}/mcp");
    let config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .with_cancellation_token(cancel.child_token());
    let service: StreamableHttpService<CodeIntelMcpAdapter, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(adapter.clone()), Default::default(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    let shutdown = cancel.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await;
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    Ok(url)
}

pub fn stdio_connector_command(url: &str) -> Option<(PathBuf, Vec<String>)> {
    let exe = std::env::current_exe().ok()?;
    if !exe.is_file() {
        return None;
    }
    Some((
        exe,
        vec![
            "--code-intel-mcp-stdio".into(),
            "--url".into(),
            url.to_string(),
        ],
    ))
}

pub fn insert_native_stdio_connector(
    specs: &mut BTreeMap<String, serde_json::Value>,
    url: &str,
) -> bool {
    let Some((command, args)) = stdio_connector_command(url) else {
        return false;
    };
    let Some(command) = command.to_str() else {
        return false;
    };
    specs.insert(
        MCP_SERVER_NAME.to_string(),
        serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
        }),
    );
    true
}

pub fn parse_stdio_bridge_url<I>(args: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if !args.iter().any(|arg| arg == "--code-intel-mcp-stdio") {
        return None;
    }
    args.windows(2)
        .find(|pair| pair[0] == "--url")
        .map(|pair| pair[1].clone())
        .filter(|url| !url.trim().is_empty())
}

pub fn run_stdio_http_bridge(url: &str) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?;
    rt.block_on(serve_stdio_http_bridge(url))
}

async fn serve_stdio_http_bridge(url: &str) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let client = reqwest::Client::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await.map_err(|err| err.to_string())? {
        if line.trim().is_empty() {
            continue;
        }
        let response = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(line)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let mut text = response.text().await.map_err(|err| err.to_string())?;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        stdout
            .write_all(text.as_bytes())
            .await
            .map_err(|err| err.to_string())?;
        stdout.flush().await.map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::file_system_runtime::FsAccessPolicy;
    use crate::agent::code_intel::default_config;

    fn fs_for(dir: &Path) -> Arc<FileSystemRuntime> {
        Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(dir)))
    }

    fn quiet_cfg() -> CodeIntelConfig {
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.codegraph.enabled = false;
        cfg.serena.enabled = false;
        cfg
    }

    #[cfg(unix)]
    fn write_argv_script(path: &Path, out_dir: &Path) {
        std::fs::create_dir_all(out_dir).unwrap();
        let body = format!(
            "#!/bin/sh\n{{\nprintf '%s\\n' \"$(pwd)\"\nprintf '%s\\n' \"$@\"\nprintf 'ENV:%s\\n' \"$CODEGRAPH_TELEMETRY\" \"$DO_NOT_TRACK\" \"$CODEGRAPH_NO_UPDATE_CHECK\"\n}} > '{}/'$$\nexit 2\n",
            out_dir.display()
        );
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn script_bodies(out_dir: &Path) -> Vec<String> {
        let mut bodies = Vec::new();
        let Ok(entries) = std::fs::read_dir(out_dir) else {
            return bodies;
        };
        for entry in entries.flatten() {
            if let Ok(body) = std::fs::read_to_string(entry.path()) {
                bodies.push(body);
            }
        }
        bodies
    }

    #[test]
    fn disabled_config_does_not_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let lease = ProjectCodeIntelSupervisor::acquire_with_config(
            dir.path().to_path_buf(),
            fs_for(dir.path()),
            default_config(),
        );
        assert!(lease.is_none());
    }

    #[test]
    fn same_canonical_workspace_reuses_supervisor() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = quiet_cfg();
        let first = ProjectCodeIntelSupervisor::acquire_with_config(
            dir.path().to_path_buf(),
            fs_for(dir.path()),
            cfg.clone(),
        )
        .unwrap();
        let second = ProjectCodeIntelSupervisor::acquire_with_config(
            dir.path().join("."),
            fs_for(dir.path()),
            cfg,
        )
        .unwrap();
        assert!(Arc::ptr_eq(&first.inner, &second.inner));
    }

    #[test]
    fn different_workspaces_do_not_share_supervisor() {
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        let cfg = quiet_cfg();
        let first = ProjectCodeIntelSupervisor::acquire_with_config(
            left.path().to_path_buf(),
            fs_for(left.path()),
            cfg.clone(),
        )
        .unwrap();
        let second = ProjectCodeIntelSupervisor::acquire_with_config(
            right.path().to_path_buf(),
            fs_for(right.path()),
            cfg,
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&first.inner, &second.inner));
        assert_ne!(first.workspace(), second.workspace());
    }

    #[test]
    fn parse_stdio_bridge_url_reads_flag() {
        let url = parse_stdio_bridge_url([
            "codeg".into(),
            "--code-intel-mcp-stdio".into(),
            "--url".into(),
            "http://127.0.0.1:9/mcp".into(),
        ]);
        assert_eq!(url.as_deref(), Some("http://127.0.0.1:9/mcp"));
        assert!(parse_stdio_bridge_url(["codeg".into()]).is_none());
    }

    #[test]
    fn native_stdio_connector_spec_targets_existing_http_endpoint() {
        let mut specs = BTreeMap::new();
        assert!(insert_native_stdio_connector(
            &mut specs,
            "http://127.0.0.1:9/mcp"
        ));
        let spec = specs.get(MCP_SERVER_NAME).expect("connector spec");
        assert_eq!(spec["type"], "stdio");
        assert_eq!(spec["args"][0], "--code-intel-mcp-stdio");
        assert_eq!(spec["args"][1], "--url");
        assert_eq!(spec["args"][2], "http://127.0.0.1:9/mcp");
        let command = spec["command"].as_str().expect("command");
        assert!(Path::new(command).is_absolute());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn master_switch_off_does_not_spawn() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let script = ws.path().join("codegraph");
        #[cfg(unix)]
        write_argv_script(&script, out.path());
        #[cfg(not(unix))]
        std::fs::write(&script, b"").unwrap();
        let mut cfg = default_config();
        cfg.enabled = false;
        cfg.codegraph.enabled = true;
        cfg.codegraph.binary_path = Some(script.to_string_lossy().into_owned());
        temp_env::async_with_vars(
            [("CODEG_HOME", Some(home.path().to_str().unwrap()))],
            async {
                crate::agent::code_intel::save_code_intel_config(&cfg).unwrap();
                let lease =
                    ProjectCodeIntelSupervisor::acquire(ws.path().to_path_buf(), fs_for(ws.path()))
                        .await;
                assert!(lease.is_none());
            },
        )
        .await;
        assert!(out.path().read_dir().unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn disabled_provider_does_not_start_and_handshake_failure_is_isolated() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("my project");
        std::fs::create_dir_all(&workspace).unwrap();
        let cg_out = root.path().join("cg-out");
        let se_out = root.path().join("se-out");
        let cg_script = root.path().join("codegraph");
        let se_script = root.path().join("serena");
        write_argv_script(&cg_script, &cg_out);
        write_argv_script(&se_script, &se_out);
        let mut cfg = quiet_cfg();
        cfg.codegraph.enabled = true;
        cfg.codegraph.binary_path = Some(cg_script.to_string_lossy().into_owned());
        cfg.serena.enabled = true;
        cfg.serena.auto_install = false;
        cfg.serena.command = Some(se_script.to_string_lossy().into_owned());
        let lease = ProjectCodeIntelSupervisor::acquire_started_with_config(
            workspace.clone(),
            fs_for(&workspace),
            cfg,
        )
        .await
        .expect("master on acquires");
        let url = lease.mcp_http_url().expect("http endpoint");
        assert!(url.ends_with("/mcp"), "{url}");
        let snap = lease.runtime_snapshot();
        assert_eq!(snap.lsp.runtime, "stopped");
        assert_eq!(snap.codegraph.runtime, "handshake_failed");
        assert_eq!(snap.serena.runtime, "handshake_failed");
        assert!(snap.codegraph.last_error.is_some());
        assert!(snap.serena.last_error.is_some());
        let cg_bodies = script_bodies(&cg_out);
        let se_bodies = script_bodies(&se_out);
        assert!(
            cg_bodies
                .iter()
                .any(|body| body.lines().any(|line| line == "serve")
                    && body.lines().any(|line| line == "--mcp")),
            "{cg_bodies:?}"
        );
        assert!(
            cg_bodies.iter().any(|body| {
                body.lines().any(|line| line == "ENV:0") && body.lines().any(|line| line == "ENV:1")
            }),
            "{cg_bodies:?}"
        );
        let project = lease.workspace().to_string_lossy().into_owned();
        assert!(
            se_bodies.iter().any(|body| {
                body.lines().any(|line| line == "start-mcp-server")
                    && body.lines().any(|line| line == project.as_str())
                    && body.lines().any(|line| line == "--context")
                    && body.lines().any(|line| line == "codex")
            }),
            "{se_bodies:?}"
        );
        assert!(
            !se_bodies
                .iter()
                .any(|body| body.contains("git+https://github.com/oraios/serena")
                    && !body.contains("@v1.7.0")),
            "{se_bodies:?}"
        );
        drop(lease);

        let mut cfg = quiet_cfg();
        cfg.codegraph.enabled = false;
        cfg.codegraph.binary_path = Some(cg_script.to_string_lossy().into_owned());
        cfg.serena.enabled = false;
        cfg.serena.command = Some(se_script.to_string_lossy().into_owned());
        let other = root.path().join("other project");
        std::fs::create_dir_all(&other).unwrap();
        let cg_out_off = root.path().join("cg-off");
        let se_out_off = root.path().join("se-off");
        let cg_off = root.path().join("codegraph-off");
        let se_off = root.path().join("serena-off");
        write_argv_script(&cg_off, &cg_out_off);
        write_argv_script(&se_off, &se_out_off);
        cfg.codegraph.binary_path = Some(cg_off.to_string_lossy().into_owned());
        cfg.serena.command = Some(se_off.to_string_lossy().into_owned());
        let lease = ProjectCodeIntelSupervisor::acquire_started_with_config(
            other,
            fs_for(root.path()),
            cfg,
        )
        .await
        .expect("master still on");
        assert_eq!(lease.runtime_snapshot().codegraph.runtime, "stopped");
        assert_eq!(lease.runtime_snapshot().serena.runtime, "stopped");
        assert_eq!(lease.runtime_snapshot().lsp.runtime, "stopped");
        assert!(script_bodies(&cg_out_off).is_empty());
        assert!(script_bodies(&se_out_off).is_empty());
    }
}

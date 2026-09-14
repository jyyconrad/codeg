//! Project-level code-intel supervisor.
//!
//! One instance per canonical workspace owns the LSP pool, host `init`/`sync`,
//! a single `codegraph serve --mcp` child, and the Streamable HTTP MCP adapter.
//! Agent sessions only connect; they do not spawn those engines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use rmcp::ServiceExt;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::acp::file_system_runtime::FileSystemRuntime;
use crate::acp::process_owner::{lock_owners, ProcessOwnerRegistry};

use super::mcp_adapter::CodeIntelMcpAdapter;
use super::mcp_tools::MCP_SERVER_NAME;
use super::{
    load_code_intel_config, resolve_codegraph_binary, should_run_host_index, spawn_host_index,
    CodeIntelConfig, HostIndexAction, LspPool,
};

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
    lsp_pool: Option<Arc<LspPool>>,
    adapter: CodeIntelMcpAdapter,
    http_url: tokio::sync::OnceCell<Option<String>>,
}

impl ProjectCodeIntelLease {
    pub fn workspace(&self) -> &Path {
        &self.inner.workspace
    }

    pub fn lsp_pool(&self) -> Option<Arc<LspPool>> {
        self.inner.lsp_pool.clone()
    }

    pub fn adapter(&self) -> CodeIntelMcpAdapter {
        self.inner.adapter.clone()
    }

    pub fn mcp_http_url(&self) -> Option<String> {
        self.inner.http_url.get().cloned().flatten()
    }

    pub fn mcp_server_name() -> &'static str {
        MCP_SERVER_NAME
    }
}

impl Drop for ProjectCodeIntelInner {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(pool) = self.lsp_pool.clone() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    pool.shutdown_all().await;
                });
            }
        }
    }
}

pub struct ProjectCodeIntelSupervisor;

impl ProjectCodeIntelSupervisor {
    pub async fn acquire(
        cwd: PathBuf,
        fs: Arc<FileSystemRuntime>,
    ) -> Option<ProjectCodeIntelLease> {
        let cfg = load_code_intel_config();
        let lease = Self::acquire_with_config(cwd, fs, cfg)?;
        lease.inner.start_host_index();
        lease.inner.spawn_codegraph_mcp().await;
        let _ = lease.inner.ensure_http().await;
        Some(lease)
    }

    pub fn acquire_with_config(
        cwd: PathBuf,
        fs: Arc<FileSystemRuntime>,
        cfg: CodeIntelConfig,
    ) -> Option<ProjectCodeIntelLease> {
        if !cfg.enabled {
            return None;
        }
        let canonical = super::lsp_pool::canonical_workspace(&cwd);
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
        let lsp_pool = cfg
            .lsp
            .auto_attach
            .then(|| LspPool::for_workspace(canonical.clone(), Arc::clone(&fs), cfg.clone()));
        let adapter =
            CodeIntelMcpAdapter::new(cfg.clone(), canonical.clone(), fs, lsp_pool.clone());
        let inner = Arc::new(ProjectCodeIntelInner {
            workspace: canonical.clone(),
            cfg: cfg.clone(),
            owners,
            cancel,
            lsp_pool,
            adapter,
            http_url: tokio::sync::OnceCell::new(),
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
}

impl ProjectCodeIntelInner {
    fn start_host_index(&self) {
        let binary = resolve_codegraph_binary(&self.cfg.codegraph);
        let action = should_run_host_index(&self.cfg, binary.as_deref(), &self.workspace);
        if matches!(action, HostIndexAction::Skip) {
            return;
        }
        let Some(binary) = binary else {
            return;
        };
        let cwd = self.workspace.clone();
        let owners = Arc::clone(&self.owners);
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            spawn_host_index(binary, cwd, action, owners, cancel).await;
        });
    }

    async fn spawn_codegraph_mcp(&self) {
        if !self.cfg.codegraph.enabled {
            return;
        }
        let Some(binary) = resolve_codegraph_binary(&self.cfg.codegraph) else {
            return;
        };
        let mut cmd = tokio::process::Command::new(&binary);
        cmd.args(["serve", "--mcp"])
            .current_dir(&self.workspace)
            .env("CODEGRAPH_TELEMETRY", "0")
            .env("DO_NOT_TRACK", "1")
            .env("CODEGRAPH_NO_UPDATE_CHECK", "1")
            .kill_on_drop(true);
        match rmcp::transport::TokioChildProcess::new(cmd) {
            Ok(transport) => {
                let pid = transport.id().unwrap_or(0);
                if pid != 0 {
                    lock_owners(&self.owners).register(pid);
                }
                match ().serve(transport).await {
                    Ok(running) => {
                        self.adapter.attach_codegraph(running.peer().clone()).await;
                        let cancel = self.cancel.clone();
                        tokio::spawn(async move {
                            cancel.cancelled().await;
                            drop(running);
                        });
                    }
                    Err(err) => {
                        tracing::debug!(error = %err, "codegraph serve --mcp handshake failed");
                    }
                }
            }
            Err(err) => {
                tracing::debug!(error = %err, "codegraph serve --mcp spawn failed");
            }
        }
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
    // Give the listener a tick to accept.
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

pub fn parse_stdio_bridge_url<I>(args: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if !args.iter().any(|a| a == "--code-intel-mcp-stdio") {
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
        let mut cfg = default_config();
        cfg.enabled = true;
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
}

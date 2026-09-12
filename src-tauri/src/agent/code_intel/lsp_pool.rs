use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::router::Router;
use async_lsp::tracing::TracingLayer;
use async_lsp::{LanguageServer, MainLoop, ServerSocket};
use lsp_types::notification::{LogMessage, Progress, PublishDiagnostics, ShowMessage};
use lsp_types::request::{
    RegisterCapability, ShowMessageRequest, WorkDoneProgressCreate, WorkspaceConfiguration,
};
use lsp_types::{
    ClientCapabilities, Diagnostic, DiagnosticSeverity, DidOpenTextDocumentParams,
    DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
    Hover, HoverContents, HoverParams, InitializeParams, InitializedParams, Location, LocationLink,
    MarkedString, Position, ReferenceContext, ReferenceParams, SymbolInformation,
    TextDocumentClientCapabilities, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url, WindowClientCapabilities, WorkDoneProgressParams,
    WorkspaceClientCapabilities, WorkspaceFolder, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tokio_util::sync::CancellationToken;
use tower::ServiceBuilder;

use crate::acp::file_system_runtime::FileSystemRuntime;
use crate::acp::process_owner::{
    force_kill_and_reap, lock_owners, pid_is_alive, ProcessOwnerRegistry,
};

use super::{detect_languages, preset_lsp_servers, CodeIntelConfig, DetectedLanguage};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

struct Stop;

struct ClientState {
    diagnostics: Arc<Mutex<HashMap<Url, Vec<Diagnostic>>>>,
}

struct RunningServer {
    server: ServerSocket,
    child: tokio::process::Child,
    pid: u32,
    opened: HashSet<Url>,
    loop_task: tokio::task::JoinHandle<()>,
}

struct PoolInner {
    running: HashMap<String, RunningServer>,
    /// Bumped by `shutdown_all` so a handshake that started earlier cannot insert.
    epoch: u64,
}

pub struct LspPool {
    cwd: PathBuf,
    fs: Arc<FileSystemRuntime>,
    cfg: CodeIntelConfig,
    owners: Arc<Mutex<ProcessOwnerRegistry>>,
    cancel: CancellationToken,
    inner: tokio::sync::Mutex<PoolInner>,
    diagnostics: Arc<Mutex<HashMap<Url, Vec<Diagnostic>>>>,
}

impl LspPool {
    pub fn new(
        cwd: PathBuf,
        fs: Arc<FileSystemRuntime>,
        cfg: CodeIntelConfig,
        owners: Arc<Mutex<ProcessOwnerRegistry>>,
        cancel: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            cwd,
            fs,
            cfg,
            owners,
            cancel,
            inner: tokio::sync::Mutex::new(PoolInner {
                running: HashMap::new(),
                epoch: 0,
            }),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn ensure_server(&self, server_id: &str) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err("lsp cancelled".into());
        }

        let epoch = {
            let inner = self.inner.lock().await;
            if inner.running.contains_key(server_id) {
                return Ok(());
            }
            inner.epoch
        };

        // Detect (up to 5000 files) and initialize (30s) must not hold `inner`.
        let detected = detect_languages(
            &self.cwd,
            self.fs.as_ref(),
            preset_lsp_servers(),
            &self.cfg.lsp.custom,
        );
        let start = servers_to_start(&self.cfg, &detected, &|id| self.command_on_path(id));
        if !start.iter().any(|id| id == server_id) {
            return Err("server not eligible".into());
        }

        if self.cancel.is_cancelled() {
            return Err("lsp cancelled".into());
        }

        {
            let inner = self.inner.lock().await;
            if inner.running.contains_key(server_id) {
                return Ok(());
            }
            if inner.epoch != epoch {
                return Err("lsp cancelled".into());
            }
            let cap = self.cfg.lsp.max_concurrent.max(1) as usize;
            if inner.running.len() >= cap {
                return Err(format!("LSP concurrency limit ({cap}) reached"));
            }
        }

        let running = self.spawn_and_handshake(server_id).await?;

        enum Insert {
            Keep,
            Duplicate(RunningServer),
            Reject(RunningServer, String),
        }

        let insert = {
            let mut inner = self.inner.lock().await;
            if inner.running.contains_key(server_id) {
                Insert::Duplicate(running)
            } else if inner.epoch != epoch || self.cancel.is_cancelled() {
                Insert::Reject(running, "lsp cancelled".into())
            } else {
                let cap = self.cfg.lsp.max_concurrent.max(1) as usize;
                if inner.running.len() >= cap {
                    Insert::Reject(running, format!("LSP concurrency limit ({cap}) reached"))
                } else {
                    inner.running.insert(server_id.to_string(), running);
                    Insert::Keep
                }
            }
        };

        match insert {
            Insert::Keep => Ok(()),
            Insert::Duplicate(extra) => {
                shutdown_running(&self.owners, extra).await;
                Ok(())
            }
            Insert::Reject(extra, err) => {
                shutdown_running(&self.owners, extra).await;
                Err(err)
            }
        }
    }

    pub async fn hover(
        &self,
        server_id: &str,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<String, String> {
        self.ensure_server(server_id).await?;
        let mut server = self.socket(server_id).await?;
        let uri = self.file_url(path)?;
        let result = timed(
            &self.cancel,
            server.hover(HoverParams {
                text_document_position_params: position_params(uri, line, character),
                work_done_progress_params: WorkDoneProgressParams::default(),
            }),
        )
        .await?;
        Ok(match result {
            Some(hover) => format_hover(hover),
            None => "no hover".into(),
        })
    }

    pub async fn definition(
        &self,
        server_id: &str,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<String, String> {
        self.ensure_server(server_id).await?;
        let mut server = self.socket(server_id).await?;
        let uri = self.file_url(path)?;
        let result = timed(
            &self.cancel,
            server.definition(GotoDefinitionParams {
                text_document_position_params: position_params(uri, line, character),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            }),
        )
        .await?;
        Ok(match result {
            Some(resp) => format_definition(resp),
            None => "no definition".into(),
        })
    }

    pub async fn references(
        &self,
        server_id: &str,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<String, String> {
        self.ensure_server(server_id).await?;
        let mut server = self.socket(server_id).await?;
        let uri = self.file_url(path)?;
        let result = timed(
            &self.cancel,
            server.references(ReferenceParams {
                text_document_position: position_params(uri, line, character),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration: true,
                },
            }),
        )
        .await?;
        Ok(match result {
            Some(locs) if !locs.is_empty() => locs
                .iter()
                .map(format_location)
                .collect::<Vec<_>>()
                .join("\n"),
            _ => "no references".into(),
        })
    }

    pub async fn document_symbol(&self, server_id: &str, path: &Path) -> Result<String, String> {
        self.ensure_server(server_id).await?;
        let mut server = self.socket(server_id).await?;
        let uri = self.file_url(path)?;
        let result = timed(
            &self.cancel,
            server.document_symbol(DocumentSymbolParams {
                text_document: TextDocumentIdentifier { uri },
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            }),
        )
        .await?;
        Ok(match result {
            Some(resp) => format_document_symbols(resp),
            None => "no document symbols".into(),
        })
    }

    pub async fn workspace_symbol(&self, server_id: &str, query: &str) -> Result<String, String> {
        self.ensure_server(server_id).await?;
        let mut server = self.socket(server_id).await?;
        let result = timed(
            &self.cancel,
            server.symbol(WorkspaceSymbolParams {
                query: query.to_string(),
                work_done_progress_params: WorkDoneProgressParams::default(),
                partial_result_params: Default::default(),
            }),
        )
        .await?;
        Ok(match result {
            Some(resp) => format_workspace_symbols(resp),
            None => "no workspace symbols".into(),
        })
    }

    pub async fn diagnostics(&self, path: &Path) -> Vec<String> {
        let Ok(uri) = self.file_url(path) else {
            return Vec::new();
        };
        let guard = self
            .diagnostics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .get(&uri)
            .map(|diags| diags.iter().map(|d| format_diagnostic(&uri, d)).collect())
            .unwrap_or_default()
    }

    pub async fn did_open(
        &self,
        server_id: &str,
        path: &Path,
        language_id: &str,
        text: String,
    ) -> Result<(), String> {
        self.ensure_server(server_id).await?;
        let uri = self.file_url(path)?;
        let mut inner = self.inner.lock().await;
        let running = inner
            .running
            .get_mut(server_id)
            .ok_or_else(|| format!("language server {server_id} is not running"))?;
        if running.opened.contains(&uri) {
            return Ok(());
        }
        running
            .server
            .did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: language_id.to_string(),
                    version: 0,
                    text,
                },
            })
            .map_err(|err| err.to_string())?;
        running.opened.insert(uri);
        Ok(())
    }

    pub async fn running_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        let mut ids: Vec<String> = inner.running.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub async fn shutdown_all(&self) {
        let running = {
            let mut inner = self.inner.lock().await;
            inner.epoch = inner.epoch.wrapping_add(1);
            std::mem::take(&mut inner.running)
        };

        for running in running.into_values() {
            shutdown_running(&self.owners, running).await;
        }
    }

    async fn socket(&self, server_id: &str) -> Result<ServerSocket, String> {
        let inner = self.inner.lock().await;
        inner
            .running
            .get(server_id)
            .map(|running| running.server.clone())
            .ok_or_else(|| format!("language server {server_id} is not running"))
    }

    fn file_url(&self, path: &Path) -> Result<Url, String> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        Url::from_file_path(&abs).map_err(|()| format!("invalid file path: {}", abs.display()))
    }

    fn command_on_path(&self, id: &str) -> bool {
        command_and_args(&self.cfg, id)
            .map(|(cmd, _)| command_exists(&cmd))
            .unwrap_or(false)
    }

    #[allow(deprecated)]
    async fn spawn_and_handshake(&self, server_id: &str) -> Result<RunningServer, String> {
        let (command, args) = command_and_args(&self.cfg, server_id)
            .ok_or_else(|| format!("unknown language server {server_id}"))?;

        let mut cmd = tokio::process::Command::new(&command);
        cmd.args(&args)
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|err| format!("failed to spawn {command}: {err}"))?;
        let pid = child.id().unwrap_or(0);
        lock_owners(&self.owners).register(pid);

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                lock_owners(&self.owners).unregister(pid);
                return Err("lsp stdout pipe missing".into());
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                lock_owners(&self.owners).unregister(pid);
                return Err("lsp stdin pipe missing".into());
            }
        };

        let diagnostics = self.diagnostics.clone();
        let (mainloop, mut server) = MainLoop::new_client(move |_server| {
            let mut router = Router::new(ClientState { diagnostics });
            router
                .notification::<PublishDiagnostics>(|this, params| {
                    let mut guard = this
                        .diagnostics
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.insert(params.uri, params.diagnostics);
                    ControlFlow::Continue(())
                })
                .notification::<ShowMessage>(|_, _| ControlFlow::Continue(()))
                .notification::<LogMessage>(|_, _| ControlFlow::Continue(()))
                .notification::<Progress>(|_, _| ControlFlow::Continue(()))
                .request::<WorkspaceConfiguration, _>(|_, params| async move {
                    Ok(params
                        .items
                        .into_iter()
                        .map(|_| serde_json::Value::Null)
                        .collect())
                })
                .request::<WorkDoneProgressCreate, _>(|_, _| async { Ok(()) })
                .request::<RegisterCapability, _>(|_, _| async { Ok(()) })
                .request::<ShowMessageRequest, _>(|_, _| async { Ok(None) })
                .unhandled_notification(|_, _| ControlFlow::Continue(()))
                .event(|_, _: Stop| ControlFlow::Break(Ok(())));

            ServiceBuilder::new()
                .layer(TracingLayer::default())
                .layer(CatchUnwindLayer::default())
                .layer(ConcurrencyLayer::default())
                .service(router)
        });

        let loop_task = tokio::spawn(async move {
            if let Err(err) = mainloop
                .run_buffered(stdout.compat(), stdin.compat_write())
                .await
            {
                tracing::debug!(error = %err, "lsp mainloop ended");
            }
        });

        let workspace_uri = match Url::from_file_path(&self.cwd) {
            Ok(uri) => uri,
            Err(()) => {
                fail_start(&self.owners, pid, &mut child, &server, &loop_task).await;
                return Err(format!("invalid workspace path: {}", self.cwd.display()));
            }
        };
        let folder_name = self
            .cwd
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_string();

        let init = timed(
            &self.cancel,
            server.initialize(InitializeParams {
                process_id: None,
                root_uri: Some(workspace_uri.clone()),
                workspace_folders: Some(vec![WorkspaceFolder {
                    uri: workspace_uri,
                    name: folder_name,
                }]),
                capabilities: client_capabilities(),
                ..InitializeParams::default()
            }),
        )
        .await;

        if let Err(err) = init {
            fail_start(&self.owners, pid, &mut child, &server, &loop_task).await;
            return Err(err);
        }

        if let Err(err) = server.initialized(InitializedParams {}) {
            fail_start(&self.owners, pid, &mut child, &server, &loop_task).await;
            return Err(err.to_string());
        }

        Ok(RunningServer {
            server,
            child,
            pid,
            opened: HashSet::new(),
            loop_task,
        })
    }
}

pub fn servers_to_start(
    cfg: &CodeIntelConfig,
    detected: &[DetectedLanguage],
    on_path: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    if !cfg.enabled || !cfg.lsp.auto_attach {
        return Vec::new();
    }

    let detected_ids: HashSet<&str> = detected.iter().map(|hit| hit.server_id.as_str()).collect();
    let checked: HashSet<&str> = cfg.lsp.checked.iter().map(String::as_str).collect();

    let mut ids = Vec::new();
    let mut push = |id: &str| {
        if ids.iter().any(|existing| existing == id) {
            return;
        }
        if checked.contains(id) && detected_ids.contains(id) && on_path(id) {
            ids.push(id.to_string());
        }
    };

    for preset in preset_lsp_servers() {
        push(preset.id);
    }
    for custom in &cfg.lsp.custom {
        push(&custom.id);
    }

    ids.truncate(cfg.lsp.max_concurrent.max(1) as usize);
    ids
}

fn command_and_args(cfg: &CodeIntelConfig, id: &str) -> Option<(String, Vec<String>)> {
    if let Some(preset) = preset_lsp_servers().iter().find(|preset| preset.id == id) {
        return Some((
            preset.binary.to_string(),
            preset.args.iter().map(|arg| (*arg).to_string()).collect(),
        ));
    }
    cfg.lsp
        .custom
        .iter()
        .find(|server| server.id == id)
        .map(|server| (server.command.clone(), server.args.clone()))
}

fn command_exists(cmd: &str) -> bool {
    let path = Path::new(cmd);
    if path.is_absolute() {
        path.is_file()
    } else {
        which::which(cmd).is_ok()
    }
}

fn client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            configuration: Some(true),
            workspace_folders: Some(true),
            ..WorkspaceClientCapabilities::default()
        }),
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..WindowClientCapabilities::default()
        }),
        text_document: Some(TextDocumentClientCapabilities::default()),
        ..ClientCapabilities::default()
    }
}

fn position_params(uri: Url, line: u32, character: u32) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri },
        position: Position::new(line, character),
    }
}

async fn timed<T, E: std::fmt::Display>(
    cancel: &CancellationToken,
    fut: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, String> {
    tokio::select! {
        _ = cancel.cancelled() => Err("lsp cancelled".into()),
        _ = tokio::time::sleep(REQUEST_TIMEOUT) => Err("lsp request timed out".into()),
        result = fut => result.map_err(|err| err.to_string()),
    }
}

async fn shutdown_running(owners: &Mutex<ProcessOwnerRegistry>, mut running: RunningServer) {
    let mut server = running.server;
    // `()` serializes shutdown params as JSON null, not {}.
    let _ = tokio::time::timeout(Duration::from_secs(5), server.shutdown(())).await;
    let _ = server.exit(());
    let _ = server.emit(Stop);
    reap_child(&mut running.child, running.pid).await;
    lock_owners(owners).unregister(running.pid);
    let _ = tokio::time::timeout(Duration::from_secs(2), running.loop_task).await;
}

async fn fail_start(
    owners: &Mutex<ProcessOwnerRegistry>,
    pid: u32,
    child: &mut tokio::process::Child,
    server: &ServerSocket,
    loop_task: &tokio::task::JoinHandle<()>,
) {
    let _ = server.emit(Stop);
    reap_child(child, pid).await;
    lock_owners(owners).unregister(pid);
    loop_task.abort();
}

async fn reap_child(child: &mut tokio::process::Child, pid: u32) {
    let waited = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    if waited.is_err() || pid_is_alive(pid) {
        let _ = force_kill_and_reap(&[pid], Duration::from_secs(2)).await;
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn format_hover(hover: Hover) -> String {
    match hover.contents {
        HoverContents::Markup(markup) => markup.value,
        HoverContents::Scalar(value) => marked_string(&value),
        HoverContents::Array(values) => values
            .iter()
            .map(marked_string)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn marked_string(value: &MarkedString) -> String {
    match value {
        MarkedString::String(text) => text.clone(),
        MarkedString::LanguageString(ls) => ls.value.clone(),
    }
}

fn format_definition(resp: GotoDefinitionResponse) -> String {
    match resp {
        GotoDefinitionResponse::Scalar(loc) => format_location(&loc),
        GotoDefinitionResponse::Array(locs) if locs.is_empty() => "no definition".into(),
        GotoDefinitionResponse::Array(locs) => locs
            .iter()
            .map(format_location)
            .collect::<Vec<_>>()
            .join("\n"),
        GotoDefinitionResponse::Link(links) if links.is_empty() => "no definition".into(),
        GotoDefinitionResponse::Link(links) => links
            .iter()
            .map(format_location_link)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn format_location(loc: &Location) -> String {
    format_uri_pos(&loc.uri, loc.range.start)
}

fn format_location_link(link: &LocationLink) -> String {
    format_uri_pos(&link.target_uri, link.target_range.start)
}

fn format_uri_pos(uri: &Url, pos: Position) -> String {
    let path = uri
        .to_file_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| uri.to_string());
    format!("{}:{}:{}", path, pos.line + 1, pos.character + 1)
}

fn format_document_symbols(resp: DocumentSymbolResponse) -> String {
    match resp {
        DocumentSymbolResponse::Flat(info) => {
            if info.is_empty() {
                return "no document symbols".into();
            }
            info.iter()
                .map(format_symbol_info)
                .collect::<Vec<_>>()
                .join("\n")
        }
        DocumentSymbolResponse::Nested(symbols) => {
            if symbols.is_empty() {
                return "no document symbols".into();
            }
            let mut lines = Vec::new();
            for symbol in &symbols {
                collect_document_symbol(symbol, &mut lines);
            }
            lines.join("\n")
        }
    }
}

fn collect_document_symbol(symbol: &lsp_types::DocumentSymbol, lines: &mut Vec<String>) {
    lines.push(format!(
        "{} ({:?}) {}:{}",
        symbol.name,
        symbol.kind,
        symbol.range.start.line + 1,
        symbol.range.start.character + 1
    ));
    if let Some(children) = &symbol.children {
        for child in children {
            collect_document_symbol(child, lines);
        }
    }
}

fn format_workspace_symbols(resp: WorkspaceSymbolResponse) -> String {
    match resp {
        WorkspaceSymbolResponse::Flat(info) => {
            if info.is_empty() {
                return "no workspace symbols".into();
            }
            info.iter()
                .map(format_symbol_info)
                .collect::<Vec<_>>()
                .join("\n")
        }
        WorkspaceSymbolResponse::Nested(symbols) => {
            if symbols.is_empty() {
                return "no workspace symbols".into();
            }
            symbols
                .iter()
                .map(|symbol| {
                    let loc = match &symbol.location {
                        lsp_types::OneOf::Left(loc) => format_location(loc),
                        lsp_types::OneOf::Right(loc) => loc.uri.to_string(),
                    };
                    format!("{} ({:?}) {loc}", symbol.name, symbol.kind)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

fn format_symbol_info(info: &SymbolInformation) -> String {
    format!(
        "{} ({:?}) {}",
        info.name,
        info.kind,
        format_location(&info.location)
    )
}

fn format_diagnostic(uri: &Url, diag: &Diagnostic) -> String {
    let severity = match diag.severity {
        Some(DiagnosticSeverity::ERROR) | None => "error",
        Some(DiagnosticSeverity::WARNING) => "warning",
        Some(DiagnosticSeverity::INFORMATION) => "info",
        Some(DiagnosticSeverity::HINT) => "hint",
        _ => "error",
    };
    format!(
        "{severity} {}: {}",
        format_uri_pos(uri, diag.range.start),
        diag.message
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
    use crate::agent::code_intel::{default_config, CustomLspServer, DetectedLanguage};

    fn detected(ids: &[&str]) -> Vec<DetectedLanguage> {
        ids.iter()
            .map(|id| DetectedLanguage {
                server_id: (*id).to_string(),
                via_manifest: true,
                via_extension: false,
            })
            .collect()
    }

    fn enabled_cfg() -> CodeIntelConfig {
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg
    }

    #[test]
    fn servers_to_start_caps_checked_detected_and_path() {
        let mut cfg = enabled_cfg();
        cfg.lsp.max_concurrent = 2;
        cfg.lsp.checked = vec![
            "rust-analyzer".into(),
            "gopls".into(),
            "pyright".into(),
            "elixir-ls".into(),
        ];
        cfg.lsp.custom.push(CustomLspServer {
            id: "elixir-ls".into(),
            language: "Elixir".into(),
            command: "elixir-ls".into(),
            args: vec![],
            extensions: vec![".ex".into()],
            manifests: vec!["mix.exs".into()],
        });
        let hits = detected(&["rust-analyzer", "gopls", "pyright", "elixir-ls"]);
        let ids = servers_to_start(&cfg, &hits, &|id| id != "pyright");
        assert_eq!(ids, vec!["rust-analyzer", "gopls"]);
    }

    #[test]
    fn servers_to_start_empty_when_auto_attach_off() {
        let mut cfg = enabled_cfg();
        cfg.lsp.auto_attach = false;
        let hits = detected(&["rust-analyzer", "gopls"]);
        let ids = servers_to_start(&cfg, &hits, &|_| true);
        assert!(ids.is_empty());
    }

    #[test]
    fn servers_to_start_empty_when_master_off() {
        let mut cfg = enabled_cfg();
        cfg.enabled = false;
        let hits = detected(&["rust-analyzer", "gopls"]);
        let ids = servers_to_start(&cfg, &hits, &|_| true);
        assert!(ids.is_empty());
    }

    #[test]
    fn servers_to_start_invokes_on_path_with_server_id() {
        let mut cfg = enabled_cfg();
        cfg.lsp.checked = vec!["pyright".into()];
        let hits = detected(&["pyright"]);
        let seen = std::sync::Mutex::new(Vec::new());
        let ids = servers_to_start(&cfg, &hits, &|id| {
            seen.lock().unwrap().push(id.to_string());
            true
        });
        assert_eq!(ids, vec!["pyright"]);
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen, vec!["pyright"]);
        assert!(!seen.iter().any(|id| id == "pyright-langserver"));
    }

    #[test]
    fn cargo_toml_pins_async_lsp_and_has_no_hand_rolled_protocol() {
        let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        assert!(
            manifest.contains("async-lsp"),
            "async-lsp must be a direct dependency"
        );
        let protocol = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/agent/code_intel/lsp_protocol.rs");
        assert!(
            !protocol.exists(),
            "do not implement Content-Length JSON-RPC in lsp_protocol.rs"
        );
    }

    #[cfg(unix)]
    const FAKE_LS: &str = r#"#!/usr/bin/env python3
import json
import sys
import time

INIT_DELAY = 0

def read_headers():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        key, _, value = line.decode("utf-8").partition(":")
        headers[key.strip().lower()] = value.strip()
    return headers

def read_message():
    headers = read_headers()
    if headers is None:
        return None
    n = int(headers.get("content-length", "0"))
    body = b""
    while len(body) < n:
        chunk = sys.stdin.buffer.read(n - len(body))
        if not chunk:
            return None
        body += chunk
    return json.loads(body)

def send(payload):
    raw = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(("Content-Length: %d\r\n\r\n" % len(raw)).encode("ascii") + raw)
    sys.stdout.buffer.flush()

def main():
    while True:
        msg = read_message()
        if msg is None:
            return
        method = msg.get("method")
        msg_id = msg.get("id")
        params = msg.get("params") or {}
        if method == "initialize":
            if INIT_DELAY:
                time.sleep(INIT_DELAY)
            send({"jsonrpc": "2.0", "id": msg_id, "result": {
                "capabilities": {
                    "textDocumentSync": 1,
                    "definitionProvider": True,
                    "hoverProvider": True,
                    "referencesProvider": True,
                    "documentSymbolProvider": True,
                    "workspaceSymbolProvider": True
                }
            }})
        elif method == "textDocument/definition":
            uri = params["textDocument"]["uri"]
            send({"jsonrpc": "2.0", "id": msg_id, "result": {
                "uri": uri,
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 4}}
            }})
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": msg_id, "result": None})
        elif method == "exit":
            return
        elif method == "textDocument/didOpen":
            uri = params["textDocument"]["uri"]
            send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                "uri": uri,
                "diagnostics": [{
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "severity": 1,
                    "message": "fake diagnostic"
                }]
            }})

if __name__ == "__main__":
    main()
"#;

    #[cfg(unix)]
    fn write_fake_ls(dir: &Path, name: &str) -> PathBuf {
        write_fake_ls_delayed(dir, name, 0.0)
    }

    fn write_fake_ls_delayed(dir: &Path, name: &str, init_delay_secs: f64) -> PathBuf {
        let src = FAKE_LS.replace("INIT_DELAY = 0", &format!("INIT_DELAY = {init_delay_secs}"));
        let path = dir.join(name);
        std::fs::write(&path, src).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn pool_for(
        dir: &Path,
        cfg: CodeIntelConfig,
    ) -> (Arc<LspPool>, Arc<Mutex<ProcessOwnerRegistry>>) {
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let fs = Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(dir)));
        let pool = LspPool::new(
            dir.to_path_buf(),
            fs,
            cfg,
            owners.clone(),
            CancellationToken::new(),
        );
        (pool, owners)
    }

    #[cfg(unix)]
    fn fake_cfg(
        command: PathBuf,
        id: &str,
        manifest: &str,
        max_concurrent: u32,
    ) -> CodeIntelConfig {
        let mut cfg = enabled_cfg();
        cfg.lsp.max_concurrent = max_concurrent;
        cfg.lsp.checked = vec![id.to_string()];
        cfg.lsp.custom.push(CustomLspServer {
            id: id.to_string(),
            language: "Fake".into(),
            command: command.to_string_lossy().into_owned(),
            args: vec![],
            extensions: vec![".fake".into()],
            manifests: vec![manifest.to_string()],
        });
        cfg
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pool_initializes_fake_server_and_answers_definition() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_fake_ls(dir.path(), "fake-ls");
        std::fs::write(dir.path().join("fake.manifest"), "").unwrap();
        std::fs::write(dir.path().join("main.fake"), "func\n").unwrap();
        let cfg = fake_cfg(script, "fake-ls", "fake.manifest", 2);
        let (pool, owners) = pool_for(dir.path(), cfg);

        assert!(pool.running_ids().await.is_empty());
        pool.ensure_server("fake-ls").await.unwrap();
        assert_eq!(pool.running_ids().await, vec!["fake-ls".to_string()]);
        assert!(!lock_owners(&owners).pids().is_empty());

        let file = dir.path().join("main.fake");
        pool.did_open("fake-ls", &file, "fake", "func\n".into())
            .await
            .unwrap();
        let def = pool.definition("fake-ls", &file, 0, 0).await.unwrap();
        assert!(
            def.contains("main.fake"),
            "definition should include the file path, got {def}"
        );

        let mut diags = Vec::new();
        for _ in 0..50 {
            diags = pool.diagnostics(&file).await;
            if !diags.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            diags.iter().any(|line| line.contains("fake diagnostic")),
            "expected stored diagnostics, got {diags:?}"
        );

        pool.shutdown_all().await;
        assert!(pool.running_ids().await.is_empty());
        assert!(lock_owners(&owners).pids().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pool_fail_open_when_server_not_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_fake_ls(dir.path(), "fake-ls");
        let cfg = fake_cfg(script, "fake-ls", "fake.manifest", 2);
        let (pool, _) = pool_for(dir.path(), cfg);
        let err = pool.ensure_server("fake-ls").await.unwrap_err();
        assert!(err.contains("not eligible"), "got {err}");
        assert!(pool.running_ids().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pool_does_not_evict_when_at_concurrency_cap() {
        let dir = tempfile::tempdir().unwrap();
        let script_a = write_fake_ls(dir.path(), "fake-ls-a");
        let script_b = write_fake_ls(dir.path(), "fake-ls-b");
        std::fs::write(dir.path().join("a.manifest"), "").unwrap();

        let mut cfg = enabled_cfg();
        cfg.lsp.max_concurrent = 1;
        cfg.lsp.checked = vec!["fake-a".into(), "fake-b".into()];
        cfg.lsp.custom = vec![
            CustomLspServer {
                id: "fake-a".into(),
                language: "Fake".into(),
                command: script_a.to_string_lossy().into_owned(),
                args: vec![],
                extensions: vec![".fake".into()],
                manifests: vec!["a.manifest".into()],
            },
            CustomLspServer {
                id: "fake-b".into(),
                language: "Fake".into(),
                command: script_b.to_string_lossy().into_owned(),
                args: vec![],
                extensions: vec![".fake".into()],
                manifests: vec!["b.manifest".into()],
            },
        ];
        let (pool, _) = pool_for(dir.path(), cfg);
        pool.ensure_server("fake-a").await.unwrap();
        assert_eq!(pool.running_ids().await, vec!["fake-a".to_string()]);

        std::fs::remove_file(dir.path().join("a.manifest")).unwrap();
        std::fs::write(dir.path().join("b.manifest"), "").unwrap();
        let err = pool.ensure_server("fake-b").await.unwrap_err();
        assert!(
            err.contains("LSP concurrency limit (1) reached"),
            "got {err}"
        );
        assert_eq!(pool.running_ids().await, vec!["fake-a".to_string()]);
        pool.shutdown_all().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_all_does_not_block_on_handshake() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_fake_ls_delayed(dir.path(), "fake-ls", 2.0);
        std::fs::write(dir.path().join("fake.manifest"), "").unwrap();
        let cfg = fake_cfg(script, "fake-ls", "fake.manifest", 2);
        let (pool, owners) = pool_for(dir.path(), cfg);

        let pending = {
            let pool = pool.clone();
            tokio::spawn(async move { pool.ensure_server("fake-ls").await })
        };

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !lock_owners(&owners).pids().is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fake server should spawn before initialize returns");

        tokio::time::timeout(Duration::from_secs(1), pool.shutdown_all())
            .await
            .expect("shutdown_all must not wait for initialize");

        let ensure = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("ensure_server should finish after handshake")
            .expect("ensure_server task should not panic");
        assert!(
            ensure.is_err(),
            "handshake after shutdown_all must not stay in the pool, got {ensure:?}"
        );
        assert!(pool.running_ids().await.is_empty());
        assert!(lock_owners(&owners).pids().is_empty());
    }
}

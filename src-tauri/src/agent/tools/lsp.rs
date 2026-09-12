//! Native `lsp` tool: one operation-enum over the session `LspPool`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use sacp::schema::ReadTextFileRequest;
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::acp::file_system_runtime::FileSystemRuntimeError;
use crate::agent::code_intel::{CodeIntelConfig, LspPool};

const UNSUPPORTED_OPERATION: &str =
    "unsupported operation; allowed: definition, references, hover, \
     document_symbol, workspace_symbol, diagnostics, servers";

#[derive(Clone)]
pub struct LspTool {
    ctx: NativeToolCtx,
    pool: Arc<LspPool>,
}

impl LspTool {
    pub fn new(ctx: NativeToolCtx, pool: Arc<LspPool>) -> Self {
        Self { ctx, pool }
    }

    async fn run(&self, args: LspArgs) -> String {
        match args.operation.trim().to_ascii_lowercase().as_str() {
            "definition" => self.definition(&args).await,
            "references" => self.references(&args).await,
            "hover" => self.hover(&args).await,
            "document_symbol" => self.document_symbol(&args).await,
            "workspace_symbol" => self.workspace_symbol(&args).await,
            "diagnostics" => self.diagnostics(&args).await,
            "servers" => self.servers().await,
            _ => UNSUPPORTED_OPERATION.to_string(),
        }
    }

    async fn definition(&self, args: &LspArgs) -> String {
        let (server, path) = match self.open_for(args).await {
            Ok(pair) => pair,
            Err(msg) => return msg,
        };
        let (line, character) = position(args);
        self.pool
            .definition(&server, &path, line, character)
            .await
            .unwrap_or_else(|err| err)
    }

    async fn references(&self, args: &LspArgs) -> String {
        let (server, path) = match self.open_for(args).await {
            Ok(pair) => pair,
            Err(msg) => return msg,
        };
        let (line, character) = position(args);
        self.pool
            .references(&server, &path, line, character)
            .await
            .unwrap_or_else(|err| err)
    }

    async fn hover(&self, args: &LspArgs) -> String {
        let (server, path) = match self.open_for(args).await {
            Ok(pair) => pair,
            Err(msg) => return msg,
        };
        let (line, character) = position(args);
        self.pool
            .hover(&server, &path, line, character)
            .await
            .unwrap_or_else(|err| err)
    }

    async fn document_symbol(&self, args: &LspArgs) -> String {
        let (server, path) = match self.open_for(args).await {
            Ok(pair) => pair,
            Err(msg) => return msg,
        };
        self.pool
            .document_symbol(&server, &path)
            .await
            .unwrap_or_else(|err| err)
    }

    async fn workspace_symbol(&self, args: &LspArgs) -> String {
        let query = args
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(query) = query else {
            return "query required for workspace_symbol".into();
        };
        let path = match optional_path(self, args.path.as_deref()) {
            Ok(path) => path,
            Err(msg) => return msg,
        };
        let server = match self
            .pool
            .pick_server(path.as_deref(), args.server.as_deref())
        {
            Ok(id) => id,
            Err(msg) => return msg,
        };
        self.pool
            .workspace_symbol(&server, query)
            .await
            .unwrap_or_else(|err| err)
    }

    async fn diagnostics(&self, args: &LspArgs) -> String {
        let (_server, path) = match self.open_for(args).await {
            Ok(pair) => pair,
            Err(msg) => return msg,
        };
        let diags = self.pool.diagnostics(&path).await;
        if diags.is_empty() {
            "no diagnostics".into()
        } else {
            diags.join("\n")
        }
    }

    async fn servers(&self) -> String {
        let running = self.pool.running_ids().await;
        let eligible = self.pool.eligible_ids();
        format!(
            "running: {}\neligible: {}",
            format_ids(&running),
            format_ids(&eligible)
        )
    }

    async fn open_for(&self, args: &LspArgs) -> Result<(String, PathBuf), String> {
        let path = resolve_tool_path(&self.ctx, args.path.as_deref())?;
        let server = self.pool.pick_server(Some(&path), args.server.as_deref())?;
        self.open_document(&server, &path).await?;
        Ok((server, path))
    }

    async fn open_document(&self, server_id: &str, path: &Path) -> Result<(), String> {
        self.ctx.fs.check_read(path).map_err(fs_err)?;
        let text = self.read_file_text(path).await?;
        let language_id = self.pool.language_id_for(server_id);
        self.pool
            .did_open(server_id, path, &language_id, text)
            .await
    }

    async fn read_file_text(&self, path: &Path) -> Result<String, String> {
        if path.is_absolute() {
            let request = ReadTextFileRequest::new(self.ctx.session_id.clone(), path);
            return self
                .ctx
                .fs
                .read_text_file(request)
                .await
                .map(|response| response.content)
                .map_err(fs_err);
        }
        std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))
    }
}

fn resolve_tool_path(ctx: &NativeToolCtx, path: Option<&str>) -> Result<PathBuf, String> {
    let Some(raw) = path.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err("path required".into());
    };
    ctx.resolve_path(raw).map_err(|err| {
        err.model_feedback()
            .unwrap_or_else(|| err.message())
            .to_string()
    })
}

fn optional_path(tool: &LspTool, path: Option<&str>) -> Result<Option<PathBuf>, String> {
    match path.map(str::trim).filter(|s| !s.is_empty()) {
        Some(_) => resolve_tool_path(&tool.ctx, path).map(Some),
        None => Ok(None),
    }
}

fn position(args: &LspArgs) -> (u32, u32) {
    (
        args.line.unwrap_or(1).saturating_sub(1),
        args.character.unwrap_or(0),
    )
}

fn format_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "(none)".into()
    } else {
        ids.join(", ")
    }
}

fn fs_err(err: FileSystemRuntimeError) -> String {
    match err {
        FileSystemRuntimeError::InvalidParams(message)
        | FileSystemRuntimeError::Internal(message) => message,
    }
}

#[derive(Debug, Deserialize)]
pub struct LspArgs {
    pub operation: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub character: Option<u32>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

pub(crate) fn should_inject_lsp(cfg: &CodeIntelConfig) -> bool {
    cfg.enabled && cfg.lsp.auto_attach
}

impl Tool for LspTool {
    const NAME: &'static str = "lsp";
    type Args = LspArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Query language servers for definition, references, hover, symbols, and \
         diagnostics. Prefer this for jump-to-definition and typed symbols; \
         fall back to grep if no server is eligible. Read-only."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "definition, references, hover, document_symbol, workspace_symbol, diagnostics, or servers"
                },
                "path": {
                    "type": "string",
                    "description": "File path relative to the session cwd (required except workspace_symbol/servers)"
                },
                "line": {
                    "type": "integer",
                    "description": "1-based line for definition, references, and hover"
                },
                "character": {
                    "type": "integer",
                    "description": "0-based column; defaults to 0"
                },
                "query": {
                    "type": "string",
                    "description": "Symbol query for workspace_symbol"
                },
                "server": {
                    "type": "string",
                    "description": "Optional language server id; otherwise picked from the file"
                }
            },
            "required": ["operation"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "operation": args.operation,
            "path": args.path,
            "line": args.line,
            "character": args.character,
            "query": args.query,
            "server": args.server,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        let out = self.run(args).await;
        self.ctx.finish_ok(fact, out).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
    use crate::acp::process_owner::ProcessOwnerRegistry;
    use crate::agent::code_intel::{default_config, CustomLspServer};
    use crate::agent::tools::{test_tool_ctx, tool_kind, tool_requires_permission};
    use rig::tool::{Tool, ToolContext};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    fn sample_args(operation: &str) -> LspArgs {
        LspArgs {
            operation: operation.to_string(),
            path: None,
            line: None,
            character: None,
            query: None,
            server: None,
        }
    }

    fn pool_for(dir: &Path, cfg: CodeIntelConfig) -> Arc<LspPool> {
        let owners = Arc::new(Mutex::new(ProcessOwnerRegistry::new()));
        let fs = Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(dir)));
        LspPool::new(dir.to_path_buf(), fs, cfg, owners, CancellationToken::new())
    }

    #[test]
    fn lsp_is_search_and_skips_permission() {
        assert_eq!(tool_kind("lsp"), "search");
        assert!(!tool_requires_permission("lsp"));
    }

    #[test]
    fn injects_only_when_master_and_auto_attach() {
        let mut cfg = default_config();
        assert!(!should_inject_lsp(&cfg));
        cfg.enabled = true;
        assert!(should_inject_lsp(&cfg));
        cfg.lsp.auto_attach = false;
        assert!(!should_inject_lsp(&cfg));
    }

    #[tokio::test]
    async fn unknown_operation_is_fail_open() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "lsp", "c1");
        let mut cfg = default_config();
        cfg.enabled = true;
        let tool = LspTool::new(ctx, pool_for(dir.path(), cfg));
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args("not-a-real-op"))
            .await
            .unwrap();
        assert!(out.contains("unsupported"), "{out}");
    }

    #[tokio::test]
    async fn no_eligible_server_is_fail_open() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hi\n").unwrap();
        let ctx = test_tool_ctx(dir.path(), "lsp", "c1");
        let mut cfg = default_config();
        cfg.enabled = true;
        let tool = LspTool::new(ctx, pool_for(dir.path(), cfg));
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                LspArgs {
                    operation: "definition".into(),
                    path: Some("notes.txt".into()),
                    line: Some(1),
                    character: Some(0),
                    query: None,
                    server: None,
                },
            )
            .await
            .unwrap();
        assert!(out.contains("fall back to grep"), "{out}");
        assert!(out.contains("no language server eligible"), "{out}");
    }

    #[cfg(unix)]
    const FAKE_LS: &str = r#"#!/usr/bin/env python3
import json
import sys

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
        let path = dir.join(name);
        std::fs::write(&path, FAKE_LS).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    fn fake_cfg(command: PathBuf, id: &str, manifest: &str) -> CodeIntelConfig {
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.lsp.max_concurrent = 2;
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
    async fn definition_against_fake_server_returns_location() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_fake_ls(dir.path(), "fake-ls");
        std::fs::write(dir.path().join("fake.manifest"), "").unwrap();
        std::fs::write(dir.path().join("main.fake"), "func\n").unwrap();
        let cfg = fake_cfg(script, "fake-ls", "fake.manifest");
        let ctx = test_tool_ctx(dir.path(), "lsp", "c1");
        let pool = pool_for(dir.path(), cfg);
        let tool = LspTool::new(ctx, pool.clone());
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                LspArgs {
                    operation: "definition".into(),
                    path: Some("main.fake".into()),
                    line: Some(1),
                    character: Some(0),
                    query: None,
                    server: None,
                },
            )
            .await
            .unwrap();
        assert!(
            out.contains("main.fake"),
            "definition should include the file path, got {out}"
        );
        pool.shutdown_all().await;
    }
}

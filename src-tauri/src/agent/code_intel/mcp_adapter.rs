//! Project-level MCP adapter: LSP facade + optional CodeGraph proxy.
//!
//! Routes LSP tools straight to [`LspPool`]. Does not speak BrokerMessage,
//! UDS, or the codeg-mcp companion schema.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::acp::file_system_runtime::FileSystemRuntime;

use super::mcp_tools::{lsp_tool_aliases, CODEGRAPH_MCP_TOOLS, LSP_MCP_TOOLS, MCP_SERVER_NAME};
use super::{CodeIntelConfig, LspPool};

#[derive(Clone)]
pub struct CodeIntelMcpAdapter {
    inner: Arc<AdapterInner>,
}

struct AdapterInner {
    cfg: CodeIntelConfig,
    workspace: PathBuf,
    fs: Arc<FileSystemRuntime>,
    lsp_pool: Option<Arc<LspPool>>,
    codegraph: tokio::sync::Mutex<Option<CodegraphProxy>>,
}

struct CodegraphProxy {
    peer: rmcp::Peer<rmcp::RoleClient>,
}

impl CodeIntelMcpAdapter {
    pub fn new(
        cfg: CodeIntelConfig,
        workspace: PathBuf,
        fs: Arc<FileSystemRuntime>,
        lsp_pool: Option<Arc<LspPool>>,
    ) -> Self {
        Self {
            inner: Arc::new(AdapterInner {
                cfg,
                workspace,
                fs,
                lsp_pool,
                codegraph: tokio::sync::Mutex::new(None),
            }),
        }
    }

    pub async fn attach_codegraph(&self, peer: rmcp::Peer<rmcp::RoleClient>) {
        *self.inner.codegraph.lock().await = Some(CodegraphProxy { peer });
    }

    pub async fn listed_tools(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if self.inner.cfg.enabled && self.inner.cfg.lsp.auto_attach {
            tools.extend(LSP_MCP_TOOLS.iter().map(spec_to_tool));
        }
        if self.inner.cfg.enabled && self.inner.cfg.codegraph.enabled {
            let guard = self.inner.codegraph.lock().await;
            if let Some(proxy) = guard.as_ref() {
                match proxy.peer.list_all_tools().await {
                    Ok(remote) if !remote.is_empty() => tools.extend(remote),
                    _ => tools.extend(CODEGRAPH_MCP_TOOLS.iter().map(spec_to_tool)),
                }
            } else {
                tools.extend(CODEGRAPH_MCP_TOOLS.iter().map(spec_to_tool));
            }
        }
        tools
    }

    pub async fn invoke(
        &self,
        name: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> CallToolResult {
        if cancel.is_cancelled() {
            return CallToolResult::error(vec![ContentBlock::text("cancelled")]);
        }
        if lsp_tool_aliases(name) {
            return self.invoke_lsp(name, &args).await;
        }
        self.invoke_codegraph(name, args, cancel).await
    }

    async fn invoke_lsp(&self, name: &str, args: &Value) -> CallToolResult {
        let Some(pool) = self.inner.lsp_pool.as_ref() else {
            return fail_open("LSP MCP tools are not enabled; fall back to grep");
        };
        let op = normalize_lsp_name(name);
        if op == "servers" {
            let running = pool.running_ids().await;
            let eligible = pool.eligible_ids();
            return ok_text(format!(
                "running: {}\neligible: {}",
                format_ids(&running),
                format_ids(&eligible)
            ));
        }
        let path = match required_path(self, args, op != "workspaceSymbol") {
            Ok(path) => path,
            Err(msg) => return fail_open(msg),
        };
        if op == "workspaceSymbol" {
            let Some(query) = args
                .get("query")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return fail_open("query required for workspaceSymbol");
            };
            let server = match pool.pick_server(path.as_deref(), arg_str(args, "server")) {
                Ok(id) => id,
                Err(msg) => return fail_open(msg),
            };
            return match pool.workspace_symbol(&server, query).await {
                Ok(text) => ok_text(text),
                Err(err) => fail_open(err),
            };
        }
        let Some(path) = path else {
            return fail_open("path required");
        };
        if let Err(err) = self.inner.fs.check_read(&path) {
            return fail_open(match err {
                crate::acp::file_system_runtime::FileSystemRuntimeError::InvalidParams(message)
                | crate::acp::file_system_runtime::FileSystemRuntimeError::Internal(message) => {
                    message
                }
            });
        }
        let server = match pool.pick_server(Some(&path), arg_str(args, "server")) {
            Ok(id) => id,
            Err(msg) => return fail_open(msg),
        };
        if let Err(err) = open_document(pool, &self.inner.fs, &server, &path).await {
            return fail_open(err);
        }
        let (line, character) = position(args);
        let result = match op {
            "goToDefinition" => pool.definition(&server, &path, line, character).await,
            "findReferences" => pool.references(&server, &path, line, character).await,
            "hover" => pool.hover(&server, &path, line, character).await,
            "documentSymbol" => pool.document_symbol(&server, &path).await,
            "diagnostics" => Ok({
                let diags = pool.diagnostics(&path).await;
                if diags.is_empty() {
                    "no diagnostics".into()
                } else {
                    diags.join("\n")
                }
            }),
            _ => return fail_open("unsupported LSP MCP tool"),
        };
        match result {
            Ok(text) => ok_text(text),
            Err(err) => fail_open(err),
        }
    }

    async fn invoke_codegraph(
        &self,
        name: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> CallToolResult {
        let guard = self.inner.codegraph.lock().await;
        let Some(proxy) = guard.as_ref() else {
            return fail_open(
                "CodeGraph MCP is not ready; host is initializing or the binary is missing. Fall back to grep.",
            );
        };
        let mut params = CallToolRequestParams::new(name.to_string());
        params.arguments = args.as_object().cloned();
        tokio::select! {
            _ = cancel.cancelled() => fail_open("cancelled"),
            result = proxy.peer.call_tool(params) => match result {
                Ok(value) => value,
                Err(err) => fail_open(err.to_string()),
            }
        }
    }
}

impl ServerHandler for CodeIntelMcpAdapter {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                MCP_SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Project-hosted LSP facade and official CodeGraph tools. Language servers and codegraph stay in the Codeg process; this session only calls MCP tools.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move { Ok(ListToolsResult::with_all_items(self.listed_tools().await)) }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            let args = request
                .arguments
                .map(Value::Object)
                .unwrap_or(Value::Object(JsonObject::new()));
            Ok(self.invoke(request.name.as_ref(), args, context.ct).await)
        }
    }
}

fn spec_to_tool(spec: &super::mcp_tools::McpToolSpec) -> Tool {
    let schema = lsp_input_schema(spec.name);
    let mut tool = Tool::new(spec.name, spec.description, schema);
    tool.annotations = Some(ToolAnnotations::from_raw(
        Some(spec.name.to_string()),
        Some(true),
        Some(false),
        Some(true),
        Some(false),
    ));
    tool
}

fn lsp_input_schema(name: &str) -> Arc<JsonObject> {
    let mut properties = json!({
        "path": { "type": "string", "description": "File path, absolute or workspace-relative" },
        "server": { "type": "string", "description": "Optional language server id" }
    });
    if matches!(
        name,
        "goToDefinition" | "findReferences" | "hover" | "definition" | "references"
    ) {
        properties["line"] = json!({ "type": "integer", "description": "1-based line" });
        properties["character"] = json!({ "type": "integer", "description": "0-based column" });
    }
    if name == "workspaceSymbol" || name == "codegraph_explore" {
        properties["query"] = json!({ "type": "string" });
    }
    let mut map = JsonObject::new();
    map.insert("type".into(), json!("object"));
    map.insert("properties".into(), properties);
    if name == "workspaceSymbol" {
        map.insert("required".into(), json!(["query"]));
    } else if name != "servers" {
        map.insert("required".into(), json!(["path"]));
    }
    Arc::new(map)
}

fn normalize_lsp_name(name: &str) -> &str {
    match name {
        "definition" => "goToDefinition",
        "references" => "findReferences",
        "document_symbol" => "documentSymbol",
        "workspace_symbol" => "workspaceSymbol",
        other => other,
    }
}

fn required_path(
    adapter: &CodeIntelMcpAdapter,
    args: &Value,
    required: bool,
) -> Result<Option<PathBuf>, String> {
    let raw = arg_str(args, "path")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match raw {
        None if required => Err("path required".into()),
        None => Ok(None),
        Some(raw) => {
            let path = if Path::new(raw).is_absolute() {
                PathBuf::from(raw)
            } else {
                adapter.inner.workspace.join(raw)
            };
            Ok(Some(path))
        }
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn position(args: &Value) -> (u32, u32) {
    let line = args
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .saturating_sub(1) as u32;
    let character = args.get("character").and_then(Value::as_u64).unwrap_or(0) as u32;
    (line, character)
}

async fn open_document(
    pool: &LspPool,
    fs: &FileSystemRuntime,
    server: &str,
    path: &Path,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let _ = fs.check_read(path);
    let language_id = pool.language_id_for(server, Some(path));
    pool.did_open(server, path, &language_id, text).await
}

fn format_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "(none)".into()
    } else {
        ids.join(", ")
    }
}

fn ok_text(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text.into())])
}

fn fail_open(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::file_system_runtime::FsAccessPolicy;
    use crate::agent::code_intel::default_config;

    fn adapter_for(dir: &Path, mut cfg: CodeIntelConfig) -> CodeIntelMcpAdapter {
        cfg.enabled = true;
        cfg.lsp.auto_attach = true;
        let fs = Arc::new(FileSystemRuntime::with_policy(FsAccessPolicy::strict(dir)));
        CodeIntelMcpAdapter::new(cfg, dir.to_path_buf(), fs, None)
    }

    #[tokio::test]
    async fn lists_lsp_mcp_tools_not_language_servers() {
        let dir = tempfile::tempdir().unwrap();
        let adapter = adapter_for(dir.path(), default_config());
        let names: Vec<_> = adapter
            .listed_tools()
            .await
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(names.contains(&"goToDefinition".into()));
        assert!(names.contains(&"findReferences".into()));
        assert!(!names.iter().any(|n| n == "rust-analyzer" || n == "gopls"));
    }

    #[tokio::test]
    async fn missing_pool_is_fail_open() {
        let dir = tempfile::tempdir().unwrap();
        let adapter = adapter_for(dir.path(), default_config());
        let result = adapter
            .invoke(
                "goToDefinition",
                json!({"path": "src/lib.rs", "line": 1, "character": 0}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
    }
}

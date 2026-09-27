//! Project MCP adapter. Proxies `tools/list` and `tools/call` from child
//! servers. Schemas and `readOnlyHint` values are the children's, not invented here.

use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::ErrorData as McpError;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::filter::{merge_provider_tools, tool_denied, ToolProvider};
use super::MCP_SERVER_NAME;

#[derive(Clone)]
pub struct CodeIntelMcpAdapter {
    inner: Arc<AdapterInner>,
}

struct AdapterInner {
    children: tokio::sync::Mutex<Vec<ChildSession>>,
}

struct ChildSession {
    provider: ToolProvider,
    peer: rmcp::Peer<rmcp::RoleClient>,
}

impl CodeIntelMcpAdapter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AdapterInner {
                children: tokio::sync::Mutex::new(Vec::new()),
            }),
        }
    }

    pub async fn attach_provider(
        &self,
        provider: ToolProvider,
        peer: rmcp::Peer<rmcp::RoleClient>,
    ) {
        let mut children = self.inner.children.lock().await;
        children.retain(|child| child.provider != provider);
        children.push(ChildSession { provider, peer });
    }

    pub async fn listed_tools(&self) -> Vec<rmcp::model::Tool> {
        let children = self.inner.children.lock().await;
        let mut groups = Vec::with_capacity(children.len());
        for child in children.iter() {
            let tools = match child.peer.list_all_tools().await {
                Ok(tools) => tools,
                Err(err) => {
                    tracing::debug!(
                        provider = provider_label(child.provider),
                        error = %err,
                        "code-intel tools/list failed"
                    );
                    Vec::new()
                }
            };
            groups.push((child.provider, tools));
        }
        merge_provider_tools(groups)
    }

    pub async fn invoke(
        &self,
        name: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> CallToolResult {
        if cancel.is_cancelled() {
            return fail_open("cancelled");
        }
        let children = self.inner.children.lock().await;
        for child in children.iter() {
            if tool_denied(child.provider, name) {
                continue;
            }
            let listed = match child.peer.list_all_tools().await {
                Ok(tools) => tools,
                Err(err) => {
                    tracing::debug!(
                        provider = provider_label(child.provider),
                        error = %err,
                        "code-intel tools/list failed before call"
                    );
                    continue;
                }
            };
            if !listed
                .iter()
                .any(|tool| tool.name.as_ref() == name && !tool_denied(child.provider, name))
            {
                continue;
            }
            let peer = child.peer.clone();
            drop(children);
            let mut params = CallToolRequestParams::new(name.to_string());
            params.arguments = args.as_object().cloned();
            return tokio::select! {
                _ = cancel.cancelled() => fail_open("cancelled"),
                result = peer.call_tool(params) => match result {
                    Ok(value) => value,
                    Err(err) => fail_open(err.to_string()),
                }
            };
        }
        fail_open(format!("tool `{name}` is not available"))
    }
}

impl Default for CodeIntelMcpAdapter {
    fn default() -> Self {
        Self::new()
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
                "Project code tools proxied from Serena and CodeGraph. Names, descriptions, and input schemas are the upstream servers'. Denied management and duplicate file tools are hidden.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.listed_tools().await))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(JsonObject::new()));
        Ok(self.invoke(request.name.as_ref(), args, context.ct).await)
    }
}

fn provider_label(provider: ToolProvider) -> &'static str {
    match provider {
        ToolProvider::Serena => "serena",
        ToolProvider::Codegraph => "codegraph",
    }
}

fn fail_open(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text.into())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_adapter_lists_no_static_catalog() {
        let adapter = CodeIntelMcpAdapter::new();
        assert!(adapter.listed_tools().await.is_empty());
        let result = adapter
            .invoke(
                "goToDefinition",
                serde_json::json!({}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(result.is_error, Some(true));
        let result = adapter
            .invoke("read_file", serde_json::json!({}), CancellationToken::new())
            .await;
        assert_eq!(result.is_error, Some(true));
    }
}

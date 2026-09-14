//! Catalog of MCP tools the project code-intel adapter advertises.
//!
//! Agents see these names — not language-server binaries. Host processes
//! (rust-analyzer, gopls, `codegraph serve --mcp`) stay behind the supervisor.

use super::CodeIntelConfig;

pub const MCP_SERVER_NAME: &str = "codeg-code-intel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub group: &'static str,
    pub description: &'static str,
}

pub const LSP_MCP_TOOLS: &[McpToolSpec] = &[
    McpToolSpec {
        name: "goToDefinition",
        group: "lsp",
        description: "Jump to the definition of a symbol (textDocument/definition).",
    },
    McpToolSpec {
        name: "findReferences",
        group: "lsp",
        description: "Find references to a symbol (textDocument/references).",
    },
    McpToolSpec {
        name: "hover",
        group: "lsp",
        description: "Hover information for a position (textDocument/hover).",
    },
    McpToolSpec {
        name: "documentSymbol",
        group: "lsp",
        description: "List symbols in a file (textDocument/documentSymbol).",
    },
    McpToolSpec {
        name: "workspaceSymbol",
        group: "lsp",
        description: "Search workspace symbols (workspace/symbol).",
    },
    McpToolSpec {
        name: "diagnostics",
        group: "lsp",
        description: "Cached diagnostics for a file (publishDiagnostics).",
    },
    McpToolSpec {
        name: "servers",
        group: "lsp",
        description: "Eligible and running language servers for this workspace.",
    },
];

pub const CODEGRAPH_MCP_TOOLS: &[McpToolSpec] = &[McpToolSpec {
    name: "codegraph_explore",
    group: "codegraph",
    description: "Official CodeGraph explore: structure, paths, and impact for a query.",
}];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CodeIntelMcpToolStatus {
    pub name: String,
    pub group: String,
    pub description: String,
    pub advertised: bool,
}

pub fn advertised_mcp_tools(cfg: &CodeIntelConfig) -> Vec<CodeIntelMcpToolStatus> {
    let lsp_on = cfg.enabled && cfg.lsp.auto_attach;
    let graph_on = cfg.enabled && cfg.codegraph.enabled;
    LSP_MCP_TOOLS
        .iter()
        .chain(CODEGRAPH_MCP_TOOLS.iter())
        .map(|spec| CodeIntelMcpToolStatus {
            name: spec.name.to_string(),
            group: spec.group.to_string(),
            description: spec.description.to_string(),
            advertised: if spec.group == "lsp" {
                lsp_on
            } else {
                graph_on
            },
        })
        .collect()
}

pub fn lsp_tool_aliases(name: &str) -> bool {
    matches!(
        name,
        "goToDefinition"
            | "definition"
            | "findReferences"
            | "references"
            | "hover"
            | "documentSymbol"
            | "document_symbol"
            | "workspaceSymbol"
            | "workspace_symbol"
            | "diagnostics"
            | "servers"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::code_intel::default_config;

    #[test]
    fn disabled_config_advertises_no_tools() {
        let cfg = default_config();
        assert!(!cfg.enabled);
        let tools = advertised_mcp_tools(&cfg);
        assert!(tools.iter().all(|t| !t.advertised));
        assert!(tools.iter().any(|t| t.name == "goToDefinition"));
        assert!(tools.iter().any(|t| t.name == "codegraph_explore"));
        assert!(!tools.iter().any(|t| t.name == "rust-analyzer"));
    }

    #[test]
    fn master_and_lsp_advertise_facade_not_servers() {
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.lsp.auto_attach = true;
        cfg.codegraph.enabled = false;
        let tools = advertised_mcp_tools(&cfg);
        assert!(tools
            .iter()
            .filter(|t| t.group == "lsp")
            .all(|t| t.advertised));
        assert!(tools
            .iter()
            .filter(|t| t.group == "codegraph")
            .all(|t| !t.advertised));
    }
}

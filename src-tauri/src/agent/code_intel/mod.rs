mod argv;
mod codegraph;
mod config;
mod detect;
mod discover;
mod filter;
mod install;
mod lsp_select;
mod mcp_adapter;
mod supervisor;
mod workspace;

pub const MCP_SERVER_NAME: &str = "codeg-code-intel";

pub use codegraph::{
    codegraph_has_index, should_run_host_index, spawn_host_index, HostIndexAction,
};
pub use config::*;
pub use detect::*;
pub use discover::{
    code_tools_managed_root, discover_codegraph, resolve_codegraph_binary, resolve_serena,
    DiscoveryKind, SerenaResolution,
};
pub use filter::{merge_provider_tools, tool_denied, ToolProvider};
pub use install::{
    install_verified_bytes, production_download_manifests, refuse_manifest_download, InstallError,
};
pub use lsp_select::{official_lsp_provider_ids, plan_lsp_starts, LspStartInput, LspStartPlan};
pub use mcp_adapter::CodeIntelMcpAdapter;
pub use supervisor::{
    insert_native_stdio_connector, parse_stdio_bridge_url, run_stdio_http_bridge,
    runtime_snapshot_for, stdio_connector_command, CodeIntelRuntimeSnapshot, ProjectCodeIntelLease,
    ProjectCodeIntelSupervisor, ProviderRuntimeSnapshot,
};
pub use workspace::canonical_workspace;

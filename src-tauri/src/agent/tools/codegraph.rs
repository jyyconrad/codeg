//! Native `codegraph` tool: whitelist query operations over the CodeGraph CLI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::acp::process_owner::ProcessOwnerRegistry;
use crate::agent::code_intel::{
    build_codegraph_argv, codegraph_has_index, spawn_codegraph, CodeIntelConfig, CodegraphOp,
    CodegraphRun, CODEGRAPH_QUERY_TIMEOUT,
};

const UNSUPPORTED_OPERATION: &str =
    "unsupported operation; allowed: explore, query, node, callers, callees, impact, files, status";
const MISSING_BINARY: &str =
    "codegraph binary not found. Install with `npm i -g @colbymchenry/codegraph` and retry, or fall back to grep.";
const MISSING_INDEX: &str =
    "index missing; host is initializing or run is pending; fall back to grep";

#[derive(Clone)]
pub struct CodegraphTool {
    ctx: NativeToolCtx,
    binary: PathBuf,
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
}

impl CodegraphTool {
    pub fn new(
        ctx: NativeToolCtx,
        binary: PathBuf,
        owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    ) -> Self {
        Self {
            ctx,
            binary,
            owners,
        }
    }

    async fn run(&self, args: CodegraphArgs) -> String {
        let op = match parse_operation(args.operation.as_deref()) {
            Ok(op) => op,
            Err(msg) => return msg,
        };
        if !self.binary.is_file() {
            return MISSING_BINARY.to_string();
        }
        if !codegraph_has_index(&self.ctx.launch_cwd) {
            return MISSING_INDEX.to_string();
        }
        let argv = match build_codegraph_argv(
            op,
            args.query.as_deref(),
            args.path.as_deref(),
            args.kind.as_deref(),
            args.limit,
            args.depth,
        ) {
            Ok(argv) => argv,
            Err(msg) => return msg,
        };
        match spawn_codegraph(
            &self.binary,
            &self.ctx.launch_cwd,
            &argv,
            self.owners.clone(),
            self.ctx.cancel.clone(),
            CODEGRAPH_QUERY_TIMEOUT,
        )
        .await
        {
            Ok(run) => present_run(run),
            Err(err) => err,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CodegraphArgs {
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub depth: Option<u32>,
}

pub(crate) fn should_inject_codegraph(cfg: &CodeIntelConfig) -> bool {
    cfg.enabled && cfg.codegraph.enabled
}

fn parse_operation(raw: Option<&str>) -> Result<CodegraphOp, String> {
    let trimmed = raw.unwrap_or("").trim();
    let name = if trimmed.is_empty() {
        "explore"
    } else {
        trimmed
    };
    CodegraphOp::parse(name).ok_or_else(|| UNSUPPORTED_OPERATION.to_string())
}

fn present_run(run: CodegraphRun) -> String {
    let body = if run.status == 0 {
        run.stdout
    } else if run.stdout.is_empty() {
        run.stderr
    } else if run.stderr.is_empty() {
        run.stdout
    } else {
        format!("{}\n{}", run.stdout.trim_end(), run.stderr)
    };
    if body.trim().is_empty() {
        "(no output)".into()
    } else {
        body
    }
}

impl Tool for CodegraphTool {
    const NAME: &'static str = "codegraph";
    type Args = CodegraphArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Query the repository code graph for structure, impact, and how X reaches Y. \
         Default operation is explore. Use this for call graphs and change impact; \
         do not chain callers, callees, and impact for one question. If the graph \
         is unavailable, fall back to grep. Never install or run codegraph via bash."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "explore (default), query, node, callers, callees, impact, files, or status"
                },
                "query": {
                    "type": "string",
                    "description": "Search string or symbol (required for explore, query, node, callers, callees, impact)"
                },
                "path": {
                    "type": "string",
                    "description": "Optional path relative to the session cwd (files)"
                },
                "kind": {
                    "type": "string",
                    "description": "Optional kind filter for query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional result limit for query"
                },
                "depth": {
                    "type": "integer",
                    "description": "Optional depth for impact"
                }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "operation": args.operation,
            "query": args.query,
            "path": args.path,
            "kind": args.kind,
            "limit": args.limit,
            "depth": args.depth,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        let out = self.run(args).await;
        self.ctx.finish_ok(fact, out).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::code_intel::default_config;
    use crate::agent::tools::{test_tool_ctx, tool_kind, tool_requires_permission};
    use rig::tool::{Tool, ToolContext};

    fn sample_args(operation: Option<&str>, query: Option<&str>) -> CodegraphArgs {
        CodegraphArgs {
            operation: operation.map(str::to_string),
            query: query.map(str::to_string),
            path: None,
            kind: None,
            limit: None,
            depth: None,
        }
    }

    #[tokio::test]
    async fn unknown_operation_does_not_spawn_install() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "codegraph", "c1");
        let tool = CodegraphTool::new(ctx, dir.path().join("nope"), None);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args(Some("install"), None))
            .await
            .unwrap();
        assert!(out.contains("unsupported"));
        assert!(!out.to_lowercase().contains("npm"));
    }

    #[tokio::test]
    async fn missing_binary_returns_install_hint() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "codegraph", "c1");
        let tool = CodegraphTool::new(ctx, dir.path().join("does-not-exist"), None);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args(Some("explore"), Some("auth")))
            .await
            .unwrap();
        assert!(out.contains("@colbymchenry/codegraph") || out.contains("codegraph"));
    }

    #[test]
    fn tool_is_search_and_skips_permission() {
        assert_eq!(tool_kind("codegraph"), "search");
        assert!(!tool_requires_permission("codegraph"));
    }

    #[test]
    fn injects_only_when_master_and_codegraph_enabled() {
        let mut cfg = default_config();
        assert!(!should_inject_codegraph(&cfg));
        cfg.enabled = true;
        assert!(should_inject_codegraph(&cfg));
        cfg.codegraph.enabled = false;
        assert!(!should_inject_codegraph(&cfg));
    }

    #[tokio::test]
    async fn missing_index_returns_grep_fallback_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("fake-codegraph");
        std::fs::write(&binary, "SHOULD_NOT_RUN").unwrap();
        let ctx = test_tool_ctx(dir.path(), "codegraph", "c1");
        let tool = CodegraphTool::new(ctx, binary, None);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args(Some("explore"), Some("auth")))
            .await
            .unwrap();
        let lower = out.to_lowercase();
        assert!(lower.contains("index") || lower.contains("grep"), "{out}");
        assert!(!out.contains("SHOULD_NOT_RUN"), "{out}");
        assert!(!lower.contains("npm"), "{out}");
    }

    #[tokio::test]
    async fn empty_query_finish_ok_without_erroring_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".codegraph")).unwrap();
        let binary = dir.path().join("fake-codegraph");
        std::fs::write(&binary, "SHOULD_NOT_RUN").unwrap();
        let ctx = test_tool_ctx(dir.path(), "codegraph", "c1");
        let tool = CodegraphTool::new(ctx, binary, None);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args(Some("explore"), Some("  ")))
            .await
            .unwrap();
        assert!(out.contains("query"), "{out}");
        assert!(!out.contains("SHOULD_NOT_RUN"), "{out}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn empty_operation_defaults_to_explore_and_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".codegraph")).unwrap();
        let binary = dir.path().join("fake-codegraph");
        std::fs::write(&binary, "#!/bin/sh\necho graph-ok argv:$@\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ctx = test_tool_ctx(dir.path(), "codegraph", "c1");
        let tool = CodegraphTool::new(ctx, binary, None);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(&mut tctx, sample_args(None, Some("auth")))
            .await
            .unwrap();
        assert!(out.contains("graph-ok"), "{out}");
        assert!(out.contains("explore"), "{out}");
        assert!(out.contains("auth"), "{out}");
    }
}

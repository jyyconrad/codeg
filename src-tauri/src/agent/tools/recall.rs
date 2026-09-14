//! Reload spilled or stored tool output by `tool_call_id` (DSH recall / Pi spill).

use std::path::Path;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::agent::context::spill::{is_under_spill_dir, read_spill, spill_file_path};
use crate::agent::context::ExecutionFact;

const DEFAULT_LIMIT: u32 = 2000;

#[derive(Clone)]
pub struct RecallTool {
    ctx: NativeToolCtx,
}

impl RecallTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

#[derive(Debug, Deserialize)]
pub struct RecallArgs {
    pub tool_call_id: String,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

impl Tool for RecallTool {
    const NAME: &'static str = "recall";
    type Args = RecallArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Reload the full output of a previous tool call after it was truncated, \
         spilled, or omitted from context. Identify the call by tool_call_id from \
         the tool result. Optional offset (1-based line) and limit page large output."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "tool_call_id": { "type": "string", "description": "Id of the original tool call" },
                "offset": { "type": "integer", "description": "1-based starting line" },
                "limit": { "type": "integer", "description": "Maximum lines to return" }
            },
            "required": ["tool_call_id"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "tool_call_id": args.tool_call_id,
            "offset": args.offset,
            "limit": args.limit,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match recall(&self.ctx, args) {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

fn recall(ctx: &NativeToolCtx, args: RecallArgs) -> Result<String, ToolExecutionError> {
    let id = args.tool_call_id.trim();
    if id.is_empty() {
        return Err(
            ToolExecutionError::invalid_args("tool_call_id must not be empty")
                .with_model_feedback("tool_call_id must not be empty"),
        );
    }
    if matches!(args.offset, Some(0)) {
        return Err(
            ToolExecutionError::invalid_args("offset must be >= 1 (1-based line number)")
                .with_model_feedback("offset must be >= 1"),
        );
    }
    let store = ctx.recorder.store();
    let fact = store
        .lock()
        .expect("store")
        .fact(id)
        .cloned()
        .ok_or_else(|| {
            ToolExecutionError::not_found(format!("no tool result for `{id}`"))
                .with_model_feedback(format!("no stored result for tool_call_id `{id}`"))
        })?;
    let body = load_body(&ctx.spill_dir, &fact, id)?;
    Ok(page_text(
        &body,
        args.offset.unwrap_or(1),
        args.limit.unwrap_or(DEFAULT_LIMIT),
    ))
}

fn load_body(
    spill_dir: &Path,
    fact: &ExecutionFact,
    id: &str,
) -> Result<String, ToolExecutionError> {
    if !spill_dir.as_os_str().is_empty() {
        if let Ok(body) = read_spill(spill_dir, id) {
            return Ok(body);
        }
        if let Some(path) = fact
            .output_locator
            .as_ref()
            .and_then(|locator| locator.path.as_deref())
        {
            let path = Path::new(path);
            if is_under_spill_dir(spill_dir, path) {
                return std::fs::read_to_string(path).map_err(|_| {
                    ToolExecutionError::not_found(format!("spilled output missing for `{id}`"))
                        .with_model_feedback(format!("spilled output missing for `{id}`"))
                });
            }
        }
        let expected = spill_file_path(spill_dir, id);
        if expected.exists() {
            return std::fs::read_to_string(&expected).map_err(|_| {
                ToolExecutionError::not_found(format!("spilled output missing for `{id}`"))
                    .with_model_feedback(format!("spilled output missing for `{id}`"))
            });
        }
    }
    fact.model_presentation
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolExecutionError::not_found(format!("no spilled or stored output for `{id}`"))
                .with_model_feedback(format!(
                    "no spilled output for tool_call_id `{id}`; the original capture was not kept"
                ))
        })
}

fn page_text(text: &str, offset: u32, limit: u32) -> String {
    let lines: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        text.lines().collect()
    };
    let start = offset.saturating_sub(1) as usize;
    let end = start.saturating_add(limit as usize);
    let slice = if start >= lines.len() {
        &[][..]
    } else {
        &lines[start..end.min(lines.len())]
    };
    let shown = slice.len() as u32;
    let last = if shown == 0 {
        offset.saturating_sub(1)
    } else {
        offset.saturating_add(shown.saturating_sub(1))
    };
    let mut out = format!("# recall (lines {offset}-{last})\n{}", slice.join("\n"));
    if shown >= limit && end < lines.len() {
        out.push_str(&format!(
            "\n[truncated: showing {shown} lines; pass offset={} to continue]",
            offset.saturating_add(shown)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::spill::write_spill;
    use crate::agent::context::{ExecutionFact, ToolOutcome, ToolPhase};
    use crate::agent::tools::test_tool_ctx;
    use rig::tool::Tool;

    fn record_fact(ctx: &NativeToolCtx, id: &str, body: &str, truncated: bool) {
        let mut fact = ExecutionFact::pending("s:1", id, "bash", json!({"command": "ls"}));
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(ToolOutcome::Success);
        fact.executed = Some(true);
        fact.model_presentation = Some(body.to_string());
        fact.truncated = truncated;
        ctx.recorder
            .store()
            .lock()
            .expect("store")
            .record_fact(fact);
    }

    #[tokio::test]
    async fn recall_reads_spill_not_truncated_store() {
        let dir = tempfile::tempdir().expect("dir");
        let ctx = test_tool_ctx(dir.path(), "recall", "call_r");
        write_spill(&ctx.spill_dir, "call_bash", "FULL-SPILL-BODY\nline2").expect("spill");
        record_fact(&ctx, "call_bash", "truncated", true);
        ctx.identity.set(crate::agent::context::CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_r".into(),
            function_name: "recall".into(),
        });
        let out = RecallTool::new(ctx)
            .call(
                &mut ToolContext::new(),
                RecallArgs {
                    tool_call_id: "call_bash".into(),
                    offset: None,
                    limit: None,
                },
            )
            .await
            .expect("recall");
        assert!(out.contains("FULL-SPILL-BODY"), "{out}");
        assert!(
            !out.contains("truncated") || out.contains("FULL-SPILL-BODY"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn recall_pages_lines() {
        let dir = tempfile::tempdir().expect("dir");
        let ctx = test_tool_ctx(dir.path(), "recall", "call_r");
        let body = (1..=5)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        record_fact(&ctx, "call_bash", &body, false);
        ctx.identity.set(crate::agent::context::CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_r".into(),
            function_name: "recall".into(),
        });
        let out = RecallTool::new(ctx)
            .call(
                &mut ToolContext::new(),
                RecallArgs {
                    tool_call_id: "call_bash".into(),
                    offset: Some(2),
                    limit: Some(2),
                },
            )
            .await
            .expect("page");
        assert!(out.contains("L2"), "{out}");
        assert!(out.contains("L3"), "{out}");
        assert!(!out.contains("L1\n"), "{out}");
        assert!(out.contains("offset=4"), "{out}");
    }

    #[tokio::test]
    async fn recall_unknown_id_fails() {
        let dir = tempfile::tempdir().expect("dir");
        let ctx = test_tool_ctx(dir.path(), "recall", "call_r");
        ctx.identity.set(crate::agent::context::CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_r".into(),
            function_name: "recall".into(),
        });
        let err = RecallTool::new(ctx)
            .call(
                &mut ToolContext::new(),
                RecallArgs {
                    tool_call_id: "missing".into(),
                    offset: None,
                    limit: None,
                },
            )
            .await
            .expect_err("missing");
        assert!(err.to_string().contains("no tool result") || err.to_string().contains("missing"));
    }
}

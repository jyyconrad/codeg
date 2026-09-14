//! Dedicated writes for plan.md / explore.md under ~/.codeg/codeg-agent/artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::agent::mode::{explore_path, plan_path};

#[derive(Clone)]
pub struct WritePlanTool {
    ctx: NativeToolCtx,
    artifacts_dir: PathBuf,
}

#[derive(Clone)]
pub struct WriteExploreReportTool {
    ctx: NativeToolCtx,
    artifacts_dir: PathBuf,
}

impl WritePlanTool {
    pub fn new(ctx: NativeToolCtx, artifacts_dir: PathBuf) -> Self {
        Self { ctx, artifacts_dir }
    }
}

impl WriteExploreReportTool {
    pub fn new(ctx: NativeToolCtx, artifacts_dir: PathBuf) -> Self {
        Self { ctx, artifacts_dir }
    }
}

#[derive(Debug, Deserialize)]
pub struct WriteArtifactArgs {
    pub content: String,
}

fn write_exact(dir: &Path, expected: &Path, content: &str) -> Result<String, ToolExecutionError> {
    fs::create_dir_all(dir).map_err(|err| {
        ToolExecutionError::other(format!("failed to create artifacts dir: {err}"))
            .with_model_feedback("could not create the artifacts directory")
    })?;
    let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if expected.parent() != Some(dir) && expected.parent() != Some(canonical_dir.as_path()) {
        return Err(ToolExecutionError::permission_denied(
            "write is limited to the session artifacts file",
        )
        .with_model_feedback("this tool can only write the session artifact file"));
    }
    fs::write(expected, content).map_err(|err| {
        ToolExecutionError::other(format!("failed to write {}: {err}", expected.display()))
            .with_model_feedback("failed to write the artifact file")
    })?;
    Ok(format!(
        "wrote {} bytes to {}",
        content.len(),
        expected.display()
    ))
}

impl Tool for WritePlanTool {
    const NAME: &'static str = "write_plan";
    type Args = WriteArtifactArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Write the session implementation plan to the Codeg artifacts directory. \
The path is fixed; pass only Markdown content. This is the only file you may write in plan mode."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Full Markdown plan" }
            },
            "required": ["content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let path = plan_path(&self.artifacts_dir);
        let raw = json!({ "content": args.content });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match write_exact(&self.artifacts_dir, &path, &args.content) {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

impl Tool for WriteExploreReportTool {
    const NAME: &'static str = "write_explore_report";
    type Args = WriteArtifactArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Write the explore report to the Codeg artifacts directory. \
The path is fixed; pass only Markdown content. This is the only file you may write."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "Full Markdown explore report" }
            },
            "required": ["content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let path = explore_path(&self.artifacts_dir);
        let raw = json!({ "content": args.content });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match write_exact(&self.artifacts_dir, &path, &args.content) {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::test_tool_ctx;
    use rig::tool::Tool;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_plan_writes_only_plan_md() {
        let dir = tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "write_plan", "c1");
        let tool = WritePlanTool::new(ctx, dir.path().to_path_buf());
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                WriteArtifactArgs {
                    content: "# Plan\n".into(),
                },
            )
            .await
            .expect("write");
        assert!(out.contains("plan.md"), "{out}");
        assert_eq!(
            fs::read_to_string(dir.path().join("plan.md")).unwrap(),
            "# Plan\n"
        );
    }

    #[tokio::test]
    async fn write_explore_report_writes_explore_md() {
        let dir = tempdir().unwrap();
        let ctx = test_tool_ctx(dir.path(), "write_explore_report", "c2");
        let tool = WriteExploreReportTool::new(ctx, dir.path().to_path_buf());
        let mut tctx = ToolContext::new();
        tool.call(
            &mut tctx,
            WriteArtifactArgs {
                content: "# Explore\n".into(),
            },
        )
        .await
        .expect("write");
        assert_eq!(
            fs::read_to_string(dir.path().join("explore.md")).unwrap(),
            "# Explore\n"
        );
    }
}

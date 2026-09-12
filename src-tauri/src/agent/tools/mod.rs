pub mod artifact;
pub mod bash;
pub mod codegraph;
pub mod companion;
pub mod fs;
pub mod lsp;
pub mod mcp;
pub mod plan;
pub mod plan_mode;
pub mod recall;
pub mod search;
pub mod skill;
pub mod subagent;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::acp::file_system_runtime::{FileSystemRuntime, FileSystemRuntimeError};
use crate::agent::context::{
    CallIdentityBridge, ExecutionFact, FactRecorder, ToolOutcome, ToolPhase,
};

pub use artifact::{WriteExploreReportTool, WritePlanTool};
pub use bash::BashTool;
pub use codegraph::CodegraphTool;
pub(crate) use companion::{
    build_companion_tools, companion_plan_from_injection, is_companion_tool,
    projected_tool_call_ids, schema_for_companion_def, CompanionPlan, CompanionRuntime,
    FeedbackDelivery,
};
pub use fs::{EditFileTool, ReadFileTool, WriteFileTool};
pub use lsp::LspTool;
pub use mcp::{mcp_tool_requires_permission, McpSession, McpTimeouts};
pub use plan::UpdatePlanTool;
pub use plan_mode::{EnterPlanModeTool, ExitPlanModeTool};
pub use recall::RecallTool;
pub use search::{GlobTool, GrepTool};
pub use skill::{SkillCatalog, SkillTool};
pub use subagent::{
    attach_subagent_extra_context, NativeInject, SubagentTable, SubagentTool, SUBAGENT_SPEC_ID,
};

/// Native tool call context: identity, facts, cancel, cwd, and fs runtime.
#[derive(Clone)]
pub struct NativeToolCtx {
    pub turn_id: u64,
    pub identity: Arc<CallIdentityBridge>,
    pub recorder: Arc<FactRecorder>,
    pub cancel: CancellationToken,
    pub launch_cwd: PathBuf,
    pub fs: Arc<FileSystemRuntime>,
    pub session_id: String,
    /// Session-local directory for spilled tool output (`recall`).
    pub spill_dir: PathBuf,
}

impl NativeToolCtx {
    pub fn resolve_path(&self, path: &str) -> Result<PathBuf, ToolExecutionError> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(ToolExecutionError::invalid_args("path must not be empty")
                .with_model_feedback("path must not be empty"));
        }
        let raw = PathBuf::from(trimmed);
        Ok(if raw.is_absolute() {
            raw
        } else {
            self.launch_cwd.join(raw)
        })
    }

    pub async fn begin(
        &self,
        name: &str,
        args: Value,
    ) -> Result<ExecutionFact, ToolExecutionError> {
        if self.cancel.is_cancelled() {
            return Err(ToolExecutionError::cancelled("tool was cancelled")
                .with_model_feedback("tool was cancelled before execution"));
        }
        let identity = self.identity.require(self.turn_id).map_err(|err| {
            ToolExecutionError::invalid_args(err).with_model_feedback("call identity rejected")
        })?;
        if identity.function_name != name {
            return Err(ToolExecutionError::invalid_args(format!(
                "call identity function `{}` does not match `{name}`",
                identity.function_name
            ))
            .with_model_feedback("call identity rejected"));
        }
        let mut fact = ExecutionFact::pending(
            identity.turn_key,
            identity.tool_call_id,
            identity.function_name,
            args,
        );
        fact.phase = ToolPhase::Started;
        self.recorder.record_started(&fact).await.map_err(|_| {
            ToolExecutionError::other("started write was not acknowledged")
                .with_model_feedback("tool was not started")
        })?;
        Ok(fact)
    }

    pub async fn finish_ok(
        &self,
        mut fact: ExecutionFact,
        presentation: String,
    ) -> Result<String, ToolExecutionError> {
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(ToolOutcome::Success);
        fact.executed = Some(true);
        fact.model_presentation = Some(presentation.clone());
        if self.recorder.record_terminal(&fact).await.is_err() {
            return Err(
                ToolExecutionError::other("terminal write was not acknowledged")
                    .with_model_feedback("tool effect may have occurred; not confirmed"),
            );
        }
        Ok(presentation)
    }

    pub async fn finish_err(
        &self,
        mut fact: ExecutionFact,
        err: ToolExecutionError,
    ) -> ToolExecutionError {
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(match err.kind() {
            ToolErrorKind::Timeout => ToolOutcome::Timeout,
            ToolErrorKind::Cancelled => ToolOutcome::Cancelled,
            _ => ToolOutcome::Error,
        });
        fact.executed = Some(true);
        fact.reason = Some(err.message().to_string());
        fact.model_presentation = Some(
            err.model_feedback()
                .unwrap_or_else(|| err.message())
                .to_string(),
        );
        let _ = self.recorder.record_terminal(&fact).await;
        err
    }
}

/// ACP permission kind for a native tool name.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "skill" | "recall" => "read",
        "write_file" | "edit_file" | "write_plan" | "write_explore_report" => "edit",
        "glob" | "grep" | "codegraph" | "lsp" => "search",
        "bash" => "execute",
        "enter_plan_mode" | "exit_plan_mode" => "think",
        _ => "other",
    }
}

/// Read-only native tools skip the permission card. MCP uses
/// [`mcp_tool_requires_permission`] (`readOnlyHint == true` only).
pub fn tool_requires_permission(name: &str) -> bool {
    !matches!(
        name,
        "read_file"
            | "glob"
            | "grep"
            | "codegraph"
            | "lsp"
            | "skill"
            | "update_plan"
            | "recall"
            | "write_plan"
            | "write_explore_report"
            | "exit_plan_mode"
    ) && !is_companion_tool(name)
}

/// Host tool cards: only a real success is `completed`. Skip/deny/cancel/error
/// are `failed` so the UI never treats a refusal as done.
pub fn acp_card_status_from_rig(status_name: &str) -> &'static str {
    if status_name == "success" {
        "completed"
    } else {
        "failed"
    }
}

/// `subagent` returns Success on start; the ACP card stays `in_progress`
/// until the supervisor inject completes or fails it.
pub fn acp_card_status_for_tool(tool_name: &str, status_name: &str) -> &'static str {
    if tool_name == "subagent" && status_name == "success" {
        "in_progress"
    } else {
        acp_card_status_from_rig(status_name)
    }
}

pub fn map_fs_error(err: FileSystemRuntimeError) -> ToolExecutionError {
    match err {
        FileSystemRuntimeError::InvalidParams(message) => {
            let lower = message.to_ascii_lowercase();
            let err = if lower.contains("no such file")
                || lower.contains("not found")
                || lower.contains("failed to read")
                || lower.contains("failed to access")
            {
                ToolExecutionError::not_found(message.clone())
            } else if lower.contains("outside the allowed") {
                ToolExecutionError::permission_denied(message.clone())
            } else {
                ToolExecutionError::invalid_args(message.clone())
            };
            err.with_model_feedback(message)
        }
        FileSystemRuntimeError::Internal(message) => {
            ToolExecutionError::other(message).with_model_feedback("filesystem operation failed")
        }
    }
}

pub fn schema_for<T: Tool>(tool: &T) -> Value {
    json!({
        "name": T::NAME,
        "description": tool.description(),
        "parameters": tool.parameters(),
    })
}

#[cfg(test)]
pub(crate) fn test_tool_ctx(
    dir: &std::path::Path,
    function_name: &str,
    call_id: &str,
) -> NativeToolCtx {
    use crate::agent::context::{CallIdentity, ContextStore};

    let store = Arc::new(Mutex::new(ContextStore::new("s")));
    let recorder = Arc::new(FactRecorder::memory(Arc::clone(&store)));
    let identity = Arc::new(CallIdentityBridge::new());
    identity.set(CallIdentity {
        turn_id: 1,
        turn_key: "s:1".into(),
        tool_call_id: call_id.into(),
        function_name: function_name.into(),
    });
    NativeToolCtx {
        turn_id: 1,
        identity,
        recorder,
        cancel: CancellationToken::new(),
        launch_cwd: dir.to_path_buf(),
        fs: Arc::new(FileSystemRuntime::with_policy(
            crate::acp::file_system_runtime::FsAccessPolicy::strict(dir),
        )),
        session_id: "s".into(),
        spill_dir: dir.join("spills"),
    }
}

/// In-memory echo tool used by PR2 contract tests. Counts every body run.
#[derive(Clone, Default)]
pub struct EchoTool {
    calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Deserialize)]
pub struct EchoArgs {
    pub text: String,
}

impl EchoTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("echo calls").clone()
    }
}

impl Tool for EchoTool {
    const NAME: &'static str = "echo";
    type Args = EchoArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Echo text back".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to echo" }
            },
            "required": ["text"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.calls
            .lock()
            .expect("echo calls")
            .push(args.text.clone());
        Ok(args.text)
    }
}

/// Controllable failure: explicit model feedback, not Display of a raw error.
#[derive(Clone, Default)]
pub struct FailTool {
    calls: Arc<Mutex<usize>>,
}

impl FailTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn call_count(&self) -> usize {
        *self.calls.lock().expect("fail calls")
    }
}

#[derive(Deserialize)]
pub struct FailArgs {}

impl Tool for FailTool {
    const NAME: &'static str = "fail_tool";
    type Args = FailArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Always fails with a file-not-found model message".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        *self.calls.lock().expect("fail calls") += 1;
        Err(ToolExecutionError::not_found("missing file demo.txt")
            .with_model_feedback("file does not exist: demo.txt"))
    }
}

/// Controllable in-memory write used to prove started-ack-before-side-effect.
/// Real file/bash tools land in PR5.
#[derive(Clone)]
pub struct SideEffectTool {
    effects: Arc<Mutex<Vec<String>>>,
    identity: Arc<CallIdentityBridge>,
    recorder: Arc<FactRecorder>,
    turn_id: u64,
}

impl SideEffectTool {
    pub fn new(
        identity: Arc<CallIdentityBridge>,
        recorder: Arc<FactRecorder>,
        turn_id: u64,
    ) -> Self {
        Self {
            effects: Arc::new(Mutex::new(Vec::new())),
            identity,
            recorder,
            turn_id,
        }
    }

    pub fn effects(&self) -> Vec<String> {
        self.effects.lock().expect("effects").clone()
    }
}

#[derive(Deserialize)]
pub struct SideEffectArgs {
    pub text: String,
}

impl Tool for SideEffectTool {
    const NAME: &'static str = "write_mem";
    type Args = SideEffectArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Record a side effect after a started write ack".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" }
            },
            "required": ["text"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let identity = self.identity.require(self.turn_id).map_err(|err| {
            ToolExecutionError::invalid_args(err).with_model_feedback("call identity rejected")
        })?;
        let mut fact = ExecutionFact::pending(
            identity.turn_key.clone(),
            identity.tool_call_id.clone(),
            identity.function_name.clone(),
            json!({ "text": args.text }),
        );
        fact.phase = ToolPhase::Started;
        self.recorder.record_started(&fact).await.map_err(|_| {
            ToolExecutionError::other("started write was not acknowledged")
                .with_model_feedback("tool was not started")
        })?;
        self.effects
            .lock()
            .expect("effects")
            .push(args.text.clone());
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(ToolOutcome::Success);
        fact.executed = Some(true);
        fact.model_presentation = Some(format!("wrote {}", args.text));
        if self.recorder.record_terminal(&fact).await.is_err() {
            return Err(
                ToolExecutionError::other("terminal write was not acknowledged")
                    .with_model_feedback("tool effect may have occurred; not confirmed"),
            );
        }
        Ok(format!("wrote {}", args.text))
    }
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolExecutionError;

    #[derive(Debug, thiserror::Error)]
    #[error("secret-operator-diagnostic")]
    struct SecretError;

    #[test]
    fn from_error_does_not_leak_display_to_the_model() {
        let err = ToolExecutionError::from_error(SecretError);
        assert_eq!(err.message(), "secret-operator-diagnostic");
        let feedback = err.model_feedback().unwrap_or_default();
        assert!(
            !feedback.contains("secret-operator-diagnostic"),
            "from_error must redact Display; got {feedback:?}"
        );
        assert_eq!(feedback, "the tool failed");
    }

    #[test]
    fn explicit_constructor_keeps_model_feedback() {
        let err = ToolExecutionError::not_found("missing file demo.txt")
            .with_model_feedback("file does not exist: demo.txt");
        assert_eq!(err.model_feedback(), Some("file does not exist: demo.txt"));
    }

    #[test]
    fn subagent_success_card_stays_in_progress() {
        assert_eq!(
            super::acp_card_status_for_tool("subagent", "success"),
            "in_progress"
        );
        assert_eq!(
            super::acp_card_status_for_tool("subagent", "error"),
            "failed"
        );
        assert_eq!(
            super::acp_card_status_for_tool("read_file", "success"),
            "completed"
        );
    }

    #[tokio::test]
    async fn started_ack_failure_does_not_run_the_side_effect() {
        use crate::agent::context::store::{
            CallIdentity, CallIdentityBridge, ContextStore, FactRecorder,
        };
        use rig::tool::{Tool, ToolContext};

        let store = std::sync::Arc::new(std::sync::Mutex::new(ContextStore::new("s")));
        let recorder = std::sync::Arc::new(FactRecorder::memory(std::sync::Arc::clone(&store)));
        recorder.fail_next_started();
        let identity = std::sync::Arc::new(CallIdentityBridge::new());
        identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_x".into(),
            function_name: "write_mem".into(),
        });
        let tool = super::SideEffectTool::new(identity, recorder, 1);
        let mut ctx = ToolContext::new();
        let err = tool
            .call(
                &mut ctx,
                super::SideEffectArgs {
                    text: "nope".into(),
                },
            )
            .await
            .expect_err("started ack must fail closed");
        assert!(
            tool.effects().is_empty(),
            "no side effect: {:?}",
            tool.effects()
        );
        assert!(store.lock().unwrap().fact("call_x").is_none());
        let _ = err;
    }
}

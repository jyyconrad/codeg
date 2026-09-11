//! Shell tool wrapping `TerminalRuntime::create_shell_terminal`.

use std::sync::Arc;
use std::time::Duration;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use sacp::schema::{
    KillTerminalRequest, ReleaseTerminalRequest, SessionId, TerminalOutputRequest,
    WaitForTerminalExitRequest,
};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::acp::terminal_runtime::{
    CreateShellTerminalRequest, TerminalRuntime, TerminalRuntimeError,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TIMEOUT: Duration = Duration::from_secs(300);
const CAPTURE_LIMIT: u64 = 1_000_000;

#[derive(Clone)]
pub struct BashTool {
    ctx: NativeToolCtx,
    terminals: Arc<TerminalRuntime>,
}

impl BashTool {
    pub fn new(ctx: NativeToolCtx, terminals: Arc<TerminalRuntime>) -> Self {
        Self { ctx, terminals }
    }
}

#[derive(Debug, Deserialize)]
pub struct BashArgs {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Tool for BashTool {
    const NAME: &'static str = "bash";
    type Args = BashArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Run a shell script through the user's configured shell. The whole command \
         is executed as a script (operators, redirects, and builtins work). Output \
         is captured once; it cannot be reread after the command finishes."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell script to run" },
                "cwd": { "type": "string", "description": "Absolute or cwd-relative working directory" },
                "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds (default 120000, max 300000)" }
            },
            "required": ["command"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "command": args.command,
            "cwd": args.cwd,
            "timeout_ms": args.timeout_ms,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match self.run_script(args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

struct ShellCapture {
    output: String,
    truncated: bool,
    captured_bytes: usize,
    exit_code: Option<u32>,
    signal: Option<String>,
    timed_out: bool,
    cancelled: bool,
}

impl BashTool {
    async fn run_script(&self, args: BashArgs) -> Result<String, ToolExecutionError> {
        if args.command.trim().is_empty() {
            return Err(
                ToolExecutionError::invalid_args("command must not be empty")
                    .with_model_feedback("command must not be empty"),
            );
        }
        let timeout = parse_timeout(args.timeout_ms)?;
        let cwd = match args.cwd.as_deref() {
            Some(cwd) => self.ctx.resolve_path(cwd)?,
            None => self.ctx.launch_cwd.clone(),
        };
        if !cwd.is_absolute() {
            return Err(ToolExecutionError::invalid_args("cwd must be absolute")
                .with_model_feedback("cwd must be an absolute path"));
        }

        let request = CreateShellTerminalRequest::new(&self.ctx.session_id, args.command)
            .output_byte_limit(CAPTURE_LIMIT)
            .cwd(cwd);

        let created = self
            .terminals
            .create_shell_terminal(request)
            .await
            .map_err(map_term_error)?;
        let terminal_id = created.terminal_id.clone();
        let session = SessionId::new(self.ctx.session_id.clone());
        let capture = self
            .wait_capture(session.clone(), terminal_id.clone(), timeout)
            .await;
        let _ = self
            .terminals
            .release_terminal(ReleaseTerminalRequest::new(session, terminal_id))
            .await;
        let capture = capture?;
        render_capture(capture)
    }

    async fn wait_capture(
        &self,
        session: SessionId,
        terminal_id: sacp::schema::TerminalId,
        timeout: Duration,
    ) -> Result<ShellCapture, ToolExecutionError> {
        let wait = self
            .terminals
            .wait_for_terminal_exit(WaitForTerminalExitRequest::new(
                session.clone(),
                terminal_id.clone(),
            ));
        let mut timed_out = false;
        let mut cancelled = false;
        tokio::select! {
            _ = self.ctx.cancel.cancelled() => {
                cancelled = true;
                let _ = self.terminals.kill_terminal(KillTerminalRequest::new(
                    session.clone(),
                    terminal_id.clone(),
                )).await;
            }
            _ = tokio::time::sleep(timeout) => {
                timed_out = true;
                let _ = self.terminals.kill_terminal(KillTerminalRequest::new(
                    session.clone(),
                    terminal_id.clone(),
                )).await;
            }
            result = wait => {
                result.map_err(map_term_error)?;
            }
        }

        let out = self
            .terminals
            .terminal_output(TerminalOutputRequest::new(session, terminal_id))
            .await
            .map_err(map_term_error)?;
        Ok(ShellCapture {
            captured_bytes: out.output.len(),
            truncated: out.truncated,
            output: out.output,
            exit_code: out.exit_status.as_ref().and_then(|s| s.exit_code),
            signal: out.exit_status.and_then(|s| s.signal),
            timed_out,
            cancelled,
        })
    }
}

fn parse_timeout(timeout_ms: Option<u64>) -> Result<Duration, ToolExecutionError> {
    match timeout_ms {
        None => Ok(DEFAULT_TIMEOUT),
        Some(0) => Err(ToolExecutionError::invalid_args("timeout_ms must be > 0")
            .with_model_feedback("timeout_ms must be greater than 0")),
        Some(ms) => Ok(Duration::from_millis(ms).min(MAX_TIMEOUT)),
    }
}

fn render_capture(capture: ShellCapture) -> Result<String, ToolExecutionError> {
    let mut header = format!(
        "exit_code: {}\ntruncated: {}\ncaptured_bytes: {}\n",
        capture
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| capture.signal.clone().unwrap_or_else(|| "unknown".into())),
        capture.truncated,
        capture.captured_bytes
    );
    if capture.truncated {
        header.push_str(
            "remaining output unavailable; the capture was released and will not be re-run\n",
        );
    }
    header.push('\n');
    header.push_str(&capture.output);
    if capture.cancelled {
        return Err(ToolExecutionError::cancelled("command cancelled")
            .with_retryable(false)
            .with_model_feedback(format!("command cancelled\n{header}")));
    }
    if capture.timed_out {
        return Err(ToolExecutionError::timeout("command timed out")
            .with_retryable(false)
            .with_model_feedback(format!(
                "timed out (captured {} bytes, truncated={})\n{}",
                capture.captured_bytes, capture.truncated, capture.output
            )));
    }
    Ok(header)
}

fn map_term_error(err: TerminalRuntimeError) -> ToolExecutionError {
    match err {
        TerminalRuntimeError::InvalidParams(message) => {
            ToolExecutionError::invalid_args(message.clone()).with_model_feedback(message)
        }
        TerminalRuntimeError::Internal(message) => {
            ToolExecutionError::other(message).with_model_feedback("shell command failed to run")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::CallIdentity;
    use crate::agent::tools::test_tool_ctx;
    use rig::tool::Tool;
    use std::collections::BTreeMap;
    use std::fs;

    fn runtime() -> Arc<TerminalRuntime> {
        Arc::new(TerminalRuntime::with_base_env(BTreeMap::new()))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bash_runs_script_semantics_and_drops_cache() {
        let dir = tempfile::tempdir().expect("dir");
        let ctx = test_tool_ctx(dir.path(), "bash", "call_b");
        let terminals = runtime();
        let tool = BashTool::new(ctx.clone(), Arc::clone(&terminals));
        let mut tctx = ToolContext::new();

        let out = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: "pwd; pwd".into(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                },
            )
            .await
            .expect("pwd");
        let cwd = dir.path().canonicalize().unwrap();
        assert!(
            out.matches(cwd.to_string_lossy().as_ref()).count() >= 2,
            "{out}"
        );

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_b2".into(),
            function_name: "bash".into(),
        });
        let out = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: "true && false".into(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                },
            )
            .await
            .expect("true && false is a completed script, not a tool crash");
        assert!(out.contains("exit_code: 1"), "{out}");

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_b3".into(),
            function_name: "bash".into(),
        });
        let marker = dir.path().join("mark.txt");
        let out = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: format!(
                        "echo once >> '{}' && cat '{}'",
                        marker.display(),
                        marker.display()
                    ),
                    cwd: None,
                    timeout_ms: Some(5_000),
                },
            )
            .await
            .expect("redirect");
        assert!(out.contains("once"), "{out}");
        assert_eq!(
            fs::read_to_string(&marker).unwrap().matches("once").count(),
            1
        );

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_b4".into(),
            function_name: "bash".into(),
        });
        let out = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: ":".into(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                },
            )
            .await
            .expect("bare builtin");
        assert!(out.contains("exit_code: 0"), "{out}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bash_timeout_returns_output_and_does_not_rerun() {
        let dir = tempfile::tempdir().expect("dir");
        let marker = dir.path().join("count.txt");
        let ctx = test_tool_ctx(dir.path(), "bash", "call_t");
        let tool = BashTool::new(ctx, runtime());
        let mut tctx = ToolContext::new();
        let err = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: format!("echo ran >> '{}'; sleep 5", marker.display()),
                    cwd: None,
                    timeout_ms: Some(200),
                },
            )
            .await
            .expect_err("timeout");
        assert_eq!(err.kind(), rig::tool::ToolErrorKind::Timeout);
        assert_eq!(
            fs::read_to_string(&marker).unwrap().matches("ran").count(),
            1
        );
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("timed out"),
            "{err:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bash_uses_selected_user_shell() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("dir");
        let shell = dir.path().join("user-shell");
        fs::write(
            &shell,
            "#!/bin/sh\nprintf '%s\\n' user-shell\nexec /bin/sh \"$@\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&shell).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shell, perms).unwrap();

        let config = crate::acp::terminal_runtime::TerminalShellRuntimeConfig::new();
        config.set(Some(shell.to_string_lossy().to_string())).await;
        let terminals = Arc::new(
            TerminalRuntime::with_base_env(BTreeMap::new()).with_default_shell_config(config),
        );
        let ctx = test_tool_ctx(dir.path(), "bash", "call_s");
        let tool = BashTool::new(ctx, terminals);
        let mut tctx = ToolContext::new();
        let out = tool
            .call(
                &mut tctx,
                BashArgs {
                    command: "printf 'script\\n'".into(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                },
            )
            .await
            .expect("user shell");
        assert!(out.contains("user-shell"), "{out}");
        assert!(out.contains("script"), "{out}");
    }
}

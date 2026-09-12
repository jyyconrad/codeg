use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::acp::process_owner::{
    force_kill_and_reap, kill_tree_signal, lock_owners, pid_is_alive, ProcessOwnerRegistry,
};

use super::{CodeIntelConfig, CodegraphSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostIndexAction {
    Skip,
    Sync,
    Init,
}

pub fn should_run_host_index(
    cfg: &CodeIntelConfig,
    binary: Option<&Path>,
    cwd: &Path,
) -> HostIndexAction {
    if !cfg.enabled || !cfg.codegraph.enabled || binary.is_none() {
        return HostIndexAction::Skip;
    }
    if codegraph_has_index(cwd) {
        HostIndexAction::Sync
    } else {
        HostIndexAction::Init
    }
}

pub async fn spawn_host_index(
    binary: PathBuf,
    cwd: PathBuf,
    action: HostIndexAction,
    owners: Arc<Mutex<ProcessOwnerRegistry>>,
    cancel: CancellationToken,
) {
    let argv = match action {
        HostIndexAction::Skip => return,
        HostIndexAction::Sync => vec!["sync".to_string()],
        HostIndexAction::Init => vec!["init".to_string()],
    };
    tracing::info!(
        action = ?action,
        cwd = %cwd.display(),
        "codegraph host index starting"
    );
    match spawn_codegraph(
        &binary,
        &cwd,
        &argv,
        Some(owners),
        cancel,
        CODEGRAPH_INIT_TIMEOUT,
    )
    .await
    {
        Ok(run) if run.status == 0 => {
            tracing::info!(status = run.status, "codegraph host index finished");
        }
        Ok(run) => {
            tracing::warn!(
                status = run.status,
                stderr = %run.stderr,
                "codegraph host index exited nonzero"
            );
        }
        Err(err) => {
            tracing::warn!(error = %err, "codegraph host index failed");
        }
    }
}

pub const CODEGRAPH_QUERY_TIMEOUT: Duration = Duration::from_secs(120);
pub const CODEGRAPH_INIT_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegraphOp {
    Explore,
    Query,
    Node,
    Callers,
    Callees,
    Impact,
    Files,
    Status,
}

impl CodegraphOp {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "explore" => Some(Self::Explore),
            "query" => Some(Self::Query),
            "node" => Some(Self::Node),
            "callers" => Some(Self::Callers),
            "callees" => Some(Self::Callees),
            "impact" => Some(Self::Impact),
            "files" => Some(Self::Files),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    pub fn as_cli(self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::Query => "query",
            Self::Node => "node",
            Self::Callers => "callers",
            Self::Callees => "callees",
            Self::Impact => "impact",
            Self::Files => "files",
            Self::Status => "status",
        }
    }

    pub fn uses_json_flag(self) -> bool {
        !matches!(self, Self::Explore | Self::Node)
    }

    fn requires_query(self) -> bool {
        matches!(
            self,
            Self::Explore | Self::Query | Self::Node | Self::Callers | Self::Callees | Self::Impact
        )
    }
}

pub fn resolve_codegraph_binary(settings: &CodegraphSettings) -> Option<PathBuf> {
    if let Some(raw) = settings.binary_path.as_deref() {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_absolute() && path.is_file() {
                return Some(path);
            }
        }
    }
    which::which("codegraph").ok()
}

pub fn codegraph_has_index(cwd: &Path) -> bool {
    cwd.join(".codegraph").is_dir()
}

pub fn build_codegraph_argv(
    op: CodegraphOp,
    query: Option<&str>,
    path: Option<&str>,
    kind: Option<&str>,
    limit: Option<u32>,
    depth: Option<u32>,
) -> Result<Vec<String>, String> {
    let mut argv = vec![op.as_cli().to_string()];

    if op.requires_query() {
        let q = query
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("codegraph {} requires a non-empty query", op.as_cli()))?;
        argv.push(q.to_string());
    }

    match op {
        CodegraphOp::Query => {
            if let Some(kind) = kind.map(str::trim).filter(|s| !s.is_empty()) {
                argv.push("--kind".into());
                argv.push(kind.to_string());
            }
            if let Some(limit) = limit {
                argv.push("--limit".into());
                argv.push(limit.to_string());
            }
        }
        CodegraphOp::Impact => {
            if let Some(depth) = depth {
                argv.push("--depth".into());
                argv.push(depth.to_string());
            }
        }
        CodegraphOp::Files => {
            if let Some(path) = path.map(str::trim).filter(|s| !s.is_empty()) {
                argv.push(path.to_string());
            }
        }
        _ => {}
    }

    if op.uses_json_flag() {
        argv.push("--json".into());
    }

    Ok(argv)
}

pub struct CodegraphRun {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub async fn spawn_codegraph(
    binary: &Path,
    cwd: &Path,
    argv: &[String],
    owners: Option<Arc<Mutex<ProcessOwnerRegistry>>>,
    cancel: CancellationToken,
    timeout: Duration,
) -> Result<CodegraphRun, String> {
    use tokio::io::AsyncReadExt;

    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(argv)
        .current_dir(cwd)
        .env("CODEGRAPH_TELEMETRY", "0")
        .env("DO_NOT_TRACK", "1")
        .env("CODEGRAPH_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("failed to spawn codegraph: {err}"))?;
    let pid = child.id().unwrap_or(0);

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "codegraph stdout pipe missing".to_string())?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "codegraph stderr pipe missing".to_string())?;

    if let Some(owners) = owners.as_ref() {
        lock_owners(owners).register(pid);
    }

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stdout_pipe
            .read_to_end(&mut buf)
            .await
            .map(|_| buf)
            .map_err(|err| err.to_string())
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stderr_pipe
            .read_to_end(&mut buf)
            .await
            .map(|_| buf)
            .map_err(|err| err.to_string())
    });

    enum WaitOutcome {
        Exited(std::io::Result<std::process::ExitStatus>),
        Cancelled,
        TimedOut,
    }

    let wait_outcome = tokio::select! {
        status = child.wait() => WaitOutcome::Exited(status),
        _ = cancel.cancelled() => {
            terminate_codegraph(pid).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            WaitOutcome::Cancelled
        }
        _ = tokio::time::sleep(timeout) => {
            terminate_codegraph(pid).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            WaitOutcome::TimedOut
        }
    };

    let stdout_buf = stdout_task
        .await
        .unwrap_or_else(|err| Err(format!("codegraph stdout task failed: {err}")));
    let stderr_buf = stderr_task
        .await
        .unwrap_or_else(|err| Err(format!("codegraph stderr task failed: {err}")));

    if let Some(owners) = owners.as_ref() {
        lock_owners(owners).unregister(pid);
    }

    let stdout = match stdout_buf {
        Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
        Err(err) => return Err(format!("codegraph stdout read failed: {err}")),
    };
    let stderr = match stderr_buf {
        Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
        Err(err) => return Err(format!("codegraph stderr read failed: {err}")),
    };

    match wait_outcome {
        WaitOutcome::Exited(Ok(status)) => Ok(CodegraphRun {
            status: status.code().unwrap_or(-1),
            stdout,
            stderr,
        }),
        WaitOutcome::Exited(Err(err)) => Err(format!("codegraph wait failed: {err}")),
        WaitOutcome::Cancelled => Err("codegraph cancelled".into()),
        WaitOutcome::TimedOut => Err("codegraph timed out".into()),
    }
}

async fn terminate_codegraph(pid: u32) {
    if pid == 0 {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        kill_tree_signal(pid, "SIGTERM");
    })
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while pid_is_alive(pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if pid_is_alive(pid) {
        let _ = force_kill_and_reap(&[pid], Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::code_intel::default_config;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn argv_explore_is_plain_text() {
        let argv = build_codegraph_argv(CodegraphOp::Explore, Some("auth"), None, None, None, None)
            .unwrap();
        assert_eq!(argv, vec!["explore", "auth"]);
    }

    #[test]
    fn argv_query_adds_json_kind_limit() {
        let argv = build_codegraph_argv(
            CodegraphOp::Query,
            Some("Foo"),
            None,
            Some("function"),
            Some(5),
            None,
        )
        .unwrap();
        assert_eq!(
            argv,
            vec!["query", "Foo", "--kind", "function", "--limit", "5", "--json"]
        );
    }

    #[test]
    fn argv_status_json_no_query() {
        let argv = build_codegraph_argv(CodegraphOp::Status, None, None, None, None, None).unwrap();
        assert_eq!(argv, vec!["status", "--json"]);
    }

    #[test]
    fn argv_explore_rejects_empty_query() {
        assert!(
            build_codegraph_argv(CodegraphOp::Explore, Some("  "), None, None, None, None).is_err()
        );
    }

    #[test]
    fn parse_rejects_install() {
        assert!(CodegraphOp::parse("install").is_none());
        assert!(CodegraphOp::parse("serve").is_none());
        assert_eq!(CodegraphOp::parse("explore"), Some(CodegraphOp::Explore));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_sets_telemetry_env_and_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-codegraph");
        {
            std::fs::write(
                &script,
                "#!/bin/sh\necho TELEMETRY=$CODEGRAPH_TELEMETRY\necho DNT=$DO_NOT_TRACK\necho NOUP=$CODEGRAPH_NO_UPDATE_CHECK\necho argv:$@\n",
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let run = spawn_codegraph(
            &script,
            dir.path(),
            &["explore".into(), "x".into()],
            None,
            CancellationToken::new(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(run.status, 0);
        assert!(run.stdout.contains("TELEMETRY=0"));
        assert!(run.stdout.contains("DNT=1"));
        assert!(run.stdout.contains("NOUP=1"));
        assert!(run.stdout.contains("argv:explore x"));
    }

    #[test]
    fn missing_index_is_false() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!codegraph_has_index(dir.path()));
        std::fs::create_dir(dir.path().join(".codegraph")).unwrap();
        assert!(codegraph_has_index(dir.path()));
    }

    #[test]
    fn resolve_prefers_absolute_binary_path_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codegraph");
        std::fs::write(&bin, "").unwrap();
        let settings = CodegraphSettings {
            enabled: true,
            binary_path: Some(bin.to_string_lossy().into()),
        };
        assert_eq!(resolve_codegraph_binary(&settings), Some(bin));
    }

    #[test]
    fn host_index_skips_when_master_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = default_config();
        cfg.enabled = false;
        assert_eq!(
            should_run_host_index(&cfg, Some(Path::new("/bin/true")), dir.path()),
            HostIndexAction::Skip
        );
    }

    #[test]
    fn host_index_inits_when_enabled_without_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = default_config();
        cfg.enabled = true;
        assert_eq!(
            should_run_host_index(&cfg, Some(Path::new("/bin/true")), dir.path()),
            HostIndexAction::Init
        );
    }

    #[test]
    fn host_index_syncs_when_index_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".codegraph")).unwrap();
        let mut cfg = default_config();
        cfg.enabled = true;
        assert_eq!(
            should_run_host_index(&cfg, Some(Path::new("/bin/true")), dir.path()),
            HostIndexAction::Sync
        );
    }

    #[test]
    fn host_index_skips_without_binary() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = default_config();
        cfg.enabled = true;
        assert_eq!(
            should_run_host_index(&cfg, None, dir.path()),
            HostIndexAction::Skip
        );
    }

    #[test]
    fn host_index_skips_when_codegraph_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.codegraph.enabled = false;
        assert_eq!(
            should_run_host_index(&cfg, Some(Path::new("/bin/true")), dir.path()),
            HostIndexAction::Skip
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_host_index_skip_does_not_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-codegraph");
        let marker = dir.path().join("ran");
        {
            std::fs::write(
                &script,
                format!("#!/bin/sh\nprintf ran > {}\n", marker.display()),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        spawn_host_index(
            script,
            dir.path().to_path_buf(),
            HostIndexAction::Skip,
            Arc::new(Mutex::new(ProcessOwnerRegistry::new())),
            CancellationToken::new(),
        )
        .await;
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_host_index_init_uses_init_argv() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-codegraph");
        let marker = dir.path().join("argv.txt");
        {
            std::fs::write(
                &script,
                format!("#!/bin/sh\nprintf '%s' \"$*\" > {}\n", marker.display()),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        spawn_host_index(
            script,
            dir.path().to_path_buf(),
            HostIndexAction::Init,
            Arc::new(Mutex::new(ProcessOwnerRegistry::new())),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "init");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_host_index_sync_uses_sync_argv() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-codegraph");
        let marker = dir.path().join("argv.txt");
        {
            std::fs::write(
                &script,
                format!("#!/bin/sh\nprintf '%s' \"$*\" > {}\n", marker.display()),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        spawn_host_index(
            script,
            dir.path().to_path_buf(),
            HostIndexAction::Sync,
            Arc::new(Mutex::new(ProcessOwnerRegistry::new())),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "sync");
    }
}

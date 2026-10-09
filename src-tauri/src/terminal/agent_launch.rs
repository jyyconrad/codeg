use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::agent_hooks::{hook_environment, AgentHookRegistry, HookSupport};
use super::error::TerminalError;
use super::manager::{resolve_shell, SpawnOptions, TerminalManager};
use super::shell_flavor::{classify_shell_family, ShellFamily};
use crate::models::agent::AgentType;
use crate::web::event_bridge::EventEmitter;

#[derive(Debug, Serialize)]
pub struct SpawnAgentResponse {
    pub terminal_id: String,
    pub run_id: String,
    pub hook_support: HookSupport,
}

fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn quote_powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn shell_command(shell: &str, program: &Path, args: &[String]) -> Result<String, TerminalError> {
    let words = std::iter::once(program.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    match classify_shell_family(shell) {
        ShellFamily::Posix => Ok(words
            .iter()
            .map(|word| quote_posix(word))
            .collect::<Vec<_>>()
            .join(" ")),
        ShellFamily::PowerShell => Ok(format!(
            "& {}",
            words
                .iter()
                .map(|word| quote_powershell(word))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        ShellFamily::Cmd => Err(TerminalError::SpawnFailed(
            "agent terminal requires a POSIX shell or PowerShell".to_string(),
        )),
    }
}

fn hook_command(helper_binary: &Path, action: &str) -> String {
    format!(
        "{} --agent-hook {}",
        quote_posix(&helper_binary.to_string_lossy()),
        action
    )
}

fn codex_override(event: &str, command: &str) -> String {
    let command_value = toml::Value::String(command.to_string()).to_string();
    format!("hooks.{event}=[{{hooks=[{{type=\"command\",command={command_value}}}]}}]")
}

fn observation_hooks(helper_binary: &Path) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": hook_command(helper_binary, "turn-start")}]}],
            "SessionStart": [{"hooks": [{"type": "command", "command": hook_command(helper_binary, "session-start")}]}],
            "Stop": [{"hooks": [{"type": "command", "command": hook_command(helper_binary, "turn-stop") }]}],
            "SessionEnd": [{"hooks": [{"type": "command", "command": hook_command(helper_binary, "session-end")}]}]
        }
    })
}

/// Grok's `Stop` fires only for a turn that completed. An API error fires
/// `StopFailure`, and an interrupt fires `StopCancelled`. Leaving those
/// unwired kept the sidebar spinner up after the CLI was already back at
/// its prompt, and the later process exit then marked the row cancelled.
fn grok_observation_hooks(helper_binary: &Path) -> serde_json::Value {
    let mut hooks = observation_hooks(helper_binary);
    let stop = hooks["hooks"]["Stop"].clone();
    hooks["hooks"]["StopFailure"] = stop.clone();
    hooks["hooks"]["StopCancelled"] = stop;
    hooks
}

fn claude_settings_file(helper_binary: &Path) -> Result<PathBuf, TerminalError> {
    let mut file = tempfile::Builder::new()
        .prefix("codeg-agent-hooks-")
        .suffix(".json")
        .tempfile()
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    let settings = observation_hooks(helper_binary);
    serde_json::to_writer(&mut file, &settings)
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    file.flush()
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    let (_handle, path) = file
        .keep()
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    Ok(path)
}

async fn native_binary(agent_type: AgentType) -> Option<PathBuf> {
    match agent_type {
        AgentType::ClaudeCode | AgentType::Codex => {
            let relation = crate::acp::registry::acp_adapter_relation(agent_type)?;
            crate::commands::acp::resolve_vendor_cli(relation.native_cmd, relation.extra_dirs).await
        }
        AgentType::Grok => crate::commands::acp::resolve_npx_command("grok").await,
        _ => None,
    }
}

async fn verified_hook_support(agent_type: AgentType, binary: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    match agent_type {
        AgentType::Codex => probe_codex_hooks(binary).await,
        AgentType::ClaudeCode => probe_claude_hooks(binary).await,
        AgentType::Grok => probe_grok_hooks(binary).await,
        _ => false,
    }
}

#[derive(Hash, PartialEq, Eq)]
struct BinaryFingerprint {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

static CODEX_HOOK_PROBES: OnceLock<Mutex<HashSet<BinaryFingerprint>>> = OnceLock::new();
/// `Some` means `--help` answered. A timeout or a binary that would not start
/// stays uncached so the next launch can try again.
static CODEX_HOOK_TRUST_BYPASS: OnceLock<Mutex<HashMap<BinaryFingerprint, bool>>> = OnceLock::new();

fn binary_fingerprint(binary: &Path) -> Option<BinaryFingerprint> {
    let metadata = std::fs::metadata(binary).ok()?;
    Some(BinaryFingerprint {
        path: binary.to_path_buf(),
        len: metadata.len(),
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    })
}

/// Run a vendor CLI from the desktop process.
///
/// npm's `codex` is `#!/usr/bin/env node`. A GUI app's PATH is only
/// `/usr/bin:/bin:/usr/sbin:/sbin`, so the probe exits 127 and hook support
/// stays off. The login shell that later runs the command does have node, and
/// Codex then starts with no observation hooks — the tab keeps its draft name.
/// The shim's own directory contains the sibling `node` that `env` needs.
fn agent_cli_command(binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    if let Some(path) = path_with_binary_dir(binary, std::env::var_os("PATH")) {
        command.env("PATH", path);
    }
    command
}

fn path_with_binary_dir(
    binary: &Path,
    current: Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    let dir = binary.parent().filter(|dir| !dir.as_os_str().is_empty())?;
    let mut dirs = vec![dir.to_path_buf()];
    if let Some(current) = current {
        dirs.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(dirs).ok()
}

async fn probe_codex_hooks(binary: &Path) -> bool {
    let fingerprint = binary_fingerprint(binary);
    let cache = CODEX_HOOK_PROBES.get_or_init(|| Mutex::new(HashSet::new()));
    if fingerprint.as_ref().is_some_and(|fingerprint| {
        cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(fingerprint)
    }) {
        return true;
    }
    // `--help` returns immediately and the trust-bypass flag only exists on
    // builds that already have UserPromptSubmit and Stop. `codex doctor`
    // loads config plus the app-server and takes ~15s here; the old 5s budget
    // always expired, so hook support stayed off and the tab kept its draft name.
    if codex_supports_hook_trust_bypass(binary).await {
        if let Some(fingerprint) = fingerprint {
            cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(fingerprint);
        }
        return true;
    }
    let mut command = agent_cli_command(binary);
    command.args(["--enable", "hooks"]);
    // SessionStart and SessionEnd are not probed. An event this Codex build
    // does not know disables every hook, including the turn events below.
    for event in ["UserPromptSubmit", "Stop"] {
        command.arg("-c").arg(codex_override(event, "true"));
    }
    command.args(["doctor", "--json"]);
    command.kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(30), command.output()).await
    else {
        return false;
    };
    // Doctor can exit nonzero for unrelated health checks. Its config.load
    // check is the structured answer about these exact hook overrides.
    let supported = codex_doctor_loaded_config(&output.stdout);
    if supported {
        if let Some(fingerprint) = fingerprint {
            cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(fingerprint);
        }
    }
    supported
}

fn codex_help_advertises_hook_trust_bypass(stdout: &[u8], stderr: &[u8]) -> bool {
    const FLAG: &[u8] = b"--dangerously-bypass-hook-trust";
    stdout.windows(FLAG.len()).any(|window| window == FLAG)
        || stderr.windows(FLAG.len()).any(|window| window == FLAG)
}

/// Whether this Codex build can run an untrusted command hook for one launch.
///
/// Codex records trust against the hook command's hash and skips anything
/// still under review. The observation command points at this binary, so the
/// hash changes whenever the binary moves and `/hooks` never stays trusted.
/// Without the bypass, `UserPromptSubmit` never arrives and the tab keeps its
/// draft name. The flag also lets the user's other enabled hooks run without
/// that review for this invocation. A build that does not advertise it must
/// omit the flag: an unknown argument refuses to start.
async fn codex_supports_hook_trust_bypass(binary: &Path) -> bool {
    let Some(fingerprint) = binary_fingerprint(binary) else {
        return false;
    };
    let cache = CODEX_HOOK_TRUST_BYPASS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(known) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&fingerprint)
        .copied()
    {
        return known;
    }
    let Some(supported) = probe_codex_hook_trust_bypass(binary).await else {
        return false;
    };
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(fingerprint, supported);
    supported
}

async fn probe_codex_hook_trust_bypass(binary: &Path) -> Option<bool> {
    let mut command = agent_cli_command(binary);
    command.arg("--help").kill_on_drop(true);
    // `--help` exits 0 on current builds and nonzero on some clap versions.
    // The text is the answer either way.
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()?
        .ok()?;
    Some(codex_help_advertises_hook_trust_bypass(
        &output.stdout,
        &output.stderr,
    ))
}

const GROK_PROBE_COMMAND_PREFIX: &str = "codeg-grok-hook-probe-";
const GROK_OBSERVE_HOOKS_FILE: &str = "codeg-observe.json";

static GROK_HOOK_PROBES: OnceLock<Mutex<HashSet<BinaryFingerprint>>> = OnceLock::new();

async fn probe_grok_hooks(binary: &Path) -> bool {
    let fingerprint = binary_fingerprint(binary);
    let cache = GROK_HOOK_PROBES.get_or_init(|| Mutex::new(HashSet::new()));
    if fingerprint.as_ref().is_some_and(|fingerprint| {
        cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(fingerprint)
    }) {
        return true;
    }
    // GROK_CONFIG cannot carry hooks. Grok scans a real `$GROK_HOME/hooks`.
    let Ok(home) = tempfile::Builder::new()
        .prefix("codeg-grok-probe-")
        .tempdir()
    else {
        return false;
    };
    let command = format!("{GROK_PROBE_COMMAND_PREFIX}{}", uuid::Uuid::new_v4());
    if write_grok_probe_hook(home.path(), &command).is_err() {
        return false;
    }
    let mut child = agent_cli_command(binary);
    child.args(["inspect", "--json"]);
    child.env("GROK_HOME", home.path());
    child.stdin(std::process::Stdio::null());
    child.kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), child.output()).await else {
        return false;
    };
    let supported = grok_inspect_contains_command(&output.stdout, &command);
    if supported {
        if let Some(fingerprint) = fingerprint {
            cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(fingerprint);
        }
    }
    supported
}

fn write_grok_probe_hook(home: &Path, command: &str) -> std::io::Result<()> {
    let hooks = home.join("hooks");
    std::fs::create_dir(&hooks)?;
    let payload = serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": command,
                }]
            }]
        }
    });
    let bytes = serde_json::to_vec(&payload).map_err(std::io::Error::other)?;
    std::fs::write(hooks.join(GROK_OBSERVE_HOOKS_FILE), bytes)
}

fn grok_inspect_contains_command(stdout: &[u8], command: &str) -> bool {
    if command.is_empty() {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return false;
    };
    json_contains_str(&value, command)
}

fn json_contains_str(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains(needle),
        serde_json::Value::Array(items) => items.iter().any(|item| json_contains_str(item, needle)),
        serde_json::Value::Object(map) => map.values().any(|item| json_contains_str(item, needle)),
        _ => false,
    }
}

fn codex_doctor_loaded_config(output: &[u8]) -> bool {
    let Ok(report) = serde_json::from_slice::<serde_json::Value>(output) else {
        return false;
    };
    let loaded = |key: Option<&str>, check: &serde_json::Value| {
        (check["id"].as_str() == Some("config.load") || key == Some("config.load"))
            && matches!(check["status"].as_str(), Some("ok" | "warning"))
    };
    match report.get("checks") {
        Some(serde_json::Value::Array(checks)) => checks.iter().any(|check| loaded(None, check)),
        Some(serde_json::Value::Object(checks)) => checks
            .iter()
            .any(|(key, check)| loaded(Some(key.as_str()), check)),
        _ => false,
    }
}

async fn probe_claude_hooks(binary: &Path) -> bool {
    let mut command = agent_cli_command(binary);
    command.arg("--version").kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(2), command.output()).await
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .split_whitespace()
        .chain(stderr.split_whitespace())
        .filter_map(|word| semver::Version::parse(word.trim_start_matches('v')).ok())
        .any(|version| version >= semver::Version::new(1, 0, 54))
}

/// A native session id passed to `resume`. Empty is a fresh launch. Rejects
/// flag-shaped values and embedded newlines so the id cannot become another
/// option. Spaces stay: Grok can resume by title.
fn resume_session_argument(value: Option<&str>) -> Result<Option<String>, TerminalError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let id = raw.trim();
    if id.is_empty() {
        return Ok(None);
    }
    if id.len() > 256
        || id.chars().any(|ch| ch == '\0' || ch == '\n' || ch == '\r')
        || id.starts_with('-')
    {
        return Err(TerminalError::SpawnFailed(
            "invalid resume session id".to_string(),
        ));
    }
    Ok(Some(id.to_string()))
}

fn build_arguments(
    agent_type: AgentType,
    prompt: &str,
    hook_enabled: bool,
    bypass_hook_trust: bool,
    helper_binary: &Path,
    temp_files: &mut Vec<PathBuf>,
    resume_session_id: Option<&str>,
    submit_prompt_on_resume: bool,
) -> Result<Vec<String>, TerminalError> {
    let resume = resume_session_argument(resume_session_id)?;
    let mut args = Vec::new();
    match agent_type {
        AgentType::ClaudeCode => {
            if hook_enabled {
                let settings = claude_settings_file(helper_binary)?;
                args.extend([
                    "--settings".to_string(),
                    settings.to_string_lossy().into_owned(),
                ]);
                temp_files.push(settings);
            }
            if let Some(id) = resume.as_ref() {
                args.extend(["--resume".to_string(), id.clone()]);
            }
        }
        AgentType::Codex => {
            if resume.is_some() {
                args.push("resume".to_string());
            }
            if hook_enabled {
                // See `codex_supports_hook_trust_bypass`. Only a build that
                // advertises the flag gets it, and only while our hooks are on.
                if bypass_hook_trust {
                    args.push("--dangerously-bypass-hook-trust".to_string());
                }
                args.extend(["--enable".to_string(), "hooks".to_string()]);
                for (event, action) in [("UserPromptSubmit", "turn-start"), ("Stop", "turn-stop")] {
                    args.push("-c".to_string());
                    args.push(codex_override(event, &hook_command(helper_binary, action)));
                }
            }
            if let Some(id) = resume.as_ref() {
                args.push(id.clone());
            }
        }
        AgentType::Grok => {
            args.push("--no-auto-update".to_string());
            if let Some(id) = resume.as_ref() {
                args.extend(["--resume".to_string(), id.clone()]);
            }
        }
        _ => {
            return Err(TerminalError::SpawnFailed(
                "this agent has no verified native terminal launch".to_string(),
            ));
        }
    }
    // Opening a saved session from the list must not submit a turn. A channel
    // follow-up sets `submit_prompt_on_resume` so the resumed CLI receives the
    // new user message.
    if !prompt.is_empty() && (resume.is_none() || submit_prompt_on_resume) {
        args.push(prompt.to_string());
    }
    Ok(args)
}

/// Remove a Claude settings file or a Grok home overlay.
///
/// A symlink root is left in place. Child symlinks are unlinked, not followed,
/// so an overlay cannot delete `~/.grok`.
pub(crate) fn cleanup_agent_temp_path(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to delete a symlink",
        ));
    }
    if metadata.is_dir() {
        remove_real_tree(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn remove_real_tree(dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            std::fs::remove_file(&path)?;
        } else if file_type.is_dir() {
            remove_real_tree(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    std::fs::remove_dir(dir)
}

fn apply_grok_hook_home(
    agent_type: AgentType,
    hook_enabled: bool,
    extra_env: &mut HashMap<String, String>,
    temp_files: &mut Vec<PathBuf>,
    source_home: &Path,
    helper_binary: &Path,
) -> Result<(), TerminalError> {
    if !(hook_enabled && agent_type == AgentType::Grok) {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let overlay = build_grok_home_overlay(source_home, helper_binary)?;
        extra_env.insert(
            "GROK_HOME".to_string(),
            overlay.to_string_lossy().into_owned(),
        );
        temp_files.push(overlay);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (extra_env, temp_files, source_home, helper_binary);
        Ok(())
    }
}

#[cfg(unix)]
fn build_grok_home_overlay(
    source_home: &Path,
    helper_binary: &Path,
) -> Result<PathBuf, TerminalError> {
    let overlay = tempfile::Builder::new()
        .prefix("codeg-grok-home-")
        .tempdir()
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?
        .keep();
    if let Err(error) = populate_grok_home_overlay(&overlay, source_home, helper_binary) {
        let _ = cleanup_agent_temp_path(&overlay);
        return Err(error);
    }
    Ok(overlay)
}

#[cfg(unix)]
fn populate_grok_home_overlay(
    overlay: &Path,
    source_home: &Path,
    helper_binary: &Path,
) -> Result<(), TerminalError> {
    // `hooks/` stays a real directory. Symlinking it would write the observe
    // file through into the user's home.
    if path_is_dir(source_home) {
        mirror_grok_source_entries(overlay, source_home)?;
        copy_grok_hooks_paths(overlay, source_home)?;
    }
    let hooks = overlay.join("hooks");
    std::fs::create_dir(&hooks).map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    let source_hooks = source_home.join("hooks");
    if path_is_dir(&source_hooks) {
        copy_grok_hook_json(&source_hooks, &hooks)?;
    }
    let body = serde_json::to_vec(&grok_observation_hooks(helper_binary))
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    std::fs::write(hooks.join(GROK_OBSERVE_HOOKS_FILE), body)
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    Ok(())
}

#[cfg(unix)]
fn path_is_dir(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(unix)]
fn mirror_grok_source_entries(overlay: &Path, source_home: &Path) -> Result<(), TerminalError> {
    for entry in std::fs::read_dir(source_home)
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?
    {
        let entry = entry.map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
        let name = entry.file_name();
        // `hooks/` and `hooks-paths` are real copies. `leader.sock` stays out
        // so this launch does not attach to the user's leader process.
        if name == "hooks" || name == "hooks-paths" || name == "leader.sock" {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), overlay.join(name))
            .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn copy_grok_hooks_paths(overlay: &Path, source_home: &Path) -> Result<(), TerminalError> {
    let source_file = source_home.join("hooks-paths");
    if !std::fs::metadata(&source_file).is_ok_and(|metadata| metadata.is_file()) {
        return Ok(());
    }
    std::fs::copy(&source_file, overlay.join("hooks-paths"))
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    Ok(())
}

#[cfg(unix)]
fn copy_grok_hook_json(source_hooks: &Path, dest_hooks: &Path) -> Result<(), TerminalError> {
    for entry in std::fs::read_dir(source_hooks)
        .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?
    {
        let entry = entry.map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
        let is_json = entry
            .path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
        if !is_json {
            continue;
        }
        let source_file = entry.path();
        if !std::fs::metadata(&source_file).is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        std::fs::copy(&source_file, dest_hooks.join(entry.file_name()))
            .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
    }
    Ok(())
}

fn merge_launch_env(
    runtime_env: BTreeMap<String, String>,
    credential_env: HashMap<String, String>,
    hook_env: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = runtime_env.into_iter().collect::<HashMap<_, _>>();
    merged.extend(credential_env);
    merged.extend(hook_env);
    merged
}

#[allow(clippy::too_many_arguments)]
pub async fn spawn_agent(
    manager: &TerminalManager,
    registry: Arc<AgentHookRegistry>,
    emitter: EventEmitter,
    agent_type: AgentType,
    working_dir: String,
    prompt: String,
    resume_session_id: Option<String>,
    terminal_id: String,
    shell: Option<String>,
    owner_window_label: String,
    data_dir: &Path,
    runtime_env: BTreeMap<String, String>,
    submit_prompt_on_resume: bool,
) -> Result<SpawnAgentResponse, TerminalError> {
    if terminal_id.is_empty() || terminal_id.len() > 256 || terminal_id.contains('\0') {
        return Err(TerminalError::SpawnFailed(
            "invalid terminal id".to_string(),
        ));
    }
    if !Path::new(&working_dir).is_dir() {
        return Err(TerminalError::SpawnFailed(
            "working directory does not exist".to_string(),
        ));
    }
    if prompt.len() > 64 * 1024 || prompt.contains('\0') {
        return Err(TerminalError::SpawnFailed(
            "invalid or oversized prompt".to_string(),
        ));
    }
    let resume_session_id = resume_session_argument(resume_session_id.as_deref())?;
    let binary = native_binary(agent_type).await.ok_or_else(|| {
        TerminalError::SpawnFailed(format!("native {} CLI is not installed", agent_type))
    })?;
    let shell = shell.unwrap_or_else(resolve_shell);
    #[cfg(target_os = "windows")]
    let shell = if matches!(classify_shell_family(&shell), ShellFamily::Cmd) {
        // cmd cannot quote an arbitrary multiline prompt without expansion.
        // PowerShell is present on supported Windows hosts and keeps this
        // launch's argument boundaries intact.
        "powershell.exe".to_string()
    } else {
        shell
    };
    // Reject unsupported quoting before creating a listener, token or file.
    shell_command(&shell, &binary, &[])?;
    let hook_enabled = verified_hook_support(agent_type, &binary).await;
    let bypass_hook_trust = hook_enabled
        && agent_type == AgentType::Codex
        && codex_supports_hook_trust_bypass(&binary).await;
    let callback_url = if hook_enabled {
        Some(
            registry
                .ensure_listener()
                .await
                .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?,
        )
    } else {
        None
    };
    let (state, token) = registry
        .register_run(&terminal_id, agent_type.as_wire().as_ref(), hook_enabled)
        .ok_or_else(|| TerminalError::SpawnFailed("agent terminal is already running".into()))?;
    let mut temp_files = Vec::new();
    let prepared = (|| {
        let helper_binary = std::env::current_exe()
            .map_err(|error| TerminalError::SpawnFailed(error.to_string()))?;
        let args = build_arguments(
            agent_type,
            &prompt,
            hook_enabled,
            bypass_hook_trust,
            &helper_binary,
            &mut temp_files,
            resume_session_id.as_deref(),
            submit_prompt_on_resume,
        )?;
        let initial_command = shell_command(&shell, &binary, &args)?;
        let credential_env =
            crate::commands::terminal::prepare_credential_env(data_dir).unwrap_or_default();
        let hook_env = match (callback_url.as_deref(), token.as_deref()) {
            (Some(url), Some(token)) => hook_environment(url, &state.run_id, token),
            _ => HashMap::new(),
        };
        let mut extra_env = merge_launch_env(runtime_env, credential_env, hook_env);
        // A failed probe leaves hook_enabled false and does not set GROK_HOME.
        apply_grok_hook_home(
            agent_type,
            hook_enabled,
            &mut extra_env,
            &mut temp_files,
            &crate::parsers::grok::resolve_grok_home_dir(),
            &helper_binary,
        )?;
        Ok::<(String, HashMap<String, String>), TerminalError>((initial_command, extra_env))
    })();
    let (initial_command, extra_env) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            registry.revoke(&terminal_id, &state.run_id);
            for file in temp_files {
                let _ = cleanup_agent_temp_path(&file);
            }
            return Err(error);
        }
    };
    let cleanup_files = temp_files.clone();
    let cleanup_registry = registry.clone();
    let cleanup_terminal_id = terminal_id.clone();
    let cleanup_run_id = state.run_id.clone();
    let check_emitter = emitter.clone();
    let spawned = manager.spawn_with_id(
        SpawnOptions {
            terminal_id: terminal_id.clone(),
            working_dir,
            owner_window_label,
            shell: Some(shell),
            initial_command: Some(initial_command),
            extra_env: Some(extra_env),
            temp_files,
            on_exit: Some(Box::new(move || {
                cleanup_registry.note_process_exit(&cleanup_terminal_id, &cleanup_run_id);
            })),
        },
        emitter,
    );
    if let Err(error) = spawned {
        registry.revoke(&terminal_id, &state.run_id);
        for file in cleanup_files {
            let _ = cleanup_agent_temp_path(&file);
        }
        return Err(error);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    if !manager
        .list_with_exit_check(Some(&check_emitter))
        .iter()
        .any(|terminal| terminal.id == terminal_id)
    {
        registry.revoke(&terminal_id, &state.run_id);
        return Err(TerminalError::SpawnFailed(
            "native agent exited during startup".to_string(),
        ));
    }
    if let Some(session_id) = resume_session_id.as_deref() {
        registry.remember_native_session(&terminal_id, session_id);
    }
    Ok(SpawnAgentResponse {
        terminal_id,
        run_id: state.run_id,
        hook_support: state.hook_support,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn launch_env_preserves_resolved_settings_and_hook_overrides() {
        let runtime_env = BTreeMap::from([
            ("OPENAI_MODEL".to_string(), "gpt-test".to_string()),
            ("CURSOR_API_KEY".to_string(), String::new()),
        ]);
        let credential_env = HashMap::from([("GIT_CONFIG_COUNT".to_string(), "1".to_string())]);
        let hook_env = HashMap::from([(
            "CODEG_AGENT_HOOK_TOKEN".to_string(),
            "hook-token".to_string(),
        )]);

        let merged = merge_launch_env(runtime_env, credential_env, hook_env);

        assert_eq!(
            merged.get("OPENAI_MODEL").map(String::as_str),
            Some("gpt-test")
        );
        assert_eq!(merged.get("CURSOR_API_KEY").map(String::as_str), Some(""));
        assert_eq!(
            merged.get("GIT_CONFIG_COUNT").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            merged.get("CODEG_AGENT_HOOK_TOKEN").map(String::as_str),
            Some("hook-token")
        );
    }

    #[cfg(unix)]
    #[test]
    fn prompt_is_one_literal_shell_argument() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("unexpected");
        let prompt = format!("中文 ' \"\n$(touch {}) ; $HOME", marker.display());
        let script = format!("printf '%s' {}", quote_posix(&prompt));
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, prompt.as_bytes());
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn launch_command_quotes_executable_path_and_prompt() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("agent cli");
        std::fs::write(&binary, "#!/bin/sh\nprintf '<%s>' \"$1\"\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let marker = dir.path().join("unexpected");
        let prompt = format!("多行 ' \"\n$(touch {})", marker.display());
        let command = shell_command("/bin/sh", &binary, std::slice::from_ref(&prompt)).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, format!("<{prompt}>").as_bytes());
        assert!(!marker.exists());
    }

    #[test]
    fn codex_override_is_valid_toml_with_quoted_command() {
        let command = "/a path/codeg --agent-hook turn-start";
        let override_value = codex_override("UserPromptSubmit", command);
        let parsed: toml::Value = format!("{override_value}\n").parse().unwrap();
        assert_eq!(
            parsed["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"].as_str(),
            Some(command)
        );
    }

    #[test]
    fn claude_temp_settings_only_add_observation_hooks() {
        let path = claude_settings_file(Path::new("/tmp/codeg app/bin/codeg")).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["hooks"].as_object().unwrap().len(), 4);
        assert!(value["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--agent-hook turn-stop"));
        assert!(value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--agent-hook session-start"));
        assert!(value["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--agent-hook session-end"));
        assert!(value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--agent-hook turn-start"));
        assert!(!content.contains("bypass"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn codex_launch_hooks_stay_on_turn_events() {
        let mut temps = Vec::new();
        let args = build_arguments(
            AgentType::Codex,
            "",
            true,
            false,
            Path::new("/tmp/codeg"),
            &mut temps,
            None,
            false,
        )
        .unwrap();
        let joined = args.join(" ");
        assert!(joined.contains("UserPromptSubmit"));
        assert!(joined.contains("Stop"));
        assert!(joined.contains("--agent-hook turn-start"));
        assert!(joined.contains("--agent-hook turn-stop"));
        assert!(!joined.contains("--dangerously-bypass-hook-trust"));
        assert!(!joined.contains("SessionStart"));
        assert!(!joined.contains("session-start"));
        assert!(!joined.contains("SessionEnd"));
        assert!(!joined.contains("session-end"));
    }

    #[test]
    fn codex_launch_bypasses_hook_trust_when_the_binary_advertises_it() {
        let mut temps = Vec::new();
        let args = build_arguments(
            AgentType::Codex,
            "ping",
            true,
            true,
            Path::new("/tmp/codeg"),
            &mut temps,
            None,
            false,
        )
        .unwrap();
        assert_eq!(args[0], "--dangerously-bypass-hook-trust");
        assert!(args.iter().any(|arg| arg == "--enable"));
        assert_eq!(args.last().map(String::as_str), Some("ping"));
        let joined = args.join(" ");
        assert!(!joined.contains("SessionStart"));
        assert!(temps.is_empty());

        let mut without_hooks = Vec::new();
        let plain = build_arguments(
            AgentType::Codex,
            "ping",
            false,
            true,
            Path::new("/tmp/codeg"),
            &mut without_hooks,
            None,
            false,
        )
        .unwrap();
        assert_eq!(plain, vec!["ping".to_string()]);
    }

    #[test]
    fn npm_shim_probe_path_puts_the_binary_directory_first() {
        let current = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        let path = path_with_binary_dir(Path::new("/opt/node/bin/codex"), Some(current)).unwrap();
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/opt/node/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]
        );
        assert!(path_with_binary_dir(Path::new("codex"), None).is_none());
    }

    #[test]
    fn codex_help_text_detects_only_the_hook_trust_bypass_flag() {
        assert!(codex_help_advertises_hook_trust_bypass(
            b"      --dangerously-bypass-hook-trust\n          Run enabled hooks\n",
            b"",
        ));
        assert!(codex_help_advertises_hook_trust_bypass(
            b"",
            b"usage: --dangerously-bypass-hook-trust\n",
        ));
        assert!(!codex_help_advertises_hook_trust_bypass(
            b"      --dangerously-bypass-approvals-and-sandbox\n",
            b"",
        ));
        assert!(!codex_help_advertises_hook_trust_bypass(b"", b""));
    }

    #[test]
    fn codex_doctor_probe_accepts_valid_hook_config_even_with_unrelated_failure() {
        assert!(codex_doctor_loaded_config(
            br#"{"overallStatus":"fail","checks":[{"id":"config.load","status":"warning"}]}"#
        ));
        assert!(codex_doctor_loaded_config(
            br#"{"overallStatus":"fail","checks":{"config.load":{"id":"config.load","status":"warning"}}}"#
        ));
        assert!(!codex_doctor_loaded_config(
            br#"{"overallStatus":"fail","checks":[{"id":"config.load","status":"fail"}]}"#
        ));
        assert!(!codex_doctor_loaded_config(
            br#"{"overallStatus":"fail","checks":{"config.load":{"id":"config.load","status":"fail"}}}"#
        ));
        assert!(!codex_doctor_loaded_config(b"not json"));
    }

    #[test]
    fn grok_arguments_are_only_no_auto_update_and_the_prompt() {
        let mut temps = Vec::new();
        let with_prompt = build_arguments(
            AgentType::Grok,
            "ping",
            true,
            false,
            Path::new("/tmp/codeg"),
            &mut temps,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            with_prompt,
            vec!["--no-auto-update".to_string(), "ping".to_string()]
        );
        assert!(temps.is_empty());
        let no_prompt = build_arguments(
            AgentType::Grok,
            "",
            true,
            false,
            Path::new("/tmp/codeg"),
            &mut temps,
            None,
            false,
        )
        .unwrap();
        assert_eq!(no_prompt, vec!["--no-auto-update".to_string()]);
        assert!(temps.is_empty());
    }

    #[test]
    fn resume_reopens_the_native_session_without_a_new_prompt() {
        let mut temps = Vec::new();
        let codex = build_arguments(
            AgentType::Codex,
            "ping",
            true,
            true,
            Path::new("/tmp/codeg"),
            &mut temps,
            Some("sid-1"),
            false,
        )
        .unwrap();
        assert_eq!(codex[0], "resume");
        assert_eq!(codex[1], "--dangerously-bypass-hook-trust");
        assert_eq!(codex.last().map(String::as_str), Some("sid-1"));
        assert!(!codex.iter().any(|arg| arg == "ping"));
        let joined = codex.join(" ");
        assert!(joined.contains("UserPromptSubmit"));
        assert!(joined.contains("Stop"));
        assert!(!joined.contains("SessionStart"));
        assert!(!joined.contains("session-start"));
        assert!(temps.is_empty());

        let mut plain_temps = Vec::new();
        let plain = build_arguments(
            AgentType::Codex,
            "ping",
            false,
            true,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("sid-1"),
            false,
        )
        .unwrap();
        assert_eq!(plain, vec!["resume".to_string(), "sid-1".to_string()]);
        let submitted = build_arguments(
            AgentType::Codex,
            "ping",
            false,
            true,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("sid-1"),
            true,
        )
        .unwrap();
        assert_eq!(
            submitted,
            vec![
                "resume".to_string(),
                "sid-1".to_string(),
                "ping".to_string()
            ]
        );

        let grok = build_arguments(
            AgentType::Grok,
            "ping",
            true,
            false,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("title with space"),
            false,
        )
        .unwrap();
        assert_eq!(
            grok,
            vec![
                "--no-auto-update".to_string(),
                "--resume".to_string(),
                "title with space".to_string(),
            ]
        );

        let claude = build_arguments(
            AgentType::ClaudeCode,
            "ping",
            false,
            false,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("sid-claude"),
            false,
        )
        .unwrap();
        assert_eq!(
            claude,
            vec!["--resume".to_string(), "sid-claude".to_string()]
        );
        assert!(plain_temps.is_empty());

        let mut hooked = Vec::new();
        let claude_hooks = build_arguments(
            AgentType::ClaudeCode,
            "ping",
            true,
            false,
            Path::new("/tmp/codeg"),
            &mut hooked,
            Some("sid-claude"),
            false,
        )
        .unwrap();
        assert_eq!(claude_hooks[0], "--settings");
        assert_eq!(
            claude_hooks[claude_hooks.len() - 2..],
            ["--resume".to_string(), "sid-claude".to_string()]
        );
        assert!(!claude_hooks.iter().any(|arg| arg == "ping"));
        assert_eq!(hooked.len(), 1);
        std::fs::remove_file(&hooked[0]).unwrap();

        assert!(build_arguments(
            AgentType::Grok,
            "",
            false,
            false,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("-rf"),
            false
        )
        .is_err());
        assert!(build_arguments(
            AgentType::Grok,
            "",
            false,
            false,
            Path::new("/tmp/codeg"),
            &mut plain_temps,
            Some("id\nnext"),
            false
        )
        .is_err());
    }

    #[test]
    fn grok_inspect_requires_the_command_inside_json() {
        let command = "codeg-grok-hook-probe-abc";
        assert!(grok_inspect_contains_command(
            br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"codeg-grok-hook-probe-abc"}]}]}}"#,
            command,
        ));
        assert!(grok_inspect_contains_command(
            b"\n{\"command\":\"prefix codeg-grok-hook-probe-abc suffix\"}\n",
            command,
        ));
        assert!(!grok_inspect_contains_command(
            br#"{"command":"codeg-grok-hook-probe-abd"}"#,
            command,
        ));
        assert!(!grok_inspect_contains_command(
            b"not json but codeg-grok-hook-probe-abc",
            command,
        ));
        assert!(!grok_inspect_contains_command(
            b"{\"command\":\"codeg-grok-hook-probe-abc\"} trailing",
            command,
        ));
        assert!(!grok_inspect_contains_command(b"", command));
    }

    #[test]
    fn cleanup_removes_a_regular_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        std::fs::write(&file, b"{}").unwrap();
        cleanup_agent_temp_path(&file).unwrap();
        assert!(!file.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_drops_overlay_symlinks_and_keeps_the_sentinel() {
        let scratch = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("sentinel.txt");
        std::fs::write(&sentinel, b"keep").unwrap();
        let outside_dir = outside.path().join("dir");
        std::fs::create_dir(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("nested.txt"), b"keep-dir").unwrap();

        let overlay = scratch.path().join("overlay");
        std::fs::create_dir(&overlay).unwrap();
        std::os::unix::fs::symlink(&sentinel, overlay.join("linked")).unwrap();
        std::os::unix::fs::symlink(&outside_dir, overlay.join("dir-link")).unwrap();
        let owned = overlay.join("owned");
        std::fs::create_dir(&owned).unwrap();
        std::fs::write(owned.join("real.txt"), b"owned").unwrap();

        cleanup_agent_temp_path(&overlay).unwrap();
        assert!(!overlay.exists());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(
            std::fs::read(outside_dir.join("nested.txt")).unwrap(),
            b"keep-dir"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_refuses_to_delete_a_symlink_root() {
        let scratch = tempfile::tempdir().unwrap();
        let real = scratch.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("file.txt"), b"keep").unwrap();
        let link = scratch.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(cleanup_agent_temp_path(&link).is_err());
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(real.join("file.txt")).unwrap(), b"keep");
        assert!(real.is_dir());
    }

    #[cfg(unix)]
    fn tree_signature(root: &Path) -> String {
        if !root.exists() {
            return String::new();
        }
        fn walk(path: &Path, root: &Path, out: &mut String) {
            let mut entries = std::fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            entries.sort();
            for entry in entries {
                let rel = entry.strip_prefix(root).unwrap().to_string_lossy();
                let meta = std::fs::symlink_metadata(&entry).unwrap();
                if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&entry).unwrap();
                    out.push_str(&format!("link {rel} -> {}\n", target.display()));
                } else if meta.is_dir() {
                    out.push_str(&format!("dir {rel}\n"));
                    walk(&entry, root, out);
                } else {
                    let bytes = std::fs::read(&entry).unwrap();
                    out.push_str(&format!("file {rel} {}\n", bytes.len()));
                    out.push_str(&String::from_utf8_lossy(&bytes));
                    out.push('\n');
                }
            }
        }
        let mut out = String::new();
        walk(root, root, &mut out);
        out
    }

    #[cfg(unix)]
    fn assert_real_dir(path: &Path) {
        let meta = std::fs::symlink_metadata(path).unwrap();
        assert!(meta.is_dir(), "{}", path.display());
        assert!(!meta.file_type().is_symlink(), "{}", path.display());
    }

    #[cfg(unix)]
    fn assert_real_file(path: &Path, bytes: &[u8]) {
        let meta = std::fs::symlink_metadata(path).unwrap();
        assert!(meta.is_file(), "{}", path.display());
        assert!(!meta.file_type().is_symlink(), "{}", path.display());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn grok_home_overlay_mirrors_source_without_writing_it() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("grok-src");
        let real_hooks = scratch.path().join("real-hooks");
        let skills_real = scratch.path().join("skills-real");
        let external_hook = scratch.path().join("external-hook.json");
        let paths_file = scratch.path().join("paths-file");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&real_hooks).unwrap();
        std::fs::create_dir(&skills_real).unwrap();
        std::fs::create_dir(real_hooks.join("nested")).unwrap();
        std::fs::write(source.join("config.toml"), b"model = \"x\"\n").unwrap();
        std::fs::write(source.join("hooks.bak"), b"backup").unwrap();
        std::fs::create_dir(source.join("projects")).unwrap();
        std::fs::write(source.join("projects").join("marker.txt"), b"project").unwrap();
        std::fs::write(skills_real.join("skill.md"), b"hi").unwrap();
        std::os::unix::fs::symlink(&skills_real, source.join("skills")).unwrap();
        std::fs::write(&external_hook, b"{\"from\":\"alias\"}\n").unwrap();
        std::fs::write(real_hooks.join("user.json"), b"{\"from\":\"user\"}\n").unwrap();
        std::os::unix::fs::symlink(&external_hook, real_hooks.join("alias.json")).unwrap();
        std::fs::write(real_hooks.join("notes.txt"), b"notes").unwrap();
        std::fs::write(
            real_hooks.join("nested").join("nested.json"),
            b"{\"nested\":true}\n",
        )
        .unwrap();
        std::fs::write(real_hooks.join("codeg-observe.json"), b"{\"hooks\":{}}\n").unwrap();
        std::os::unix::fs::symlink(&real_hooks, source.join("hooks")).unwrap();
        std::fs::write(&paths_file, b"/tmp/a\n/tmp/b\n").unwrap();
        std::os::unix::fs::symlink(&paths_file, source.join("hooks-paths")).unwrap();
        std::fs::write(source.join("leader.sock"), b"sock").unwrap();

        let source_before = tree_signature(&source);
        let hooks_before = tree_signature(&real_hooks);
        let helper = Path::new("/tmp/codeg app/bin/codeg");
        let overlay = build_grok_home_overlay(&source, helper).unwrap();

        assert_eq!(tree_signature(&source), source_before);
        assert_eq!(tree_signature(&real_hooks), hooks_before);
        assert_eq!(
            std::fs::read(&external_hook).unwrap(),
            b"{\"from\":\"alias\"}\n"
        );
        assert_eq!(std::fs::read(&paths_file).unwrap(), b"/tmp/a\n/tmp/b\n");
        assert_real_dir(&overlay);
        assert_real_dir(&overlay.join("hooks"));
        assert_eq!(
            std::fs::read_link(overlay.join("config.toml")).unwrap(),
            source.join("config.toml")
        );
        assert_eq!(
            std::fs::read_link(overlay.join("hooks.bak")).unwrap(),
            source.join("hooks.bak")
        );
        assert_eq!(
            std::fs::read_link(overlay.join("projects")).unwrap(),
            source.join("projects")
        );
        assert_eq!(
            std::fs::read_link(overlay.join("skills")).unwrap(),
            source.join("skills")
        );
        assert!(!overlay.join("leader.sock").exists());
        assert!(!overlay.join("hooks").join("notes.txt").exists());
        assert!(!overlay.join("hooks").join("nested").exists());
        assert_real_file(
            &overlay.join("hooks").join("user.json"),
            b"{\"from\":\"user\"}\n",
        );
        assert_real_file(
            &overlay.join("hooks").join("alias.json"),
            b"{\"from\":\"alias\"}\n",
        );
        assert_real_file(&overlay.join("hooks-paths"), b"/tmp/a\n/tmp/b\n");

        let observe = overlay.join("hooks").join("codeg-observe.json");
        assert_real_file_exists(&observe);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&observe).unwrap()).unwrap();
        assert_eq!(value["hooks"].as_object().unwrap().len(), 6);
        for (event, action) in [
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "turn-start"),
            ("Stop", "turn-stop"),
            ("StopFailure", "turn-stop"),
            ("StopCancelled", "turn-stop"),
            ("SessionEnd", "session-end"),
        ] {
            let command = value["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap();
            assert_eq!(value["hooks"][event][0]["hooks"][0]["type"], "command");
            assert!(command.contains("--agent-hook"));
            assert!(command.contains(action), "{command}");
            assert!(command.contains("/tmp/codeg app/bin/codeg"), "{command}");
        }
        assert_eq!(
            std::fs::read(real_hooks.join("codeg-observe.json")).unwrap(),
            b"{\"hooks\":{}}\n"
        );

        cleanup_agent_temp_path(&overlay).unwrap();
        assert!(!overlay.exists());
        assert_eq!(tree_signature(&source), source_before);
        assert_eq!(tree_signature(&real_hooks), hooks_before);
        assert_eq!(std::fs::read(skills_real.join("skill.md")).unwrap(), b"hi");
    }

    #[cfg(unix)]
    fn assert_real_file_exists(path: &Path) {
        let meta = std::fs::symlink_metadata(path).unwrap();
        assert!(meta.is_file(), "{}", path.display());
        assert!(!meta.file_type().is_symlink(), "{}", path.display());
    }

    #[cfg(unix)]
    #[test]
    fn grok_overlay_from_a_missing_home_does_not_create_it() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("missing-home");
        let helper = Path::new("/tmp/codeg app/bin/codeg");
        let overlay = build_grok_home_overlay(&missing, helper).unwrap();
        assert!(!missing.exists());
        assert_real_dir(&overlay);
        assert_real_dir(&overlay.join("hooks"));
        assert!(overlay.join("hooks").join("codeg-observe.json").is_file());
        let entries = std::fs::read_dir(&overlay).unwrap().count();
        assert_eq!(entries, 1);
        cleanup_agent_temp_path(&overlay).unwrap();
        assert!(!overlay.exists());
        assert!(!missing.exists());
    }

    #[cfg(unix)]
    #[test]
    fn grok_home_is_exported_only_for_an_enabled_grok_launch() {
        let scratch = tempfile::tempdir().unwrap();
        let source = scratch.path().join("grok-src");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("config.toml"), b"model = \"x\"\n").unwrap();
        let helper = Path::new("/tmp/codeg app/bin/codeg");

        for (agent_type, hook_enabled) in [
            (AgentType::Grok, false),
            (AgentType::ClaudeCode, true),
            (AgentType::Codex, true),
        ] {
            let mut env = HashMap::new();
            let mut temps = Vec::new();
            apply_grok_hook_home(
                agent_type,
                hook_enabled,
                &mut env,
                &mut temps,
                &source,
                helper,
            )
            .unwrap();
            assert!(env.get("GROK_HOME").is_none());
            assert!(temps.is_empty());
        }

        let mut env = HashMap::new();
        let mut temps = Vec::new();
        apply_grok_hook_home(AgentType::Grok, true, &mut env, &mut temps, &source, helper).unwrap();
        let home = PathBuf::from(env.get("GROK_HOME").expect("GROK_HOME"));
        assert_eq!(temps, vec![home.clone()]);
        assert_ne!(home, source);
        assert!(home.join("hooks").join("codeg-observe.json").is_file());
        assert!(home
            .join("config.toml")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        cleanup_agent_temp_path(&home).unwrap();
        assert!(!home.exists());
        assert_eq!(
            std::fs::read(source.join("config.toml")).unwrap(),
            b"model = \"x\"\n"
        );
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn run_count(path: &Path) -> usize {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .count()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grok_hook_probe_caches_success_for_one_binary_fingerprint() {
        let scratch = tempfile::tempdir().unwrap();
        let binary = scratch.path().join("grok");
        let runs = scratch.path().join("runs");
        let capture = scratch.path().join("capture.json");
        let home_note = scratch.path().join("home-path");
        let runs_q = quote_posix(&runs.to_string_lossy());
        let capture_q = quote_posix(&capture.to_string_lossy());
        let home_note_q = quote_posix(&home_note.to_string_lossy());
        write_executable(
            &binary,
            &format!(
                r#"#!/bin/sh
set -eu
printf '1\n' >> {runs_q}
[ "$1" = inspect ]
[ "$2" = --json ]
[ "$#" -eq 2 ]
home="$GROK_HOME"
[ -n "$home" ]
[ ! -L "$home" ]
printf '%s\n' "$home" > {home_note_q}
found=$(find "$home" -mindepth 1 -maxdepth 1 -print)
[ "$found" = "$home/hooks" ]
[ ! -L "$home/hooks" ]
hook=$(find "$home/hooks" -mindepth 1 -maxdepth 1 -print)
[ "$hook" = "$home/hooks/codeg-observe.json" ]
cp "$hook" {capture_q}
cat "$hook"
"#
            ),
        );

        assert!(verified_hook_support(AgentType::Grok, &binary).await);
        assert_eq!(run_count(&runs), 1);
        assert!(verified_hook_support(AgentType::Grok, &binary).await);
        assert_eq!(run_count(&runs), 1, "a successful probe is cached");

        let probed_home = std::fs::read_to_string(&home_note).unwrap();
        assert!(!Path::new(probed_home.trim()).exists());
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
        let command = value["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(value["hooks"].as_object().unwrap().len(), 1);
        assert_eq!(
            value["hooks"]["SessionStart"][0]["hooks"][0]["type"],
            "command"
        );
        assert!(command.starts_with("codeg-grok-hook-probe-"), "{command}");
        assert!(command.len() > "codeg-grok-hook-probe-".len());

        write_executable(
            &binary,
            &format!(
                "#!/bin/sh\nset -eu\nprintf '1\\n' >> {runs_q}\nprintf '%s\\n' '{{\"hooks\":[]}}'\n"
            ),
        );
        assert!(!verified_hook_support(AgentType::Grok, &binary).await);
        assert_eq!(run_count(&runs), 2, "a changed binary is probed again");
        assert!(!verified_hook_support(AgentType::Grok, &binary).await);
        assert_eq!(run_count(&runs), 3, "a failed probe is not cached");
    }
}

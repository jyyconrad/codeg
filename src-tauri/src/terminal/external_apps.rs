//! Installed terminal *applications* (Ghostty, iTerm, …), distinct from the
//! PTY shell in `SystemTerminalSettings.default_shell`.
//!
//! Scanning and launch planning stay pure so tests never open a window.
//! `launch_now` / `launch_directory` are the only functions that spawn.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::{AvailableTerminalApps, TerminalAppOption};

const OPTION_SYSTEM: &str = "system";
const OPTION_CUSTOM: &str = "custom";

static PREFERRED_TERMINAL_APP: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Macos,
    /// Linux and every other Unix. They share the emulator probe the old
    /// external launcher used.
    Linux,
    Windows,
    Other,
}

impl HostOs {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(unix) {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// What a scan can see. Tests build this directly; production fills it from
/// the real host.
#[derive(Debug, Clone)]
pub struct TerminalHost {
    pub os: HostOs,
    /// macOS bundle file names that exist, e.g. `Ghostty.app`.
    pub bundles: BTreeSet<String>,
    /// Command name → absolute executable.
    pub binaries: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    InvalidInput(String),
    Failed(String),
}

impl LaunchError {
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidInput(message) | Self::Failed(message) => message,
        }
    }
}

#[derive(Clone, Copy)]
struct KnownApp {
    id: &'static str,
    label: &'static str,
    macos: bool,
    linux: bool,
    windows: bool,
    bundles: &'static [&'static str],
    /// Executable inside `Contents/MacOS`, when launching needs that binary.
    bundle_exe: Option<&'static str>,
    binaries: &'static [&'static str],
    kind: AppKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppKind {
    Ghostty,
    Iterm,
    Warp,
    Alacritty,
    Kitty,
    Wezterm,
    WindowsTerminal,
    GnomeTerminal,
    Konsole,
    Xterm,
    XTerminalEmulator,
}

const KNOWN: &[KnownApp] = &[
    KnownApp {
        id: "ghostty",
        label: "Ghostty",
        macos: true,
        linux: true,
        windows: true,
        bundles: &["Ghostty.app"],
        bundle_exe: Some("ghostty"),
        binaries: &["ghostty"],
        kind: AppKind::Ghostty,
    },
    KnownApp {
        id: "iterm",
        label: "iTerm",
        macos: true,
        linux: false,
        windows: false,
        bundles: &["iTerm.app"],
        bundle_exe: None,
        binaries: &[],
        kind: AppKind::Iterm,
    },
    KnownApp {
        id: "warp",
        label: "Warp",
        macos: true,
        linux: false,
        windows: false,
        bundles: &["Warp.app"],
        bundle_exe: None,
        binaries: &[],
        kind: AppKind::Warp,
    },
    KnownApp {
        id: "alacritty",
        label: "Alacritty",
        macos: true,
        linux: true,
        windows: true,
        bundles: &["Alacritty.app"],
        bundle_exe: Some("alacritty"),
        binaries: &["alacritty"],
        kind: AppKind::Alacritty,
    },
    KnownApp {
        id: "kitty",
        label: "kitty",
        macos: true,
        linux: true,
        windows: true,
        bundles: &["kitty.app"],
        bundle_exe: Some("kitty"),
        binaries: &["kitty"],
        kind: AppKind::Kitty,
    },
    KnownApp {
        id: "wezterm",
        label: "WezTerm",
        macos: true,
        linux: true,
        windows: true,
        bundles: &["WezTerm.app"],
        bundle_exe: Some("wezterm"),
        binaries: &["wezterm"],
        kind: AppKind::Wezterm,
    },
    KnownApp {
        id: "windows-terminal",
        label: "Windows Terminal",
        macos: false,
        linux: false,
        windows: true,
        bundles: &[],
        bundle_exe: None,
        binaries: &["wt.exe", "wt"],
        kind: AppKind::WindowsTerminal,
    },
    KnownApp {
        id: "gnome-terminal",
        label: "GNOME Terminal",
        macos: false,
        linux: true,
        windows: false,
        bundles: &[],
        bundle_exe: None,
        binaries: &["gnome-terminal"],
        kind: AppKind::GnomeTerminal,
    },
    KnownApp {
        id: "konsole",
        label: "Konsole",
        macos: false,
        linux: true,
        windows: false,
        bundles: &[],
        bundle_exe: None,
        binaries: &["konsole"],
        kind: AppKind::Konsole,
    },
    KnownApp {
        id: "xterm",
        label: "xterm",
        macos: false,
        linux: true,
        windows: false,
        bundles: &[],
        bundle_exe: None,
        binaries: &["xterm"],
        kind: AppKind::Xterm,
    },
    KnownApp {
        id: "x-terminal-emulator",
        label: "x-terminal-emulator",
        macos: false,
        linux: true,
        windows: false,
        bundles: &[],
        bundle_exe: None,
        binaries: &["x-terminal-emulator"],
        kind: AppKind::XTerminalEmulator,
    },
];

const LINUX_SYSTEM_ORDER: &[&str] = &["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"];

enum Preference<'a> {
    System,
    Known(KnownApp),
    Custom(&'a str),
}

pub fn set_preferred_terminal_app(value: Option<String>) {
    let normalized = value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == OPTION_SYSTEM {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    *PREFERRED_TERMINAL_APP
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = normalized;
}

pub fn preferred_terminal_app() -> Option<String> {
    PREFERRED_TERMINAL_APP
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn available_terminal_apps(stored: Option<&str>) -> AvailableTerminalApps {
    available_terminal_apps_for(stored, &current_host())
}

pub fn available_terminal_apps_for(
    stored: Option<&str>,
    host: &TerminalHost,
) -> AvailableTerminalApps {
    let stored_id = stored.map(str::trim).filter(|value| !value.is_empty());
    let mut options = vec![system_option()];
    for app in KNOWN {
        let supported = app.supports(host.os);
        let installed = supported && app.installed(host);
        let selected = stored_id == Some(app.id);
        if installed || selected {
            options.push(TerminalAppOption {
                id: app.id.to_string(),
                label_key: "terminalAppName".to_string(),
                label: Some(app.label.to_string()),
                value: Some(app.id.to_string()),
                exists: installed,
                accepts_custom_path: false,
            });
        }
    }
    options.push(custom_option());
    AvailableTerminalApps { options }
}

/// Spawn an external terminal that runs `command` in `cwd`. System default
/// uses Terminal.app, Windows `cmd`, or the first Linux emulator that exists.
pub fn launch_now(preference: Option<&str>, command: &str, cwd: &str) -> Result<(), LaunchError> {
    let spec = plan_launch(preference, Some(command), cwd, &current_host())?;
    spawn_launch(&spec)
}

/// Open `cwd` in the preferred app. System default returns `Ok(false)` so the
/// caller keeps the built-in terminal. A chosen app returns `Ok(true)` after
/// the process is spawned.
pub fn launch_directory(preference: Option<&str>, cwd: &str) -> Result<bool, LaunchError> {
    if matches!(classify(preference), Preference::System) {
        return Ok(false);
    }
    if !Path::new(cwd).is_dir() {
        return Err(LaunchError::InvalidInput(
            "directory does not exist".to_string(),
        ));
    }
    let spec = plan_launch(preference, None, cwd, &current_host())?;
    spawn_launch(&spec)?;
    Ok(true)
}

pub fn probe_terminal_app_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r', '\0']) {
        return false;
    }
    let candidate = Path::new(trimmed);
    if candidate.is_file() {
        return true;
    }
    let bundle = trimmed.trim_end_matches(['/', '\\']);
    if bundle.ends_with(".app") && Path::new(bundle).is_dir() {
        return true;
    }
    if !trimmed.contains('/') && !trimmed.contains('\\') {
        return which::which(trimmed).is_ok();
    }
    false
}

pub(crate) fn plan_launch(
    preference: Option<&str>,
    command: Option<&str>,
    cwd: &str,
    host: &TerminalHost,
) -> Result<LaunchCommand, LaunchError> {
    if command.is_some_and(|value| value.contains(['\n', '\r'])) || cwd.contains(['\n', '\r']) {
        return Err(LaunchError::InvalidInput(
            "terminal command and cwd must not contain newlines".to_string(),
        ));
    }
    let command = command.map(str::trim).filter(|value| !value.is_empty());
    match classify(preference) {
        Preference::System => plan_system(command, cwd, host),
        Preference::Known(app) => {
            if !app.supports(host.os) || !app.installed(host) {
                return Err(LaunchError::Failed(format!(
                    "{} is not installed",
                    app.label
                )));
            }
            plan_known(app, command, cwd, host)
        }
        Preference::Custom(path) => plan_custom(path, command, cwd),
    }
}

fn current_host() -> TerminalHost {
    let os = HostOs::current();
    let mut bundles = BTreeSet::new();
    let mut binaries = BTreeMap::new();
    if os == HostOs::Macos {
        for app in KNOWN {
            for bundle in app.bundles {
                if application_dirs()
                    .iter()
                    .any(|root| root.join(bundle).is_dir())
                {
                    bundles.insert((*bundle).to_string());
                }
                let Some(exe_name) = app.bundle_exe else {
                    continue;
                };
                for root in application_dirs() {
                    let exe = root.join(bundle).join("Contents/MacOS").join(exe_name);
                    if exe.is_file() {
                        binaries
                            .entry(exe_name.to_string())
                            .or_insert_with(|| exe.display().to_string());
                    }
                }
            }
        }
    }
    for app in KNOWN {
        for name in app.binaries {
            if binaries.contains_key(*name) {
                continue;
            }
            if let Ok(path) = which::which(name) {
                binaries.insert((*name).to_string(), path.display().to_string());
            }
        }
    }
    TerminalHost {
        os,
        bundles,
        binaries,
    }
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    dirs
}

fn classify(preference: Option<&str>) -> Preference<'_> {
    let Some(value) = preference.map(str::trim).filter(|value| !value.is_empty()) else {
        return Preference::System;
    };
    if value == OPTION_SYSTEM {
        return Preference::System;
    }
    if let Some(app) = KNOWN.iter().copied().find(|app| app.id == value) {
        return Preference::Known(app);
    }
    Preference::Custom(value)
}

impl KnownApp {
    fn supports(self, os: HostOs) -> bool {
        match os {
            HostOs::Macos => self.macos,
            HostOs::Linux => self.linux,
            HostOs::Windows => self.windows,
            HostOs::Other => false,
        }
    }

    fn installed(self, host: &TerminalHost) -> bool {
        self.bundles
            .iter()
            .any(|bundle| host.bundles.contains(*bundle))
            || self
                .binaries
                .iter()
                .any(|name| host.binaries.contains_key(*name))
    }

    fn binary(self, host: &TerminalHost) -> Option<String> {
        self.binaries
            .iter()
            .find_map(|name| host.binaries.get(*name).cloned())
    }
}

fn system_option() -> TerminalAppOption {
    TerminalAppOption {
        id: OPTION_SYSTEM.to_string(),
        label_key: "terminalSystemDefault".to_string(),
        label: None,
        value: None,
        exists: true,
        accepts_custom_path: false,
    }
}

fn custom_option() -> TerminalAppOption {
    TerminalAppOption {
        id: OPTION_CUSTOM.to_string(),
        label_key: "terminalShellCustom".to_string(),
        label: None,
        value: None,
        exists: true,
        accepts_custom_path: true,
    }
}

fn plan_system(
    command: Option<&str>,
    cwd: &str,
    host: &TerminalHost,
) -> Result<LaunchCommand, LaunchError> {
    match host.os {
        HostOs::Macos => Ok(osascript(&terminal_app_script(cwd, command))),
        HostOs::Windows => Ok(windows_cmd(cwd, command)),
        HostOs::Linux => plan_linux_system(command, cwd, host),
        HostOs::Other => Err(LaunchError::Failed(
            "unsupported platform for terminal launch".to_string(),
        )),
    }
}

fn plan_linux_system(
    command: Option<&str>,
    cwd: &str,
    host: &TerminalHost,
) -> Result<LaunchCommand, LaunchError> {
    for name in LINUX_SYSTEM_ORDER {
        let Some(program) = host.binaries.get(*name).cloned() else {
            continue;
        };
        let kind = KNOWN
            .iter()
            .find(|app| app.binaries.contains(name))
            .map(|app| app.kind)
            .unwrap_or(AppKind::Xterm);
        return plan_emulator(kind, &program, command, cwd);
    }
    Err(LaunchError::Failed(
        "no supported terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xterm)".to_string(),
    ))
}

fn plan_known(
    app: KnownApp,
    command: Option<&str>,
    cwd: &str,
    host: &TerminalHost,
) -> Result<LaunchCommand, LaunchError> {
    match app.kind {
        AppKind::Ghostty => plan_ghostty(command, cwd, host),
        AppKind::Iterm => Ok(osascript(&iterm_script(cwd, command))),
        AppKind::Warp => {
            if command.is_some() {
                return Err(LaunchError::InvalidInput(
                    "Warp can open a directory but cannot run a command. Choose Terminal, Ghostty, iTerm, Alacritty, kitty, or WezTerm.".to_string(),
                ));
            }
            Ok(LaunchCommand {
                program: "open".to_string(),
                args: vec!["-na".to_string(), "Warp".to_string(), cwd.to_string()],
                cwd: None,
            })
        }
        AppKind::WindowsTerminal => {
            let Some(program) = app.binary(host) else {
                return Err(LaunchError::Failed(format!(
                    "{} is not installed",
                    app.label
                )));
            };
            plan_emulator(app.kind, &program, command, cwd)
        }
        AppKind::Alacritty
        | AppKind::Kitty
        | AppKind::Wezterm
        | AppKind::GnomeTerminal
        | AppKind::Konsole
        | AppKind::Xterm
        | AppKind::XTerminalEmulator => {
            let Some(program) = app.binary(host) else {
                return Err(LaunchError::Failed(format!(
                    "{} is installed but its executable was not found",
                    app.label
                )));
            };
            plan_emulator(app.kind, &program, command, cwd)
        }
    }
}

fn plan_ghostty(
    command: Option<&str>,
    cwd: &str,
    host: &TerminalHost,
) -> Result<LaunchCommand, LaunchError> {
    let mut args = ghostty_args(cwd, command, host.os == HostOs::Linux);
    if host.os == HostOs::Macos && host.bundles.contains("Ghostty.app") {
        let mut open_args = vec![
            "-na".to_string(),
            "Ghostty".to_string(),
            "--args".to_string(),
        ];
        open_args.append(&mut args);
        return Ok(LaunchCommand {
            program: "open".to_string(),
            args: open_args,
            cwd: None,
        });
    }
    let Some(program) = host.binaries.get("ghostty").cloned() else {
        return Err(LaunchError::Failed("Ghostty is not installed".to_string()));
    };
    Ok(LaunchCommand {
        program,
        args,
        cwd: Some(cwd.to_string()),
    })
}

fn ghostty_args(cwd: &str, command: Option<&str>, linux: bool) -> Vec<String> {
    let mut args = Vec::new();
    if linux {
        args.push("--gtk-single-instance=false".to_string());
    }
    args.push(format!("--working-directory={cwd}"));
    if let Some(command) = command {
        let script = keep_open_script(command);
        args.push(format!("--command=sh -c {}", shell_single_quote(&script)));
    }
    args
}

fn plan_emulator(
    kind: AppKind,
    program: &str,
    command: Option<&str>,
    cwd: &str,
) -> Result<LaunchCommand, LaunchError> {
    let script = command.map(keep_open_script);
    let args = match kind {
        AppKind::Alacritty => {
            let mut args = vec!["--working-directory".to_string(), cwd.to_string()];
            if let Some(script) = script {
                args.extend(["-e".to_string(), "sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::Kitty => {
            let mut args = vec!["--directory".to_string(), cwd.to_string()];
            if let Some(script) = script {
                args.extend(["sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::Wezterm => {
            let mut args = vec!["start".to_string(), "--cwd".to_string(), cwd.to_string()];
            if let Some(script) = script {
                args.extend(["--".to_string(), "sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::WindowsTerminal => {
            let mut args = vec!["-d".to_string(), cwd.to_string()];
            if let Some(command) = command {
                args.extend(["cmd".to_string(), "/K".to_string(), command.to_string()]);
            }
            args
        }
        AppKind::GnomeTerminal => {
            let mut args = vec![format!("--working-directory={cwd}")];
            if let Some(script) = script {
                args.extend(["--".to_string(), "sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::Konsole => {
            let mut args = vec!["--workdir".to_string(), cwd.to_string()];
            if let Some(script) = script {
                args.extend(["-e".to_string(), "sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::Xterm | AppKind::XTerminalEmulator => {
            let mut args = Vec::new();
            if let Some(script) = script {
                args.extend(["-e".to_string(), "sh".to_string(), "-c".to_string(), script]);
            }
            args
        }
        AppKind::Ghostty | AppKind::Iterm | AppKind::Warp => {
            return Err(LaunchError::Failed(
                "internal terminal launch mismatch".to_string(),
            ));
        }
    };
    Ok(LaunchCommand {
        program: program.to_string(),
        args,
        cwd: Some(cwd.to_string()),
    })
}

fn plan_custom(path: &str, command: Option<&str>, cwd: &str) -> Result<LaunchCommand, LaunchError> {
    if is_app_bundle(path) {
        if command.is_some() {
            return Err(LaunchError::InvalidInput(
                "a custom app bundle can open a directory but cannot run a command".to_string(),
            ));
        }
        return Ok(LaunchCommand {
            program: "open".to_string(),
            args: vec!["-na".to_string(), path.to_string(), cwd.to_string()],
            cwd: None,
        });
    }
    let mut args = Vec::new();
    if let Some(command) = command {
        args.extend([
            "-e".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            keep_open_script(command),
        ]);
    }
    Ok(LaunchCommand {
        program: path.to_string(),
        args,
        cwd: Some(cwd.to_string()),
    })
}

fn is_app_bundle(path: &str) -> bool {
    path.trim_end_matches(['/', '\\']).ends_with(".app")
}

fn windows_cmd(cwd: &str, command: Option<&str>) -> LaunchCommand {
    let mut args = vec![
        "/C".to_string(),
        "start".to_string(),
        String::new(),
        "cmd".to_string(),
        "/K".to_string(),
    ];
    if let Some(command) = command {
        args.push(command.to_string());
    }
    LaunchCommand {
        program: "cmd".to_string(),
        args,
        cwd: Some(cwd.to_string()),
    }
}

fn osascript(script: &str) -> LaunchCommand {
    LaunchCommand {
        program: "osascript".to_string(),
        args: vec!["-e".to_string(), script.to_string()],
        cwd: None,
    }
}

fn terminal_app_script(cwd: &str, command: Option<&str>) -> String {
    let shell = shell_line(cwd, command);
    let escaped = applescript_escape(&shell);
    format!("tell application \"Terminal\"\nactivate\ndo script \"{escaped}\"\nend tell")
}

fn iterm_script(cwd: &str, command: Option<&str>) -> String {
    let shell = shell_line(cwd, command);
    let escaped = applescript_escape(&shell);
    format!(
        "tell application \"iTerm\"\nactivate\ncreate window with default profile\ntell current session of current window\nwrite text \"{escaped}\"\nend tell\nend tell"
    )
}

fn shell_line(cwd: &str, command: Option<&str>) -> String {
    match command {
        Some(command) => format!("cd {} && {command}", shell_single_quote(cwd)),
        None => format!("cd {}", shell_single_quote(cwd)),
    }
}

fn keep_open_script(command: &str) -> String {
    format!("{command}; exec \"${{SHELL:-sh}}\"")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn applescript_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn spawn_launch(spec: &LaunchCommand) -> Result<(), LaunchError> {
    let mut command = std::process::Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| LaunchError::Failed(format!("open {} failed: {error}", spec.program)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(os: HostOs, bundles: &[&str], binaries: &[(&str, &str)]) -> TerminalHost {
        TerminalHost {
            os,
            bundles: bundles.iter().map(|bundle| (*bundle).to_string()).collect(),
            binaries: binaries
                .iter()
                .map(|(name, path)| ((*name).to_string(), (*path).to_string()))
                .collect(),
        }
    }

    #[test]
    fn macos_lists_installed_apps_and_a_saved_missing_one() {
        let machine = host(HostOs::Macos, &["Ghostty.app"], &[]);
        let listed = available_terminal_apps_for(None, &machine);
        let ids: Vec<_> = listed
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        assert_eq!(ids, ["system", "ghostty", "custom"]);
        assert!(listed.options[1].exists);
        assert_eq!(listed.options[1].label.as_deref(), Some("Ghostty"));
        assert_eq!(listed.options[1].value.as_deref(), Some("ghostty"));

        let saved = available_terminal_apps_for(Some("iterm"), &machine);
        let iterm = saved
            .options
            .iter()
            .find(|option| option.id == "iterm")
            .unwrap();
        assert!(!iterm.exists);
        assert_eq!(
            saved
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["system", "ghostty", "iterm", "custom"]
        );
    }

    #[test]
    fn linux_hides_macos_only_apps() {
        let machine = host(HostOs::Linux, &[], &[("ghostty", "/usr/bin/ghostty")]);
        let listed = available_terminal_apps_for(None, &machine);
        let ids: Vec<_> = listed
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        assert_eq!(ids, ["system", "ghostty", "custom"]);
    }

    #[test]
    fn ghostty_on_macos_uses_open_equals_args() {
        let machine = host(HostOs::Macos, &["Ghostty.app"], &[]);
        let spec = plan_launch(Some("ghostty"), Some("echo hi"), "/work", &machine).unwrap();
        assert_eq!(spec.program, "open");
        assert_eq!(&spec.args[..3], ["-na", "Ghostty", "--args"]);
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "--working-directory=/work"));
        let command = spec
            .args
            .iter()
            .find(|arg| arg.starts_with("--command="))
            .unwrap();
        assert!(command.starts_with("--command=sh -c "));
        assert!(command.contains("echo hi"));
        assert!(command.contains("exec"));
        assert!(spec.cwd.is_none());
    }

    #[test]
    fn ghostty_on_linux_disables_gtk_single_instance() {
        let machine = host(HostOs::Linux, &[], &[("ghostty", "/usr/bin/ghostty")]);
        let spec = plan_launch(Some("ghostty"), None, "/work", &machine).unwrap();
        assert_eq!(spec.program, "/usr/bin/ghostty");
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "--gtk-single-instance=false"));
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "--working-directory=/work"));
        assert!(spec.args.iter().all(|arg| !arg.starts_with("--command=")));
    }

    #[test]
    fn missing_known_app_is_an_error() {
        let machine = host(HostOs::Macos, &[], &[]);
        let error = plan_launch(Some("ghostty"), Some("true"), "/work", &machine).unwrap_err();
        assert_eq!(
            error,
            LaunchError::Failed("Ghostty is not installed".into())
        );
    }

    #[test]
    fn system_macos_still_types_into_terminal_app() {
        let machine = host(HostOs::Macos, &[], &[]);
        let spec = plan_launch(Some("system"), Some("echo hi"), "/work/it's", &machine).unwrap();
        assert_eq!(spec.program, "osascript");
        let script = &spec.args[1];
        assert!(script.contains("tell application \"Terminal\""));
        assert!(script.contains("echo hi"));
        // AppleScript doubles the backslash that bash uses to escape the quote,
        // so the script text contains two backslashes. A normal Rust string
        // would swallow one of them as `\'`.
        assert!(script.contains(r"cd '/work/it'\\''s'"));
    }

    #[test]
    fn system_windows_stays_on_cmd_and_wt_is_explicit() {
        let machine = host(HostOs::Windows, &[], &[("wt.exe", "C:\\wt.exe")]);
        let system = plan_launch(None, Some("echo hi"), "C:\\work", &machine).unwrap();
        assert_eq!(system.program, "cmd");
        assert_eq!(system.args, ["/C", "start", "", "cmd", "/K", "echo hi"]);
        let wt = plan_launch(
            Some("windows-terminal"),
            Some("echo hi"),
            "C:\\work",
            &machine,
        )
        .unwrap();
        assert_eq!(wt.program, "C:\\wt.exe");
        assert_eq!(wt.args, ["-d", "C:\\work", "cmd", "/K", "echo hi"]);
    }

    #[test]
    fn warp_opens_a_directory_and_refuses_a_command() {
        let machine = host(HostOs::Macos, &["Warp.app"], &[]);
        let spec = plan_launch(Some("warp"), None, "/work", &machine).unwrap();
        assert_eq!(spec.args, ["-na", "Warp", "/work"]);
        let error = plan_launch(Some("warp"), Some("echo hi"), "/work", &machine).unwrap_err();
        match error {
            LaunchError::InvalidInput(message) => {
                assert!(message.contains("Warp"));
                assert!(message.contains("Ghostty"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn custom_bundle_refuses_a_command_and_an_executable_keeps_the_shell() {
        let machine = host(HostOs::Macos, &[], &[]);
        let error = plan_launch(
            Some("/Applications/Foo.app"),
            Some("echo hi"),
            "/work",
            &machine,
        )
        .unwrap_err();
        assert!(matches!(error, LaunchError::InvalidInput(_)));
        let opened = plan_launch(Some("/Applications/Foo.app"), None, "/work", &machine).unwrap();
        assert_eq!(opened.args, ["-na", "/Applications/Foo.app", "/work"]);
        let binary = plan_launch(Some("/opt/term"), Some("echo hi"), "/work", &machine).unwrap();
        assert_eq!(binary.program, "/opt/term");
        assert_eq!(binary.args[0], "-e");
        assert!(binary.args[3].contains("echo hi"));
        assert_eq!(binary.cwd.as_deref(), Some("/work"));
    }

    #[test]
    fn newlines_are_rejected_and_linux_system_uses_the_first_installed_emulator() {
        let machine = host(
            HostOs::Linux,
            &[],
            &[
                ("gnome-terminal", "/usr/bin/gnome-terminal"),
                ("xterm", "/usr/bin/xterm"),
            ],
        );
        let error = plan_launch(None, Some("echo\nhi"), "/work", &machine).unwrap_err();
        assert!(matches!(error, LaunchError::InvalidInput(_)));
        let spec = plan_launch(None, Some("echo hi"), "/work", &machine).unwrap();
        assert_eq!(spec.program, "/usr/bin/gnome-terminal");
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "--working-directory=/work"));

        let empty = host(HostOs::Linux, &[], &[]);
        let missing = plan_launch(None, Some("echo hi"), "/work", &empty).unwrap_err();
        assert!(missing.message().contains("x-terminal-emulator"));
    }

    #[test]
    fn iterm_writes_into_a_new_window() {
        let machine = host(HostOs::Macos, &["iTerm.app"], &[]);
        let spec = plan_launch(Some("iterm"), Some("echo hi"), "/work", &machine).unwrap();
        assert_eq!(spec.program, "osascript");
        assert!(spec.args[1].contains("tell application \"iTerm\""));
        assert!(spec.args[1].contains("write text"));
        assert!(spec.args[1].contains("echo hi"));
    }
}

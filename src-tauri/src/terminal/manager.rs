use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use super::error::TerminalError;
#[cfg(target_os = "windows")]
use super::shell_flavor::ShellFamily;
use super::types::{TerminalEvent, TerminalInfo, TerminalSnapshot};
use crate::browser::service_url::ServiceScanner;
use crate::browser::services::ServiceWatch;
use crate::browser::types::ServiceSource;
use crate::web::event_bridge::EventEmitter;

/// How much recent PTY output a terminal keeps for re-attaching viewers. Sized
/// to cover a screenful of `ls -R` or a compile log, not a whole session: this
/// is a "pick the pane back up where you left it" buffer, not a transcript.
const SCROLLBACK_MAX_CHARS: usize = 128 * 1024;

/// Recent output of one terminal, kept so a viewer that mounts after the spawn
/// (or re-mounts after its host view was unmounted — a canvas terminal card
/// crossing a route switch) can redraw instead of showing a blank pane while
/// the shell sits there waiting at a prompt it already printed.
///
/// Whole chunks are evicted from the front rather than characters: the buffer
/// holds raw terminal bytes, and slicing one at an arbitrary offset would cut
/// an escape sequence in half and paint the replay with whatever the truncated
/// tail happens to mean.
#[derive(Default)]
struct Scrollback {
    chunks: VecDeque<String>,
    chars: usize,
    /// Chunks appended since the terminal spawned — the monotonic cursor the
    /// output events carry. Never reset, so it stays comparable across a
    /// re-attach.
    seq: u64,
}

impl Scrollback {
    /// Record a chunk and return the seq it was assigned (= the seq an event
    /// carrying this chunk must report).
    fn append(&mut self, data: &str) -> u64 {
        self.seq += 1;
        self.chars += data.chars().count();
        self.chunks.push_back(data.to_string());
        while self.chars > SCROLLBACK_MAX_CHARS && self.chunks.len() > 1 {
            if let Some(old) = self.chunks.pop_front() {
                self.chars = self.chars.saturating_sub(old.chars().count());
            }
        }
        self.seq
    }

    fn read(&self) -> (String, u64) {
        (self.chunks.iter().cloned().collect(), self.seq)
    }
}

#[derive(Default)]
struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        let mut combined = std::mem::take(&mut self.pending);
        combined.extend_from_slice(bytes);
        let mut output = String::new();
        let mut offset = 0;
        while offset < combined.len() {
            match std::str::from_utf8(&combined[offset..]) {
                Ok(valid) => {
                    output.push_str(valid);
                    break;
                }
                Err(error) => {
                    let end = offset + error.valid_up_to();
                    output.push_str(std::str::from_utf8(&combined[offset..end]).unwrap_or(""));
                    offset = end;
                    if let Some(invalid_len) = error.error_len() {
                        output.push('\u{fffd}');
                        offset += invalid_len;
                    } else {
                        self.pending.extend_from_slice(&combined[offset..]);
                        break;
                    }
                }
            }
        }
        output
    }

    fn finish(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned()
    }
}

struct TerminalInstance {
    instance_id: uuid::Uuid,
    write_tx: mpsc::Sender<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
    title: String,
    owner_window_label: String,
    /// Shared with this terminal's reader thread — the thread appends, viewers
    /// read. Held behind its own lock rather than the map's so a chunk of
    /// output never waits on a write / resize / list call.
    scrollback: Arc<Mutex<Scrollback>>,
    /// Temp files (credential store + helper script) to clean up on exit.
    temp_files: Vec<std::path::PathBuf>,
    on_exit: Option<Box<dyn FnOnce() + Send>>,
}

pub struct TerminalManager {
    terminals: Arc<Mutex<HashMap<String, TerminalInstance>>>,
}

/// Lock the terminal table, ignoring the poison flag.
///
/// Every reader and writer of this map goes through here, so the policy is one
/// decision rather than a per-call-site one.
///
/// The flag guards nothing here. A `HashMap<String, TerminalInstance>` cannot be
/// observed half-updated: a panic between two of its mutations leaves entries
/// that are each individually whole, and the callers below all re-read the map
/// rather than caching a view of it. What the flag WOULD do is convert an
/// unrelated earlier panic — one tokio swallowed, in a task that touched a
/// terminal — into a permanent failure of every later terminal operation.
///
/// Two of those operations make that fatal rather than merely broken.
/// [`TerminalManager::kill_by_owner_window`] runs inside Tauri's
/// `on_window_event` and [`TerminalManager::kill_all`] inside
/// `RunEvent::ExitRequested`: both on the main thread, inside the platform
/// event loop, where a panic unwinds across an `extern "system"` boundary and
/// Rust turns that into an immediate `abort` (Windows reports it as
/// `0xc0000409`). So a poisoned mutex would take the whole process down at the
/// next window close. The reader thread's own removal is the mirror case — it
/// would leak the entry and its temp files for the rest of the process.
///
/// Matches the form already used in `office_watch` and `background_watch`.
fn lock_terminals(
    terminals: &Mutex<HashMap<String, TerminalInstance>>,
) -> std::sync::MutexGuard<'_, HashMap<String, TerminalInstance>> {
    terminals.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn resolve_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            let trimmed = shell.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Ok(comspec) = std::env::var("COMSPEC") {
            let trimmed = comspec.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        "cmd.exe".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            let trimmed = shell.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        // Try common shells in order of preference
        for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
            if std::path::Path::new(candidate).exists() {
                return candidate.to_string();
            }
        }
        "/bin/sh".to_string()
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy)]
enum WindowsShellFlavor {
    Cmd,
    PowerShell,
    Posix,
}

#[cfg(target_os = "windows")]
fn detect_windows_shell_flavor(shell: &str) -> WindowsShellFlavor {
    // Shares one classifier with the ACP terminal runtime so the two can't
    // drift on what `COMSPEC` means — see `terminal::shell_flavor`.
    match crate::terminal::shell_flavor::classify_shell_family(shell) {
        ShellFamily::PowerShell => WindowsShellFlavor::PowerShell,
        ShellFamily::Posix => WindowsShellFlavor::Posix,
        ShellFamily::Cmd => WindowsShellFlavor::Cmd,
    }
}

/// POSIX-side shell flavor. We only inject the `-l -i` login/interactive
/// flags and the `eval "$CODEG_CMD"` wrapping for shells we know speak that
/// dialect — passing those to nu / xonsh / elvish / pwsh would cause spawn
/// failures or weird behavior. Unknown shells get raw spawn (no flags) and,
/// when an `initial_command` is requested, a plain `-c <command>` (the
/// closest thing to a universal "run this and exit" convention).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Copy)]
enum PosixShellFlavor {
    /// bash / zsh / sh / dash / ksh / ash / mksh / busybox / fish — accept
    /// `-l -i` and the `eval "$VAR"` pattern.
    BashLike,
    /// Anything else. Don't assume POSIX flag conventions.
    Unknown,
}

#[cfg(not(target_os = "windows"))]
fn detect_posix_shell_flavor(shell: &str) -> PosixShellFlavor {
    let name = crate::terminal::shell_flavor::shell_basename(shell);

    if matches!(
        name.as_str(),
        "bash" | "zsh" | "sh" | "dash" | "ksh" | "ash" | "mksh" | "busybox" | "fish"
    ) {
        PosixShellFlavor::BashLike
    } else {
        PosixShellFlavor::Unknown
    }
}

fn configure_shell_command(cmd: &mut CommandBuilder, shell: &str, initial_command: Option<&str>) {
    #[cfg(target_os = "windows")]
    {
        // Force UTF-8 output for all Windows shell flavors
        cmd.env("PYTHONUTF8", "1");
        cmd.env("PYTHONIOENCODING", "utf-8");

        match detect_windows_shell_flavor(shell) {
            WindowsShellFlavor::Cmd => {
                if let Some(command) = initial_command {
                    cmd.env("CODEG_CMD", command);
                    // Set UTF-8 code page before running the actual command
                    cmd.args(["/D", "/S", "/C", "chcp 65001 >nul & %CODEG_CMD%"]);
                } else {
                    // /K runs the command then stays open for interactive use
                    cmd.args(["/D", "/S", "/K", "chcp 65001 >nul"]);
                }
            }
            WindowsShellFlavor::PowerShell => {
                if let Some(command) = initial_command {
                    cmd.env("CODEG_CMD", command);
                    cmd.args([
                        "-NoLogo",
                        "-NoProfile",
                        "-Command",
                        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; Invoke-Expression $env:CODEG_CMD",
                    ]);
                } else {
                    // -NoExit runs the command then stays open for interactive use
                    cmd.args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NoExit",
                        "-Command",
                        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $host.UI.RawUI.WindowTitle = 'codeg'",
                    ]);
                }
            }
            WindowsShellFlavor::Posix => {
                cmd.env("TERM", "xterm-256color");
                cmd.env("COLORTERM", "truecolor");
                cmd.env("TERM_PROGRAM", "codeg");
                cmd.env("LANG", "C.UTF-8");
                if let Some(command) = initial_command {
                    cmd.env("CODEG_CMD", command);
                    cmd.args(["-l", "-i", "-c", "eval \"$CODEG_CMD\""]);
                } else {
                    cmd.args(["-l", "-i"]);
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // GUI app environments often miss TERM; force a sane terminal type so
        // readline/zle can redraw lines correctly (history navigation, etc.).
        // Locale env (LANG/LC_ALL) is intentionally NOT injected — interactive
        // PTYs should respect whatever the user's shell rc files set up.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "codeg");

        match detect_posix_shell_flavor(shell) {
            PosixShellFlavor::BashLike => {
                if let Some(command) = initial_command {
                    // Indirection via env var avoids quoting/escaping bugs
                    // for arbitrary commands (and keeps long commands off
                    // argv for readability in `ps`).
                    cmd.env("CODEG_CMD", command);
                    cmd.args(["-l", "-i", "-c", "eval \"$CODEG_CMD\""]);
                } else {
                    cmd.args(["-l", "-i"]);
                }
            }
            PosixShellFlavor::Unknown => {
                // No-flag spawn for nu/xonsh/elvish/pwsh on Linux/etc. Most
                // modern shells default to interactive when stdin is a TTY,
                // so we get a usable session without guessing flag syntax.
                if let Some(command) = initial_command {
                    cmd.args(["-c", command]);
                }
            }
        }
    }
}

/// Options for spawning a new terminal session.
pub struct SpawnOptions {
    pub terminal_id: String,
    pub working_dir: String,
    pub owner_window_label: String,
    pub shell: Option<String>,
    pub initial_command: Option<String>,
    pub extra_env: Option<HashMap<String, String>>,
    pub temp_files: Vec<std::path::PathBuf>,
    pub on_exit: Option<Box<dyn FnOnce() + Send>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns a shallow clone sharing the same underlying terminal map.
    pub fn clone_ref(&self) -> Self {
        Self {
            terminals: self.terminals.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_id(
        &self,
        opts: SpawnOptions,
        emitter: EventEmitter,
    ) -> Result<String, TerminalError> {
        // Reject duplicate IDs to prevent orphaning an existing PTY process.
        {
            let terminals = lock_terminals(&self.terminals);
            if terminals.contains_key(&opts.terminal_id) {
                return Err(TerminalError::SpawnFailed(format!(
                    "terminal id '{}' already exists",
                    opts.terminal_id
                )));
            }
        }

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

        let shell = opts
            .shell
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(resolve_shell);
        let mut cmd = CommandBuilder::new(&shell);
        configure_shell_command(&mut cmd, &shell, opts.initial_command.as_deref());
        cmd.cwd(&opts.working_dir);

        // Inject extra environment variables (e.g. git credential helper config).
        // Match ACP's launch convention: an empty value removes an inherited
        // variable instead of leaving an empty variable visible to the CLI.
        if let Some(env) = &opts.extra_env {
            for (key, value) in env {
                if value.is_empty() {
                    cmd.env_remove(key);
                } else {
                    cmd.env(key, value);
                }
            }
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| TerminalError::SpawnFailed(e.to_string()))?;

        let terminal_id = opts.terminal_id;
        let instance_id = uuid::Uuid::new_v4();
        // Boundary-, length-, and NUL-safe prefix for the PTY thread names; see
        // `thread_name_prefix`. `terminal_id` is caller-supplied.
        let short_id = thread_name_prefix(&terminal_id);

        let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
        let scrollback = Arc::new(Mutex::new(Scrollback::default()));

        let owner_window = opts.owner_window_label.clone();
        let instance = TerminalInstance {
            instance_id,
            write_tx,
            master: pair.master,
            _child: child,
            title: "Terminal".to_string(),
            owner_window_label: opts.owner_window_label,
            scrollback: scrollback.clone(),
            temp_files: opts.temp_files,
            on_exit: opts.on_exit,
        };

        lock_terminals(&self.terminals).insert(terminal_id.clone(), instance);

        // Named writer thread
        if let Err(error) = std::thread::Builder::new()
            .name(format!("pty-writer-{short_id}"))
            .spawn(move || {
                write_loop(writer, write_rx);
            })
        {
            let _ = self.kill(&terminal_id);
            return Err(TerminalError::SpawnFailed(error.to_string()));
        }

        // Named reader thread — emits per-terminal events
        let id_for_reader = terminal_id.clone();
        let terminals_ref = self.terminals.clone();
        // The watch that notices a dev server announcing itself in this
        // terminal's output. Built here because this is where the owning
        // window is known; it probes and emits on threads of its own, so the
        // reader below never waits on a socket.
        let watch = ServiceWatch::new(emitter.clone(), owner_window, ServiceSource::Terminal);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("pty-reader-{short_id}"))
            .spawn(move || {
                read_loop(
                    reader,
                    id_for_reader,
                    instance_id,
                    &emitter,
                    &terminals_ref,
                    &scrollback,
                    &watch,
                );
            })
        {
            let _ = self.kill(&terminal_id);
            return Err(TerminalError::SpawnFailed(error.to_string()));
        }

        Ok(terminal_id)
    }

    pub fn write(&self, terminal_id: &str, data: &[u8]) -> Result<(), TerminalError> {
        let terminals = lock_terminals(&self.terminals);
        let instance = terminals
            .get(terminal_id)
            .ok_or_else(|| TerminalError::NotFound(terminal_id.to_string()))?;
        instance
            .write_tx
            .send(data.to_vec())
            .map_err(|e| TerminalError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let terminals = lock_terminals(&self.terminals);
        let instance = terminals
            .get(terminal_id)
            .ok_or_else(|| TerminalError::NotFound(terminal_id.to_string()))?;
        instance
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::ResizeFailed(e.to_string()))?;
        Ok(())
    }

    /// Recent output of a live terminal, for a viewer attaching to a PTY that
    /// was spawned before it mounted.
    ///
    /// Never an error: "no such terminal" is the answer that tells the caller
    /// to spawn one instead, and a caller that has to distinguish that from a
    /// transport failure would have to parse the error string to do it. Note
    /// the map lock is taken only to find the instance — the buffer has its own,
    /// so a large scrollback is never copied while output is blocked.
    pub fn snapshot(&self, terminal_id: &str) -> TerminalSnapshot {
        let buffer = {
            let terminals = lock_terminals(&self.terminals);
            match terminals.get(terminal_id) {
                Some(instance) => instance.scrollback.clone(),
                None => {
                    return TerminalSnapshot {
                        alive: false,
                        data: String::new(),
                        seq: 0,
                    }
                }
            }
        };
        let (data, seq) = buffer
            .lock()
            .map(|s| s.read())
            .unwrap_or_else(|_| (String::new(), 0));
        TerminalSnapshot {
            alive: true,
            data,
            seq,
        }
    }

    pub fn kill(&self, terminal_id: &str) -> Result<(), TerminalError> {
        let mut instance = lock_terminals(&self.terminals)
            .remove(terminal_id)
            .ok_or_else(|| TerminalError::NotFound(terminal_id.to_string()))?;
        terminate_terminal(&mut instance);
        Ok(())
    }

    /// THE liveness gate. Drops terminals whose child has exited and returns
    /// their ids so the caller can announce them.
    ///
    /// Every "is this terminal still running" question routes through here.
    /// A second copy of the `try_wait` logic is how a close confirmation ends
    /// up claiming three terminals will die while the kill that follows
    /// reports two.
    ///
    /// Reaped instances get their temp files removed. Dropping a
    /// `TerminalInstance` releases the PTY but not the credential store and
    /// helper script on disk — only [`terminate_terminal`] did that, and it is
    /// not on this path.
    fn reap_exited(terminals: &mut HashMap<String, TerminalInstance>) -> Vec<String> {
        let mut exited_terminal_ids: Vec<String> = Vec::new();

        // Windows ConPTY may not always surface EOF promptly; reconcile exited
        // child processes here so frontend running-state can recover reliably.
        for (id, instance) in terminals.iter_mut() {
            match instance._child.try_wait() {
                Ok(Some(_)) => exited_terminal_ids.push(id.clone()),
                Ok(None) => {}
                Err(err) => {
                    tracing::error!(
                        "[TERM] failed to query child status for terminal {}: {}",
                        id,
                        err
                    );
                    exited_terminal_ids.push(id.clone());
                }
            }
        }

        for terminal_id in &exited_terminal_ids {
            if let Some(mut instance) = terminals.remove(terminal_id) {
                finish_terminal(&mut instance);
            }
        }

        exited_terminal_ids
    }

    pub fn list_with_exit_check(&self, emitter: Option<&EventEmitter>) -> Vec<TerminalInfo> {
        let mut terminals = lock_terminals(&self.terminals);
        let exited_terminal_ids = Self::reap_exited(&mut terminals);

        let infos = terminals
            .iter()
            .map(|(id, inst)| TerminalInfo {
                id: id.clone(),
                title: inst.title.clone(),
            })
            .collect();

        if let Some(emitter) = emitter {
            for terminal_id in exited_terminal_ids {
                emit_terminal_exit_event(emitter, &terminal_id);
            }
        }

        drop(terminals);

        infos
    }

    /// How many of `owner_window_label`'s terminals are still running.
    ///
    /// Shares [`Self::reap_exited`] with `list_with_exit_check` so a finished
    /// build is never counted as work in progress — the close confirmation
    /// this feeds is ignored the moment it cries wolf.
    pub fn count_live_by_owner_window(
        &self,
        owner_window_label: &str,
        emitter: Option<&EventEmitter>,
    ) -> usize {
        let mut terminals = lock_terminals(&self.terminals);
        let exited_terminal_ids = Self::reap_exited(&mut terminals);

        let live = terminals
            .values()
            .filter(|instance| instance.owner_window_label == owner_window_label)
            .count();

        if let Some(emitter) = emitter {
            for terminal_id in exited_terminal_ids {
                emit_terminal_exit_event(emitter, &terminal_id);
            }
        }

        drop(terminals);

        live
    }

    pub fn kill_by_owner_window(&self, owner_window_label: &str) -> usize {
        let mut instances = {
            let mut terminals = lock_terminals(&self.terminals);
            let ids: Vec<String> = terminals
                .iter()
                .filter_map(|(id, instance)| {
                    if instance.owner_window_label == owner_window_label {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();

            let mut removed = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(instance) = terminals.remove(&id) {
                    removed.push(instance);
                }
            }
            removed
        };

        let killed = instances.len();
        for instance in &mut instances {
            terminate_terminal(instance);
        }
        killed
    }

    /// Poison-tolerant for the reason given on [`Self::kill_by_owner_window`]:
    /// the quit path runs inside `RunEvent::ExitRequested` on the main thread.
    pub fn kill_all(&self) -> usize {
        let mut instances: Vec<TerminalInstance> = {
            let mut terminals = lock_terminals(&self.terminals);
            terminals.drain().map(|(_, inst)| inst).collect()
        };
        let killed = instances.len();
        for instance in &mut instances {
            terminate_terminal(instance);
        }
        tracing::info!("[TERM] kill_all killed_terminals={}", killed);
        killed
    }
}

fn terminate_terminal(instance: &mut TerminalInstance) {
    if instance.on_exit.is_some() && matches!(instance._child.try_wait(), Ok(None)) {
        if let Some(pid) = instance._child.process_id() {
            let config = kill_tree::Config {
                signal: "SIGKILL".to_string(),
                ..Default::default()
            };
            if let Err(error) = kill_tree::blocking::kill_tree_with_config(pid, &config) {
                tracing::warn!("[TERM] failed to stop agent process tree {pid}: {error}");
            }
        }
    }
    let _ = instance._child.kill();
    let _ = instance._child.wait();
    finish_terminal(instance);
}

fn finish_terminal(instance: &mut TerminalInstance) {
    cleanup_temp_files(&mut instance.temp_files);
    if let Some(on_exit) = instance.on_exit.take() {
        on_exit();
    }
}

fn cleanup_temp_files(files: &mut Vec<std::path::PathBuf>) {
    for path in files.drain(..) {
        let _ = super::agent_launch::cleanup_agent_temp_path(&path);
    }
}

fn write_loop(mut writer: Box<dyn Write + Send>, rx: mpsc::Receiver<Vec<u8>>) {
    while let Ok(data) = rx.recv() {
        if writer.write_all(&data).is_err() {
            break;
        }
        while let Ok(more) = rx.try_recv() {
            if writer.write_all(&more).is_err() {
                return;
            }
        }
        if writer.flush().is_err() {
            break;
        }
    }
}

fn read_loop(
    mut reader: Box<dyn Read + Send>,
    terminal_id: String,
    instance_id: uuid::Uuid,
    emitter: &EventEmitter,
    terminals: &Arc<Mutex<HashMap<String, TerminalInstance>>>,
    scrollback: &Arc<Mutex<Scrollback>>,
    watch: &ServiceWatch,
) {
    let output_event = format!("terminal://output/{}", terminal_id);
    let mut buf = [0u8; 8192];
    // Thread-confined, so the carry buffer and the rate limit need no lock.
    let mut services = ServiceScanner::new();
    let mut decoder = Utf8StreamDecoder::default();

    let mut publish = |data: String| {
        if data.is_empty() {
            return;
        }
        let seq = scrollback
            .lock()
            .map(|mut scrollback| scrollback.append(&data))
            .unwrap_or_default();
        watch.feed(&mut services, &data, &terminal_id);
        let event = TerminalEvent {
            terminal_id: terminal_id.clone(),
            data,
            seq,
        };
        crate::web::event_bridge::emit_event(emitter, &output_event, event);
    };

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // A multibyte character may straddle PTY reads. Retain only
                // its incomplete suffix, then publish complete text in order.
                publish(decoder.push(&buf[..n]));
            }
            Err(_) => break,
        }
    }
    publish(decoder.finish());

    // Terminal exited — remove from map and clean up temp files. Poison-tolerant
    // like the scrollback lock above: this runs on the long-lived `pty-reader-*`
    // thread, and refusing the removal would leak the entry and its temp files
    // for the rest of the process.
    {
        let mut map = lock_terminals(terminals);
        if map
            .get(&terminal_id)
            .is_some_and(|instance| instance.instance_id == instance_id)
        {
            if let Some(mut instance) = map.remove(&terminal_id) {
                finish_terminal(&mut instance);
            }
            // Keep the id unavailable to a replacement until its old exit
            // notification has been published.
            emit_terminal_exit_event(emitter, &terminal_id);
        }
    }
}

fn emit_terminal_exit_event(emitter: &EventEmitter, terminal_id: &str) {
    let exit_event = format!("terminal://exit/{}", terminal_id);
    let event = TerminalEvent {
        terminal_id: terminal_id.to_string(),
        data: String::new(),
        seq: 0,
    };
    crate::web::event_bridge::emit_event(emitter, &exit_event, event.clone());
}

/// Build a thread-name-safe short prefix from a caller-supplied `terminal_id`.
///
/// `terminal_id` arrives from the frontend (Tauri/web spawn paths) and is not
/// guaranteed to be ASCII, at least 8 bytes long, or free of NUL bytes. Naive
/// `&terminal_id[..8]` panics on a short id or a multibyte char straddling
/// byte 8, and `std::thread::Builder::spawn` panics if the resulting thread
/// name contains an interior NUL. Take the first 8 Unicode scalar values
/// (boundary- and length-safe) and replace NUL with `_`.
fn thread_name_prefix(terminal_id: &str) -> String {
    terminal_id
        .chars()
        .take(8)
        .map(|c| if c == '\0' { '_' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{thread_name_prefix, Scrollback, Utf8StreamDecoder, SCROLLBACK_MAX_CHARS};
    #[cfg(not(target_os = "windows"))]
    use super::{Arc, EventEmitter, SpawnOptions, TerminalManager};

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn stale_reader_cannot_remove_a_reused_terminal_id() {
        use super::read_loop;
        use crate::browser::services::ServiceWatch;
        use crate::browser::types::ServiceSource;
        use std::io::Cursor;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        let manager = TerminalManager::new();
        let emitter = EventEmitter::Noop;
        let exits = Arc::new(AtomicUsize::new(0));
        let exit_counter = exits.clone();
        manager
            .spawn_with_id(
                SpawnOptions {
                    terminal_id: "reused-id".into(),
                    working_dir: std::env::temp_dir().to_string_lossy().into_owned(),
                    owner_window_label: "main".into(),
                    shell: Some("/bin/sh".into()),
                    initial_command: None,
                    extra_env: None,
                    temp_files: Vec::new(),
                    on_exit: Some(Box::new(move || {
                        exit_counter.fetch_add(1, Ordering::SeqCst);
                    })),
                },
                emitter.clone(),
            )
            .unwrap();
        let current_generation = super::lock_terminals(&manager.terminals)
            .get("reused-id")
            .unwrap()
            .instance_id;
        let stale_generation = uuid::Uuid::new_v4();
        assert_ne!(current_generation, stale_generation);
        let scrollback = Arc::new(Mutex::new(Scrollback::default()));
        let watch = ServiceWatch::new(emitter.clone(), "main".into(), ServiceSource::Terminal);
        read_loop(
            Box::new(Cursor::new(Vec::<u8>::new())),
            "reused-id".into(),
            stale_generation,
            &emitter,
            &manager.terminals,
            &scrollback,
            &watch,
        );
        assert!(manager.snapshot("reused-id").alive);
        assert_eq!(exits.load(Ordering::SeqCst), 0);
        manager.kill("reused-id").unwrap();
        assert_eq!(exits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn utf8_decoder_preserves_characters_split_across_reads() {
        let mut decoder = Utf8StreamDecoder::default();
        let text = "中文🙂";
        let bytes = text.as_bytes();
        assert_eq!(decoder.push(&bytes[..2]), "");
        assert_eq!(decoder.push(&bytes[2..5]), "中");
        assert_eq!(decoder.push(&bytes[5..]), "文🙂");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn scrollback_seq_counts_every_chunk_and_never_rewinds() {
        // The seq is what a re-attaching client uses to tell "already in the
        // snapshot" from "arrived after it". A repeated or reset value would
        // make the replay either double-print or swallow output.
        let mut buffer = Scrollback::default();
        assert_eq!(buffer.append("a"), 1);
        assert_eq!(buffer.append("b"), 2);
        assert_eq!(buffer.read(), ("ab".to_string(), 2));
    }

    #[test]
    fn scrollback_evicts_whole_chunks_and_keeps_counting() {
        // Whole chunks, never a character slice: the buffer holds raw terminal
        // bytes, and cutting one mid-escape paints the replay with whatever the
        // truncated tail happens to mean.
        let mut buffer = Scrollback::default();
        let chunk = "x".repeat(SCROLLBACK_MAX_CHARS / 2 + 1);
        buffer.append(&chunk);
        buffer.append(&chunk);
        let seq = buffer.append("tail");
        let (data, read_seq) = buffer.read();
        assert_eq!(read_seq, seq, "eviction must not rewind the cursor");
        assert!(data.ends_with("tail"));
        assert!(
            data.chars().count() <= SCROLLBACK_MAX_CHARS,
            "kept {} chars",
            data.chars().count()
        );
    }

    #[test]
    fn scrollback_keeps_the_last_chunk_even_when_it_alone_is_too_big() {
        // A single chunk over the cap must not evict itself into an empty
        // buffer — the newest output is the part worth keeping.
        let mut buffer = Scrollback::default();
        let huge = "y".repeat(SCROLLBACK_MAX_CHARS * 2);
        buffer.append(&huge);
        let (data, seq) = buffer.read();
        assert_eq!(seq, 1);
        assert_eq!(data.chars().count(), huge.chars().count());
    }

    #[test]
    fn a_missing_terminal_reports_not_alive_rather_than_failing() {
        // "No such terminal" is the answer that tells a caller to spawn one;
        // an error would force it to parse a string to tell that apart from a
        // transport failure.
        let manager = super::TerminalManager::new();
        let snapshot = manager.snapshot("nope");
        assert!(!snapshot.alive);
        assert!(snapshot.data.is_empty());
        assert_eq!(snapshot.seq, 0);
    }

    #[test]
    fn keeps_short_ascii_id() {
        assert_eq!(thread_name_prefix("abc"), "abc");
        assert_eq!(thread_name_prefix(""), "");
    }

    #[test]
    fn truncates_to_first_eight_chars() {
        assert_eq!(thread_name_prefix("0123456789"), "01234567");
    }

    #[test]
    fn is_char_boundary_safe() {
        // '密' occupies bytes 7..10, so `&s[..8]` would slice inside it and
        // panic; taking 8 scalar values keeps the whole char.
        assert_eq!(thread_name_prefix("abcdefg密钥"), "abcdefg密");
    }

    #[test]
    fn sanitizes_interior_nul_so_thread_spawns() {
        assert_eq!(thread_name_prefix("ab\0cd"), "ab_cd");
        // The result must be usable as a real thread name without panicking.
        std::thread::Builder::new()
            .name(thread_name_prefix("ab\0cdefghij"))
            .spawn(|| {})
            .expect("spawn with sanitized name")
            .join()
            .expect("join");
    }

    /// The whole local-server path over a REAL pty: a real shell prints a real
    /// banner, the watch reads it out of the output stream, connects to the
    /// socket, and the frontend's event arrives naming the window that owns
    /// the terminal.
    ///
    /// The pieces have unit tests of their own; what only an end-to-end run
    /// can show is that the watch is wired into the reader at all, and that a
    /// banner survives a real PTY (its line endings, its echo, its shell).
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn a_server_announced_in_a_terminal_reaches_the_frontend() {
        use crate::browser::types::SERVICE_DETECTED_EVENT;
        use crate::web::event_bridge::WebEventBroadcaster;
        use std::time::Duration;

        // Something really listening, so the probe has something to find.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();

        let broadcaster = Arc::new(WebEventBroadcaster::new());
        // Subscribed before the spawn: the broadcaster drops what it sends
        // with no receivers, and a shell prints fast.
        let mut events = broadcaster.subscribe();
        let emitter = EventEmitter::test_web_only(broadcaster);

        let manager = TerminalManager::new();
        manager
            .spawn_with_id(
                SpawnOptions {
                    terminal_id: "svc-e2e".to_string(),
                    working_dir: std::env::temp_dir().to_string_lossy().to_string(),
                    owner_window_label: "main".to_string(),
                    shell: Some("/bin/sh".to_string()),
                    initial_command: Some(format!(
                        "printf '  ➜  Local:   http://127.0.0.1:{port}/\\n'"
                    )),
                    extra_env: None,
                    temp_files: vec![],
                    on_exit: None,
                },
                emitter,
            )
            .expect("spawn");

        let detected = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let event = events.recv().await.expect("event bus");
                if event.channel == SERVICE_DETECTED_EVENT {
                    return event;
                }
            }
        })
        .await
        .expect("the service event");

        let payload = detected.payload;
        assert_eq!(payload["origin"], format!("http://127.0.0.1:{port}"));
        assert_eq!(payload["url"], format!("http://127.0.0.1:{port}/"));
        assert_eq!(payload["authority"], format!("127.0.0.1:{port}"));
        assert_eq!(payload["ownerWindow"], "main");
        assert_eq!(payload["source"], "terminal");
        assert_eq!(payload["terminalId"], "svc-e2e");

        let _ = manager.kill("svc-e2e");
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn empty_extra_env_removes_inherited_values() {
        use crate::web::event_bridge::WebEventBroadcaster;
        use std::collections::HashMap;
        use std::time::Duration;

        let key = format!("CODEG_TERMINAL_ENV_TEST_{}", uuid::Uuid::new_v4());
        std::env::set_var(&key, "inherited-secret");
        let command =
            format!("if env | grep -q '^{key}='; then printf present; else printf absent; fi");
        let broadcaster = Arc::new(WebEventBroadcaster::new());
        let mut events = broadcaster.subscribe();
        let manager = TerminalManager::new();
        manager
            .spawn_with_id(
                SpawnOptions {
                    terminal_id: "empty-env".to_string(),
                    working_dir: std::env::temp_dir().to_string_lossy().to_string(),
                    owner_window_label: "main".to_string(),
                    shell: Some("/bin/sh".to_string()),
                    initial_command: Some(command),
                    extra_env: Some(HashMap::from([(key.clone(), String::new())])),
                    temp_files: vec![],
                    on_exit: None,
                },
                EventEmitter::test_web_only(broadcaster),
            )
            .expect("spawn");

        let output = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.expect("event bus");
                if event.channel == "terminal://output/empty-env" {
                    if let Some(data) = event.payload["data"].as_str() {
                        if !data.is_empty() {
                            break data.to_string();
                        }
                    }
                }
            }
        })
        .await
        .expect("shell output");

        assert!(output.contains("absent"), "output={output:?}");
        let _ = manager.kill("empty-env");
        std::env::remove_var(key);
    }
}

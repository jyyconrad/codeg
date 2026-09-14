// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // When called as a git credential helper, handle it immediately and exit.
    // This avoids starting the full Tauri GUI runtime.
    if std::env::args().any(|a| a == "--credential-helper") {
        // Subprocess mode, before the desktop logging init in `run()`: install a
        // stderr-only subscriber so helper diagnostics aren't dropped, while
        // stdout stays the git credential protocol channel.
        let _log_guard = codeg_lib::logging::init::init_stderr_only();
        codeg_lib::git_credential::run_credential_helper();
        return;
    }

    if let Some(url) = codeg_lib::agent::code_intel::parse_stdio_bridge_url(std::env::args()) {
        let _log_guard = codeg_lib::logging::init::init_mcp();
        if let Err(err) = codeg_lib::agent::code_intel::run_stdio_http_bridge(&url) {
            eprintln!("code-intel MCP stdio connector failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    codeg_lib::run()
}

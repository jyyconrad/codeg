//! Native session collaboration modes (`code` / `plan`). Explore is a subagent, not a mode.

use std::fs;
use std::path::{Path, PathBuf};

use crate::acp::session_state::SessionState;
use crate::acp::types::{AcpEvent, SessionModeInfo, SessionModeStateInfo};
use crate::agent::context::encode_session_cwd;
use crate::paths::codeg_agent_dir;
use crate::web::event_bridge::{emit_with_state, EventEmitter};
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

pub const MODE_CODE: &str = "code";
pub const MODE_PLAN: &str = "plan";
pub const SESSION_MODE_FILE: &str = "session_mode";
pub const PLAN_FILE_NAME: &str = "plan.md";
pub const EXPLORE_FILE_NAME: &str = "explore.md";
pub const EXPLORE_SUMMARY_MAX_BYTES: usize = 2048;

pub const PLAN_MODE_ATTACHMENT: &str = "\
## Session mode: plan\n\
You are in plan mode until a mode change ends it. Do not edit repository source.\n\
Research with read/search tools. Use ask_user_question for decisions that change the plan.\n\
Write the implementation plan with write_plan. When the plan is ready, call exit_plan_mode.\n\
If the user asks you to implement while still in plan mode, update the plan instead of editing source.\n\
update_plan is not available in plan mode.\
";

pub const EXPLORE_PREAMBLE_INTRO: &str = concat!(
    "You are a read-only explore agent. Search and analyze the codebase; you cannot edit repository source or run bash.\n",
    "1. Investigate using read_file, glob, grep, recall, and skill. Prefer glob and grep before reading.\n",
    "2. Write the full report with write_explore_report (evidence, paths, and open questions belong in that file). Use absolute paths.\n",
    "3. Your last message must be an execution summary for the parent agent: what you searched, the main findings in one or two sentences, which sections the report contains, and what you did not check. Do not paste the report body.\n",
    "4. After the summary, stop. Do not call more tools."
);

pub fn parse_mode(id: &str) -> Option<&'static str> {
    match id.trim() {
        MODE_CODE => Some(MODE_CODE),
        MODE_PLAN => Some(MODE_PLAN),
        _ => None,
    }
}

pub fn available_mode_infos() -> Vec<SessionModeInfo> {
    vec![
        SessionModeInfo {
            id: MODE_CODE.into(),
            name: "Code".into(),
            description: Some("Implement, edit files, and run commands.".into()),
        },
        SessionModeInfo {
            id: MODE_PLAN.into(),
            name: "Plan".into(),
            description: Some(
                "Research and write an implementation plan. Source edits are blocked.".into(),
            ),
        },
    ]
}

pub fn session_modes_state(current: &str) -> SessionModeStateInfo {
    SessionModeStateInfo {
        current_mode_id: current.to_string(),
        available_modes: available_mode_infos(),
    }
}

pub fn artifacts_dir(cwd: &str, session_id: &str) -> PathBuf {
    codeg_agent_dir()
        .join("artifacts")
        .join(encode_session_cwd(cwd))
        .join(session_id)
}

/// Compact agent write root: `<artifacts_dir>/context`.
pub fn compact_context_dir(artifacts_dir: &Path) -> PathBuf {
    artifacts_dir.join("context")
}

pub fn plan_path(dir: &Path) -> PathBuf {
    dir.join(PLAN_FILE_NAME)
}

pub fn explore_path(dir: &Path) -> PathBuf {
    dir.join(EXPLORE_FILE_NAME)
}

pub fn mode_file(dir: &Path) -> PathBuf {
    dir.join(SESSION_MODE_FILE)
}

pub fn load_persisted_mode(dir: &Path) -> String {
    fs::read_to_string(mode_file(dir))
        .ok()
        .and_then(|s| parse_mode(&s).map(str::to_string))
        .unwrap_or_else(|| MODE_CODE.to_string())
}

pub fn persist_mode(dir: &Path, mode: &str) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(mode_file(dir), mode);
}

pub fn with_mode_attachment(preamble: &str, mode: &str) -> String {
    if mode == MODE_PLAN {
        format!("{preamble}\n\n{PLAN_MODE_ATTACHMENT}")
    } else {
        preamble.to_string()
    }
}

pub fn cap_explore_summary(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(no summary)".into();
    }
    let mut end = EXPLORE_SUMMARY_MAX_BYTES.min(trimmed.len());
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_string()
}

pub fn format_explore_handoff(path: &Path, status: &str, summary: &str) -> String {
    format!(
        "Explore report ready.\npath: {}\nstatus: {status}\nsummary:\n{}\n\n\
Read the file with read_file before planning or implementing. The summary is not a substitute for the report.",
        path.display(),
        cap_explore_summary(summary)
    )
}

pub fn plan_compact_prompt(base: &str) -> String {
    format!(
        "{base}\n\nThe session is in plan mode (read-only planning). Mention that in the summary."
    )
}

pub async fn emit_modes(
    state: &Arc<TokioRwLock<SessionState>>,
    emitter: &EventEmitter,
    current: &str,
) {
    emit_with_state(
        state,
        emitter,
        AcpEvent::SessionModes {
            modes: session_modes_state(current),
        },
    )
    .await;
}

pub async fn apply_mode(
    state: &Arc<TokioRwLock<SessionState>>,
    emitter: &EventEmitter,
    slot: &Arc<TokioRwLock<String>>,
    artifacts: &Path,
    mode: &str,
) {
    persist_mode(artifacts, mode);
    *slot.write().await = mode.to_string();
    emit_with_state(
        state,
        emitter,
        AcpEvent::ModeChanged {
            mode_id: mode.to_string(),
        },
    )
    .await;
    emit_modes(state, emitter, mode).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_mode_accepts_code_and_plan_only() {
        assert_eq!(parse_mode("code"), Some(MODE_CODE));
        assert_eq!(parse_mode(" plan "), Some(MODE_PLAN));
        assert_eq!(parse_mode("explore"), None);
        assert_eq!(parse_mode("unknown"), None);
    }

    #[test]
    fn available_modes_are_code_and_plan() {
        let ids: Vec<_> = available_mode_infos().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["code", "plan"]);
    }

    #[test]
    fn persist_and_load_mode_round_trip() {
        let dir = tempdir().unwrap();
        persist_mode(dir.path(), MODE_PLAN);
        assert_eq!(load_persisted_mode(dir.path()), MODE_PLAN);
    }

    #[test]
    fn load_missing_mode_defaults_to_code() {
        let dir = tempdir().unwrap();
        assert_eq!(load_persisted_mode(dir.path()), MODE_CODE);
    }

    #[test]
    fn explore_handoff_contains_path_and_summary_not_report_body() {
        let path = Path::new("/tmp/explore.md");
        let note = format_explore_handoff(path, "ok", "looked at auth\n## huge report body");
        assert!(note.contains("path: /tmp/explore.md"), "{note}");
        assert!(note.contains("status: ok"), "{note}");
        assert!(note.contains("looked at auth"), "{note}");
        assert!(note.contains("read_file"), "{note}");
    }

    #[test]
    fn empty_summary_uses_placeholder() {
        assert_eq!(cap_explore_summary("  "), "(no summary)");
    }

    #[test]
    fn plan_attachment_only_in_plan_mode() {
        let text = with_mode_attachment("base", MODE_PLAN);
        assert!(text.contains("Session mode: plan"));
        let code = with_mode_attachment("base", MODE_CODE);
        assert_eq!(code, "base");
    }

    #[test]
    fn compact_context_dir_is_under_global_session_artifacts() {
        let artifacts = artifacts_dir("/tmp/ws", "sess-1");
        let context = compact_context_dir(&artifacts);
        assert!(
            context.starts_with(&artifacts),
            "{} should stay under {}",
            context.display(),
            artifacts.display()
        );
        assert_eq!(
            context.file_name().and_then(|s| s.to_str()),
            Some("context")
        );
        assert!(!context.ends_with("plan.md"));
    }
}

//! 定义从 ACP 会话边界传给 Wiki 的用户、助手和工具观察片段。
//! supervisor 结束成功轮次时构造，source 消费；不包含隐藏推理或额外读取会话存储。
//! 按文本与工具数量预算保留有限材料，并记录截断情况供后续整理理解上下文。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Combined Unicode-char budget for user + assistant ACP bodies.
pub const ACP_TEXT_COMBINED_MAX: usize = 32768;
/// Per-side budget (head+tail kept when truncated).
pub const ACP_TEXT_SIDE_BUDGET: usize = 16384;
/// Maximum tool observations retained on an ACP snapshot.
pub const ACP_TOOL_OBS_MAX: usize = 50;

const TRUNCATION_MARKER: &str = "\n…[truncated]…\n";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiFileChange {
    pub path: String,
    pub operation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiToolObservation {
    pub id: String,
    pub path: Option<String>,
    pub kind: String,
    pub status: String,
    /// Short rendered summary — never raw secret-bearing tool input.
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiTurnSnapshot {
    pub run_id: String,
    pub connection_id: String,
    pub conversation_id: Option<i32>,
    pub agent_type: String,
    pub working_dir: Option<String>,
    pub folder_id: Option<i32>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub captured_at: DateTime<Utc>,
    pub user_text: String,
    pub assistant_text: String,
    pub user_original_chars: usize,
    pub assistant_original_chars: usize,
    pub user_truncated: bool,
    pub assistant_truncated: bool,
    pub tool_observations: Vec<WikiToolObservation>,
    pub tool_dropped_count: usize,
    pub file_changes: Vec<WikiFileChange>,
}

impl WikiTurnSnapshot {
    /// Apply ACP-only truncation: separate per-side budgets summing to
    /// [`ACP_TEXT_COMBINED_MAX`], head+tail kept, original lengths recorded.
    pub fn apply_acp_truncation(&mut self) {
        let (user, user_n, user_t) = truncate_head_tail(&self.user_text, ACP_TEXT_SIDE_BUDGET);
        let (asst, asst_n, asst_t) = truncate_head_tail(&self.assistant_text, ACP_TEXT_SIDE_BUDGET);
        self.user_original_chars = user_n;
        self.assistant_original_chars = asst_n;
        self.user_truncated = user_t;
        self.assistant_truncated = asst_t;
        self.user_text = user;
        self.assistant_text = asst;

        if self.tool_observations.len() > ACP_TOOL_OBS_MAX {
            self.tool_dropped_count = self.tool_observations.len() - ACP_TOOL_OBS_MAX;
            self.tool_observations.truncate(ACP_TOOL_OBS_MAX);
        }
    }

    pub fn has_visible_content(&self) -> bool {
        !self.assistant_text.trim().is_empty() || !self.tool_observations.is_empty()
    }
}

/// Keep head+tail when `text` exceeds `budget` Unicode characters.
pub fn truncate_head_tail(text: &str, budget: usize) -> (String, usize, bool) {
    let original = text.chars().count();
    if original <= budget {
        return (text.to_string(), original, false);
    }
    let marker_len = TRUNCATION_MARKER.chars().count();
    if budget <= marker_len {
        let sliced: String = text.chars().take(budget).collect();
        return (sliced, original, true);
    }
    let keep = budget - marker_len;
    let head_n = keep / 2;
    let tail_n = keep - head_n;
    let head: String = text.chars().take(head_n).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_n)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (format!("{head}{TRUNCATION_MARKER}{tail}"), original, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WikiTurnSnapshot {
        WikiTurnSnapshot {
            run_id: "run-1".into(),
            connection_id: "conn".into(),
            conversation_id: Some(1),
            agent_type: "claude_code".into(),
            working_dir: None,
            folder_id: Some(1),
            model: Some("sonnet".into()),
            mode: Some("code".into()),
            occurred_at: Utc::now(),
            captured_at: Utc::now(),
            user_text: "hello".into(),
            assistant_text: "world".into(),
            user_original_chars: 5,
            assistant_original_chars: 5,
            user_truncated: false,
            assistant_truncated: false,
            tool_observations: vec![],
            tool_dropped_count: 0,
            file_changes: vec![],
        }
    }

    #[test]
    fn snapshot_identity_keeps_user_assistant_tools() {
        let mut snap = sample();
        snap.tool_observations.push(WikiToolObservation {
            id: "t1".into(),
            path: Some("src/a.rs".into()),
            kind: "edit".into(),
            status: "completed".into(),
            summary: "patched".into(),
        });
        snap.file_changes.push(WikiFileChange {
            path: "src/a.rs".into(),
            operation: "edit".into(),
        });
        snap.apply_acp_truncation();
        assert_eq!(snap.user_text, "hello");
        assert_eq!(snap.assistant_text, "world");
        assert!(!snap.user_truncated);
        assert!(!snap.assistant_truncated);
        assert_eq!(snap.tool_observations.len(), 1);
        assert_eq!(snap.file_changes[0].path, "src/a.rs");
        assert_eq!(snap.run_id, "run-1");
    }

    #[test]
    fn truncate_keeps_head_and_tail() {
        let text: String = "abcdefghijklmnopqrstuvwxyz".repeat(4); // 104 chars
        let (out, orig, truncated) = truncate_head_tail(&text, 20);
        assert_eq!(orig, 104);
        assert!(truncated);
        assert!(out.contains(TRUNCATION_MARKER));
        assert!(out.chars().count() <= 20);
        assert!(out.starts_with("ab"));
        assert!(out.ends_with("xyz"));
    }

    #[test]
    fn combined_budget_is_split_across_sides() {
        let user: String = "u".repeat(40_000);
        let asst: String = "a".repeat(40_000);
        let mut snap = sample();
        snap.user_text = user;
        snap.assistant_text = asst;
        snap.apply_acp_truncation();
        assert_eq!(snap.user_original_chars, 40_000);
        assert_eq!(snap.assistant_original_chars, 40_000);
        assert!(snap.user_truncated);
        assert!(snap.assistant_truncated);
        assert!(snap.user_text.chars().count() <= ACP_TEXT_SIDE_BUDGET);
        assert!(snap.assistant_text.chars().count() <= ACP_TEXT_SIDE_BUDGET);
        assert!(
            snap.user_text.chars().count() + snap.assistant_text.chars().count()
                <= ACP_TEXT_COMBINED_MAX
        );
    }

    #[test]
    fn tool_observations_cap_records_dropped() {
        let mut snap = sample();
        snap.tool_observations = (0..60)
            .map(|i| WikiToolObservation {
                id: format!("t{i}"),
                path: None,
                kind: "read".into(),
                status: "completed".into(),
                summary: format!("s{i}"),
            })
            .collect();
        snap.apply_acp_truncation();
        assert_eq!(snap.tool_observations.len(), ACP_TOOL_OBS_MAX);
        assert_eq!(snap.tool_dropped_count, 10);
    }
}

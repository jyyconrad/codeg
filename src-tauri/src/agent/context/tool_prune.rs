//! Per-tool distillation of tool results for request projection and compact input.
//!
//! Claude Code / OpenCode / OpenClaw all prune tool output before summarizing
//! turns, but they do not treat every tool the same: reads are reloadable,
//! writes collapse to a path+size ack, bash output cannot be replayed.

use super::spill::recover_hint;
use super::store::ExecutionFact;

const CLEARED_TOOL_RESULT: &str = "[Old tool result content cleared]";
const LIVE_READ_KEEP_CHARS: usize = 2_000;
const LIVE_BASH_KEEP_CHARS: usize = 2_000;
const LIVE_DEFAULT_KEEP_CHARS: usize = 4_000;
const LIVE_EXCERPT_CHARS: usize = 800;
const SUMMARIZE_KEEP_CHARS: usize = 400;
const SUMMARIZE_EXCERPT_CHARS: usize = 400;
const SUBAGENT_KEEP_CHARS: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DistillKind {
    /// Protected tail of the live request history.
    Live,
    /// Evicted turns sent to L1 TemplateCompactor / L2 LLM.
    Summarize,
}

pub(crate) fn tool_skips_hard_clear(name: &str) -> bool {
    matches!(name, "skill" | "update_plan")
}

pub(crate) fn distill_tool_result(
    name: &str,
    fact: Option<&ExecutionFact>,
    text: &str,
    kind: DistillKind,
) -> String {
    match name {
        "read_file" | "skill" => distill_read(name, fact, text, kind),
        "write_file" => distill_write(fact, text),
        "edit_file" => distill_edit(fact, text),
        "bash" => distill_bash(fact, text, kind),
        "grep" | "glob" | "codegraph" | "lsp" => distill_search(name, fact, text, kind),
        "subagent" => distill_generic(name, text, kind, SUBAGENT_KEEP_CHARS),
        _ => distill_generic(name, text, kind, LIVE_DEFAULT_KEEP_CHARS),
    }
}

pub(crate) fn hard_clear_tool_result(
    name: &str,
    fact: Option<&ExecutionFact>,
    text: &str,
) -> String {
    if tool_skips_hard_clear(name) {
        return distill_tool_result(name, fact, text, DistillKind::Summarize);
    }
    let chars = text.chars().count();
    match name {
        "read_file" => format!(
            "Read {} complete ({chars} chars). Full contents omitted; re-read the file if needed.",
            tool_path(fact, text)
        ),
        "write_file" => write_notice(fact, text),
        "edit_file" => format!("Updated {}.", tool_path(fact, text)),
        "bash" => format!(
            "bash finished ({chars} chars captured). Output omitted.{}",
            recover_hint(fact)
        ),
        "grep" | "glob" | "codegraph" | "lsp" => format!(
            "{name} completed ({chars} chars). Output omitted; re-run the search if needed.{}",
            recover_hint(fact)
        ),
        "subagent" => format!(
            "subagent finished ({chars} chars). Output omitted.{}",
            recover_hint(fact)
        ),
        _ => format!(
            "{CLEARED_TOOL_RESULT} ({name}, {chars} chars).{}",
            recover_hint(fact)
        ),
    }
}

fn distill_read(name: &str, fact: Option<&ExecutionFact>, text: &str, kind: DistillKind) -> String {
    let keep = keep_chars(kind, LIVE_READ_KEEP_CHARS);
    let chars = text.chars().count();
    if chars <= keep {
        return text.to_string();
    }
    let path = tool_path(fact, text);
    let (head, tail) = excerpt_sizes(kind);
    let verb = if name == "skill" {
        "Loaded skill"
    } else {
        "Read"
    };
    format!(
        "{verb} {path} complete ({chars} chars). Start/end excerpt; re-read the file for full contents:\n{}\n...\n{}",
        prefix_chars(text, head),
        suffix_chars(text, tail),
    )
}

fn distill_write(fact: Option<&ExecutionFact>, text: &str) -> String {
    if text.chars().count() <= 400 && looks_like_write_ack(text) {
        return text.to_string();
    }
    write_notice(fact, text)
}

fn distill_edit(fact: Option<&ExecutionFact>, text: &str) -> String {
    if text.chars().count() <= 400 {
        return text.to_string();
    }
    format!("Updated {}.", tool_path(fact, text))
}

fn distill_bash(fact: Option<&ExecutionFact>, text: &str, kind: DistillKind) -> String {
    let keep = keep_chars(kind, LIVE_BASH_KEEP_CHARS);
    let chars = text.chars().count();
    if chars <= keep {
        return text.to_string();
    }
    let (head, tail) = excerpt_sizes(kind);
    let exit = text
        .lines()
        .find(|line| line.starts_with("exit_code:"))
        .unwrap_or("bash finished");
    let command = fact
        .and_then(|item| item.raw_input.get("command").and_then(|v| v.as_str()))
        .map(|cmd| {
            let clipped = prefix_chars(cmd, 120);
            if clipped.len() < cmd.len() {
                format!("{clipped}…")
            } else {
                clipped.to_string()
            }
        });
    let cmd_bit = command
        .map(|cmd| format!(" command `{cmd}`"))
        .unwrap_or_default();
    format!(
        "{exit}{cmd_bit} ({chars} chars). Start/end excerpt:{}\n{}\n...\n{}",
        recover_hint(fact),
        prefix_chars(text, head),
        suffix_chars(text, tail),
    )
}

fn distill_search(
    name: &str,
    fact: Option<&ExecutionFact>,
    text: &str,
    kind: DistillKind,
) -> String {
    let keep = keep_chars(kind, LIVE_DEFAULT_KEEP_CHARS);
    let chars = text.chars().count();
    if chars <= keep {
        return text.to_string();
    }
    let (head, tail) = excerpt_sizes(kind);
    let pattern = fact
        .and_then(|item| item.raw_input.get("pattern").and_then(|v| v.as_str()))
        .unwrap_or(name);
    format!(
        "{name} `{pattern}` completed ({chars} chars). Re-run the search if needed. Start/end excerpt:\n{}\n...\n{}",
        prefix_chars(text, head),
        suffix_chars(text, tail),
    )
}

fn distill_generic(name: &str, text: &str, kind: DistillKind, live_keep: usize) -> String {
    let keep = keep_chars(kind, live_keep);
    let chars = text.chars().count();
    if chars <= keep {
        return text.to_string();
    }
    let (head, tail) = excerpt_sizes(kind);
    format!(
        "{name} completed ({chars} chars). Start/end excerpt:\n{}\n...\n{}",
        prefix_chars(text, head),
        suffix_chars(text, tail),
    )
}

fn keep_chars(kind: DistillKind, live: usize) -> usize {
    match kind {
        DistillKind::Live => live,
        DistillKind::Summarize => SUMMARIZE_KEEP_CHARS.min(live),
    }
}

fn excerpt_sizes(kind: DistillKind) -> (usize, usize) {
    match kind {
        DistillKind::Live => (LIVE_EXCERPT_CHARS, LIVE_EXCERPT_CHARS),
        DistillKind::Summarize => (SUMMARIZE_EXCERPT_CHARS, SUMMARIZE_EXCERPT_CHARS),
    }
}

fn write_notice(fact: Option<&ExecutionFact>, text: &str) -> String {
    let path = tool_path(fact, text);
    let chars = fact
        .and_then(|item| item.raw_input.get("content").and_then(|v| v.as_str()))
        .map(|content| content.chars().count())
        .unwrap_or_else(|| text.chars().count());
    format!("Wrote {path} ({chars} chars).")
}

fn looks_like_write_ack(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("wrote ") || lower.starts_with("updated ")
}

fn tool_path(fact: Option<&ExecutionFact>, text: &str) -> String {
    if let Some(path) = fact.and_then(|item| {
        item.output_locator
            .as_ref()
            .and_then(|locator| locator.path.as_deref())
            .or_else(|| item.raw_input.get("path").and_then(|v| v.as_str()))
    }) {
        return path.to_string();
    }
    if let Some(rest) = text.strip_prefix("# ") {
        let line = rest.lines().next().unwrap_or(rest);
        if let Some((path, _)) = line.split_once(" (lines ") {
            return path.to_string();
        }
        return line.to_string();
    }
    "unknown path".to_string()
}

fn prefix_chars(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

fn suffix_chars(text: &str, n: usize) -> &str {
    let count = text.chars().count();
    if count <= n {
        return text;
    }
    match text.char_indices().nth(count - n) {
        Some((index, _)) => &text[index..],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::store::ExecutionFact;
    use crate::agent::context::{ToolOutcome, ToolPhase};
    use serde_json::json;

    fn fact(name: &str, input: serde_json::Value, body: &str) -> ExecutionFact {
        ExecutionFact {
            tool_call_id: "call_1".into(),
            function_name: name.into(),
            raw_input: input,
            phase: ToolPhase::Terminal,
            outcome: Some(ToolOutcome::Success),
            executed: Some(true),
            model_presentation: Some(body.to_string()),
            truncated: false,
            output_locator: None,
            reason: None,
            turn_id: "s:1".into(),
        }
    }

    #[test]
    fn read_distill_keeps_path_excerpts_and_reread_hint() {
        let body = format!("HEAD-MARKER-{}-TAIL-MARKER", "n".repeat(5_000));
        let item = fact("read_file", json!({"path": "src/a.rs"}), &body);
        let out = distill_tool_result("read_file", Some(&item), &body, DistillKind::Live);
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(out.contains("HEAD-MARKER-"), "{out}");
        assert!(out.contains("TAIL-MARKER"), "{out}");
        assert!(out.contains("re-read"), "{out}");
        assert!(!out.contains(&"n".repeat(3_000)), "{out}");
    }

    #[test]
    fn write_distill_collapses_to_path_and_size() {
        let body = format!("FILE-BODY-{}", "w".repeat(8_000));
        let item = fact(
            "write_file",
            json!({"path": "src/out.rs", "content": "hello world"}),
            &body,
        );
        let out = distill_tool_result("write_file", Some(&item), &body, DistillKind::Live);
        assert_eq!(out, "Wrote src/out.rs (11 chars).");
    }

    #[test]
    fn bash_distill_keeps_exit_code_and_tail() {
        let body = format!(
            "exit_code: 1\ntruncated: false\ncaptured_bytes: 9000\n\nHEAD-OUT-{}-TAIL-ERR",
            "x".repeat(6_000)
        );
        let item = fact("bash", json!({"command": "cargo test"}), &body);
        let out = distill_tool_result("bash", Some(&item), &body, DistillKind::Live);
        assert!(out.contains("exit_code: 1"), "{out}");
        assert!(out.contains("cargo test"), "{out}");
        assert!(out.contains("HEAD-OUT-"), "{out}");
        assert!(out.contains("TAIL-ERR"), "{out}");
    }

    #[test]
    fn bash_hard_clear_points_at_recall_when_spilled() {
        let body = format!("OLD-BASH-{}", "x".repeat(4_000));
        let mut item = fact("bash", json!({"command": "ls"}), &body);
        item.truncated = true;
        item.output_locator = Some(crate::agent::context::OutputLocator {
            path: Some("/tmp/spill.txt".into()),
            line: Some(1),
        });
        let out = hard_clear_tool_result("bash", Some(&item), &body);
        assert!(out.contains("tool_call_id `call_1`"), "{out}");
        assert!(!out.contains("OLD-BASH-"), "{out}");
    }

    #[test]
    fn hard_clear_read_keeps_path_without_body() {
        let body = format!("OLD-TOOL-BODY-{}", "x".repeat(4_000));
        let item = fact("read_file", json!({"path": "f0.txt"}), &body);
        let out = hard_clear_tool_result("read_file", Some(&item), &body);
        assert!(out.contains("f0.txt"), "{out}");
        assert!(out.contains("re-read"), "{out}");
        assert!(!out.contains("OLD-TOOL-BODY-"), "{out}");
    }

    #[test]
    fn codegraph_distill_uses_search_path() {
        let body = format!("HEAD-GRAPH-{}-TAIL-GRAPH", "n".repeat(5_000));
        let item = fact("codegraph", json!({"query": "auth"}), &body);
        let out = distill_tool_result("codegraph", Some(&item), &body, DistillKind::Live);
        assert!(out.contains("codegraph"), "{out}");
        assert!(out.contains("run the search"), "{out}");
        assert!(out.contains("HEAD-GRAPH-"), "{out}");
        assert!(out.contains("TAIL-GRAPH"), "{out}");
        assert!(!out.contains(&"n".repeat(3_000)), "{out}");
    }

    #[test]
    fn hard_clear_codegraph_omits_body_like_grep() {
        let body = format!("OLD-GRAPH-{}", "x".repeat(4_000));
        let item = fact("codegraph", json!({"query": "auth"}), &body);
        let out = hard_clear_tool_result("codegraph", Some(&item), &body);
        assert!(out.contains("codegraph"), "{out}");
        assert!(out.contains("re-run the search"), "{out}");
        assert!(!out.contains("OLD-GRAPH-"), "{out}");
    }

    #[test]
    fn lsp_distill_uses_search_path() {
        let body = format!("HEAD-LSP-{}-TAIL-LSP", "n".repeat(5_000));
        let item = fact("lsp", json!({"query": "Foo"}), &body);
        let out = distill_tool_result("lsp", Some(&item), &body, DistillKind::Live);
        assert!(out.contains("lsp"), "{out}");
        assert!(out.contains("run the search"), "{out}");
        assert!(out.contains("HEAD-LSP-"), "{out}");
        assert!(out.contains("TAIL-LSP"), "{out}");
        assert!(!out.contains(&"n".repeat(3_000)), "{out}");
    }

    #[test]
    fn hard_clear_lsp_omits_body_like_grep() {
        let body = format!("OLD-LSP-{}", "x".repeat(4_000));
        let item = fact("lsp", json!({"query": "Foo"}), &body);
        let out = hard_clear_tool_result("lsp", Some(&item), &body);
        assert!(out.contains("lsp"), "{out}");
        assert!(out.contains("re-run the search"), "{out}");
        assert!(!out.contains("OLD-LSP-"), "{out}");
    }
}

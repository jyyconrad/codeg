//! Session system preamble: body + OpenCode-style env + optional workspace context.

use std::path::Path;

use crate::acp::native_config::ContextInject;
use crate::agent::workspace_context::{
    detect_runtime_env, format_env_block, format_instructions, load_top_level_instruction,
    workspace_tree_markdown, InstructionFile, RuntimeEnv,
};

/// Built-in preamble body when `CODEG_AGENT_SYSTEM_PROMPT` is empty.
///
/// Must stay byte-identical to `CODEG_BUILTIN_SYSTEM_PROMPT` in
/// `src/lib/codeg-agent-prompts.ts`. Shaped after OpenCode's layered system
/// prompt (identity + how-you-work); env, instructions, tree, and skills are
/// appended at spawn and are not part of this body.
pub const DEFAULT_SYSTEM_PROMPT: &str = concat!(
    "You are Codeg Agent, a coding assistant that runs inside Codeg. You and the user share this workspace and work toward the user's goal.\n",
    "\n",
    "Inspect the workspace before changing code. Prefer the smallest correct change. Verify with the checks this repository already uses. Persist until the goal is complete unless the user pauses or redirects you.\n",
    "\n",
    "How you work:\n",
    "- Use read_file, glob, and grep to inspect before editing. Prefer edit_file for small changes; write_file only for new files.\n",
    "- Call independent tools in parallel when neither call needs the other's output.\n",
    "- Load a listed skill with the skill tool when the task matches it. Do not reread a skill already in context.\n",
    "- Use subagent (explore) for broad multi-file investigation; read the report path before planning or implementing.\n",
    "- Ask with ask_user_question only when a decision changes the approach.\n",
    "- Follow existing conventions. Do not add comments, fallbacks, or extra files unless they are needed.\n",
    "- Never revert or overwrite changes you did not make unless the user asks.\n",
    "- Prefer concise, correct answers. Do not narrate routine reads.\n",
    "\n",
    "Output:\n",
    "- Write explanations in Markdown.\n",
    "- When naming a workspace file, emit a markdown link whose target is a workspace-relative path, for example `[使用手册.docx](docs/使用手册.docx)`. Use a `file://` URL only when the file is outside the workspace.\n",
    "- Do not wrap the only copy of a file path in backticks. If you show a path as code, also include the clickable link."
);

pub struct PreambleSpec<'a> {
    pub skills_section: Option<&'a str>,
    pub system_prompt: Option<&'a str>,
    pub runtime: RuntimeEnv,
    pub instructions: &'a [InstructionFile],
    pub tree: Option<&'a str>,
}

/// Empty / whitespace `system_prompt` uses [`DEFAULT_SYSTEM_PROMPT`]. Env,
/// cwd, global storage, and the skill catalog are always appended so a custom
/// prompt cannot drop them.
pub fn session_preamble(spec: PreambleSpec<'_>) -> String {
    let body = spec
        .system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_SYSTEM_PROMPT);
    let mut text = body.to_string();
    text.push_str("\n\n");
    text.push_str(&format_env_block(&spec.runtime));
    if let Some(instructions) = format_instructions(spec.instructions) {
        text.push_str("\n\n");
        text.push_str(&instructions);
    }
    if let Some(tree) = spec.tree.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n");
        text.push_str(tree);
    }
    if let Some(section) = spec.skills_section.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n");
        text.push_str(section);
    }
    text
}

/// Live preamble: detect env, optionally load top-level instruction files and
/// the 3-level workspace tree.
///
/// Instruction files (`AGENTS.md` / `CLAUDE.md`) are main-agent only in this
/// phase and only scanned at the working-directory top level.
pub fn live_session_preamble(
    cwd: &Path,
    skills_section: Option<&str>,
    system_prompt: Option<&str>,
    inject: ContextInject,
) -> String {
    let runtime = detect_runtime_env(cwd);
    let mut instructions = Vec::new();
    if inject.agents_md {
        if let Some(file) = load_top_level_instruction(cwd, "AGENTS.md") {
            instructions.push(file);
        }
    }
    if inject.claude_md {
        if let Some(file) = load_top_level_instruction(cwd, "CLAUDE.md") {
            instructions.push(file);
        }
    }
    let tree = inject.tree.then(|| workspace_tree_markdown(cwd));
    session_preamble(PreambleSpec {
        skills_section,
        system_prompt,
        runtime,
        instructions: &instructions,
        tree: tree.as_deref(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec<'a>(
        skills: Option<&'a str>,
        system: Option<&'a str>,
        instructions: &'a [InstructionFile],
        tree: Option<&'a str>,
    ) -> PreambleSpec<'a> {
        PreambleSpec {
            skills_section: skills,
            system_prompt: system,
            runtime: RuntimeEnv {
                platform: "macos".into(),
                now: "2026-09-15 12:00:00 +0800".into(),
                cwd: PathBuf::from("/work"),
                global_dir: PathBuf::from("/tmp/codeg-agent"),
                git_repo: false,
            },
            instructions,
            tree,
        }
    }

    #[test]
    fn preamble_includes_catalog_and_env() {
        let text = session_preamble(spec(
            Some("Available skills (call the skill tool with `name` to load SKILL.md):\n- `demo`: does the thing"),
            None,
            &[],
            None,
        ));
        assert!(text.contains("Working directory: /work"), "{text}");
        assert!(text.contains("Platform: macos"), "{text}");
        assert!(text.contains("Global storage: /tmp/codeg-agent"), "{text}");
        assert!(
            text.contains("Current time: 2026-09-15 12:00:00 +0800"),
            "{text}"
        );
        assert!(text.contains("`demo`"), "{text}");
        assert!(text.contains("does the thing"), "{text}");
        assert!(text.contains(DEFAULT_SYSTEM_PROMPT), "{text}");
    }

    #[test]
    fn preamble_omits_empty_catalog() {
        let text = session_preamble(spec(None, None, &[], None));
        assert!(!text.contains("Available skills"), "{text}");
        let text = session_preamble(spec(Some("  "), None, &[], None));
        assert!(!text.contains("Available skills"), "{text}");
    }

    #[test]
    fn custom_system_prompt_replaces_builtin_body() {
        let text = session_preamble(spec(None, Some(" Be terse. "), &[], None));
        assert!(
            text.starts_with("Be terse.\n\nHere is some useful information"),
            "{text}"
        );
        assert!(
            !text.contains("You are Codeg Agent, a coding assistant that runs inside Codeg."),
            "{text}"
        );
    }

    #[test]
    fn whitespace_system_prompt_uses_builtin() {
        let text = session_preamble(spec(None, Some("  \n"), &[], None));
        assert!(text.contains(DEFAULT_SYSTEM_PROMPT), "{text}");
    }

    #[test]
    fn builtin_prompt_asks_for_clickable_file_links() {
        assert!(
            DEFAULT_SYSTEM_PROMPT.contains("[使用手册.docx](docs/使用手册.docx)"),
            "{DEFAULT_SYSTEM_PROMPT}"
        );
    }

    #[test]
    fn live_preamble_injects_top_level_agents_md_and_tree() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("AGENTS.md"), "do not commit secrets\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn x() {}\n").unwrap();
        let text = live_session_preamble(
            dir.path(),
            None,
            None,
            ContextInject {
                agents_md: true,
                claude_md: false,
                tree: true,
            },
        );
        assert!(text.contains("do not commit secrets"), "{text}");
        assert!(text.contains("Instructions from:"), "{text}");
        assert!(text.contains("max 3 levels injected"), "{text}");
        assert!(text.contains("src/"), "{text}");
        assert!(text.contains("lib.rs"), "{text}");
        assert!(text.contains("Working directory:"), "{text}");
        assert!(text.contains("Global storage:"), "{text}");
        assert!(text.contains("Platform:"), "{text}");
        assert!(text.contains("Current time:"), "{text}");
    }

    #[test]
    fn live_preamble_injects_top_level_claude_md_without_tree() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("CLAUDE.md"), "prefer existing helpers\n").unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/CLAUDE.md"), "nested-claude\n").unwrap();
        let text = live_session_preamble(
            dir.path(),
            None,
            None,
            ContextInject {
                agents_md: false,
                claude_md: true,
                tree: false,
            },
        );
        assert!(text.contains("prefer existing helpers"), "{text}");
        assert!(!text.contains("nested-claude"), "{text}");
        assert!(!text.contains("max 3 levels injected"), "{text}");
    }

    #[test]
    fn live_preamble_skips_instructions_when_flags_off() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("AGENTS.md"), "secret-rule\n").unwrap();
        let text = live_session_preamble(dir.path(), None, None, ContextInject::default());
        assert!(!text.contains("secret-rule"), "{text}");
        assert!(!text.contains("max 3 levels injected"), "{text}");
        assert!(text.contains("Working directory:"), "{text}");
        assert!(text.contains("Global storage:"), "{text}");
        assert!(text.contains("Platform:"), "{text}");
        assert!(text.contains("Current time:"), "{text}");
    }
}

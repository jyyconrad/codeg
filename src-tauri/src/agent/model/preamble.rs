//! cwd / git-root instruction files and the skill catalog.

use std::path::Path;

/// Built-in preamble body when `CODEG_AGENT_SYSTEM_PROMPT` is empty.
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are Codeg Agent, a coding assistant running inside Codeg. \
Prefer concise, correct answers.";

/// Short coding preamble plus cwd and an optional skill catalog section.
///
/// Empty / whitespace `system_prompt` uses [`DEFAULT_SYSTEM_PROMPT`]. cwd and
/// the skill catalog are always appended so a custom prompt cannot drop them.
pub fn session_preamble(
    cwd: &Path,
    skills_section: Option<&str>,
    system_prompt: Option<&str>,
) -> String {
    let body = system_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_SYSTEM_PROMPT);
    let mut text = format!("{body}\nWorking directory: {}.", cwd.display());
    if let Some(section) = skills_section.map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str("\n\n");
        text.push_str(section);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_includes_catalog_section() {
        let text = session_preamble(
            Path::new("/work"),
            Some("Available skills (call the skill tool with `name` to load SKILL.md):\n- `demo`: does the thing"),
            None,
        );
        assert!(text.contains("Working directory: /work"), "{text}");
        assert!(text.contains("`demo`"), "{text}");
        assert!(text.contains("does the thing"), "{text}");
        assert!(text.contains(DEFAULT_SYSTEM_PROMPT), "{text}");
    }

    #[test]
    fn preamble_omits_empty_catalog() {
        let text = session_preamble(Path::new("/work"), None, None);
        assert!(!text.contains("Available skills"), "{text}");
        let text = session_preamble(Path::new("/work"), Some("  "), None);
        assert!(!text.contains("Available skills"), "{text}");
    }

    #[test]
    fn custom_system_prompt_replaces_builtin_body() {
        let text = session_preamble(Path::new("/work"), None, Some(" Be terse. "));
        assert!(
            text.starts_with("Be terse.\nWorking directory: /work."),
            "{text}"
        );
        assert!(!text.contains(DEFAULT_SYSTEM_PROMPT), "{text}");
    }

    #[test]
    fn whitespace_system_prompt_uses_builtin() {
        let text = session_preamble(Path::new("/work"), None, Some("  \n"));
        assert!(text.contains(DEFAULT_SYSTEM_PROMPT), "{text}");
    }
}

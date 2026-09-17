//! Workspace context injected into Codeg Agent preambles.
//!
//! OpenCode's system-context builtins supply `<env>` (cwd, workspace root,
//! git, platform) plus today's date, and instruction-context loads AGENTS.md.
//! Codeg follows that shape. Phase 1 instruction files are **main-agent only**
//! and only the **working-directory top level** is scanned (no parent walk).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::paths::codeg_agent_dir;

/// Max directory depth from the working directory (cwd = depth 0, children = 1).
/// Deeper than 3 is omitted; the injected tree header states this cap.
pub const TREE_MAX_DEPTH: usize = 3;
const TREE_MAX_ENTRIES: usize = 400;
const INSTRUCTION_MAX_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionFile {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEnv {
    pub platform: String,
    pub now: String,
    pub cwd: PathBuf,
    pub global_dir: PathBuf,
    pub git_repo: bool,
}

pub fn detect_runtime_env(cwd: &Path) -> RuntimeEnv {
    RuntimeEnv {
        platform: platform_label().to_string(),
        now: chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S %z")
            .to_string(),
        cwd: cwd.to_path_buf(),
        global_dir: codeg_agent_dir(),
        git_repo: cwd.join(".git").exists(),
    }
}

pub fn platform_label() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    }
}

/// Phase 1: only `{cwd}/{name}` if it is a regular file. No parent walk.
/// 首期只实现主 agent 注入、以及一级目录的扫描。
pub fn load_top_level_instruction(cwd: &Path, name: &str) -> Option<InstructionFile> {
    let path = cwd.join(name);
    if !path.is_file() {
        return None;
    }
    let mut content = fs::read_to_string(&path).ok()?;
    if content.len() > INSTRUCTION_MAX_BYTES {
        let mut end = INSTRUCTION_MAX_BYTES;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
        content.push_str("\n\n[truncated: file exceeded 32KB]");
    }
    Some(InstructionFile { path, content })
}

pub fn format_env_block(env: &RuntimeEnv) -> String {
    format!(
        concat!(
            "Here is some useful information about the environment you are running in:\n",
            "<env>\n",
            "  Platform: {}\n",
            "  Current time: {}\n",
            "  Working directory: {}\n",
            "  Global storage: {}\n",
            "  Is directory a git repo: {}\n",
            "</env>"
        ),
        env.platform,
        env.now,
        env.cwd.display(),
        env.global_dir.display(),
        if env.git_repo { "yes" } else { "no" },
    )
}

pub fn format_instructions(files: &[InstructionFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut out = String::from("Project instructions (working-directory top level only):\n");
    for (i, file) in files.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str("Instructions from: ");
        out.push_str(&file.path.display().to_string());
        out.push('\n');
        out.push_str(file.content.trim_end());
    }
    Some(out)
}

/// Markdown directory tree, at most [`TREE_MAX_DEPTH`] levels from `cwd`.
pub fn workspace_tree_markdown(cwd: &Path) -> String {
    let mut header =
        String::from("Workspace tree (max 3 levels injected; deeper paths omitted):\n```\n.\n");
    let tree = collect_tree(cwd);
    render_tree(&tree, "", &mut header);
    header.push_str("```");
    header
}

struct TreeNode {
    is_dir: bool,
    children: BTreeMap<String, TreeNode>,
}

fn collect_tree(cwd: &Path) -> BTreeMap<String, TreeNode> {
    let mut root = TreeNode {
        is_dir: true,
        children: BTreeMap::new(),
    };
    let walker = WalkBuilder::new(cwd)
        .max_depth(Some(TREE_MAX_DEPTH))
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut count = 0usize;
    for entry in walker {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path == cwd {
            continue;
        }
        let Ok(rel) = path.strip_prefix(cwd) else {
            continue;
        };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        insert_rel(&mut root, rel, is_dir);
        count += 1;
        if count >= TREE_MAX_ENTRIES {
            break;
        }
    }
    root.children
}

fn insert_rel(root: &mut TreeNode, rel: &Path, is_dir: bool) {
    let mut node = root;
    let parts: Vec<_> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect();
    let last = parts.len().saturating_sub(1);
    for (i, name) in parts.into_iter().enumerate() {
        let is_last = i == last;
        node = node.children.entry(name).or_insert_with(|| TreeNode {
            is_dir: if is_last { is_dir } else { true },
            children: BTreeMap::new(),
        });
        if is_last {
            node.is_dir = is_dir || node.is_dir;
        }
    }
}

fn render_tree(children: &BTreeMap<String, TreeNode>, prefix: &str, out: &mut String) {
    let len = children.len();
    for (i, (name, node)) in children.iter().enumerate() {
        let last = i + 1 == len;
        let branch = if last { "└── " } else { "├── " };
        let label = if node.is_dir {
            format!("{name}/")
        } else {
            name.clone()
        };
        out.push_str(prefix);
        out.push_str(branch);
        out.push_str(&label);
        out.push('\n');
        if !node.children.is_empty() {
            let next = format!("{prefix}{}", if last { "    " } else { "│   " });
            render_tree(&node.children, &next, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_instruction_ignores_nested_files() {
        let dir = tempfile::tempdir().expect("dir");
        fs::write(dir.path().join("AGENTS.md"), "root rules\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/AGENTS.md"), "nested\n").unwrap();
        let loaded = load_top_level_instruction(dir.path(), "AGENTS.md").expect("root");
        assert!(loaded.content.contains("root rules"), "{}", loaded.content);
        assert!(!loaded.content.contains("nested"), "{}", loaded.content);
        assert!(load_top_level_instruction(dir.path(), "CLAUDE.md").is_none());
    }

    #[test]
    fn tree_stops_at_three_levels_and_declares_the_cap() {
        let dir = tempfile::tempdir().expect("dir");
        fs::create_dir_all(dir.path().join("a/b/c/d")).unwrap();
        fs::write(dir.path().join("a/b/c/d/hidden.txt"), "x").unwrap();
        fs::write(dir.path().join("README.md"), "hi").unwrap();
        let tree = workspace_tree_markdown(dir.path());
        assert!(tree.contains("max 3 levels injected"), "{tree}");
        assert!(tree.contains("README.md"), "{tree}");
        assert!(tree.contains("a/"), "{tree}");
        assert!(tree.contains("b/"), "{tree}");
        assert!(tree.contains("c/"), "{tree}");
        assert!(
            !tree.contains("hidden.txt"),
            "depth-4 file must be omitted: {tree}"
        );
    }

    #[test]
    fn env_block_lists_platform_time_cwd_and_global_storage() {
        let env = RuntimeEnv {
            platform: "macos".into(),
            now: "2026-09-15 12:00:00 +0800".into(),
            cwd: PathBuf::from("/work/proj"),
            global_dir: PathBuf::from("/home/.codeg/codeg-agent"),
            git_repo: true,
        };
        let block = format_env_block(&env);
        assert!(block.contains("\n  Platform: macos\n"), "{block}");
        assert!(
            block.contains("Current time: 2026-09-15 12:00:00 +0800"),
            "{block}"
        );
        assert!(block.contains("Working directory: /work/proj"), "{block}");
        assert!(
            block.contains("Global storage: /home/.codeg/codeg-agent"),
            "{block}"
        );
        assert!(block.contains("Is directory a git repo: yes"), "{block}");
    }
}

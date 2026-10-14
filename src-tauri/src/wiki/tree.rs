//! 列出 Wiki 目录，供默认目录阅读器和高级文件浏览器导航使用。
//! 支持单层懒加载与有限深度递归；返回目录内相对路径，不跟随符号链接。
//! 只隐藏点开头的条目（.obsidian、.git 及其他点文件）；不按 raw、sources、journal、work 等名称隐藏。

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::wiki::paths::{is_safe_vault_relative, join_vault_relative};

/// Maximum directory depth walked when `recursive` is true (vault root = 0).
pub const MAX_TREE_DEPTH: u32 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiVaultTreeEntry {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub is_dir: bool,
    /// Present only for recursive listings of directories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WikiVaultTreeEntry>>,
}

#[derive(Debug)]
pub enum VaultTreeError {
    UnsafePath,
    NotFound,
    NotDirectory,
    Io(io::Error),
}

fn skip_child_name(name: &str, _include_raw: bool) -> bool {
    // Dot entries only. `include_raw` stays in the request so existing clients parse.
    name.starts_with('.')
}

fn child_rel(parent: &str, name: &str) -> String {
    if parent.trim().is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent.trim().trim_end_matches('/'), name)
    }
}

fn list_level(
    vault: &Path,
    rel: &str,
    include_raw: bool,
) -> Result<Vec<WikiVaultTreeEntry>, VaultTreeError> {
    if !is_safe_vault_relative(rel) {
        return Err(VaultTreeError::UnsafePath);
    }
    let dir = join_vault_relative(vault, rel).map_err(|_| VaultTreeError::UnsafePath)?;
    if !dir.exists() {
        if rel.trim().is_empty() {
            return Ok(Vec::new());
        }
        return Err(VaultTreeError::NotFound);
    }
    let canonical_root = vault.canonicalize().map_err(VaultTreeError::Io)?;
    let canonical_dir = dir.canonicalize().map_err(VaultTreeError::Io)?;
    if !canonical_dir.starts_with(canonical_root) {
        return Err(VaultTreeError::UnsafePath);
    }
    if !dir.is_dir() {
        return Err(VaultTreeError::NotDirectory);
    }

    let mut entries = Vec::new();
    let rd = fs::read_dir(&dir).map_err(VaultTreeError::Io)?;
    for ent in rd {
        let ent = ent.map_err(VaultTreeError::Io)?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if skip_child_name(&name, include_raw) {
            continue;
        }
        let child = child_rel(rel, &name);
        if !is_safe_vault_relative(&child) {
            continue;
        }
        let kind = ent.file_type().map_err(VaultTreeError::Io)?;
        if kind.is_symlink() {
            continue;
        }
        let is_dir = kind.is_dir();
        entries.push(WikiVaultTreeEntry {
            title: None,
            path: child.replace('\\', "/"),
            name,
            is_dir,
            children: None,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn walk(
    vault: &Path,
    rel: &str,
    include_raw: bool,
    recursive: bool,
    depth: u32,
) -> Result<Vec<WikiVaultTreeEntry>, VaultTreeError> {
    let mut entries = list_level(vault, rel, include_raw)?;
    if recursive && depth < MAX_TREE_DEPTH {
        for entry in &mut entries {
            if !entry.is_dir {
                continue;
            }
            entry.children = Some(walk(vault, &entry.path, include_raw, true, depth + 1)?);
        }
    }
    Ok(entries)
}

/// List `rel` under `vault`. `rel` empty means the vault root.
pub fn list_vault_tree(
    vault: &Path,
    rel: &str,
    recursive: bool,
    include_raw: bool,
) -> Result<Vec<WikiVaultTreeEntry>, VaultTreeError> {
    walk(vault, rel, include_raw, recursive, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn seed_vault(root: &Path) {
        for dir in [
            "work/projects",
            "capabilities",
            "knowledge/concepts",
            "sources",
            "journal",
            "raw/sessions",
            ".obsidian",
            ".git",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("index.md"), "# index").unwrap();
        fs::write(root.join("AGENTS.md"), "# agents").unwrap();
        fs::write(root.join("log.md"), "# log").unwrap();
        fs::write(root.join("work/index.md"), "# work").unwrap();
        fs::write(root.join("work/projects/alpha.md"), "# alpha").unwrap();
        fs::write(root.join("raw/sessions/turn.md"), "secret").unwrap();
        fs::write(root.join(".obsidian/app.json"), "{}").unwrap();
        fs::write(root.join(".git/config"), "").unwrap();
    }

    fn paths(entries: &[WikiVaultTreeEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.path.as_str()).collect()
    }

    fn find<'a>(entries: &'a [WikiVaultTreeEntry], path: &str) -> Option<&'a WikiVaultTreeEntry> {
        for entry in entries {
            if entry.path == path {
                return Some(entry);
            }
            if let Some(found) = entry.children.as_deref().and_then(|kids| find(kids, path)) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn non_recursive_is_one_level_and_skips_noise() {
        let dir = tempdir().unwrap();
        seed_vault(dir.path());
        let entries = list_vault_tree(dir.path(), "", false, false).unwrap();
        let listed = paths(&entries);
        assert!(listed.contains(&"index.md"));
        assert!(listed.contains(&"work"));
        assert!(listed.contains(&"knowledge"));
        assert!(listed.contains(&"capabilities"));
        assert!(listed.contains(&"sources"));
        assert!(listed.contains(&"journal"));
        assert!(listed.contains(&"raw"));
        assert!(listed.contains(&"AGENTS.md"));
        assert!(listed.contains(&"log.md"));
        assert!(!listed.iter().any(|p| p.contains("obsidian")));
        assert!(!listed.iter().any(|p| p.contains(".git")));
        assert!(entries.iter().all(|e| e.children.is_none()));
        assert!(find(&entries, "work/projects/alpha.md").is_none());
    }

    #[test]
    fn recursive_nests_children_and_hides_only_dot_entries() {
        let dir = tempdir().unwrap();
        seed_vault(dir.path());
        let entries = list_vault_tree(dir.path(), "", true, false).unwrap();
        let alpha = find(&entries, "work/projects/alpha.md").expect("nested note");
        assert!(!alpha.is_dir);
        assert!(find(&entries, "index.md").is_some());
        assert!(find(&entries, "AGENTS.md").is_some());
        assert!(find(&entries, "journal").is_some());
        assert!(find(&entries, "sources").is_some());
        assert!(find(&entries, ".obsidian").is_none());
        assert!(find(&entries, ".obsidian/app.json").is_none());
        assert!(find(&entries, ".git").is_none());
        assert!(find(&entries, "raw").is_some());
        assert!(find(&entries, "raw/sessions/turn.md").is_some());
        let work = find(&entries, "work").expect("work dir");
        assert!(work.is_dir);
        assert!(work.children.is_some());
    }

    #[test]
    fn include_raw_lists_raw_but_still_skips_obsidian() {
        let dir = tempdir().unwrap();
        seed_vault(dir.path());
        let entries = list_vault_tree(dir.path(), "", true, true).unwrap();
        assert!(find(&entries, "raw/sessions/turn.md").is_some());
        assert!(find(&entries, "AGENTS.md").is_some());
        assert!(find(&entries, ".obsidian").is_none());
        assert!(find(&entries, ".git").is_none());
    }

    #[test]
    fn rejects_parent_dir_and_absolute_segments() {
        let dir = tempdir().unwrap();
        seed_vault(dir.path());
        assert!(matches!(
            list_vault_tree(dir.path(), "..", true, false),
            Err(VaultTreeError::UnsafePath)
        ));
        assert!(matches!(
            list_vault_tree(dir.path(), "work/../secrets", false, false),
            Err(VaultTreeError::UnsafePath)
        ));
        assert!(matches!(
            list_vault_tree(dir.path(), "/etc/passwd", false, false),
            Err(VaultTreeError::UnsafePath)
        ));
    }

    #[test]
    fn missing_root_is_empty_missing_rel_is_not_found() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let empty = list_vault_tree(&missing, "", true, false).unwrap();
        assert!(empty.is_empty());
        seed_vault(dir.path());
        assert!(matches!(
            list_vault_tree(dir.path(), "does-not-exist", false, false),
            Err(VaultTreeError::NotFound)
        ));
    }

    #[test]
    fn obsidian_vault_lists_chinese_note_and_hides_only_dot_entries() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(root.join(".obsidian/app.json"), "{}").unwrap();
        fs::write(root.join(".codeg-wiki.json"), "{\"kind\":\"codeg-wiki\"}\n").unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "").unwrap();
        fs::create_dir_all(root.join("日记")).unwrap();
        fs::write(root.join("日记/今天.md"), "# 今天\n").unwrap();
        fs::create_dir_all(root.join("journal")).unwrap();
        fs::write(root.join("journal/note.md"), "# note\n").unwrap();
        fs::create_dir_all(root.join("sources")).unwrap();
        fs::write(root.join("sources/kept.md"), "# kept\n").unwrap();
        fs::create_dir_all(root.join("capabilities")).unwrap();
        fs::write(root.join("capabilities/skill.md"), "# skill\n").unwrap();
        fs::create_dir_all(root.join("knowledge/concepts")).unwrap();
        fs::write(root.join("knowledge/concepts/idea.md"), "# idea\n").unwrap();
        fs::create_dir_all(root.join("work/records")).unwrap();
        fs::write(root.join("work/records/kept.md"), "# kept\n").unwrap();
        let entries = list_vault_tree(root, "", true, false).unwrap();
        assert!(find(&entries, "日记/今天.md").is_some());
        assert!(find(&entries, "journal/note.md").is_some());
        assert!(find(&entries, "sources/kept.md").is_some());
        assert!(find(&entries, "capabilities/skill.md").is_some());
        assert!(find(&entries, "knowledge/concepts/idea.md").is_some());
        assert!(find(&entries, "work/records/kept.md").is_some());
        assert!(find(&entries, ".obsidian").is_none());
        assert!(find(&entries, ".obsidian/app.json").is_none());
        assert!(find(&entries, ".git").is_none());
        assert!(find(&entries, ".codeg-wiki.json").is_none());
    }

    #[test]
    fn explicit_raw_path_can_still_be_listed() {
        let dir = tempdir().unwrap();
        seed_vault(dir.path());
        let entries = list_vault_tree(dir.path(), "raw", false, true).unwrap();
        assert!(paths(&entries).contains(&"raw/sessions"));
    }
}

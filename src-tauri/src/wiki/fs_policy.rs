//! 限制 Wiki 生成页面的路径与类型。
//! 新正文只允许落在 wiki-state 的 generated/ 下，不能按页面类型进入用户库。
//! 模型工具的读取和 staging 权限由 llm 与 FileSystemRuntime 负责。

use std::fs;
use std::path::{Path, PathBuf};

use crate::wiki::paths::{is_safe_vault_relative, join_vault_relative};

/// Page types the host may record on a generated body.
///
/// New files use wiki-state paths (`generated/turn`, `generated/session`,
/// `generated/note`). Page type does not choose a vault directory.
pub const ALLOWED_PAGE_TYPES: &[&str] = &[
    "work-record",
    "decision",
    "outcome",
    "project",
    "area",
    "capability",
    "concept",
    "method",
    "entity",
    "source",
    "daily",
    "index",
    "turn-summary",
    "session-summary",
];

const DENIED_SEGMENTS: &[&str] = &[
    ".obsidian",
    ".git",
    "originals",
    "session_store",
    ".codeg-wiki.lock",
];

/// Obsidian-owned names. Still rejected outside `.obsidian/codeg/`.
const OBSIDIAN_OWNED: &[&str] = &[
    "app.json",
    "appearance.json",
    "workspace.json",
    "core-plugins.json",
    "community-plugins.json",
    "plugins",
    "snippets",
    "themes",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsPolicyError {
    Denied(String),
}

impl std::fmt::Display for FsPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FsPolicyError {}

#[derive(Debug, Clone)]
pub struct WikiFsPolicy {
    vault: PathBuf,
}

impl WikiFsPolicy {
    pub fn new(vault: &Path) -> Self {
        Self {
            vault: vault.to_path_buf(),
        }
    }

    /// Vault-relative compile target. Rejects `raw/`, denied segments, `..`.
    pub fn check_commit_rel(&self, rel: &str) -> Result<PathBuf, FsPolicyError> {
        let rel = rel.trim();
        if rel.is_empty() {
            return Err(FsPolicyError::Denied("empty vault path".into()));
        }
        if !is_safe_vault_relative(rel) {
            return Err(FsPolicyError::Denied(format!(
                "path must be vault-relative without '..' or absolute segments: {rel}"
            )));
        }
        if has_denied_segment(rel) {
            return Err(FsPolicyError::Denied(format!(
                "path is not an allowed wiki target: {rel}"
            )));
        }
        if crate::wiki::paths::is_raw_rel(rel) {
            return Err(FsPolicyError::Denied(
                "compile must not write raw/ evidence".into(),
            ));
        }
        if rel == "AGENTS.md" {
            return Err(FsPolicyError::Denied("AGENTS.md is user-owned".into()));
        }
        if !is_allowed_commit_rel(rel) {
            return Err(FsPolicyError::Denied(format!(
                "path is not an allowed page location: {rel}"
            )));
        }
        let dest = join_vault_relative(&self.vault, rel).map_err(FsPolicyError::Denied)?;
        let vault_canon = canonical_existing_or_parent(&self.vault)?;
        let dest_canon = canonical_existing_or_parent(&dest)?;
        if !dest_canon.starts_with(&vault_canon) {
            return Err(FsPolicyError::Denied(format!(
                "commit path escapes the vault: {rel}"
            )));
        }
        Ok(dest)
    }

    /// Accept a wiki-state generated body. Taxonomy paths are not writable.
    pub fn check_page_type_path(&self, page_type: &str, rel: &str) -> Result<(), FsPolicyError> {
        if !ALLOWED_PAGE_TYPES.contains(&page_type) {
            return Err(FsPolicyError::Denied(format!(
                "page type is not writable: {page_type}"
            )));
        }
        if !crate::wiki::paths::is_generated_body_rel(rel) {
            return Err(FsPolicyError::Denied(format!(
                "new notes stay under wiki-state, not the vault: {rel}"
            )));
        }
        Ok(())
    }
}

pub fn is_allowed_commit_rel(rel: &str) -> bool {
    let rel = rel.trim().trim_start_matches('/');
    if !(rel.ends_with(".md") || rel == "log.md") {
        return false;
    }
    matches!(
        rel,
        "index.md" | "work/index.md" | "capabilities/index.md" | "log.md"
    ) || rel.starts_with("work/projects/")
        || rel.starts_with("work/areas/")
        || rel.starts_with("work/records/")
        || rel.starts_with("work/decisions/")
        || rel.starts_with("work/outcomes/")
        || rel.starts_with("work/turns/")
        || rel.starts_with("work/sessions/")
        || rel.starts_with("capabilities/")
        || rel.starts_with("knowledge/concepts/")
        || rel.starts_with("knowledge/methods/")
        || rel.starts_with("knowledge/entities/")
        || rel.starts_with("sources/")
        || rel.starts_with("journal/")
}

pub fn page_type_matches_rel(page_type: &str, rel: &str) -> bool {
    let rel = rel.trim().trim_start_matches('/');
    match page_type {
        "work-record" => rel.starts_with("work/records/") && rel.ends_with(".md"),
        "decision" => rel.starts_with("work/decisions/") && rel.ends_with(".md"),
        "outcome" => rel.starts_with("work/outcomes/") && rel.ends_with(".md"),
        "project" => rel.starts_with("work/projects/") && rel.ends_with(".md"),
        "area" => rel.starts_with("work/areas/") && rel.ends_with(".md"),
        "capability" => {
            rel.starts_with("capabilities/")
                && rel.ends_with(".md")
                && rel != "capabilities/index.md"
        }
        "concept" => rel.starts_with("knowledge/concepts/") && rel.ends_with(".md"),
        "method" => rel.starts_with("knowledge/methods/") && rel.ends_with(".md"),
        "entity" => rel.starts_with("knowledge/entities/") && rel.ends_with(".md"),
        "source" => rel.starts_with("sources/") && rel.ends_with(".md"),
        "daily" => rel.starts_with("journal/") && rel.ends_with(".md"),
        "index" => {
            matches!(
                rel,
                "index.md" | "work/index.md" | "capabilities/index.md" | "log.md"
            )
        }
        "turn-summary" => rel.starts_with("work/turns/") && rel.ends_with(".md"),
        "session-summary" => rel.starts_with("work/sessions/") && rel.ends_with(".md"),
        _ => false,
    }
}

/// Mint a wiki-state relative path. The page type is not a vault folder.
/// Titles never serve as an existing page's identity.
pub fn allocate_note_rel(
    page_type: &str,
    title: &str,
    note_id: &uuid::Uuid,
) -> Result<String, FsPolicyError> {
    if !matches!(
        page_type,
        "work-record"
            | "decision"
            | "outcome"
            | "project"
            | "area"
            | "capability"
            | "concept"
            | "method"
            | "entity"
    ) {
        return Err(FsPolicyError::Denied(format!(
            "cannot allocate type {page_type}"
        )));
    }
    let mut slug = String::new();
    for ch in title.chars().take(64) {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "note" } else { slug };
    Ok(format!(
        "generated/note/{slug}-{}.md",
        &note_id.simple().to_string()[..12]
    ))
}

fn has_denied_segment(rel: &str) -> bool {
    let segs: Vec<&str> = rel
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect();
    // `.obsidian` stays denied except Codeg's own `.obsidian/codeg/` tree.
    let codeg_tree = segs
        .first()
        .is_some_and(|seg| seg.eq_ignore_ascii_case(".obsidian"))
        && segs
            .get(1)
            .is_some_and(|seg| seg.eq_ignore_ascii_case("codeg"));
    segs.iter().enumerate().any(|(index, seg)| {
        if codeg_tree && index == 0 {
            return false;
        }
        DENIED_SEGMENTS
            .iter()
            .any(|denied| seg.eq_ignore_ascii_case(denied))
            || (!codeg_tree
                && segs
                    .first()
                    .is_some_and(|root| root.eq_ignore_ascii_case(".obsidian"))
                && OBSIDIAN_OWNED
                    .iter()
                    .any(|owned| seg.eq_ignore_ascii_case(owned)))
    })
}

/// 新页面尚不存在时解析最近的真实父目录，再拼回文件名，阻止符号链接把输出带出Wiki。
fn canonical_existing_or_parent(path: &Path) -> Result<PathBuf, FsPolicyError> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|e| {
            FsPolicyError::Denied(format!("cannot canonicalize {}: {e}", path.display()))
        });
    }
    let mut cur = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        let Some(name) = cur.file_name() else {
            return Err(FsPolicyError::Denied(format!(
                "cannot determine parent directory for {}",
                path.display()
            )));
        };
        missing.push(name.to_os_string());
        match cur.parent() {
            Some(parent) if parent != cur.as_path() => cur = parent.to_path_buf(),
            _ => {
                return Err(FsPolicyError::Denied(format!(
                    "parent directory does not exist: {}",
                    path.display()
                )));
            }
        }
        if cur.exists() {
            break;
        }
    }
    let mut canon = fs::canonicalize(&cur).map_err(|e| {
        FsPolicyError::Denied(format!("cannot canonicalize {}: {e}", cur.display()))
    })?;
    for name in missing.iter().rev() {
        canon.push(name);
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_parent_and_absolute_commit_paths() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault);
        assert!(p.check_commit_rel("../secrets.md").is_err());
        assert!(p.check_commit_rel("/etc/passwd").is_err());
        assert!(p.check_commit_rel("/work/projects/page.md").is_err());
        assert!(p.check_commit_rel("raw/sessions/x.md").is_err());
        assert!(p.check_commit_rel("work/raw/sessions/x.md").is_err());
        assert!(p.check_commit_rel(".obsidian/app.json").is_err());
        assert!(p.check_commit_rel("capabilities/ok.md").is_ok());
    }

    #[test]
    fn obsidian_owned_files_stay_denied_and_codeg_prefix_is_allowed() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault);
        for rel in [
            ".obsidian/app.json",
            ".obsidian/appearance.json",
            ".obsidian/workspace.json",
            ".obsidian/core-plugins.json",
            ".obsidian/community-plugins.json",
            ".obsidian/plugins/x.md",
            ".obsidian/snippets/x.md",
            ".obsidian/themes/x.md",
            "notes/.obsidian/codeg/marker.md",
        ] {
            let err = p.check_commit_rel(rel).unwrap_err();
            assert!(
                err.to_string().contains("not an allowed wiki target"),
                "{rel}: {err}"
            );
        }
        let metadata = p
            .check_commit_rel(".obsidian/codeg/metadata/projects/a.md")
            .unwrap_err();
        assert!(
            metadata
                .to_string()
                .contains("not an allowed page location"),
            "{metadata}"
        );
        assert!(!has_denied_segment(".obsidian/codeg/contract.json"));
        assert!(!has_denied_segment(".obsidian/codeg/marker.json"));
        assert!(has_denied_segment(".obsidian/app.json"));
        assert!(has_denied_segment(".obsidian/codeg/.git/hooks.md"));
    }

    #[test]
    fn rejects_path_escape_in_proposal() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault);
        let err = p
            .check_commit_rel("work/projects/../../etc/passwd")
            .unwrap_err();
        assert!(err.to_string().contains("..") || err.to_string().contains("allowed"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected_on_commit() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join("work/projects")).unwrap();
        let secret = dir.path().join("secret.md");
        fs::write(&secret, "outside the vault").unwrap();
        std::os::unix::fs::symlink(&secret, vault.join("work/projects/note.md")).unwrap();
        let policy = WikiFsPolicy::new(&vault);
        assert!(policy.check_commit_rel("work/projects/note.md").is_err());
    }

    #[test]
    fn new_notes_are_not_allocated_into_vault_taxonomy() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault);

        assert!(page_type_matches_rel(
            "turn-summary",
            "work/turns/source-id.md"
        ));
        assert!(page_type_matches_rel(
            "session-summary",
            "work/sessions/c42.md"
        ));
        assert!(!page_type_matches_rel(
            "turn-summary",
            "generated/turn/source-id.md"
        ));
        assert!(p
            .check_page_type_path("turn-summary", "work/turns/source-id.md")
            .is_err());
        assert!(p
            .check_page_type_path("session-summary", "work/sessions/c42.md")
            .is_err());
        assert!(p
            .check_page_type_path("turn-summary", "generated/turn/source-id.md")
            .is_ok());
        assert!(p
            .check_page_type_path("session-summary", "generated/session/c42.md")
            .is_ok());
        assert!(p
            .check_page_type_path("method", "knowledge/methods/x.md")
            .is_err());
        assert!(!vault.join("work").exists());
        assert!(!vault.join("knowledge").exists());

        let rel = allocate_note_rel("method", "Cursor paging", &uuid::Uuid::nil()).unwrap();
        assert!(rel.starts_with("generated/note/"));
        assert!(!rel.contains("knowledge"));
        assert!(!rel.contains("work/"));
        assert!(p.check_page_type_path("method", &rel).is_ok());
        assert!(allocate_note_rel("source", "Imported", &uuid::Uuid::nil()).is_err());
    }
}

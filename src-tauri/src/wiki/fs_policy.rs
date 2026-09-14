//! 限制Wiki生成页面的提交路径、类型与身份分配位置。
//! compile检查候选笔记，commit在写入前再次校验，防止越出Wiki、写原始材料
//! 或覆盖应用维护目录；模型工具的读取/staging权限由llm和FileSystemRuntime负责。

use std::fs;
use std::path::{Path, PathBuf};

use crate::wiki::paths::{is_safe_vault_relative, join_vault_relative};

/// Page types the host may write during compile / memory jobs.
///
/// Memory-note stable paths (host-owned; model does not mint filenames):
/// `work/turns/{source_id}.md`, `work/sessions/c{conversation_id}.md`.
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
        if rel == "raw" || rel.starts_with("raw/") {
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

    pub fn check_page_type_path(&self, page_type: &str, rel: &str) -> Result<(), FsPolicyError> {
        if !ALLOWED_PAGE_TYPES.contains(&page_type) {
            return Err(FsPolicyError::Denied(format!(
                "page type is not writable: {page_type}"
            )));
        }
        let _ = self.check_commit_rel(rel)?;
        if !page_type_matches_rel(page_type, rel) {
            return Err(FsPolicyError::Denied(format!(
                "path {rel} does not match type {page_type}"
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

/// The host mints a path once and persists it in the commit manifest. Titles
/// never serve as an existing page's identity.
pub fn allocate_note_rel(
    page_type: &str,
    title: &str,
    note_id: &uuid::Uuid,
) -> Result<String, FsPolicyError> {
    let directory = match page_type {
        "work-record" => "work/records",
        "decision" => "work/decisions",
        "outcome" => "work/outcomes",
        "project" => "work/projects",
        "area" => "work/areas",
        "capability" => "capabilities",
        "concept" => "knowledge/concepts",
        "method" => "knowledge/methods",
        "entity" => "knowledge/entities",
        "source" => "sources",
        _ => {
            return Err(FsPolicyError::Denied(format!(
                "cannot allocate type {page_type}"
            )))
        }
    };
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
        "{directory}/{slug}-{}.md",
        &note_id.simple().to_string()[..12]
    ))
}

fn has_denied_segment(rel: &str) -> bool {
    rel.split(['/', '\\']).any(|seg| {
        if seg.is_empty() || seg == "." {
            return false;
        }
        DENIED_SEGMENTS.iter().any(|d| seg.eq_ignore_ascii_case(d))
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
        assert!(p.check_commit_rel(".obsidian/app.json").is_err());
        assert!(p.check_commit_rel("capabilities/ok.md").is_ok());
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
    fn turn_and_session_summary_only_match_memory_dirs() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault);

        // Stable paths: work/turns/{source_id}.md, work/sessions/c{conversation_id}.md
        assert!(is_allowed_commit_rel("work/turns/source-id.md"));
        assert!(is_allowed_commit_rel("work/sessions/c42.md"));
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
            "work/sessions/c42.md"
        ));
        assert!(!page_type_matches_rel(
            "session-summary",
            "work/turns/source-id.md"
        ));
        assert!(!page_type_matches_rel("turn-summary", "work/projects/x.md"));
        assert!(!page_type_matches_rel(
            "session-summary",
            "work/records/x.md"
        ));
        assert!(ALLOWED_PAGE_TYPES.contains(&"turn-summary"));
        assert!(ALLOWED_PAGE_TYPES.contains(&"session-summary"));

        assert!(p
            .check_page_type_path("turn-summary", "work/turns/source-id.md")
            .is_ok());
        assert!(p
            .check_page_type_path("session-summary", "work/sessions/c42.md")
            .is_ok());
        assert!(p
            .check_page_type_path("turn-summary", "work/projects/x.md")
            .is_err());
        assert!(p
            .check_page_type_path("session-summary", "work/turns/source-id.md")
            .is_err());
        assert!(p.check_commit_rel("work/turns/source-id.md").is_ok());
        assert!(p.check_commit_rel("work/sessions/c42.md").is_ok());
    }
}

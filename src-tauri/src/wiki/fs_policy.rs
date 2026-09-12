//! Canonical path allowlist for wiki read/glob/grep/stage-write/commit.
//!
//! `PermissionPolicy::AutoAllow` is not a sandbox. Every path is checked here
//! (or the same rules in commit). Empty roots do **not** mean unrestricted.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::wiki::paths::{is_safe_vault_relative, join_vault_relative};

/// Page types the host may write during compile.
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
    Io(String),
}

impl std::fmt::Display for FsPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(s) | Self::Io(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FsPolicyError {}

impl From<io::Error> for FsPolicyError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct WikiFsPolicy {
    vault: PathBuf,
    state_root: PathBuf,
    staging_dir: Option<PathBuf>,
    /// Canonical files or directories the model/host may read.
    allowed_read: Vec<PathBuf>,
}

impl WikiFsPolicy {
    pub fn new(vault: &Path, state_root: &Path) -> Self {
        Self {
            vault: vault.to_path_buf(),
            state_root: state_root.to_path_buf(),
            staging_dir: None,
            allowed_read: Vec::new(),
        }
    }

    /// Compile-job policy: listed notes + AGENTS.md + this job's staging +
    /// listed raw segment files. Writes only under `staging_dir`.
    pub fn for_compile_job(
        vault: &Path,
        state_root: &Path,
        staging_dir: &Path,
        allowed_read_files: &[PathBuf],
    ) -> Self {
        let mut allowed_read = Vec::new();
        for p in allowed_read_files {
            allowed_read.push(p.to_path_buf());
        }
        allowed_read.push(vault.join("AGENTS.md"));
        allowed_read.push(staging_dir.to_path_buf());
        Self {
            vault: vault.to_path_buf(),
            state_root: state_root.to_path_buf(),
            staging_dir: Some(staging_dir.to_path_buf()),
            allowed_read,
        }
    }

    pub fn allow_read(&mut self, path: PathBuf) {
        self.allowed_read.push(path);
    }

    pub fn check_read(&self, path: &Path) -> Result<PathBuf, FsPolicyError> {
        self.check_path(path, false)
    }

    pub fn check_stage_write(&self, path: &Path) -> Result<PathBuf, FsPolicyError> {
        let canonical = self.check_path(path, true)?;
        let Some(staging) = self.staging_dir.as_ref() else {
            return Err(FsPolicyError::Denied(
                "stage writes require a job staging directory".into(),
            ));
        };
        let staging_canon = canonical_existing_or_parent(staging)?;
        if !canonical.starts_with(&staging_canon) {
            return Err(FsPolicyError::Denied(format!(
                "write is outside job staging dir: {}",
                path.display()
            )));
        }
        Ok(canonical)
    }

    /// Vault-relative compile target. Rejects `raw/`, denied segments, `..`.
    pub fn check_commit_rel(&self, rel: &str) -> Result<PathBuf, FsPolicyError> {
        let rel = rel.trim().trim_start_matches('/');
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
        join_vault_relative(&self.vault, rel).map_err(FsPolicyError::Denied)
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

    fn check_path(&self, path: &Path, for_write: bool) -> Result<PathBuf, FsPolicyError> {
        if path.as_os_str().is_empty() {
            return Err(FsPolicyError::Denied("empty path".into()));
        }
        for c in path.components() {
            if matches!(c, Component::ParentDir) {
                return Err(FsPolicyError::Denied(format!(
                    "path must not contain '..': {}",
                    path.display()
                )));
            }
        }
        let display = path.to_string_lossy();
        if has_denied_segment(&display) {
            return Err(FsPolicyError::Denied(format!(
                "path is not allowed: {}",
                path.display()
            )));
        }
        // Never let tools walk originals or the rest of wiki-state except staging.
        if let Ok(state_canon) = canonical_existing_or_parent(&self.state_root) {
            if let Ok(target) = canonical_existing_or_parent(path) {
                if target.starts_with(&state_canon) {
                    let originals = state_canon.join("originals");
                    if target.starts_with(&originals) {
                        return Err(FsPolicyError::Denied(
                            "originals storage is not readable by the wiki worker".into(),
                        ));
                    }
                    if for_write {
                        // writes in state_root only via check_stage_write
                    } else if let Some(staging) = &self.staging_dir {
                        if let Ok(st) = canonical_existing_or_parent(staging) {
                            if !target.starts_with(&st) && !self.is_allowed_read(&target) {
                                return Err(FsPolicyError::Denied(
                                    "wiki-state is not a general read root".into(),
                                ));
                            }
                        }
                    } else if !self.is_allowed_read(&target) {
                        return Err(FsPolicyError::Denied(
                            "wiki-state is not a general read root".into(),
                        ));
                    }
                }
            }
        }

        let target = if for_write {
            canonical_existing_or_parent(path)?
        } else if path.exists() {
            fs::canonicalize(path).map_err(|e| {
                FsPolicyError::Denied(format!("cannot canonicalize {}: {e}", path.display()))
            })?
        } else {
            return Err(FsPolicyError::Denied(format!(
                "path does not exist: {}",
                path.display()
            )));
        };

        if for_write {
            return Ok(target);
        }
        if self.is_allowed_read(&target) {
            Ok(target)
        } else {
            Err(FsPolicyError::Denied(format!(
                "path is outside the wiki allowlist: {}",
                path.display()
            )))
        }
    }

    fn is_allowed_read(&self, canonical: &Path) -> bool {
        // Symlink/hardlink targets that escape the vault or staging are denied
        // even if the link path itself was listed.
        let in_vault = canonical_existing_or_parent(&self.vault)
            .ok()
            .map(|v| canonical.starts_with(&v))
            .unwrap_or(false);
        let in_staging = self
            .staging_dir
            .as_ref()
            .and_then(|s| canonical_existing_or_parent(s).ok())
            .map(|s| canonical.starts_with(&s))
            .unwrap_or(false);
        if !in_vault && !in_staging {
            return false;
        }
        self.allowed_read.iter().any(|root| {
            let ok = canonical_existing_or_parent(root).ok();
            match ok {
                Some(r) if r.is_file() || (r.exists() && !r.is_dir()) => canonical == r,
                Some(r) => canonical.starts_with(&r),
                None => false,
            }
        })
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
            rel.starts_with("capabilities/") && rel.ends_with(".md") && rel != "capabilities/index.md"
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
        _ => false,
    }
}

fn has_denied_segment(rel: &str) -> bool {
    rel.split(['/', '\\']).any(|seg| {
        if seg.is_empty() || seg == "." {
            return false;
        }
        DENIED_SEGMENTS
            .iter()
            .any(|d| seg.eq_ignore_ascii_case(d))
            || seg.eq_ignore_ascii_case("session_store")
    })
}

fn canonical_existing_or_parent(path: &Path) -> Result<PathBuf, FsPolicyError> {
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|e| FsPolicyError::Denied(format!("cannot canonicalize {}: {e}", path.display())));
    }
    let parent = path.parent().ok_or_else(|| {
        FsPolicyError::Denied(format!(
            "cannot determine parent directory for {}",
            path.display()
        ))
    })?;
    if !parent.exists() {
        return Err(FsPolicyError::Denied(format!(
            "parent directory does not exist: {}",
            parent.display()
        )));
    }
    let parent = fs::canonicalize(parent)
        .map_err(|e| FsPolicyError::Denied(format!("cannot canonicalize {}: {e}", parent.display())))?;
    let name = path.file_name().ok_or_else(|| {
        FsPolicyError::Denied(format!("path has no file name: {}", path.display()))
    })?;
    Ok(parent.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn policy(vault: &Path, state: &Path) -> WikiFsPolicy {
        WikiFsPolicy::new(vault, state)
    }

    #[test]
    fn rejects_parent_and_absolute_commit_paths() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = policy(&vault, &dir.path().join("state"));
        assert!(p.check_commit_rel("../secrets.md").is_err());
        assert!(p.check_commit_rel("/etc/passwd").is_err());
        assert!(p.check_commit_rel("raw/sessions/x.md").is_err());
        assert!(p.check_commit_rel(".obsidian/app.json").is_err());
        assert!(p.check_commit_rel("capabilities/ok.md").is_ok());
    }

    #[test]
    fn rejects_path_escape_in_proposal() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let p = policy(&vault, &dir.path().join("state"));
        let err = p.check_commit_rel("work/projects/../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("..") || err.to_string().contains("allowed"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected_on_read() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&state).unwrap();
        let secret = dir.path().join("secret.txt");
        fs::write(&secret, "nope").unwrap();
        let link = vault.join("note.md");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        let mut p = WikiFsPolicy::new(&vault, &state);
        p.allow_read(link.clone());
        assert!(p.check_read(&link).is_err());
    }

    #[test]
    fn staging_write_stays_inside_job_dir() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        let staging = state.join("staging/job-1");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&staging).unwrap();
        let p = WikiFsPolicy::for_compile_job(&vault, &state, &staging, &[]);
        let ok = staging.join("capabilities/a.md");
        fs::create_dir_all(ok.parent().unwrap()).unwrap();
        fs::write(&ok, "x").unwrap();
        assert!(p.check_stage_write(&ok).is_ok());
        let escape = vault.join("capabilities/a.md");
        fs::create_dir_all(escape.parent().unwrap()).unwrap();
        fs::write(&escape, "x").unwrap();
        assert!(p.check_stage_write(&escape).is_err());
    }

    #[test]
    fn originals_are_not_readable() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        let originals = state.join("originals/s1/file.bin");
        fs::create_dir_all(originals.parent().unwrap()).unwrap();
        fs::write(&originals, "bin").unwrap();
        fs::create_dir_all(&vault).unwrap();
        let p = WikiFsPolicy::new(&vault, &state);
        assert!(p.check_read(&originals).is_err());
    }
}

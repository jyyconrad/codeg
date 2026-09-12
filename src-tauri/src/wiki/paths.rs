//! Vault and wiki-state path resolution.
//!
//! Priority (do not use `paths::codeg_home_dir`, which has no `CODEG_DATA_DIR`
//! fallback): explicit settings path → `$CODEG_HOME/wiki` → `$CODEG_DATA_DIR/wiki`
//! → `~/.codeg/wiki`. State root uses the same order with `wiki-state/`.

use std::path::{Path, PathBuf};

const CODEG_DIR_NAME: &str = ".codeg";
const WIKI_DIR_NAME: &str = "wiki";
const WIKI_STATE_DIR_NAME: &str = "wiki-state";

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn default_codeg_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(CODEG_DIR_NAME))
        .unwrap_or_else(|| PathBuf::from(CODEG_DIR_NAME))
}

fn resolve_under(explicit: Option<&str>, leaf: &str) -> PathBuf {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw);
    }
    if let Some(home) = env_dir("CODEG_HOME") {
        return home.join(leaf);
    }
    if let Some(data) = env_dir("CODEG_DATA_DIR") {
        return data.join(leaf);
    }
    default_codeg_dir().join(leaf)
}

/// Vault root: explicit `vault_path` → `$CODEG_HOME/wiki` → `$CODEG_DATA_DIR/wiki`
/// → `~/.codeg/wiki`.
pub fn resolve_vault_path(explicit: Option<&str>) -> PathBuf {
    resolve_under(explicit, WIKI_DIR_NAME)
}

/// State root (originals, logs, staging): same priority with `wiki-state/`.
pub fn resolve_state_root(explicit_vault: Option<&str>) -> PathBuf {
    if let Some(raw) = explicit_vault.map(str::trim).filter(|s| !s.is_empty()) {
        // Explicit vault path is independent of state; state still follows env
        // so originals never land inside a user-chosen Obsidian vault.
        let _ = raw;
    }
    if env_dir("CODEG_HOME").is_some() || env_dir("CODEG_DATA_DIR").is_some() {
        return resolve_under(None, WIKI_STATE_DIR_NAME);
    }
    default_codeg_dir().join(WIKI_STATE_DIR_NAME)
}

/// True when `rel` is a vault-relative path with no `..` or absolute prefix.
pub fn is_safe_vault_relative(rel: &str) -> bool {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return true;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return false;
    }
    use std::path::Component;
    for c in path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return false,
        }
    }
    true
}

/// Join `rel` onto `vault` using `/` internally. Does not canonicalize.
pub fn join_vault_relative(vault: &Path, rel: &str) -> Result<PathBuf, String> {
    if !is_safe_vault_relative(rel) {
        return Err("path must be vault-relative without '..' or absolute segments".into());
    }
    let trimmed = rel.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(vault.to_path_buf());
    }
    let mut out = vault.to_path_buf();
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        out.push(part);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_vault_path_wins() {
        let p = resolve_vault_path(Some("/opt/my-wiki"));
        assert_eq!(p, PathBuf::from("/opt/my-wiki"));
    }

    #[test]
    fn empty_explicit_falls_through() {
        let p = resolve_vault_path(Some("  "));
        assert_ne!(p, PathBuf::from("  "));
    }

    #[test]
    fn codeg_home_beats_data_dir() {
        temp_env::with_vars(
            [
                ("CODEG_HOME", Some("/tmp/codeg-home-wiki-test")),
                ("CODEG_DATA_DIR", Some("/tmp/codeg-data-wiki-test")),
            ],
            || {
                assert_eq!(
                    resolve_vault_path(None),
                    PathBuf::from("/tmp/codeg-home-wiki-test/wiki")
                );
                assert_eq!(
                    resolve_state_root(None),
                    PathBuf::from("/tmp/codeg-home-wiki-test/wiki-state")
                );
            },
        );
    }

    #[test]
    fn data_dir_used_without_codeg_home() {
        temp_env::with_vars(
            [
                ("CODEG_HOME", None),
                ("CODEG_DATA_DIR", Some("/tmp/codeg-data-wiki-test")),
            ],
            || {
                assert_eq!(
                    resolve_vault_path(None),
                    PathBuf::from("/tmp/codeg-data-wiki-test/wiki")
                );
            },
        );
    }

    #[test]
    fn rejects_parent_and_absolute() {
        assert!(!is_safe_vault_relative(".."));
        assert!(!is_safe_vault_relative("work/../secrets"));
        assert!(!is_safe_vault_relative("/etc/passwd"));
        assert!(is_safe_vault_relative("work/projects/a.md"));
        assert!(is_safe_vault_relative(""));
    }
}

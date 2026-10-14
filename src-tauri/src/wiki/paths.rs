//! 解析 Wiki 正文目录与独立运行状态目录，并校验目录内相对路径。
//! 正文优先使用用户配置，再读取 CODEG_HOME，最后使用 ~/.codeg。
//! 不把桌面应用的 CODEG_DATA_DIR（Application Support）当作 Wiki 默认目录。
//! 状态目录始终独立于用户的 Obsidian 目录，供原件、暂存和恢复记录使用。

use std::path::{Path, PathBuf};

const CODEG_DIR_NAME: &str = ".codeg";
const WIKI_DIR_NAME: &str = "wiki";
const WIKI_STATE_DIR_NAME: &str = "wiki-state";

/// Machine metadata lives under Obsidian's dot directory, in Codeg's own folder.
pub const CODEG_OBS_DIR: &str = ".obsidian/codeg";
pub const METADATA_DIR: &str = ".obsidian/codeg/metadata";
pub const LEGACY_METADATA_DIR: &str = "metadata";
pub const CONTRACT_FILE: &str = ".obsidian/codeg/contract.json";
pub const LEGACY_CONTRACT_FILE: &str = ".codeg-wiki-v3.json";
pub const MARKER_FILE: &str = ".obsidian/codeg/marker.json";
pub const LEGACY_MARKER_FILE: &str = ".codeg-wiki.json";

/// New process dumps. Legacy vault-root `raw/` stays readable until it is absent.
pub const RAW_DIR: &str = "work/raw";
pub const LEGACY_RAW_DIR: &str = "raw";
/// New archive placement. Nothing here moves or deletes an existing vault tree.
pub const ARCHIVE_DIR: &str = "work/_archive";
pub const LEGACY_ARCHIVE_DIR: &str = "_archive";

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
    default_codeg_dir().join(leaf)
}

/// Vault root: explicit `vault_path` → `$CODEG_HOME/wiki` → `~/.codeg/wiki`.
pub fn resolve_vault_path(explicit: Option<&str>) -> PathBuf {
    resolve_under(explicit, WIKI_DIR_NAME)
}

pub fn legacy_app_data_wiki_dirs() -> Vec<PathBuf> {
    let Some(data) = env_dir("CODEG_DATA_DIR") else {
        return Vec::new();
    };
    vec![data.join(WIKI_DIR_NAME), data.join("wiki-v2")]
}

pub fn legacy_app_data_state_dirs() -> Vec<PathBuf> {
    let Some(data) = env_dir("CODEG_DATA_DIR") else {
        return Vec::new();
    };
    vec![data.join(WIKI_STATE_DIR_NAME), data.join("wiki-state-v2")]
}

/// State root (originals, logs, staging): same priority with `wiki-state/`.
pub fn resolve_state_root() -> PathBuf {
    resolve_under(None, WIKI_STATE_DIR_NAME)
}

/// Wiki-state relative body written by turn, session, compile, or import jobs.
/// These paths are not vault taxonomy locations.
pub fn is_generated_body_rel(rel: &str) -> bool {
    let rel = rel.trim().trim_start_matches('/');
    if !rel.ends_with(".md") || !is_safe_vault_relative(rel) {
        return false;
    }
    let mut parts = rel.split('/').filter(|part| !part.is_empty());
    let Some(root) = parts.next() else {
        return false;
    };
    let Some(kind) = parts.next() else {
        return false;
    };
    let Some(file) = parts.next() else {
        return false;
    };
    root == "generated"
        && matches!(kind, "turn" | "session" | "note" | "import")
        && parts.next().is_none()
        && !file.starts_with('.')
        && file != "index.md"
}

/// Join a generated body onto the wiki-state root. Does not create directories.
pub fn join_generated_body(state_root: &Path, rel: &str) -> Result<PathBuf, String> {
    if !is_generated_body_rel(rel) {
        return Err("generated body must stay under wiki-state".into());
    }
    let dest = join_vault_relative(state_root, rel.trim().trim_start_matches('/'))?;
    if !dest.starts_with(state_root) {
        return Err("generated body escapes wiki-state".into());
    }
    Ok(dest)
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

fn normalize_rel(rel: &str) -> &str {
    rel.trim().trim_start_matches(['/', '\\'])
}

fn strip_dir<'a>(rel: &'a str, dir: &str) -> Option<&'a str> {
    if rel == dir {
        return Some("");
    }
    rel.strip_prefix(dir)?.strip_prefix('/')
}

fn join_dir(dir: &str, rest: &str) -> String {
    if rest.is_empty() {
        dir.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// `raw/...` and `work/raw/...` in either order: new path first, then vault-root `raw/`.
/// Metadata uses `.obsidian/codeg/metadata/` first, then vault-root `metadata/`.
pub fn stored_read_rels(rel: &str) -> Vec<String> {
    let rel = normalize_rel(rel);
    if let Some(pair) = dual_dirs(rel, RAW_DIR, LEGACY_RAW_DIR) {
        return pair;
    }
    if let Some(pair) = dual_dirs(rel, METADATA_DIR, LEGACY_METADATA_DIR) {
        return pair;
    }
    vec![rel.to_string()]
}

fn dual_dirs(rel: &str, newer: &str, older: &str) -> Option<Vec<String>> {
    let rest = strip_dir(rel, newer).or_else(|| strip_dir(rel, older))?;
    let mut out = vec![join_dir(newer, rest)];
    let legacy = join_dir(older, rest);
    if legacy != out[0] {
        out.push(legacy);
    }
    Some(out)
}

/// First existing file among the dual-read candidates. Missing files yield `None`.
pub fn existing_stored_file(vault: &Path, rel: &str) -> Option<PathBuf> {
    stored_read_rels(rel).into_iter().find_map(|candidate| {
        let path = join_vault_relative(vault, &candidate).ok()?;
        path.is_file().then_some(path)
    })
}

pub fn is_raw_rel(rel: &str) -> bool {
    let rel = normalize_rel(rel);
    strip_dir(rel, RAW_DIR).is_some() || strip_dir(rel, LEGACY_RAW_DIR).is_some()
}

pub fn is_archive_rel(rel: &str) -> bool {
    let rel = normalize_rel(rel);
    strip_dir(rel, ARCHIVE_DIR).is_some() || strip_dir(rel, LEGACY_ARCHIVE_DIR).is_some()
}

/// Vault path for a new session dump. Does not create directories.
pub fn raw_session_rel(source_id: &str) -> String {
    format!("{RAW_DIR}/sessions/{source_id}.md")
}

/// Vault path for a new import dump. Does not create directories.
pub fn raw_import_rel(source_id: &str) -> String {
    format!("{RAW_DIR}/imports/{source_id}.md")
}

/// Rewrite a raw relative path to `work/raw`. Other paths are unchanged.
pub fn raw_write_rel(rel: &str) -> String {
    let rel = normalize_rel(rel);
    if let Some(rest) = strip_dir(rel, RAW_DIR).or_else(|| strip_dir(rel, LEGACY_RAW_DIR)) {
        return join_dir(RAW_DIR, rest);
    }
    rel.to_string()
}

/// Destination for a new archive placement. Does not move, copy, or delete.
pub fn archive_destination_rel(rel: &str) -> Result<String, String> {
    let rel = normalize_rel(rel);
    if !is_safe_vault_relative(rel) {
        return Err("archive path escapes the vault".into());
    }
    if rel.is_empty() {
        return Ok(ARCHIVE_DIR.to_string());
    }
    if let Some(rest) = strip_dir(rel, ARCHIVE_DIR).or_else(|| strip_dir(rel, LEGACY_ARCHIVE_DIR)) {
        return Ok(join_dir(ARCHIVE_DIR, rest));
    }
    Ok(join_dir(ARCHIVE_DIR, rel))
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
                    resolve_state_root(),
                    PathBuf::from("/tmp/codeg-home-wiki-test/wiki-state")
                );
            },
        );
    }

    #[test]
    fn app_data_dir_is_not_the_default_wiki_location() {
        temp_env::with_vars(
            [
                ("CODEG_HOME", None),
                ("CODEG_DATA_DIR", Some("/tmp/codeg-data-wiki-test")),
            ],
            || {
                let vault = resolve_vault_path(None);
                let state = resolve_state_root();
                assert_ne!(vault, PathBuf::from("/tmp/codeg-data-wiki-test/wiki"));
                assert_eq!(vault, default_codeg_dir().join("wiki"));
                assert_eq!(state, default_codeg_dir().join("wiki-state"));
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

    #[test]
    fn generated_bodies_stay_under_wiki_state() {
        let root = PathBuf::from("/tmp/wiki-state");
        assert!(is_generated_body_rel("generated/turn/src.md"));
        assert!(is_generated_body_rel("generated/session/c3.md"));
        assert!(is_generated_body_rel("generated/note/slug-abcdef012345.md"));
        assert!(is_generated_body_rel("generated/import/src.md"));
        assert!(!is_generated_body_rel("work/turns/src.md"));
        assert!(!is_generated_body_rel("work/sessions/c3.md"));
        assert!(!is_generated_body_rel("capabilities/index.md"));
        assert!(!is_generated_body_rel("knowledge/methods/a.md"));
        assert!(!is_generated_body_rel("sources/a.md"));
        assert!(!is_generated_body_rel("journal/a.md"));
        assert!(!is_generated_body_rel("raw/imports/a.md"));
        assert!(!is_generated_body_rel("generated/../secrets.md"));
        assert_eq!(
            join_generated_body(&root, "generated/turn/src.md").unwrap(),
            root.join("generated/turn/src.md")
        );
        assert!(join_generated_body(&root, "work/turns/src.md").is_err());
    }

    #[test]
    fn raw_reads_prefer_work_raw_then_vault_root() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        std::fs::create_dir_all(vault.join("raw/sessions")).unwrap();
        std::fs::write(vault.join("raw/sessions/old.md"), "legacy").unwrap();
        assert_eq!(
            stored_read_rels("raw/sessions/old.md"),
            vec![
                "work/raw/sessions/old.md".to_string(),
                "raw/sessions/old.md".to_string(),
            ]
        );
        assert_eq!(
            existing_stored_file(vault, "work/raw/sessions/old.md")
                .unwrap()
                .file_name()
                .unwrap(),
            "old.md"
        );
        std::fs::create_dir_all(vault.join("work/raw/sessions")).unwrap();
        std::fs::write(vault.join("work/raw/sessions/old.md"), "newer").unwrap();
        let found = existing_stored_file(vault, "raw/sessions/old.md").unwrap();
        assert!(found.ends_with("work/raw/sessions/old.md"));
        assert_eq!(std::fs::read_to_string(&found).unwrap(), "newer");
        assert!(!vault.join("raw/sessions/missing.md").exists());
        assert!(existing_stored_file(vault, "raw/sessions/missing.md").is_none());
    }

    #[test]
    fn metadata_reads_prefer_obsidian_codeg_then_vault_root() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        std::fs::create_dir_all(vault.join("metadata/projects")).unwrap();
        std::fs::write(vault.join("metadata/projects/a.md"), "old").unwrap();
        assert_eq!(
            existing_stored_file(vault, ".obsidian/codeg/metadata/projects/a.md").unwrap(),
            vault.join("metadata/projects/a.md")
        );
        std::fs::create_dir_all(vault.join(".obsidian/codeg/metadata/projects")).unwrap();
        std::fs::write(vault.join(".obsidian/codeg/metadata/projects/a.md"), "new").unwrap();
        let found = existing_stored_file(vault, "metadata/projects/a.md").unwrap();
        assert!(found.ends_with(".obsidian/codeg/metadata/projects/a.md"));
        assert_eq!(
            std::fs::read_to_string(vault.join("metadata/projects/a.md")).unwrap(),
            "old"
        );
    }

    #[test]
    fn new_raw_and_archive_paths_do_not_touch_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(raw_session_rel("src-1"), "work/raw/sessions/src-1.md");
        assert_eq!(raw_import_rel("src-1"), "work/raw/imports/src-1.md");
        assert_eq!(raw_write_rel("raw/imports/a.md"), "work/raw/imports/a.md");
        assert_eq!(
            archive_destination_rel("_archive/2024/note.md").unwrap(),
            "work/_archive/2024/note.md"
        );
        assert_eq!(
            archive_destination_rel("knowledge/old.md").unwrap(),
            "work/_archive/knowledge/old.md"
        );
        assert!(archive_destination_rel("../secret.md").is_err());
        assert!(!dir.path().join("work/raw").exists());
        assert!(!dir.path().join("work/_archive").exists());
        assert!(!dir.path().join("_archive").exists());
    }
}

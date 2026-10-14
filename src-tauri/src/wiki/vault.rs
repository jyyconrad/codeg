//! 初始化个人 Wiki 时只创建库目录和用途标记。
//! 启用功能、选择目录或导入素材时使用；不补分类目录、索引、log.md 或 AGENTS.md。
//! 目录标记只识别 Codeg Wiki 的用途，不划分数据版本。
//! 已有笔记与 .obsidian 配置保持原字节。用户选中的目录可以是空目录、已有标记、Obsidian 库或任意已有文件。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub const FORMAT_MARKER: &str = crate::wiki::paths::MARKER_FILE;
pub const LEGACY_MARKER: &str = crate::wiki::paths::LEGACY_MARKER_FILE;
pub const LEGACY_V2_MARKER: &str = ".codeg-wiki-v2.json";

pub const CONTENT_START: &str = "<!-- codeg-content:start -->";
pub const CONTENT_END: &str = "<!-- codeg-content:end -->";

const STATE_DIRS: &[&str] = &["originals", "logs", "staging", "sources", "extracts"];

fn write_if_missing(path: &Path, contents: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

/// New marker when it exists, otherwise the legacy vault-root marker.
/// Does not create or delete either file.
pub fn marker_read_path(vault: &Path) -> PathBuf {
    let current = vault.join(FORMAT_MARKER);
    if current.is_file() {
        return current;
    }
    let legacy = vault.join(LEGACY_MARKER);
    if legacy.is_file() {
        return legacy;
    }
    current
}

pub fn has_purpose_marker(vault: &Path) -> bool {
    marker_read_path(vault).is_file()
}

/// Create the marker, then ensure directories from the v3 contract or v2 defaults.
pub fn initialize_vault(vault: &Path) -> io::Result<()> {
    fs::create_dir_all(vault)?;
    write_if_missing(&vault.join(FORMAT_MARKER), "{\"kind\":\"codeg-wiki\"}\n")?;
    let contract = crate::wiki::contract::load(vault);
    crate::wiki::contract::ensure_layout(vault, &contract)?;
    crate::wiki::contract::converge(vault, &contract)?;
    tracing::info!("[wiki] layout: .obsidian is not written except codeg/");
    Ok(())
}

fn path_escapes(vault: &Path) -> bool {
    vault
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

/// Accept an explicit directory choice.
///
/// A missing path can be created. An existing directory is accepted when it is
/// empty, already has `.codeg-wiki.json`, contains `.obsidian/`, or already
/// holds files. A normal Obsidian vault does not need `work/` or `capabilities/`.
/// `..` segments are still rejected so the choice cannot escape its parent.
pub fn validate_new_location(vault: &Path) -> io::Result<()> {
    if path_escapes(vault) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "vault path escapes its parent",
        ));
    }
    if !vault.exists() {
        return Ok(());
    }
    if !vault.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "vault path must be a directory",
        ));
    }
    Ok(())
}

pub fn initialize_state_root(state_root: &Path) -> io::Result<()> {
    fs::create_dir_all(state_root)?;
    for dir in STATE_DIRS {
        fs::create_dir_all(state_root.join(dir))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn initialize_does_not_overwrite_existing_notes() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::create_dir_all(vault).unwrap();
        fs::write(vault.join("index.md"), "human text").unwrap();
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        fs::write(vault.join(".obsidian/app.json"), "{\"x\":1}").unwrap();
        let obsidian = fs::read(vault.join(".obsidian/app.json")).unwrap();
        initialize_vault(vault).unwrap();
        assert_eq!(fs::read(vault.join("index.md")).unwrap(), b"human text");
        assert_eq!(
            fs::read(vault.join(".obsidian/app.json")).unwrap(),
            obsidian
        );
        assert!(vault.join("work/projects").is_dir());
        assert!(vault.join("work/turns").is_dir());
        assert!(!vault.join("work/areas").exists());
        assert!(!vault.join("work/outcomes").exists());
        assert!(!vault.join("work/records").exists());
        assert!(!vault.join("work/raw").exists());
        assert!(vault.join("capabilities").is_dir());
        assert!(!vault.join("AGENTS.md").exists());
        assert!(!vault.join("log.md").exists());
        assert_eq!(
            fs::read_to_string(vault.join(FORMAT_MARKER)).unwrap(),
            "{\"kind\":\"codeg-wiki\"}\n"
        );
    }

    #[test]
    fn initialize_creates_required_v2_scaffolds() {
        let dir = tempdir().unwrap();
        let decisions =
            crate::wiki::contract::ensure_layout(dir.path(), &crate::wiki::contract::v2_contract())
                .unwrap();
        assert!(decisions
            .iter()
            .any(|decision| decision.action == "create" && decision.scaffold == "work"));
        assert!(dir.path().join("journal").is_dir());
        assert!(dir.path().join("work/turns").is_dir());
    }
    #[test]
    fn initialized_wiki_reopens_with_a_purpose_marker_without_version_checks() {
        let dir = tempdir().unwrap();
        initialize_vault(dir.path()).unwrap();
        assert_eq!(FORMAT_MARKER, ".obsidian/codeg/marker.json");
        assert_eq!(LEGACY_MARKER, ".codeg-wiki.json");
        let marker: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(FORMAT_MARKER)).unwrap())
                .unwrap();
        assert_eq!(marker, serde_json::json!({"kind":"codeg-wiki"}));
        fs::create_dir_all(dir.path().join("work/records")).unwrap();
        fs::write(
            dir.path().join("work/records/preserved.md"),
            "Existing personal note",
        )
        .unwrap();
        validate_new_location(dir.path()).unwrap();
        initialize_vault(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("work/records/preserved.md")).unwrap(),
            "Existing personal note"
        );
    }

    #[test]
    fn markdown_without_codeg_layout_is_accepted_without_modification() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.md"), "existing content").unwrap();
        validate_new_location(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("index.md")).unwrap(),
            "existing content"
        );
        assert!(!dir.path().join(FORMAT_MARKER).exists());
        assert!(!dir.path().join("work").exists());
        assert!(!dir.path().join("capabilities").exists());
    }

    #[test]
    fn empty_directory_and_existing_files_are_accepted() {
        let empty = tempdir().unwrap();
        validate_new_location(empty.path()).unwrap();
        let files = tempdir().unwrap();
        fs::write(files.path().join("notes.txt"), "plain").unwrap();
        validate_new_location(files.path()).unwrap();
        assert_eq!(
            fs::read_to_string(files.path().join("notes.txt")).unwrap(),
            "plain"
        );
        assert!(!files.path().join(FORMAT_MARKER).exists());
    }

    #[test]
    fn obsidian_vault_with_chinese_note_is_accepted_without_scaffold_or_app_json_edits() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(
            vault.join(crate::wiki::contract::LEGACY_V3_FILE),
            r#"{"version":3,"layout":{"scaffolds":{"capabilities":"disabled","journal":"disabled","knowledge":"disabled","sources":"disabled","raw":"lazy","work":"required","metadata":"required"},"language":"zh-CN"}}"#,
        )
        .unwrap();
        fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let app_json = "{\"userIgnoreFilters\":[\"attachments/\"]}\n";
        fs::write(vault.join(".obsidian/app.json"), app_json).unwrap();
        fs::create_dir_all(vault.join("日记")).unwrap();
        let note = "# 今天\n这是已有的中文笔记。\n";
        fs::write(vault.join("日记/今天.md"), note).unwrap();
        validate_new_location(vault).unwrap();
        initialize_vault(vault).unwrap();
        assert_eq!(
            fs::read_to_string(vault.join(".obsidian/app.json")).unwrap(),
            app_json
        );
        assert_eq!(
            fs::read_to_string(vault.join("日记/今天.md")).unwrap(),
            note
        );
        assert!(vault.join("work/projects").is_dir());
        assert!(vault.join("work/turns").is_dir());
        assert!(vault.join("work/sessions").is_dir());
        assert!(vault.join("work/decisions").is_dir());
        assert!(vault.join(crate::wiki::paths::METADATA_DIR).is_dir());
        assert!(!vault.join("metadata").exists());
        for rel in [
            "capabilities",
            "journal",
            "knowledge",
            "sources",
            "raw",
            "work/raw",
            "work/areas",
            "work/outcomes",
            "work/records",
            "index.md",
            "capabilities/index.md",
        ] {
            assert!(!vault.join(rel).exists(), "{rel}");
        }
        assert_eq!(
            fs::read_to_string(vault.join(FORMAT_MARKER)).unwrap(),
            "{\"kind\":\"codeg-wiki\"}\n"
        );
    }

    #[test]
    fn parent_segments_are_rejected_as_path_escape() {
        let err = validate_new_location(Path::new("notes/../secrets")).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("escapes"));
        assert!(!message.contains("choose an empty directory or a Codeg Wiki directory"));
        assert!(validate_new_location(Path::new("/tmp/wiki-parent-escape/../outside")).is_err());
    }

    #[test]
    fn existing_file_wiki_layout_is_the_same_wiki_and_keeps_notes() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work/records")).unwrap();
        fs::write(dir.path().join("index.md"), "Personal Wiki home").unwrap();
        fs::write(
            dir.path().join("work/records/kept.md"),
            "existing file wiki",
        )
        .unwrap();
        validate_new_location(dir.path()).unwrap();
        initialize_vault(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("work/records/kept.md")).unwrap(),
            "existing file wiki"
        );
        let marker: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(FORMAT_MARKER)).unwrap())
                .unwrap();
        assert_eq!(marker, serde_json::json!({"kind":"codeg-wiki"}));
    }

    #[test]
    fn legacy_directory_wiki_v2_marker_is_the_same_wiki() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("capabilities")).unwrap();
        fs::write(dir.path().join("index.md"), "Directory wiki home").unwrap();
        fs::write(dir.path().join(LEGACY_V2_MARKER), "{\"version\":2}\n").unwrap();
        fs::write(
            dir.path().join("capabilities/kept.md"),
            "existing directory wiki",
        )
        .unwrap();
        validate_new_location(dir.path()).unwrap();
        initialize_vault(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("capabilities/kept.md")).unwrap(),
            "existing directory wiki"
        );
        assert!(dir.path().join(FORMAT_MARKER).is_file());
    }

    #[test]
    fn legacy_marker_is_read_and_kept_when_the_new_marker_is_written() {
        let dir = tempdir().unwrap();
        let legacy = "{\"kind\":\"codeg-wiki\",\"note\":\"keep\"}\n";
        fs::write(dir.path().join(LEGACY_MARKER), legacy).unwrap();
        assert_eq!(marker_read_path(dir.path()), dir.path().join(LEGACY_MARKER));
        initialize_vault(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(LEGACY_MARKER)).unwrap(),
            legacy
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(FORMAT_MARKER)).unwrap(),
            "{\"kind\":\"codeg-wiki\"}\n"
        );
        assert_eq!(marker_read_path(dir.path()), dir.path().join(FORMAT_MARKER));
    }
}

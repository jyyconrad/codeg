//! 初始化个人 Wiki 的目录、首页模板、偏好文件和运行状态子目录。
//! 启用功能、选择目录或导入素材时使用；已有笔记与 .obsidian 配置不覆盖。
//! 目录标记只识别 Codeg Wiki 的用途，不划分数据版本。
//! 文件 Wiki 笔记布局与旧目录 Wiki 标记视为同一份库；现有正文始终保留。

use std::fs;
use std::io;
use std::path::Path;

pub const FORMAT_MARKER: &str = ".codeg-wiki.json";
pub const LEGACY_V2_MARKER: &str = ".codeg-wiki-v2.json";

pub const CONTENT_START: &str = "<!-- codeg-content:start -->";
pub const CONTENT_END: &str = "<!-- codeg-content:end -->";

const VAULT_DIRS: &[&str] = &[
    "work/projects",
    "work/areas",
    "work/records",
    "work/decisions",
    "work/outcomes",
    "work/turns",
    "work/sessions",
    "capabilities",
    "knowledge/concepts",
    "knowledge/methods",
    "knowledge/entities",
    "sources",
    "raw/sessions",
    "raw/imports",
    "journal",
];

const STATE_DIRS: &[&str] = &["originals", "logs", "staging", "sources", "extracts"];

fn wrapped(body: &str) -> String {
    format!("{CONTENT_START}\n{body}\n{CONTENT_END}\n")
}

fn write_if_missing(path: &Path, contents: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

/// Create vault layout from spec §5. Existing notes / `.obsidian` are left alone.
pub fn initialize_vault(vault: &Path) -> io::Result<()> {
    fs::create_dir_all(vault)?;
    write_if_missing(&vault.join(FORMAT_MARKER), "{\"kind\":\"codeg-wiki\"}\n")?;
    for dir in VAULT_DIRS {
        fs::create_dir_all(vault.join(dir))?;
    }

    write_if_missing(
        &vault.join("index.md"),
        &format!(
            "---\ntitle: \"Personal Wiki\"\ntype: index\ntags:\n  - \"type/index\"\n---\n\n# Personal Wiki\n\n{}",
            wrapped("- [[work/index|Work]]\n- [[capabilities/index|Capabilities]]\n- [[sources|Sources]]")
        ),
    )?;
    write_if_missing(
        &vault.join("work/index.md"),
        &format!(
            "---\ntitle: \"Work\"\ntype: index\ntags:\n  - \"type/index\"\n---\n\n# Work\n\n{}",
            wrapped("- Projects, areas, records, decisions, outcomes.")
        ),
    )?;
    write_if_missing(
        &vault.join("capabilities/index.md"),
        &format!(
            "---\ntitle: \"Capabilities\"\ntype: index\ntags:\n  - \"type/index\"\n---\n\n# Capabilities\n\n{}",
            wrapped("- Capability catalog, evidence gaps, next practice.")
        ),
    )?;
    write_if_missing(
        &vault.join("log.md"),
        &format!(
            "---\ntitle: \"Wiki log\"\ntype: index\n---\n\n# Wiki log\n\n{}",
            wrapped("")
        ),
    )?;
    write_if_missing(
        &vault.join("AGENTS.md"),
        concat!(
            "# Wiki preferences\n\n",
            "This file is user-editable organization preference. It cannot raise tool ",
            "permissions, expand read scope, or require network access.\n"
        ),
    )?;
    Ok(())
}

fn has_unified_marker(vault: &Path) -> io::Result<bool> {
    let marker = vault.join(FORMAT_MARKER);
    if !marker.is_file() {
        return Ok(false);
    }
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(marker)?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(value.get("kind").and_then(|v| v.as_str()) == Some("codeg-wiki"))
}

fn has_legacy_directory_wiki_marker(vault: &Path) -> bool {
    vault.join(LEGACY_V2_MARKER).is_file()
}

/// Markdown notes in `work/` or `capabilities/` are the same Wiki as a marked vault.
fn has_file_wiki_layout(vault: &Path) -> bool {
    vault.join("index.md").is_file()
        && (vault.join("work").is_dir() || vault.join("capabilities").is_dir())
}

/// 空目录可初始化。带统一标记、旧目录 Wiki 标记，或已有笔记布局的目录是同一份 Wiki。
/// 不接管未经选择的普通项目目录。
pub fn validate_new_location(vault: &Path) -> io::Result<()> {
    if !vault.exists() {
        return Ok(());
    }
    if has_unified_marker(vault)?
        || has_legacy_directory_wiki_marker(vault)
        || has_file_wiki_layout(vault)
        || fs::read_dir(vault)?.next().is_none()
    {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "choose an empty directory or a Codeg Wiki directory",
    ))
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
        initialize_vault(vault).unwrap();
        assert_eq!(
            fs::read_to_string(vault.join("index.md")).unwrap(),
            "human text"
        );
        assert_eq!(
            fs::read_to_string(vault.join(".obsidian/app.json")).unwrap(),
            "{\"x\":1}"
        );
        assert!(vault.join("work/projects").is_dir());
        assert!(vault.join("work/turns").is_dir());
        assert!(vault.join("work/sessions").is_dir());
        assert!(vault.join("raw/sessions").is_dir());
        assert!(vault.join("AGENTS.md").is_file());
        let log = fs::read_to_string(vault.join("log.md")).unwrap();
        assert!(log.contains(CONTENT_START));
        assert!(log.contains(CONTENT_END));
    }
    #[test]
    fn initialized_wiki_reopens_with_a_purpose_marker_without_version_checks() {
        let dir = tempdir().unwrap();
        initialize_vault(dir.path()).unwrap();
        assert_eq!(FORMAT_MARKER, ".codeg-wiki.json");
        let marker: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(FORMAT_MARKER)).unwrap())
                .unwrap();
        assert_eq!(marker, serde_json::json!({"kind":"codeg-wiki"}));
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
    fn selecting_an_unmarked_nonempty_folder_is_rejected_without_modification() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("index.md"), "existing content").unwrap();
        assert!(validate_new_location(dir.path()).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("index.md")).unwrap(),
            "existing content"
        );
        assert!(!dir.path().join(FORMAT_MARKER).exists());
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
        fs::write(dir.path().join(".codeg-wiki-v2.json"), "{\"version\":2}\n").unwrap();
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
}

//! Vault directory initialization. Never overwrite existing notes or `.obsidian`.

use std::fs;
use std::io;
use std::path::Path;

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

const STATE_DIRS: &[&str] = &["originals", "logs", "staging"];

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
}

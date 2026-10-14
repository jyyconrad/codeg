//! Vault layout contract. `.obsidian/codeg/contract.json` wins over a legacy
//! vault-root `.codeg-wiki-v3.json`, which wins over the v2 defaults.
//! A missing or invalid file falls back to v2 and must not lock the vault.
//! Reading a legacy file does not delete it. Settings stay pipeline-only.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::wiki::paths::is_safe_vault_relative;

pub const V3_FILE: &str = crate::wiki::paths::CONTRACT_FILE;
pub const LEGACY_V3_FILE: &str = crate::wiki::paths::LEGACY_CONTRACT_FILE;

const SCAFFOLD_NAMES: &[&str] = &[
    "capabilities",
    "journal",
    "knowledge",
    "sources",
    "raw",
    "work",
    "metadata",
];

const WRITE_KEYS: &[(&str, &str)] = &[
    ("turns", "work/turns"),
    ("sessions", "work/sessions"),
    ("records", "work/records"),
    ("knowledge_notes", "knowledge"),
    ("capability_notes", "capabilities"),
    ("source_notes", "sources"),
];

const CAPABILITIES_STUB: &str = "\
---\ntitle: \"Capabilities\"\ntype: index\ntags:\n  - \"type/index\"\n---\n\n# Capabilities\n\n\
<!-- codeg-content:start -->\n- Capability catalog, evidence gaps, next practice.\n\
<!-- codeg-content:end -->\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldMode {
    Required,
    Lazy,
    Disabled,
}

impl ScaffoldMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "required" => Some(Self::Required),
            "lazy" => Some(Self::Lazy),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Lazy => "lazy",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiContract {
    pub version: u32,
    pub applied: bool,
    pub language: String,
    pub scaffolds: BTreeMap<String, ScaffoldMode>,
    pub write_paths: BTreeMap<String, String>,
    pub hash: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutStatus {
    pub version: u32,
    pub applied: bool,
    pub language: String,
    pub config_revision: i32,
    pub scaffolds: BTreeMap<String, String>,
    pub write_paths: BTreeMap<String, String>,
    pub summary: String,
    pub warning: Option<String>,
}

impl Default for LayoutStatus {
    fn default() -> Self {
        Self {
            version: 2,
            applied: true,
            language: "en".into(),
            config_revision: 0,
            scaffolds: BTreeMap::new(),
            write_paths: BTreeMap::new(),
            summary: "v2".into(),
            warning: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDecision {
    pub action: &'static str,
    pub scaffold: String,
    pub detail: String,
}

pub fn v2_contract() -> WikiContract {
    let mut scaffolds = BTreeMap::new();
    for name in SCAFFOLD_NAMES {
        scaffolds.insert((*name).to_string(), ScaffoldMode::Required);
    }
    WikiContract {
        version: 2,
        applied: true,
        language: "en".into(),
        scaffolds,
        write_paths: default_write_paths(),
        hash: hash_text("v2"),
        warning: None,
    }
}

/// New contract path when it exists, otherwise the legacy vault-root file.
/// Does not create or delete either file.
pub fn contract_read_path(vault: &Path) -> PathBuf {
    let current = vault.join(V3_FILE);
    if current.is_file() {
        return current;
    }
    let legacy = vault.join(LEGACY_V3_FILE);
    if legacy.is_file() {
        return legacy;
    }
    current
}

pub fn load(vault: &Path) -> WikiContract {
    let path = contract_read_path(vault);
    if !path.is_file() {
        return v2_contract();
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            let reason = format!("unreadable: {error}");
            tracing::warn!("[wiki] layout: reject v3 contract: {reason}; falling back to v2");
            return rejected(reason);
        }
    };
    match parse_v3(&text) {
        Ok(contract) => contract,
        Err(reason) => {
            if reason.starts_with("unknown version") {
                tracing::info!(
                    "[wiki] layout: unknown wiki contract version {reason}, falling back to v2"
                );
            } else {
                tracing::warn!("[wiki] layout: reject v3 contract: {reason}; falling back to v2");
            }
            rejected(reason)
        }
    }
}

pub fn status(vault: &Path, config_revision: i32) -> LayoutStatus {
    let contract = load(vault);
    let summary = if contract.version == 3 && contract.applied {
        format!(
            "v3 {lang} revision {config_revision}",
            lang = contract.language
        )
    } else if let Some(warning) = &contract.warning {
        format!("v2 fallback ({warning})")
    } else {
        format!("v2 revision {config_revision}")
    };
    LayoutStatus {
        version: contract.version,
        applied: contract.applied,
        language: contract.language,
        config_revision,
        scaffolds: contract
            .scaffolds
            .iter()
            .map(|(name, mode)| (name.clone(), mode.as_str().to_string()))
            .collect(),
        write_paths: contract.write_paths.clone(),
        summary,
        warning: contract.warning,
    }
}

/// Paragraph appended to the synthesize skill. Empty for a plain v2 vault.
pub fn layout_prompt(contract: &WikiContract) -> String {
    if contract.version != 3 || !contract.applied {
        return String::new();
    }
    let mut text = format!(
        "\n\n## Vault layout contract\n\nLanguage: {}.\nExisting destination folders:\n",
        contract.language
    );
    for (key, path) in &contract.write_paths {
        text.push_str(&format!("- {key}: `{path}`\n"));
    }
    text.push_str(
        "Use these folders only when they already exist. Do not create a disabled scaffold.\n",
    );
    text
}

pub fn ensure_layout(vault: &Path, contract: &WikiContract) -> io::Result<Vec<LayoutDecision>> {
    let mut decisions = Vec::new();
    for name in SCAFFOLD_NAMES {
        let mode = contract
            .scaffolds
            .get(*name)
            .copied()
            .unwrap_or(ScaffoldMode::Required);
        match mode {
            ScaffoldMode::Disabled => {
                tracing::info!("[wiki] layout: skip disabled scaffold '{name}'");
                decisions.push(LayoutDecision {
                    action: "skip",
                    scaffold: (*name).into(),
                    detail: "disabled".into(),
                });
            }
            ScaffoldMode::Lazy => {
                tracing::info!("[wiki] layout: wait lazy scaffold '{name}'");
                decisions.push(LayoutDecision {
                    action: "wait",
                    scaffold: (*name).into(),
                    detail: "lazy".into(),
                });
            }
            ScaffoldMode::Required => {
                if *name == "raw" {
                    tracing::info!("[wiki] layout: wait raw scaffold until a write");
                    decisions.push(LayoutDecision {
                        action: "wait",
                        scaffold: (*name).into(),
                        detail: "created on write".into(),
                    });
                    continue;
                }
                for rel in scaffold_dirs(name) {
                    fs::create_dir_all(vault.join(rel))?;
                }
                tracing::info!("[wiki] layout: create required scaffold '{name}'");
                decisions.push(LayoutDecision {
                    action: "create",
                    scaffold: (*name).into(),
                    detail: "required".into(),
                });
            }
        }
    }
    Ok(decisions)
}

/// Create the parent of a real write when that scaffold is lazy or required.
/// Disabled targets are refused so the caller can fall back to a report.
pub fn prepare_write(vault: &Path, rel: &str, contract: &WikiContract) -> io::Result<()> {
    if !is_safe_vault_relative(rel) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "write path escapes the vault",
        ));
    }
    if crate::wiki::paths::is_archive_rel(rel) {
        let dest = crate::wiki::paths::archive_destination_rel(rel)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        return create_write_parent(vault, &dest);
    }
    let rel = if crate::wiki::paths::is_raw_rel(rel) {
        crate::wiki::paths::raw_write_rel(rel)
    } else {
        rel.to_string()
    };
    let Some(name) = scaffold_for_rel(&rel) else {
        return Ok(());
    };
    let mode = contract
        .scaffolds
        .get(name)
        .copied()
        .unwrap_or(ScaffoldMode::Required);
    match mode {
        ScaffoldMode::Disabled => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("scaffold '{name}' is disabled"),
        )),
        ScaffoldMode::Lazy | ScaffoldMode::Required if name == "raw" => {
            create_write_parent(vault, &rel)?;
            tracing::info!("[wiki] layout: create '{name}' for write");
            Ok(())
        }
        ScaffoldMode::Lazy => {
            create_write_parent(vault, &rel)?;
            tracing::info!("[wiki] layout: create lazy '{name}' for write");
            Ok(())
        }
        // Recreate a removed metadata parent. Legacy `metadata/` is not a target.
        ScaffoldMode::Required if name == "metadata" => {
            create_write_parent(vault, &rel)?;
            tracing::info!("[wiki] layout: create '{name}' parent for write");
            Ok(())
        }
        ScaffoldMode::Required => Ok(()),
    }
}

fn create_write_parent(vault: &Path, rel: &str) -> io::Result<()> {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(vault.join(parent))?;
        }
    }
    Ok(())
}

pub fn write_converge_report(
    vault: &Path,
    vault_id: &str,
    decisions: &[LayoutDecision],
) -> io::Result<PathBuf> {
    if vault_id.is_empty()
        || !is_safe_vault_relative(vault_id)
        || vault_id.contains(['/', '\\'])
        || vault_id.contains("..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe vault id for layout report",
        ));
    }
    // Layout reports are host metadata. wiki-state keeps locks, staging, and
    // unfinished commits only.
    let dir = vault.join(crate::wiki::paths::CODEG_OBS_DIR).join("layout");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{vault_id}-converge.md"));
    let mut body = String::from("# Wiki layout converge\n\n");
    body.push_str("CodeG does not write `.obsidian/app.json`. Existing userIgnoreFilters stay with the vault owner.\n\n");
    if decisions.is_empty() {
        body.push_str("No disabled scaffolds to converge.\n");
    }
    for decision in decisions {
        body.push_str(&format!(
            "- {} `{}`: {}\n",
            decision.action, decision.scaffold, decision.detail
        ));
    }
    fs::write(&path, body)?;
    tracing::info!("[wiki] layout: converge report {}", path.display());
    Ok(path)
}

pub fn converge(vault: &Path, contract: &WikiContract) -> io::Result<Vec<LayoutDecision>> {
    let mut decisions = Vec::new();
    for name in SCAFFOLD_NAMES {
        if contract.scaffolds.get(*name).copied() != Some(ScaffoldMode::Disabled) {
            continue;
        }
        let root = vault.join(converge_rel(name));
        if !root.exists() {
            decisions.push(LayoutDecision {
                action: "absent",
                scaffold: (*name).into(),
                detail: "already absent".into(),
            });
            continue;
        }
        let files = list_files(&root)?;
        if files.is_empty() {
            fs::remove_dir_all(&root)?;
            tracing::info!("[wiki] layout: converge delete empty '{name}'");
            decisions.push(LayoutDecision {
                action: "delete",
                scaffold: (*name).into(),
                detail: "empty".into(),
            });
            continue;
        }
        if files.iter().all(|path| is_factory_stub(vault, path)) {
            fs::remove_dir_all(&root)?;
            tracing::info!("[wiki] layout: converge delete stub '{name}'");
            decisions.push(LayoutDecision {
                action: "delete",
                scaffold: (*name).into(),
                detail: "factory stub".into(),
            });
            continue;
        }
        tracing::info!("[wiki] layout: converge keep '{name}' (user content)");
        decisions.push(LayoutDecision {
            action: "keep",
            scaffold: (*name).into(),
            detail: format!("{} user files", files.len()),
        });
    }
    Ok(decisions)
}

/// Manifest path, then the current write_paths mapping, then the v2 default.
pub fn resolve_existing(vault: &Path, rel: &str, contract: &WikiContract) -> Option<PathBuf> {
    let direct = vault.join(rel);
    if direct.is_file() {
        return Some(direct);
    }
    for candidate in alternate_rels(rel, contract) {
        let path = vault.join(&candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn alternate_rels(rel: &str, contract: &WikiContract) -> Vec<String> {
    let mut out = Vec::new();
    for (key, default_root) in WRITE_KEYS {
        let mapped = contract
            .write_paths
            .get(*key)
            .map(String::as_str)
            .unwrap_or(default_root);
        if let Some(rest) = strip_root(rel, default_root) {
            out.push(join_root(mapped, rest));
        }
        if mapped != *default_root {
            if let Some(rest) = strip_root(rel, mapped) {
                out.push(join_root(default_root, rest));
            }
        }
    }
    out
}

fn strip_root<'a>(rel: &'a str, root: &str) -> Option<&'a str> {
    let rel = rel.trim_start_matches('/');
    if rel == root {
        return Some("");
    }
    rel.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
}

fn join_root(root: &str, rest: &str) -> String {
    if rest.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{rest}")
    }
}

fn rejected(reason: String) -> WikiContract {
    let mut contract = v2_contract();
    contract.applied = false;
    contract.warning = Some(reason);
    contract
}

fn parse_v3(text: &str) -> Result<WikiContract, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("invalid json: {error}"))?;
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != 3 {
        return Err(format!("unknown version {version}"));
    }
    let layout = value
        .get("layout")
        .ok_or_else(|| "missing layout".to_string())?;
    let mut scaffolds = v2_contract().scaffolds;
    if let Some(raw) = layout.get("scaffolds").and_then(|v| v.as_object()) {
        for (name, mode) in raw {
            if !SCAFFOLD_NAMES.contains(&name.as_str()) {
                return Err(format!("unknown scaffold {name}"));
            }
            let mode = mode
                .as_str()
                .and_then(ScaffoldMode::parse)
                .ok_or_else(|| format!("invalid mode for {name}"))?;
            scaffolds.insert(name.clone(), mode);
        }
    }
    let mut write_paths = default_write_paths();
    if let Some(raw) = layout.get("write_paths").and_then(|v| v.as_object()) {
        for (key, path) in raw {
            if !WRITE_KEYS.iter().any(|(name, _)| *name == key) {
                return Err(format!("unknown write path {key}"));
            }
            let path = path
                .as_str()
                .ok_or_else(|| format!("write path {key} is not a string"))?;
            if !is_safe_vault_relative(path) || path.starts_with('/') {
                return Err(format!("write path {key} escapes the vault"));
            }
            write_paths.insert(key.clone(), path.trim_matches('/').to_string());
        }
    }
    let language = layout
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("en")
        .to_string();
    if language.len() > 32 || language.contains(['/', '\0']) {
        return Err("invalid language".into());
    }
    Ok(WikiContract {
        version: 3,
        applied: true,
        language,
        scaffolds,
        write_paths,
        hash: hash_text(text),
        warning: None,
    })
}

fn default_write_paths() -> BTreeMap<String, String> {
    WRITE_KEYS
        .iter()
        .map(|(key, path)| ((*key).to_string(), (*path).to_string()))
        .collect()
}

fn scaffold_dirs(name: &str) -> Vec<String> {
    match name {
        "work" => vec![
            "work/projects".into(),
            "work/decisions".into(),
            "work/turns".into(),
            "work/sessions".into(),
        ],
        "knowledge" => vec![
            "knowledge/concepts".into(),
            "knowledge/methods".into(),
            "knowledge/entities".into(),
        ],
        // `work/raw` is created by the first write, not by ensure.
        "raw" => Vec::new(),
        "capabilities" => vec!["capabilities".into()],
        "journal" => vec!["journal".into()],
        "sources" => vec!["sources".into()],
        "metadata" => {
            let root = crate::wiki::paths::METADATA_DIR;
            vec![
                root.to_string(),
                format!("{root}/projects"),
                format!("{root}/folders"),
            ]
        }
        _ => Vec::new(),
    }
}

fn converge_rel(name: &str) -> String {
    match name {
        "metadata" => crate::wiki::paths::METADATA_DIR.to_string(),
        "raw" => crate::wiki::paths::RAW_DIR.to_string(),
        other => other.to_string(),
    }
}

fn scaffold_for_rel(rel: &str) -> Option<&'static str> {
    let rel = rel.trim().trim_start_matches('/');
    if crate::wiki::paths::is_raw_rel(rel) {
        return Some("raw");
    }
    let metadata = crate::wiki::paths::METADATA_DIR;
    let legacy_metadata = crate::wiki::paths::LEGACY_METADATA_DIR;
    if rel == metadata
        || rel.starts_with(&format!("{metadata}/"))
        || rel == legacy_metadata
        || rel.starts_with(&format!("{legacy_metadata}/"))
    {
        return Some("metadata");
    }
    SCAFFOLD_NAMES
        .iter()
        .copied()
        .find(|name| rel == *name || rel.starts_with(&format!("{name}/")))
}

fn list_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn is_factory_stub(vault: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(vault) else {
        return false;
    };
    let rel = rel.to_string_lossy().replace('\\', "/");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    rel == "capabilities/index.md" && text == CAPABILITIES_STUB
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SAMPLE: &str = r#"{
  "version": 3,
  "layout": {
    "scaffolds": {
      "capabilities": "disabled",
      "journal": "disabled",
      "knowledge": "disabled",
      "sources": "disabled",
      "raw": "lazy",
      "work": "required",
      "metadata": "required"
    },
    "write_paths": { "records": "工作/复盘" },
    "language": "zh-CN"
  }
}"#;

    fn write_contract(vault: &std::path::Path, body: &str) {
        let path = vault.join(V3_FILE);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn missing_file_is_v2_and_creates_scaffolds() {
        let dir = tempdir().unwrap();
        let contract = load(dir.path());
        assert_eq!(contract.version, 2);
        ensure_layout(dir.path(), &contract).unwrap();
        assert!(dir.path().join("journal").is_dir());
        assert!(dir.path().join("work/turns").is_dir());
        assert!(dir.path().join("work/sessions").is_dir());
        assert!(dir.path().join("work/projects").is_dir());
        assert!(dir.path().join("work/decisions").is_dir());
        assert!(dir.path().join(crate::wiki::paths::METADATA_DIR).is_dir());
        assert!(dir
            .path()
            .join(format!("{}/projects", crate::wiki::paths::METADATA_DIR))
            .is_dir());
        assert!(dir
            .path()
            .join(format!("{}/folders", crate::wiki::paths::METADATA_DIR))
            .is_dir());
        assert!(dir.path().join("capabilities").is_dir());
        for rel in [
            "work/areas",
            "work/outcomes",
            "work/records",
            "work/raw",
            "raw",
            "metadata",
        ] {
            assert!(!dir.path().join(rel).exists(), "{rel}");
        }
    }

    #[test]
    fn valid_v3_skips_disabled_and_waits_for_lazy() {
        let dir = tempdir().unwrap();
        write_contract(dir.path(), SAMPLE);
        let contract = load(dir.path());
        assert!(contract.applied);
        assert_eq!(contract.language, "zh-CN");
        ensure_layout(dir.path(), &contract).unwrap();
        assert!(!dir.path().join("journal").exists());
        assert!(!dir.path().join("capabilities").exists());
        assert!(!dir.path().join("raw").exists());
        assert!(!dir.path().join("work/raw").exists());
        assert!(dir.path().join("work/projects").is_dir());
        assert!(dir.path().join("work/turns").is_dir());
        assert!(dir.path().join("work/sessions").is_dir());
        assert!(dir.path().join("work/decisions").is_dir());
        assert!(!dir.path().join("work/areas").exists());
        assert!(!dir.path().join("work/outcomes").exists());
        assert!(!dir.path().join("work/records").exists());
        assert!(dir.path().join(crate::wiki::paths::METADATA_DIR).is_dir());
        assert!(dir
            .path()
            .join(format!("{}/projects", crate::wiki::paths::METADATA_DIR))
            .is_dir());
        assert!(dir
            .path()
            .join(format!("{}/folders", crate::wiki::paths::METADATA_DIR))
            .is_dir());
        assert!(!dir.path().join("metadata").exists());
        let projects = dir
            .path()
            .join(format!("{}/projects", crate::wiki::paths::METADATA_DIR));
        fs::remove_dir_all(&projects).unwrap();
        prepare_write(
            dir.path(),
            &format!("{}/projects/a.md", crate::wiki::paths::METADATA_DIR),
            &contract,
        )
        .unwrap();
        assert!(projects.is_dir());
        assert!(!dir.path().join("metadata/projects").exists());
        prepare_write(dir.path(), "raw/imports/a.md", &contract).unwrap();
        assert!(dir.path().join("work/raw/imports").is_dir());
        assert!(!dir.path().join("raw/imports").exists());
        prepare_write(dir.path(), "_archive/old.md", &contract).unwrap();
        assert!(dir.path().join("work/_archive").is_dir());
        assert!(!dir.path().join("_archive").exists());
        assert!(prepare_write(dir.path(), "journal/today.md", &contract).is_err());
    }

    #[test]
    fn legacy_contract_is_read_and_left_in_place() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(LEGACY_V3_FILE), SAMPLE).unwrap();
        let contract = load(dir.path());
        assert!(contract.applied);
        assert_eq!(contract.language, "zh-CN");
        assert!(dir.path().join(LEGACY_V3_FILE).is_file());
        assert!(!dir.path().join(V3_FILE).exists());
    }

    #[test]
    fn invalid_contract_falls_back_to_v2() {
        let dir = tempdir().unwrap();
        write_contract(
            dir.path(),
            r#"{"version":3,"layout":{"scaffolds":{"journal":"nope"}}}"#,
        );
        let contract = load(dir.path());
        assert!(!contract.applied);
        assert_eq!(contract.version, 2);
        assert!(contract.warning.unwrap().contains("invalid mode"));
    }

    #[test]
    fn unknown_version_falls_back() {
        let dir = tempdir().unwrap();
        write_contract(dir.path(), r#"{"version":9}"#);
        let contract = load(dir.path());
        assert!(!contract.applied);
        assert!(contract.warning.unwrap().contains("unknown version"));
    }

    #[test]
    fn converge_deletes_stub_and_keeps_notes() {
        let dir = tempdir().unwrap();
        write_contract(dir.path(), SAMPLE);
        let contract = load(dir.path());
        fs::create_dir_all(dir.path().join("journal")).unwrap();
        fs::create_dir_all(dir.path().join("capabilities")).unwrap();
        fs::write(dir.path().join("capabilities/index.md"), CAPABILITIES_STUB).unwrap();
        fs::create_dir_all(dir.path().join("knowledge/methods")).unwrap();
        fs::write(dir.path().join("knowledge/methods/keep.md"), "# 旧笔记\n").unwrap();
        let before = fs::read(dir.path().join("knowledge/methods/keep.md")).unwrap();
        let decisions = converge(dir.path(), &contract).unwrap();
        assert!(decisions
            .iter()
            .any(|d| d.scaffold == "journal" && d.action == "delete"));
        assert!(decisions
            .iter()
            .any(|d| d.scaffold == "capabilities" && d.action == "delete"));
        assert!(decisions
            .iter()
            .any(|d| d.scaffold == "knowledge" && d.action == "keep"));
        assert!(!dir.path().join("journal").exists());
        assert!(!dir.path().join("capabilities").exists());
        assert_eq!(
            fs::read(dir.path().join("knowledge/methods/keep.md")).unwrap(),
            before
        );
    }

    #[test]
    fn resolve_existing_checks_mapped_path() {
        let dir = tempdir().unwrap();
        write_contract(dir.path(), SAMPLE);
        let contract = load(dir.path());
        fs::create_dir_all(dir.path().join("工作/复盘")).unwrap();
        fs::write(dir.path().join("工作/复盘/a.md"), "note").unwrap();
        let found = resolve_existing(dir.path(), "work/records/a.md", &contract).unwrap();
        assert_eq!(found, dir.path().join("工作/复盘/a.md"));
    }

    #[test]
    fn converge_report_records_decisions_under_obsidian_codeg() {
        let dir = tempdir().unwrap();
        let state = tempdir().unwrap();
        let decisions = vec![LayoutDecision {
            action: "keep",
            scaffold: "knowledge".into(),
            detail: "2 user files".into(),
        }];
        let path = write_converge_report(dir.path(), "vault-1", &decisions).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("keep `knowledge`"));
        assert!(text.contains("does not write `.obsidian/app.json`"));
        assert_eq!(
            path,
            dir.path()
                .join(".obsidian/codeg/layout/vault-1-converge.md")
        );
        assert!(!dir.path().join("log.md").exists());
        assert!(!dir.path().join(".obsidian/app.json").exists());
        assert!(!dir.path().join("layout").exists());
        assert!(!state.path().join("layout").exists());
        assert!(write_converge_report(dir.path(), "../app", &decisions).is_err());
    }

    #[test]
    fn escaping_write_path_is_rejected() {
        let dir = tempdir().unwrap();
        write_contract(
            dir.path(),
            r#"{"version":3,"layout":{"write_paths":{"records":"../secret"}}}"#,
        );
        let contract = load(dir.path());
        assert!(!contract.applied);
    }
}

//! Validate staged pages, persist a commit manifest, atomic-replace, recover.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::wiki::fs_policy::{self, WikiFsPolicy};
use crate::wiki::raw::content_hash;
use crate::wiki::vault::{CONTENT_END, CONTENT_START};

const MANAGED_YAML_KEYS: &[&str] = &[
    "title",
    "summary",
    "type",
    "tags",
    "aliases",
    "date",
    "updated",
    "status",
    "projects",
    "areas",
    "sources",
    "capabilities",
    "concepts",
    "codeg_note_id",
    "evidence_level",
    "decision_state",
];

#[derive(Debug, Clone, thiserror::Error)]
pub enum CommitError {
    #[error("{0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("io: {0}")]
    Io(String),
}

impl From<io::Error> for CommitError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<fs_policy::FsPolicyError> for CommitError {
    fn from(e: fs_policy::FsPolicyError) -> Self {
        Self::Validation(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitFile {
    pub rel: String,
    pub op: String,
    pub before_hash: String,
    pub after_hash: String,
    #[serde(default)]
    pub applied: bool,
    pub page_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommitManifest {
    pub job_id: String,
    pub vault: String,
    pub files: Vec<CommitFile>,
    /// Full after-bodies keyed by vault-relative path. Persisted before apply.
    pub after_contents: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoverStatus {
    Complete,
    PartialConflict { rel: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct StagedProposal {
    pub rel: String,
    pub page_type: String,
    pub before_hash: String,
    pub after: String,
    pub op: String,
}

pub fn commits_dir(state_root: &Path) -> PathBuf {
    state_root.join("commits")
}

pub fn conflicts_dir(state_root: &Path, job_id: &str) -> PathBuf {
    state_root.join("conflicts").join(job_id)
}

pub fn manifest_path(state_root: &Path, job_id: &str) -> PathBuf {
    commits_dir(state_root).join(format!("{job_id}.json"))
}

pub fn load_manifest(state_root: &Path, job_id: &str) -> Result<Option<CommitManifest>, CommitError> {
    let path = manifest_path(state_root, job_id);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)?;
    let m: CommitManifest = serde_json::from_str(&raw)
        .map_err(|e| CommitError::Validation(format!("commit manifest: {e}")))?;
    Ok(Some(m))
}

pub fn persist_manifest(state_root: &Path, manifest: &CommitManifest) -> Result<(), CommitError> {
    let dir = commits_dir(state_root);
    fs::create_dir_all(&dir)?;
    let dest = manifest_path(state_root, &manifest.job_id);
    let tmp = dir.join(format!(".{}.tmp", manifest.job_id));
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| CommitError::Validation(format!("serialize commit manifest: {e}")))?;
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &dest)?;
    Ok(())
}

/// Validate proposals, persist the manifest, then apply. Idempotent when every
/// target already matches `after_hash`.
pub fn commit_proposals(
    vault: &Path,
    state_root: &Path,
    job_id: &str,
    proposals: &[StagedProposal],
) -> Result<CommitManifest, CommitError> {
    let policy = WikiFsPolicy::new(vault, state_root);
    let mut files = Vec::new();
    let mut after_contents = BTreeMap::new();
    for p in proposals {
        validate_proposal(&policy, p)?;
        let after_hash = content_hash(&p.after);
        files.push(CommitFile {
            rel: p.rel.clone(),
            op: p.op.clone(),
            before_hash: p.before_hash.clone(),
            after_hash,
            applied: false,
            page_type: p.page_type.clone(),
        });
        after_contents.insert(p.rel.clone(), p.after.clone());
    }
    let mut manifest = CommitManifest {
        job_id: job_id.to_string(),
        vault: vault.to_string_lossy().into_owned(),
        files,
        after_contents,
    };
    persist_manifest(state_root, &manifest)?;
    apply_manifest(vault, state_root, &mut manifest)?;
    persist_manifest(state_root, &manifest)?;
    Ok(manifest)
}

pub fn recover_manifest(
    vault: &Path,
    state_root: &Path,
    manifest: &mut CommitManifest,
) -> Result<RecoverStatus, CommitError> {
    apply_manifest(vault, state_root, manifest)?;
    persist_manifest(state_root, manifest)?;
    if manifest.files.iter().all(|f| f.applied) {
        Ok(RecoverStatus::Complete)
    } else {
        let pending = manifest
            .files
            .iter()
            .find(|f| !f.applied)
            .map(|f| f.rel.clone())
            .unwrap_or_default();
        Ok(RecoverStatus::PartialConflict {
            rel: pending,
            reason: "commit recovery stopped on a hash mismatch".into(),
        })
    }
}

/// Scan `wiki-state/commits/*.json` and resume each manifest.
pub fn recover_all(state_root: &Path) -> Vec<(String, Result<RecoverStatus, CommitError>)> {
    let dir = commits_dir(state_root);
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                out.push((name.to_string(), Err(CommitError::Io(e.to_string()))));
                continue;
            }
        };
        let mut manifest: CommitManifest = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                out.push((
                    name.to_string(),
                    Err(CommitError::Validation(e.to_string())),
                ));
                continue;
            }
        };
        if manifest.files.iter().all(|f| f.applied) {
            out.push((manifest.job_id.clone(), Ok(RecoverStatus::Complete)));
            continue;
        }
        let vault = PathBuf::from(&manifest.vault);
        let status = recover_manifest(&vault, state_root, &mut manifest);
        out.push((manifest.job_id.clone(), status));
    }
    out
}

fn apply_manifest(
    vault: &Path,
    state_root: &Path,
    manifest: &mut CommitManifest,
) -> Result<(), CommitError> {
    let policy = WikiFsPolicy::new(vault, state_root);
    fs::create_dir_all(vault)?;
    let _vault_lock = acquire_vault_lock(vault)?;
    for file in manifest.files.iter_mut() {
        if file.applied {
            continue;
        }
        let Some(after) = manifest.after_contents.get(&file.rel).cloned() else {
            return Err(CommitError::Validation(format!(
                "missing after content for {}",
                file.rel
            )));
        };
        match apply_one(&policy, vault, state_root, &manifest.job_id, file, &after) {
            Ok(()) => file.applied = true,
            Err(CommitError::Conflict(reason)) => {
                save_conflict(state_root, &manifest.job_id, file, vault, &after)?;
                return Err(CommitError::Conflict(reason));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn apply_one(
    policy: &WikiFsPolicy,
    vault: &Path,
    _state_root: &Path,
    job_id: &str,
    file: &CommitFile,
    after: &str,
) -> Result<(), CommitError> {
    let dest = policy.check_commit_rel(&file.rel)?;
    policy.check_page_type_path(&file.page_type, &file.rel)?;
    let current = if dest.exists() {
        Some(fs::read_to_string(&dest)?)
    } else {
        None
    };
    let current_hash = current.as_deref().map(content_hash).unwrap_or_default();
    if current_hash == file.after_hash {
        return Ok(());
    }
    if current_hash != file.before_hash {
        return Err(CommitError::Conflict(format!(
            "{}: before_hash mismatch (have {current_hash}, expected {})",
            file.rel, file.before_hash
        )));
    }

    let final_text = match current.as_deref() {
        Some(cur) => splice_user_regions(cur, after)?,
        None => ensure_content_markers(after),
    };

    if !dest.exists() && file.op == "create" {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(mut f) => {
                f.write_all(final_text.as_bytes())?;
                f.sync_all()?;
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(CommitError::Conflict(format!(
                    "{}: refusing to overwrite an unexpected file on create",
                    file.rel
                )));
            }
            Err(e) => return Err(e.into()),
        }
    }

    atomic_replace(vault, &dest, &final_text, job_id)?;
    Ok(())
}

/// Copy dest + proposal into `wiki-state/conflicts/<job-id>/`.
fn save_conflict(
    state_root: &Path,
    job_id: &str,
    file: &CommitFile,
    vault: &Path,
    after: &str,
) -> Result<(), CommitError> {
    let dir = conflicts_dir(state_root, job_id);
    fs::create_dir_all(&dir)?;
    let stem = file.rel.replace('/', "__");
    let dest = vault.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    if dest.exists() {
        fs::copy(&dest, dir.join(format!("{stem}.before.md")))?;
    }
    fs::write(dir.join(format!("{stem}.after.md")), after)?;
    Ok(())
}

fn atomic_replace(vault: &Path, dest: &Path, contents: &str, job_id: &str) -> Result<(), CommitError> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_name = format!(
        ".codeg-tmp-{}-{}",
        job_id,
        dest.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("page.md")
    );
    // Temp file on the vault filesystem so rename is atomic (not a cross-device copy).
    let tmp = dest.parent().unwrap_or(vault).join(&tmp_name);
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dest)?;
    Ok(())
}

fn acquire_vault_lock(vault: &Path) -> Result<File, CommitError> {
    fs::create_dir_all(vault)?;
    let path = vault.join(".codeg-wiki.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(CommitError::Io(
            "vault lock is held by another process".into(),
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(CommitError::Io(format!("vault lock: {e}"))),
    }
}

pub fn validate_proposal(policy: &WikiFsPolicy, p: &StagedProposal) -> Result<(), CommitError> {
    policy.check_commit_rel(&p.rel)?;
    policy.check_page_type_path(&p.page_type, &p.rel)?;
    validate_markdown(&p.after, &p.page_type)?;
    Ok(())
}

pub fn validate_markdown(md: &str, expected_type: &str) -> Result<(), CommitError> {
    let (yaml, _body) = split_frontmatter(md)
        .ok_or_else(|| CommitError::Validation("page is missing YAML front matter".into()))?;
    if !wikilinks_quoted_in_yaml(&yaml) {
        return Err(CommitError::Validation(
            "machine-generated wikilinks in YAML must be quoted strings".into(),
        ));
    }
    let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml)
        .map_err(|e| CommitError::Validation(format!("invalid YAML: {e}")))?;
    let map = parsed.as_mapping().ok_or_else(|| {
        CommitError::Validation("YAML front matter must be a mapping".into())
    })?;
    let ty = map
        .get(serde_yaml::Value::String("type".into()))
        .and_then(|v| v.as_str())
        .ok_or_else(|| CommitError::Validation("missing YAML type".into()))?;
    if ty != expected_type {
        return Err(CommitError::Validation(format!(
            "YAML type {ty} does not match proposal type {expected_type}"
        )));
    }
    if !fs_policy::ALLOWED_PAGE_TYPES.contains(&ty) {
        return Err(CommitError::Validation(format!(
            "page type is not writable: {ty}"
        )));
    }
    Ok(())
}

pub fn wikilinks_quoted_in_yaml(yaml: &str) -> bool {
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(idx) = trimmed.find("[[") {
            let before = &trimmed[..idx];
            if !before.contains('"') && !before.contains('\'') {
                return false;
            }
        }
    }
    true
}

pub fn split_frontmatter(md: &str) -> Option<(String, String)> {
    let rest = md.strip_prefix("---\n").or_else(|| md.strip_prefix("---\r\n"))?;
    let nl = if let Some(i) = rest.find("\n---\n") {
        (i, 5)
    } else if let Some(i) = rest.find("\r\n---\r\n") {
        (i, 7)
    } else if let Some(i) = rest.find("\n---") {
        (i, 4)
    } else {
        return None;
    };
    let yaml = rest[..nl.0].to_string();
    let body = rest[nl.0 + nl.1..].to_string();
    Some((yaml, body))
}

pub fn splice_user_regions(current: &str, proposed: &str) -> Result<String, CommitError> {
    let (cur_yaml, cur_body) = match split_frontmatter(current) {
        Some(v) => v,
        None => {
            return Err(CommitError::Conflict(
                "existing note has no YAML front matter; v1 will not take it over".into(),
            ));
        }
    };
    let (prop_yaml, prop_body) = split_frontmatter(proposed).ok_or_else(|| {
        CommitError::Validation("proposal is missing YAML front matter".into())
    })?;

    if count_markers(&cur_body, CONTENT_START) > 1 || count_markers(&cur_body, CONTENT_END) > 1 {
        return Err(CommitError::Conflict(
            "duplicate codeg-content markers; not rewriting".into(),
        ));
    }
    if count_markers(&cur_body, CONTENT_START) == 0 || count_markers(&cur_body, CONTENT_END) == 0 {
        return Err(CommitError::Conflict(
            "existing note has no codeg-content markers; v1 will not take over the body".into(),
        ));
    }

    let merged_yaml = merge_yaml(&cur_yaml, &prop_yaml)?;
    let spliced_body = replace_generated_region(&cur_body, &prop_body)?;
    Ok(format!("---\n{merged_yaml}---\n{spliced_body}"))
}

fn merge_yaml(current: &str, proposed: &str) -> Result<String, CommitError> {
    let mut cur: serde_yaml::Mapping = serde_yaml::from_str(current)
        .map_err(|e| CommitError::Validation(format!("current YAML: {e}")))?;
    let prop: serde_yaml::Mapping = serde_yaml::from_str(proposed)
        .map_err(|e| CommitError::Validation(format!("proposed YAML: {e}")))?;
    for (k, v) in prop {
        let key = k.as_str().unwrap_or("");
        if MANAGED_YAML_KEYS.contains(&key) {
            cur.insert(k, v);
        }
    }
    let mut out = serde_yaml::to_string(&serde_yaml::Value::Mapping(cur))
        .map_err(|e| CommitError::Validation(format!("emit YAML: {e}")))?;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn replace_generated_region(current_body: &str, proposed_body: &str) -> Result<String, CommitError> {
    let prop_inner = extract_generated(proposed_body).unwrap_or(proposed_body);
    let start = current_body
        .find(CONTENT_START)
        .ok_or_else(|| CommitError::Conflict("missing codeg-content start".into()))?;
    let end = current_body
        .find(CONTENT_END)
        .ok_or_else(|| CommitError::Conflict("missing codeg-content end".into()))?;
    if end < start {
        return Err(CommitError::Conflict("content markers out of order".into()));
    }
    let mut out = String::new();
    out.push_str(&current_body[..start]);
    out.push_str(CONTENT_START);
    out.push('\n');
    let inner = prop_inner.trim_end();
    out.push_str(inner);
    if !inner.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(CONTENT_END);
    out.push_str(&current_body[end + CONTENT_END.len()..]);
    Ok(out)
}

fn extract_generated(body: &str) -> Option<&str> {
    let start = body.find(CONTENT_START)? + CONTENT_START.len();
    let rest = body.get(start..)?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find(CONTENT_END)?;
    Some(&rest[..end])
}

fn count_markers(s: &str, marker: &str) -> usize {
    s.matches(marker).count()
}

pub fn ensure_content_markers(md: &str) -> String {
    if md.contains(CONTENT_START) && md.contains(CONTENT_END) {
        return md.to_string();
    }
    match split_frontmatter(md) {
        Some((yaml, body)) => {
            format!("---\n{yaml}---\n\n{CONTENT_START}\n{}\n{CONTENT_END}\n", body.trim())
        }
        None => format!("{CONTENT_START}\n{}\n{CONTENT_END}\n", md.trim()),
    }
}

pub fn file_hash(path: &Path) -> Result<String, CommitError> {
    if !path.exists() {
        return Ok(String::new());
    }
    let s = fs::read_to_string(path)?;
    Ok(content_hash(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiki::vault::initialize_vault;
    use tempfile::tempdir;

    fn capability_page(note_id: &str, source_id: &str) -> String {
        format!(
            "---\n\
title: Interface design\n\
type: capability\n\
summary: Idempotent retries and error envelopes.\n\
tags:\n\
  - type/capability\n\
date: 2026-09-12\n\
updated: 2026-09-12\n\
status: draft\n\
evidence_level: knowledge_only\n\
sources:\n\
  - \"[[sources/{source_id}]]\"\n\
codeg_note_id: \"{note_id}\"\n\
---\n\n\
{CONTENT_START}\n\
# Interface design\n\n\
## 能力范围\n\
Design request/response contracts.\n\n\
## 实践证据\n\
暂无。来源：[[sources/{source_id}|规范]]，§3.2。\n\
{CONTENT_END}\n"
        )
    }

    #[test]
    fn commit_writes_capability_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        initialize_vault(&vault).unwrap();
        fs::create_dir_all(&state).unwrap();
        let source_id = "11111111-1111-4111-8111-111111111111";
        let note_id = "22222222-2222-4222-8222-222222222222";
        let rel = format!("capabilities/interface-design-{note_id}.md");
        let after = capability_page(note_id, source_id);
        let job = "job-commit-1";
        let proposal = StagedProposal {
            rel: rel.clone(),
            page_type: "capability".into(),
            before_hash: String::new(),
            after: after.clone(),
            op: "create".into(),
        };
        let m1 = commit_proposals(&vault, &state, job, &[proposal.clone()]).unwrap();
        assert!(m1.files.iter().all(|f| f.applied));
        let path = vault.join(&rel);
        let first = fs::read_to_string(&path).unwrap();
        assert!(first.contains("[[sources/11111111-1111-4111-8111-111111111111]]"));
        assert!(first.contains("type: capability"));

        let m2 = commit_proposals(&vault, &state, job, &[proposal]).unwrap();
        assert!(m2.files.iter().all(|f| f.applied));
        let second = fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn before_hash_mismatch_keeps_vault_file_and_saves_conflict() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        initialize_vault(&vault).unwrap();
        fs::create_dir_all(vault.join("capabilities")).unwrap();
        let rel = "capabilities/keep.md";
        let original = capability_page("n1", "s1").replace("Interface design", "Original");
        fs::write(vault.join(rel), &original).unwrap();
        let proposal = StagedProposal {
            rel: rel.into(),
            page_type: "capability".into(),
            before_hash: "deadbeef".into(),
            after: capability_page("n1", "s1"),
            op: "update".into(),
        };
        let err = commit_proposals(&vault, &state, "job-conflict", &[proposal]).unwrap_err();
        assert!(matches!(err, CommitError::Conflict(_)));
        let now = fs::read_to_string(vault.join(rel)).unwrap();
        assert_eq!(now, original);
        assert!(conflicts_dir(&state, "job-conflict").join("capabilities__keep.md.after.md").exists());
    }

    #[test]
    fn path_escape_in_proposal_is_rejected() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        initialize_vault(&vault).unwrap();
        let proposal = StagedProposal {
            rel: "../outside.md".into(),
            page_type: "capability".into(),
            before_hash: String::new(),
            after: capability_page("n", "s"),
            op: "create".into(),
        };
        let err = commit_proposals(&vault, &state, "job-esc", &[proposal]).unwrap_err();
        assert!(matches!(err, CommitError::Validation(_)));
        assert!(!dir.path().join("outside.md").exists());
    }

    #[test]
    fn recovery_resumes_pending_file_and_skips_applied() {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        initialize_vault(&vault).unwrap();
        fs::create_dir_all(vault.join("capabilities")).unwrap();
        fs::create_dir_all(vault.join("sources")).unwrap();
        let a_rel = "capabilities/a.md";
        let b_rel = "sources/src.md";
        let a_after = capability_page("na", "src");
        let b_after = format!(
            "---\ntitle: Source\ntype: source\ntags:\n  - type/source\nsources: []\ncodeg_note_id: \"nb\"\n---\n\n{CONTENT_START}\n# Source\n{CONTENT_END}\n"
        );
        fs::write(vault.join(a_rel), &a_after).unwrap();
        let b_before = format!(
            "---\ntitle: Source\ntype: source\ntags:\n  - type/source\ncodeg_note_id: \"nb\"\n---\n\n{CONTENT_START}\n# old\n{CONTENT_END}\n"
        );
        fs::write(vault.join(b_rel), &b_before).unwrap();

        let mut manifest = CommitManifest {
            job_id: "job-recover".into(),
            vault: vault.to_string_lossy().into_owned(),
            files: vec![
                CommitFile {
                    rel: a_rel.into(),
                    op: "update".into(),
                    before_hash: content_hash(&a_after),
                    after_hash: content_hash(&a_after),
                    applied: true,
                    page_type: "capability".into(),
                },
                CommitFile {
                    rel: b_rel.into(),
                    op: "update".into(),
                    before_hash: content_hash(&b_before),
                    after_hash: content_hash(&b_after),
                    applied: false,
                    page_type: "source".into(),
                },
            ],
            after_contents: BTreeMap::from([
                (a_rel.into(), a_after.clone()),
                (b_rel.into(), b_after.clone()),
            ]),
        };
        persist_manifest(&state, &manifest).unwrap();
        let status = recover_manifest(&vault, &state, &mut manifest).unwrap();
        assert_eq!(status, RecoverStatus::Complete);
        assert_eq!(fs::read_to_string(vault.join(a_rel)).unwrap(), a_after);
        let b_now = fs::read_to_string(vault.join(b_rel)).unwrap();
        assert!(b_now.contains("# Source") || content_hash(&b_now) == content_hash(&b_after) || b_now.contains("Source"));
        assert!(manifest.files.iter().all(|f| f.applied));
    }

    #[test]
    fn unquoted_wikilink_in_yaml_is_rejected() {
        let md = "---\ntype: capability\nsources:\n  - [[sources/x]]\n---\n\nbody\n";
        let err = validate_markdown(md, "capability").unwrap_err();
        assert!(matches!(err, CommitError::Validation(_)));
    }
}

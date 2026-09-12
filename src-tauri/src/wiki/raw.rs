//! Deterministic raw session dumps. Host YAML serializer; exclusive create.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::wiki::snapshot::WikiTurnSnapshot;
use crate::wiki::vault::{CONTENT_END, CONTENT_START};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawWriteOutcome {
    Created {
        path: PathBuf,
        hash: String,
    },
    Identical {
        path: PathBuf,
        hash: String,
    },
    Conflict {
        path: PathBuf,
        existing_hash: String,
        new_hash: String,
    },
}

#[derive(Debug, Clone)]
pub struct RawSessionMeta<'a> {
    pub source_id: &'a str,
    pub source_group_id: &'a str,
    pub captured_at: DateTime<Utc>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub redacted: bool,
    pub folder_path: Option<&'a str>,
    pub git_branch: Option<&'a str>,
    pub root_folder_id: Option<i32>,
    pub model_fallback: bool,
}

fn yaml_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn yaml_bool(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

/// Flattened, host-serialized YAML front matter. Wikilink-shaped strings are quoted.
pub fn render_session_raw(snap: &WikiTurnSnapshot, meta: &RawSessionMeta<'_>) -> (String, String) {
    let body = render_body(snap, meta.redacted);
    let hash = content_hash(&body);
    let title_owned = first_line_owned(&snap.user_text);
    let title = title_owned.as_deref().unwrap_or("ACP turn");
    let date = meta.occurred_at.unwrap_or(snap.occurred_at).date_naive();
    let mut yaml = String::from("---\n");
    push_str(&mut yaml, "title", title);
    push_str(&mut yaml, "type", "session-dump");
    yaml.push_str("tags:\n  - \"type/session-dump\"\n");
    push_str(&mut yaml, "date", &date.to_string());
    push_str(&mut yaml, "source_kind", "acp-turn");
    push_str(&mut yaml, "codeg_source_id", meta.source_id);
    push_str(&mut yaml, "codeg_source_group_id", meta.source_group_id);
    push_str(&mut yaml, "captured_at", &snap.captured_at.to_rfc3339());
    if let Some(at) = meta.occurred_at {
        push_str(&mut yaml, "occurred_at", &at.to_rfc3339());
    }
    push_str(&mut yaml, "codeg_content_hash", &hash);
    yaml.push_str(&format!(
        "codeg_truncated: {}\n",
        yaml_bool(snap.user_truncated || snap.assistant_truncated || snap.tool_dropped_count > 0)
    ));
    yaml.push_str(&format!("codeg_redacted: {}\n", yaml_bool(meta.redacted)));
    push_str(&mut yaml, "agent", &snap.agent_type);
    if let Some(model) = &snap.model {
        push_str(&mut yaml, "model", model);
    }
    if let Some(mode) = &snap.mode {
        push_str(&mut yaml, "mode", mode);
    }
    if let Some(cwd) = &snap.working_dir {
        push_str(&mut yaml, "cwd", cwd);
    }
    if let Some(fp) = meta.folder_path {
        push_str(&mut yaml, "folder_path", fp);
    }
    if let Some(br) = meta.git_branch {
        push_str(&mut yaml, "git_branch", br);
    }
    push_str(&mut yaml, "codeg_run_id", &snap.run_id);
    if let Some(cid) = snap.conversation_id {
        push_str(&mut yaml, "codeg_conversation_id", &cid.to_string());
    }
    push_str(&mut yaml, "codeg_connection_id", &snap.connection_id);
    if let Some(fid) = snap.folder_id {
        push_str(&mut yaml, "codeg_folder_id", &fid.to_string());
    }
    if let Some(rid) = meta.root_folder_id {
        push_str(&mut yaml, "codeg_root_folder_id", &rid.to_string());
    }
    if meta.model_fallback {
        yaml.push_str("codeg_model_fallback: true\n");
    }
    yaml.push_str("---\n\n");
    yaml.push_str(&body);
    if !body.ends_with('\n') {
        yaml.push('\n');
    }
    (yaml, hash)
}

fn push_str(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&yaml_quote(value));
    out.push('\n');
}

fn first_line_owned(s: &str) -> Option<String> {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().count() <= 80 {
        Some(line.to_string())
    } else {
        Some(line.chars().take(80).collect())
    }
}

fn render_body(snap: &WikiTurnSnapshot, redacted: bool) -> String {
    let mut body = String::from("# Session dump\n\n");
    if snap.user_truncated || snap.assistant_truncated {
        body.push_str(&format!(
            "> Truncated: user {}→{} chars, assistant {}→{} chars.\n\n",
            snap.user_original_chars,
            snap.user_text.chars().count(),
            snap.assistant_original_chars,
            snap.assistant_text.chars().count()
        ));
    }
    if redacted {
        body.push_str("> Host redaction applied; secrets replaced with `[REDACTED]`.\n\n");
    }
    body.push_str("## User\n\n");
    body.push_str(snap.user_text.trim_end());
    body.push_str("\n\n## Assistant\n\n");
    body.push_str(snap.assistant_text.trim_end());
    body.push_str("\n\n## Tool observations\n\n");
    if snap.tool_observations.is_empty() {
        body.push_str("_None._\n");
    } else {
        for obs in &snap.tool_observations {
            body.push_str(&format!(
                "### {} ({}, {})\n\n",
                obs.id, obs.kind, obs.status
            ));
            if let Some(path) = &obs.path {
                body.push_str(&format!("- path: `{path}`\n"));
            }
            if !obs.summary.is_empty() {
                body.push('\n');
                body.push_str(obs.summary.trim_end());
                body.push('\n');
            }
            body.push('\n');
        }
    }
    if snap.tool_dropped_count > 0 {
        body.push_str(&format!(
            "\n_{} additional tool observation(s) dropped._\n",
            snap.tool_dropped_count
        ));
    }
    body.push_str("\n## File changes\n\n");
    if snap.file_changes.is_empty() {
        body.push_str("_None reported by tools._\n");
    } else {
        for ch in &snap.file_changes {
            body.push_str(&format!("- {}: `{}`\n", ch.operation, ch.path));
        }
    }
    body
}

pub fn content_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn raw_session_path(vault: &Path, source_id: &str) -> PathBuf {
    vault.join("raw/sessions").join(format!("{source_id}.md"))
}

/// Exclusive create. Same hash at path = success; different content = conflict.
pub fn write_raw_exclusive(path: &Path, contents: &str, hash: &str) -> io::Result<RawWriteOutcome> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => {
            f.write_all(contents.as_bytes())?;
            f.sync_all()?;
            Ok(RawWriteOutcome::Created {
                path: path.to_path_buf(),
                hash: hash.to_string(),
            })
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let existing = fs::read_to_string(path)?;
            let existing_hash = extract_hash(&existing).unwrap_or_else(|| content_hash(&existing));
            if existing_hash == hash || existing == contents {
                Ok(RawWriteOutcome::Identical {
                    path: path.to_path_buf(),
                    hash: existing_hash,
                })
            } else {
                Ok(RawWriteOutcome::Conflict {
                    path: path.to_path_buf(),
                    existing_hash,
                    new_hash: hash.to_string(),
                })
            }
        }
        Err(e) => Err(e),
    }
}

fn extract_hash(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("codeg_content_hash:") {
            let v = rest.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
        if line == "---" {
            // continue; first --- is start
        }
    }
    None
}

/// Idempotent log.md append keyed by job id.
pub fn append_log_idempotent(log_path: &Path, job_id: &str, line: &str) -> io::Result<()> {
    let marker = format!("wiki-job:{job_id}");
    if log_path.exists() {
        let existing = fs::read_to_string(log_path)?;
        if existing.contains(&marker) {
            return Ok(());
        }
        let entry = format!("- {line} <!-- {marker} -->\n");
        if let Some(idx) = existing.find(CONTENT_END) {
            let mut next = String::new();
            next.push_str(&existing[..idx]);
            if !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(&entry);
            next.push_str(&existing[idx..]);
            fs::write(log_path, next)?;
        } else {
            let mut next = existing;
            if !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(&entry);
            fs::write(log_path, next)?;
        }
        return Ok(());
    }
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = format!(
        "---\ntitle: \"Wiki log\"\ntype: index\n---\n\n# Wiki log\n\n{CONTENT_START}\n- {line} <!-- {marker} -->\n{CONTENT_END}\n"
    );
    fs::write(log_path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiki::snapshot::{WikiFileChange, WikiToolObservation, WikiTurnSnapshot};
    use crate::wiki::vault::initialize_vault;
    use tempfile::tempdir;

    fn snap() -> WikiTurnSnapshot {
        WikiTurnSnapshot {
            run_id: "run-1".into(),
            connection_id: "conn".into(),
            conversation_id: Some(1),
            agent_type: "claude_code".into(),
            working_dir: Some("/tmp/proj".into()),
            folder_id: Some(2),
            model: Some("sonnet".into()),
            mode: None,
            occurred_at: Utc::now(),
            captured_at: Utc::now(),
            user_text: "please fix".into(),
            assistant_text: "done".into(),
            user_original_chars: 11,
            assistant_original_chars: 4,
            user_truncated: false,
            assistant_truncated: false,
            tool_observations: vec![WikiToolObservation {
                id: "t1".into(),
                path: Some("a.rs".into()),
                kind: "edit".into(),
                status: "completed".into(),
                summary: "patched".into(),
            }],
            tool_dropped_count: 0,
            file_changes: vec![WikiFileChange {
                path: "a.rs".into(),
                operation: "edit".into(),
            }],
        }
    }

    fn meta<'a>(source_id: &'a str) -> RawSessionMeta<'a> {
        RawSessionMeta {
            source_id,
            source_group_id: source_id,
            captured_at: Utc::now(),
            occurred_at: None,
            redacted: false,
            folder_path: None,
            git_branch: None,
            root_folder_id: None,
            model_fallback: false,
        }
    }

    #[test]
    fn yaml_quotes_strings_and_body_keeps_roles() {
        let s = snap();
        let (doc, hash) = render_session_raw(&s, &meta("src-1"));
        assert!(doc.starts_with("---\n"));
        assert!(doc.contains("source_kind: \"acp-turn\""));
        assert!(doc.contains("codeg_run_id: \"run-1\""));
        assert!(doc.contains("## User"));
        assert!(doc.contains("please fix"));
        assert!(doc.contains("## Assistant"));
        assert!(doc.contains("## Tool observations"));
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn exclusive_create_identical_hash_ok_different_conflict() {
        let dir = tempdir().unwrap();
        initialize_vault(dir.path()).unwrap();
        let path = raw_session_path(dir.path(), "src-1");
        let s = snap();
        let (doc, hash) = render_session_raw(&s, &meta("src-1"));
        let first = write_raw_exclusive(&path, &doc, &hash).unwrap();
        assert!(matches!(first, RawWriteOutcome::Created { .. }));
        let second = write_raw_exclusive(&path, &doc, &hash).unwrap();
        assert!(matches!(second, RawWriteOutcome::Identical { .. }));
        let mut other = s;
        other.assistant_text = "changed".into();
        let (doc2, hash2) = render_session_raw(&other, &meta("src-1"));
        let conflict = write_raw_exclusive(&path, &doc2, &hash2).unwrap();
        assert!(matches!(conflict, RawWriteOutcome::Conflict { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), doc);
    }

    #[test]
    fn log_append_is_idempotent_by_job_id() {
        let dir = tempdir().unwrap();
        initialize_vault(dir.path()).unwrap();
        let log = dir.path().join("log.md");
        append_log_idempotent(&log, "job-1", "ingest succeeded").unwrap();
        append_log_idempotent(&log, "job-1", "ingest succeeded").unwrap();
        let text = fs::read_to_string(&log).unwrap();
        assert_eq!(text.matches("wiki-job:job-1").count(), 1);
    }
}

//! 原始资料位置索引：登记会话/文件地址，不把对话正文复制进 wiki 或 wiki-state。
//! persist、import 和整理任务共用读写；正文仍由原始 session 文件或当次 staging 提供。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceLocator {
    pub source_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

pub fn locator_path(state_root: &Path, source_id: &str) -> PathBuf {
    state_root.join("sources").join(format!("{source_id}.json"))
}

pub fn write_locator(state_root: &Path, locator: &SourceLocator) -> io::Result<PathBuf> {
    let path = locator_path(state_root, &locator.source_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let json = serde_json::to_vec_pretty(locator)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

pub fn read_locator(state_root: &Path, source_id: &str) -> io::Result<SourceLocator> {
    let text = fs::read_to_string(locator_path(state_root, source_id))?;
    serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn is_dump_rel(rel: &str) -> bool {
    let rel = rel.trim().trim_start_matches("./");
    rel.starts_with("raw/") || rel.starts_with("wiki-state/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn locator_round_trips_without_session_body() {
        let dir = tempdir().unwrap();
        let locator = SourceLocator {
            source_id: "src-1".into(),
            kind: "acp-turn".into(),
            conversation_id: Some(9),
            run_id: Some("run-1".into()),
            agent_type: Some("claude_code".into()),
            session_id: None,
            original_path: Some("/tmp/session.jsonl".into()),
            title: Some("方案 C".into()),
        };
        write_locator(dir.path(), &locator).unwrap();
        let text = fs::read_to_string(locator_path(dir.path(), "src-1")).unwrap();
        assert!(!text.contains("User"));
        assert!(!text.contains("Assistant"));
        assert_eq!(read_locator(dir.path(), "src-1").unwrap(), locator);
    }

    #[test]
    fn dump_paths_are_vault_copies_not_originals() {
        assert!(is_dump_rel("raw/sessions/a.md"));
        assert!(is_dump_rel("raw/imports/b.md"));
        assert!(!is_dump_rel("work/turns/a.md"));
        assert!(!is_dump_rel("/Users/me/.codex/sessions/a.jsonl"));
    }
}

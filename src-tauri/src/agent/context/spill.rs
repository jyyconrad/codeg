//! Spill oversized tool output next to the session transcript so `recall` can
//! reload it after truncation or projection prune.
//!
//! Layout: `<sessions_root>/<group>/spills/<session_id>/<call_id>.txt`
//! (Pi cache/tool-spill and DSH spill-policy: keep the full bytes off the
//! model prompt, addressable later).

use std::fs;
use std::path::{Path, PathBuf};

use super::store::ExecutionFact;
use super::transcript::OutputLocator;

const MAX_CALL_ID_CHARS: usize = 120;

pub fn spill_dir(root: &Path, group: &str, session_id: &str) -> PathBuf {
    root.join(group).join("spills").join(session_id)
}

pub fn sanitize_call_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len().min(MAX_CALL_ID_CHARS));
    for ch in id.chars().take(MAX_CALL_ID_CHARS) {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

pub fn spill_file_path(dir: &Path, tool_call_id: &str) -> PathBuf {
    dir.join(format!("{}.txt", sanitize_call_id(tool_call_id)))
}

pub fn write_spill(dir: &Path, tool_call_id: &str, body: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let dest = spill_file_path(dir, tool_call_id);
    let tmp = dest.with_extension("txt.tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, &dest)?;
    Ok(dest)
}

pub fn read_spill(dir: &Path, tool_call_id: &str) -> std::io::Result<String> {
    fs::read_to_string(spill_file_path(dir, tool_call_id))
}

pub fn locator_for(dir: &Path, tool_call_id: &str) -> OutputLocator {
    OutputLocator {
        path: Some(
            spill_file_path(dir, tool_call_id)
                .to_string_lossy()
                .into_owned(),
        ),
        line: Some(1),
    }
}

pub fn is_under_spill_dir(dir: &Path, path: &Path) -> bool {
    let Ok(dir) = dir.canonicalize() else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    path.starts_with(&dir)
}

pub fn spill_notice(tool_call_id: &str, original_chars: usize) -> String {
    format!("[spilled: {original_chars} chars; recall tool_call_id `{tool_call_id}`]")
}

pub fn recover_hint(fact: Option<&ExecutionFact>) -> String {
    let Some(fact) = fact else {
        return String::new();
    };
    let spilled = fact
        .output_locator
        .as_ref()
        .and_then(|locator| locator.path.as_ref())
        .is_some();
    if spilled || fact.truncated {
        format!(
            " Recall tool_call_id `{}` for the full output.",
            fact.tool_call_id
        )
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().expect("dir");
        let dest = write_spill(dir.path(), "call/1", "hello\nworld").expect("write");
        assert!(dest.ends_with("call_1.txt"));
        assert_eq!(
            read_spill(dir.path(), "call/1").expect("read"),
            "hello\nworld"
        );
    }

    #[test]
    fn empty_id_uses_unknown() {
        assert_eq!(sanitize_call_id(""), "unknown");
        assert_eq!(sanitize_call_id("$$$"), "___");
    }

    #[test]
    fn path_escape_stays_inside_dir() {
        let dir = tempfile::tempdir().expect("dir");
        let dest = spill_file_path(dir.path(), "../../etc/passwd");
        assert_eq!(dest.parent(), Some(dir.path()));
        assert!(dest
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".."));
    }
}

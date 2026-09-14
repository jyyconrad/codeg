//! 更新由代码维护的目录与元数据文档，保留校验标记后的人工补充。
//! library 统一持有 Wiki 写锁后调用；生成区域被人工改动时保留原文并返回冲突提示。
//! 校验用于避免覆盖人工输出，不固定素材版本，也不要求 Agent 提供读取证明。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::db::error::DbError;
use crate::wiki::raw::content_hash;

const MARKER: &str = "<!-- codeg-library:";

fn destination(root: &Path, rel: &str) -> Result<PathBuf, DbError> {
    crate::wiki::paths::join_vault_relative(root, rel).map_err(DbError::Validation)?;
    if !rel.ends_with(".md") || rel.contains('\\') {
        return Err(DbError::Validation(
            "expected a relative Markdown path".into(),
        ));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(rel).components() {
        let Component::Normal(part) = component else {
            return Err(DbError::Validation("invalid document path".into()));
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(DbError::Validation(
                    "document path contains a symbolic link".into(),
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

/// The caller serializes updates using the vault commit lock. Text following the
/// checksum marker belongs to the user; edits before it stop automatic updates.
pub fn write(root: &Path, rel: &str, template: &str, adopt: &[&str]) -> Result<(), DbError> {
    let path = destination(root, rel)?;
    let before = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let tail = match before.as_deref() {
        None => "\n",
        Some(text) if adopt.contains(&text) => "\n",
        Some(text) => {
            let conflict = || {
                DbError::Conflict(format!(
                    "{rel}: generated content was edited or is user-owned"
                ))
            };
            let (body, marked) = text.split_once(MARKER).ok_or_else(conflict)?;
            let (hash, tail) = marked.split_once(" -->\n").ok_or_else(conflict)?;
            if hash != content_hash(body) || tail.contains(MARKER) {
                return Err(conflict());
            }
            tail
        }
    };
    let body = format!("{}\n", template.trim_end());
    let after = format!("{body}{MARKER}{} -->\n{tail}", content_hash(&body));
    if before.as_deref() == Some(after.as_str()) {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| DbError::Validation("missing document parent".into()))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".codeg-library-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), DbError> {
        let mut output = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        output.write_all(after.as_bytes())?;
        output.sync_all()?;
        // Recheck after preparing bytes so concurrent external changes are not
        // silently replaced. New files use create-only installation.
        destination(root, rel)?;
        if let Some(before) = before {
            if fs::read_to_string(&path)? != before {
                return Err(DbError::Conflict(format!("{rel}: changed during refresh")));
            }
            fs::rename(&tmp, &path)?;
        } else {
            fs::hard_link(&tmp, &path)?;
        }
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn refresh_preserves_annotations_and_refuses_edited_generated_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "metadata/project.md",
            "---\ntitle: One\n---\n# One\n",
            &[],
        )
        .unwrap();
        let path = root.join("metadata/project.md");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{original}\nMy annotation.\n")).unwrap();
        write(
            root,
            "metadata/project.md",
            "---\ntitle: Two\n---\n# Two\n",
            &[],
        )
        .unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.starts_with("---\ntitle: Two"));
        assert!(updated.ends_with("My annotation.\n"));
        let edited = updated.replace("# Two", "# Human heading");
        fs::write(&path, &edited).unwrap();
        assert!(write(root, "metadata/project.md", "new", &[]).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
    }

    #[test]
    fn unowned_files_are_only_adopted_on_exact_template_match() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("index.md"), "custom home").unwrap();
        assert!(write(root, "index.md", "new home", &["old template"]).is_err());
        assert_eq!(
            fs::read_to_string(root.join("index.md")).unwrap(),
            "custom home"
        );
        fs::write(root.join("index.md"), "old template").unwrap();
        write(root, "index.md", "new home", &["old template"]).unwrap();
        assert!(fs::read_to_string(root.join("index.md"))
            .unwrap()
            .starts_with("new home"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_parent_paths_without_touching_the_target() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("metadata")).unwrap();
        assert!(write(root.path(), "metadata/project.md", "bad", &[]).is_err());
        assert!(write(root.path(), "../escape.md", "bad", &[]).is_err());
        assert!(!outside.path().join("project.md").exists());
    }
}

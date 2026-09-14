//! 从 Wiki 文件系统建立可读笔记目录，并提供统一的 Markdown 文件读取边界。
//! query 用于搜索/正文阅读，library 用于生成标题导航；目录页和空壳不计入笔记统计。
//! 读取限制文件大小并防止目录逃逸；目录扫描不跟随符号链接或暴露维护目录。

use std::fs;
use std::path::Path;

use super::document::Document;
use super::WikiNoteSummary;
use crate::db::error::DbError;

pub struct CatalogEntry {
    pub summary: WikiNoteSummary,
    pub body: String,
}

pub fn read_file(vault: &Path, rel: &str) -> Result<String, DbError> {
    let path = crate::wiki::paths::join_vault_relative(vault, rel).map_err(DbError::Validation)?;
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        return Err(DbError::Validation(
            "only Markdown documents can be read".into(),
        ));
    }
    let root = vault.canonicalize()?;
    let resolved = path
        .canonicalize()
        .map_err(|_| DbError::NotFound("Wiki document".into()))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(DbError::Validation(
            "document must stay inside the Wiki".into(),
        ));
    }
    if resolved.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(DbError::Validation(
            "document exceeds the reading limit".into(),
        ));
    }
    Ok(fs::read_to_string(resolved)?)
}

pub fn summary(vault: &Path, rel: &str, doc: &Document) -> WikiNoteSummary {
    let inferred_type = match rel.split('/').nth(1) {
        Some("turns") => "turn-summary",
        Some("sessions") => "session-summary",
        Some("projects") => "project",
        Some("areas") => "area",
        Some("decisions") => "decision",
        Some("outcomes") => "outcome",
        Some("methods") => "method",
        Some("concepts") => "concept",
        Some("entities") => "entity",
        _ if rel.starts_with("capabilities/") => "capability",
        _ => "work-record",
    };
    let mut source_ids = doc.strings("sources");
    source_ids.extend(doc.strings("source_ids"));
    source_ids.extend(doc.strings("codeg_source_id"));
    source_ids.retain(|id| !id.contains('/') && !id.contains('['));
    source_ids.sort();
    source_ids.dedup();
    let mut projects = doc.strings("projects");
    projects.extend(doc.strings("codeg_project_binding_id"));
    projects.sort();
    projects.dedup();
    WikiNoteSummary {
        note_id: doc
            .string("codeg_note_id")
            .unwrap_or_else(|| rel.to_string()),
        path: rel.to_string(),
        title: doc.title().unwrap_or_else(|| {
            Path::new(rel)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into()
        }),
        summary: doc.summary(),
        page_type: doc.string("type").unwrap_or_else(|| inferred_type.into()),
        updated_at: doc
            .string("updated")
            .or_else(|| doc.string("occurred_at"))
            .or_else(|| {
                fs::metadata(vault.join(rel))
                    .ok()?
                    .modified()
                    .ok()
                    .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
            }),
        project_ids: projects,
        source_ids,
        evidence_level: doc.string("evidence_level"),
        excerpt: None,
        source_id: None,
    }
}

pub fn scan(vault: &Path) -> Result<Vec<CatalogEntry>, DbError> {
    let mut entries = Vec::new();
    for dir in ["work", "capabilities", "knowledge"] {
        walk(vault, Path::new(dir), 0, &mut entries)?;
    }
    entries.sort_by(|a, b| {
        b.summary
            .updated_at
            .cmp(&a.summary.updated_at)
            .then(a.summary.note_id.cmp(&b.summary.note_id))
    });
    Ok(entries)
}

fn walk(
    vault: &Path,
    rel: &Path,
    depth: usize,
    entries: &mut Vec<CatalogEntry>,
) -> Result<(), DbError> {
    if depth > 12 {
        return Ok(());
    }
    let dir = vault.join(rel);
    let metadata = match fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    for child in fs::read_dir(dir)? {
        let child = child?;
        let name = child.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let kind = child.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let child_rel = rel.join(name.as_ref());
        if kind.is_dir() {
            walk(vault, &child_rel, depth + 1, entries)?;
        } else if kind.is_file() && name.ends_with(".md") && name != "index.md" {
            let rel = child_rel.to_string_lossy().replace('\\', "/");
            let source = read_file(vault, &rel)?;
            let doc = Document::parse(&source);
            if !doc.has_content()
                || matches!(doc.string("type").as_deref(), Some("index" | "source"))
            {
                continue;
            }
            entries.push(CatalogEntry {
                summary: summary(vault, &rel, &doc),
                body: doc.body,
            });
        }
    }
    Ok(())
}

pub fn excerpt(body: &str, query: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let lowercase = body.to_lowercase();
    // Compute the character offset in the folded text to avoid slicing inside
    // UTF-8 codepoints; excerpts are descriptive, not byte-precise evidence.
    let hit = lowercase
        .find(query)
        .map(|byte| lowercase[..byte].chars().count())
        .unwrap_or(0);
    let start = hit.saturating_sub(45).min(chars.len());
    let end = (start + 200).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "…" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn catalog_excludes_skeleton_and_internals_and_preserves_same_titles() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("work/projects")).unwrap();
        fs::create_dir_all(dir.path().join("raw")).unwrap();
        fs::write(dir.path().join("work/index.md"), "# 索引\n目录说明。").unwrap();
        fs::write(dir.path().join("raw/source.md"), "秘密资料正文").unwrap();
        fs::write(dir.path().join("work/projects/empty.md"), "# 项目\n暂无。").unwrap();
        for id in ["a", "b"] {
            fs::write(dir.path().join(format!("work/projects/{id}.md")), format!("---\ncodeg_note_id: {id}\ntitle: 分页\ntype: project\n---\n# 分页\n检查中文游标的排序规则。")).unwrap();
        }
        let notes = scan(dir.path()).unwrap();
        assert_eq!(notes.len(), 2);
        assert_ne!(notes[0].summary.note_id, notes[1].summary.note_id);
        assert!(notes.iter().all(|n| n.summary.title == "分页"));
    }

    #[cfg(unix)]
    #[test]
    fn source_reads_and_catalog_never_escape_vault_through_symlink() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("work")).unwrap();
        assert!(scan(root.path()).unwrap().is_empty());
        assert!(read_file(root.path(), "work/secret.md").is_err());
        assert!(read_file(root.path(), "../secret.md").is_err());
    }
}

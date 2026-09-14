//! 维护可脱离应用阅读的 Markdown 首页、目录索引与项目/文件夹元数据页。
//! 目录阅读页显式刷新、后台笔记生成后刷新共用此入口；不调用模型。
//! project_metadata 采集事实，managed_document 保护人工编辑，read_model 提供正文解析。

use std::path::Path;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use serde::Serialize;

use crate::db::entities::wiki_source;
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::read_model::{catalog, document::Document};
use crate::wiki::{managed_document, paths, project_metadata, settings, tree, vault};

#[derive(Serialize)]
pub struct WikiLibrary {
    pub vault_path: String,
    pub home_path: String,
    pub tree: Vec<tree::WikiVaultTreeEntry>,
    pub warnings: Vec<String>,
}

/// An explicit refresh writes only derived navigation/metadata. It neither
/// enables capture nor calls a model, and emits no recursive refresh events.
pub async fn refresh(conn: &DatabaseConnection) -> Result<WikiLibrary, DbError> {
    let _transition = crate::wiki::lifecycle::lock().await;
    let config = settings::load_settings(conn).await?;
    let root = paths::resolve_vault_path(config.vault_path.as_deref());
    vault::validate_new_location(&root)?;
    if !root.join(vault::FORMAT_MARKER).exists() {
        vault::initialize_vault(&root)?;
    }
    let active = wiki_service::ensure_active_vault(conn, &root.to_string_lossy()).await?;
    refresh_at(conn, &active.id, &root).await
}

/// Generation uses its own vault identity even if the user switches libraries.
pub async fn refresh_at(
    conn: &DatabaseConnection,
    vault_id: &str,
    root: &Path,
) -> Result<WikiLibrary, DbError> {
    let root = root.to_path_buf();
    let projects = project_metadata::collect(conn, vault_id).await?;
    let documents = project_metadata::documents(&projects);
    let sources = wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .filter(wiki_source::Column::Eligibility.ne("withdrawn"))
        .all(conn)
        .await?
        .into_iter()
        .filter_map(|source| {
            let path = source.raw_path?;
            let title = source
                .source_title
                .or(source.original_filename)
                .unwrap_or(source.id);
            Some((path, title))
        })
        .collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        let _lock = crate::wiki::commit::acquire_vault_lock(&root)
            .map_err(|e| DbError::Validation(e.to_string()))?;
        let warnings = refresh_files(&root, &documents, &sources)?;
        Ok(WikiLibrary {
            vault_path: root.to_string_lossy().into_owned(),
            home_path: "index.md".into(),
            tree: library_tree(&root)?,
            warnings,
        })
    })
    .await
    .map_err(|e| DbError::Validation(format!("Wiki refresh failed: {e}")))?
}

fn directory_title(rel: &str) -> &str {
    match rel {
        "" => "个人 Wiki",
        "work" => "工作",
        "work/projects" => "项目笔记",
        "work/areas" => "职责领域",
        "work/records" => "工作记录",
        "work/decisions" => "决策",
        "work/outcomes" => "成果",
        "work/turns" => "单轮记录",
        "work/sessions" => "对话总结",
        "capabilities" => "能力",
        "knowledge" => "知识",
        "knowledge/concepts" => "概念",
        "knowledge/methods" => "方法",
        "knowledge/entities" => "实体",
        "metadata" => "项目与文件夹",
        "metadata/projects" => "项目仓库",
        "metadata/folders" => "文件夹",
        "sources" => "原始资料",
        _ => rel.rsplit('/').next().unwrap_or(rel),
    }
}

fn library_tree(root: &Path) -> Result<Vec<tree::WikiVaultTreeEntry>, DbError> {
    fn clean(root: &Path, entries: Vec<tree::WikiVaultTreeEntry>) -> Vec<tree::WikiVaultTreeEntry> {
        entries
            .into_iter()
            .filter_map(|mut entry| {
                if entry.name.starts_with('.')
                    || matches!(
                        entry.name.as_str(),
                        "raw" | "journal" | "AGENTS.md" | "log.md"
                    )
                {
                    return None;
                }
                if entry.is_dir {
                    let mut children = clean(root, entry.children.take().unwrap_or_default());
                    if entry.path == "sources" {
                        children.retain(|child| child.path == "sources/index.md");
                    }
                    entry.title = Some(directory_title(&entry.path).into());
                    entry.children = Some(children);
                } else {
                    if !entry.name.ends_with(".md") {
                        return None;
                    }
                    entry.title = catalog::read_file(root, &entry.path)
                        .ok()
                        .and_then(|text| Document::parse(&text).title());
                }
                Some(entry)
            })
            .collect()
    }
    let entries = tree::list_vault_tree(root, "", true, true)
        .map_err(|e| DbError::Validation(format!("reading Wiki directory: {e:?}")))?;
    Ok(clean(root, entries))
}

#[cfg(test)]
fn walk_markdown(root: &Path) -> Result<Vec<String>, DbError> {
    fn collect(entries: Vec<tree::WikiVaultTreeEntry>, out: &mut Vec<String>) {
        for entry in entries {
            if let Some(children) = entry.children {
                collect(children, out);
            } else {
                out.push(entry.path);
            }
        }
    }
    let mut out = Vec::new();
    collect(library_tree(root)?, &mut out);
    Ok(out)
}

fn link(path: &str, title: &str, directory: &str) -> String {
    let label = title.replace(['\n', '\r', '[', ']', '|'], " ");
    // Obsidian cannot address these characters in wikilinks. Markdown links
    // retain such user filenames; encode the destination without data loss.
    if path.contains(['#', '|', '[', ']', '^', '%']) {
        let encoded: String = path
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let prefix = "../".repeat(directory.split('/').filter(|s| !s.is_empty()).count());
        format!("- [{label}]({prefix}{encoded})")
    } else {
        format!("- [[{}|{label}]]", path.strip_suffix(".md").unwrap_or(path))
    }
}

fn old_template(rel: &str) -> Option<String> {
    let (title, body) = match rel {
        "index.md" => (
            "Personal Wiki",
            "- [[work/index|Work]]\n- [[capabilities/index|Capabilities]]\n- [[sources|Sources]]",
        ),
        "work/index.md" => ("Work", "- Projects, areas, records, decisions, outcomes."),
        "capabilities/index.md" => (
            "Capabilities",
            "- Capability catalog, evidence gaps, next practice.",
        ),
        _ => return None,
    };
    Some(format!("---\ntitle: \"{title}\"\ntype: index\ntags:\n  - \"type/index\"\n---\n\n# {title}\n\n{}\n{body}\n{}\n", vault::CONTENT_START, vault::CONTENT_END))
}

fn refresh_files(
    root: &Path,
    documents: &[(String, String)],
    sources: &[(String, String)],
) -> Result<Vec<String>, DbError> {
    let mut warnings = Vec::new();
    for (rel, text) in documents {
        if let Err(error) = managed_document::write(root, rel, text, &[]) {
            warnings.push(format!("{rel}: {error}"));
        }
    }
    // Always create navigable section pages, including an empty metadata catalog.
    for rel in [
        "metadata/projects/index.md",
        "metadata/folders/index.md",
        "sources/index.md",
    ] {
        if !root.join(rel).exists() {
            let title = directory_title(rel.trim_end_matches("/index.md"));
            if let Err(error) = managed_document::write(root, rel, &index_document(title, ""), &[])
            {
                warnings.push(format!("{rel}: {error}"));
            }
        }
    }
    let entries = library_tree(root)?;
    fn index(
        root: &Path,
        rel: &str,
        entries: &[tree::WikiVaultTreeEntry],
        sources: &[(String, String)],
        warnings: &mut Vec<String>,
    ) {
        let mut links = Vec::new();
        for entry in entries {
            if entry.is_dir {
                index(
                    root,
                    &entry.path,
                    entry.children.as_deref().unwrap_or_default(),
                    sources,
                    warnings,
                );
                let path = format!("{}/index.md", entry.path);
                if catalog::read_file(root, &path).is_ok() {
                    links.push(link(
                        &path,
                        entry.title.as_deref().unwrap_or(&entry.name),
                        rel,
                    ));
                }
            } else if entry.name != "index.md" {
                links.push(link(
                    &entry.path,
                    entry.title.as_deref().unwrap_or(&entry.name),
                    rel,
                ));
            }
        }
        if rel == "sources" {
            links.extend(
                sources
                    .iter()
                    .filter(|(path, _)| catalog::read_file(root, path).is_ok())
                    .map(|(path, title)| link(path, title, rel)),
            );
        }
        let path = if rel.is_empty() {
            "index.md".into()
        } else {
            format!("{rel}/index.md")
        };
        let mut body = links.join("\n");
        if rel.is_empty() {
            body = format!("从目录进入项目、工作记录、能力与知识。页面之间通过链接关联，原始资料可从笔记出处追溯。\n\n{body}\n\n## 在 Obsidian 中阅读\n\n选择“打开文件夹为仓库”，打开本 Wiki 所在的整个目录，再进入 `index.md`。文件夹内的 Markdown 文件可独立阅读。\n\n## 人工补充\n\n可在文件末尾的代码校验标记之后补充内容；修改代码维护区域时，后续刷新会保留修改并给出提示。");
        } else if body.is_empty() {
            body = "此目录暂时没有笔记。".into();
        }
        let old = old_template(&path);
        let adopt = old.as_deref().into_iter().collect::<Vec<_>>();
        if let Err(error) = managed_document::write(
            root,
            &path,
            &index_document(directory_title(rel), &body),
            &adopt,
        ) {
            warnings.push(format!("{path}: {error}"));
        }
    }
    index(root, "", &entries, sources, &mut warnings);
    Ok(warnings)
}

fn index_document(title: &str, body: &str) -> String {
    let title_yaml = serde_json::to_string(title).expect("string serializes");
    format!("---\ntitle: {title_yaml}\ntype: index\n---\n\n# {title}\n\n{body}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_links_exist_and_refresh_tracks_note_deletions() {
        let dir = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        std::fs::write(
            dir.path().join("work/projects/a.md"),
            "# 项目 A\n已经完成可阅读内容。",
        )
        .unwrap();
        let warnings = refresh_files(dir.path(), &[], &[]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let home = std::fs::read_to_string(dir.path().join("index.md")).unwrap();
        assert!(home.contains("[[knowledge/index|"));
        assert!(home.contains("[[metadata/index|"));
        assert!(home.contains("[[sources/index|"));
        let project_index = dir.path().join("work/projects/index.md");
        assert!(std::fs::read_to_string(&project_index)
            .unwrap()
            .contains("[[work/projects/a|项目 A]]"));
        for entry in walk_markdown(dir.path()).unwrap() {
            let text = std::fs::read_to_string(dir.path().join(entry)).unwrap();
            for link in text.split("[[").skip(1) {
                let target = link.split(['|', ']']).next().unwrap();
                assert!(
                    dir.path().join(format!("{target}.md")).is_file(),
                    "{target}"
                );
            }
        }
        std::fs::remove_file(dir.path().join("work/projects/a.md")).unwrap();
        refresh_files(dir.path(), &[], &[]).unwrap();
        assert!(!std::fs::read_to_string(project_index)
            .unwrap()
            .contains("[[work/projects/a|"));
    }

    #[test]
    fn custom_home_is_preserved_and_remains_browsable() {
        let dir = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        std::fs::write(dir.path().join("index.md"), "# 我的首页\n手工组织的内容。").unwrap();
        let warnings = refresh_files(dir.path(), &[], &[]).unwrap();
        assert!(warnings.iter().any(|s| s.contains("index.md")));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
            "# 我的首页\n手工组织的内容。"
        );
    }

    #[test]
    fn directory_indexes_are_not_inputs_to_automatic_synthesis() {
        let dir = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        refresh_files(dir.path(), &[], &[]).unwrap();
        assert!(crate::wiki::compile::scan_memory_notes(dir.path()).is_empty());
    }
}

//! 刷新 Wiki 阅读视图。不创建分类目录，也不写首页、目录索引或导航页。
//! 目录阅读页显式刷新、后台笔记生成后刷新共用此入口；不调用模型。
//! 项目元数据写到 `.obsidian/codeg/metadata/`，缺父目录时一并创建。
//! 不回写库根旧 `metadata/`。

use std::path::Path;

use sea_orm::DatabaseConnection;
use serde::Serialize;

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

/// An explicit refresh lists the vault. It does not enable capture, call a
/// model, create taxonomy directories, or write navigation pages.
pub async fn refresh(conn: &DatabaseConnection) -> Result<WikiLibrary, DbError> {
    let _transition = crate::wiki::lifecycle::lock().await;
    crate::wiki::relocate::relocate_legacy_app_data_wiki(conn).await?;
    let config = settings::load_settings(conn).await?;
    let root = paths::resolve_vault_path(config.vault_path.as_deref());
    vault::validate_new_location(&root)?;
    if !vault::has_purpose_marker(&root) {
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
    tokio::task::spawn_blocking(move || {
        let _lock = crate::wiki::commit::acquire_vault_lock(&root)
            .map_err(|e| DbError::Validation(e.to_string()))?;
        let warnings = refresh_files(&root, &documents)?;
        let tree = library_tree(&root)?;
        Ok(WikiLibrary {
            vault_path: root.to_string_lossy().into_owned(),
            home_path: choose_home(&root, &tree),
            tree,
            warnings,
        })
    })
    .await
    .map_err(|e| DbError::Validation(format!("Wiki refresh failed: {e}")))?
}

fn library_tree(root: &Path) -> Result<Vec<tree::WikiVaultTreeEntry>, DbError> {
    fn clean(root: &Path, entries: Vec<tree::WikiVaultTreeEntry>) -> Vec<tree::WikiVaultTreeEntry> {
        entries
            .into_iter()
            .filter_map(|mut entry| {
                if entry.name.starts_with('.') {
                    return None;
                }
                if entry.is_dir {
                    entry.children = Some(clean(root, entry.children.take().unwrap_or_default()));
                } else if entry.name.ends_with(".md") {
                    entry.title = catalog::read_file(root, &entry.path)
                        .ok()
                        .and_then(|text| Document::parse(&text).title());
                } else {
                    return None;
                }
                Some(entry)
            })
            .collect()
    }
    let entries = tree::list_vault_tree(root, "", true, true)
        .map_err(|e| DbError::Validation(format!("reading Wiki directory: {e:?}")))?;
    Ok(clean(root, entries))
}

fn first_markdown(entries: &[tree::WikiVaultTreeEntry]) -> Option<String> {
    for entry in entries {
        if !entry.is_dir && entry.name.ends_with(".md") {
            return Some(entry.path.clone());
        }
        if let Some(path) = entry.children.as_deref().and_then(first_markdown) {
            return Some(path);
        }
    }
    None
}

/// Prefer the vault's own `index.md`. Otherwise open an existing note so an
/// Obsidian folder without a Codeg home still has a page to render.
fn choose_home(root: &Path, tree: &[tree::WikiVaultTreeEntry]) -> String {
    if root.join("index.md").is_file() {
        "index.md".into()
    } else {
        first_markdown(tree).unwrap_or_else(|| "index.md".into())
    }
}

fn is_generated_index(rel: &str) -> bool {
    rel == "index.md"
        || rel == "sources/index.md"
        || (rel.starts_with("metadata/") && rel.ends_with("/index.md"))
        || (rel.starts_with(".obsidian/codeg/metadata/") && rel.ends_with("/index.md"))
}

fn parent_exists(root: &Path, rel: &str) -> bool {
    match Path::new(rel).parent() {
        None => true,
        Some(parent) if parent.as_os_str().is_empty() => true,
        Some(parent) => root.join(parent).is_dir(),
    }
}

/// Host metadata pages under `.obsidian/codeg/metadata/`. Legacy `metadata/`
/// stays dual-read only and is not recreated.
fn is_host_metadata_page(rel: &str) -> bool {
    let prefix = format!("{}/", crate::wiki::paths::METADATA_DIR);
    rel.starts_with(&prefix) && !is_generated_index(rel)
}

fn refresh_files(root: &Path, documents: &[(String, String)]) -> Result<Vec<String>, DbError> {
    let mut warnings = Vec::new();
    for (rel, text) in documents {
        // Missing taxonomy folders stay absent. Navigation indexes are not written.
        // Codeg metadata pages create `.obsidian/codeg/metadata/{projects,folders}`.
        if is_generated_index(rel) || (!parent_exists(root, rel) && !is_host_metadata_page(rel)) {
            continue;
        }
        if let Err(error) = managed_document::write(root, rel, text, &[]) {
            warnings.push(format!("{rel}: {error}"));
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_taxonomy_dirs_stay_gone_and_notes_stay_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for rel in [
            "capabilities",
            "journal",
            "knowledge/concepts",
            "sources",
            "work/records",
            ".obsidian",
        ] {
            std::fs::create_dir_all(root.join(rel)).unwrap();
        }
        let note = root.join("work/records/kept.md");
        let home = root.join("index.md");
        let obsidian = root.join(".obsidian/app.json");
        std::fs::write(&note, "keep this note\n").unwrap();
        std::fs::write(&home, "# 我的首页\n手工组织的内容。\n").unwrap();
        std::fs::write(
            root.join(crate::wiki::contract::LEGACY_V3_FILE),
            r#"{"version":3,"layout":{"scaffolds":{"capabilities":"disabled","journal":"disabled","knowledge":"disabled","sources":"disabled","raw":"lazy","work":"required","metadata":"required"}}}"#,
        )
        .unwrap();
        std::fs::write(&obsidian, "{\"userIgnoreFilters\":[]}\n").unwrap();
        let note_bytes = std::fs::read(&note).unwrap();
        let home_bytes = std::fs::read(&home).unwrap();
        let obsidian_bytes = std::fs::read(&obsidian).unwrap();
        for rel in ["capabilities", "journal", "knowledge", "sources"] {
            std::fs::remove_dir_all(root.join(rel)).unwrap();
        }
        crate::wiki::vault::initialize_vault(root).unwrap();
        let warnings = refresh_files(
            root,
            &[("metadata/projects/p.md".into(), "generated\n".into())],
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(root.join("work/projects").is_dir());
        assert!(root.join("work/turns").is_dir());
        assert!(!root.join("work/areas").exists());
        assert!(!root.join("work/outcomes").exists());
        assert!(root.join(crate::wiki::paths::METADATA_DIR).is_dir());
        assert!(!root.join("metadata").exists());
        assert!(!root.join("work/raw").exists());
        for rel in [
            "capabilities",
            "journal",
            "knowledge",
            "sources",
            "log.md",
            "AGENTS.md",
            "sources/index.md",
            "metadata/projects/index.md",
            "metadata/folders/index.md",
            ".obsidian/codeg/metadata/projects/index.md",
            ".obsidian/codeg/metadata/folders/index.md",
        ] {
            assert!(!root.join(rel).exists(), "{rel} came back");
        }
        assert_eq!(std::fs::read(&note).unwrap(), note_bytes);
        assert_eq!(std::fs::read(&home).unwrap(), home_bytes);
        assert_eq!(std::fs::read(&obsidian).unwrap(), obsidian_bytes);
        crate::wiki::vault::initialize_vault(root).unwrap();
        refresh_files(root, &[]).unwrap();
        for rel in ["capabilities", "journal", "knowledge", "sources"] {
            assert!(!root.join(rel).exists(), "{rel}");
        }
        assert_eq!(std::fs::read(&note).unwrap(), note_bytes);
        assert_eq!(std::fs::read(&home).unwrap(), home_bytes);
    }

    #[test]
    fn codeg_metadata_pages_are_written_when_the_parent_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::wiki::vault::initialize_vault(root).unwrap();
        let projects = root.join(format!("{}/projects", crate::wiki::paths::METADATA_DIR));
        let folders = root.join(format!("{}/folders", crate::wiki::paths::METADATA_DIR));
        std::fs::remove_dir_all(&projects).unwrap();
        std::fs::remove_dir_all(&folders).unwrap();
        let project_rel = format!("{}/projects/p.md", crate::wiki::paths::METADATA_DIR);
        let folder_rel = format!("{}/folders/f.md", crate::wiki::paths::METADATA_DIR);
        let index_rel = format!("{}/projects/index.md", crate::wiki::paths::METADATA_DIR);
        let warnings = refresh_files(
            root,
            &[
                (project_rel.clone(), "project\n".into()),
                (folder_rel.clone(), "folder\n".into()),
                (index_rel.clone(), "index\n".into()),
                ("metadata/projects/legacy.md".into(), "legacy\n".into()),
            ],
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let project = std::fs::read_to_string(root.join(&project_rel)).unwrap();
        let folder = std::fs::read_to_string(root.join(&folder_rel)).unwrap();
        assert!(project.contains("project"));
        assert!(folder.contains("folder"));
        assert!(!root.join(&index_rel).exists());
        assert!(!root.join("metadata").exists());
        assert!(!root.join(".obsidian/app.json").exists());
    }

    fn find<'a>(
        entries: &'a [tree::WikiVaultTreeEntry],
        path: &str,
    ) -> Option<&'a tree::WikiVaultTreeEntry> {
        for entry in entries {
            if entry.path == path {
                return Some(entry);
            }
            if let Some(found) = entry.children.as_deref().and_then(|kids| find(kids, path)) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn obsidian_tree_shows_user_folders_and_hides_dot_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        let app_json = "{}\n";
        std::fs::write(root.join(".obsidian/app.json"), app_json).unwrap();
        std::fs::create_dir_all(root.join("日记")).unwrap();
        std::fs::write(root.join("日记/今天.md"), "# 今天\n正文\n").unwrap();
        std::fs::create_dir_all(root.join("journal")).unwrap();
        std::fs::write(root.join("journal/note.md"), "# j\n").unwrap();
        std::fs::create_dir_all(root.join("sources")).unwrap();
        std::fs::write(root.join("sources/kept.md"), "# s\n").unwrap();
        std::fs::create_dir_all(root.join("raw/sessions")).unwrap();
        std::fs::write(root.join("raw/sessions/turn.md"), "# t\n").unwrap();
        std::fs::create_dir_all(root.join("capabilities")).unwrap();
        std::fs::write(root.join("capabilities/skill.md"), "# c\n").unwrap();
        std::fs::create_dir_all(root.join("knowledge/concepts")).unwrap();
        std::fs::write(root.join("knowledge/concepts/idea.md"), "# k\n").unwrap();
        std::fs::create_dir_all(root.join("work/records")).unwrap();
        std::fs::write(root.join("work/records/kept.md"), "# w\n").unwrap();
        std::fs::write(root.join("AGENTS.md"), "# agents\n").unwrap();
        std::fs::write(root.join("log.md"), "# log\n").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let tree = library_tree(root).unwrap();
        assert!(find(&tree, "日记/今天.md").is_some());
        assert!(find(&tree, "journal/note.md").is_some());
        assert!(find(&tree, "sources/kept.md").is_some());
        assert!(find(&tree, "raw/sessions/turn.md").is_some());
        assert!(find(&tree, "capabilities/skill.md").is_some());
        assert!(find(&tree, "knowledge/concepts/idea.md").is_some());
        assert!(find(&tree, "work/records/kept.md").is_some());
        assert!(find(&tree, "AGENTS.md").is_some());
        assert!(find(&tree, "log.md").is_some());
        assert!(find(&tree, ".obsidian").is_none());
        assert!(find(&tree, ".git").is_none());
        assert!(find(&tree, "日记").unwrap().title.is_none());
        let home = choose_home(root, &tree);
        assert!(root.join(&home).is_file(), "{home}");
        assert_ne!(home, "index.md");
        assert_eq!(
            std::fs::read_to_string(root.join(".obsidian/app.json")).unwrap(),
            app_json
        );
        assert!(!root.join("index.md").exists());
        std::fs::write(root.join("index.md"), "# home\n").unwrap();
        let tree = library_tree(root).unwrap();
        assert_eq!(choose_home(root, &tree), "index.md");
    }

    #[test]
    fn directory_indexes_are_not_inputs_to_automatic_synthesis() {
        let dir = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        refresh_files(dir.path(), &[]).unwrap();
        assert!(crate::wiki::compile::scan_memory_notes(dir.path()).is_empty());
        assert!(!dir.path().join("index.md").exists());
    }
}

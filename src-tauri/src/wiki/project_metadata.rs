//! 从 Codeg 文件夹记录与本地 Git 收集项目、仓库和 worktree 的关联信息。
//! library 将事实输出为 Markdown 元数据页；compile 只读取当前素材关联的项目上下文。
//! 稳定绑定负责关联，名称与路径是属性；不访问远程仓库，输出 remote 前移除凭据。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

use crate::db::entities::{folder, wiki_project_binding};
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::vault::{CONTENT_END, CONTENT_START};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryRemote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub status: String,
    pub root: Option<String>,
    pub common_dir: Option<String>,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub detached: Option<bool>,
    pub unborn: Option<bool>,
    pub remotes: Vec<RepositoryRemote>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderMetadata {
    pub id: i32,
    pub root_folder_id: i32,
    pub parent_id: Option<i32>,
    pub name: String,
    pub alias: Option<String>,
    pub path: String,
    pub kind: String,
    pub status: String,
    pub repository: RepositoryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectMetadata {
    pub schema_version: u32,
    pub captured_at: String,
    pub binding_id: String,
    pub db_instance_id: String,
    pub root_folder_id: i32,
    pub root_folder: Option<FolderMetadata>,
    pub folders: Vec<FolderMetadata>,
}

/// Collect all known projects, including regular folders without captured turns.
pub async fn collect(
    conn: &DatabaseConnection,
    vault_id: &str,
) -> Result<Vec<ProjectMetadata>, DbError> {
    let instance = crate::wiki::settings::ensure_db_instance_id(conn).await?;
    let folders = folder::Entity::find()
        .filter(folder::Column::Kind.eq(folder::FolderKind::Regular))
        .filter(folder::Column::DeletedAt.is_null())
        .order_by_asc(folder::Column::Id)
        .all(conn)
        .await?;
    let roots: BTreeSet<_> = folders
        .iter()
        .map(|f| f.parent_id.unwrap_or(f.id))
        .collect();
    for root in roots {
        wiki_service::ensure_project_binding(conn, vault_id, &instance, root).await?;
    }
    let bindings = wiki_project_binding::Entity::find()
        .filter(wiki_project_binding::Column::VaultId.eq(vault_id))
        .filter(wiki_project_binding::Column::DbInstanceId.eq(&instance))
        .order_by_asc(wiki_project_binding::Column::RootFolderId)
        .all(conn)
        .await?;
    collect_rows(bindings, folders).await
}

/// Model execution probes only projects explicitly associated with its inputs.
pub async fn collect_for_bindings(
    conn: &DatabaseConnection,
    vault_id: &str,
    binding_ids: &[String],
) -> Result<Vec<ProjectMetadata>, DbError> {
    if binding_ids.is_empty() {
        return Ok(Vec::new());
    }
    let instance = crate::wiki::settings::ensure_db_instance_id(conn).await?;
    let bindings = wiki_project_binding::Entity::find()
        .filter(wiki_project_binding::Column::VaultId.eq(vault_id))
        .filter(wiki_project_binding::Column::DbInstanceId.eq(instance))
        .filter(wiki_project_binding::Column::Id.is_in(binding_ids.to_vec()))
        .order_by_asc(wiki_project_binding::Column::RootFolderId)
        .all(conn)
        .await?;
    if bindings.is_empty() {
        return Ok(Vec::new());
    }
    let roots: Vec<_> = bindings.iter().map(|b| b.root_folder_id).collect();
    let folders = folder::Entity::find()
        .filter(folder::Column::Kind.eq(folder::FolderKind::Regular))
        .filter(folder::Column::DeletedAt.is_null())
        .filter(
            sea_orm::Condition::any()
                .add(folder::Column::Id.is_in(roots.clone()))
                .add(folder::Column::ParentId.is_in(roots)),
        )
        .order_by_asc(folder::Column::Id)
        .all(conn)
        .await?;
    collect_rows(bindings, folders).await
}

async fn collect_rows(
    bindings: Vec<wiki_project_binding::Model>,
    folders: Vec<folder::Model>,
) -> Result<Vec<ProjectMetadata>, DbError> {
    let captured_at = chrono::Utc::now().to_rfc3339();
    let mut groups: BTreeMap<i32, Vec<folder::Model>> = BTreeMap::new();
    for folder in folders {
        groups
            .entry(folder.parent_id.unwrap_or(folder.id))
            .or_default()
            .push(folder);
    }
    let mut projects = Vec::new();
    for binding in bindings {
        let mut related = Vec::new();
        for folder in groups.remove(&binding.root_folder_id).unwrap_or_default() {
            let path = Path::new(&folder.path);
            let status = match std::fs::metadata(path) {
                Ok(meta) if meta.is_dir() => "available",
                Ok(_) => "not_directory",
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing",
                Err(_) => "unavailable",
            };
            let repository = probe_repository(path).await;
            related.push(FolderMetadata {
                id: folder.id,
                root_folder_id: binding.root_folder_id,
                parent_id: folder.parent_id,
                name: folder.name,
                alias: folder.alias,
                path: folder.path,
                kind: "regular".into(),
                status: status.into(),
                repository,
            });
        }
        projects.push(ProjectMetadata {
            schema_version: 1,
            captured_at: captured_at.clone(),
            binding_id: binding.id,
            db_instance_id: binding.db_instance_id,
            root_folder_id: binding.root_folder_id,
            root_folder: related
                .iter()
                .find(|f| f.id == binding.root_folder_id)
                .cloned(),
            folders: related,
        });
    }
    Ok(projects)
}

fn empty_repository(status: &str, error_code: Option<&str>) -> RepositoryMetadata {
    RepositoryMetadata {
        status: status.into(),
        root: None,
        common_dir: None,
        branch: None,
        head: None,
        detached: None,
        unborn: None,
        remotes: Vec::new(),
        error_code: error_code.map(str::to_string),
    }
}

async fn git_output(path: &Path, args: &[&str]) -> Result<std::process::Output, &'static str> {
    let mut command = crate::process::tokio_command("git");
    command
        .args(args)
        .current_dir(path)
        .kill_on_drop(true)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    tokio::time::timeout(Duration::from_secs(3), command.output())
        .await
        .map_err(|_| "git_timeout")?
        .map_err(|_| "git_unavailable")
}

fn output_text(output: &std::process::Output) -> Option<String> {
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn probe_repository(path: &Path) -> RepositoryMetadata {
    if !path.is_dir() {
        return empty_repository("unavailable", Some("folder_unavailable"));
    }
    match probe_repository_inner(path).await {
        Ok(repo) => repo,
        Err(code) => empty_repository("unavailable", Some(code)),
    }
}

async fn probe_repository_inner(path: &Path) -> Result<RepositoryMetadata, &'static str> {
    let inside = git_output(path, &["rev-parse", "--is-inside-work-tree"]).await?;
    if !inside.status.success() {
        return if String::from_utf8_lossy(&inside.stderr).contains("not a git repository") {
            Ok(empty_repository("not_repository", None))
        } else {
            Err("git_probe_failed")
        };
    }
    let common = git_output(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    let common_dir = output_text(&common).ok_or("git_probe_failed")?;
    let root = if output_text(&inside).as_deref() == Some("true") {
        let output = git_output(path, &["rev-parse", "--show-toplevel"]).await?;
        Some(output_text(&output).ok_or("git_probe_failed")?)
    } else {
        None // A bare repository has no working-tree root.
    };
    let branch_output = git_output(path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    if !branch_output.status.success() && branch_output.status.code() != Some(1) {
        return Err("git_probe_failed");
    }
    let branch = output_text(&branch_output);
    let head_output = git_output(path, &["rev-parse", "--verify", "HEAD"]).await?;
    let head = output_text(&head_output);
    if head.is_none() && branch.is_none() {
        return Err("git_head_unavailable");
    }
    let names_output = git_output(path, &["remote"]).await?;
    if !names_output.status.success() {
        return Err("git_remote_unavailable");
    }
    let mut remotes = Vec::new();
    for name in String::from_utf8_lossy(&names_output.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Read the configured URL; no remote transport is contacted.
        let urls = git_output(path, &["remote", "get-url", "--all", "--", name]).await?;
        if !urls.status.success() {
            return Err("git_remote_unavailable");
        }
        for url in String::from_utf8_lossy(&urls.stdout)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            remotes.push(RepositoryRemote {
                name: name.into(),
                url: sanitize_remote(url),
            });
        }
    }
    Ok(RepositoryMetadata {
        status: "repository".into(),
        root,
        common_dir: Some(common_dir),
        detached: Some(branch.is_none() && head.is_some()),
        unborn: Some(branch.is_some() && head.is_none()),
        branch,
        head,
        remotes,
        error_code: None,
    })
}

fn sanitize_remote(raw: &str) -> String {
    let raw = raw.trim().split(['?', '#']).next().unwrap_or_default();
    if let Some((scheme, rest)) = raw.split_once("://") {
        let (authority, path) = rest
            .split_once('/')
            .map_or((rest, ""), |(host, path)| (host, path));
        let authority = authority.rsplit('@').next().unwrap_or_default();
        return if path.is_empty() {
            format!("{scheme}://{authority}")
        } else {
            format!("{scheme}://{authority}/{path}")
        };
    }
    if let Some((authority, path)) = raw.split_once(':') {
        if !authority.contains('/') {
            return format!(
                "{}:{path}",
                authority.rsplit('@').next().unwrap_or_default()
            );
        }
    }
    raw.to_string()
}

fn path_key(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        value.into()
    } else {
        crate::wiki::raw::content_hash(value)
    }
}

fn label(value: &str) -> String {
    value.replace(['[', ']', '|', '\n', '\r'], " ")
}

fn folder_title(folder: &FolderMetadata) -> String {
    folder
        .alias
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&folder.name)
        .to_string()
}

fn property(map: &mut serde_yaml::Mapping, name: &str, value: impl Serialize) {
    map.insert(
        name.into(),
        serde_yaml::to_value(value).expect("metadata values are serializable"),
    );
}

fn repository_properties(map: &mut serde_yaml::Mapping, repo: &RepositoryMetadata) {
    property(map, "git_status", &repo.status);
    property(map, "git_root", &repo.root);
    property(map, "git_common_dir", &repo.common_dir);
    property(map, "git_branch", &repo.branch);
    property(map, "git_head", &repo.head);
    property(map, "git_detached", repo.detached);
    property(map, "git_unborn", repo.unborn);
    property(
        map,
        "git_remotes",
        repo.remotes
            .iter()
            .map(|r| format!("{}: {}", r.name, r.url))
            .collect::<Vec<_>>(),
    );
    property(map, "git_error_code", &repo.error_code);
}

fn repository_body(repo: &RepositoryMetadata) -> String {
    let mut body = format!("- Git 状态：{}\n", repo.status);
    for (name, value) in [
        ("仓库根目录", &repo.root),
        ("共享 Git 目录", &repo.common_dir),
        ("分支", &repo.branch),
        ("HEAD", &repo.head),
    ] {
        if let Some(value) = value {
            body.push_str(&format!("- {name}：`{}`\n", value.replace('`', "｀")));
        }
    }
    for remote in &repo.remotes {
        body.push_str(&format!(
            "- 远程 {}：`{}`\n",
            label(&remote.name),
            remote.url.replace('`', "｀")
        ));
    }
    if let Some(error) = &repo.error_code {
        body.push_str(&format!("- 采集结果：{error}\n"));
    }
    body
}

fn render_document(metadata: serde_yaml::Mapping, body: &str) -> String {
    let yaml = serde_yaml::to_string(&metadata).expect("metadata values are serializable");
    format!("---\n{yaml}---\n\n{CONTENT_START}\n{body}\n{CONTENT_END}\n")
}

/// Portable, flat Obsidian properties and readable pages. Caller owns durable,
/// conflict-aware writes; these paths never become model proposal targets.
pub fn documents(projects: &[ProjectMetadata]) -> Vec<(String, String)> {
    let mut documents = Vec::new();
    for project in projects {
        let project_path = format!("metadata/projects/{}", path_key(&project.binding_id));
        let title = project
            .root_folder
            .as_ref()
            .map(folder_title)
            .unwrap_or_else(|| format!("项目 {}", project.root_folder_id));
        let mut base = serde_yaml::Mapping::new();
        property(&mut base, "codeg_metadata_version", project.schema_version);
        property(&mut base, "codeg_project_binding_id", &project.binding_id);
        property(&mut base, "codeg_db_instance_id", &project.db_instance_id);
        property(&mut base, "codeg_root_folder_id", project.root_folder_id);
        property(&mut base, "captured_at", &project.captured_at);
        let mut metadata = base.clone();
        property(&mut metadata, "title", &title);
        property(&mut metadata, "type", "project-metadata");
        property(&mut metadata, "tags", ["type/project-metadata"]);
        property(
            &mut metadata,
            "folder_ids",
            project.folders.iter().map(|f| f.id).collect::<Vec<_>>(),
        );
        if let Some(root) = &project.root_folder {
            property(&mut metadata, "folder_name", &root.name);
            property(&mut metadata, "folder_alias", &root.alias);
            property(&mut metadata, "folder_path", &root.path);
            repository_properties(&mut metadata, &root.repository);
        }
        let mut body = format!(
            "# {}\n\n代码采集的目录与仓库事实，采集时间：{}。\n\n## 文件夹\n\n",
            label(&title),
            project.captured_at
        );
        for folder in &project.folders {
            let folder_path = format!(
                "metadata/folders/{}-{}",
                path_key(&project.db_instance_id),
                folder.id
            );
            body.push_str(&format!(
                "- [[{folder_path}|{}]] — `{}`\n",
                label(&folder_title(folder)),
                folder.path.replace('`', "｀")
            ));
            let mut folder_metadata = base.clone();
            property(&mut folder_metadata, "title", folder_title(folder));
            property(&mut folder_metadata, "type", "folder-metadata");
            property(&mut folder_metadata, "tags", ["type/folder-metadata"]);
            property(&mut folder_metadata, "codeg_folder_id", folder.id);
            property(
                &mut folder_metadata,
                "codeg_parent_folder_id",
                folder.parent_id,
            );
            property(&mut folder_metadata, "folder_name", &folder.name);
            property(&mut folder_metadata, "folder_alias", &folder.alias);
            property(&mut folder_metadata, "folder_path", &folder.path);
            property(&mut folder_metadata, "folder_kind", &folder.kind);
            property(&mut folder_metadata, "folder_status", &folder.status);
            property(
                &mut folder_metadata,
                "project",
                format!("[[{project_path}]]"),
            );
            repository_properties(&mut folder_metadata, &folder.repository);
            let folder_body = format!("# {}\n\n所属项目：[[{project_path}|{}]]\n\n- 文件夹：`{}`\n- 目录状态：{}\n- 采集时间：{}\n\n## 仓库\n\n{}", label(&folder_title(folder)), label(&title), folder.path.replace('`', "｀"), folder.status, project.captured_at, repository_body(&folder.repository));
            documents.push((
                format!("{folder_path}.md"),
                render_document(folder_metadata, &folder_body),
            ));
        }
        if project.folders.is_empty() {
            body.push_str("当前工作区中没有可读取的文件夹，项目标识仍保留。\n");
        }
        if let Some(root) = &project.root_folder {
            body.push_str(&format!(
                "\n## 仓库\n\n{}",
                repository_body(&root.repository)
            ));
        }
        documents.push((
            format!("{project_path}.md"),
            render_document(metadata, &body),
        ));
    }
    documents.sort_by(|a, b| a.0.cmp(&b.0));
    documents
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use tempfile::tempdir;

    fn git(path: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn remotes_never_export_credentials_queries_or_fragments() {
        assert_eq!(
            sanitize_remote("https://user:secret@example.test/org/repo.git?token=secret#fragment"),
            "https://example.test/org/repo.git"
        );
        assert_eq!(
            sanitize_remote("ssh://git:secret@example.test:2222/org/repo.git"),
            "ssh://example.test:2222/org/repo.git"
        );
        assert_eq!(
            sanitize_remote("git@example.test:org/repo.git"),
            "example.test:org/repo.git"
        );
        assert_eq!(sanitize_remote("../local/repo.git"), "../local/repo.git");
    }

    #[tokio::test]
    async fn ordinary_and_missing_directories_have_different_status() {
        let root = tempdir().unwrap();
        assert_eq!(probe_repository(root.path()).await.status, "not_repository");
        let missing = probe_repository(&root.path().join("absent")).await;
        assert_eq!(missing.status, "unavailable");
        assert!(missing.root.is_none());
    }

    #[tokio::test]
    async fn git_snapshot_distinguishes_unborn_detached_and_worktree() {
        let root = tempdir().unwrap();
        git(root.path(), &["init", "-b", "main"]);
        let unborn = probe_repository(root.path()).await;
        assert_eq!(unborn.status, "repository");
        assert_eq!(unborn.branch.as_deref(), Some("main"));
        assert_eq!(unborn.unborn, Some(true));
        assert!(unborn.head.is_none());
        git(
            root.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "seed",
            ],
        );
        git(
            root.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://user:secret@example.test/org/repo.git?token=secret",
            ],
        );
        let main = probe_repository(root.path()).await;
        assert_eq!(main.unborn, Some(false));
        assert_eq!(main.head.as_ref().unwrap().len(), 40);
        assert_eq!(main.remotes[0].url, "https://example.test/org/repo.git");
        let worktree = root.path().join("linked");
        git(
            root.path(),
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );
        let linked = probe_repository(&worktree).await;
        assert_eq!(linked.common_dir, main.common_dir);
        assert_ne!(linked.root, main.root);
        assert_eq!(linked.branch.as_deref(), Some("feature"));
        git(&worktree, &["checkout", "--detach"]);
        let detached = probe_repository(&worktree).await;
        assert_eq!(detached.detached, Some(true));
        assert_eq!(detached.head, main.head);
        assert!(detached.branch.is_none());
    }

    #[tokio::test]
    async fn collection_keeps_root_identity_and_exports_linked_folder_documents() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let root = tempdir().unwrap();
        let linked = tempdir().unwrap();
        let root_id =
            crate::db::test_helpers::seed_folder(&db, root.path().to_str().unwrap()).await;
        let child_id =
            crate::db::test_helpers::seed_folder(&db, linked.path().to_str().unwrap()).await;
        let mut child: folder::ActiveModel = folder::Entity::find_by_id(child_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        child.parent_id = Set(Some(root_id));
        child.alias = Set(Some("开发分支".into()));
        child.update(&db.conn).await.unwrap();
        let first = collect(&db.conn, "vault").await.unwrap();
        let second = collect(&db.conn, "vault").await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].binding_id, second[0].binding_id);
        assert_eq!(first[0].folders.len(), 2);
        let docs = documents(&first);
        assert_eq!(docs.len(), 3);
        let project_path = format!("metadata/projects/{}.md", first[0].binding_id);
        let project = docs.iter().find(|(path, _)| path == &project_path).unwrap();
        assert!(project.1.contains("开发分支"));
        for (path, text) in docs {
            let (yaml, _) = crate::wiki::commit::split_frontmatter(&text).unwrap();
            let metadata: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(
                metadata["codeg_project_binding_id"].as_str(),
                Some(first[0].binding_id.as_str())
            );
            assert!(text.contains(CONTENT_START));
            if path.contains("/folders/") {
                assert!(text.contains(&format!("[[metadata/projects/{}|", first[0].binding_id)));
            }
        }
    }

    #[tokio::test]
    async fn model_context_only_collects_requested_bindings_in_its_vault() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let one = tempdir().unwrap();
        let two = tempdir().unwrap();
        crate::db::test_helpers::seed_folder(&db, one.path().to_str().unwrap()).await;
        crate::db::test_helpers::seed_folder(&db, two.path().to_str().unwrap()).await;
        let all = collect(&db.conn, "one").await.unwrap();
        assert_eq!(all.len(), 2);
        let ids = vec![all[0].binding_id.clone()];
        let requested = collect_for_bindings(&db.conn, "one", &ids).await.unwrap();
        assert_eq!(requested.len(), 1);
        assert_eq!(requested[0].binding_id, ids[0]);
        assert!(collect_for_bindings(&db.conn, "other", &ids)
            .await
            .unwrap()
            .is_empty());
        assert!(collect_for_bindings(&db.conn, "one", &[])
            .await
            .unwrap()
            .is_empty());
    }
}

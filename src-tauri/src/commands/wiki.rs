//! Wiki settings, source/job listing, and vault browse. Dual-mode `_core` fns.

use std::fs;
use std::path::Path;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::app_error::AppCommandError;
use crate::db::error::DbError;
use crate::db::service::wiki_service::{
    self, WikiImportResult, WikiJobInfo, WikiProjectBindingInfo, WikiSourceInfo,
};
#[cfg(feature = "tauri-runtime")]
use crate::wiki::import::ImportFilePart;
use crate::wiki::import::{
    self, ImportFilesParams, ImportFilesResult, ImportTextParams, LinkVersionParams,
    UpdateAnnotationsParams,
};
use crate::wiki::paths::{self, join_vault_relative, resolve_vault_path};
use crate::wiki::settings::{self, WikiSettings, WikiSettingsView};
use crate::wiki::tree::{self, VaultTreeError};
use crate::wiki::vault;

pub use crate::wiki::tree::WikiVaultTreeEntry;

#[cfg(feature = "tauri-runtime")]
use crate::db::AppDatabase;

pub async fn get_wiki_settings_core(
    conn: &DatabaseConnection,
) -> Result<WikiSettingsView, DbError> {
    let settings = settings::load_settings(conn).await?;
    let next_compile_at = settings::next_compile_at(&settings);
    let pending_source_count = wiki_service::pending_source_count(conn).await?;
    Ok(WikiSettingsView {
        settings,
        next_compile_at,
        pending_source_count,
    })
}

pub async fn update_wiki_settings_core(
    conn: &DatabaseConnection,
    settings: WikiSettings,
) -> Result<WikiSettingsView, DbError> {
    settings::save_settings(conn, &settings).await?;
    if settings.enabled {
        let vault_path = resolve_vault_path(settings.vault_path.as_deref());
        vault::initialize_vault(&vault_path)?;
        vault::initialize_state_root(&crate::wiki::paths::resolve_state_root(
            settings.vault_path.as_deref(),
        ))?;
        let canonical = vault_path.to_string_lossy().to_string();
        wiki_service::ensure_active_vault(conn, &canonical).await?;
        let _ = settings::ensure_db_instance_id(conn).await?;
    }
    get_wiki_settings_core(conn).await
}

pub async fn wiki_list_jobs_core(
    conn: &DatabaseConnection,
    limit: Option<u64>,
    offset: Option<u64>,
    status: Option<String>,
) -> Result<Vec<WikiJobInfo>, DbError> {
    wiki_service::list_jobs(
        conn,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        status.as_deref(),
    )
    .await
}

pub async fn wiki_get_job_core(
    conn: &DatabaseConnection,
    id: String,
) -> Result<WikiJobInfo, DbError> {
    wiki_service::get_job(conn, &id).await
}

pub async fn wiki_list_sources_core(
    conn: &DatabaseConnection,
    limit: Option<u64>,
    offset: Option<u64>,
    source_kind: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<WikiSourceInfo>, DbError> {
    wiki_service::list_sources(
        conn,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        source_kind.as_deref(),
        project_id.as_deref(),
    )
    .await
}

pub async fn wiki_list_project_bindings_core(
    conn: &DatabaseConnection,
    vault_id: Option<String>,
) -> Result<Vec<WikiProjectBindingInfo>, DbError> {
    wiki_service::list_project_bindings(conn, vault_id.as_deref()).await
}

pub async fn wiki_get_source_core(
    conn: &DatabaseConnection,
    id: String,
) -> Result<WikiSourceInfo, DbError> {
    wiki_service::get_source(conn, &id).await
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WikiVaultTreeParams {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub recursive: Option<bool>,
    #[serde(default)]
    pub include_raw: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiVaultFile {
    pub path: String,
    pub content: String,
}

fn vault_root_from_settings(
    settings: &WikiSettings,
) -> Result<std::path::PathBuf, AppCommandError> {
    Ok(resolve_vault_path(settings.vault_path.as_deref()))
}

fn reject_unsafe_path(rel: &str) -> Result<(), AppCommandError> {
    if !paths::is_safe_vault_relative(rel) {
        return Err(AppCommandError::invalid_input(
            "path must be vault-relative without '..' or absolute segments",
        ));
    }
    Ok(())
}

pub async fn wiki_vault_tree_core(
    conn: &DatabaseConnection,
    params: WikiVaultTreeParams,
) -> Result<Vec<WikiVaultTreeEntry>, AppCommandError> {
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    let vault = vault_root_from_settings(&settings)?;
    let rel = params.path.unwrap_or_default();
    reject_unsafe_path(&rel)?;
    let recursive = params.recursive.unwrap_or(false);
    let include_raw = params.include_raw.unwrap_or(false);
    match tree::list_vault_tree(&vault, &rel, recursive, include_raw) {
        Ok(entries) => Ok(entries),
        Err(VaultTreeError::UnsafePath) => Err(AppCommandError::invalid_input(
            "path must be vault-relative without '..' or absolute segments",
        )),
        Err(VaultTreeError::NotFound) => Err(AppCommandError::new(
            crate::app_error::AppErrorCode::NotFound,
            "path not found",
        )),
        Err(VaultTreeError::NotDirectory) => {
            Err(AppCommandError::invalid_input("path is not a directory"))
        }
        Err(VaultTreeError::Io(err)) => Err(AppCommandError::io_error(err.to_string())),
    }
}

pub async fn wiki_import_text_core(
    conn: &DatabaseConnection,
    params: ImportTextParams,
) -> Result<WikiImportResult, AppCommandError> {
    import::import_text(conn, params).await
}

pub async fn wiki_import_files_core(
    conn: &DatabaseConnection,
    params: ImportFilesParams,
) -> Result<ImportFilesResult, AppCommandError> {
    import::import_files_with_result(conn, params).await
}

pub async fn wiki_accept_extraction_core(
    conn: &DatabaseConnection,
    source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    import::accept_extraction(conn, source_id).await
}

pub async fn wiki_update_source_annotations_core(
    conn: &DatabaseConnection,
    params: UpdateAnnotationsParams,
) -> Result<WikiSourceInfo, AppCommandError> {
    import::update_source_annotations(conn, params).await
}

pub async fn wiki_reextract_core(
    conn: &DatabaseConnection,
    source_id: String,
) -> Result<WikiImportResult, AppCommandError> {
    import::reextract(conn, source_id).await
}

pub async fn wiki_link_source_version_core(
    conn: &DatabaseConnection,
    params: LinkVersionParams,
) -> Result<WikiSourceInfo, AppCommandError> {
    import::link_source_version(conn, params).await
}

pub async fn wiki_vault_read_core(
    conn: &DatabaseConnection,
    path: String,
) -> Result<WikiVaultFile, AppCommandError> {
    reject_unsafe_path(&path)?;
    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ext.eq_ignore_ascii_case("md") {
        return Err(AppCommandError::invalid_input(
            "only markdown files can be read",
        ));
    }
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    let vault = vault_root_from_settings(&settings)?;
    let file = join_vault_relative(&vault, &path).map_err(AppCommandError::invalid_input)?;
    let vault_canon = vault
        .canonicalize()
        .map_err(|e| AppCommandError::io_error(e.to_string()))?;
    let file_canon = file.canonicalize().map_err(|_| {
        AppCommandError::new(crate::app_error::AppErrorCode::NotFound, "file not found")
    })?;
    if !file_canon.starts_with(&vault_canon) {
        return Err(AppCommandError::invalid_input(
            "path must stay inside the wiki vault",
        ));
    }
    if !file_canon.is_file() {
        return Err(AppCommandError::new(
            crate::app_error::AppErrorCode::NotFound,
            "file not found",
        ));
    }
    let content =
        fs::read_to_string(&file_canon).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    Ok(WikiVaultFile {
        path: path.replace('\\', "/"),
        content,
    })
}

// ── Tauri wrappers ──────────────────────────────────────────────────────────

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn get_wiki_settings(
    db: tauri::State<'_, AppDatabase>,
) -> Result<WikiSettingsView, AppCommandError> {
    get_wiki_settings_core(&db.conn)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn update_wiki_settings(
    db: tauri::State<'_, AppDatabase>,
    settings: WikiSettings,
) -> Result<WikiSettingsView, AppCommandError> {
    update_wiki_settings_core(&db.conn, settings)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_list_jobs(
    db: tauri::State<'_, AppDatabase>,
    limit: Option<u64>,
    offset: Option<u64>,
    status: Option<String>,
) -> Result<Vec<WikiJobInfo>, AppCommandError> {
    wiki_list_jobs_core(&db.conn, limit, offset, status)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_get_job(
    db: tauri::State<'_, AppDatabase>,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    wiki_get_job_core(&db.conn, id)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_list_sources(
    db: tauri::State<'_, AppDatabase>,
    limit: Option<u64>,
    offset: Option<u64>,
    source_kind: Option<String>,
    project_id: Option<String>,
) -> Result<Vec<WikiSourceInfo>, AppCommandError> {
    wiki_list_sources_core(&db.conn, limit, offset, source_kind, project_id)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_list_project_bindings(
    db: tauri::State<'_, AppDatabase>,
    vault_id: Option<String>,
) -> Result<Vec<WikiProjectBindingInfo>, AppCommandError> {
    wiki_list_project_bindings_core(&db.conn, vault_id)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_get_source(
    db: tauri::State<'_, AppDatabase>,
    id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    wiki_get_source_core(&db.conn, id)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_vault_tree(
    db: tauri::State<'_, AppDatabase>,
    path: Option<String>,
    recursive: Option<bool>,
    include_raw: Option<bool>,
) -> Result<Vec<WikiVaultTreeEntry>, AppCommandError> {
    wiki_vault_tree_core(
        &db.conn,
        WikiVaultTreeParams {
            path,
            recursive,
            include_raw,
        },
    )
    .await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_vault_read(
    db: tauri::State<'_, AppDatabase>,
    path: String,
) -> Result<WikiVaultFile, AppCommandError> {
    wiki_vault_read_core(&db.conn, path).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_import_text(
    db: tauri::State<'_, AppDatabase>,
    request_id: String,
    text: String,
    title: Option<String>,
    source_url: Option<String>,
    author: Option<String>,
    material_role: Option<String>,
    personal_role: Option<String>,
    project_ids: Option<Vec<String>>,
    area_ids: Option<Vec<String>>,
) -> Result<WikiImportResult, AppCommandError> {
    wiki_import_text_core(
        &db.conn,
        ImportTextParams {
            request_id,
            text,
            title,
            source_url,
            author,
            material_role,
            personal_role,
            project_ids,
            area_ids,
        },
    )
    .await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_import_files(
    db: tauri::State<'_, AppDatabase>,
    request_id: String,
    files: Vec<ImportFilePart>,
    material_role: Option<String>,
    personal_role: Option<String>,
    title: Option<String>,
    source_url: Option<String>,
    author: Option<String>,
    batch_id: Option<String>,
    project_ids: Option<Vec<String>>,
    area_ids: Option<Vec<String>>,
) -> Result<ImportFilesResult, AppCommandError> {
    wiki_import_files_core(
        &db.conn,
        ImportFilesParams {
            request_id,
            files,
            material_role,
            personal_role,
            title,
            source_url,
            author,
            batch_id,
            project_ids,
            area_ids,
        },
    )
    .await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_accept_extraction(
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    wiki_accept_extraction_core(&db.conn, source_id).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_update_source_annotations(
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
    material_role: Option<String>,
    personal_role: Option<String>,
    project_ids: Option<Vec<String>>,
    area_ids: Option<Vec<String>>,
) -> Result<WikiSourceInfo, AppCommandError> {
    wiki_update_source_annotations_core(
        &db.conn,
        UpdateAnnotationsParams {
            source_id,
            material_role,
            personal_role,
            project_ids,
            area_ids,
        },
    )
    .await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_reextract(
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
) -> Result<WikiImportResult, AppCommandError> {
    wiki_reextract_core(&db.conn, source_id).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_link_source_version(
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
    previous_source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    wiki_link_source_version_core(
        &db.conn,
        LinkVersionParams {
            source_id,
            previous_source_id,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::AppErrorCode;
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::settings::WikiSettings;

    #[tokio::test]
    async fn vault_tree_rejects_parent_dir() {
        let db = fresh_in_memory_db().await;
        let err = wiki_vault_tree_core(
            &db.conn,
            WikiVaultTreeParams {
                path: Some("work/../secrets".into()),
                recursive: Some(true),
                include_raw: None,
            },
        )
        .await
        .expect_err("parent segments");
        assert!(matches!(err.code, AppErrorCode::InvalidInput));
    }

    #[tokio::test]
    async fn vault_tree_recursive_skips_obsidian_and_raw() {
        let db = fresh_in_memory_db().await;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        crate::wiki::vault::initialize_vault(&vault).unwrap();
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join(".obsidian/app.json"), "{}").unwrap();
        std::fs::create_dir_all(vault.join("work/projects")).unwrap();
        std::fs::write(vault.join("work/projects/alpha.md"), "note").unwrap();
        std::fs::create_dir_all(vault.join("raw/sessions")).unwrap();
        std::fs::write(vault.join("raw/sessions/turn.md"), "raw").unwrap();

        let mut settings = WikiSettings::default();
        settings.vault_path = Some(vault.to_string_lossy().into_owned());
        settings::save_settings(&db.conn, &settings)
            .await
            .expect("save settings");

        let entries = wiki_vault_tree_core(
            &db.conn,
            WikiVaultTreeParams {
                path: None,
                recursive: Some(true),
                include_raw: Some(false),
            },
        )
        .await
        .expect("tree");

        fn contains(entries: &[WikiVaultTreeEntry], path: &str) -> bool {
            entries.iter().any(|e| {
                e.path == path
                    || e.children
                        .as_deref()
                        .is_some_and(|kids| contains(kids, path))
            })
        }

        assert!(contains(&entries, "work/projects/alpha.md"));
        assert!(contains(&entries, "index.md"));
        assert!(!contains(&entries, ".obsidian"));
        assert!(!contains(&entries, ".obsidian/app.json"));
        assert!(!contains(&entries, "raw"));
        assert!(!contains(&entries, "raw/sessions/turn.md"));
    }
}

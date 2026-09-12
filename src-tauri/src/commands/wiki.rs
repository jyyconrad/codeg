//! Wiki settings, source/job listing, and vault browse. Dual-mode `_core` fns.

use std::fs;
use std::path::Path;

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::app_error::AppCommandError;
use crate::db::error::DbError;
use crate::db::service::wiki_service::{self, WikiJobInfo, WikiSourceInfo};
use crate::wiki::paths::{self, join_vault_relative, resolve_vault_path};
use crate::wiki::settings::{self, WikiSettings, WikiSettingsView};
use crate::wiki::vault;

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
) -> Result<Vec<WikiSourceInfo>, DbError> {
    wiki_service::list_sources(
        conn,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        source_kind.as_deref(),
    )
    .await
}

pub async fn wiki_get_source_core(
    conn: &DatabaseConnection,
    id: String,
) -> Result<WikiSourceInfo, DbError> {
    wiki_service::get_source(conn, &id).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiVaultTreeEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
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
    path: Option<String>,
) -> Result<Vec<WikiVaultTreeEntry>, AppCommandError> {
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    let vault = vault_root_from_settings(&settings)?;
    let rel = path.unwrap_or_default();
    reject_unsafe_path(&rel)?;
    let dir = join_vault_relative(&vault, &rel).map_err(AppCommandError::invalid_input)?;
    if !dir.exists() {
        if rel.trim().is_empty() {
            return Ok(Vec::new());
        }
        return Err(AppCommandError::new(
            crate::app_error::AppErrorCode::NotFound,
            "path not found",
        ));
    }
    if !dir.is_dir() {
        return Err(AppCommandError::invalid_input("path is not a directory"));
    }
    let mut entries = Vec::new();
    let rd = fs::read_dir(&dir).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    for ent in rd {
        let ent = ent.map_err(|e| AppCommandError::io_error(e.to_string()))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let child = if rel.trim().is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel.trim().trim_end_matches('/'), name)
        };
        entries.push(WikiVaultTreeEntry {
            path: child.replace('\\', "/"),
            name,
            is_dir,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
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
    if !file.is_file() {
        return Err(AppCommandError::new(
            crate::app_error::AppErrorCode::NotFound,
            "file not found",
        ));
    }
    let content =
        fs::read_to_string(&file).map_err(|e| AppCommandError::io_error(e.to_string()))?;
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
) -> Result<Vec<WikiSourceInfo>, AppCommandError> {
    wiki_list_sources_core(&db.conn, limit, offset, source_kind)
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
) -> Result<Vec<WikiVaultTreeEntry>, AppCommandError> {
    wiki_vault_tree_core(&db.conn, path).await
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_vault_read(
    db: tauri::State<'_, AppDatabase>,
    path: String,
) -> Result<WikiVaultFile, AppCommandError> {
    wiki_vault_read_core(&db.conn, path).await
}

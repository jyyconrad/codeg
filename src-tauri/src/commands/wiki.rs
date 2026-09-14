//! 个人 Wiki 的桌面适配与共享设置/浏览用例。
//! 设置修改协调目录、模型建议和运行任务；导入直接交给 wiki/import 与 session_import。
//! 分页阅读由 wiki_read 适配读模型，任务控制由 wiki_engine 处理，避免多套查询逻辑。

use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::app_error::AppCommandError;
use crate::db::error::DbError;
use crate::db::service::wiki_service;
#[cfg(feature = "tauri-runtime")]
use crate::db::service::wiki_service::{
    WikiImportResult, WikiJobInfo, WikiProjectBindingInfo, WikiSourceInfo,
};
#[cfg(feature = "tauri-runtime")]
use crate::wiki::import::ImportFilePart;
#[cfg(feature = "tauri-runtime")]
use crate::wiki::import::{
    self, ImportBatchResult, ImportFilesParams, ImportTextParams, LinkVersionParams,
    UpdateAnnotationsParams,
};
use crate::wiki::paths::resolve_vault_path;
#[cfg(feature = "tauri-runtime")]
use crate::wiki::session_import::{
    self, ImportDirectoryParams, ImportLocalSessionsParams, WikiBulkImportResult,
};
use crate::wiki::settings::{self, WikiSettings, WikiSettingsView};
use crate::wiki::tree::{self, VaultTreeError};
use crate::wiki::vault;

#[cfg(feature = "tauri-runtime")]
use crate::wiki::read_model::WikiVaultFile;
pub use crate::wiki::tree::WikiVaultTreeEntry;

#[cfg(feature = "tauri-runtime")]
use crate::db::AppDatabase;

pub async fn get_wiki_settings_core(
    conn: &DatabaseConnection,
) -> Result<WikiSettingsView, DbError> {
    {
        let _transition = crate::wiki::lifecycle::lock().await;
        crate::wiki::relocate::relocate_legacy_app_data_wiki(conn).await?;
    }
    wiki_settings_view(conn).await
}

async fn wiki_settings_view(conn: &DatabaseConnection) -> Result<WikiSettingsView, DbError> {
    let mut settings = settings::load_settings(conn).await?;
    // Suggestions are only offered before the first save. A missing slot in
    // saved configuration must remain visible instead of tracking chat changes.
    if crate::db::service::app_metadata_service::get_value(conn, settings::WIKI_SETTINGS_KEY)
        .await?
        .is_none()
    {
        hydrate_default_bindings(conn, &mut settings).await;
    }
    let next_compile_at = settings::next_compile_at(&settings);
    let pending_source_count = crate::wiki::read_model::get_overview(conn)
        .await?
        .pending_memory_count as u64;
    let resolved_vault_path = resolve_vault_path(settings.vault_path.as_deref())
        .to_string_lossy()
        .into_owned();
    Ok(WikiSettingsView {
        settings,
        next_compile_at,
        pending_source_count,
        turn_summary_builtin_prompt: settings::WIKI_TURN_SUMMARY_BUILTIN.to_string(),
        session_rollup_builtin_prompt: settings::WIKI_SESSION_ROLLUP_BUILTIN.to_string(),
        synthesize_builtin_prompt: settings::WIKI_SYNTHESIZE_BUILTIN.to_string(),
        resolved_vault_path,
    })
}

async fn hydrate_default_bindings(conn: &DatabaseConnection, settings: &mut WikiSettings) {
    let Some(agent) = crate::db::service::agent_setting_service::get_by_agent_type(
        conn,
        crate::models::AgentType::CodegAgent,
    )
    .await
    .ok()
    .flatten() else {
        return;
    };
    let Some(provider_id) = agent.model_provider_id else {
        return;
    };
    let Some(provider) = crate::db::service::model_provider_service::get_by_id(conn, provider_id)
        .await
        .ok()
        .flatten()
    else {
        return;
    };
    let model_id = crate::acp::native_config::completions_model_id(provider.model.as_deref());
    for slot in [&mut settings.turn_summary, &mut settings.session_rollup] {
        if slot.provider_id.is_none() {
            slot.provider_id = Some(provider_id);
            slot.model_id = model_id.clone();
        }
    }
    if settings.synthesize.provider_id.is_none() {
        settings.synthesize.provider_id = Some(provider_id);
        settings.synthesize.model_id = model_id;
    }
}

pub async fn update_wiki_settings_core(
    conn: &DatabaseConnection,
    settings: WikiSettings,
) -> Result<WikiSettingsView, DbError> {
    let transition = crate::wiki::lifecycle::lock().await;
    crate::wiki::relocate::relocate_legacy_app_data_wiki(conn).await?;
    settings::validate_settings(&settings)?;
    let previous = settings::load_settings(conn).await?;
    let vault_path = resolve_vault_path(settings.vault_path.as_deref());
    let previous_path = resolve_vault_path(previous.vault_path.as_deref());
    if vault_path != previous_path {
        let running = wiki_service::list_jobs_by_status(conn, "running").await?;
        if !running.is_empty() {
            return Err(DbError::Conflict(
                "wait for current Wiki processing before changing its location".into(),
            ));
        }
    }
    if settings.enabled || vault_path != previous_path {
        vault::validate_new_location(&vault_path)?;
        vault::initialize_vault(&vault_path)?;
        vault::initialize_state_root(&crate::wiki::paths::resolve_state_root())?;
        let canonical = vault_path.canonicalize()?.to_string_lossy().to_string();
        wiki_service::ensure_active_vault(conn, &canonical).await?;
        let _ = settings::ensure_db_instance_id(conn).await?;
    }
    settings::save_settings(conn, &settings).await?;
    if previous.enabled && !settings.enabled {
        for job in wiki_service::list_jobs_by_status(conn, "running").await? {
            crate::wiki::engine::request_cancel(&job.id);
        }
    }
    crate::wiki::engine::notify_jobs();
    drop(transition);
    wiki_settings_view(conn).await
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

pub async fn wiki_vault_tree_core(
    conn: &DatabaseConnection,
    params: WikiVaultTreeParams,
) -> Result<Vec<WikiVaultTreeEntry>, AppCommandError> {
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    let vault = resolve_vault_path(settings.vault_path.as_deref());
    let rel = params.path.unwrap_or_default();
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
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    settings: WikiSettings,
) -> Result<WikiSettingsView, AppCommandError> {
    let result = async {
        update_wiki_settings_core(&db.conn, settings)
            .await
            .map_err(AppCommandError::from)
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn wiki_get_job(
    db: tauri::State<'_, AppDatabase>,
    id: String,
) -> Result<WikiJobInfo, AppCommandError> {
    crate::wiki::read_model::read_job(&db.conn, &id)
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_list_project_bindings(
    db: tauri::State<'_, AppDatabase>,
    vault_id: Option<String>,
) -> Result<Vec<WikiProjectBindingInfo>, AppCommandError> {
    wiki_service::list_project_bindings(&db.conn, vault_id.as_deref())
        .await
        .map_err(AppCommandError::from)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
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
    crate::wiki::read_model::read_vault_file(&db.conn, path)
        .await
        .map_err(super::wiki_read::map_error)
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
#[allow(clippy::too_many_arguments)] // Tauri exposes the existing scalar import arguments.
pub async fn wiki_import_text(
    app: tauri::AppHandle,
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
    let result = async {
        import::import_text(
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
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
#[allow(clippy::too_many_arguments)] // Tauri exposes the existing scalar import arguments.
pub async fn wiki_import_files(
    app: tauri::AppHandle,
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
) -> Result<ImportBatchResult, AppCommandError> {
    let result = async {
        import::import_files_with_result(
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
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_accept_extraction(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    let result = async { import::accept_extraction(&db.conn, source_id).await }.await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_update_source_annotations(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
    material_role: Option<String>,
    personal_role: Option<String>,
    project_ids: Option<Vec<String>>,
    area_ids: Option<Vec<String>>,
) -> Result<WikiSourceInfo, AppCommandError> {
    let result = async {
        import::update_source_annotations(
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
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_reextract(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
) -> Result<WikiImportResult, AppCommandError> {
    let result = async { import::reextract(&db.conn, source_id).await }.await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_link_source_version(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    source_id: String,
    previous_source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    let result = async {
        import::link_source_version(
            &db.conn,
            LinkVersionParams {
                source_id,
                previous_source_id,
            },
        )
        .await
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_import_local_sessions(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    request_id: String,
    selections: Option<Vec<crate::models::SelectedSessionKey>>,
    all: Option<bool>,
) -> Result<WikiBulkImportResult, AppCommandError> {
    let result = async {
        session_import::import_local_sessions(
            &db.conn,
            ImportLocalSessionsParams {
                request_id,
                selections: selections.unwrap_or_default(),
                all: all.unwrap_or(false),
            },
        )
        .await
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
}

#[cfg(feature = "tauri-runtime")]
#[cfg_attr(feature = "tauri-runtime", tauri::command(rename_all = "snake_case"))]
pub async fn wiki_import_directory(
    app: tauri::AppHandle,
    db: tauri::State<'_, AppDatabase>,
    request_id: String,
    path: String,
) -> Result<WikiBulkImportResult, AppCommandError> {
    let result = async {
        session_import::import_directory(&db.conn, ImportDirectoryParams { request_id, path }).await
    }
    .await;
    if result.is_ok() {
        crate::wiki::events::content_changed(
            &db.conn,
            &crate::web::event_bridge::EventEmitter::Tauri(app),
        )
        .await;
    }
    result
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

        let settings = WikiSettings {
            vault_path: Some(vault.to_string_lossy().into_owned()),
            ..Default::default()
        };
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

    #[tokio::test]
    async fn enabling_personal_wiki_accepts_existing_file_wiki_directory() {
        let db = fresh_in_memory_db().await;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("wiki");
        std::fs::create_dir_all(vault.join("work/records")).unwrap();
        std::fs::write(vault.join("index.md"), "home").unwrap();
        std::fs::write(vault.join("work/records/kept.md"), "keep me").unwrap();
        let home = dir.path().to_string_lossy().into_owned();
        temp_env::async_with_vars(
            [
                ("CODEG_HOME", Some(home.as_str())),
                ("CODEG_DATA_DIR", None::<&str>),
            ],
            async {
                let view = update_wiki_settings_core(
                    &db.conn,
                    WikiSettings {
                        enabled: true,
                        vault_path: Some(vault.to_string_lossy().into_owned()),
                        ..Default::default()
                    },
                )
                .await
                .expect("enable existing file wiki");
                assert!(view.settings.enabled);
                assert_eq!(
                    std::fs::read_to_string(vault.join("work/records/kept.md")).unwrap(),
                    "keep me"
                );
                assert!(vault.join(crate::wiki::vault::FORMAT_MARKER).is_file());
            },
        )
        .await;
    }
}

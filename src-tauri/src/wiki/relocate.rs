//! 将桌面应用数据目录里的 Wiki 正文和运行状态迁到 ~/.codeg。
//! 保留同一份资料库身份、来源和任务；正文相对路径不变，只改目录指针与提交记录中的绝对地址。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sea_orm::DatabaseConnection;

use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::paths::{self, resolve_state_root, resolve_vault_path};
use crate::wiki::settings::{self, WikiSettings};
use crate::wiki::vault;

pub async fn relocate_legacy_app_data_wiki(conn: &DatabaseConnection) -> Result<PathBuf, DbError> {
    let dest_vault = resolve_vault_path(None);
    let dest_state = resolve_state_root();
    let mut settings = settings::load_settings(conn).await?;
    let sources = vault_sources(&settings);
    if sources.iter().all(|src| same_dir(src, &dest_vault)) {
        return Ok(dest_vault);
    }
    let copied_from = copy_first_existing(&sources, &dest_vault)?;
    copy_first_existing(&paths::legacy_app_data_state_dirs(), &dest_state)?;
    if dest_vault.exists() {
        vault::initialize_vault(&dest_vault)?;
        vault::initialize_state_root(&dest_state)?;
        rewrite_commit_vaults(&dest_state, &sources, &dest_vault)?;
        let mut from_paths = sources;
        if let Some(from) = copied_from {
            if !from_paths.iter().any(|path| same_dir(path, &from)) {
                from_paths.push(from);
            }
        }
        for from in &from_paths {
            let _ = wiki_service::retarget_canonical_path(
                conn,
                &from.to_string_lossy(),
                &dest_vault.to_string_lossy(),
            )
            .await?;
        }
        if should_clear_stored_path(&settings, &dest_vault) {
            settings.vault_path = None;
            settings::save_settings(conn, &settings).await?;
        }
    }
    Ok(dest_vault)
}

fn vault_sources(settings: &WikiSettings) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(stored) = settings
        .vault_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
    {
        if !is_legacy_app_data_path(&stored) {
            return dirs;
        }
        dirs.push(stored);
    }
    for dir in paths::legacy_app_data_wiki_dirs() {
        if !dirs.iter().any(|existing| same_dir(existing, &dir)) {
            dirs.push(dir);
        }
    }
    dirs
}

fn is_legacy_app_data_path(path: &Path) -> bool {
    paths::legacy_app_data_wiki_dirs()
        .into_iter()
        .any(|legacy| same_dir(&legacy, path))
}

fn should_clear_stored_path(settings: &WikiSettings, dest: &Path) -> bool {
    match settings
        .vault_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => false,
        Some(stored) => {
            is_legacy_app_data_path(Path::new(stored)) || same_dir(Path::new(stored), dest)
        }
    }
}

fn copy_first_existing(sources: &[PathBuf], dest: &Path) -> io::Result<Option<PathBuf>> {
    let mut copied = None;
    for src in sources {
        if !src.exists() || same_dir(src, dest) {
            continue;
        }
        copy_tree(src, dest)?;
        if copied.is_none() {
            copied = Some(src.clone());
        }
    }
    Ok(copied)
}

fn copy_tree(src: &Path, dest: &Path) -> io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".codeg-wiki.lock" {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&from, &to)?;
        } else if !to.exists() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn rewrite_commit_vaults(state: &Path, from: &[PathBuf], dest: &Path) -> io::Result<()> {
    let dir = state.join("commits");
    if !dir.is_dir() {
        return Ok(());
    }
    let dest_s = dest.to_string_lossy();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let mut text = fs::read_to_string(&path)?;
        let mut changed = false;
        for src in from {
            let src_s = src.to_string_lossy();
            if src_s.is_empty() || src_s == dest_s {
                continue;
            }
            if text.contains(src_s.as_ref()) {
                text = text.replace(src_s.as_ref(), dest_s.as_ref());
                changed = true;
            }
        }
        if changed {
            fs::write(path, text)?;
        }
    }
    Ok(())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::settings::WikiSettings;
    use crate::wiki::vault::{self, FORMAT_MARKER};

    #[tokio::test]
    async fn copies_app_data_wiki_and_state_to_home_without_new_vault_id() {
        let db = fresh_in_memory_db().await;
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let src_vault = data.path().join("wiki");
        let src_state = data.path().join("wiki-state");
        vault::initialize_vault(&src_vault).unwrap();
        vault::initialize_state_root(&src_state).unwrap();
        fs::create_dir_all(src_vault.join("work/turns")).unwrap();
        fs::write(src_vault.join("work/turns/kept.md"), "turn note").unwrap();
        fs::create_dir_all(src_state.join("originals/abc")).unwrap();
        fs::write(src_state.join("originals/abc/file.bin"), b"raw-bytes").unwrap();
        fs::create_dir_all(src_state.join("commits")).unwrap();
        fs::write(
            src_state.join("commits/job.json"),
            format!(
                "{{\"job_id\":\"job\",\"vault\":\"{}\"}}\n",
                src_vault.to_string_lossy()
            ),
        )
        .unwrap();
        let vault = wiki_service::ensure_active_vault(&db.conn, &src_vault.to_string_lossy())
            .await
            .unwrap();
        settings::save_settings(
            &db.conn,
            &WikiSettings {
                enabled: true,
                vault_path: Some(src_vault.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let vault_id = vault.id.clone();
        temp_env::async_with_vars(
            [
                ("CODEG_HOME", Some(home.path().to_str().unwrap())),
                ("CODEG_DATA_DIR", Some(data.path().to_str().unwrap())),
            ],
            async {
                let dest = relocate_legacy_app_data_wiki(&db.conn).await.unwrap();
                assert_eq!(dest, home.path().join("wiki"));
                assert_eq!(
                    fs::read_to_string(dest.join("work/turns/kept.md")).unwrap(),
                    "turn note"
                );
                assert!(dest.join(FORMAT_MARKER).is_file());
                assert_eq!(
                    fs::read_to_string(home.path().join("wiki-state/originals/abc/file.bin"))
                        .unwrap(),
                    "raw-bytes"
                );
                let commit =
                    fs::read_to_string(home.path().join("wiki-state/commits/job.json")).unwrap();
                assert!(commit.contains(&dest.to_string_lossy().into_owned()));
                assert!(!commit.contains(&src_vault.to_string_lossy().into_owned()));
                let active = wiki_service::active_vault(&db.conn).await.unwrap().unwrap();
                assert_eq!(active.id, vault_id);
                assert_eq!(
                    PathBuf::from(&active.canonical_path)
                        .canonicalize()
                        .unwrap(),
                    dest.canonicalize().unwrap()
                );
                let settings = settings::load_settings(&db.conn).await.unwrap();
                assert!(settings.vault_path.is_none());
                relocate_legacy_app_data_wiki(&db.conn).await.unwrap();
                assert_eq!(
                    fs::read_to_string(dest.join("work/turns/kept.md")).unwrap(),
                    "turn note"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn custom_vault_outside_app_data_is_left_alone() {
        let db = fresh_in_memory_db().await;
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        vault::initialize_vault(custom.path()).unwrap();
        fs::write(custom.path().join("index.md"), "custom home").unwrap();
        let leftover = data.path().join("wiki");
        vault::initialize_vault(&leftover).unwrap();
        fs::write(leftover.join("index.md"), "must not move").unwrap();
        settings::save_settings(
            &db.conn,
            &WikiSettings {
                enabled: true,
                vault_path: Some(custom.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        temp_env::async_with_vars(
            [
                ("CODEG_HOME", Some(home.path().to_str().unwrap())),
                ("CODEG_DATA_DIR", Some(data.path().to_str().unwrap())),
            ],
            async {
                relocate_legacy_app_data_wiki(&db.conn).await.unwrap();
                assert_eq!(
                    fs::read_to_string(custom.path().join("index.md")).unwrap(),
                    "custom home"
                );
                assert!(!home.path().join("wiki/index.md").exists());
                let settings = settings::load_settings(&db.conn).await.unwrap();
                assert_eq!(
                    settings.vault_path.as_deref(),
                    Some(custom.path().to_string_lossy().as_ref())
                );
            },
        )
        .await;
    }
}

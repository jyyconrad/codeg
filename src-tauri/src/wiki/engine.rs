//! WikiWorker process lock, vault lock, cron tick, crash recovery.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::db::entities::wiki_job;
use crate::db::AppDatabase;
use crate::web::event_bridge::{emit_event, EventEmitter};
use crate::wiki::commit::{self, RecoverStatus};
use crate::wiki::compile;
use crate::wiki::llm::{self, ProductionWikiLlm, WikiLlm};
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::settings::{self, WikiSettings};
use crate::wiki::vault;
use crate::wiki::worker;
use crate::db::service::wiki_service;

pub const WIKI_JOB_CHANGED_EVENT: &str = "wiki://job-changed";
const TICK_SECS: u64 = 5;
const JOB_BUDGET_MINUTES: i64 = 30;

static WAKE: OnceLock<Arc<Notify>> = OnceLock::new();

fn wake_signal() -> Arc<Notify> {
    WAKE.get_or_init(|| Arc::new(Notify::new())).clone()
}

pub fn notify_jobs() {
    wake_signal().notify_one();
}

enum Ownership {
    Exclusive(File),
    Taken,
    Unavailable,
}

fn acquire_lock(path: &Path) -> Ownership {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!("[wiki] engine lock dir: {e}");
            return Ownership::Unavailable;
        }
    }
    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("[wiki] engine lock open failed: {e}");
            return Ownership::Unavailable;
        }
    };
    match file.try_lock() {
        Ok(()) => Ownership::Exclusive(file),
        Err(std::fs::TryLockError::WouldBlock) => Ownership::Taken,
        Err(std::fs::TryLockError::Error(e)) => {
            tracing::warn!("[wiki] engine lock failed: {e}");
            Ownership::Unavailable
        }
    }
}

struct WikiEngine {
    db: AppDatabase,
    emitter: EventEmitter,
    state_root: PathBuf,
    _engine_lock: File,
}

fn try_build_engine(
    db: AppDatabase,
    emitter: EventEmitter,
    state_root: PathBuf,
) -> Option<WikiEngine> {
    let _ = vault::initialize_state_root(&state_root);
    let lock_path = state_root.join("engine.lock");
    let lock = match acquire_lock(&lock_path) {
        Ownership::Exclusive(f) => f,
        Ownership::Taken => {
            tracing::info!(
                "[wiki] another process holds the wiki engine lock at {}",
                lock_path.display()
            );
            return None;
        }
        Ownership::Unavailable => {
            tracing::warn!("[wiki] wiki engine lock unavailable; worker disabled");
            return None;
        }
    };
    Some(WikiEngine {
        db,
        emitter,
        state_root,
        _engine_lock: lock,
    })
}

/// Start the wiki engine if this process wins the wiki-state OS lock.
/// Second spawn against the same lock file is a no-op.
pub fn spawn(db: AppDatabase, emitter: EventEmitter) {
    let state_root = resolve_state_root(None);
    let Some(engine) = try_build_engine(db, emitter, state_root) else {
        return;
    };
    #[cfg(feature = "tauri-runtime")]
    {
        tauri::async_runtime::spawn(engine.run());
        return;
    }
    #[cfg(not(feature = "tauri-runtime"))]
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        h.spawn(engine.run());
    } else {
        tracing::warn!("[wiki] no tokio runtime to spawn the engine");
    }
}

pub fn spawn_with_roots(
    db: AppDatabase,
    emitter: EventEmitter,
    state_root: PathBuf,
    _vault_override: Option<PathBuf>,
) -> Option<JoinHandle<()>> {
    let engine = try_build_engine(db, emitter, state_root)?;
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        Some(h.spawn(engine.run()))
    } else {
        None
    }
}

impl WikiEngine {
    async fn run(self) {
        if let Err(e) = recover_on_start(&self.db.conn, &self.state_root, &self.emitter).await {
            tracing::warn!("[wiki] recovery error: {e}");
        }
        let mut tick = tokio::time::interval(Duration::from_secs(TICK_SECS));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let wake = wake_signal();
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = wake.notified() => {}
            }
            if let Err(e) = self.tick_once().await {
                tracing::warn!("[wiki] engine tick: {e}");
            }
        }
    }

    async fn tick_once(&self) -> Result<(), String> {
        let conn = &self.db.conn;
        let settings = settings::load_settings(conn)
            .await
            .map_err(|e| e.to_string())?;
        if !settings.enabled {
            return Ok(());
        }
        let vault = resolve_vault_path(settings.vault_path.as_deref());
        let _ = vault::initialize_vault(&vault);
        let _ = vault::initialize_state_root(&self.state_root);

        expire_overdue_jobs(conn, &self.emitter).await;

        if let Some(job) = wiki_service::claim_next_queued_job(conn, "ingest")
            .await
            .map_err(|e| e.to_string())?
        {
            emit_job(&self.emitter, &job.id, "running");
            handle_ingest(conn, &job, &settings, &vault, &self.emitter).await;
        }

        maybe_enqueue_scheduled_compile(conn, &settings, &vault).await;

        if settings.compile.enabled {
            if let Some(job) = wiki_service::claim_next_queued_job(conn, "compile")
                .await
                .map_err(|e| e.to_string())?
            {
                emit_job(&self.emitter, &job.id, "running");
                handle_compile(conn, &job, &settings, &vault, &self.state_root, &self.emitter)
                    .await;
            }
        }
        Ok(())
    }
}

async fn recover_on_start(
    conn: &DatabaseConnection,
    state_root: &Path,
    emitter: &EventEmitter,
) -> Result<(), String> {
    // 1. Resume commit manifests. Do not requeue running jobs to rewrite files.
    for (job_id, status) in commit::recover_all(state_root) {
        match status {
            Ok(RecoverStatus::Complete) => {
                if let Ok(job) = wiki_service::get_job_model(conn, &job_id).await {
                    if job.status == "running" {
                        if let Some(raw) = job.input_manifest.as_deref() {
                            if let Ok(manifest) =
                                serde_json::from_str::<compile::CompileJobManifest>(raw)
                            {
                                let _ = compile::register_consumed(conn, &job_id, &manifest).await;
                            }
                        }
                        let _ = wiki_service::mark_job(conn, &job_id, "succeeded", None, None).await;
                        emit_job(emitter, &job_id, "succeeded");
                    }
                }
            }
            Ok(RecoverStatus::PartialConflict { rel, reason }) => {
                let msg = format!("{rel}: {reason}");
                let _ = wiki_service::mark_job(
                    conn,
                    &job_id,
                    "failed",
                    Some("conflict"),
                    Some(&msg),
                )
                .await;
                emit_job(emitter, &job_id, "failed");
            }
            Err(e) => {
                tracing::warn!(job_id = %job_id, "[wiki] commit recovery: {e}");
            }
        }
    }

    // 2. Interrupted jobs: ingest with raw stays summarizable; compile without
    // a resume-able manifest fails (never blindly rewrite).
    let running = wiki_service::list_jobs_by_status(conn, "running")
        .await
        .map_err(|e| e.to_string())?;
    for job in running {
        if job.kind == "ingest" {
            let _ = wiki_service::mark_job(conn, &job.id, "queued", None, None).await;
            emit_job(emitter, &job.id, "queued");
            continue;
        }
        if job.kind == "compile" {
            if commit::load_manifest(state_root, &job.id)
                .ok()
                .flatten()
                .is_some()
            {
                continue;
            }
            let _ = wiki_service::mark_job(
                conn,
                &job.id,
                "failed",
                Some("interrupted"),
                Some("compile interrupted before a commit manifest was persisted"),
            )
            .await;
            emit_job(emitter, &job.id, "failed");
        }
    }
    Ok(())
}

async fn handle_ingest(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    emitter: &EventEmitter,
) {
    let llm = match bind_optional(conn, settings.ingest.model_id.as_deref(), "ingest").await {
        Ok(v) => v,
        Err(e) => {
            tracing::info!("[wiki] ingest summary without model: {e}");
            None
        }
    };
    let llm_ref = llm.as_ref().map(|p| p as &dyn WikiLlm);
    match worker::run_ingest_summary(conn, job, llm_ref, vault).await {
        Ok(_) => {
            let _ = wiki_service::mark_job(conn, &job.id, "succeeded", None, None).await;
            emit_job(emitter, &job.id, "succeeded");
        }
        Err(e) => {
            let _ = wiki_service::mark_job(
                conn,
                &job.id,
                "failed",
                Some(e.error_code()),
                Some(&e.to_string()),
            )
            .await;
            emit_job(emitter, &job.id, "failed");
        }
    }
}

async fn handle_compile(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    state_root: &Path,
    emitter: &EventEmitter,
) {
    if !settings.compile.enabled {
        let _ = wiki_service::mark_job(
            conn,
            &job.id,
            "failed",
            Some(llm::BLOCKED_BY_CONFIGURATION),
            Some("compile is disabled"),
        )
        .await;
        emit_job(emitter, &job.id, "failed");
        return;
    }
    let bound = match llm::bind_wiki_model(conn, settings.compile.model_id.as_deref()).await {
        Ok(b) => b,
        Err(e) => {
            let _ = wiki_service::set_job_model_meta(conn, &job.id, None, None).await;
            let _ = wiki_service::mark_job(
                conn,
                &job.id,
                "failed",
                Some(e.error_code()),
                Some(&e.to_string()),
            )
            .await;
            emit_job(emitter, &job.id, "failed");
            return;
        }
    };
    let model_id = bound.model_id.clone();
    let protocol = bound.protocol.as_str().to_string();
    let _ = wiki_service::set_job_model_meta(conn, &job.id, Some(&model_id), Some(&protocol)).await;
    let skill = include_str!("../../agent-skills/wiki-compile/SKILL.md").to_string();
    let llm = ProductionWikiLlm::new(bound, skill, settings.compile.prompt.clone());
    match worker::run_compile_attempt(conn, job, &llm, vault, state_root).await {
        Ok(()) => {
            let _ = wiki_service::mark_job(conn, &job.id, "succeeded", None, None).await;
            emit_job(emitter, &job.id, "succeeded");
        }
        Err(e) => {
            let code = e.error_code();
            let retryable = match &e {
                worker::WorkerError::Compile(c) => c.retryable(),
                _ => false,
            };
            let status = "failed";
            let _ = wiki_service::mark_job(conn, &job.id, status, Some(code), Some(&e.to_string()))
                .await;
            if retryable && job.attempt < 3 {
                // Backoff 1/5/15 minutes is recorded; next claim waits via started_at.
                let _ = wiki_service::requeue_after_failure(conn, &job.id, job.attempt).await;
            }
            emit_job(emitter, &job.id, status);
        }
    }
}

async fn bind_optional(
    conn: &DatabaseConnection,
    model_id: Option<&str>,
    skill: &str,
) -> Result<Option<ProductionWikiLlm>, llm::WikiLlmError> {
    match llm::bind_wiki_model(conn, model_id).await {
        Ok(bound) => {
            let body = if skill == "ingest" {
                include_str!("../../agent-skills/wiki-ingest/SKILL.md").to_string()
            } else {
                include_str!("../../agent-skills/wiki-compile/SKILL.md").to_string()
            };
            Ok(Some(ProductionWikiLlm::new(bound, body, None)))
        }
        Err(llm::WikiLlmError::Blocked(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn maybe_enqueue_scheduled_compile(
    conn: &DatabaseConnection,
    settings: &WikiSettings,
    vault: &Path,
) {
    if !settings.compile.enabled {
        return;
    }
    let Some(vault_row) = wiki_service::active_vault(conn).await.ok().flatten() else {
        return;
    };
    let now = Utc::now();
    let due = vault_row
        .next_compile_at
        .map(|t| t <= now)
        .unwrap_or_else(|| settings::next_compile_at(settings).is_some());
    if !due {
        // First run: seed next_compile_at.
        if vault_row.next_compile_at.is_none() {
            if let Some(next) = settings::next_compile_at(settings) {
                let _ = wiki_service::set_vault_next_compile_at(conn, &vault_row.id, Some(next))
                    .await;
            }
        }
        return;
    }
    let cutoff = wiki_service::max_source_seq(conn, &vault_row.id)
        .await
        .ok()
        .flatten()
        .unwrap_or(0);
    let Ok(manifest) = compile::freeze_manifest(conn, &vault_row.id, cutoff, vault).await else {
        return;
    };
    if manifest.inputs.is_empty() {
        if let Some(next) = next_after(settings, now) {
            let _ = wiki_service::set_vault_next_compile_at(conn, &vault_row.id, Some(next)).await;
        }
        return;
    }
    // Coalesce missed ticks: one catch-up keyed by scheduled_for date.
    let scheduled_for = now.date_naive().to_string();
    let dedupe = format!("{}:compile:scheduled:{scheduled_for}", vault_row.id);
    let json = serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".into());
    if wiki_service::find_job_by_dedupe_key(conn, &dedupe)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        let _ = wiki_service::insert_compile_job(conn, &vault_row.id, &dedupe, Some(&json)).await;
        notify_jobs();
    }
    if let Some(next) = next_after(settings, now) {
        let _ = wiki_service::set_vault_next_compile_at(conn, &vault_row.id, Some(next)).await;
    }
}

fn next_after(settings: &WikiSettings, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    crate::db::service::automation_service::compute_next_run(
        &settings.compile_cron,
        &settings.timezone,
        after,
    )
    .ok()
    .flatten()
}

async fn expire_overdue_jobs(conn: &DatabaseConnection, emitter: &EventEmitter) {
    let Ok(running) = wiki_service::list_jobs_by_status(conn, "running").await else {
        return;
    };
    let now = Utc::now();
    for job in running {
        let Some(started) = job.started_at else {
            continue;
        };
        if now.signed_duration_since(started).num_minutes() < JOB_BUDGET_MINUTES {
            continue;
        }
        let _ = wiki_service::mark_job(
            conn,
            &job.id,
            "failed",
            Some("timeout"),
            Some("job exceeded the 30 minute budget"),
        )
        .await;
        emit_job(emitter, &job.id, "failed");
    }
}

fn emit_job(emitter: &EventEmitter, id: &str, status: &str) {
    emit_event(
        emitter,
        WIKI_JOB_CHANGED_EVENT,
        serde_json::json!({
            "id": id,
            "status": status,
            "version": 1,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::fresh_in_memory_db;
    use tempfile::tempdir;

    #[tokio::test]
    async fn second_spawn_is_noop() {
        let dir = tempdir().unwrap();
        let state = dir.path().join("wiki-state");
        fs::create_dir_all(&state).unwrap();
        let db1 = fresh_in_memory_db().await;
        let db2 = AppDatabase {
            conn: db1.conn.clone(),
        };
        let h1 = spawn_with_roots(db1, EventEmitter::Noop, state.clone(), None);
        assert!(h1.is_some(), "first spawn must take the engine lock");
        let h2 = spawn_with_roots(db2, EventEmitter::Noop, state, None);
        assert!(h2.is_none(), "second spawn must be a no-op");
        h1.unwrap().abort();
    }

    #[test]
    fn engine_lock_second_acquire_is_taken() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("engine.lock");
        let a = acquire_lock(&path);
        assert!(matches!(a, Ownership::Exclusive(_)));
        let b = acquire_lock(&path);
        assert!(matches!(b, Ownership::Taken));
    }
}

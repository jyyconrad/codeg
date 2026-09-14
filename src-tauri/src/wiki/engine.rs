//! WikiWorker process lock, vault lock, cron tick, crash recovery.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::db::entities::wiki_job;
use crate::db::service::wiki_service;
use crate::db::AppDatabase;
use crate::web::event_bridge::{emit_event, EventEmitter};
use crate::wiki::commit::{self, RecoverStatus};
use crate::wiki::compile;
use crate::wiki::llm::{self, ProductionWikiLlm};
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::settings::{self, WikiSettings};
use crate::wiki::vault;
use crate::wiki::worker;

pub const WIKI_JOB_CHANGED_EVENT: &str = "wiki://job-changed";
const TICK_SECS: u64 = 5;
const JOB_BUDGET_MINUTES: i64 = 30;

static WAKE: OnceLock<Arc<Notify>> = OnceLock::new();
static CANCELLATIONS: OnceLock<Mutex<HashMap<String, Arc<Notify>>>> = OnceLock::new();

fn wake_signal() -> Arc<Notify> {
    WAKE.get_or_init(|| Arc::new(Notify::new())).clone()
}

pub fn notify_jobs() {
    wake_signal().notify_one();
}

fn cancellation_map() -> &'static Mutex<HashMap<String, Arc<Notify>>> {
    CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cancellation_token(job_id: &str) -> Arc<Notify> {
    let mut map = cancellation_map()
        .lock()
        .expect("wiki cancellation map poisoned");
    map.entry(job_id.to_string())
        .or_insert_with(|| Arc::new(Notify::new()))
        .clone()
}

/// Signal an in-flight worker to stop before it can continue to a commit.
pub fn request_cancel(job_id: &str) {
    let token = cancellation_token(job_id);
    // `notify_one` retains a permit when cancellation races waiter setup.
    token.notify_one();
    notify_jobs();
}

pub fn clear_cancellation(job_id: &str) {
    if let Ok(mut map) = cancellation_map().lock() {
        map.remove(job_id);
    }
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
        let recovery_root = settings::load_settings(&self.db.conn)
            .await
            .map(|s| resolve_state_root(s.vault_path.as_deref()))
            .unwrap_or_else(|_| self.state_root.clone());
        // Enqueue writes to the stable default pending directory before it
        // can consult async settings. Scan both roots so a vault switch cannot
        // strand snapshots created before the worker observes the new path.
        if let Err(e) = crate::wiki::source::recover_pending(&self.db.conn).await {
            tracing::warn!("[wiki] pending ACP recovery error: {e}");
        }
        if recovery_root != resolve_state_root(None) {
            if let Err(e) =
                crate::wiki::source::recover_pending_at(&self.db.conn, &recovery_root).await
            {
                tracing::warn!("[wiki] configured pending ACP recovery error: {e}");
            }
        }
        if let Err(e) = recover_on_start(&self.db.conn, &recovery_root, &self.emitter).await {
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
        // Resolve vault and state together from the same settings snapshot.
        // This prevents a vault switch from reusing a stale state root.
        let state_root = resolve_state_root(settings.vault_path.as_deref());
        let _ = vault::initialize_state_root(&state_root);

        expire_overdue_jobs(conn, &self.emitter).await;
        backfill_memory_pipeline(conn, &vault).await;

        if let Some(job) = wiki_service::claim_next_queued_job(conn, "turn_summary")
            .await
            .map_err(|e| e.to_string())?
        {
            emit_job(&self.emitter, &job.id, "running");
            handle_turn_summary(conn, &job, &settings, &vault, &state_root, &self.emitter).await;
        }

        if let Some(job) = claim_next_session_rollup(conn)
            .await
            .map_err(|e| e.to_string())?
        {
            emit_job(&self.emitter, &job.id, "running");
            handle_session_rollup(conn, &job, &settings, &vault, &state_root, &self.emitter).await;
        }

        maybe_enqueue_scheduled_compile(conn, &settings, &vault).await;

        if settings.synthesize.enabled {
            if let Some(job) = wiki_service::claim_next_queued_job(conn, "wiki_synthesize")
                .await
                .map_err(|e| e.to_string())?
            {
                emit_job(&self.emitter, &job.id, "running");
                handle_compile(conn, &job, &settings, &vault, &state_root, &self.emitter).await;
            }
        }
        Ok(())
    }
}

async fn claim_next_session_rollup(
    conn: &DatabaseConnection,
) -> Result<Option<wiki_job::Model>, crate::db::error::DbError> {
    let queued = wiki_service::list_queued_jobs_by_kind(conn, "session_rollup").await?;
    for job in queued {
        let Some(cid) = crate::wiki::session_rollup::conversation_id_from_job(&job) else {
            if let Some(claimed) = wiki_service::claim_job_if_queued(conn, &job.id).await? {
                return Ok(Some(claimed));
            }
            continue;
        };
        if wiki_service::conversation_has_active_turn_summary(conn, cid).await? {
            continue;
        }
        if let Some(claimed) = wiki_service::claim_job_if_queued(conn, &job.id).await? {
            return Ok(Some(claimed));
        }
    }
    Ok(None)
}

pub(crate) async fn backfill_memory_pipeline(conn: &DatabaseConnection, vault: &Path) {
    for kind in ["compile", "ingest"] {
        let Ok(jobs) = wiki_service::list_jobs_by_kind(conn, kind).await else {
            continue;
        };
        let reason = if kind == "compile" {
            "compile job superseded by wiki_synthesize"
        } else {
            "ingest job superseded by turn_summary"
        };
        for job in jobs {
            if matches!(job.status.as_str(), "queued" | "running" | "failed") {
                let _ = wiki_service::mark_job(
                    conn,
                    &job.id,
                    "cancelled",
                    Some("superseded"),
                    Some(reason),
                )
                .await;
            }
        }
    }
    let Ok(sources) = wiki_service::list_sources_by_kind(conn, "acp-turn").await else {
        return;
    };
    let mut conversation_ids = HashSet::new();
    for source in sources {
        let Some(raw_hash) = source.raw_hash.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(raw_path) = source.raw_path.as_deref().filter(|s| !s.is_empty()) else {
            continue;
        };
        if let Some(cid) = source.conversation_id {
            conversation_ids.insert(cid);
        }
        let rel = crate::wiki::turn_summary::page_rel(&source.id);
        if vault.join(&rel).is_file() {
            continue;
        }
        let key = crate::wiki::turn_summary::dedupe_key(&source.id, raw_hash);
        // Migration only supplies missing jobs. Terminal failures and empty
        // successful summaries remain terminal until an explicit retry/event.
        match wiki_service::find_job_by_dedupe_key(conn, &key).await {
            Ok(None) => {}
            _ => continue,
        }
        let _ = crate::wiki::turn_summary::enqueue_for_source(
            conn,
            &source.vault_id,
            &source.id,
            raw_hash,
            source.conversation_id,
            raw_path,
        )
        .await;
    }
    // Only conversations belonging to frozen ACP sources are historical
    // rollup candidates. Uncaptured local sessions require a new completion
    // event, which exports their transcript before enqueueing.
    for cid in conversation_ids {
        let rel = crate::wiki::session_rollup::page_rel(cid);
        if vault.join(&rel).is_file() {
            continue;
        }
        let key = crate::wiki::session_rollup::dedupe_key(cid);
        match wiki_service::find_job_by_dedupe_key(conn, &key).await {
            Ok(None) => {}
            _ => continue,
        }
        // Reuse Completed/deleted/capture exclusions without reusing the
        // event's permission to replace an existing terminal job.
        crate::wiki::session_rollup::enqueue_on_completed(conn, cid).await;
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
                        let _ =
                            wiki_service::mark_job(conn, &job_id, "succeeded", None, None).await;
                        emit_job(emitter, &job_id, "succeeded");
                    }
                }
            }
            Ok(RecoverStatus::PartialConflict { rel, reason }) => {
                let msg = format!("{rel}: {reason}");
                let _ =
                    wiki_service::mark_job(conn, &job_id, "failed", Some("conflict"), Some(&msg))
                        .await;
                emit_job(emitter, &job_id, "failed");
            }
            Err(e) => {
                tracing::warn!(job_id = %job_id, "[wiki] commit recovery: {e}");
            }
        }
    }

    // 2. Interrupted jobs: turn/session with raw stay summarizable; synthesize
    // without a resume-able manifest fails (never blindly rewrite).
    let running = wiki_service::list_jobs_by_status(conn, "running")
        .await
        .map_err(|e| e.to_string())?;
    for job in running {
        if matches!(
            job.kind.as_str(),
            "turn_summary" | "session_rollup" | "ingest"
        ) {
            let _ = wiki_service::mark_job(conn, &job.id, "queued", None, None).await;
            emit_job(emitter, &job.id, "queued");
            continue;
        }
        if matches!(job.kind.as_str(), "wiki_synthesize" | "compile") {
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
                Some("wiki_synthesize interrupted before a commit manifest was persisted"),
            )
            .await;
            emit_job(emitter, &job.id, "failed");
        }
    }
    Ok(())
}

async fn handle_memory_job(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    state_root: &Path,
    emitter: &EventEmitter,
    kind: &str,
) {
    let (model_id, prompt, skill) = match kind {
        "turn_summary" => (
            settings.turn_summary.model_id.as_deref(),
            settings.turn_summary.prompt.clone(),
            "turn_summary",
        ),
        _ => (
            settings.session_rollup.model_id.as_deref(),
            settings.session_rollup.prompt.clone(),
            "session_rollup",
        ),
    };
    let bound = match llm::bind_wiki_model(conn, model_id).await {
        Ok(b) => b,
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
            return;
        }
    };
    let model_id = bound.model_id.clone();
    let protocol = bound.protocol.as_str().to_string();
    let _ = wiki_service::set_job_model_meta(conn, &job.id, Some(&model_id), Some(&protocol)).await;
    let skill_body = if skill == "turn_summary" {
        include_str!("../../agent-skills/wiki-turn-summary/SKILL.md").to_string()
    } else {
        include_str!("../../agent-skills/wiki-session-rollup/SKILL.md").to_string()
    };
    let extra = prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let llm = ProductionWikiLlm::new(bound, skill_body, extra);
    let result = if kind == "turn_summary" {
        worker::run_turn_summary(conn, job, &llm, vault, state_root).await
    } else {
        worker::run_session_rollup(conn, job, &llm, vault, state_root).await
    };
    match result {
        Ok(_) => {
            let _ = wiki_service::mark_job(conn, &job.id, "succeeded", None, None).await;
            emit_job(emitter, &job.id, "succeeded");
        }
        Err(e) => {
            let retryable = e.retryable();
            let _ = wiki_service::mark_job(
                conn,
                &job.id,
                "failed",
                Some(e.error_code()),
                Some(&e.to_string()),
            )
            .await;
            if retryable && job.attempt < 3 {
                let _ = wiki_service::requeue_after_failure(conn, &job.id, job.attempt).await;
            }
            emit_job(emitter, &job.id, "failed");
        }
    }
}

async fn handle_turn_summary(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    state_root: &Path,
    emitter: &EventEmitter,
) {
    handle_memory_job(
        conn,
        job,
        settings,
        vault,
        state_root,
        emitter,
        "turn_summary",
    )
    .await;
}

async fn handle_session_rollup(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    state_root: &Path,
    emitter: &EventEmitter,
) {
    handle_memory_job(
        conn,
        job,
        settings,
        vault,
        state_root,
        emitter,
        "session_rollup",
    )
    .await;
}

async fn handle_compile(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    settings: &WikiSettings,
    vault: &Path,
    state_root: &Path,
    emitter: &EventEmitter,
) {
    // A cancellation can race with claiming the job. Check the durable DB
    // state before binding a model or starting any model calls.
    if let Ok(current) = wiki_service::get_job_model(conn, &job.id).await {
        if current.status == "cancelled" {
            clear_cancellation(&job.id);
            return;
        }
    }
    if !settings.synthesize.enabled {
        let _ = wiki_service::mark_job(
            conn,
            &job.id,
            "failed",
            Some(llm::BLOCKED_BY_CONFIGURATION),
            Some("synthesize is disabled"),
        )
        .await;
        emit_job(emitter, &job.id, "failed");
        return;
    }
    let bound = match llm::bind_wiki_model(conn, settings.synthesize.model_id.as_deref()).await {
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
    let skill = include_str!("../../agent-skills/wiki-synthesize/SKILL.md").to_string();
    let extra = settings
        .synthesize
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let llm = ProductionWikiLlm::new(bound, skill, extra);
    let cancel = cancellation_token(&job.id);
    let attempt = worker::run_compile_attempt(conn, job, &llm, vault, state_root);
    tokio::pin!(attempt);
    let outcome = tokio::select! {
        result = &mut attempt => Some(result),
        _ = cancel.notified() => None,
    };
    clear_cancellation(&job.id);
    match outcome {
        None => {
            let current = wiki_service::get_job_model(conn, &job.id).await.ok();
            if current.as_ref().map(|j| j.status.as_str()) == Some("failed") {
                emit_job(emitter, &job.id, "failed");
            } else {
                let _ = wiki_service::mark_job(
                    conn,
                    &job.id,
                    "cancelled",
                    Some("cancelled"),
                    Some("wiki_synthesize cancelled before commit"),
                )
                .await;
                emit_job(emitter, &job.id, "cancelled");
            }
        }
        Some(Ok(())) => {
            // Cancellation may have won the DB race just as the attempt
            // completed. Preserve the user's terminal state.
            let cancelled = wiki_service::get_job_model(conn, &job.id)
                .await
                .map(|j| j.status == "cancelled")
                .unwrap_or(false);
            if cancelled {
                emit_job(emitter, &job.id, "cancelled");
            } else {
                let _ = wiki_service::mark_job(conn, &job.id, "succeeded", None, None).await;
                emit_job(emitter, &job.id, "succeeded");
            }
        }
        Some(Err(e)) => {
            let code = e.error_code();
            let retryable = e.retryable();
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

async fn maybe_enqueue_scheduled_compile(
    conn: &DatabaseConnection,
    settings: &WikiSettings,
    vault: &Path,
) {
    if !settings.synthesize.enabled {
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
                let _ =
                    wiki_service::set_vault_next_compile_at(conn, &vault_row.id, Some(next)).await;
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
    // Empty memory notes still enqueue one succeeded catch-up job per date.
    let scheduled_for = now.date_naive().to_string();
    let dedupe = format!("{}:wiki_synthesize:scheduled:{scheduled_for}", vault_row.id);
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
        request_cancel(&job.id);
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

    async fn backfill_fixture() -> (tempfile::TempDir, AppDatabase, String, i32) {
        let dir = tempdir().unwrap();
        vault::initialize_vault(dir.path()).unwrap();
        let db = fresh_in_memory_db().await;
        let folder = crate::db::test_helpers::seed_folder(&db, "/tmp/wiki-backfill").await;
        let mut config = WikiSettings::default();
        config.enabled = true;
        config.vault_path = Some(dir.path().to_string_lossy().into_owned());
        settings::save_settings(&db.conn, &config).await.unwrap();
        let vault_row = wiki_service::ensure_active_vault(&db.conn, &dir.path().to_string_lossy())
            .await
            .unwrap();
        (dir, db, vault_row.id, folder)
    }

    async fn seed_completed_for_backfill(db: &AppDatabase, folder: i32) -> i32 {
        use crate::db::entities::conversation;
        use sea_orm::{ActiveModelTrait, EntityTrait, Set};

        let cid = crate::db::test_helpers::seed_conversation(
            db,
            folder,
            crate::models::AgentType::CodegAgent,
        )
        .await;
        let mut row: conversation::ActiveModel = conversation::Entity::find_by_id(cid)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        // Seed historical state without emitting a new Completed event.
        row.status = Set(conversation::ConversationStatus::Completed);
        row.update(&db.conn).await.unwrap();
        cid
    }

    async fn seed_acp_for_backfill(
        conn: &DatabaseConnection,
        vault: &Path,
        vault_id: &str,
        cid: i32,
    ) -> wiki_job::Model {
        let inserted = wiki_service::insert_acp_source_and_ingest_job(
            conn,
            wiki_service::NewAcpSource {
                vault_id: vault_id.into(),
                run_id: format!("backfill-{cid}"),
                conversation_id: Some(cid),
                folder_id: None,
                root_folder_id: None,
                agent_type: None,
                model: None,
                mode: None,
                captured_at: Utc::now(),
                occurred_at: Some(Utc::now()),
                truncated: false,
                redacted: false,
                source_title: None,
            },
        )
        .await
        .unwrap();
        let sid = inserted.source.id;
        let rel = format!("raw/sessions/{sid}.md");
        fs::create_dir_all(vault.join("raw/sessions")).unwrap();
        fs::write(vault.join(&rel), "Completed work").unwrap();
        wiki_service::mark_source_raw(conn, &sid, &rel, "raw-hash", "ready")
            .await
            .unwrap();
        let job = inserted.job.unwrap();
        let key = crate::wiki::turn_summary::dedupe_key(&sid, "raw-hash");
        wiki_service::set_job_dedupe_and_manifest(
            conn,
            &job.id,
            &key,
            &serde_json::json!({"conversation_id": cid}).to_string(),
        )
        .await
        .unwrap();
        wiki_service::get_job_model(conn, &job.id).await.unwrap()
    }

    async fn assert_backfill_preserves_terminal(kind: &str) {
        use sea_orm::{ActiveModelTrait, Set};

        for (status, code, attempt) in [
            ("failed", Some("blocked-by-configuration"), 1),
            ("failed", Some("worker_failed"), 3),
            ("succeeded", None, 1),
            ("cancelled", None, 1),
        ] {
            let (dir, db, vault_id, folder) = backfill_fixture().await;
            let cid = seed_completed_for_backfill(&db, folder).await;
            let turn = seed_acp_for_backfill(&db.conn, dir.path(), &vault_id, cid).await;
            let session =
                crate::wiki::session_rollup::enqueue_session_job(&db.conn, &vault_id, cid)
                    .await
                    .unwrap();
            let job = if kind == "turn_summary" {
                turn
            } else {
                session
            };
            let id = job.id.clone();
            let mut row: wiki_job::ActiveModel = job.into();
            row.attempt = Set(attempt);
            row.update(&db.conn).await.unwrap();
            wiki_service::mark_job(&db.conn, &id, status, code, None)
                .await
                .unwrap();
            for _ in 0..2 {
                backfill_memory_pipeline(&db.conn, dir.path()).await;
            }
            let after = wiki_service::get_job_model(&db.conn, &id).await.unwrap();
            assert_eq!(after.status, status, "{kind} terminal state changed");
            assert_eq!(after.attempt, attempt, "{kind} was retried by backfill");
            assert_eq!(after.error_code.as_deref(), code);
            assert_eq!(
                wiki_service::list_jobs(&db.conn, 20, 0, None)
                    .await
                    .unwrap()
                    .len(),
                2,
                "backfill created a replacement for a terminal {kind} job"
            );
        }
    }

    #[tokio::test]
    async fn backfill_preserves_terminal_turn_jobs() {
        assert_backfill_preserves_terminal("turn_summary").await;
    }

    #[tokio::test]
    async fn backfill_preserves_terminal_session_jobs() {
        assert_backfill_preserves_terminal("session_rollup").await;
    }

    #[tokio::test]
    async fn backfill_only_rolls_up_eligible_completed_acp_conversations() {
        use crate::db::entities::conversation::{self, ConversationKind, ConversationStatus};
        use sea_orm::{ActiveModelTrait, EntityTrait, Set};

        let (dir, db, vault_id, folder) = backfill_fixture().await;
        let allowed = seed_completed_for_backfill(&db, folder).await;
        seed_acp_for_backfill(&db.conn, dir.path(), &vault_id, allowed).await;
        // A historical local conversation has no frozen ACP raw and is outside
        // the migration. It must not become a permanently failing rollup job.
        seed_completed_for_backfill(&db, folder).await;
        let excluded_folder =
            crate::db::test_helpers::seed_folder(&db, "/tmp/wiki-backfill-excluded").await;
        for case in ["pending", "delegate", "loop", "agent", "folder", "deleted"] {
            let cid = seed_completed_for_backfill(
                &db,
                if case == "folder" {
                    excluded_folder
                } else {
                    folder
                },
            )
            .await;
            seed_acp_for_backfill(&db.conn, dir.path(), &vault_id, cid).await;
            let mut row: conversation::ActiveModel = conversation::Entity::find_by_id(cid)
                .one(&db.conn)
                .await
                .unwrap()
                .unwrap()
                .into();
            match case {
                "pending" => row.status = Set(ConversationStatus::PendingReview),
                "delegate" => {
                    row.kind = Set(ConversationKind::Delegate);
                    row.parent_id = Set(Some(allowed));
                }
                "loop" => row.kind = Set(ConversationKind::Loop),
                "agent" => row.agent_type = Set("claude_code".into()),
                "deleted" => row.deleted_at = Set(Some(Utc::now())),
                _ => {}
            }
            row.update(&db.conn).await.unwrap();
        }
        let mut config = settings::load_settings(&db.conn).await.unwrap();
        config.capture.exclude_agent_types = vec!["claude_code".into()];
        config.capture.exclude_folder_ids = vec![excluded_folder];
        settings::save_settings(&db.conn, &config).await.unwrap();

        for _ in 0..2 {
            backfill_memory_pipeline(&db.conn, dir.path()).await;
        }
        let jobs = wiki_service::list_jobs_by_kind(&db.conn, "session_rollup")
            .await
            .unwrap();
        let actual: Vec<_> = jobs
            .iter()
            .filter_map(crate::wiki::session_rollup::conversation_id_from_job)
            .collect();
        assert_eq!(actual, vec![allowed]);
    }

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

    #[tokio::test]
    async fn session_rollup_is_not_claimed_while_turn_summary_queued() {
        use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
        use crate::db::test_helpers::fresh_in_memory_db;
        use sea_orm::{ActiveModelTrait, Set};

        let db = fresh_in_memory_db().await;
        let now = Utc::now();
        wiki_vault::ActiveModel {
            id: Set("v1".into()),
            canonical_path: Set("/tmp/wiki-engine".into()),
            config_revision: Set(0),
            next_compile_at: Set(None),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        wiki_source::ActiveModel {
            id: Set("src-1".into()),
            source_group_id: Set("src-1".into()),
            vault_id: Set("v1".into()),
            source_kind: Set("acp-turn".into()),
            source_seq: Set(1),
            run_id: Set(Some("run".into())),
            original_hash: Set(None),
            raw_path: Set(Some("raw/sessions/src-1.md".into())),
            raw_hash: Set(Some("h".into())),
            extractor_version: Set(None),
            coverage_status: Set(None),
            eligibility: Set("ready".into()),
            material_role: Set(None),
            personal_role: Set(None),
            annotation_revision: Set(0),
            conversation_id: Set(Some(7)),
            folder_id: Set(None),
            root_folder_id: Set(None),
            agent_type: Set(None),
            model: Set(None),
            mode: Set(None),
            captured_at: Set(None),
            occurred_at: Set(None),
            truncated: Set(false),
            redacted: Set(false),
            request_id: Set(None),
            original_filename: Set(None),
            format: Set(None),
            source_title: Set(None),
            source_url: Set(None),
            author: Set(None),
            project_ids: Set(None),
            area_ids: Set(None),
            warnings: Set(None),
            page_count: Set(None),
            previous_source_id: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        wiki_job::ActiveModel {
            id: Set("turn-q".into()),
            vault_id: Set("v1".into()),
            source_id: Set(Some("src-1".into())),
            kind: Set("turn_summary".into()),
            status: Set("queued".into()),
            dedupe_key: Set(Some("turn_summary:src-1:h".into())),
            input_manifest: Set(Some(r#"{"conversation_id":7}"#.into())),
            config_version: Set(None),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            started_at: Set(None),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        wiki_job::ActiveModel {
            id: Set("sess-q".into()),
            vault_id: Set("v1".into()),
            source_id: Set(None),
            kind: Set("session_rollup".into()),
            status: Set("queued".into()),
            dedupe_key: Set(Some("session_rollup:7".into())),
            input_manifest: Set(Some(r#"{"conversation_id":7}"#.into())),
            config_version: Set(None),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            started_at: Set(None),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let claimed = claim_next_session_rollup(&db.conn).await.unwrap();
        assert!(
            claimed.is_none(),
            "must skip session while turn_summary queued"
        );
        wiki_service::mark_job(&db.conn, "turn-q", "succeeded", None, None)
            .await
            .unwrap();
        let claimed = claim_next_session_rollup(&db.conn).await.unwrap();
        assert_eq!(claimed.unwrap().id, "sess-q");
    }

    #[tokio::test]
    async fn backfill_enqueues_turn_summary_and_cancels_compile() {
        use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
        use crate::db::test_helpers::fresh_in_memory_db;
        use crate::wiki::vault;
        use sea_orm::{ActiveModelTrait, Set};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let vault_path = dir.path().join("vault");
        vault::initialize_vault(&vault_path).unwrap();
        let db = fresh_in_memory_db().await;
        let now = Utc::now();
        wiki_vault::ActiveModel {
            id: Set("v1".into()),
            canonical_path: Set(vault_path.to_string_lossy().into_owned()),
            config_revision: Set(0),
            next_compile_at: Set(None),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let source_id = "backfill-src";
        let rel = format!("raw/sessions/{source_id}.md");
        fs::create_dir_all(vault_path.join("raw/sessions")).unwrap();
        fs::write(vault_path.join(&rel), "raw body").unwrap();
        wiki_source::ActiveModel {
            id: Set(source_id.into()),
            source_group_id: Set(source_id.into()),
            vault_id: Set("v1".into()),
            source_kind: Set("acp-turn".into()),
            source_seq: Set(1),
            run_id: Set(Some("run-b".into())),
            original_hash: Set(None),
            raw_path: Set(Some(rel)),
            raw_hash: Set(Some("hash-b".into())),
            extractor_version: Set(None),
            coverage_status: Set(None),
            eligibility: Set("ready".into()),
            material_role: Set(None),
            personal_role: Set(None),
            annotation_revision: Set(0),
            conversation_id: Set(None),
            folder_id: Set(None),
            root_folder_id: Set(None),
            agent_type: Set(None),
            model: Set(None),
            mode: Set(None),
            captured_at: Set(None),
            occurred_at: Set(None),
            truncated: Set(false),
            redacted: Set(false),
            request_id: Set(None),
            original_filename: Set(None),
            format: Set(None),
            source_title: Set(None),
            source_url: Set(None),
            author: Set(None),
            project_ids: Set(None),
            area_ids: Set(None),
            warnings: Set(None),
            page_count: Set(None),
            previous_source_id: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        wiki_job::ActiveModel {
            id: Set("old-compile".into()),
            vault_id: Set("v1".into()),
            source_id: Set(None),
            kind: Set("compile".into()),
            status: Set("failed".into()),
            dedupe_key: Set(Some("old".into())),
            input_manifest: Set(None),
            config_version: Set(None),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(Some("too_many_candidates".into())),
            error_message: Set(Some("segment s16 returned more than 5 candidates".into())),
            output_manifest: Set(None),
            started_at: Set(None),
            finished_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        wiki_job::ActiveModel {
            id: Set("old-ingest".into()),
            vault_id: Set("v1".into()),
            source_id: Set(Some(source_id.into())),
            kind: Set("ingest".into()),
            status: Set("queued".into()),
            dedupe_key: Set(Some("old-ingest".into())),
            input_manifest: Set(None),
            config_version: Set(None),
            model_id: Set(None),
            protocol: Set(None),
            attempt: Set(1),
            error_code: Set(None),
            error_message: Set(None),
            output_manifest: Set(None),
            started_at: Set(None),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        backfill_memory_pipeline(&db.conn, &vault_path).await;
        let compile = wiki_service::get_job(&db.conn, "old-compile")
            .await
            .unwrap();
        assert_eq!(compile.status, "cancelled");
        assert_eq!(compile.error_code.as_deref(), Some("superseded"));
        let ingest = wiki_service::get_job(&db.conn, "old-ingest").await.unwrap();
        assert_eq!(ingest.status, "cancelled");
        assert_eq!(ingest.error_code.as_deref(), Some("superseded"));
        let jobs = wiki_service::list_jobs(&db.conn, 20, 0, None)
            .await
            .unwrap();
        assert!(jobs
            .iter()
            .any(|j| j.kind == "turn_summary" && j.status == "queued"));
    }

    #[tokio::test]
    async fn cancellation_signal_is_retained_until_waiter_is_ready() {
        let id = "cancel-retained-test";
        request_cancel(id);
        let token = cancellation_token(id);
        tokio::time::timeout(Duration::from_millis(100), token.notified())
            .await
            .expect("cancel signal should wake a waiter");
        clear_cancellation(id);
    }
}

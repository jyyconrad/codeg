//! 调度个人Wiki后台任务，处理唤醒、定时归纳、取消和启动恢复。
//! 桌面与服务器共用引擎；数据库负责领取当前Wiki的任务，worker负责调用
//! 具体整理业务，commit负责恢复已开始的文件提交，事件用于刷新界面状态。
//! 进程锁避免重复消费，Wiki切换锁只覆盖领取阶段，不跨模型执行持有。

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use tokio::sync::Notify;
#[cfg(test)]
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

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
static CANCELLATIONS: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();

fn wake_signal() -> Arc<Notify> {
    WAKE.get_or_init(|| Arc::new(Notify::new())).clone()
}

pub fn notify_jobs() {
    wake_signal().notify_one();
}

fn cancellation_map() -> &'static Mutex<HashMap<String, CancellationToken>> {
    CANCELLATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cancellation_token(job_id: &str) -> CancellationToken {
    let mut map = cancellation_map()
        .lock()
        .expect("wiki cancellation map poisoned");
    map.entry(job_id.to_string()).or_default().clone()
}

/// Signal an in-flight worker to stop before it can continue to a commit.
pub fn request_cancel(job_id: &str) {
    let token = cancellation_token(job_id);
    token.cancel();
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
    let state_root = resolve_state_root();
    let Some(engine) = try_build_engine(db, emitter, state_root) else {
        return;
    };
    #[cfg(feature = "tauri-runtime")]
    {
        tauri::async_runtime::spawn(engine.run());
    }
    #[cfg(not(feature = "tauri-runtime"))]
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        h.spawn(engine.run());
    } else {
        tracing::warn!("[wiki] no tokio runtime to spawn the engine");
    }
}

#[cfg(test)]
fn spawn_with_roots(
    db: AppDatabase,
    emitter: EventEmitter,
    state_root: PathBuf,
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
        // 状态目录与用户选中的Wiki目录分离；启动时在引擎持有锁的目录恢复一次即可。
        if let Err(e) = async {
            let _transition = crate::wiki::lifecycle::lock().await;
            crate::wiki::relocate::relocate_legacy_app_data_wiki(&self.db.conn).await
        }
        .await
        {
            tracing::warn!("[wiki] relocate error: {e}");
        }
        if let Err(e) = crate::wiki::source::recover_pending(&self.db.conn).await {
            tracing::warn!("[wiki] pending ACP recovery error: {e}");
        }
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
        // Settings saves hold the same short lock while checking active jobs.
        // Claim under it, then release before binding or running any model.
        let transition = crate::wiki::lifecycle::lock().await;
        crate::wiki::relocate::relocate_legacy_app_data_wiki(conn)
            .await
            .map_err(|e| e.to_string())?;
        let settings = settings::load_settings(conn)
            .await
            .map_err(|e| e.to_string())?;
        if !settings.enabled {
            return Ok(());
        }
        let configured = resolve_vault_path(settings.vault_path.as_deref());
        vault::initialize_vault(&configured).map_err(|e| e.to_string())?;
        let vault = configured.canonicalize().map_err(|e| e.to_string())?;
        let state_root = resolve_state_root();
        vault::initialize_state_root(&state_root).map_err(|e| e.to_string())?;
        let active = wiki_service::ensure_active_vault(conn, &vault.to_string_lossy())
            .await
            .map_err(|e| e.to_string())?;
        expire_overdue_jobs(conn, &self.emitter).await;
        let mut job = wiki_service::claim_next_queued_job(conn, &active.id, "turn_summary")
            .await
            .map_err(|e| e.to_string())?;
        if job.is_none() {
            job = claim_next_session_rollup(conn, &active.id)
                .await
                .map_err(|e| e.to_string())?;
        }
        if job.is_none() {
            // The stage toggle controls automatic scheduling only. Manual jobs
            // remain runnable while Wiki itself is enabled.
            maybe_enqueue_scheduled_compile(conn, &settings, &vault).await;
            job = wiki_service::claim_next_queued_job(conn, &active.id, "wiki_synthesize")
                .await
                .map_err(|e| e.to_string())?;
        }
        drop(transition);
        if let Some(job) = job {
            emit_job(&self.emitter, &job.id, "running");
            match job.kind.as_str() {
                "turn_summary" => {
                    handle_turn_summary(conn, &job, &settings, &vault, &state_root, &self.emitter)
                        .await
                }
                "session_rollup" => {
                    handle_session_rollup(conn, &job, &settings, &vault, &state_root, &self.emitter)
                        .await
                }
                "wiki_synthesize" => {
                    handle_compile(conn, &job, &settings, &vault, &state_root, &self.emitter).await
                }
                _ => unreachable!("only known kinds are claimed"),
            }
            notify_jobs();
        }
        Ok(())
    }
}

async fn claim_next_session_rollup(
    conn: &DatabaseConnection,
    vault_id: &str,
) -> Result<Option<wiki_job::Model>, crate::db::error::DbError> {
    let queued = wiki_service::list_queued_jobs_by_kind(conn, vault_id, "session_rollup").await?;
    for job in queued {
        let Some(cid) = crate::wiki::session_rollup::conversation_id_from_job(&job) else {
            if let Some(claimed) =
                wiki_service::claim_job_if_queued(conn, vault_id, &job.id).await?
            {
                return Ok(Some(claimed));
            }
            continue;
        };
        if wiki_service::conversation_has_active_turn_summary(conn, vault_id, cid).await? {
            continue;
        }
        if let Some(claimed) = wiki_service::claim_job_if_queued(conn, vault_id, &job.id).await? {
            return Ok(Some(claimed));
        }
    }
    Ok(None)
}

async fn recover_on_start(
    conn: &DatabaseConnection,
    state_root: &Path,
    emitter: &EventEmitter,
) -> Result<(), String> {
    for (id, outcome) in compile::recover_batches_outcomes(conn, state_root).await {
        let job = match wiki_service::get_job_model(conn, &id).await {
            Ok(job) => job,
            Err(error) => {
                tracing::warn!(job_id=%id,%error,"[wiki] recovery has no matching job");
                continue;
            }
        };
        match outcome {
            Ok(result) if job.status == "running" && result.remaining_inputs.is_empty() => {
                wiki_service::mark_job(conn, &id, "succeeded", None, None)
                    .await
                    .map_err(|e| e.to_string())?;
                emit_job(emitter, &id, "succeeded");
            }
            Ok(_) => {}
            Err(error) => {
                wiki_service::mark_job(
                    conn,
                    &id,
                    "failed",
                    Some(error.error_code()),
                    Some(&error.to_string()),
                )
                .await
                .map_err(|e| e.to_string())?;
                emit_job(emitter, &id, "failed");
            }
        }
    }
    for (job_id, status) in commit::recover_all(state_root) {
        match status {
            Ok(RecoverStatus::Complete) => {
                let job = wiki_service::get_job_model(conn, &job_id)
                    .await
                    .map_err(|e| e.to_string())?;
                if !matches!(job.kind.as_str(), "turn_summary" | "session_rollup") {
                    continue;
                }
                let manifest = commit::load_manifest(state_root, &job_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("missing recovered manifest")?;
                let mut required = Vec::new();
                let mut outputs = Vec::new();
                for file in &manifest.files {
                    let actual = fs::read_to_string(Path::new(&manifest.vault).join(&file.rel))
                        .map_err(|e| e.to_string())?;
                    if crate::wiki::raw::content_hash(&actual) != file.after_hash {
                        return Err("recovered output hash mismatch".into());
                    }
                    let document = crate::wiki::read_model::document::Document::parse(&actual);
                    let mut source_ids = document.strings("source_ids");
                    source_ids.extend(document.strings("codeg_source_id"));
                    source_ids.extend(job.source_id.iter().cloned());
                    source_ids.sort();
                    source_ids.dedup();
                    for locator in document.strings("sources") {
                        let rel = locator
                            .trim_start_matches("[[")
                            .trim_end_matches("]]")
                            .split('|')
                            .next()
                            .unwrap_or(&locator);
                        let rel = if rel.ends_with(".md") {
                            rel.into()
                        } else {
                            format!("{rel}.md")
                        };
                        if !required
                            .iter()
                            .any(|input: &crate::wiki::result::WikiInput| input.rel == rel)
                        {
                            required.push(crate::wiki::result::WikiInput {
                                rel,
                                content_hash: String::new(),

                                source_ids: source_ids.clone(),
                            });
                        }
                    }
                    outputs.push(crate::wiki::result::WikiOutput {
                        note_id: compile::yaml_string(&actual, "codeg_note_id")
                            .ok_or("recovered note has no identity")?,
                        path: file.rel.clone(),
                        title: compile::yaml_string(&actual, "title").unwrap_or_default(),
                        page_type: file.page_type.clone(),
                        content_hash: file.after_hash.clone(),
                    });
                }
                let mut result = crate::wiki::result::JobOutputManifest::generated(
                    outputs,
                    &required,
                    Vec::new(),
                );
                if job.status == "cancelled" {
                    result.outcome = "partial".into();
                }
                wiki_service::set_job_output_manifest(
                    conn,
                    &job_id,
                    &serde_json::to_string(&result).map_err(|e| e.to_string())?,
                )
                .await
                .map_err(|e| e.to_string())?;
                crate::wiki::turn_summary::register_memory_contributions(conn, &job, &result)
                    .await
                    .map_err(|e| e.to_string())?;
                commit::mark_finalized(state_root, &job_id).map_err(|e| e.to_string())?;
                if job.status == "running" {
                    wiki_service::mark_job(conn, &job_id, "succeeded", None, None)
                        .await
                        .map_err(|e| e.to_string())?;
                    emit_job(emitter, &job_id, "succeeded");
                }
            }
            Ok(RecoverStatus::PartialConflict { rel, reason }) => {
                wiki_service::mark_job(
                    conn,
                    &job_id,
                    "failed",
                    Some("write_conflict"),
                    Some(&format!("{rel}: {reason}")),
                )
                .await
                .map_err(|e| e.to_string())?;
                emit_job(emitter, &job_id, "failed");
            }
            Err(e) => tracing::warn!(job_id = %job_id, "[wiki] commit recovery: {e}"),
        }
    }

    // 2. Interrupted jobs: turn/session with raw stay summarizable; synthesize
    // without a resume-able manifest fails (never blindly rewrite).
    let running = wiki_service::list_jobs_by_status(conn, "running")
        .await
        .map_err(|e| e.to_string())?;
    for job in running {
        if job
            .output_manifest
            .as_deref()
            .and_then(|raw| {
                serde_json::from_str::<crate::wiki::result::JobOutputManifest>(raw).ok()
            })
            .is_some_and(|r| {
                r.version == 1
                    && matches!(r.outcome.as_str(), "no_content" | "no_new_input")
                    && r.remaining_inputs.is_empty()
            })
        {
            wiki_service::mark_job(conn, &job.id, "succeeded", None, None)
                .await
                .map_err(|e| e.to_string())?;
            emit_job(emitter, &job.id, "succeeded");
            continue;
        }
        if matches!(job.kind.as_str(), "turn_summary" | "session_rollup") {
            wiki_service::mark_job(
                conn,
                &job.id,
                "failed",
                Some("interrupted"),
                Some("attempt interrupted before durable result"),
            )
            .await
            .map_err(|e| e.to_string())?;
            wiki_service::requeue_after_failure(conn, &job.id, job.attempt)
                .await
                .map_err(|e| e.to_string())?;
            let current = wiki_service::get_job_model(conn, &job.id)
                .await
                .map_err(|e| e.to_string())?;
            emit_job(emitter, &job.id, &current.status);
            continue;
        }
        if matches!(job.kind.as_str(), "wiki_synthesize") {
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
    let (provider_id, model_id, prompt, skill) = match kind {
        "turn_summary" => (
            settings.turn_summary.provider_id,
            settings.turn_summary.model_id.as_deref(),
            settings.turn_summary.prompt.clone(),
            "turn_summary",
        ),
        _ => (
            settings.session_rollup.provider_id,
            settings.session_rollup.model_id.as_deref(),
            settings.session_rollup.prompt.clone(),
            "session_rollup",
        ),
    };
    let bound = match llm::bind_wiki_model(conn, provider_id, model_id).await {
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
    let cancel = cancellation_token(&job.id);
    let llm = ProductionWikiLlm::new(bound, skill_body, extra).with_cancellation(cancel.clone());
    let attempt = async {
        worker::ensure_commit_allowed(conn, &job.id, &llm).await?;
        if kind == "turn_summary" {
            worker::run_turn_summary(conn, job, &llm, vault, state_root).await
        } else {
            worker::run_session_rollup(conn, job, &llm, vault, state_root).await
        }
    };
    let result = run_with_deadline(attempt, &cancel).await;
    finish_attempt(conn, job, result, emitter).await;
    clear_cancellation(&job.id);
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
    if !settings.enabled {
        let _ = wiki_service::mark_job(
            conn,
            &job.id,
            "failed",
            Some(llm::BLOCKED_BY_CONFIGURATION),
            Some("wiki is disabled"),
        )
        .await;
        emit_job(emitter, &job.id, "failed");
        return;
    }
    let bound = match llm::bind_wiki_model(
        conn,
        settings.synthesize.provider_id,
        settings.synthesize.model_id.as_deref(),
    )
    .await
    {
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
    let cancel = cancellation_token(&job.id);
    let llm = ProductionWikiLlm::new(bound, skill, extra).with_cancellation(cancel.clone());
    let attempt = worker::run_compile_attempt(conn, job, &llm, vault, state_root);
    let outcome = run_with_deadline(attempt, &cancel).await;
    finish_attempt(conn, job, outcome, emitter).await;
    clear_cancellation(&job.id);
}

async fn run_with_deadline<F>(
    attempt: F,
    cancel: &CancellationToken,
) -> Result<(), worker::WorkerError>
where
    F: std::future::Future<Output = Result<(), worker::WorkerError>>,
{
    tokio::pin!(attempt);
    let stopped = tokio::select! {
        biased;
        _ = cancel.cancelled() => llm::WikiLlmError::Cancelled,
        result = &mut attempt => return result,
        _ = tokio::time::sleep(Duration::from_secs(JOB_BUDGET_MINUTES as u64 * 60)) => llm::WikiLlmError::DeadlineExceeded,
    };
    cancel.cancel();
    // Model/tools observe the token immediately. Once the synchronous file
    // commit has started, let its DB registration finish before reporting stop.
    // Dropping this future here would leave applied files without their result.
    let _ = attempt.await;
    Err(stopped.into())
}

async fn finish_attempt(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    result: Result<(), worker::WorkerError>,
    emitter: &EventEmitter,
) {
    let current = match wiki_service::get_job_model(conn, &job.id).await {
        Ok(row) => row,
        Err(error) => {
            tracing::error!(job_id=%job.id, %error, "[wiki] cannot load job terminal state");
            return;
        }
    };
    if current.status == "cancelled" {
        preserve_partial_result(conn, &current).await;
        emit_job(emitter, &job.id, "cancelled");
        return;
    }
    let (status, code, message, retry) = match result {
        Ok(()) => ("succeeded", None, None, false),
        Err(error) => {
            let status = if error.error_code() == "cancelled" {
                "cancelled"
            } else {
                "failed"
            };
            (
                status,
                Some(error.error_code()),
                Some(error.to_string()),
                error.retryable() && job.attempt < 3,
            )
        }
    };
    if status != "succeeded" {
        preserve_partial_result(conn, &current).await;
    }
    if let Err(error) =
        wiki_service::mark_job(conn, &job.id, status, code, message.as_deref()).await
    {
        tracing::error!(job_id=%job.id, %error, "[wiki] cannot persist job outcome");
        return;
    }
    if retry {
        if let Err(error) = wiki_service::requeue_after_failure(conn, &job.id, job.attempt).await {
            tracing::error!(job_id=%job.id, %error, "[wiki] cannot schedule retry");
        }
    }
    // Cancel may race the terminal update; emit the durable state, never a
    // guessed success derived from a stale pre-update row.
    match wiki_service::get_job_model(conn, &job.id).await {
        Ok(saved) => emit_job(emitter, &job.id, &saved.status),
        Err(error) => {
            tracing::error!(job_id=%job.id, %error, "[wiki] cannot confirm saved job state")
        }
    }
}

async fn preserve_partial_result(conn: &DatabaseConnection, job: &wiki_job::Model) {
    let Some(mut result) = job
        .output_manifest
        .as_deref()
        .and_then(|raw| serde_json::from_str::<crate::wiki::result::JobOutputManifest>(raw).ok())
    else {
        return;
    };
    if !result.outputs.is_empty() {
        result.outcome = "partial".into();
        if let Err(error) = wiki_service::set_job_output_manifest(
            conn,
            &job.id,
            &serde_json::to_string(&result).unwrap_or_default(),
        )
        .await
        {
            tracing::error!(job_id=%job.id, %error, "[wiki] cannot persist partial result");
        }
    }
}

async fn maybe_enqueue_scheduled_compile(
    conn: &DatabaseConnection,
    settings: &WikiSettings,
    vault: &Path,
) {
    if !settings.enabled || !settings.synthesize.enabled {
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
    let Ok(manifest) = compile::list_pending_inputs(conn, &vault_row.id, vault).await else {
        return;
    };
    // Empty memory notes still enqueue one succeeded catch-up job per date.
    let scheduled_for = now.date_naive().to_string();
    let dedupe = format!("{}:wiki_synthesize:scheduled:{scheduled_for}", vault_row.id);
    let json = serde_json::to_string(&manifest).unwrap_or_else(|_| "{}".into());
    if wiki_service::find_job_by_dedupe_key(conn, &vault_row.id, &dedupe)
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
            Some("deadline_exceeded"),
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

    #[tokio::test]
    async fn second_spawn_is_noop() {
        let dir = tempdir().unwrap();
        let state = dir.path().join("wiki-state");
        fs::create_dir_all(&state).unwrap();
        let db1 = fresh_in_memory_db().await;
        let db2 = AppDatabase {
            conn: db1.conn.clone(),
        };
        let h1 = spawn_with_roots(db1, EventEmitter::Noop, state.clone());
        assert!(h1.is_some(), "first spawn must take the engine lock");
        let h2 = spawn_with_roots(db2, EventEmitter::Noop, state);
        assert!(h2.is_none(), "second spawn must be a no-op");
        h1.unwrap().abort();
    }

    #[tokio::test]
    async fn automatic_synthesis_disabled_does_not_enqueue_due_schedule() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        vault::initialize_vault(dir.path()).unwrap();
        let settings = WikiSettings {
            enabled: true,
            vault_path: Some(dir.path().to_string_lossy().into_owned()),
            synthesize: settings::WikiSynthesizeSettings {
                enabled: false,
                ..settings::WikiSynthesizeSettings::default()
            },
            ..WikiSettings::default()
        };
        let active = wiki_service::ensure_active_vault(&db.conn, &dir.path().to_string_lossy())
            .await
            .unwrap();
        wiki_service::set_vault_next_compile_at(
            &db.conn,
            &active.id,
            Some(Utc::now() - chrono::Duration::minutes(1)),
        )
        .await
        .unwrap();
        maybe_enqueue_scheduled_compile(&db.conn, &settings, dir.path()).await;
        assert!(wiki_service::list_jobs_by_kind(&db.conn, "wiki_synthesize")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn manually_queued_job_is_claimed_with_automatic_synthesis_disabled() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        vault::initialize_vault(&vault).unwrap();
        let settings = WikiSettings {
            enabled: true,
            vault_path: Some(vault.to_string_lossy().into_owned()),
            synthesize: settings::WikiSynthesizeSettings {
                enabled: false,
                ..settings::WikiSynthesizeSettings::default()
            },
            ..WikiSettings::default()
        };
        settings::save_settings(&db.conn, &settings).await.unwrap();
        let active = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
            .await
            .unwrap();
        let job = wiki_service::insert_compile_job(&db.conn, &active.id, "manual-job", None)
            .await
            .unwrap();
        let engine = try_build_engine(db, EventEmitter::Noop, dir.path().join("state")).unwrap();
        engine.tick_once().await.unwrap();
        let saved = wiki_service::get_job_model(&engine.db.conn, &job.id)
            .await
            .unwrap();
        assert_eq!(
            saved.status, "failed",
            "manual queue must reach model binding even with automatic scheduling off"
        );
        assert_eq!(saved.error_code.as_deref(), Some("model_unavailable"));
        assert!(!saved
            .error_message
            .unwrap_or_default()
            .contains("synthesize is disabled"));
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
            next_attempt_at: Set(None),
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
            next_attempt_at: Set(None),
            started_at: Set(None),
            finished_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let claimed = claim_next_session_rollup(&db.conn, "v1").await.unwrap();
        assert!(
            claimed.is_none(),
            "must skip session while turn_summary queued"
        );
        wiki_service::mark_job(&db.conn, "turn-q", "succeeded", None, None)
            .await
            .unwrap();
        let claimed = claim_next_session_rollup(&db.conn, "v1").await.unwrap();
        assert_eq!(claimed.unwrap().id, "sess-q");
    }

    #[tokio::test(start_paused = true)]
    async fn attempt_deadline_cancels_inflight_work_without_waiting_for_a_tick() {
        let cancel = CancellationToken::new();
        let work_cancel = cancel.clone();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = finished.clone();
        let result = run_with_deadline(
            async move {
                work_cancel.cancelled().await;
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            &cancel,
        )
        .await;
        assert!(
            finished.load(std::sync::atomic::Ordering::SeqCst),
            "stop must await the protected work's cleanup"
        );
        assert_eq!(result.unwrap_err().error_code(), "deadline_exceeded");
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn cancellation_signal_is_retained_until_waiter_is_ready() {
        let id = "cancel-retained-test";
        request_cancel(id);
        let token = cancellation_token(id);
        tokio::time::timeout(Duration::from_millis(100), token.cancelled())
            .await
            .expect("cancel signal should wake a waiter");
        clear_cancellation(id);
    }
}

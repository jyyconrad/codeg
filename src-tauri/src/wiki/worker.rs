//! 衔接Wiki任务调度与轮次、对话、综合整理三个业务入口。
//! engine传入已领取任务和模型，本模块统一错误分类、提交前取消检查，
//! 并在成功后刷新目录索引；目录刷新失败仅告警，不回滚已保存笔记。

use std::path::Path;

use sea_orm::DatabaseConnection;

use crate::db::entities::wiki_job;
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::compile::{self, CompileError};
use crate::wiki::llm::{WikiLlm, WikiLlmError};
use crate::wiki::session_rollup;
use crate::wiki::turn_summary;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Llm(#[from] WikiLlmError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl WorkerError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Failed(_) => "worker_failed",
            Self::Compile(e) => e.error_code(),
            Self::Llm(e) => e.error_code(),
            Self::Db(_) => "database",
        }
    }

    pub fn retryable(&self) -> bool {
        match self {
            Self::Failed(_) | Self::Db(_) => true,
            Self::Compile(e) => e.retryable(),
            Self::Llm(e) => e.retryable(),
        }
    }
}

pub async fn run_turn_summary(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    turn_summary::run_turn_summary_job(conn, job, llm, vault, state_root).await?;
    refresh_library(conn, job, vault).await;
    Ok(())
}

pub async fn run_session_rollup(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    session_rollup::run_session_rollup_job(conn, job, llm, vault, state_root).await?;
    refresh_library(conn, job, vault).await;
    Ok(())
}

pub async fn run_compile_attempt(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<(), WorkerError> {
    compile::run_compile_job(conn, job, llm, vault, state_root).await?;
    refresh_library(conn, job, vault).await;
    Ok(())
}

async fn refresh_library(conn: &DatabaseConnection, job: &wiki_job::Model, vault: &Path) {
    match crate::wiki::library::refresh_at(conn, &job.vault_id, vault).await {
        Ok(library) => {
            for warning in library.warnings {
                tracing::warn!(job_id = %job.id, %warning, "[wiki] navigation refresh");
            }
        }
        Err(error) => tracing::warn!(job_id = %job.id, %error, "[wiki] navigation refresh failed"),
    }
}

/// 模型返回后仍可能收到取消或超时，因此进入受保护的文件提交前再检查一次。
pub async fn ensure_commit_allowed(
    conn: &DatabaseConnection,
    job_id: &str,
    llm: &dyn WikiLlm,
) -> Result<(), WorkerError> {
    llm.check_cancelled()?;
    let job = wiki_service::get_job_model(conn, job_id).await?;
    if job.status == "cancelled" {
        return Err(WikiLlmError::Cancelled.into());
    }
    if job.status == "failed" && job.error_code.as_deref() == Some("deadline_exceeded") {
        return Err(WikiLlmError::DeadlineExceeded.into());
    }
    Ok(())
}

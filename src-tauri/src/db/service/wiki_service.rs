//! Wiki 的资料、项目绑定与任务持久化服务，供桌面和服务器共用。
//!
//! source/import/session_rollup 写入来源记录，engine 通过本模块创建、认领和重试任务；
//! 来源只保存地址与元数据，Markdown 正文读取和当前文件可用性由 wiki/read_model 负责。
//! 本模块维护数据库事务、幂等键及每次执行记录；批次提交后的消费/来源关联见
//! wiki_pipeline_service，模型调用和文件提交仍归 wiki 业务模块。

use chrono::Utc;
#[cfg(test)]
use sea_orm::QuerySelect;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::db::entities::{
    wiki_contribution, wiki_job, wiki_job_attempt, wiki_project_binding, wiki_source, wiki_vault,
};
use crate::db::error::DbError;

#[derive(Debug, Clone)]
pub struct InsertedSource {
    pub source: wiki_source::Model,
    pub job: Option<wiki_job::Model>,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertMode {
    /// 同一逻辑键已有任务即复用，用于一次性轮次记录和归纳请求。
    ReuseTerminal,
    /// 复用排队/运行中的任务；后续会话事件可重试失败任务或重新生成已有页面。
    ActiveOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiJobInfo {
    pub id: String,
    pub vault_id: String,
    pub source_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub attempt: i32,
    pub title: Option<String>,
    /// read_model 阅读时补充当前文件状态；这里的历史任务结果不会因此改写。
    pub output_availability: std::collections::BTreeMap<String, String>,
    pub attempts: Vec<WikiJobAttemptInfo>,
    pub input_manifest: Option<String>,
    pub output_manifest: Option<String>,
    pub result: Option<crate::wiki::result::JobOutputManifest>,
    pub next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiJobAttemptInfo {
    pub attempt: i32,
    pub status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub input_manifest: Option<String>,
    pub output_manifest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiSourceInfo {
    pub id: String,
    pub source_group_id: String,
    pub vault_id: String,
    pub source_kind: String,
    pub source_seq: i64,
    pub run_id: Option<String>,
    pub raw_path: Option<String>,
    pub raw_hash: Option<String>,
    pub eligibility: String,
    pub conversation_id: Option<i32>,
    pub folder_id: Option<i32>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub truncated: bool,
    pub redacted: bool,
    pub captured_at: Option<chrono::DateTime<chrono::Utc>>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation_revision: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiImportResult {
    #[serde(flatten)]
    pub source: WikiSourceInfo,
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiProjectBindingInfo {
    pub id: String,
    pub vault_id: String,
    pub db_instance_id: String,
    pub root_folder_id: i32,
    pub project_note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_folder_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_folder_path: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn project_binding_info(
    m: wiki_project_binding::Model,
    folder: Option<&crate::db::entities::folder::Model>,
) -> WikiProjectBindingInfo {
    WikiProjectBindingInfo {
        id: m.id,
        vault_id: m.vault_id,
        db_instance_id: m.db_instance_id,
        root_folder_id: m.root_folder_id,
        project_note_id: m.project_note_id,
        root_folder_name: folder.map(|f| f.name.clone()),
        root_folder_path: folder.map(|f| f.path.clone()),
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

/// 以资料库、数据库实例和根目录确定稳定项目身份，供来源元数据及项目页交叉引用。
pub async fn ensure_project_binding<C: ConnectionTrait>(
    conn: &C,
    vault_id: &str,
    db_instance_id: &str,
    root_folder_id: i32,
) -> Result<wiki_project_binding::Model, DbError> {
    if let Some(row) = wiki_project_binding::Entity::find()
        .filter(wiki_project_binding::Column::VaultId.eq(vault_id))
        .filter(wiki_project_binding::Column::DbInstanceId.eq(db_instance_id))
        .filter(wiki_project_binding::Column::RootFolderId.eq(root_folder_id))
        .one(conn)
        .await?
    {
        return Ok(row);
    }
    let now = Utc::now();
    let model = wiki_project_binding::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        vault_id: Set(vault_id.to_string()),
        db_instance_id: Set(db_instance_id.to_string()),
        root_folder_id: Set(root_folder_id),
        project_note_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    match model.insert(conn).await {
        Ok(row) => Ok(row),
        Err(e) if e.to_string().contains("UNIQUE constraint failed") => {
            wiki_project_binding::Entity::find()
                .filter(wiki_project_binding::Column::VaultId.eq(vault_id))
                .filter(wiki_project_binding::Column::DbInstanceId.eq(db_instance_id))
                .filter(wiki_project_binding::Column::RootFolderId.eq(root_folder_id))
                .one(conn)
                .await?
                .ok_or_else(|| DbError::Conflict("wiki project binding unique race".into()))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn list_project_bindings(
    conn: &DatabaseConnection,
    vault_id: Option<&str>,
) -> Result<Vec<WikiProjectBindingInfo>, DbError> {
    let mut q = wiki_project_binding::Entity::find()
        .order_by_asc(wiki_project_binding::Column::RootFolderId);
    if let Some(vault) = vault_id.filter(|s| !s.is_empty()) {
        q = q.filter(wiki_project_binding::Column::VaultId.eq(vault));
    }
    let rows = q.all(conn).await?;
    let folder_ids: Vec<i32> = rows.iter().map(|r| r.root_folder_id).collect();
    let folders = if folder_ids.is_empty() {
        Vec::new()
    } else {
        crate::db::entities::folder::Entity::find()
            .filter(crate::db::entities::folder::Column::Id.is_in(folder_ids))
            .all(conn)
            .await?
    };
    let by_id: std::collections::HashMap<i32, crate::db::entities::folder::Model> =
        folders.into_iter().map(|f| (f.id, f)).collect();
    Ok(rows
        .into_iter()
        .map(|m| {
            let folder = by_id.get(&m.root_folder_id);
            project_binding_info(m, folder)
        })
        .collect())
}

fn job_info(m: wiki_job::Model) -> WikiJobInfo {
    let result = m
        .output_manifest
        .as_deref()
        .and_then(|raw| serde_json::from_str::<crate::wiki::result::JobOutputManifest>(raw).ok())
        .filter(|r| r.version == 1);
    WikiJobInfo {
        output_availability: Default::default(),
        attempts: Vec::new(),
        title: result
            .as_ref()
            .and_then(|r| r.outputs.first())
            .map(|o| o.title.clone()),
        input_manifest: m.input_manifest,
        output_manifest: m.output_manifest,
        result,
        next_attempt_at: m.next_attempt_at,
        id: m.id,
        vault_id: m.vault_id,
        source_id: m.source_id,
        kind: m.kind,
        status: m.status,
        attempt: m.attempt,
        error_code: m.error_code,
        error: m.error_message.clone(),
        error_message: m.error_message,
        created_at: m.created_at,
        updated_at: m.updated_at,
        started_at: m.started_at,
        finished_at: m.finished_at,
    }
}

fn parse_string_list(raw: &Option<String>) -> Option<Vec<String>> {
    let s = raw.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
    serde_json::from_str(s).ok()
}

fn parse_warnings(raw: &Option<String>) -> Vec<String> {
    parse_string_list(raw).unwrap_or_default()
}

pub fn source_info(m: wiki_source::Model) -> WikiSourceInfo {
    let source_title = m.source_title.clone();
    WikiSourceInfo {
        id: m.id,
        source_group_id: m.source_group_id,
        vault_id: m.vault_id,
        source_kind: m.source_kind,
        source_seq: m.source_seq,
        run_id: m.run_id,
        raw_path: m.raw_path,
        raw_hash: m.raw_hash,
        eligibility: m.eligibility,
        conversation_id: m.conversation_id,
        folder_id: m.folder_id,
        agent_type: m.agent_type,
        model: m.model,
        truncated: m.truncated,
        redacted: m.redacted,
        captured_at: m.captured_at,
        occurred_at: m.occurred_at,
        created_at: m.created_at,
        extraction_status: m.coverage_status,
        original_filename: m.original_filename,
        format: m.format,
        title: source_title.clone(),
        source_title,
        source_summary: None,
        source_url: m.source_url,
        author: m.author,
        material_role: m.material_role,
        personal_role: m.personal_role,
        annotation_revision: Some(m.annotation_revision),
        page_count: m.page_count,
        warnings: parse_warnings(&m.warnings),
        project_ids: parse_string_list(&m.project_ids),
        area_ids: parse_string_list(&m.area_ids),
        request_id: m.request_id,
        previous_source_id: m.previous_source_id,
        extractor_version: m.extractor_version,
        original_hash: m.original_hash,
    }
}

pub async fn fill_source_title_if_empty(
    conn: &DatabaseConnection,
    source_id: &str,
    title: &str,
) -> Result<(), DbError> {
    let title = title.trim();
    if title.is_empty() {
        return Ok(());
    }
    let row = wiki_source::Entity::find_by_id(source_id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki source {source_id}")))?;
    if row
        .source_title
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return Ok(());
    }
    let mut active: wiki_source::ActiveModel = row.into();
    active.source_title = Set(Some(title.to_string()));
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

pub async fn ensure_active_vault(
    conn: &DatabaseConnection,
    canonical_path: &str,
) -> Result<wiki_vault::Model, DbError> {
    // 设置、资料导入和轮次采集共用物理目录身份，避免符号链接或 /tmp 别名拆出两份数据。
    let canonical = std::path::Path::new(canonical_path).canonicalize()?;
    let canonical_path = canonical.to_string_lossy();
    let canonical_path: &str = canonical_path.as_ref();

    let now = Utc::now();
    if let Some(existing) = wiki_vault::Entity::find()
        .filter(wiki_vault::Column::CanonicalPath.eq(canonical_path))
        .one(conn)
        .await?
    {
        if existing.is_active {
            return Ok(existing);
        }
        let txn = conn.begin().await?;
        deactivate_all(&txn).await?;
        let mut active: wiki_vault::ActiveModel = existing.into();
        active.is_active = Set(true);
        active.updated_at = Set(now);
        let updated = active.update(&txn).await?;
        txn.commit().await?;
        return Ok(updated);
    }

    let txn = conn.begin().await?;
    deactivate_all(&txn).await?;
    let model = wiki_vault::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        canonical_path: Set(canonical_path.to_string()),
        config_revision: Set(0),
        next_compile_at: Set(None),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let inserted = model.insert(&txn).await?;
    txn.commit().await?;
    Ok(inserted)
}

async fn deactivate_all<C: ConnectionTrait>(conn: &C) -> Result<(), DbError> {
    use sea_orm::sea_query::Expr;
    wiki_vault::Entity::update_many()
        .col_expr(wiki_vault::Column::IsActive, Expr::value(false))
        .col_expr(wiki_vault::Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(wiki_vault::Column::IsActive.eq(true))
        .exec(conn)
        .await?;
    Ok(())
}

pub async fn active_vault(conn: &DatabaseConnection) -> Result<Option<wiki_vault::Model>, DbError> {
    Ok(wiki_vault::Entity::find()
        .filter(wiki_vault::Column::IsActive.eq(true))
        .one(conn)
        .await?)
}

pub struct NewAcpSource {
    pub vault_id: String,
    pub run_id: String,
    pub conversation_id: Option<i32>,
    pub folder_id: Option<i32>,
    pub root_folder_id: Option<i32>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub truncated: bool,
    pub redacted: bool,
    pub source_title: Option<String>,
}

/// 同一事务创建轮次来源和 turn_summary 任务；(vault_id, run_id) 保证重复事件不重复采集。
pub async fn insert_acp_source_and_turn_job(
    conn: &DatabaseConnection,
    new: NewAcpSource,
) -> Result<InsertedSource, DbError> {
    if let Some(existing) = find_source_by_run(conn, &new.vault_id, &new.run_id).await? {
        let job = find_turn_summary_job(conn, &existing.id).await?;
        return Ok(InsertedSource {
            source: existing,
            job,
            created: false,
        });
    }

    let now = Utc::now();
    let source_id = uuid::Uuid::new_v4().to_string();
    let job_id = uuid::Uuid::new_v4().to_string();
    let dedupe_key = format!("turn_summary:{source_id}:pending");
    let db_instance_id = crate::wiki::settings::ensure_db_instance_id(conn).await?;

    let txn = conn.begin().await?;
    if let Some(existing) = find_source_by_run(&txn, &new.vault_id, &new.run_id).await? {
        let job = find_turn_summary_job(&txn, &existing.id).await?;
        txn.commit().await?;
        return Ok(InsertedSource {
            source: existing,
            job,
            created: false,
        });
    }

    let last = wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(&new.vault_id))
        .order_by_desc(wiki_source::Column::SourceSeq)
        .one(&txn)
        .await?;
    let source_seq = last.map(|s| s.source_seq + 1).unwrap_or(1);

    let source = wiki_source::ActiveModel {
        id: Set(source_id.clone()),
        source_group_id: Set(source_id.clone()),
        vault_id: Set(new.vault_id.clone()),
        source_kind: Set("acp-turn".into()),
        source_seq: Set(source_seq),
        run_id: Set(Some(new.run_id.clone())),
        original_hash: Set(None),
        raw_path: Set(None),
        raw_hash: Set(None),
        extractor_version: Set(None),
        coverage_status: Set(None),
        eligibility: Set("processing".into()),
        material_role: Set(Some("unspecified".into())),
        personal_role: Set(None),
        annotation_revision: Set(0),
        conversation_id: Set(new.conversation_id),
        folder_id: Set(new.folder_id),
        root_folder_id: Set(new.root_folder_id),
        agent_type: Set(new.agent_type),
        model: Set(new.model),
        mode: Set(new.mode),
        captured_at: Set(Some(new.captured_at)),
        occurred_at: Set(new.occurred_at),
        truncated: Set(new.truncated),
        redacted: Set(new.redacted),
        request_id: Set(None),
        original_filename: Set(None),
        format: Set(None),
        source_title: Set(new.source_title.clone()),
        source_url: Set(None),
        author: Set(None),
        project_ids: Set(if let Some(root_id) = new.root_folder_id {
            let binding =
                ensure_project_binding(&txn, &new.vault_id, &db_instance_id, root_id).await?;
            Some(serde_json::to_string(&vec![binding.id]).unwrap_or_else(|_| "[]".into()))
        } else {
            None
        }),
        area_ids: Set(None),
        warnings: Set(None),
        page_count: Set(None),
        previous_source_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let source_model = match source.insert(&txn).await {
        Ok(m) => m,
        Err(e) if e.to_string().contains("UNIQUE constraint failed") => {
            txn.rollback().await.ok();
            let existing = find_source_by_run(conn, &new.vault_id, &new.run_id)
                .await?
                .ok_or_else(|| DbError::Conflict("wiki source unique race".into()))?;
            let job = find_turn_summary_job(conn, &existing.id).await?;
            return Ok(InsertedSource {
                source: existing,
                job,
                created: false,
            });
        }
        Err(e) => return Err(e.into()),
    };

    let job = wiki_job::ActiveModel {
        id: Set(job_id),
        vault_id: Set(new.vault_id),
        source_id: Set(Some(source_id)),
        kind: Set("turn_summary".into()),
        status: Set("queued".into()),
        dedupe_key: Set(Some(dedupe_key)),
        input_manifest: Set(None),
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
    };
    let job_model = job.insert(&txn).await?;
    txn.commit().await?;
    Ok(InsertedSource {
        source: source_model,
        job: Some(job_model),
        created: true,
    })
}

async fn find_source_by_run<C: ConnectionTrait>(
    conn: &C,
    vault_id: &str,
    run_id: &str,
) -> Result<Option<wiki_source::Model>, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .filter(wiki_source::Column::RunId.eq(run_id))
        .one(conn)
        .await?)
}

async fn find_turn_summary_job<C: ConnectionTrait>(
    conn: &C,
    source_id: &str,
) -> Result<Option<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::SourceId.eq(source_id))
        .filter(wiki_job::Column::Kind.eq("turn_summary"))
        .order_by_asc(wiki_job::Column::CreatedAt)
        .one(conn)
        .await?)
}

pub async fn mark_source_raw(
    conn: &DatabaseConnection,
    source_id: &str,
    raw_path: &str,
    raw_hash: &str,
    eligibility: &str,
) -> Result<(), DbError> {
    let Some(row) = wiki_source::Entity::find_by_id(source_id).one(conn).await? else {
        return Err(DbError::NotFound(format!("wiki source {source_id}")));
    };
    let mut active: wiki_source::ActiveModel = row.into();
    active.raw_path = Set(Some(raw_path.to_string()));
    active.raw_hash = Set(Some(raw_hash.to_string()));
    active.eligibility = Set(eligibility.to_string());
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

// 按 (job_id, attempt) 更新当前执行快照；重试只开启下一次 attempt，保留前次失败详情。
async fn record_job_attempt<C: ConnectionTrait>(
    conn: &C,
    job: &wiki_job::Model,
) -> Result<(), DbError> {
    use sea_orm::sea_query::OnConflict;
    let record = wiki_job_attempt::ActiveModel {
        id: Set(format!("{}:{}", job.id, job.attempt)),
        job_id: Set(job.id.clone()),
        attempt: Set(job.attempt),
        status: Set(job.status.clone()),
        input_manifest: Set(job.input_manifest.clone()),
        output_manifest: Set(job.output_manifest.clone()),
        model_id: Set(job.model_id.clone()),
        protocol: Set(job.protocol.clone()),
        error_code: Set(job.error_code.clone()),
        error_message: Set(job.error_message.clone()),
        started_at: Set(job.started_at),
        finished_at: Set(job.finished_at),
    };
    wiki_job_attempt::Entity::insert(record)
        .on_conflict(
            OnConflict::columns([
                wiki_job_attempt::Column::JobId,
                wiki_job_attempt::Column::Attempt,
            ])
            .update_columns([
                wiki_job_attempt::Column::Status,
                wiki_job_attempt::Column::InputManifest,
                wiki_job_attempt::Column::OutputManifest,
                wiki_job_attempt::Column::ModelId,
                wiki_job_attempt::Column::Protocol,
                wiki_job_attempt::Column::ErrorCode,
                wiki_job_attempt::Column::ErrorMessage,
                wiki_job_attempt::Column::StartedAt,
                wiki_job_attempt::Column::FinishedAt,
            ])
            .to_owned(),
        )
        .exec(conn)
        .await?;
    Ok(())
}

/// 状态与执行记录一并提交；已取消任务不能被迟到的执行结果重新标记成功。
pub async fn mark_job(
    conn: &DatabaseConnection,
    job_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), DbError> {
    let txn = conn.begin().await?;
    let Some(row) = wiki_job::Entity::find_by_id(job_id).one(&txn).await? else {
        return Err(DbError::NotFound(format!("wiki job {job_id}")));
    };
    if row.status == "cancelled" && status != "cancelled" {
        return Ok(());
    }
    let now = Utc::now();
    let mut active: wiki_job::ActiveModel = row.into();
    active.status = Set(status.to_string());
    active.error_code = Set(error_code.map(str::to_string));
    active.error_message = Set(error_message.map(str::to_string));
    active.updated_at = Set(now);
    if status == "running" {
        active.started_at = Set(Some(now));
    }
    if matches!(status, "succeeded" | "failed" | "cancelled") {
        active.finished_at = Set(Some(now));
    }
    let updated = active.update(&txn).await?;
    record_job_attempt(&txn, &updated).await?;
    txn.commit().await?;
    Ok(())
}

pub async fn insert_failed_job(
    conn: &DatabaseConnection,
    vault_id: Option<&str>,
    source_id: Option<&str>,
    error_code: &str,
    error_message: &str,
) -> Result<wiki_job::Model, DbError> {
    let now = Utc::now();
    let vault_id = match vault_id {
        Some(v) => v.to_string(),
        None => active_vault(conn)
            .await?
            .map(|v| v.id)
            .unwrap_or_else(|| "unbound".into()),
    };
    let job = wiki_job::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        vault_id: Set(vault_id),
        source_id: Set(source_id.map(str::to_string)),
        kind: Set("turn_summary".into()),
        status: Set("failed".into()),
        dedupe_key: Set(None),
        input_manifest: Set(None),
        config_version: Set(None),
        model_id: Set(None),
        protocol: Set(None),
        attempt: Set(1),
        error_code: Set(Some(error_code.into())),
        error_message: Set(Some(error_message.into())),
        output_manifest: Set(None),
        next_attempt_at: Set(None),
        started_at: Set(None),
        finished_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(job.insert(conn).await?)
}

// 导入/会话行为测试直接检查持久化结果；生产列表通过 wiki/read_model 分页读取。
#[cfg(test)]
pub async fn list_jobs(
    conn: &DatabaseConnection,
    limit: u64,
    offset: u64,
    status: Option<&str>,
) -> Result<Vec<WikiJobInfo>, DbError> {
    let mut q = wiki_job::Entity::find().order_by_desc(wiki_job::Column::CreatedAt);
    if let Some(st) = status.filter(|s| !s.is_empty()) {
        q = q.filter(wiki_job::Column::Status.eq(st));
    }
    let rows = q.offset(offset).limit(limit).all(conn).await?;
    Ok(rows.into_iter().map(job_info).collect())
}

/// 组装任务持久化详情及执行历史；文件是否存在由 read_model 在读取时补充。
pub async fn get_job(conn: &DatabaseConnection, id: &str) -> Result<WikiJobInfo, DbError> {
    let row = get_job_model(conn, id).await?;
    let mut info = job_info(row);
    if let Some(source_id) = &info.source_id {
        if let Some(source) = wiki_source::Entity::find_by_id(source_id).one(conn).await? {
            info.title = source
                .source_title
                .or(source.original_filename)
                .or(info.title);
        }
    }
    if info.title.is_none() {
        let conversation_id = info
            .input_manifest
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|manifest| manifest.get("conversation_id").and_then(|id| id.as_i64()));
        if let Some(conversation_id) = conversation_id {
            if let Some(conversation) =
                crate::db::entities::conversation::Entity::find_by_id(conversation_id as i32)
                    .one(conn)
                    .await?
            {
                info.title = conversation.title;
            }
        }
    }
    info.attempts = list_job_attempts(conn, id)
        .await?
        .into_iter()
        .map(|attempt| WikiJobAttemptInfo {
            attempt: attempt.attempt,
            status: attempt.status,
            started_at: attempt.started_at,
            finished_at: attempt.finished_at,
            error_code: attempt.error_code,
            error_message: attempt.error_message,
            input_manifest: attempt.input_manifest,
            output_manifest: attempt.output_manifest,
        })
        .collect();
    Ok(info)
}

// 测试辅助查询保留旧签名以复用导入场景；生产资料列表归 wiki/read_model。
#[cfg(test)]
pub async fn list_sources(
    conn: &DatabaseConnection,
    limit: u64,
    offset: u64,
    source_kind: Option<&str>,
    project_id: Option<&str>,
) -> Result<Vec<WikiSourceInfo>, DbError> {
    let mut q = wiki_source::Entity::find().order_by_desc(wiki_source::Column::SourceSeq);
    if let Some(kind) = source_kind.filter(|s| !s.is_empty()) {
        q = q.filter(wiki_source::Column::SourceKind.eq(kind));
    }
    let project_id = project_id.filter(|s| !s.is_empty());
    let rows = if project_id.is_some() {
        q.all(conn).await?
    } else {
        q.offset(offset).limit(limit).all(conn).await?
    };
    let iter = rows.into_iter().filter(|row| {
        project_id
            .map(|id| {
                parse_string_list(&row.project_ids).is_some_and(|ids| ids.iter().any(|v| v == id))
            })
            .unwrap_or(true)
    });
    let items: Vec<WikiSourceInfo> = if project_id.is_some() {
        iter.skip(offset as usize)
            .take(limit as usize)
            .map(source_info)
            .collect()
    } else {
        iter.map(source_info).collect()
    };
    Ok(items)
}

pub async fn get_source(conn: &DatabaseConnection, id: &str) -> Result<WikiSourceInfo, DbError> {
    let row = wiki_source::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki source {id}")))?;
    let items = vec![source_info(row)];
    Ok(items.into_iter().next().expect("one source"))
}

pub async fn get_job_model<C: ConnectionTrait>(
    conn: &C,
    id: &str,
) -> Result<wiki_job::Model, DbError> {
    wiki_job::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki job {id}")))
}

pub async fn get_source_model(
    conn: &DatabaseConnection,
    id: &str,
) -> Result<wiki_source::Model, DbError> {
    wiki_source::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki source {id}")))
}

pub async fn find_source_by_request_id<C: ConnectionTrait>(
    conn: &C,
    vault_id: &str,
    request_id: &str,
) -> Result<Option<wiki_source::Model>, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .filter(wiki_source::Column::RequestId.eq(request_id))
        .one(conn)
        .await?)
}

pub async fn find_source_by_original_hash<C: ConnectionTrait>(
    conn: &C,
    vault_id: &str,
    original_hash: &str,
) -> Result<Option<wiki_source::Model>, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .filter(wiki_source::Column::OriginalHash.eq(original_hash))
        .order_by_asc(wiki_source::Column::SourceSeq)
        .one(conn)
        .await?)
}

pub struct InsertedImportSource {
    pub source: wiki_source::Model,
    pub created: bool,
}

pub struct NewImportSource {
    pub id: Option<String>,
    pub vault_id: String,
    pub source_kind: String,
    pub request_id: String,
    pub original_hash: String,
    pub original_filename: Option<String>,
    pub format: Option<String>,
    pub source_title: Option<String>,
    pub source_url: Option<String>,
    pub author: Option<String>,
    pub material_role: String,
    pub personal_role: Option<String>,
    pub project_ids: Option<String>,
    pub area_ids: Option<String>,
    pub extractor_version: Option<String>,
    pub coverage_status: Option<String>,
    pub eligibility: String,
    pub warnings: Option<String>,
    pub page_count: Option<i32>,
    pub truncated: bool,
    pub redacted: bool,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub source_group_id: Option<String>,
    pub previous_source_id: Option<String>,
    pub skip_hash_dedup: bool,
    pub raw_path: Option<String>,
    pub raw_hash: Option<String>,
}

/// Insert an extracted document/pasted source. Idempotent on (vault_id, request_id).
/// Same original_hash returns the earliest existing source unless `skip_hash_dedup`.
/// 登记导入资料；request_id 防止请求重放，原文哈希仅用于识别重复导入，不冻结输入。
pub async fn insert_import_source(
    conn: &DatabaseConnection,
    new: NewImportSource,
) -> Result<InsertedImportSource, DbError> {
    if let Some(existing) = find_source_by_request_id(conn, &new.vault_id, &new.request_id).await? {
        return Ok(existing_import(existing));
    }
    if !new.skip_hash_dedup {
        if let Some(existing) =
            find_source_by_original_hash(conn, &new.vault_id, &new.original_hash).await?
        {
            return Ok(existing_import(existing));
        }
    }

    let now = Utc::now();
    let source_id = new
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let vault_id = new.vault_id.clone();
    let request_id = new.request_id.clone();
    let original_hash = new.original_hash.clone();
    let skip_hash_dedup = new.skip_hash_dedup;
    let group_id = new
        .source_group_id
        .clone()
        .unwrap_or_else(|| source_id.clone());

    let txn = conn.begin().await?;
    if let Some(existing) = find_source_by_request_id(&txn, &vault_id, &request_id).await? {
        txn.commit().await?;
        return Ok(existing_import(existing));
    }
    if !skip_hash_dedup {
        if let Some(existing) =
            find_source_by_original_hash(&txn, &vault_id, &original_hash).await?
        {
            txn.commit().await?;
            return Ok(existing_import(existing));
        }
    }

    let last = wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(&vault_id))
        .order_by_desc(wiki_source::Column::SourceSeq)
        .one(&txn)
        .await?;
    let source_seq = last.map(|s| s.source_seq + 1).unwrap_or(1);

    let source = wiki_source::ActiveModel {
        id: Set(source_id.clone()),
        source_group_id: Set(group_id),
        vault_id: Set(vault_id.clone()),
        source_kind: Set(new.source_kind),
        source_seq: Set(source_seq),
        run_id: Set(None),
        original_hash: Set(Some(original_hash.clone())),
        raw_path: Set(new.raw_path),
        raw_hash: Set(new.raw_hash),
        extractor_version: Set(new.extractor_version),
        coverage_status: Set(new.coverage_status),
        eligibility: Set(new.eligibility),
        material_role: Set(Some(new.material_role)),
        personal_role: Set(new.personal_role),
        annotation_revision: Set(0),
        conversation_id: Set(None),
        folder_id: Set(None),
        root_folder_id: Set(None),
        agent_type: Set(None),
        model: Set(None),
        mode: Set(None),
        captured_at: Set(Some(new.captured_at)),
        occurred_at: Set(None),
        truncated: Set(new.truncated),
        redacted: Set(new.redacted),
        request_id: Set(Some(request_id.clone())),
        original_filename: Set(new.original_filename),
        format: Set(new.format),
        source_title: Set(new.source_title),
        source_url: Set(new.source_url),
        author: Set(new.author),
        project_ids: Set(new.project_ids),
        area_ids: Set(new.area_ids),
        warnings: Set(new.warnings),
        page_count: Set(new.page_count),
        previous_source_id: Set(new.previous_source_id),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let source_model = match source.insert(&txn).await {
        Ok(m) => m,
        Err(e) if e.to_string().contains("UNIQUE constraint failed") => {
            txn.rollback().await.ok();
            if let Some(existing) = find_source_by_request_id(conn, &vault_id, &request_id).await? {
                return Ok(existing_import(existing));
            }
            if !skip_hash_dedup {
                if let Some(existing) =
                    find_source_by_original_hash(conn, &vault_id, &original_hash).await?
                {
                    return Ok(existing_import(existing));
                }
            }
            return Err(DbError::Conflict("wiki import unique race".into()));
        }
        Err(e) => return Err(e.into()),
    };

    txn.commit().await?;
    Ok(InsertedImportSource {
        source: source_model,
        created: true,
    })
}

fn existing_import(existing: wiki_source::Model) -> InsertedImportSource {
    InsertedImportSource {
        source: existing,
        created: false,
    }
}

pub struct AnnotationPatch {
    pub material_role: Option<String>,
    pub personal_role: Option<String>,
    pub project_ids: Option<Vec<String>>,
    pub area_ids: Option<Vec<String>>,
}

pub async fn update_source_annotations(
    conn: &DatabaseConnection,
    source_id: &str,
    patch: AnnotationPatch,
) -> Result<wiki_source::Model, DbError> {
    let row = get_source_model(conn, source_id).await?;
    let mut active: wiki_source::ActiveModel = row.clone().into();
    if let Some(role) = patch.material_role {
        active.material_role = Set(Some(role));
    }
    if let Some(role) = patch.personal_role {
        active.personal_role = Set(if role.is_empty() { None } else { Some(role) });
    }
    if let Some(ids) = patch.project_ids {
        active.project_ids = Set(Some(
            serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()),
        ));
    }
    if let Some(ids) = patch.area_ids {
        active.area_ids = Set(Some(
            serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()),
        ));
    }
    active.annotation_revision = Set(row.annotation_revision.saturating_add(1));
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

pub async fn set_source_eligibility(
    conn: &DatabaseConnection,
    source_id: &str,
    eligibility: &str,
) -> Result<wiki_source::Model, DbError> {
    let row = get_source_model(conn, source_id).await?;
    let mut active: wiki_source::ActiveModel = row.into();
    active.eligibility = Set(eligibility.to_string());
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

pub async fn link_source_version(
    conn: &DatabaseConnection,
    source_id: &str,
    previous_source_id: &str,
) -> Result<wiki_source::Model, DbError> {
    let previous = get_source_model(conn, previous_source_id).await?;
    let row = get_source_model(conn, source_id).await?;
    if row.vault_id != previous.vault_id {
        return Err(DbError::Validation(
            "cannot link sources from different vaults".into(),
        ));
    }
    let mut active: wiki_source::ActiveModel = row.into();
    active.source_group_id = Set(previous.source_group_id);
    active.previous_source_id = Set(Some(previous.id));
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

pub async fn claim_next_queued_job(
    conn: &DatabaseConnection,
    vault_id: &str,
    kind: &str,
) -> Result<Option<wiki_job::Model>, DbError> {
    let Some(job) = wiki_job::Entity::find()
        .filter(wiki_job::Column::Kind.eq(kind))
        .filter(wiki_job::Column::VaultId.eq(vault_id))
        .filter(wiki_job::Column::Status.eq("queued"))
        .filter(
            sea_orm::Condition::any()
                .add(wiki_job::Column::NextAttemptAt.is_null())
                .add(wiki_job::Column::NextAttemptAt.lte(Utc::now())),
        )
        .order_by_asc(wiki_job::Column::CreatedAt)
        .one(conn)
        .await?
    else {
        return Ok(None);
    };
    // 候选读取与认领之间可能被另一调度器抢先；统一入口会再次检查状态和退避时间。
    claim_job_if_queued(conn, vault_id, &job.id).await
}

pub async fn list_jobs_by_status(
    conn: &DatabaseConnection,
    status: &str,
) -> Result<Vec<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::Status.eq(status))
        .all(conn)
        .await?)
}

pub async fn find_job_by_dedupe_key(
    conn: &DatabaseConnection,
    vault_id: &str,
    dedupe_key: &str,
) -> Result<Option<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::DedupeKey.eq(dedupe_key))
        .filter(wiki_job::Column::VaultId.eq(vault_id))
        .order_by_asc(wiki_job::Column::CreatedAt)
        .one(conn)
        .await?)
}

/// 创建 Wiki 归纳任务，记录生成契约版本；并发插入命中唯一键时复用同一任务。
pub async fn insert_compile_job(
    conn: &DatabaseConnection,
    vault_id: &str,
    dedupe_key: &str,
    input_manifest: Option<&str>,
) -> Result<wiki_job::Model, DbError> {
    if let Some(existing) = find_job_by_dedupe_key(conn, vault_id, dedupe_key).await? {
        return Ok(existing);
    }
    let now = Utc::now();
    let job = wiki_job::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        vault_id: Set(vault_id.to_string()),
        source_id: Set(None),
        kind: Set("wiki_synthesize".into()),
        status: Set("queued".into()),
        dedupe_key: Set(Some(dedupe_key.to_string())),
        input_manifest: Set(input_manifest.map(str::to_string)),
        config_version: Set(Some(
            crate::wiki::llm::SYNTHESIZE_CONTRACT_VERSION.to_string(),
        )),
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
    };
    match job.insert(conn).await {
        Ok(m) => Ok(m),
        Err(e) if e.to_string().contains("UNIQUE constraint failed") => {
            find_job_by_dedupe_key(conn, vault_id, dedupe_key)
                .await?
                .ok_or_else(|| DbError::Conflict("wiki compile job unique race".into()))
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn insert_kind_job(
    conn: &DatabaseConnection,
    vault_id: &str,
    kind: &str,
    dedupe_key: &str,
    source_id: Option<&str>,
    input_manifest: Option<&str>,
    mode: InsertMode,
) -> Result<wiki_job::Model, DbError> {
    if let Some(existing) = find_job_by_dedupe_key(conn, vault_id, dedupe_key).await? {
        match mode {
            InsertMode::ReuseTerminal => return Ok(existing),
            InsertMode::ActiveOnly if matches!(existing.status.as_str(), "queued" | "running") => {
                return Ok(existing);
            }
            InsertMode::ActiveOnly if existing.status == "failed" => {
                return retry_job(conn, &existing.id).await;
            }
            InsertMode::ActiveOnly => {
                let mut active: wiki_job::ActiveModel = existing.into();
                active.status = Set("queued".into());
                active.attempt = Set(active.attempt.take().unwrap_or(0) + 1);
                active.input_manifest = Set(input_manifest.map(str::to_string));
                active.output_manifest = Set(None);
                active.started_at = Set(None);
                active.finished_at = Set(None);
                active.next_attempt_at = Set(None);
                active.error_code = Set(None);
                active.error_message = Set(None);
                active.updated_at = Set(Utc::now());
                return Ok(active.update(conn).await?);
            }
        }
    }
    let now = Utc::now();
    let job = wiki_job::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        vault_id: Set(vault_id.to_string()),
        source_id: Set(source_id.map(str::to_string)),
        kind: Set(kind.to_string()),
        status: Set("queued".into()),
        dedupe_key: Set(Some(dedupe_key.to_string())),
        input_manifest: Set(input_manifest.map(str::to_string)),
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
    };
    Ok(job.insert(conn).await?)
}

pub async fn set_job_dedupe_and_manifest(
    conn: &DatabaseConnection,
    job_id: &str,
    dedupe_key: &str,
    manifest: &str,
) -> Result<(), DbError> {
    let row = get_job_model(conn, job_id).await?;
    let mut active: wiki_job::ActiveModel = row.into();
    active.dedupe_key = Set(Some(dedupe_key.to_string()));
    active.input_manifest = Set(Some(manifest.to_string()));
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

#[cfg(test)]
pub async fn list_jobs_by_kind(
    conn: &DatabaseConnection,
    kind: &str,
) -> Result<Vec<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::Kind.eq(kind))
        .order_by_asc(wiki_job::Column::CreatedAt)
        .all(conn)
        .await?)
}

pub async fn list_queued_jobs_by_kind(
    conn: &DatabaseConnection,
    vault_id: &str,
    kind: &str,
) -> Result<Vec<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::Kind.eq(kind))
        .filter(wiki_job::Column::VaultId.eq(vault_id))
        .filter(wiki_job::Column::Status.eq("queued"))
        .filter(
            sea_orm::Condition::any()
                .add(wiki_job::Column::NextAttemptAt.is_null())
                .add(wiki_job::Column::NextAttemptAt.lte(Utc::now())),
        )
        .order_by_asc(wiki_job::Column::CreatedAt)
        .all(conn)
        .await?)
}

/// 使用带状态条件的 UPDATE 原子认领任务，并在同一事务记录本次执行。
/// 轮次、会话汇总和 Wiki 归纳统一经过这里，保证资料库隔离与重试退避一致。
pub async fn claim_job_if_queued(
    conn: &DatabaseConnection,
    vault_id: &str,
    job_id: &str,
) -> Result<Option<wiki_job::Model>, DbError> {
    use sea_orm::sea_query::Expr;
    let txn = conn.begin().await?;
    let now = Utc::now();
    let res = wiki_job::Entity::update_many()
        .col_expr(wiki_job::Column::Status, Expr::value("running"))
        .col_expr(wiki_job::Column::StartedAt, Expr::value(now))
        .col_expr(wiki_job::Column::UpdatedAt, Expr::value(now))
        .filter(wiki_job::Column::Id.eq(job_id))
        .filter(wiki_job::Column::VaultId.eq(vault_id))
        .filter(wiki_job::Column::Status.eq("queued"))
        .filter(
            sea_orm::Condition::any()
                .add(wiki_job::Column::NextAttemptAt.is_null())
                .add(wiki_job::Column::NextAttemptAt.lte(Utc::now())),
        )
        .exec(&txn)
        .await?;
    if res.rows_affected == 0 {
        return Ok(None);
    }
    let claimed = get_job_model(&txn, job_id).await?;
    record_job_attempt(&txn, &claimed).await?;
    txn.commit().await?;
    Ok(Some(claimed))
}

pub async fn conversation_has_active_turn_summary(
    conn: &DatabaseConnection,
    vault_id: &str,
    conversation_id: i32,
) -> Result<bool, DbError> {
    let jobs = wiki_job::Entity::find()
        .filter(wiki_job::Column::Kind.eq("turn_summary"))
        .filter(wiki_job::Column::VaultId.eq(vault_id))
        .filter(wiki_job::Column::Status.is_in(["queued", "running"]))
        .all(conn)
        .await?;
    for job in jobs {
        if let Some(raw) = job.input_manifest.as_deref() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
                let cid = v.get("conversation_id").and_then(|x| {
                    x.as_i64()
                        .map(|n| n as i32)
                        .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
                });
                if cid == Some(conversation_id) {
                    return Ok(true);
                }
            }
        }
        if let Some(sid) = job.source_id.as_deref() {
            if let Ok(src) = get_source_model(conn, sid).await {
                if src.conversation_id == Some(conversation_id) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub async fn list_sources_for_conversation(
    conn: &DatabaseConnection,
    vault_id: &str,
    conversation_id: i32,
) -> Result<Vec<wiki_source::Model>, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::ConversationId.eq(conversation_id))
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .order_by_asc(wiki_source::Column::SourceSeq)
        .all(conn)
        .await?)
}

pub async fn find_local_session_source(
    conn: &DatabaseConnection,
    vault_id: &str,
    conversation_id: i32,
) -> Result<Option<wiki_source::Model>, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::ConversationId.eq(conversation_id))
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .filter(wiki_source::Column::SourceKind.eq("local-session"))
        .order_by_asc(wiki_source::Column::CreatedAt)
        .one(conn)
        .await?)
}

pub struct NewLocalSessionSource {
    pub id: String,
    pub vault_id: String,
    pub conversation_id: i32,
    pub folder_id: Option<i32>,
    pub root_folder_id: Option<i32>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    pub source_title: Option<String>,
    pub raw_path: String,
    pub raw_hash: String,
    pub project_ids: Option<String>,
}

pub async fn insert_local_session_source(
    conn: &DatabaseConnection,
    new: NewLocalSessionSource,
) -> Result<wiki_source::Model, DbError> {
    if let Some(existing) =
        find_local_session_source(conn, &new.vault_id, new.conversation_id).await?
    {
        return Ok(existing);
    }
    let now = Utc::now();
    let last = wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(&new.vault_id))
        .order_by_desc(wiki_source::Column::SourceSeq)
        .one(conn)
        .await?;
    let source_seq = last.map(|s| s.source_seq + 1).unwrap_or(1);
    let source = wiki_source::ActiveModel {
        id: Set(new.id.clone()),
        source_group_id: Set(new.id.clone()),
        vault_id: Set(new.vault_id),
        source_kind: Set("local-session".into()),
        source_seq: Set(source_seq),
        run_id: Set(None),
        original_hash: Set(None),
        raw_path: Set(Some(new.raw_path)),
        raw_hash: Set(Some(new.raw_hash)),
        extractor_version: Set(None),
        coverage_status: Set(None),
        eligibility: Set("ready".into()),
        material_role: Set(Some("unspecified".into())),
        personal_role: Set(None),
        annotation_revision: Set(0),
        conversation_id: Set(Some(new.conversation_id)),
        folder_id: Set(new.folder_id),
        root_folder_id: Set(new.root_folder_id),
        agent_type: Set(new.agent_type),
        model: Set(new.model),
        mode: Set(None),
        captured_at: Set(Some(new.captured_at)),
        occurred_at: Set(new.occurred_at),
        truncated: Set(false),
        redacted: Set(false),
        request_id: Set(None),
        original_filename: Set(None),
        format: Set(None),
        source_title: Set(new.source_title),
        source_url: Set(None),
        author: Set(None),
        project_ids: Set(new.project_ids),
        area_ids: Set(None),
        warnings: Set(None),
        page_count: Set(None),
        previous_source_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(source.insert(conn).await?)
}

pub async fn set_job_input_manifest(
    conn: &DatabaseConnection,
    job_id: &str,
    manifest: &str,
) -> Result<(), DbError> {
    let row = get_job_model(conn, job_id).await?;
    let mut active: wiki_job::ActiveModel = row.into();
    active.input_manifest = Set(Some(manifest.to_string()));
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

pub async fn set_job_output_manifest(
    conn: &DatabaseConnection,
    job_id: &str,
    manifest: &str,
) -> Result<(), DbError> {
    let txn = conn.begin().await?;
    let row = get_job_model(&txn, job_id).await?;
    let mut active: wiki_job::ActiveModel = row.into();
    active.output_manifest = Set(Some(manifest.to_string()));
    active.updated_at = Set(Utc::now());
    let updated = active.update(&txn).await?;
    record_job_attempt(&txn, &updated).await?;
    txn.commit().await?;
    Ok(())
}

pub async fn set_job_model_meta(
    conn: &DatabaseConnection,
    job_id: &str,
    model_id: Option<&str>,
    protocol: Option<&str>,
) -> Result<(), DbError> {
    let row = get_job_model(conn, job_id).await?;
    let mut active: wiki_job::ActiveModel = row.into();
    active.model_id = Set(model_id.map(str::to_string));
    active.protocol = Set(protocol.map(str::to_string));
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

/// 自动重试最多三次，等待 1/5 分钟；历史失败详情已由 mark_job 写入 attempt 表。
pub async fn requeue_after_failure(
    conn: &DatabaseConnection,
    job_id: &str,
    attempt: i32,
) -> Result<(), DbError> {
    let row = get_job_model(conn, job_id).await?;
    if row.status != "failed" || attempt >= 3 {
        return Ok(());
    }
    let mut active: wiki_job::ActiveModel = row.into();
    active.status = Set("queued".into());
    active.attempt = Set(attempt + 1);
    active.finished_at = Set(None);
    active.started_at = Set(None);
    active.next_attempt_at = Set(Some(
        Utc::now() + chrono::Duration::minutes(if attempt <= 1 { 1 } else { 5 }),
    ));
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

/// 用户主动重试不等待自动退避；增加 attempt，已提交产物仍供任务恢复流程使用。
pub async fn retry_job(conn: &DatabaseConnection, id: &str) -> Result<wiki_job::Model, DbError> {
    let row = get_job_model(conn, id).await?;
    if row.status != "failed" {
        return Err(DbError::Validation(format!(
            "job {id} is {} and cannot be retried",
            row.status
        )));
    }
    let attempt = row.attempt;
    let mut active: wiki_job::ActiveModel = row.into();
    active.status = Set("queued".into());
    active.attempt = Set(attempt + 1);
    active.error_code = Set(None);
    active.error_message = Set(None);
    active.finished_at = Set(None);
    active.started_at = Set(None);
    active.next_attempt_at = Set(None);
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

pub async fn cancel_job(conn: &DatabaseConnection, id: &str) -> Result<wiki_job::Model, DbError> {
    let row = get_job_model(conn, id).await?;
    if matches!(row.status.as_str(), "succeeded" | "cancelled") {
        return Ok(row);
    }
    mark_job(conn, id, "cancelled", Some("cancelled"), None).await?;
    get_job_model(conn, id).await
}

async fn list_job_attempts(
    conn: &DatabaseConnection,
    id: &str,
) -> Result<Vec<wiki_job_attempt::Model>, DbError> {
    Ok(wiki_job_attempt::Entity::find()
        .filter(wiki_job_attempt::Column::JobId.eq(id))
        .order_by_asc(wiki_job_attempt::Column::Attempt)
        .all(conn)
        .await?)
}

pub async fn set_vault_next_compile_at(
    conn: &DatabaseConnection,
    vault_id: &str,
    next: Option<chrono::DateTime<Utc>>,
) -> Result<(), DbError> {
    let Some(row) = wiki_vault::Entity::find_by_id(vault_id).one(conn).await? else {
        return Err(DbError::NotFound(format!("wiki vault {vault_id}")));
    };
    let mut active: wiki_vault::ActiveModel = row.into();
    active.next_compile_at = Set(next);
    active.updated_at = Set(Utc::now());
    active.update(conn).await?;
    Ok(())
}

pub async fn max_source_seq(
    conn: &DatabaseConnection,
    vault_id: &str,
) -> Result<Option<i64>, DbError> {
    let row = wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(vault_id))
        .order_by_desc(wiki_source::Column::SourceSeq)
        .one(conn)
        .await?;
    Ok(row.map(|s| s.source_seq))
}

/// 记录轮次工作记录与来源的页面级关联；批次归纳的关联由 pipeline 事务统一写入。
pub async fn insert_contribution(
    conn: &DatabaseConnection,
    source_id: &str,
    raw_hash: &str,
    annotation_revision: i32,
    note_id: &str,
    commit_id: Option<&str>,
) -> Result<wiki_contribution::Model, DbError> {
    let row = wiki_contribution::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        source_id: Set(source_id.to_string()),
        raw_hash: Set(raw_hash.to_string()),
        annotation_revision: Set(annotation_revision),
        note_id: Set(note_id.to_string()),
        claim_id: Set(None),
        evidence_id: Set(None),
        case_id: Set(None),
        commit_id: Set(commit_id.map(str::to_string)),
        created_at: Set(Utc::now()),
    };
    Ok(row.insert(conn).await?)
}

pub async fn list_contributions_for_source(
    conn: &DatabaseConnection,
    source_id: &str,
) -> Result<Vec<wiki_contribution::Model>, DbError> {
    Ok(wiki_contribution::Entity::find()
        .filter(wiki_contribution::Column::SourceId.eq(source_id))
        .order_by_asc(wiki_contribution::Column::CreatedAt)
        .all(conn)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::fresh_in_memory_db;

    #[tokio::test]
    async fn retry_backoff_prevents_immediate_reclaim() {
        let db = fresh_in_memory_db().await;
        let directory = tempfile::tempdir().unwrap();
        let vault = ensure_active_vault(&db.conn, &directory.path().to_string_lossy())
            .await
            .unwrap();
        let job = insert_kind_job(
            &db.conn,
            &vault.id,
            "turn_summary",
            "retry-test",
            None,
            None,
            InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let claimed = claim_next_queued_job(&db.conn, &vault.id, "turn_summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, job.id);
        mark_job(&db.conn, &job.id, "failed", Some("model_failed"), None)
            .await
            .unwrap();
        requeue_after_failure(&db.conn, &job.id, 1).await.unwrap();
        assert!(
            claim_next_queued_job(&db.conn, &vault.id, "turn_summary")
                .await
                .unwrap()
                .is_none(),
            "a transient failure must wait one minute before its second attempt"
        );
        assert!(
            claim_job_if_queued(&db.conn, &vault.id, &job.id)
                .await
                .unwrap()
                .is_none(),
            "direct session claims must honor the same backoff"
        );
    }

    #[tokio::test]
    async fn retry_preserves_previous_attempt_result() {
        let db = fresh_in_memory_db().await;
        let directory = tempfile::tempdir().unwrap();
        let vault = ensure_active_vault(&db.conn, &directory.path().to_string_lossy())
            .await
            .unwrap();
        let job = insert_kind_job(
            &db.conn,
            &vault.id,
            "turn_summary",
            "attempt-test",
            None,
            None,
            InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        claim_next_queued_job(&db.conn, &vault.id, "turn_summary")
            .await
            .unwrap();
        set_job_input_manifest(&db.conn, &job.id, "{\"frozen\":1}")
            .await
            .unwrap();
        mark_job(
            &db.conn,
            &job.id,
            "failed",
            Some("invalid_output"),
            Some("first failure"),
        )
        .await
        .unwrap();
        retry_job(&db.conn, &job.id).await.unwrap();
        claim_next_queued_job(&db.conn, &vault.id, "turn_summary")
            .await
            .unwrap();
        mark_job(&db.conn, &job.id, "succeeded", None, None)
            .await
            .unwrap();
        let attempts = list_job_attempts(&db.conn, &job.id).await.unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].status, "failed");
        assert_eq!(attempts[0].error_code.as_deref(), Some("invalid_output"));
        assert_eq!(
            attempts[0].input_manifest.as_deref(),
            Some("{\"frozen\":1}")
        );
        assert_eq!(attempts[1].status, "succeeded");
    }

    #[tokio::test]
    async fn queued_jobs_from_another_vault_are_not_claimed() {
        let db = fresh_in_memory_db().await;
        let first_directory = tempfile::tempdir().unwrap();
        let active_directory = tempfile::tempdir().unwrap();
        let first = ensure_active_vault(&db.conn, &first_directory.path().to_string_lossy())
            .await
            .unwrap();
        let old = insert_kind_job(
            &db.conn,
            &first.id,
            "turn_summary",
            "same-conversation-job",
            None,
            None,
            InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let active = ensure_active_vault(&db.conn, &active_directory.path().to_string_lossy())
            .await
            .unwrap();
        let new = insert_kind_job(
            &db.conn,
            &active.id,
            "turn_summary",
            "same-conversation-job",
            None,
            None,
            InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let claimed = claim_next_queued_job(&db.conn, &active.id, "turn_summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            claimed.id, new.id,
            "switching vaults must leave the old queue untouched"
        );
        assert!(claim_job_if_queued(&db.conn, &active.id, &old.id)
            .await
            .unwrap()
            .is_none());
        assert!(
            list_queued_jobs_by_kind(&db.conn, &active.id, "turn_summary")
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            get_job_model(&db.conn, &old.id).await.unwrap().status,
            "queued"
        );
    }

    #[tokio::test]
    async fn project_binding_is_stable_for_workspace_key() {
        let db = fresh_in_memory_db().await;
        let first = ensure_project_binding(&db.conn, "vault", "db", 42)
            .await
            .unwrap();
        let second = ensure_project_binding(&db.conn, "vault", "db", 42)
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        let other = ensure_project_binding(&db.conn, "vault", "db", 43)
            .await
            .unwrap();
        assert_ne!(first.id, other.id);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn physical_vault_identity_is_shared_by_symlink_aliases() {
        let db = fresh_in_memory_db().await;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let first = ensure_active_vault(&db.conn, &real.to_string_lossy())
            .await
            .unwrap();
        let second = ensure_active_vault(&db.conn, &alias.to_string_lossy())
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
    }
}

//! Wiki vault / source / job persistence. Mode-agnostic.

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::db::entities::{wiki_job, wiki_source, wiki_vault};
use crate::db::error::DbError;

#[derive(Debug, Clone)]
pub struct InsertedSource {
    pub source: wiki_source::Model,
    pub job: wiki_job::Model,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiJobInfo {
    pub id: String,
    pub vault_id: String,
    pub source_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub attempt: i32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
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
}

fn job_info(m: wiki_job::Model) -> WikiJobInfo {
    WikiJobInfo {
        id: m.id,
        vault_id: m.vault_id,
        source_id: m.source_id,
        kind: m.kind,
        status: m.status,
        attempt: m.attempt,
        error_code: m.error_code,
        error_message: m.error_message,
        created_at: m.created_at,
        updated_at: m.updated_at,
        started_at: m.started_at,
        finished_at: m.finished_at,
    }
}

fn source_info(m: wiki_source::Model) -> WikiSourceInfo {
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
    }
}

pub async fn ensure_active_vault(
    conn: &DatabaseConnection,
    canonical_path: &str,
) -> Result<wiki_vault::Model, DbError> {
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
}

/// Insert source + ingest job in one transaction. Idempotent on (vault_id, run_id).
pub async fn insert_acp_source_and_ingest_job(
    conn: &DatabaseConnection,
    new: NewAcpSource,
) -> Result<InsertedSource, DbError> {
    if let Some(existing) = find_source_by_run(conn, &new.vault_id, &new.run_id).await? {
        let job = find_ingest_job(conn, &existing.id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("ingest job for source {}", existing.id)))?;
        return Ok(InsertedSource {
            source: existing,
            job,
            created: false,
        });
    }

    let now = Utc::now();
    let source_id = uuid::Uuid::new_v4().to_string();
    let job_id = uuid::Uuid::new_v4().to_string();
    let dedupe_key = format!("{}:ingest:{}", new.vault_id, new.run_id);

    let txn = conn.begin().await?;
    if let Some(existing) = find_source_by_run(&txn, &new.vault_id, &new.run_id).await? {
        let job = find_ingest_job(&txn, &existing.id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("ingest job for source {}", existing.id)))?;
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
            let job = find_ingest_job(conn, &existing.id).await?.ok_or_else(|| {
                DbError::NotFound(format!("ingest job for source {}", existing.id))
            })?;
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
        kind: Set("ingest".into()),
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
        started_at: Set(None),
        finished_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let job_model = job.insert(&txn).await?;
    txn.commit().await?;
    Ok(InsertedSource {
        source: source_model,
        job: job_model,
        created: true,
    })
}

pub async fn find_source_by_run<C: ConnectionTrait>(
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

pub async fn find_ingest_job<C: ConnectionTrait>(
    conn: &C,
    source_id: &str,
) -> Result<Option<wiki_job::Model>, DbError> {
    Ok(wiki_job::Entity::find()
        .filter(wiki_job::Column::SourceId.eq(source_id))
        .filter(wiki_job::Column::Kind.eq("ingest"))
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

pub async fn mark_job(
    conn: &DatabaseConnection,
    job_id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), DbError> {
    let Some(row) = wiki_job::Entity::find_by_id(job_id).one(conn).await? else {
        return Err(DbError::NotFound(format!("wiki job {job_id}")));
    };
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
    active.update(conn).await?;
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
        kind: Set("ingest".into()),
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
        started_at: Set(None),
        finished_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(job.insert(conn).await?)
}

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

pub async fn get_job(conn: &DatabaseConnection, id: &str) -> Result<WikiJobInfo, DbError> {
    let row = wiki_job::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki job {id}")))?;
    Ok(job_info(row))
}

pub async fn list_sources(
    conn: &DatabaseConnection,
    limit: u64,
    offset: u64,
    source_kind: Option<&str>,
) -> Result<Vec<WikiSourceInfo>, DbError> {
    let mut q = wiki_source::Entity::find().order_by_desc(wiki_source::Column::SourceSeq);
    if let Some(kind) = source_kind.filter(|s| !s.is_empty()) {
        q = q.filter(wiki_source::Column::SourceKind.eq(kind));
    }
    let rows = q.offset(offset).limit(limit).all(conn).await?;
    Ok(rows.into_iter().map(source_info).collect())
}

pub async fn get_source(conn: &DatabaseConnection, id: &str) -> Result<WikiSourceInfo, DbError> {
    let row = wiki_source::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("wiki source {id}")))?;
    Ok(source_info(row))
}

pub async fn pending_source_count(conn: &DatabaseConnection) -> Result<u64, DbError> {
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::Eligibility.is_in(["processing", "ready"]))
        .count(conn)
        .await?)
}

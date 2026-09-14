//! Wiki 归纳批次的数据库收尾：把已生成页面、来源关联和工作笔记消费记录原子提交。
//!
//! compile 在 Markdown 文件提交成功后调用 finalize_batch；重启恢复也重用同一入口，
//! 防止文件已生成但数据库漏记，或同一批次重复增加关联。wiki_service 管理任务调度状态，
//! 本模块只管理归纳产物的持久化，不读取素材正文、不执行模型调用或文件操作。
use crate::db::entities::{
    wiki_contribution, wiki_job, wiki_job_batch, wiki_memory_consumption, wiki_source,
};
use crate::db::error::DbError;
use crate::wiki::commit::BatchMetadata;
use crate::wiki::result::JobOutputManifest;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    Set, TransactionTrait,
};

/// 去重范围包含资料库、笔记内容和契约版本；内容改变后允许再次进入归纳。
pub async fn memory_consumed(
    conn: &DatabaseConnection,
    vault_id: &str,
    input_ref: &str,
    hash: &str,
    contract: &str,
) -> Result<bool, DbError> {
    Ok(wiki_memory_consumption::Entity::find()
        .filter(wiki_memory_consumption::Column::VaultId.eq(vault_id))
        .filter(wiki_memory_consumption::Column::InputKind.eq("memory_note"))
        .filter(wiki_memory_consumption::Column::InputRef.eq(input_ref))
        .filter(wiki_memory_consumption::Column::ContentHash.eq(hash))
        .filter(wiki_memory_consumption::Column::ContractVersion.eq(contract))
        .one(conn)
        .await?
        .is_some())
}
pub async fn load_job_result<C: ConnectionTrait>(
    conn: &C,
    job_id: &str,
) -> Result<Option<JobOutputManifest>, DbError> {
    let row = wiki_job::Entity::find_by_id(job_id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("job {job_id}")))?;
    row.output_manifest
        .as_deref()
        .map(|s| {
            serde_json::from_str(s).map_err(|e| DbError::Validation(format!("job result: {e}")))
        })
        .transpose()
}
/// 同一批次只能对应同一提交内容，恢复流程据此判断是否已完成数据库收尾。
pub async fn batch_finalized<C: ConnectionTrait>(
    conn: &C,
    job_id: &str,
    attempt: i32,
    batch_id: &str,
    hash: &str,
) -> Result<bool, DbError> {
    let row = wiki_job_batch::Entity::find()
        .filter(wiki_job_batch::Column::JobId.eq(job_id))
        .filter(wiki_job_batch::Column::Attempt.eq(attempt))
        .filter(wiki_job_batch::Column::BatchId.eq(batch_id))
        .one(conn)
        .await?;
    match row {
        Some(row) if row.manifest_hash != hash => Err(DbError::Conflict(
            "batch manifest changed after finalization".into(),
        )),
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

/// 文件已经提交后一次性登记批次、输入消费、页面来源和任务聚合结果。
/// 任一步失败都回滚，恢复时可根据 commit manifest 重放整个批次。
pub async fn finalize_batch(
    conn: &DatabaseConnection,
    job_id: &str,
    batch: &BatchMetadata,
    manifest_path: &str,
    manifest_hash: &str,
) -> Result<JobOutputManifest, DbError> {
    let txn = conn.begin().await?;
    if batch_finalized(&txn, job_id, batch.attempt, &batch.batch_id, manifest_hash).await? {
        return load_job_result(&txn, job_id)
            .await?
            .ok_or_else(|| DbError::Validation("finalized batch has no job result".into()));
    }
    let job = wiki_job::Entity::find_by_id(job_id)
        .one(&txn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("job {job_id}")))?;
    if job.vault_id != batch.vault_id {
        return Err(DbError::Conflict(
            "batch vault differs from job vault".into(),
        ));
    }
    let now = Utc::now();
    // 先占用唯一批次键，再写消费和关联；数据库异常时一并回滚，避免重复计入同一提交。
    wiki_job_batch::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        job_id: Set(job_id.into()),
        attempt: Set(batch.attempt),
        batch_id: Set(batch.batch_id.clone()),
        manifest_path: Set(manifest_path.into()),
        manifest_hash: Set(manifest_hash.into()),
        result_json: Set(
            serde_json::to_string(&batch.result).map_err(|e| DbError::Validation(e.to_string()))?
        ),
        finalized_at: Set(now),
    }
    .insert(&txn)
    .await?;
    for input in &batch.result.processed_inputs {
        let consumed = wiki_memory_consumption::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            vault_id: Set(batch.vault_id.clone()),
            input_kind: Set("memory_note".into()),
            input_ref: Set(input.rel.clone()),
            content_hash: Set(input.content_hash.clone()),
            contract_version: Set(batch.contract_version.clone()),
            job_id: Set(job_id.into()),
            batch_id: Set(batch.batch_id.clone()),
            disposition: Set(input.disposition.clone()),
            created_at: Set(now),
        };
        wiki_memory_consumption::Entity::insert(consumed)
            .on_conflict(
                sea_orm::sea_query::OnConflict::columns([
                    wiki_memory_consumption::Column::VaultId,
                    wiki_memory_consumption::Column::InputKind,
                    wiki_memory_consumption::Column::InputRef,
                    wiki_memory_consumption::Column::ContentHash,
                    wiki_memory_consumption::Column::ContractVersion,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        for source_id in &input.source_ids {
            let source = wiki_source::Entity::find_by_id(source_id)
                .one(&txn)
                .await?
                .ok_or_else(|| DbError::Validation(format!("unknown source {source_id}")))?;
            if source.vault_id != batch.vault_id {
                return Err(DbError::Validation(
                    "source belongs to another vault".into(),
                ));
            }
            for output in &batch.result.outputs {
                // 只关联实际声明此来源的页面，避免把同批次其他页面误标为该资料的产出。
                if !batch
                    .source_ids_by_note
                    .get(&output.note_id)
                    .is_some_and(|ids| ids.contains(source_id))
                {
                    continue;
                }
                let id = crate::wiki::raw::content_hash(&format!(
                    "{}:{}:{}:{}",
                    manifest_hash, source_id, output.note_id, input.content_hash
                ));
                wiki_contribution::Entity::insert(wiki_contribution::ActiveModel {
                    id: Set(id),
                    source_id: Set(source_id.clone()),
                    raw_hash: Set(source.raw_hash.clone().unwrap_or_default()),
                    annotation_revision: Set(source.annotation_revision),
                    note_id: Set(output.note_id.clone()),
                    claim_id: Set(None),
                    evidence_id: Set(None),
                    case_id: Set(None),
                    commit_id: Set(Some(job_id.into())),
                    created_at: Set(now),
                })
                .on_conflict(
                    sea_orm::sea_query::OnConflict::column(wiki_contribution::Column::Id)
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
            }
        }
    }
    // 后续批次替换同一 note_id 的产出；已处理输入按内容版本去重，待处理项沿用最新批次。
    let mut aggregate = job
        .output_manifest
        .as_deref()
        .map(|s| {
            serde_json::from_str::<JobOutputManifest>(s)
                .map_err(|e| DbError::Validation(e.to_string()))
        })
        .transpose()?
        .unwrap_or_else(|| JobOutputManifest {
            version: 1,
            outcome: "partial".into(),
            reason_code: None,
            outputs: Vec::new(),
            processed_inputs: Vec::new(),
            remaining_inputs: Vec::new(),
            warnings: Vec::new(),
        });
    for output in &batch.result.outputs {
        aggregate.outputs.retain(|o| o.note_id != output.note_id);
        aggregate.outputs.push(output.clone());
    }
    for input in &batch.result.processed_inputs {
        if !aggregate
            .processed_inputs
            .iter()
            .any(|i| i.rel == input.rel && i.content_hash == input.content_hash)
        {
            aggregate.processed_inputs.push(input.clone());
        }
    }
    aggregate
        .warnings
        .extend(batch.result.warnings.iter().cloned());
    aggregate.warnings.sort();
    aggregate.warnings.dedup();
    aggregate.remaining_inputs = batch.result.remaining_inputs.clone();
    aggregate.outcome = if aggregate.remaining_inputs.is_empty() {
        if aggregate.outputs.is_empty() {
            "no_content"
        } else {
            "generated"
        }
    } else {
        "partial"
    }
    .into();
    aggregate.reason_code = if aggregate.outcome == "no_content" {
        Some("no_durable_content".into())
    } else {
        None
    };
    let mut active: wiki_job::ActiveModel = job.into();
    active.output_manifest = Set(Some(
        serde_json::to_string(&aggregate).map_err(|e| DbError::Validation(e.to_string()))?,
    ));
    active.updated_at = Set(now);
    active.update(&txn).await?;
    txn.commit().await?;
    Ok(aggregate)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::service::wiki_service;
    use crate::wiki::result::ProcessedInput;
    #[tokio::test]
    async fn consumption_is_vault_scoped_and_finalize_is_idempotent() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let dir = tempfile::tempdir().unwrap();
        let vault = wiki_service::ensure_active_vault(&db.conn, &dir.path().to_string_lossy())
            .await
            .unwrap();
        let job = wiki_service::insert_kind_job(
            &db.conn,
            &vault.id,
            "wiki_synthesize",
            "test-batch",
            None,
            None,
            wiki_service::InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let batch = BatchMetadata {
            vault_id: vault.id.clone(),
            attempt: 1,
            batch_id: "batch-1".into(),
            contract_version: "v2".into(),
            source_ids_by_note: std::collections::BTreeMap::new(),
            result: JobOutputManifest {
                version: 1,
                outcome: "no_content".into(),
                reason_code: Some("no_durable_content".into()),
                outputs: vec![],
                processed_inputs: vec![ProcessedInput {
                    rel: "work/turns/a.md".into(),
                    content_hash: "hash".into(),
                    disposition: "no_content".into(),
                    source_ids: vec![],
                }],
                remaining_inputs: vec![],
                warnings: vec![],
            },
        };
        finalize_batch(&db.conn, &job.id, &batch, "/manifest", "fixed-hash")
            .await
            .unwrap();
        finalize_batch(&db.conn, &job.id, &batch, "/manifest", "fixed-hash")
            .await
            .unwrap();
        assert!(
            memory_consumed(&db.conn, &vault.id, "work/turns/a.md", "hash", "v2")
                .await
                .unwrap()
        );
        assert!(
            !memory_consumed(&db.conn, "other-vault", "work/turns/a.md", "hash", "v2")
                .await
                .unwrap()
        );
        assert!(matches!(
            finalize_batch(&db.conn, &job.id, &batch, "/manifest", "different-hash").await,
            Err(DbError::Conflict(_))
        ));
        assert_eq!(
            wiki_memory_consumption::Entity::find()
                .all(&db.conn)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            wiki_job_batch::Entity::find()
                .all(&db.conn)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

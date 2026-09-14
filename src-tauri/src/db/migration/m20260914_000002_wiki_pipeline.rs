//! 建立 Wiki 归纳批次提交记录与素材处理去重表。
//! wiki_pipeline_service 使用唯一键将批次结果、贡献和处理状态原子登记，支持输出提交恢复。
//! 记录用于幂等与变更去重，不要求模型冻结输入正文或提交读取证明。

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE wiki_memory_consumption (
                id TEXT PRIMARY KEY NOT NULL,
                vault_id TEXT NOT NULL REFERENCES wiki_vault(id) ON DELETE CASCADE,
                input_kind TEXT NOT NULL,
                input_ref TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                contract_version TEXT NOT NULL,
                job_id TEXT NOT NULL REFERENCES wiki_job(id) ON DELETE CASCADE,
                batch_id TEXT NOT NULL,
                disposition TEXT NOT NULL CHECK (disposition IN ('used','no_content')),
                created_at TEXT NOT NULL,
                UNIQUE(vault_id,input_kind,input_ref,content_hash,contract_version)
            );
            CREATE TABLE wiki_job_batch (
                id TEXT PRIMARY KEY NOT NULL,
                job_id TEXT NOT NULL REFERENCES wiki_job(id) ON DELETE CASCADE,
                attempt INTEGER NOT NULL,
                batch_id TEXT NOT NULL,
                manifest_path TEXT NOT NULL,
                manifest_hash TEXT NOT NULL,
                result_json TEXT NOT NULL,
                finalized_at TEXT NOT NULL,
                UNIQUE(job_id,attempt,batch_id)
            );",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "DROP TABLE IF EXISTS wiki_job_batch; DROP TABLE IF EXISTS wiki_memory_consumption;"
        ).await?;
        Ok(())
    }
}

//! 建立 Wiki 逻辑任务每次执行的历史记录，保留重试前的结果与错误。
//! wiki_service 在认领、完成和重试时维护 attempt；读模型用于展示处理记录。
//! 逻辑任务与执行尝试分离，防止重试覆盖历史或重复增加活动任务统计。

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE wiki_job_attempt (
                id TEXT PRIMARY KEY NOT NULL,
                job_id TEXT NOT NULL REFERENCES wiki_job(id) ON DELETE CASCADE,
                attempt INTEGER NOT NULL,
                status TEXT NOT NULL,
                input_manifest TEXT,
                output_manifest TEXT,
                model_id TEXT,
                protocol TEXT,
                error_code TEXT,
                error_message TEXT,
                started_at TEXT,
                finished_at TEXT,
                UNIQUE(job_id, attempt)
            );",
            )
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS wiki_job_attempt")
            .await?;
        Ok(())
    }
}

//! 为 Wiki 文件导入增加来源标注、提取状态、原件信息及批次字段。
//! 现行 import 服务继续使用其中的来源字段；旧分段结构由后续 v2 reset 迁移移除。
//! 保留迁移顺序以支持已有安装升级，不在代码清理时修改历史数据库结构。

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_nullable_string(manager, WikiSource::RequestId).await?;
        add_nullable_string(manager, WikiSource::OriginalFilename).await?;
        add_nullable_string(manager, WikiSource::Format).await?;
        add_nullable_string(manager, WikiSource::SourceTitle).await?;
        add_nullable_text(manager, WikiSource::SourceUrl).await?;
        add_nullable_text(manager, WikiSource::Author).await?;
        add_nullable_text(manager, WikiSource::ProjectIds).await?;
        add_nullable_text(manager, WikiSource::AreaIds).await?;
        add_nullable_text(manager, WikiSource::Warnings).await?;
        manager
            .alter_table(
                Table::alter()
                    .table(WikiSource::Table)
                    .add_column(ColumnDef::new(WikiSource::PageCount).integer().null())
                    .to_owned(),
            )
            .await?;
        add_nullable_string(manager, WikiSource::PreviousSourceId).await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_wiki_source_vault_request \
                 ON wiki_source (vault_id, request_id) WHERE request_id IS NOT NULL",
            )
            .await?;
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_wiki_source_vault_original_hash \
                 ON wiki_source (vault_id, original_hash) WHERE original_hash IS NOT NULL",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS idx_wiki_source_vault_request")
            .await?;
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS idx_wiki_source_vault_original_hash")
            .await?;
        for col in [
            WikiSource::RequestId,
            WikiSource::OriginalFilename,
            WikiSource::Format,
            WikiSource::SourceTitle,
            WikiSource::SourceUrl,
            WikiSource::Author,
            WikiSource::ProjectIds,
            WikiSource::AreaIds,
            WikiSource::Warnings,
            WikiSource::PageCount,
            WikiSource::PreviousSourceId,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(WikiSource::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

async fn add_nullable_string(manager: &SchemaManager<'_>, col: WikiSource) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(WikiSource::Table)
                .add_column(ColumnDef::new(col).string().null())
                .to_owned(),
        )
        .await
}

async fn add_nullable_text(manager: &SchemaManager<'_>, col: WikiSource) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(WikiSource::Table)
                .add_column(ColumnDef::new(col).text().null())
                .to_owned(),
        )
        .await
}

#[derive(Iden)]
enum WikiSource {
    Table,
    RequestId,
    OriginalFilename,
    Format,
    SourceTitle,
    SourceUrl,
    Author,
    ProjectIds,
    AreaIds,
    Warnings,
    PageCount,
    PreviousSourceId,
}

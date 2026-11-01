//! Feishu message id that anchors one topic for a folder↔channel binding.
//! NULL means the folder has not entered a topic yet, so replies keep the
//! per-message route.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(FolderChatChannel::Table)
                    .add_column(
                        ColumnDef::new(FolderChatChannel::TopicMessageId)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(FolderChatChannel::Table)
                    .drop_column(FolderChatChannel::TopicMessageId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum FolderChatChannel {
    Table,
    TopicMessageId,
}

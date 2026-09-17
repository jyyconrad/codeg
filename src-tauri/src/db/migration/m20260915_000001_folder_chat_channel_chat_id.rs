//! Optional per-folder chat/session id override on a folder↔channel binding.
//! NULL/empty means the channel's configured default chat_id.

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
                    .add_column(ColumnDef::new(FolderChatChannel::ChatId).string().null())
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
                    .drop_column(FolderChatChannel::ChatId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum FolderChatChannel {
    Table,
    ChatId,
}

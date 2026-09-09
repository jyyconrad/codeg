use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One folder may bind many chat channels for run-end fan-out. Worktree
        // folders store their own rows; readers do not walk parent_id.
        manager
            .create_table(
                Table::create()
                    .table(FolderChatChannel::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(FolderChatChannel::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(FolderChatChannel::FolderId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FolderChatChannel::ChannelId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FolderChatChannel::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_fcc_folder_id")
                            .from(FolderChatChannel::Table, FolderChatChannel::FolderId)
                            .to(Folder::Table, Folder::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_fcc_channel_id")
                            .from(FolderChatChannel::Table, FolderChatChannel::ChannelId)
                            .to(ChatChannel::Table, ChatChannel::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_folder_chat_channel_folder_id")
                    .table(FolderChatChannel::Table)
                    .col(FolderChatChannel::FolderId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_folder_chat_channel_folder_id_channel_id")
                    .table(FolderChatChannel::Table)
                    .col(FolderChatChannel::FolderId)
                    .col(FolderChatChannel::ChannelId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(FolderChatChannel::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum FolderChatChannel {
    Table,
    Id,
    FolderId,
    ChannelId,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Folder {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum ChatChannel {
    Table,
    Id,
}

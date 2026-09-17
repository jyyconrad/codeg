//! Maps provider (Lark/Telegram/…) message ids to Codeg conversations so a
//! quoted/replied IM message can be routed back to the session that produced it.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatChannelMessageMap::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::ChannelId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::ProviderMessageId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::ConversationId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ChatChannelMessageMap::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_ccmm_channel_id")
                            .from(
                                ChatChannelMessageMap::Table,
                                ChatChannelMessageMap::ChannelId,
                            )
                            .to(ChatChannel::Table, ChatChannel::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_ccmm_conversation_id")
                            .from(
                                ChatChannelMessageMap::Table,
                                ChatChannelMessageMap::ConversationId,
                            )
                            .to(Conversation::Table, Conversation::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_ccmm_channel_provider_message")
                    .table(ChatChannelMessageMap::Table)
                    .col(ChatChannelMessageMap::ChannelId)
                    .col(ChatChannelMessageMap::ProviderMessageId)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatChannelMessageMap::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ChatChannelMessageMap {
    Table,
    Id,
    ChannelId,
    ProviderMessageId,
    ConversationId,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ChatChannel {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Conversation {
    Table,
    Id,
}

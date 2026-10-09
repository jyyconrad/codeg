//! Channel default agent, and one channel session per folder.
//!
//! `default_agent_type` is the agent that handles an inbound message when the
//! sender did not quote an existing conversation. Existing folder bindings are
//! collapsed to one row per folder and one folder per channel session.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ChatChannel::Table)
                    .add_column(
                        ColumnDef::new(ChatChannel::DefaultAgentType)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        let conn = manager.get_connection();
        conn.execute_unprepared(
            "DELETE FROM folder_chat_channel \
             WHERE id NOT IN ( \
               SELECT id FROM ( \
                 SELECT MAX(id) AS id FROM folder_chat_channel GROUP BY folder_id \
               ) \
             )",
        )
        .await?;
        conn.execute_unprepared(
            "DELETE FROM folder_chat_channel \
             WHERE id NOT IN ( \
               SELECT id FROM ( \
                 SELECT MAX(id) AS id FROM folder_chat_channel \
                 GROUP BY channel_id, COALESCE(NULLIF(TRIM(chat_id), ''), '') \
               ) \
             )",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ChatChannel::Table)
                    .drop_column(ChatChannel::DefaultAgentType)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum ChatChannel {
    Table,
    DefaultAgentType,
}

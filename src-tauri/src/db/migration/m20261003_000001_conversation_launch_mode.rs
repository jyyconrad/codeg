use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Conversation::Table)
                    .add_column(
                        ColumnDef::new(Conversation::LaunchMode)
                            .string()
                            .not_null()
                            .default("acp"),
                    )
                    .to_owned(),
            )
            .await?;
        // Rows created by a terminal CLI never import a transcript, so they
        // stay at message_count 0 with a native session id. ACP history has
        // messages. This one-time backfill is what lets those existing list
        // rows reopen in the CLI. New rows are stamped explicitly.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE conversation SET launch_mode = 'terminal' \
                 WHERE message_count = 0 AND external_id IS NOT NULL \
                   AND agent_type IN ('grok', 'codex', 'claude_code') \
                   AND deleted_at IS NULL",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Conversation::Table)
                    .drop_column(Conversation::LaunchMode)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Conversation {
    Table,
    LaunchMode,
}

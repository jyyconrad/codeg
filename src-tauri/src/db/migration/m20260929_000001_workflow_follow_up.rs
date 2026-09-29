//! One workflow follow-up delivery per (conversation, external session, run).
//! Reconnect and replay read this row so a completion prompt is not sent twice.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(WorkflowFollowUp::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(WorkflowFollowUp::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::ConversationId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::ExternalSessionId)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(WorkflowFollowUp::RunId).string().not_null())
                    .col(
                        ColumnDef::new(WorkflowFollowUp::AgentType)
                            .string()
                            .not_null(),
                    )
                    // pending | claimed | sent | skipped | failed
                    .col(
                        ColumnDef::new(WorkflowFollowUp::DeliveryStatus)
                            .string()
                            .not_null(),
                    )
                    // completed | failed — workflow outcome, not the delivery state.
                    .col(
                        ColumnDef::new(WorkflowFollowUp::TerminalStatus)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::WorkflowName)
                            .string()
                            .not_null(),
                    )
                    // final_summary | error_detail | unavailable
                    .col(
                        ColumnDef::new(WorkflowFollowUp::ResultSource)
                            .string()
                            .not_null(),
                    )
                    // Caller truncates; may be ~24KiB. Service logs must not print it.
                    .col(
                        ColumnDef::new(WorkflowFollowUp::EvidenceText)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::FailureReason)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(WorkflowFollowUp::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_workflow_follow_up_conversation")
                            .from(WorkflowFollowUp::Table, WorkflowFollowUp::ConversationId)
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
                    .name("uq_workflow_follow_up_delivery")
                    .table(WorkflowFollowUp::Table)
                    .col(WorkflowFollowUp::ConversationId)
                    .col(WorkflowFollowUp::ExternalSessionId)
                    .col(WorkflowFollowUp::RunId)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(WorkflowFollowUp::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum WorkflowFollowUp {
    Table,
    Id,
    ConversationId,
    ExternalSessionId,
    RunId,
    AgentType,
    DeliveryStatus,
    TerminalStatus,
    WorkflowName,
    ResultSource,
    EvidenceText,
    FailureReason,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Conversation {
    Table,
    Id,
}

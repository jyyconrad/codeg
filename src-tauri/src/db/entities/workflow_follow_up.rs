//! One workflow follow-up delivery per (conversation, external session, run).
//!
//! Evidence may change only while `pending`. `claimed` stays claimed across a
//! crash so recovery does not send twice. `sent`, `skipped`, and `failed` are
//! not moved back to `pending`.

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Delivery state for one workflow follow-up.
///
/// `pending` may refresh evidence. `claimed` is the crash-sticky send lock.
/// The only rewind is `claimed` → `pending`, and only before the prompt is queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "claimed")]
    Claimed,
    #[sea_orm(string_value = "sent")]
    Sent,
    #[sea_orm(string_value = "skipped")]
    Skipped,
    #[sea_orm(string_value = "failed")]
    Failed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "workflow_follow_up")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub conversation_id: i32,
    pub external_session_id: String,
    pub run_id: String,
    pub agent_type: String,
    pub delivery_status: DeliveryStatus,
    /// `completed` | `failed` — the workflow outcome, not [`DeliveryStatus`].
    pub terminal_status: String,
    pub workflow_name: String,
    /// `final_summary` | `error_detail` | `unavailable`.
    pub result_source: String,
    /// Already truncated by the caller. Do not log this column.
    #[sea_orm(column_type = "Text")]
    pub evidence_text: String,
    pub failure_reason: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::conversation::Entity",
        from = "Column::ConversationId",
        to = "super::conversation::Column::Id"
    )]
    Conversation,
}

impl Related<super::conversation::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Conversation.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

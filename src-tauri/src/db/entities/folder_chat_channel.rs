use sea_orm::entity::prelude::*;

/// Binding of one workspace folder to one channel session.
///
/// A folder has at most one row. A `(channel_id, chat_id)` pair belongs to at
/// most one folder. `chat_id` `None` (or blank) uses the channel's configured
/// default chat. Worktree folders bind their own row (no parent walk).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "folder_chat_channel")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub folder_id: i32,
    pub channel_id: i32,
    /// Optional destination chat/session id for this folder on this channel.
    /// `None` (or blank) uses the channel's configured default.
    pub chat_id: Option<String>,
    /// Feishu message that anchors this folder's single topic. Replies use
    /// `reply_in_thread` against it. Not part of the binding dialog.
    pub topic_message_id: Option<String>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::folder::Entity",
        from = "Column::FolderId",
        to = "super::folder::Column::Id"
    )]
    Folder,
    #[sea_orm(
        belongs_to = "super::chat_channel::Entity",
        from = "Column::ChannelId",
        to = "super::chat_channel::Column::Id"
    )]
    ChatChannel,
}

impl Related<super::folder::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Folder.def()
    }
}

impl Related<super::chat_channel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ChatChannel.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

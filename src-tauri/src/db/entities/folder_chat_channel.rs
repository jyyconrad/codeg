use sea_orm::entity::prelude::*;

/// Binding of a workspace folder to a chat channel. One folder may notify many
/// channels; worktree folders bind their own rows (no parent walk).
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

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait, Set,
};

use crate::db::entities::conversation::ConversationKind;
use crate::db::entities::{chat_channel_message_map, conversation};
use crate::db::error::DbError;

fn normalize_provider_message_id(provider_message_id: &str) -> Option<String> {
    let trimmed = provider_message_id.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub async fn upsert(
    conn: &DatabaseConnection,
    channel_id: i32,
    provider_message_id: &str,
    conversation_id: i32,
) -> Result<(), DbError> {
    let Some(provider_message_id) = normalize_provider_message_id(provider_message_id) else {
        return Ok(());
    };
    let now = Utc::now();
    if let Some(existing) = get(conn, channel_id, &provider_message_id).await? {
        if existing.conversation_id == conversation_id {
            return Ok(());
        }
        let mut active = existing.into_active_model();
        active.conversation_id = Set(conversation_id);
        active.updated_at = Set(now);
        active.update(conn).await?;
        return Ok(());
    }

    let active = chat_channel_message_map::ActiveModel {
        id: NotSet,
        channel_id: Set(channel_id),
        provider_message_id: Set(provider_message_id),
        conversation_id: Set(conversation_id),
        created_at: Set(now),
        updated_at: Set(now),
    };
    active.insert(conn).await?;
    Ok(())
}

pub async fn get(
    conn: &DatabaseConnection,
    channel_id: i32,
    provider_message_id: &str,
) -> Result<Option<chat_channel_message_map::Model>, DbError> {
    let Some(provider_message_id) = normalize_provider_message_id(provider_message_id) else {
        return Ok(None);
    };
    Ok(chat_channel_message_map::Entity::find()
        .filter(chat_channel_message_map::Column::ChannelId.eq(channel_id))
        .filter(chat_channel_message_map::Column::ProviderMessageId.eq(provider_message_id))
        .one(conn)
        .await?)
}

pub async fn conversation_id_for(
    conn: &DatabaseConnection,
    channel_id: i32,
    provider_message_id: &str,
) -> Result<Option<i32>, DbError> {
    Ok(get(conn, channel_id, provider_message_id)
        .await?
        .map(|row| row.conversation_id))
}

/// Latest conversation this channel has already spoken about in `folder_id`.
///
/// Used so an unquoted Feishu/Telegram follow-up continues the session that
/// was last pushed to the chat, not a newer desktop session in the same folder.
pub async fn latest_conversation_in_folder(
    conn: &DatabaseConnection,
    channel_id: i32,
    folder_id: i32,
) -> Result<Option<i32>, DbError> {
    Ok(chat_channel_message_map::Entity::find()
        .join(
            JoinType::InnerJoin,
            chat_channel_message_map::Relation::Conversation.def(),
        )
        .filter(chat_channel_message_map::Column::ChannelId.eq(channel_id))
        .filter(conversation::Column::FolderId.eq(folder_id))
        .filter(conversation::Column::DeletedAt.is_null())
        .filter(conversation::Column::ParentId.is_null())
        .filter(conversation::Column::Kind.eq(ConversationKind::Regular))
        .order_by_desc(chat_channel_message_map::Column::UpdatedAt)
        .order_by_desc(chat_channel_message_map::Column::Id)
        .one(conn)
        .await?
        .map(|row| row.conversation_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::service::chat_channel_service;
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::AgentType;

    async fn seed_channel(db: &crate::db::AppDatabase) -> i32 {
        chat_channel_service::create(
            &db.conn,
            "map test".to_string(),
            "lark".to_string(),
            "{}".to_string(),
            true,
            false,
            None,
        )
        .await
        .expect("seed chat channel")
        .id
    }

    #[tokio::test]
    async fn upsert_then_get_round_trips_conversation_id() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/ccmm").await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_channel(&db).await;

        upsert(&db.conn, channel_id, "om_parent", conv_id)
            .await
            .unwrap();

        assert_eq!(
            conversation_id_for(&db.conn, channel_id, "om_parent")
                .await
                .unwrap(),
            Some(conv_id)
        );
        assert_eq!(
            conversation_id_for(&db.conn, channel_id, "om_missing")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn upsert_replaces_conversation_for_same_provider_id() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/ccmm-replace").await;
        let first = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let second = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_channel(&db).await;

        upsert(&db.conn, channel_id, "om_1", first).await.unwrap();
        upsert(&db.conn, channel_id, "om_1", second).await.unwrap();

        assert_eq!(
            conversation_id_for(&db.conn, channel_id, "om_1")
                .await
                .unwrap(),
            Some(second)
        );
    }

    #[tokio::test]
    async fn upsert_ignores_blank_provider_message_id() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/ccmm-blank").await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_channel(&db).await;

        upsert(&db.conn, channel_id, "  ", conv_id).await.unwrap();
        assert_eq!(
            conversation_id_for(&db.conn, channel_id, "  ")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn latest_conversation_in_folder_returns_most_recent_mapped() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/ccmm-latest").await;
        let other_folder = seed_folder(&db, "/tmp/ccmm-other").await;
        let older = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let newer = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let other = seed_conversation(&db, other_folder, AgentType::Grok).await;
        let channel_id = seed_channel(&db).await;

        upsert(&db.conn, channel_id, "om_old", older).await.unwrap();
        upsert(&db.conn, channel_id, "om_new", newer).await.unwrap();
        upsert(&db.conn, channel_id, "om_other", other)
            .await
            .unwrap();

        assert_eq!(
            latest_conversation_in_folder(&db.conn, channel_id, folder_id)
                .await
                .unwrap(),
            Some(newer)
        );
    }
}

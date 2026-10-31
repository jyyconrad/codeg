use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, Set,
};

use crate::chat_channel::types::ChannelMessageTarget;
use crate::db::entities::{chat_channel, chat_channel_thread_binding};
use crate::db::error::DbError;

/// A conversation in an ordinary IM chat, not a provider-side forum topic.
pub const CHANNEL_CHAT_ROUTE_KIND: &str = "channel_chat";

/// Keep a chat-mode conversation's reply address across turns and restarts.
/// Its row id is an internal route key and must never be sent as a topic id.
pub async fn set_chat_conversation_target(
    conn: &DatabaseConnection,
    target: &ChannelMessageTarget,
    conversation_id: i32,
    sender_id: &str,
) -> Result<(), DbError> {
    let channel = chat_channel::Entity::find_by_id(target.channel_id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("chat channel {}", target.channel_id)))?;
    let route = ChannelMessageTarget {
        channel_id: target.channel_id,
        chat_id: Some(target.chat_id.clone().unwrap_or_default()),
        thread_key: Some(conversation_id.to_string()),
        thread_kind: Some(CHANNEL_CHAT_ROUTE_KIND.to_string()),
        provider_payload: target.provider_payload.clone(),
    };
    upsert_for_target(
        conn,
        &route,
        &channel.channel_type,
        conversation_id,
        None,
        sender_id,
        None,
    )
    .await?;
    Ok(())
}

pub async fn chat_conversation_target(
    conn: &DatabaseConnection,
    conversation_id: i32,
) -> Result<Option<ChannelMessageTarget>, DbError> {
    let Some(route) = chat_channel_thread_binding::Entity::find()
        .filter(chat_channel_thread_binding::Column::ConversationId.eq(conversation_id))
        .filter(chat_channel_thread_binding::Column::ThreadKind.eq(CHANNEL_CHAT_ROUTE_KIND))
        .one(conn)
        .await?
    else {
        return Ok(None);
    };
    let provider_payload = route
        .provider_payload_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| DbError::Validation(format!("invalid channel chat target: {e}")))?;
    Ok(Some(ChannelMessageTarget {
        channel_id: route.channel_id,
        chat_id: (!route.chat_id.is_empty()).then_some(route.chat_id),
        thread_key: None,
        thread_kind: None,
        provider_payload,
    }))
}

pub async fn get_by_target(
    conn: &DatabaseConnection,
    target: &ChannelMessageTarget,
) -> Result<Option<chat_channel_thread_binding::Model>, DbError> {
    let Some(chat_id) = target.chat_id.as_deref() else {
        return Ok(None);
    };
    let Some(thread_key) = target.thread_key.as_deref() else {
        return Ok(None);
    };
    let Some(thread_kind) = target.thread_kind.as_deref() else {
        return Ok(None);
    };

    Ok(chat_channel_thread_binding::Entity::find()
        .filter(chat_channel_thread_binding::Column::ChannelId.eq(target.channel_id))
        .filter(chat_channel_thread_binding::Column::ThreadKind.eq(thread_kind))
        .filter(chat_channel_thread_binding::Column::ChatId.eq(chat_id))
        .filter(chat_channel_thread_binding::Column::ThreadKey.eq(thread_key))
        .one(conn)
        .await?)
}

pub async fn list_by_conversation(
    conn: &DatabaseConnection,
    conversation_id: i32,
) -> Result<Vec<chat_channel_thread_binding::Model>, DbError> {
    Ok(chat_channel_thread_binding::Entity::find()
        .filter(chat_channel_thread_binding::Column::ConversationId.eq(conversation_id))
        .all(conn)
        .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_for_target(
    conn: &DatabaseConnection,
    target: &ChannelMessageTarget,
    channel_type: &str,
    conversation_id: i32,
    connection_id: Option<String>,
    created_by_sender_id: &str,
    display_title: Option<String>,
) -> Result<chat_channel_thread_binding::Model, DbError> {
    let chat_id = target
        .chat_id
        .as_deref()
        .ok_or_else(|| DbError::Validation("thread binding requires chat_id".to_string()))?;
    let thread_key = target
        .thread_key
        .as_deref()
        .ok_or_else(|| DbError::Validation("thread binding requires thread_key".to_string()))?;
    let thread_kind = target
        .thread_kind
        .as_deref()
        .ok_or_else(|| DbError::Validation("thread binding requires thread_kind".to_string()))?;
    let now = Utc::now();

    if let Some(existing) = get_by_target(conn, target).await? {
        let mut active = existing.into_active_model();
        active.channel_type = Set(channel_type.to_string());
        active.conversation_id = Set(conversation_id);
        active.connection_id = Set(connection_id);
        active.created_by_sender_id = Set(created_by_sender_id.to_string());
        active.display_title = Set(display_title);
        active.updated_at = Set(now);
        return Ok(active.update(conn).await?);
    }

    let active = chat_channel_thread_binding::ActiveModel {
        id: NotSet,
        channel_id: Set(target.channel_id),
        channel_type: Set(channel_type.to_string()),
        chat_id: Set(chat_id.to_string()),
        thread_key: Set(thread_key.to_string()),
        thread_kind: Set(thread_kind.to_string()),
        conversation_id: Set(conversation_id),
        connection_id: Set(connection_id),
        created_by_sender_id: Set(created_by_sender_id.to_string()),
        display_title: Set(display_title),
        title_sync_enabled: Set(true),
        provider_payload_json: Set(target
            .provider_payload
            .as_ref()
            .map(serde_json::Value::to_string)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(active.insert(conn).await?)
}

pub async fn update_display_title(
    conn: &DatabaseConnection,
    id: i32,
    display_title: String,
) -> Result<chat_channel_thread_binding::Model, DbError> {
    let model = chat_channel_thread_binding::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("thread binding {id}")))?;
    let mut active = model.into_active_model();
    active.display_title = Set(Some(display_title));
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

pub async fn clear_connection(
    conn: &DatabaseConnection,
    id: i32,
) -> Result<chat_channel_thread_binding::Model, DbError> {
    let model = chat_channel_thread_binding::Entity::find_by_id(id)
        .one(conn)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("thread binding {id}")))?;
    let mut active = model.into_active_model();
    active.connection_id = Set(None);
    active.updated_at = Set(Utc::now());
    Ok(active.update(conn).await?)
}

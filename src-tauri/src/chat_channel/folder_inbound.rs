use chrono::{DateTime, Duration, TimeZone, Utc};
use sea_orm::DatabaseConnection;

use super::types::{ChannelMessageTarget, IncomingCommand};
use crate::db::error::DbError;
use crate::db::service::{
    app_metadata_service, chat_channel_message_map_service, conversation_service,
    folder_chat_channel_service,
};

pub const FOLDER_INBOUND_IDLE_KEY: &str = "chat_folder_inbound_idle_minutes";
pub const DEFAULT_FOLDER_INBOUND_IDLE_MINUTES: i64 = 30;
pub const MAX_FOLDER_INBOUND_IDLE_MINUTES: i64 = 7 * 24 * 60;

pub fn parse_folder_inbound_idle_minutes(raw: Option<&str>) -> i64 {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_FOLDER_INBOUND_IDLE_MINUTES;
    };
    match value.parse::<i64>() {
        Ok(minutes) if (0..=MAX_FOLDER_INBOUND_IDLE_MINUTES).contains(&minutes) => minutes,
        _ => DEFAULT_FOLDER_INBOUND_IDLE_MINUTES,
    }
}

pub fn folder_inbound_idle_duration(minutes: i64) -> Duration {
    Duration::minutes(minutes.max(0))
}

pub async fn load_folder_inbound_idle(db: &DatabaseConnection) -> Duration {
    let raw = app_metadata_service::get_value(db, FOLDER_INBOUND_IDLE_KEY)
        .await
        .ok()
        .flatten();
    folder_inbound_idle_duration(parse_folder_inbound_idle_minutes(raw.as_deref()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderInboundAction {
    Continue { conversation_id: i32 },
    StartNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderInboundPlan {
    pub bound_folder_id: i32,
    pub action: FolderInboundAction,
}

/// A quote that maps to a conversation continues that conversation. Anything
/// else, including an unquoted follow-up in a folder chat, starts a new one.
/// The channel's default agent is chosen by the caller.
pub fn decide_folder_inbound(quoted_conversation_id: Option<i32>) -> FolderInboundAction {
    match quoted_conversation_id {
        Some(conversation_id) => FolderInboundAction::Continue { conversation_id },
        None => FolderInboundAction::StartNew,
    }
}

pub fn inbound_chat_id(cmd: &IncomingCommand) -> Option<String> {
    nonempty(cmd.target.chat_id.as_deref())
        .or_else(|| json_str(cmd.metadata.pointer("/event/message/chat_id")))
        .or_else(|| json_scalar(cmd.metadata.pointer("/message/chat/id")))
        .or_else(|| json_scalar(cmd.metadata.pointer("/chat/id")))
}

pub fn inbound_quoted_message_id(cmd: &IncomingCommand) -> Option<String> {
    inbound_quoted_message_ids(cmd).into_iter().next()
}

pub fn inbound_quoted_message_ids(cmd: &IncomingCommand) -> Vec<String> {
    let self_id = inbound_provider_message_id(cmd);
    let mut ids = Vec::new();
    let push = |ids: &mut Vec<String>, id: Option<String>| {
        let Some(id) = id else {
            return;
        };
        if self_id.as_deref() == Some(id.as_str()) {
            return;
        }
        if ids.iter().any(|existing| existing == &id) {
            return;
        }
        ids.push(id);
    };
    push(&mut ids, nonempty(cmd.quoted_message_id.as_deref()));
    push(
        &mut ids,
        json_str(cmd.metadata.pointer("/event/message/parent_id")),
    );
    push(
        &mut ids,
        json_str(cmd.metadata.pointer("/event/message/quote/message_id")),
    );
    push(
        &mut ids,
        json_str(cmd.metadata.pointer("/event/message/quote/id")),
    );
    push(
        &mut ids,
        json_scalar(cmd.metadata.pointer("/message/reply_to_message/message_id")),
    );
    push(
        &mut ids,
        json_scalar(cmd.metadata.pointer("/reply_to_message/message_id")),
    );
    push(
        &mut ids,
        json_str(cmd.metadata.pointer("/event/message/root_id")),
    );
    ids
}

pub fn inbound_provider_message_id(cmd: &IncomingCommand) -> Option<String> {
    nonempty(cmd.provider_message_id.as_deref())
        .or_else(|| json_str(cmd.metadata.pointer("/event/message/message_id")))
        .or_else(|| json_scalar(cmd.metadata.pointer("/message/message_id")))
        .or_else(|| json_scalar(cmd.metadata.get("message_id")))
}

pub fn inbound_message_time(cmd: &IncomingCommand) -> DateTime<Utc> {
    if let Some(ms) = parse_unix_ms(cmd.metadata.pointer("/event/message/create_time")) {
        return ms;
    }
    if let Some(secs) = parse_unix_secs(cmd.metadata.pointer("/message/date")) {
        return secs;
    }
    if let Some(secs) = parse_unix_secs(cmd.metadata.get("date")) {
        return secs;
    }
    Utc::now()
}

pub async fn route_folder_inbound(
    db: &DatabaseConnection,
    cmd: &IncomingCommand,
) -> Result<Option<FolderInboundPlan>, DbError> {
    // A quote of a card we sent is the session, including when the card was
    // delivered to the bot DM and the folder's configured chat is the group.
    // Requiring the inbound chat to match the folder first dropped those
    // replies before the message map was consulted.
    for quoted_id in inbound_quoted_message_ids(cmd) {
        let Some(conversation_id) =
            chat_channel_message_map_service::conversation_id_for(db, cmd.channel_id, &quoted_id)
                .await?
        else {
            continue;
        };
        if let Some(folder_id) = conversation_folder(db, conversation_id).await? {
            return Ok(Some(FolderInboundPlan {
                bound_folder_id: folder_id,
                action: FolderInboundAction::Continue { conversation_id },
            }));
        }
    }

    let chat_id = inbound_chat_id(cmd);
    let Some(folder_id) = folder_chat_channel_service::find_folder_id_for_inbound(
        db,
        cmd.channel_id,
        chat_id.as_deref(),
    )
    .await?
    else {
        return Ok(None);
    };

    Ok(Some(FolderInboundPlan {
        bound_folder_id: folder_id,
        action: decide_folder_inbound(None),
    }))
}

async fn conversation_folder(
    db: &DatabaseConnection,
    conversation_id: i32,
) -> Result<Option<i32>, DbError> {
    match conversation_service::get_by_id(db, conversation_id).await {
        Ok(conv) => Ok(Some(conv.folder_id)),
        Err(DbError::NotFound(_)) | Err(DbError::Migration(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn json_str(value: Option<&serde_json::Value>) -> Option<String> {
    nonempty(value.and_then(|v| v.as_str()))
}

fn json_scalar(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if let Some(s) = value.as_str() {
        return nonempty(Some(s));
    }
    if let Some(i) = value.as_i64() {
        return Some(i.to_string());
    }
    value.as_u64().map(|u| u.to_string())
}

fn parse_unix_ms(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let ms = match value? {
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok()?,
        serde_json::Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    Utc.timestamp_millis_opt(ms).single()
}

fn parse_unix_secs(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let secs = match value? {
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok()?,
        serde_json::Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    Utc.timestamp_opt(secs, 0).single()
}

pub async fn remember_provider_message(
    db: &DatabaseConnection,
    channel_id: i32,
    provider_message_id: Option<&str>,
    conversation_id: i32,
) {
    let Some(provider_message_id) = provider_message_id.map(str::trim).filter(|s| !s.is_empty())
    else {
        return;
    };
    if let Err(e) = chat_channel_message_map_service::upsert(
        db,
        channel_id,
        provider_message_id,
        conversation_id,
    )
    .await
    {
        tracing::warn!(
            channel_id,
            conversation_id,
            error = %e,
            "[ChatChannel] failed to remember provider message id"
        );
    }
}

pub async fn remember_sent_message(
    db: &DatabaseConnection,
    target: &ChannelMessageTarget,
    sent_id: &str,
    conversation_id: i32,
) {
    remember_provider_message(db, target.channel_id, Some(sent_id), conversation_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::conversation;
    use crate::db::service::{
        app_metadata_service, chat_channel_service, folder_chat_channel_service,
    };
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::AgentType;
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

    fn lark_cmd(channel_id: i32, event: serde_json::Value) -> IncomingCommand {
        let chat_id = event
            .pointer("/event/message/chat_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let mut cmd = IncomingCommand::plain(
            channel_id,
            "ou_sender",
            "follow up",
            match chat_id {
                Some(id) => ChannelMessageTarget::with_chat_id(channel_id, id),
                None => ChannelMessageTarget::channel(channel_id),
            },
        );
        cmd.metadata = event;
        cmd
    }

    async fn seed_lark_channel(db: &crate::db::AppDatabase) -> i32 {
        chat_channel_service::create(
            &db.conn,
            "lark".to_string(),
            "lark".to_string(),
            serde_json::json!({ "chat_id": "oc_default" }).to_string(),
            true,
            false,
            None,
        )
        .await
        .expect("seed chat channel")
        .id
    }

    async fn bind_folder(
        db: &crate::db::AppDatabase,
        folder_id: i32,
        channel_id: i32,
        chat_id: Option<&str>,
    ) {
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: chat_id.map(str::to_string),
            }],
        )
        .await
        .expect("bind folder");
    }

    async fn set_updated_at(db: &crate::db::AppDatabase, conversation_id: i32, at: DateTime<Utc>) {
        let conv = conversation::Entity::find_by_id(conversation_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let mut active = conv.into_active_model();
        active.updated_at = Set(at);
        active.update(&db.conn).await.unwrap();
    }

    #[test]
    fn quote_continues_that_conversation() {
        assert_eq!(
            decide_folder_inbound(Some(9)),
            FolderInboundAction::Continue { conversation_id: 9 }
        );
    }

    #[test]
    fn no_quote_starts_new() {
        assert_eq!(decide_folder_inbound(None), FolderInboundAction::StartNew);
    }

    #[test]
    fn parse_idle_minutes_defaults_and_clamps_invalid() {
        assert_eq!(parse_folder_inbound_idle_minutes(None), 30);
        assert_eq!(parse_folder_inbound_idle_minutes(Some("15")), 15);
        assert_eq!(parse_folder_inbound_idle_minutes(Some(" 0 ")), 0);
        assert_eq!(parse_folder_inbound_idle_minutes(Some("-1")), 30);
        assert_eq!(parse_folder_inbound_idle_minutes(Some("abc")), 30);
        assert_eq!(parse_folder_inbound_idle_minutes(Some("10081")), 30);
        assert_eq!(parse_folder_inbound_idle_minutes(Some("10080")), 10080);
    }

    #[test]
    fn extracts_lark_chat_quote_and_time() {
        let cmd = lark_cmd(
            1,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "parent_id": "om_parent",
                        "root_id": "om_root",
                        "chat_id": "oc_bound",
                        "create_time": "1694779200000"
                    }
                }
            }),
        );
        assert_eq!(inbound_chat_id(&cmd).as_deref(), Some("oc_bound"));
        assert_eq!(
            inbound_quoted_message_id(&cmd).as_deref(),
            Some("om_parent")
        );
        assert_eq!(
            inbound_quoted_message_ids(&cmd),
            vec!["om_parent".to_string(), "om_root".to_string()]
        );
        assert_eq!(
            inbound_provider_message_id(&cmd).as_deref(),
            Some("om_child")
        );
        assert_eq!(
            inbound_message_time(&cmd),
            Utc.timestamp_millis_opt(1694779200000).single().unwrap()
        );
    }

    /// Feishu 引用回复 often leaves `parent_id` empty and puts the quoted
    /// message id on `event.message.quote.message_id`.
    #[test]
    fn extracts_lark_quote_field_when_parent_id_missing() {
        let cmd = lark_cmd(
            1,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "chat_id": "oc_bound",
                        "quote": { "message_id": "om_quoted" }
                    }
                }
            }),
        );
        assert_eq!(
            inbound_quoted_message_id(&cmd).as_deref(),
            Some("om_quoted")
        );
        assert_eq!(
            inbound_quoted_message_ids(&cmd),
            vec!["om_quoted".to_string()]
        );
    }

    #[test]
    fn extracts_telegram_reply_to_message() {
        let mut cmd = IncomingCommand::plain(
            2,
            "99",
            "again",
            ChannelMessageTarget::with_chat_id(2, "-100123"),
        );
        cmd.metadata = serde_json::json!({
            "message": {
                "message_id": 42,
                "date": 1694779200,
                "chat": { "id": -100123 },
                "reply_to_message": { "message_id": 41 }
            }
        });
        assert_eq!(inbound_chat_id(&cmd).as_deref(), Some("-100123"));
        assert_eq!(inbound_quoted_message_id(&cmd).as_deref(), Some("41"));
        assert_eq!(inbound_provider_message_id(&cmd).as_deref(), Some("42"));
        assert_eq!(
            inbound_message_time(&cmd),
            Utc.timestamp_opt(1694779200, 0).single().unwrap()
        );
    }

    #[test]
    fn ignores_quote_when_parent_equals_self() {
        let cmd = lark_cmd(
            1,
            serde_json::json!({
                "event": { "message": { "message_id": "om_1", "parent_id": "om_1" } }
            }),
        );
        assert_eq!(inbound_quoted_message_id(&cmd), None);
    }

    #[tokio::test]
    async fn unbound_chat_is_not_folder_routed() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_lark_channel(&db).await;
        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({ "event": { "message": { "chat_id": "oc_unbound" } } }),
        );
        assert_eq!(route_folder_inbound(&db.conn, &cmd).await.unwrap(), None);
    }

    #[tokio::test]
    async fn quote_field_continues_mapped_session_when_parent_id_missing() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-quote-field").await;
        let quoted = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let latest = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_quoted", quoted)
            .await
            .unwrap();
        set_updated_at(&db, latest, Utc::now()).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "chat_id": "oc_bound",
                        "quote": { "message_id": "om_quoted" },
                        "create_time": "1694779200000"
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(
            plan.action,
            FolderInboundAction::Continue {
                conversation_id: quoted
            }
        );
    }

    #[tokio::test]
    async fn quoted_message_continues_mapped_session() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-quote").await;
        let quoted = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let latest = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_parent", quoted)
            .await
            .unwrap();
        set_updated_at(&db, latest, Utc::now()).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "parent_id": "om_parent",
                        "chat_id": "oc_bound",
                        "create_time": "1694779200000"
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.bound_folder_id, folder_id);
        assert_eq!(
            plan.action,
            FolderInboundAction::Continue {
                conversation_id: quoted
            }
        );
    }

    /// The card may have been delivered to the bot DM. Quoting it there still
    /// continues the folder session; the inbound chat does not have to be the
    /// folder's configured session id.
    #[tokio::test]
    async fn quote_in_another_chat_continues_mapped_session() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-other-chat").await;
        let quoted = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_group")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_parent", quoted)
            .await
            .unwrap();

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_reply",
                        "parent_id": "om_parent",
                        "chat_id": "oc_bot_dm",
                        "chat_type": "p2p",
                        "create_time": "1694779200000"
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("quote maps to the folder session");
        assert_eq!(plan.bound_folder_id, folder_id);
        assert_eq!(
            plan.action,
            FolderInboundAction::Continue {
                conversation_id: quoted
            }
        );
    }

    #[tokio::test]
    async fn no_quote_starts_a_new_conversation_even_when_recent() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-recent").await;
        let latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, latest, now - Duration::minutes(10)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_new",
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
    }

    #[tokio::test]
    async fn no_quote_starts_new_after_idle_window() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-stale").await;
        let latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, latest, now - Duration::minutes(31)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_new",
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
        assert_eq!(plan.bound_folder_id, folder_id);
    }

    #[tokio::test]
    async fn stored_idle_minutes_override_the_default_window() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-custom-idle").await;
        let latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        app_metadata_service::upsert_value(&db.conn, FOLDER_INBOUND_IDLE_KEY, "5")
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, latest, now - Duration::minutes(10)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_new",
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
    }

    #[tokio::test]
    async fn latest_follows_updated_at_not_created_at() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-updated-at").await;
        let older = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let newer_created = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, older, now - Duration::minutes(2)).await;
        set_updated_at(&db, newer_created, now - Duration::hours(2)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
    }

    #[tokio::test]
    async fn unknown_quote_falls_through_to_idle_window() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-unknown-quote").await;
        let latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, latest, now - Duration::minutes(5)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "parent_id": "om_unknown",
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
    }

    #[tokio::test]
    async fn unquoted_prefers_mapped_conversation_over_newer_unmapped() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-mapped-idle").await;
        let spoken = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let newer_desktop = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_done", spoken)
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
        set_updated_at(&db, spoken, now - Duration::minutes(8)).await;
        set_updated_at(&db, newer_desktop, now - Duration::minutes(1)).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_followup",
                        "chat_id": "oc_bound",
                        "create_time": now.timestamp_millis().to_string()
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(plan.action, FolderInboundAction::StartNew);
    }

    #[tokio::test]
    async fn root_id_quote_continues_when_parent_is_unmapped() {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/folder-inbound-root-id").await;
        let quoted = seed_conversation(&db, folder_id, AgentType::Codex).await;
        let latest = seed_conversation(&db, folder_id, AgentType::Grok).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, folder_id, channel_id, Some("oc_bound")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_root", quoted)
            .await
            .unwrap();
        set_updated_at(&db, latest, Utc::now()).await;

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "parent_id": "om_unknown",
                        "root_id": "om_root",
                        "chat_id": "oc_bound",
                        "create_time": "1694779200000"
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("bound");
        assert_eq!(
            plan.action,
            FolderInboundAction::Continue {
                conversation_id: quoted
            }
        );
    }

    /// Quoting a card follows that card's conversation, even when the reply
    /// arrives in a chat bound to a different folder. The previous rule started
    /// a new task in the inbound chat and never ran the session that sent the
    /// card.
    #[tokio::test]
    async fn quote_of_another_folders_card_continues_that_session() {
        let db = fresh_in_memory_db().await;
        let bound = seed_folder(&db, "/tmp/folder-inbound-bound").await;
        let other = seed_folder(&db, "/tmp/folder-inbound-other").await;
        let elsewhere = seed_conversation(&db, other, AgentType::Codex).await;
        let channel_id = seed_lark_channel(&db).await;
        bind_folder(&db, bound, channel_id, Some("oc_bound")).await;
        chat_channel_message_map_service::upsert(&db.conn, channel_id, "om_other", elsewhere)
            .await
            .unwrap();

        let cmd = lark_cmd(
            channel_id,
            serde_json::json!({
                "event": {
                    "message": {
                        "message_id": "om_child",
                        "parent_id": "om_other",
                        "chat_id": "oc_bound"
                    }
                }
            }),
        );
        let plan = route_folder_inbound(&db.conn, &cmd)
            .await
            .unwrap()
            .expect("quote maps to a session");
        assert_eq!(plan.bound_folder_id, other);
        assert_eq!(
            plan.action,
            FolderInboundAction::Continue {
                conversation_id: elsewhere
            }
        );
    }
}

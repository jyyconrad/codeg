//! Where a Feishu message should be acknowledged and where the finished task
//! should be posted.
//!
//! A topic-mode group turns every new `im/v1/messages` send into a new topic.
//! Status replies therefore react to the inbound message. The finished task
//! is a reply inside the topic, a quote reply, or a normal chat message.
//!
//! Once a folder has seen a topic, that first message stays the anchor. Later
//! topic mentions do not move it. Every later card for that folder's channel
//! replies to the anchor, so every conversation in the folder stays in one
//! topic. The reaction still lands on the message that was just received.

use sea_orm::DatabaseConnection;
use serde_json::{Map, Value};

use super::types::ChannelMessageTarget;
use crate::db::service::{
    conversation_service, folder_chat_channel_service, thread_binding_service,
};

pub const LARK_DELIVERY_KEY: &str = "lark_delivery";
/// Same reaction OpenClaw uses as its Feishu ack. It renders as a pill under
/// the inbound message instead of a new topic.
pub const LARK_ACK_EMOJI: &str = "Typing";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LarkCompletion {
    /// Reply inside the topic (`reply_in_thread`).
    Topic { message_id: String },
    /// Quote-reply to the message the user cited.
    Quote { message_id: String },
    /// Ordinary chat message in the bound conversation.
    Chat,
}

pub fn attach_delivery(target: &mut ChannelMessageTarget, event: &Value) {
    let Some(delivery) = delivery_from_event(event) else {
        return;
    };
    let mut payload = match target.provider_payload.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            target.provider_payload = Some(other);
            return;
        }
        None => Map::new(),
    };
    payload.insert(LARK_DELIVERY_KEY.to_string(), delivery);
    target.provider_payload = Some(Value::Object(payload));
}

pub fn merge_stored_delivery(target: &mut ChannelMessageTarget, stored: &Option<Value>) {
    let Some(delivery) = stored
        .as_ref()
        .and_then(|value| value.get(LARK_DELIVERY_KEY))
        .cloned()
    else {
        return;
    };
    let mut payload = match target.provider_payload.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            target.provider_payload = Some(other);
            return;
        }
        None => Map::new(),
    };
    payload.insert(LARK_DELIVERY_KEY.to_string(), delivery);
    target.provider_payload = Some(Value::Object(payload));
}

pub fn ack_message_id(payload: &Option<Value>) -> Option<String> {
    nonempty_str(delivery(payload)?.get("ack_message_id"))
}

pub fn completion_plan(payload: &Option<Value>) -> Option<LarkCompletion> {
    let delivery = delivery(payload)?;
    match delivery.get("mode").and_then(Value::as_str)? {
        "topic" => Some(LarkCompletion::Topic {
            message_id: nonempty_str(delivery.get("reply_message_id"))?,
        }),
        "quote" => Some(LarkCompletion::Quote {
            message_id: nonempty_str(delivery.get("reply_message_id"))?,
        }),
        "chat" => Some(LarkCompletion::Chat),
        _ => None,
    }
}

/// Inbound message a topic mention should pin as the folder anchor.
/// Quote and ordinary chat messages do not move the folder topic.
pub fn topic_anchor_message_id(payload: &Option<Value>) -> Option<String> {
    match completion_plan(payload) {
        Some(LarkCompletion::Topic { message_id }) => Some(message_id),
        _ => None,
    }
}

pub fn bound_folder_id(payload: &Option<Value>) -> Option<i32> {
    let value = delivery(payload)?.get("bound_folder_id")?;
    let id = value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim().parse::<i64>().ok())
    })?;
    i32::try_from(id).ok().filter(|id| *id > 0)
}

/// Remember which bound folder a channel conversation answers for. Channel
/// chats live in their own hidden folder, so the send path cannot discover
/// the binding from `conversation.folder_id` alone.
pub fn stamp_bound_folder(target: &mut ChannelMessageTarget, folder_id: i32) {
    let Some(mut delivery) = take_delivery(target) else {
        return;
    };
    delivery.insert(
        "bound_folder_id".to_string(),
        Value::Number(folder_id.into()),
    );
    put_delivery(target, delivery);
}

/// Reply inside the folder topic. Keeps the inbound ack id and any other
/// payload keys.
pub fn force_topic_reply(target: &mut ChannelMessageTarget, anchor: &str) {
    let anchor = anchor.trim();
    if anchor.is_empty() {
        return;
    }
    let Some(mut delivery) = take_delivery(target) else {
        return;
    };
    delivery.insert("mode".to_string(), Value::String("topic".to_string()));
    delivery.insert(
        "reply_message_id".to_string(),
        Value::String(anchor.to_string()),
    );
    put_delivery(target, delivery);
}

/// When this folder already has a topic anchor, send the card there.
/// Returns whether the target now replies inside that topic.
pub async fn apply_folder_topic(
    db: &DatabaseConnection,
    conversation_id: Option<i32>,
    target: &mut ChannelMessageTarget,
) -> bool {
    let Some(anchor) = folder_topic_anchor(db, conversation_id, target).await else {
        return false;
    };
    force_topic_reply(target, &anchor);
    completion_plan(&target.provider_payload) == Some(LarkCompletion::Topic { message_id: anchor })
}

async fn folder_topic_anchor(
    db: &DatabaseConnection,
    conversation_id: Option<i32>,
    target: &ChannelMessageTarget,
) -> Option<String> {
    let mut folders = Vec::new();
    if let Some(conversation_id) = conversation_id {
        if let Ok(conv) = conversation_service::get_by_id(db, conversation_id).await {
            push_folder(&mut folders, conv.folder_id);
        }
    }
    if let Some(folder_id) = bound_folder_id(&target.provider_payload) {
        push_folder(&mut folders, folder_id);
    }
    if let Some(conversation_id) = conversation_id {
        if let Ok(Some(stored)) =
            thread_binding_service::chat_conversation_target(db, conversation_id).await
        {
            if let Some(folder_id) = bound_folder_id(&stored.provider_payload) {
                push_folder(&mut folders, folder_id);
            }
        }
    }
    if let Some(chat_id) = target
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|chat_id| !chat_id.is_empty())
    {
        if let Ok(Some(folder_id)) = folder_chat_channel_service::find_folder_id_for_inbound(
            db,
            target.channel_id,
            Some(chat_id),
        )
        .await
        {
            push_folder(&mut folders, folder_id);
        }
    }

    for folder_id in folders {
        match folder_chat_channel_service::folder_topic_message(db, folder_id, target.channel_id)
            .await
        {
            Ok(Some(anchor)) => return Some(anchor),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    folder_id,
                    channel_id = target.channel_id,
                    error = %error,
                    "[ChatChannel] failed to load folder topic"
                );
            }
        }
    }
    None
}

fn push_folder(folders: &mut Vec<i32>, folder_id: i32) {
    if folder_id > 0 && !folders.contains(&folder_id) {
        folders.push(folder_id);
    }
}

fn take_delivery(target: &mut ChannelMessageTarget) -> Option<Map<String, Value>> {
    let mut payload = match target.provider_payload.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            target.provider_payload = Some(other);
            return None;
        }
        None => Map::new(),
    };
    let delivery = match payload.remove(LARK_DELIVERY_KEY) {
        Some(Value::Object(map)) => map,
        Some(other) => {
            payload.insert(LARK_DELIVERY_KEY.to_string(), other);
            target.provider_payload = Some(Value::Object(payload));
            return None;
        }
        None => Map::new(),
    };
    target.provider_payload = Some(Value::Object(payload));
    Some(delivery)
}

fn put_delivery(target: &mut ChannelMessageTarget, delivery: Map<String, Value>) {
    let mut payload = match target.provider_payload.take() {
        Some(Value::Object(map)) => map,
        Some(other) => {
            target.provider_payload = Some(other);
            return;
        }
        None => Map::new(),
    };
    payload.insert(LARK_DELIVERY_KEY.to_string(), Value::Object(delivery));
    target.provider_payload = Some(Value::Object(payload));
}

fn delivery(payload: &Option<Value>) -> Option<&Value> {
    payload.as_ref()?.get(LARK_DELIVERY_KEY)
}

fn delivery_from_event(event: &Value) -> Option<Value> {
    let message_id = nonempty_str(event.pointer("/event/message/message_id"))?;
    let thread_id = nonempty_str(event.pointer("/event/message/thread_id"));
    // Topic-group posts are topics even when the receive event omits
    // `thread_id`. Creating a chat message there opens another topic, so the
    // finished task replies to this message with `reply_in_thread`.
    let topic_group = event
        .pointer("/event/message/chat_type")
        .and_then(Value::as_str)
        .is_some_and(|chat_type| chat_type.trim() == "topic_group");
    let quote_id = explicit_quote_id(event, &message_id);
    let (mode, reply_message_id) = if thread_id.is_some() || topic_group {
        ("topic", Some(message_id.clone()))
    } else if let Some(quote_id) = quote_id {
        ("quote", Some(quote_id))
    } else {
        ("chat", None)
    };
    Some(serde_json::json!({
        "ack_message_id": message_id,
        "mode": mode,
        "reply_message_id": reply_message_id,
    }))
}

/// Quote targets are the message the user cited. A topic id wins over these
/// so a topic that embeds a quote still stays in that topic. `root_id` counts
/// only when this message is itself a reply (`root_id` differs from itself).
fn explicit_quote_id(event: &Value, message_id: &str) -> Option<String> {
    let different = |id: String| (id != message_id).then_some(id);
    nonempty_str(event.pointer("/event/message/parent_id"))
        .and_then(different)
        .or_else(|| nonempty_str(event.pointer("/event/message/quote/message_id")))
        .or_else(|| nonempty_str(event.pointer("/event/message/quote/id")))
        .or_else(|| quote_token_in_content(event))
        .or_else(|| nonempty_str(event.pointer("/event/message/root_id")).and_then(different))
}

fn quote_token_in_content(event: &Value) -> Option<String> {
    let content = event
        .pointer("/event/message/content")
        .and_then(Value::as_str)?;
    let value = serde_json::from_str::<Value>(content).ok()?;
    find_quote_token(&value)
}

fn find_quote_token(value: &Value) -> Option<String> {
    match value {
        Value::Array(values) => values.iter().find_map(find_quote_token),
        Value::Object(object) => {
            if object.get("tag").and_then(Value::as_str) == Some("quote") {
                for key in ["token", "message_id", "id"] {
                    if let Some(id) = nonempty_str(object.get(key)) {
                        return Some(id);
                    }
                }
            }
            object.values().find_map(find_quote_token)
        }
        _ => None,
    }
}

fn nonempty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(message: Value) -> Value {
        serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": { "message": message }
        })
    }

    fn plan(message: Value) -> (Option<String>, Option<LarkCompletion>) {
        let mut target = ChannelMessageTarget::with_chat_id(1, "oc_group");
        attach_delivery(&mut target, &event(message));
        (
            ack_message_id(&target.provider_payload),
            completion_plan(&target.provider_payload),
        )
    }

    #[test]
    fn topic_message_acks_in_place_and_completes_inside_the_topic() {
        let (ack, completion) = plan(serde_json::json!({
            "message_id": "om_topic",
            "thread_id": "omt_topic",
            "chat_id": "oc_group"
        }));
        assert_eq!(ack.as_deref(), Some("om_topic"));
        assert_eq!(
            completion,
            Some(LarkCompletion::Topic {
                message_id: "om_topic".into()
            })
        );
    }

    #[test]
    fn topic_keeps_the_reply_even_when_the_post_quotes_another_message() {
        let (_, completion) = plan(serde_json::json!({
            "message_id": "om_topic",
            "thread_id": "omt_topic",
            "parent_id": "om_quoted",
            "content": serde_json::json!({
                "content": [[{ "tag": "quote", "token": "om_quoted" }]]
            }).to_string()
        }));
        assert_eq!(
            completion,
            Some(LarkCompletion::Topic {
                message_id: "om_topic".into()
            })
        );
    }

    #[test]
    fn quoted_chat_message_replies_to_the_quoted_message() {
        let (ack, completion) = plan(serde_json::json!({
            "message_id": "om_user",
            "parent_id": "om_quoted"
        }));
        assert_eq!(ack.as_deref(), Some("om_user"));
        assert_eq!(
            completion,
            Some(LarkCompletion::Quote {
                message_id: "om_quoted".into()
            })
        );
    }

    #[test]
    fn plain_chat_message_stays_in_the_conversation() {
        let (_, completion) = plan(serde_json::json!({
            "message_id": "om_user",
            "chat_id": "oc_group"
        }));
        assert_eq!(completion, Some(LarkCompletion::Chat));
    }

    #[test]
    fn topic_group_without_thread_id_stays_in_the_topic() {
        let (ack, completion) = plan(serde_json::json!({
            "message_id": "om_root",
            "chat_id": "oc_group",
            "chat_type": "topic_group"
        }));
        assert_eq!(ack.as_deref(), Some("om_root"));
        assert_eq!(
            completion,
            Some(LarkCompletion::Topic {
                message_id: "om_root".into()
            })
        );
    }

    #[test]
    fn topic_group_reply_without_thread_id_does_not_become_a_quote() {
        let (_, completion) = plan(serde_json::json!({
            "message_id": "om_reply",
            "root_id": "om_root",
            "parent_id": "om_root",
            "chat_type": "topic_group"
        }));
        assert_eq!(
            completion,
            Some(LarkCompletion::Topic {
                message_id: "om_reply".into()
            })
        );
    }

    #[test]
    fn root_id_of_a_reply_is_the_quoted_message_when_there_is_no_topic() {
        let (_, completion) = plan(serde_json::json!({
            "message_id": "om_reply",
            "root_id": "om_root"
        }));
        assert_eq!(
            completion,
            Some(LarkCompletion::Quote {
                message_id: "om_root".into()
            })
        );
    }

    #[test]
    fn merge_adds_delivery_without_dropping_an_existing_payload_key() {
        let mut target = ChannelMessageTarget::with_chat_id(3, "oc_folder");
        target.provider_payload = Some(serde_json::json!({ "root_id": "om_root" }));
        let stored = serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_in",
                "mode": "topic",
                "reply_message_id": "om_in"
            }
        });
        merge_stored_delivery(&mut target, &Some(stored));
        assert_eq!(
            target
                .provider_payload
                .as_ref()
                .and_then(|v| v.get("root_id")),
            Some(&serde_json::json!("om_root"))
        );
        assert_eq!(
            completion_plan(&target.provider_payload),
            Some(LarkCompletion::Topic {
                message_id: "om_in".into()
            })
        );
    }

    #[test]
    fn folder_topic_keeps_the_ack_and_replaces_a_quote() {
        let mut target = ChannelMessageTarget::with_chat_id(3, "oc_folder");
        target.provider_payload = Some(serde_json::json!({
            "root_id": "om_root",
            "lark_delivery": {
                "ack_message_id": "om_in",
                "mode": "quote",
                "reply_message_id": "om_quoted"
            }
        }));
        force_topic_reply(&mut target, "om_anchor");
        assert_eq!(
            ack_message_id(&target.provider_payload).as_deref(),
            Some("om_in")
        );
        assert_eq!(
            completion_plan(&target.provider_payload),
            Some(LarkCompletion::Topic {
                message_id: "om_anchor".into()
            })
        );
        assert_eq!(
            target
                .provider_payload
                .as_ref()
                .and_then(|v| v.get("root_id")),
            Some(&serde_json::json!("om_root"))
        );
        assert_eq!(
            topic_anchor_message_id(&target.provider_payload).as_deref(),
            Some("om_anchor")
        );
    }

    #[test]
    fn quote_and_chat_do_not_supply_a_folder_anchor() {
        let quote = serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_in",
                "mode": "quote",
                "reply_message_id": "om_quoted"
            }
        });
        let chat = serde_json::json!({
            "lark_delivery": { "ack_message_id": "om_in", "mode": "chat" }
        });
        assert_eq!(topic_anchor_message_id(&Some(quote)), None);
        assert_eq!(topic_anchor_message_id(&Some(chat)), None);
    }

    #[tokio::test]
    async fn folder_topic_overrides_each_conversation_reply_target() {
        use crate::db::service::{
            chat_channel_service, conversation_service, folder_chat_channel_service,
            thread_binding_service,
        };
        use crate::db::test_helpers::{self, fresh_in_memory_db};
        use crate::models::agent::AgentType;

        let db = fresh_in_memory_db().await;
        let folder_id = test_helpers::seed_folder(&db, "/tmp/lark-folder-topic").await;
        let hidden_id = test_helpers::seed_folder(&db, "/tmp/lark-folder-topic-chat").await;
        let channel = chat_channel_service::create(
            &db.conn,
            "lark".into(),
            "lark".into(),
            serde_json::json!({ "chat_id": "oc_default" }).to_string(),
            true,
            false,
            None,
        )
        .await
        .unwrap();
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id: channel.id,
                chat_id: Some("oc_group".into()),
            }],
        )
        .await
        .unwrap();
        folder_chat_channel_service::set_folder_topic_message(
            &db.conn,
            folder_id,
            channel.id,
            "om_anchor",
        )
        .await
        .unwrap();

        let in_folder = conversation_service::create(
            &db.conn,
            folder_id,
            AgentType::Grok,
            Some("desk".into()),
            None,
        )
        .await
        .unwrap();
        let mut desk = ChannelMessageTarget::with_chat_id(channel.id, "oc_group");
        desk.provider_payload = Some(serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_desk",
                "mode": "topic",
                "reply_message_id": "om_desk"
            }
        }));
        assert!(apply_folder_topic(&db.conn, Some(in_folder.id), &mut desk).await);
        assert_eq!(
            completion_plan(&desk.provider_payload),
            Some(LarkCompletion::Topic {
                message_id: "om_anchor".into()
            })
        );
        assert_eq!(
            ack_message_id(&desk.provider_payload).as_deref(),
            Some("om_desk")
        );

        let hidden = conversation_service::create(
            &db.conn,
            hidden_id,
            AgentType::Grok,
            Some("channel chat".into()),
            None,
        )
        .await
        .unwrap();
        let mut stored = ChannelMessageTarget::with_chat_id(channel.id, "oc_group");
        stored.provider_payload = Some(serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_own",
                "mode": "chat",
                "reply_message_id": null,
                "bound_folder_id": folder_id
            }
        }));
        thread_binding_service::set_chat_conversation_target(
            &db.conn, &stored, hidden.id, "ou_user",
        )
        .await
        .unwrap();
        // The live session target is not the bound chat id, so the anchor has
        // to come from the folder stamped on the stored route.
        let mut live = ChannelMessageTarget::with_chat_id(channel.id, "oc_dm");
        live.provider_payload = Some(serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_own",
                "mode": "topic",
                "reply_message_id": "om_own"
            }
        }));
        assert!(apply_folder_topic(&db.conn, Some(hidden.id), &mut live).await);
        assert_eq!(
            completion_plan(&live.provider_payload),
            Some(LarkCompletion::Topic {
                message_id: "om_anchor".into()
            })
        );
        assert_eq!(
            ack_message_id(&live.provider_payload).as_deref(),
            Some("om_own")
        );

        let mut untouched = live.clone();
        untouched.provider_payload = Some(serde_json::json!({
            "lark_delivery": {
                "ack_message_id": "om_own",
                "mode": "quote",
                "reply_message_id": "om_quoted"
            }
        }));
        // Anchor still exists, so the quote is pulled back into the topic.
        assert!(apply_folder_topic(&db.conn, Some(hidden.id), &mut untouched).await);
        assert_eq!(
            completion_plan(&untouched.provider_payload),
            Some(LarkCompletion::Topic {
                message_id: "om_anchor".into()
            })
        );
    }
}

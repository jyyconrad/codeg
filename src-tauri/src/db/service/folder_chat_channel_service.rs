use std::collections::HashSet;

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::NotSet, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set, TransactionTrait,
};

use crate::db::entities::{chat_channel, folder_chat_channel};
use crate::db::error::DbError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderChannelBinding {
    pub channel_id: i32,
    pub chat_id: Option<String>,
}

/// One folder's Feishu topic on a channel. Later cards reply to
/// `topic_message_id` instead of opening a new chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderTopicAnchor {
    pub folder_id: i32,
    pub chat_id: Option<String>,
    pub topic_message_id: String,
}

fn normalize_chat_id(chat_id: Option<String>) -> Option<String> {
    chat_id.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn dedup_bindings(bindings: &[FolderChannelBinding]) -> Vec<FolderChannelBinding> {
    let mut seen = HashSet::with_capacity(bindings.len());
    let mut unique = Vec::with_capacity(bindings.len());
    for binding in bindings {
        if seen.insert(binding.channel_id) {
            unique.push(FolderChannelBinding {
                channel_id: binding.channel_id,
                chat_id: normalize_chat_id(binding.chat_id.clone()),
            });
        }
    }
    unique
}

fn dedup_channel_ids(channel_ids: &[i32]) -> Vec<i32> {
    let mut seen = HashSet::with_capacity(channel_ids.len());
    let mut unique = Vec::with_capacity(channel_ids.len());
    for &id in channel_ids {
        if seen.insert(id) {
            unique.push(id);
        }
    }
    unique
}

async fn require_channels_exist<C: ConnectionTrait>(
    conn: &C,
    channel_ids: &[i32],
) -> Result<(), DbError> {
    if channel_ids.is_empty() {
        return Ok(());
    }
    let found = chat_channel::Entity::find()
        .filter(chat_channel::Column::Id.is_in(channel_ids.to_vec()))
        .all(conn)
        .await?;
    if found.len() == channel_ids.len() {
        return Ok(());
    }
    let present: HashSet<i32> = found.into_iter().map(|row| row.id).collect();
    let missing: Vec<String> = channel_ids
        .iter()
        .filter(|id| !present.contains(id))
        .map(i32::to_string)
        .collect();
    Err(DbError::Validation(format!(
        "channel not found: {}",
        missing.join(", ")
    )))
}

pub async fn list_bindings(
    conn: &DatabaseConnection,
    folder_id: i32,
) -> Result<Vec<FolderChannelBinding>, DbError> {
    let rows = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::FolderId.eq(folder_id))
        .order_by_asc(folder_chat_channel::Column::ChannelId)
        .all(conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| FolderChannelBinding {
            channel_id: row.channel_id,
            chat_id: normalize_chat_id(row.chat_id),
        })
        .collect())
}

pub async fn list_channel_ids(
    conn: &DatabaseConnection,
    folder_id: i32,
) -> Result<Vec<i32>, DbError> {
    Ok(list_bindings(conn, folder_id)
        .await?
        .into_iter()
        .map(|row| row.channel_id)
        .collect())
}

async fn ensure_session_available<C: ConnectionTrait>(
    conn: &C,
    folder_id: i32,
    binding: &FolderChannelBinding,
) -> Result<(), DbError> {
    let rows = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::ChannelId.eq(binding.channel_id))
        .all(conn)
        .await?;
    let wanted = normalize_chat_id(binding.chat_id.clone());
    if rows
        .iter()
        .any(|row| row.folder_id != folder_id && normalize_chat_id(row.chat_id.clone()) == wanted)
    {
        return Err(DbError::Validation(
            "this channel session is already bound to another folder".to_string(),
        ));
    }
    Ok(())
}

fn channel_config_chat_id(config_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(config_json).ok()?;
    normalize_chat_id(
        value
            .get("chat_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    )
}

/// Resolve which folder an inbound IM chat should land in.
///
/// Prefer an exact `folder_chat_channel.chat_id` match. If none, a binding
/// with empty `chat_id` (channel default) matches when the inbound chat is
/// missing or equals the channel's configured `chat_id`.
pub async fn find_folder_id_for_inbound(
    conn: &DatabaseConnection,
    channel_id: i32,
    incoming_chat_id: Option<&str>,
) -> Result<Option<i32>, DbError> {
    let incoming = normalize_chat_id(incoming_chat_id.map(str::to_string));
    let rows = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::ChannelId.eq(channel_id))
        .order_by_desc(folder_chat_channel::Column::Id)
        .all(conn)
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }

    if let Some(incoming) = incoming.as_deref() {
        if let Some(row) = rows
            .iter()
            .find(|row| normalize_chat_id(row.chat_id.clone()).as_deref() == Some(incoming))
        {
            return Ok(Some(row.folder_id));
        }
    }

    let default_chat_id = match chat_channel::Entity::find_by_id(channel_id)
        .one(conn)
        .await?
    {
        Some(channel) => channel_config_chat_id(&channel.config_json),
        None => None,
    };
    let incoming_matches_default = match (incoming.as_deref(), default_chat_id.as_deref()) {
        (Some(incoming), Some(default)) => incoming == default,
        (None, _) => true,
        (Some(_), None) => false,
    };
    if !incoming_matches_default {
        return Ok(None);
    }
    Ok(rows
        .iter()
        .find(|row| normalize_chat_id(row.chat_id.clone()).is_none())
        .map(|row| row.folder_id))
}

pub async fn set_bindings(
    conn: &DatabaseConnection,
    folder_id: i32,
    bindings: &[FolderChannelBinding],
) -> Result<Vec<FolderChannelBinding>, DbError> {
    let unique = dedup_bindings(bindings);
    if unique.len() > 1 {
        return Err(DbError::Validation(
            "a folder can bind only one channel session".to_string(),
        ));
    }
    let channel_ids: Vec<i32> = unique.iter().map(|row| row.channel_id).collect();
    let txn = conn.begin().await?;
    require_channels_exist(&txn, &channel_ids).await?;
    if let Some(binding) = unique.first() {
        ensure_session_available(&txn, folder_id, binding).await?;
    }

    // Re-saving the dialog deletes and inserts. Keep the topic anchor when
    // the same channel session stays bound. A different chat drops it,
    // because that message id belongs to the previous conversation.
    let previous = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::FolderId.eq(folder_id))
        .all(&txn)
        .await?;

    folder_chat_channel::Entity::delete_many()
        .filter(folder_chat_channel::Column::FolderId.eq(folder_id))
        .exec(&txn)
        .await?;

    if !unique.is_empty() {
        let now = Utc::now();
        let models = unique
            .iter()
            .map(|binding| {
                let topic_message_id = previous.iter().find_map(|row| {
                    let same_session = row.channel_id == binding.channel_id
                        && normalize_chat_id(row.chat_id.clone()) == binding.chat_id;
                    same_session
                        .then(|| normalize_chat_id(row.topic_message_id.clone()))
                        .flatten()
                });
                folder_chat_channel::ActiveModel {
                    id: NotSet,
                    folder_id: Set(folder_id),
                    channel_id: Set(binding.channel_id),
                    chat_id: Set(binding.chat_id.clone()),
                    topic_message_id: Set(topic_message_id),
                    created_at: Set(now),
                }
            })
            .collect::<Vec<_>>();
        folder_chat_channel::Entity::insert_many(models)
            .exec(&txn)
            .await?;
    }

    txn.commit().await?;
    list_bindings(conn, folder_id).await
}

/// Topic anchors on this channel. A channel with exactly one anchor is the
/// bound topic that replaces the channel's original default chat.
pub async fn list_topic_anchors(
    conn: &DatabaseConnection,
    channel_id: i32,
) -> Result<Vec<FolderTopicAnchor>, DbError> {
    let rows = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::ChannelId.eq(channel_id))
        .order_by_asc(folder_chat_channel::Column::Id)
        .all(conn)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let topic_message_id = normalize_chat_id(row.topic_message_id)?;
            Some(FolderTopicAnchor {
                folder_id: row.folder_id,
                chat_id: normalize_chat_id(row.chat_id),
                topic_message_id,
            })
        })
        .collect())
}

/// Feishu message id that later cards in this folder reply to, inside one topic.
pub async fn folder_topic_message(
    conn: &DatabaseConnection,
    folder_id: i32,
    channel_id: i32,
) -> Result<Option<String>, DbError> {
    let row = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::FolderId.eq(folder_id))
        .filter(folder_chat_channel::Column::ChannelId.eq(channel_id))
        .one(conn)
        .await?;
    Ok(row.and_then(|row| normalize_chat_id(row.topic_message_id)))
}

/// Point the folder's channel binding at one topic anchor. Does nothing when
/// the folder has no row for this channel. The first topic stays. A later
/// topic does not replace it, so every conversation in the folder keeps
/// replying in that same topic.
pub async fn set_folder_topic_message(
    conn: &DatabaseConnection,
    folder_id: i32,
    channel_id: i32,
    message_id: &str,
) -> Result<bool, DbError> {
    let Some(message_id) = normalize_chat_id(Some(message_id.to_string())) else {
        return Ok(false);
    };
    let Some(row) = folder_chat_channel::Entity::find()
        .filter(folder_chat_channel::Column::FolderId.eq(folder_id))
        .filter(folder_chat_channel::Column::ChannelId.eq(channel_id))
        .one(conn)
        .await?
    else {
        return Ok(false);
    };
    if normalize_chat_id(row.topic_message_id.clone()).is_some() {
        return Ok(true);
    }
    let mut active = row.into_active_model();
    active.topic_message_id = Set(Some(message_id));
    active.update(conn).await?;
    Ok(true)
}

pub async fn set_channel_ids(
    conn: &DatabaseConnection,
    folder_id: i32,
    channel_ids: &[i32],
) -> Result<Vec<i32>, DbError> {
    let unique = dedup_channel_ids(channel_ids);
    let bindings: Vec<FolderChannelBinding> = unique
        .iter()
        .map(|&channel_id| FolderChannelBinding {
            channel_id,
            chat_id: None,
        })
        .collect();
    Ok(set_bindings(conn, folder_id, &bindings)
        .await?
        .into_iter()
        .map(|row| row.channel_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::service::chat_channel_service;

    async fn seed_channel(db: &crate::db::AppDatabase, channel_type: &str) -> i32 {
        chat_channel_service::create(
            &db.conn,
            format!("{channel_type} test"),
            channel_type.to_string(),
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
    async fn set_then_list_round_trips_and_replaces() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc").await;
        let a = seed_channel(&db, "tg").await;
        let b = seed_channel(&db, "lark").await;
        let err = set_channel_ids(&db.conn, folder_id, &[a, b])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("only one"));
        set_channel_ids(&db.conn, folder_id, &[a]).await.unwrap();
        assert_eq!(
            list_channel_ids(&db.conn, folder_id).await.unwrap(),
            vec![a]
        );
        set_channel_ids(&db.conn, folder_id, &[b]).await.unwrap();
        assert_eq!(
            list_channel_ids(&db.conn, folder_id).await.unwrap(),
            vec![b]
        );
    }

    #[tokio::test]
    async fn set_rejects_unknown_channel() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc2").await;
        let err = set_channel_ids(&db.conn, folder_id, &[9_999_999])
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("channel") || msg.contains("not found") || msg.contains("Validation"));
    }

    #[tokio::test]
    async fn set_empty_clears_bindings() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-empty").await;
        let a = seed_channel(&db, "tg").await;
        set_channel_ids(&db.conn, folder_id, &[a]).await.unwrap();
        set_channel_ids(&db.conn, folder_id, &[]).await.unwrap();
        assert!(list_channel_ids(&db.conn, folder_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn set_ignores_duplicate_channel_ids() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-dup").await;
        let a = seed_channel(&db, "tg").await;
        set_channel_ids(&db.conn, folder_id, &[a, a]).await.unwrap();
        assert_eq!(
            list_channel_ids(&db.conn, folder_id).await.unwrap(),
            vec![a]
        );
    }

    #[tokio::test]
    async fn folders_bind_channels_independently() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let root = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-root").await;
        let worktree = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-wt").await;
        let a = seed_channel(&db, "tg").await;
        let b = seed_channel(&db, "lark").await;
        set_channel_ids(&db.conn, root, &[a]).await.unwrap();
        set_channel_ids(&db.conn, worktree, &[b]).await.unwrap();
        assert_eq!(list_channel_ids(&db.conn, root).await.unwrap(), vec![a]);
        assert_eq!(list_channel_ids(&db.conn, worktree).await.unwrap(), vec![b]);
    }

    #[tokio::test]
    async fn set_then_list_preserves_optional_chat_id() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-chat-id").await;
        let a = seed_channel(&db, "tg").await;
        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: a,
                chat_id: Some("-100999".into()),
            }],
        )
        .await
        .unwrap();
        let rows = list_bindings(&db.conn, folder_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].chat_id.as_deref(), Some("-100999"));

        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: a,
                chat_id: Some("  ".into()),
            }],
        )
        .await
        .unwrap();
        let rows = list_bindings(&db.conn, folder_id).await.unwrap();
        assert_eq!(rows[0].chat_id, None);
    }

    #[tokio::test]
    async fn set_rejects_a_session_already_bound_to_another_folder() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let first = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-owner").await;
        let second = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-other").await;
        let channel = seed_channel(&db, "lark").await;
        set_bindings(
            &db.conn,
            first,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_one".into()),
            }],
        )
        .await
        .unwrap();
        let err = set_bindings(
            &db.conn,
            second,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_one".into()),
            }],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("already bound"));
        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_one"))
                .await
                .unwrap(),
            Some(first)
        );
    }

    #[tokio::test]
    async fn inbound_prefers_exact_chat_id_over_default_binding() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let bound = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-exact").await;
        let fallback = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-default").await;
        let channel = seed_channel(&db, "lark").await;
        set_bindings(
            &db.conn,
            fallback,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: None,
            }],
        )
        .await
        .unwrap();
        set_bindings(
            &db.conn,
            bound,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_bound".into()),
            }],
        )
        .await
        .unwrap();

        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_bound"))
                .await
                .unwrap(),
            Some(bound)
        );
        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_other"))
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn inbound_null_binding_matches_channel_default_chat_id() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-default-chat").await;
        let channel = chat_channel_service::create(
            &db.conn,
            "lark default".to_string(),
            "lark".to_string(),
            serde_json::json!({ "chat_id": "oc_default" }).to_string(),
            true,
            false,
            None,
        )
        .await
        .expect("seed chat channel")
        .id;
        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: None,
            }],
        )
        .await
        .unwrap();

        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_default"))
                .await
                .unwrap(),
            Some(folder_id)
        );
        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_other"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, None)
                .await
                .unwrap(),
            Some(folder_id)
        );
    }

    #[tokio::test]
    async fn topic_anchor_survives_resaving_the_same_session() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let folder_id = crate::db::test_helpers::seed_folder(&db, "/tmp/fcc-topic").await;
        let channel = seed_channel(&db, "lark").await;
        let other = seed_channel(&db, "tg").await;
        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_group".into()),
            }],
        )
        .await
        .unwrap();
        assert!(
            set_folder_topic_message(&db.conn, folder_id, channel, " om_anchor ")
                .await
                .unwrap()
        );
        assert!(
            set_folder_topic_message(&db.conn, folder_id, channel, "om_later")
                .await
                .unwrap()
        );
        assert_eq!(
            folder_topic_message(&db.conn, folder_id, channel)
                .await
                .unwrap()
                .as_deref(),
            Some("om_anchor")
        );

        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_group".into()),
            }],
        )
        .await
        .unwrap();
        assert_eq!(
            folder_topic_message(&db.conn, folder_id, channel)
                .await
                .unwrap()
                .as_deref(),
            Some("om_anchor")
        );

        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: channel,
                chat_id: Some("oc_other".into()),
            }],
        )
        .await
        .unwrap();
        assert_eq!(
            folder_topic_message(&db.conn, folder_id, channel)
                .await
                .unwrap(),
            None
        );

        assert!(
            set_folder_topic_message(&db.conn, folder_id, channel, "om_again")
                .await
                .unwrap()
        );
        set_bindings(
            &db.conn,
            folder_id,
            &[FolderChannelBinding {
                channel_id: other,
                chat_id: Some("oc_other".into()),
            }],
        )
        .await
        .unwrap();
        assert_eq!(
            folder_topic_message(&db.conn, folder_id, other)
                .await
                .unwrap(),
            None
        );
        assert!(
            !set_folder_topic_message(&db.conn, folder_id, channel, "om_missing")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn inbound_unbound_channel_returns_none() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let channel = seed_channel(&db, "lark").await;
        assert_eq!(
            find_folder_id_for_inbound(&db.conn, channel, Some("oc_x"))
                .await
                .unwrap(),
            None
        );
    }
}

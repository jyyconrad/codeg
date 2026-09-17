use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sea_orm::DatabaseConnection;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::command_handlers;
use super::folder_inbound::{remember_sent_message, route_folder_inbound};
use super::i18n::{self, Lang};
use super::manager::ChatChannelManager;
use super::session_bridge::SessionBridge;
use super::session_commands;
use super::types::{ChannelMessageTarget, IncomingCommand, InteractiveMessage, RichMessage};
use crate::acp::manager::ConnectionManager;
use crate::db::service::{app_metadata_service, chat_channel_message_log_service};
use crate::web::event_bridge::EventEmitter;

const COMMAND_PREFIX_KEY: &str = "chat_command_prefix";
const DEFAULT_COMMAND_PREFIX: &str = "/";
const MESSAGE_LANGUAGE_KEY: &str = "chat_message_language";
/// How often to refresh cached config from DB.
const CONFIG_CACHE_TTL_SECS: u64 = 30;

struct CommandConfigCache {
    prefix: String,
    lang: Lang,
    last_refresh: Instant,
}

impl CommandConfigCache {
    fn new() -> Self {
        Self {
            prefix: DEFAULT_COMMAND_PREFIX.to_string(),
            lang: Lang::default(),
            // Force refresh on first use
            last_refresh: Instant::now() - Duration::from_secs(CONFIG_CACHE_TTL_SECS + 1),
        }
    }

    async fn refresh_if_needed(&mut self, db: &DatabaseConnection) {
        if self.last_refresh.elapsed() < Duration::from_secs(CONFIG_CACHE_TTL_SECS) {
            return;
        }

        if let Ok(Some(val)) = app_metadata_service::get_value(db, COMMAND_PREFIX_KEY).await {
            self.prefix = val;
        }
        if let Ok(Some(val)) = app_metadata_service::get_value(db, MESSAGE_LANGUAGE_KEY).await {
            self.lang = Lang::from_str_lossy(&val);
        }

        self.last_refresh = Instant::now();
    }
}

pub fn spawn_command_dispatcher(
    mut command_rx: mpsc::Receiver<IncomingCommand>,
    manager: ChatChannelManager,
    db_conn: DatabaseConnection,
    data_dir: PathBuf,
    conn_mgr: ConnectionManager,
    emitter: EventEmitter,
    bridge: Arc<Mutex<SessionBridge>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut config = CommandConfigCache::new();

        while let Some(cmd) = command_rx.recv().await {
            let text = cmd.command_text.trim();
            tracing::info!(
                "[ChatChannel] received command from channel={} sender={}: {:?}",
                cmd.channel_id,
                cmd.sender_id,
                text
            );

            // Log inbound command
            let _ = chat_channel_message_log_service::create_log(
                &db_conn,
                cmd.channel_id,
                "inbound",
                "command_query",
                text,
                "sent",
                None,
            )
            .await;

            config.refresh_if_needed(&db_conn).await;

            let response = dispatch_command(
                &cmd,
                &config.prefix,
                &db_conn,
                &manager,
                &conn_mgr,
                &emitter,
                &bridge,
                &data_dir,
                config.lang,
            )
            .await;

            if response.message.is_none()
                && response.extra_messages.is_empty()
                && response.post_action.is_none()
            {
                tracing::debug!("[ChatChannel] dispatch result: no response");
                continue;
            };

            let conversation_id = response.conversation_id;
            let mut messages = Vec::new();
            if let Some(message) = response.message {
                messages.push((message, response.target));
            }
            messages.extend(response.extra_messages);

            for (message, target) in messages {
                send_dispatch_message(
                    &db_conn,
                    &manager,
                    cmd.channel_id,
                    text,
                    message,
                    target,
                    conversation_id,
                )
                .await;
            }

            if let Some(action) = response.post_action {
                if let Some((message, target)) =
                    session_commands::handle_post_action(action, &db_conn, &conn_mgr, &bridge).await
                {
                    send_dispatch_message(
                        &db_conn,
                        &manager,
                        cmd.channel_id,
                        text,
                        DispatchMessage::Rich(message),
                        target,
                        conversation_id,
                    )
                    .await;
                }
            }
        }
    })
}

async fn send_dispatch_message(
    db: &DatabaseConnection,
    manager: &ChatChannelManager,
    channel_id: i32,
    command_text: &str,
    message: DispatchMessage,
    target: ChannelMessageTarget,
    conversation_id: Option<i32>,
) {
    tracing::info!(
        "[ChatChannel] dispatch result: title={:?}, body_len={}",
        message.title(),
        message.body_len()
    );

    let send_result = match &message {
        DispatchMessage::Rich(message) => manager.send_to_target(&target, message).await,
        DispatchMessage::Interactive(message) => {
            manager.send_interactive_to_target(&target, message).await
        }
    };
    if let (Ok(sent), Some(conversation_id)) = (&send_result, conversation_id) {
        remember_sent_message(db, &target, &sent.0, conversation_id).await;
    }
    let (status, error_detail) = match &send_result {
        Ok(_) => ("sent", None),
        Err(e) => {
            tracing::error!(
                "[ChatChannel] failed to send response for {:?} to channel {}: {e}",
                command_text,
                channel_id
            );
            ("failed", Some(e.to_string()))
        }
    };

    let _ = chat_channel_message_log_service::create_log(
        db,
        channel_id,
        "outbound",
        "command_response",
        &message.to_plain_text(),
        status,
        error_detail,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_command(
    cmd: &IncomingCommand,
    prefix: &str,
    db: &DatabaseConnection,
    manager: &ChatChannelManager,
    conn_mgr: &ConnectionManager,
    emitter: &EventEmitter,
    bridge: &Arc<Mutex<SessionBridge>>,
    data_dir: &Path,
    lang: Lang,
) -> DispatchResponse {
    let text = cmd.command_text.trim();
    let channel_id = cmd.channel_id;
    let sender_id = cmd.sender_id.as_str();
    let target = &cmd.target;

    if let Some(data) = cmd.callback_data.as_deref() {
        return DispatchResponse::current(
            session_commands::handle_callback(db, data, channel_id, sender_id, lang, prefix).await,
            target,
        );
    }

    // Strip prefix; if text doesn't start with it, try as follow-up
    let without_prefix = match text.strip_prefix(prefix) {
        Some(rest) => rest,
        None => {
            // Forum topics stay 1:1 with their bound session. Folder-bound
            // chats (matched by inbound chat_id) quote → that session, else
            // reuse the folder's latest conversation within 30 minutes or
            // start a new one.
            if target.is_telegram_forum_topic() {
                return DispatchResponse::current(
                    session_commands::handle_followup(session_commands::FollowupRequest {
                        db,
                        text,
                        channel_id,
                        sender_id,
                        target,
                        conn_mgr,
                        emitter,
                        bridge,
                        data_dir,
                        lang,
                        prefix,
                    })
                    .await,
                    target,
                );
            }

            match route_folder_inbound(db, cmd).await {
                Ok(Some(plan)) => {
                    let result = session_commands::handle_folder_bound_inbound(
                        db, cmd, plan, manager, conn_mgr, emitter, bridge, lang, prefix, data_dir,
                    )
                    .await;
                    return DispatchResponse::from_command_result(result);
                }
                Ok(None) => {}
                Err(e) => {
                    return DispatchResponse::current(
                        RichMessage::error(format!(
                            "{}{e}",
                            i18n::failed_to_load_context_label(lang)
                        )),
                        target,
                    );
                }
            }

            if target.is_telegram_general_topic() {
                return DispatchResponse::none(target);
            }

            // Check if sender has an active session for follow-up
            let has_session = {
                let guard = bridge.lock().await;
                guard.find_by_sender(channel_id, sender_id).is_some()
            };
            if has_session {
                return DispatchResponse::current(
                    session_commands::handle_followup(session_commands::FollowupRequest {
                        db,
                        text,
                        channel_id,
                        sender_id,
                        target,
                        conn_mgr,
                        emitter,
                        bridge,
                        data_dir,
                        lang,
                        prefix,
                    })
                    .await,
                    target,
                );
            }
            return DispatchResponse::current(command_handlers::handle_help(prefix, lang), target);
        }
    };

    let parts: Vec<&str> = without_prefix.splitn(2, ' ').collect();
    let command = parts[0].to_lowercase();
    let args = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match command.as_str() {
        // Existing commands
        "search" => {
            if args.is_empty() {
                DispatchResponse::current(
                    RichMessage::info(i18n::search_usage(lang, prefix))
                        .with_title(i18n::invalid_args_title(lang)),
                    target,
                )
            } else {
                DispatchResponse::current(
                    command_handlers::handle_search(db, args, lang).await,
                    target,
                )
            }
        }
        "today" => {
            DispatchResponse::current(command_handlers::handle_today(db, lang).await, target)
        }
        "status" => {
            DispatchResponse::current(command_handlers::handle_status(manager, lang).await, target)
        }
        "help" | "start" => {
            DispatchResponse::current(command_handlers::handle_help(prefix, lang), target)
        }

        // Session commands
        "folder" => {
            if args.is_empty() {
                DispatchResponse::from_session_message(
                    session_commands::handle_folder_picker(db, channel_id, sender_id, lang, prefix)
                        .await,
                    target,
                )
            } else {
                DispatchResponse::current(
                    session_commands::handle_folder(db, args, channel_id, sender_id, lang, prefix)
                        .await,
                    target,
                )
            }
        }
        "agent" => {
            if args.is_empty() {
                DispatchResponse::from_session_message(
                    session_commands::handle_agent_picker(db, channel_id, sender_id, lang, prefix)
                        .await,
                    target,
                )
            } else {
                DispatchResponse::current(
                    session_commands::handle_agent(db, args, channel_id, sender_id, lang, prefix)
                        .await,
                    target,
                )
            }
        }
        "task" | "do" => {
            let result = session_commands::handle_task(
                db, args, channel_id, sender_id, target, manager, conn_mgr, emitter, bridge, lang,
                prefix, data_dir,
            )
            .await;
            DispatchResponse::from_command_result(result)
        }
        "sessions" => DispatchResponse::current(
            session_commands::handle_sessions(db, channel_id, sender_id, target, lang, prefix)
                .await,
            target,
        ),
        "resume" => DispatchResponse::current(
            session_commands::handle_resume(
                db, args, channel_id, sender_id, target, manager, conn_mgr, emitter, bridge, lang,
                prefix, data_dir,
            )
            .await,
            target,
        ),
        "cancel" => DispatchResponse::current(
            session_commands::handle_cancel(
                db, channel_id, sender_id, target, conn_mgr, bridge, lang,
            )
            .await,
            target,
        ),
        "approve" => {
            let always = args.eq_ignore_ascii_case("always");
            DispatchResponse::current(
                session_commands::handle_permission_response(
                    true, always, db, channel_id, sender_id, target, conn_mgr, bridge, lang,
                )
                .await,
                target,
            )
        }
        "deny" => DispatchResponse::current(
            session_commands::handle_permission_response(
                false, false, db, channel_id, sender_id, target, conn_mgr, bridge, lang,
            )
            .await,
            target,
        ),

        _ => DispatchResponse::current(
            RichMessage::info(i18n::unknown_command(lang, prefix, &command))
                .with_title(i18n::unknown_command_title(lang)),
            target,
        ),
    }
}

struct DispatchResponse {
    message: Option<DispatchMessage>,
    target: ChannelMessageTarget,
    extra_messages: Vec<(DispatchMessage, ChannelMessageTarget)>,
    post_action: Option<session_commands::CommandPostAction>,
    conversation_id: Option<i32>,
}

impl DispatchResponse {
    fn current(message: RichMessage, target: &ChannelMessageTarget) -> Self {
        Self {
            message: Some(DispatchMessage::Rich(message)),
            target: target.clone(),
            extra_messages: Vec::new(),
            post_action: None,
            conversation_id: None,
        }
    }

    fn from_session_message(
        message: session_commands::SessionCommandMessage,
        target: &ChannelMessageTarget,
    ) -> Self {
        Self {
            message: Some(match message {
                session_commands::SessionCommandMessage::Rich(message) => {
                    DispatchMessage::Rich(message)
                }
                session_commands::SessionCommandMessage::Interactive(message) => {
                    DispatchMessage::Interactive(message)
                }
            }),
            target: target.clone(),
            extra_messages: Vec::new(),
            post_action: None,
            conversation_id: None,
        }
    }

    fn from_command_result(result: session_commands::CommandMessageResult) -> Self {
        Self {
            message: Some(DispatchMessage::Rich(result.message)),
            target: result.response_target,
            extra_messages: result
                .extra_responses
                .into_iter()
                .map(|(message, target)| (DispatchMessage::Rich(message), target))
                .collect(),
            post_action: result.post_action,
            conversation_id: result.conversation_id,
        }
    }

    fn none(target: &ChannelMessageTarget) -> Self {
        Self {
            message: None,
            target: target.clone(),
            extra_messages: Vec::new(),
            post_action: None,
            conversation_id: None,
        }
    }
}

enum DispatchMessage {
    Rich(RichMessage),
    Interactive(InteractiveMessage),
}

impl DispatchMessage {
    fn title(&self) -> Option<&String> {
        match self {
            Self::Rich(message) => message.title.as_ref(),
            Self::Interactive(message) => message.base.title.as_ref(),
        }
    }

    fn body_len(&self) -> usize {
        match self {
            Self::Rich(message) => message.body.len(),
            Self::Interactive(message) => message.base.body.len(),
        }
    }

    fn to_plain_text(&self) -> String {
        match self {
            Self::Rich(message) => message.to_plain_text(),
            Self::Interactive(message) => message.to_rich_fallback().to_plain_text(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::connection::ConnectionCommand;
    use crate::acp::types::PromptInputBlock;
    use crate::chat_channel::session_bridge::ActiveSession;
    use crate::db::service::{
        chat_channel_service, folder_chat_channel_service, sender_context_service,
    };
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::agent::AgentType;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use std::time::Instant;

    async fn seed_chat_channel(db: &crate::db::AppDatabase) -> i32 {
        chat_channel_service::create(
            &db.conn,
            "Telegram test".to_string(),
            "telegram".to_string(),
            serde_json::json!({ "chat_id": "-100123", "topic_mode": true }).to_string(),
            true,
            false,
            None,
        )
        .await
        .expect("seed chat channel")
        .id
    }

    async fn dispatch(
        db: &crate::db::AppDatabase,
        cmd: &IncomingCommand,
        bridge: &Arc<Mutex<SessionBridge>>,
        conn_mgr: &ConnectionManager,
    ) -> DispatchResponse {
        dispatch_command(
            cmd,
            "/",
            &db.conn,
            &ChatChannelManager::new(),
            conn_mgr,
            &EventEmitter::Noop,
            bridge,
            std::path::Path::new("/tmp/codeg-dispatch-data"),
            Lang::En,
        )
        .await
    }

    #[tokio::test]
    async fn callback_data_dispatches_without_command_prefix() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_id = seed_folder(&db, "/tmp/codeg-dispatch-callback").await;
        let target = ChannelMessageTarget::telegram_general(channel_id, "-100123");
        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        let mut cmd = IncomingCommand::plain(
            channel_id,
            "sender-1",
            "cfg:folder:ignored-by-callback-data",
            target,
        );
        cmd.callback_data = Some(format!("cfg:folder:{folder_id}"));

        let response = dispatch(&db, &cmd, &bridge, &ConnectionManager::new()).await;
        let ctx = sender_context_service::get_or_create(&db.conn, channel_id, "sender-1")
            .await
            .expect("context");

        assert!(matches!(response.message, Some(DispatchMessage::Rich(_))));
        assert_eq!(ctx.current_folder_id, Some(folder_id));
    }

    #[tokio::test]
    async fn general_topic_plain_text_returns_no_response() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let target = ChannelMessageTarget::telegram_general(channel_id, "-100123");
        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        let cmd = IncomingCommand::plain(channel_id, "sender-1", "hello group", target.clone());

        let response = dispatch(&db, &cmd, &bridge, &ConnectionManager::new()).await;

        assert!(response.message.is_none());
        assert_eq!(response.target, target);
    }

    #[tokio::test]
    async fn folder_bound_plain_text_continues_latest_conversation() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_id = seed_folder(&db, "/tmp/codeg-dispatch-folder-bound").await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::OpenCode).await;
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("-100123".into()),
            }],
        )
        .await
        .expect("bind folder");

        let target = ChannelMessageTarget::telegram_general(channel_id, "-100123");
        let conn_mgr = ConnectionManager::new();
        let mut rx = conn_mgr
            .insert_test_connection_live(
                "conn-folder-bound",
                AgentType::OpenCode,
                Some(std::path::PathBuf::from("/tmp/codeg-dispatch-folder-bound")),
                EventEmitter::Noop,
            )
            .await;
        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        bridge.lock().await.register(
            "conn-folder-bound".to_string(),
            ActiveSession {
                channel_id,
                sender_id: "sender-1".to_string(),
                target: target.clone(),
                conversation_id: conv_id,
                connection_id: "conn-folder-bound".to_string(),
                agent_type: AgentType::OpenCode,
                content_buffer: String::new(),
                tool_calls: Vec::new(),
                tool_call_inputs: std::collections::HashMap::new(),
                delegation_rendered: std::collections::HashSet::new(),
                last_flushed: Instant::now(),
                pending_prompt: None,
                permission_pending: None,
            },
        );

        let cmd = IncomingCommand::plain(channel_id, "sender-2", "keep going", target);
        let response = dispatch(&db, &cmd, &bridge, &conn_mgr).await;
        let Some(DispatchMessage::Rich(message)) = response.message else {
            panic!("expected rich response");
        };
        assert_eq!(
            message.body,
            crate::chat_channel::i18n::message_sent(Lang::En)
        );
        assert_eq!(response.conversation_id, Some(conv_id));

        let command = rx.recv().await.expect("prompt command");
        let ConnectionCommand::Prompt { blocks, .. } = command else {
            panic!("expected prompt command");
        };
        assert!(matches!(
            &blocks[0],
            PromptInputBlock::Text { text } if text == "keep going"
        ));
    }

    #[tokio::test]
    async fn folder_bound_quoted_message_continues_mapped_session() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_id = seed_folder(&db, "/tmp/codeg-dispatch-quote").await;
        let quoted = seed_conversation(&db, folder_id, AgentType::OpenCode).await;
        let _latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("oc_bound".into()),
            }],
        )
        .await
        .expect("bind folder");
        crate::db::service::chat_channel_message_map_service::upsert(
            &db.conn,
            channel_id,
            "om_parent",
            quoted,
        )
        .await
        .expect("map quoted message");

        let target = ChannelMessageTarget::with_chat_id(channel_id, "oc_bound");
        let conn_mgr = ConnectionManager::new();
        let mut rx = conn_mgr
            .insert_test_connection_live(
                "conn-quoted",
                AgentType::OpenCode,
                Some(std::path::PathBuf::from("/tmp/codeg-dispatch-quote")),
                EventEmitter::Noop,
            )
            .await;
        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        bridge.lock().await.register(
            "conn-quoted".to_string(),
            ActiveSession {
                channel_id,
                sender_id: "sender-1".to_string(),
                target: target.clone(),
                conversation_id: quoted,
                connection_id: "conn-quoted".to_string(),
                agent_type: AgentType::OpenCode,
                content_buffer: String::new(),
                tool_calls: Vec::new(),
                tool_call_inputs: std::collections::HashMap::new(),
                delegation_rendered: std::collections::HashSet::new(),
                last_flushed: Instant::now(),
                pending_prompt: None,
                permission_pending: None,
            },
        );

        let mut cmd = IncomingCommand::plain(channel_id, "sender-9", "reply on quote", target);
        cmd.quoted_message_id = Some("om_parent".into());
        cmd.provider_message_id = Some("om_child".into());
        let response = dispatch(&db, &cmd, &bridge, &conn_mgr).await;
        assert_eq!(response.conversation_id, Some(quoted));
        assert_eq!(
            crate::db::service::chat_channel_message_map_service::conversation_id_for(
                &db.conn, channel_id, "om_child",
            )
            .await
            .unwrap(),
            Some(quoted)
        );

        let command = rx.recv().await.expect("prompt command");
        let ConnectionCommand::Prompt { blocks, .. } = command else {
            panic!("expected prompt command");
        };
        assert!(matches!(
            &blocks[0],
            PromptInputBlock::Text { text } if text == "reply on quote"
        ));
    }

    #[tokio::test]
    async fn folder_bound_stale_latest_starts_new_without_help() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_id = seed_folder(&db, "/tmp/codeg-dispatch-stale").await;
        let latest = seed_conversation(&db, folder_id, AgentType::Codex).await;
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("oc_bound".into()),
            }],
        )
        .await
        .expect("bind folder");

        let conv = crate::db::entities::conversation::Entity::find_by_id(latest)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let mut active: crate::db::entities::conversation::ActiveModel = conv.into();
        active.updated_at = Set(chrono::Utc::now() - chrono::Duration::minutes(45));
        active.update(&db.conn).await.unwrap();

        let target = ChannelMessageTarget::with_chat_id(channel_id, "oc_bound");
        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        let cmd = IncomingCommand::plain(channel_id, "sender-1", "new work please", target);
        let response = dispatch(&db, &cmd, &bridge, &ConnectionManager::new()).await;
        let Some(DispatchMessage::Rich(message)) = response.message else {
            panic!("expected rich response");
        };
        assert!(
            message.body.contains("No agent selected")
                || message.body.contains("No folder selected"),
            "stale idle window should start a folder task, not help: {}",
            message.body
        );
        let ctx = sender_context_service::get_or_create(&db.conn, channel_id, "sender-1")
            .await
            .expect("context");
        assert_eq!(ctx.current_folder_id, Some(folder_id));
    }

    #[tokio::test]
    async fn folder_bound_does_not_steal_busy_desktop_session() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_path = "/tmp/codeg-dispatch-busy-desktop";
        let folder_id = seed_folder(&db, folder_path).await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::OpenCode).await;
        crate::db::service::conversation_service::bind_external_id(
            &db.conn,
            conv_id,
            "sess-desktop",
            &[],
        )
        .await
        .expect("bind session");
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("-100123".into()),
            }],
        )
        .await
        .expect("bind folder");

        let target = ChannelMessageTarget::with_chat_id(channel_id, "-100123");
        let conn_mgr = ConnectionManager::new();
        let _rx = conn_mgr
            .insert_test_connection_live(
                "conn-desktop",
                AgentType::OpenCode,
                Some(std::path::PathBuf::from(folder_path)),
                EventEmitter::Noop,
            )
            .await;
        {
            let state = conn_mgr.get_state("conn-desktop").await.unwrap();
            let mut snap = state.write().await;
            snap.external_id = Some("sess-desktop".into());
            snap.conversation_id = Some(conv_id);
            snap.turn_in_flight = true;
        }

        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        let cmd = IncomingCommand::plain(channel_id, "sender-feishu", "介绍一下你的能力", target);
        let response = dispatch(&db, &cmd, &bridge, &conn_mgr).await;
        let Some(DispatchMessage::Rich(message)) = response.message else {
            panic!("expected rich response");
        };
        assert!(
            !message.body.contains("still processing") && !message.body.contains("请稍后再发送"),
            "must not swallow the Feishu follow-up into a busy desktop session: {}",
            message.body
        );
        assert!(
            message.body.contains("No agent selected")
                || message.body.contains("No folder selected")
                || message.title.as_deref() == Some("开始任务")
                || message
                    .title
                    .as_deref()
                    .is_some_and(|t| t.contains("Task") || t.contains("Started")),
            "busy desktop session should start a new folder task, got title={:?} body={}",
            message.title,
            message.body
        );
    }

    #[tokio::test]
    async fn folder_bound_quoted_does_not_steal_busy_desktop_session() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_path = "/tmp/codeg-dispatch-quoted-busy";
        let folder_id = seed_folder(&db, folder_path).await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::OpenCode).await;
        crate::db::service::conversation_service::bind_external_id(
            &db.conn,
            conv_id,
            "sess-quoted-desktop",
            &[],
        )
        .await
        .expect("bind session");
        crate::db::service::chat_channel_message_map_service::upsert(
            &db.conn,
            channel_id,
            "om_hi_done",
            conv_id,
        )
        .await
        .expect("map quoted card");
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("-100123".into()),
            }],
        )
        .await
        .expect("bind folder");

        let target = ChannelMessageTarget::with_chat_id(channel_id, "-100123");
        let conn_mgr = ConnectionManager::new();
        let _rx = conn_mgr
            .insert_test_connection_live(
                "conn-quoted-desktop",
                AgentType::OpenCode,
                Some(std::path::PathBuf::from(folder_path)),
                EventEmitter::Noop,
            )
            .await;
        {
            let state = conn_mgr.get_state("conn-quoted-desktop").await.unwrap();
            let mut snap = state.write().await;
            snap.external_id = Some("sess-quoted-desktop".into());
            snap.conversation_id = Some(conv_id);
            snap.turn_in_flight = true;
        }

        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        let mut cmd =
            IncomingCommand::plain(channel_id, "sender-feishu", "介绍一下你的能力", target);
        cmd.quoted_message_id = Some("om_hi_done".into());
        let response = dispatch(&db, &cmd, &bridge, &conn_mgr).await;
        let Some(DispatchMessage::Rich(message)) = response.message else {
            panic!("expected rich response");
        };
        assert!(
            !message.body.contains("still processing") && !message.body.contains("请稍后再发送"),
            "quoting a busy desktop session must not return retry-later: {}",
            message.body
        );
        assert!(
            message.body.contains("No agent selected")
                || message.body.contains("No folder selected")
                || message.title.as_deref().is_some_and(|t| t.contains("Task")
                    || t.contains("Started")
                    || t == "开始任务"),
            "quoted busy desktop session should start a new folder task, got title={:?} body={}",
            message.title,
            message.body
        );
        assert!(
            bridge.lock().await.find_by_conversation(conv_id).is_none(),
            "must not attach the desktop connection to the chat bridge"
        );
    }

    #[tokio::test]
    async fn folder_bound_busy_bridged_session_starts_new_instead_of_retry_later() {
        let db = fresh_in_memory_db().await;
        let channel_id = seed_chat_channel(&db).await;
        let folder_path = "/tmp/codeg-dispatch-bridged-busy";
        let folder_id = seed_folder(&db, folder_path).await;
        let conv_id = seed_conversation(&db, folder_id, AgentType::OpenCode).await;
        folder_chat_channel_service::set_bindings(
            &db.conn,
            folder_id,
            &[folder_chat_channel_service::FolderChannelBinding {
                channel_id,
                chat_id: Some("-100123".into()),
            }],
        )
        .await
        .expect("bind folder");

        let target = ChannelMessageTarget::with_chat_id(channel_id, "-100123");
        let conn_mgr = ConnectionManager::new();
        let _rx = conn_mgr
            .insert_test_connection_live(
                "conn-bridged-busy",
                AgentType::OpenCode,
                Some(std::path::PathBuf::from(folder_path)),
                EventEmitter::Noop,
            )
            .await;
        {
            let state = conn_mgr.get_state("conn-bridged-busy").await.unwrap();
            let mut snap = state.write().await;
            snap.conversation_id = Some(conv_id);
            snap.turn_in_flight = true;
        }

        let bridge = Arc::new(Mutex::new(SessionBridge::new()));
        bridge.lock().await.register(
            "conn-bridged-busy".to_string(),
            ActiveSession {
                channel_id,
                sender_id: "other-sender".to_string(),
                target: target.clone(),
                conversation_id: conv_id,
                connection_id: "conn-bridged-busy".to_string(),
                agent_type: AgentType::OpenCode,
                content_buffer: String::new(),
                tool_calls: Vec::new(),
                tool_call_inputs: std::collections::HashMap::new(),
                delegation_rendered: std::collections::HashSet::new(),
                last_flushed: Instant::now(),
                pending_prompt: None,
                permission_pending: None,
            },
        );

        let cmd = IncomingCommand::plain(channel_id, "sender-feishu", "介绍一下你的能力", target);
        let response = dispatch(&db, &cmd, &bridge, &conn_mgr).await;
        let Some(DispatchMessage::Rich(message)) = response.message else {
            panic!("expected rich response");
        };
        assert!(
            !message.body.contains("still processing") && !message.body.contains("请稍后再发送"),
            "folder inbound must not leave the Feishu user on retry-later: {}",
            message.body
        );
        assert!(
            message.body.contains("No agent selected")
                || message.body.contains("No folder selected")
                || message.title.as_deref().is_some_and(|t| t.contains("Task")
                    || t.contains("Started")
                    || t == "开始任务"),
            "busy bridged session should start a new folder task, got title={:?} body={}",
            message.title,
            message.body
        );
    }
}

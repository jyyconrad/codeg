use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, OnceCell};

use crate::chat_channel::error::ChatChannelError;
use crate::chat_channel::traits::ChatChannelBackend;
use crate::chat_channel::types::*;

pub struct TelegramBackend {
    bot_token: String,
    chat_id: String,
    topic_mode: bool,
    client: reqwest::Client,
    status: Arc<Mutex<ChannelConnectionStatus>>,
    channel_id: i32,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    /// Cached canonical numeric chat id, resolved from an `@username`
    /// `chat_id` via `getChat`. Empty for numeric configs (never populated).
    resolved_chat_id: OnceCell<String>,
}

impl TelegramBackend {
    pub fn new(channel_id: i32, bot_token: String, chat_id: String, topic_mode: bool) -> Self {
        Self {
            bot_token,
            chat_id,
            topic_mode,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            status: Arc::new(Mutex::new(ChannelConnectionStatus::Disconnected)),
            channel_id,
            shutdown_tx: Arc::new(Mutex::new(None)),
            resolved_chat_id: OnceCell::new(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.bot_token, method)
    }

    /// Canonical **numeric** chat id used for topic-binding keys.
    ///
    /// Telegram delivers inbound updates keyed by the numeric `/chat/id`, but a
    /// channel may be configured with an `@username`. Topic bindings are written
    /// from `create_thread`'s returned target and later looked up against inbound
    /// targets, so both sides must agree on the id form or the lookup misses
    /// (leaving follow-ups "unbound" and spawning a duplicate agent). Resolve
    /// `@username` → numeric once via `getChat` and cache it; numeric ids pass
    /// through untouched. On failure the cell stays empty so the next call
    /// retries.
    async fn canonical_chat_id(&self) -> Result<String, ChatChannelError> {
        if !self.chat_id.trim_start().starts_with('@') {
            return Ok(self.chat_id.clone());
        }
        self.resolved_chat_id
            .get_or_try_init(|| async {
                let body = serde_json::json!({ "chat_id": self.chat_id });
                let resp = self
                    .client
                    .post(self.api_url("getChat"))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
                    })?;
                let result: serde_json::Value = resp.json().await.map_err(|e| {
                    ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
                })?;
                result
                    .pointer("/result/id")
                    .and_then(|v| v.as_i64())
                    .map(|id| id.to_string())
                    .ok_or_else(|| {
                        ChatChannelError::SendFailed(
                            "Telegram getChat returned no numeric chat id".to_string(),
                        )
                    })
            })
            .await
            .cloned()
    }

    async fn send_text(
        &self,
        text: &str,
        parse_mode: Option<&str>,
        target: Option<&ChannelMessageTarget>,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_text_with_reply_markup(text, parse_mode, target, None)
            .await
    }

    async fn send_text_with_reply_markup(
        &self,
        text: &str,
        parse_mode: Option<&str>,
        target: Option<&ChannelMessageTarget>,
        reply_markup: Option<serde_json::Value>,
    ) -> Result<SentMessageId, ChatChannelError> {
        let body =
            telegram_send_message_body(&self.chat_id, text, parse_mode, target, reply_markup)?;

        let resp = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
            })?;

        let result: serde_json::Value = resp.json().await.map_err(|e| {
            ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
        })?;

        if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(ChatChannelError::SendFailed(desc.to_string()));
        }

        let message_id = result
            .pointer("/result/message_id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_default();

        Ok(SentMessageId(message_id))
    }

    async fn send_interactive_message_with_target(
        &self,
        message: &InteractiveMessage,
        target: Option<&ChannelMessageTarget>,
    ) -> Result<SentMessageId, ChatChannelError> {
        let reply_markup = telegram_inline_keyboard(message);
        let html_text = format_telegram_html(&message.base);
        let result = self
            .send_text_with_reply_markup(&html_text, Some("HTML"), target, reply_markup.clone())
            .await;

        match result {
            Ok(id) => Ok(id),
            Err(e) => {
                tracing::warn!(
                    "[Telegram] HTML interactive send failed: {e}, retrying as plain text"
                );
                self.send_text_with_reply_markup(
                    &message.base.to_plain_text(),
                    None,
                    target,
                    reply_markup,
                )
                .await
            }
        }
    }

    async fn send_rich_message_with_target(
        &self,
        message: &RichMessage,
        target: Option<&ChannelMessageTarget>,
    ) -> Result<SentMessageId, ChatChannelError> {
        let html_text = format_telegram_html(message);
        let result = self.send_text(&html_text, Some("HTML"), target).await;

        match result {
            Ok(id) => Ok(id),
            Err(e) => {
                tracing::warn!("[Telegram] HTML send failed: {e}, retrying as plain text");
                let plain_text = message.to_plain_text();
                self.send_text(&plain_text, None, target).await
            }
        }
    }
}

#[async_trait]
impl ChatChannelBackend for TelegramBackend {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
        command_tx: mpsc::Sender<IncomingCommand>,
    ) -> Result<(), ChatChannelError> {
        *self.status.lock().await = ChannelConnectionStatus::Connecting;

        // Verify bot token and extract bot username for group @mention filtering
        let resp = self
            .client
            .get(self.api_url("getMe"))
            .send()
            .await
            .map_err(|e| {
                ChatChannelError::ConnectionFailed(redact_token(e.to_string(), &self.bot_token))
            })?;

        let me_body: serde_json::Value = resp.json().await.map_err(|e| {
            ChatChannelError::ConnectionFailed(redact_token(e.to_string(), &self.bot_token))
        })?;

        if me_body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            *self.status.lock().await = ChannelConnectionStatus::Error;
            return Err(ChatChannelError::AuthenticationFailed(
                "Invalid bot token".to_string(),
            ));
        }

        let bot_username = me_body
            .pointer("/result/username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();

        *self.status.lock().await = ChannelConnectionStatus::Connected;

        // Start long-polling loop
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let client = self.client.clone();
        let bot_token = self.bot_token.clone();
        let channel_id = self.channel_id;
        // Resolve `@username` → numeric once so inbound targets/matching use the
        // same id form the topic bindings are stored under. Falls back to the
        // raw value on failure (`telegram_message_chat_matches` still matches via
        // /chat/username).
        let configured_chat_id = self
            .canonical_chat_id()
            .await
            .unwrap_or_else(|_| self.chat_id.clone());
        let topic_mode = self.topic_mode;
        let status = self.status.clone();

        tokio::spawn(async move {
            let mut offset: i64 = 0;
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let url = format!("https://api.telegram.org/bot{}/getUpdates", bot_token);
                let body = serde_json::json!({
                    "timeout": 30,
                    "offset": offset,
                    "allowed_updates": ["message", "callback_query"],
                });

                let result = tokio::select! {
                    r = client.post(&url).json(&body).send() => r,
                    _ = shutdown_rx.changed() => break,
                };

                match result {
                    Ok(resp) => {
                        // Recover from error state after successful poll
                        {
                            let mut s = status.lock().await;
                            if *s == ChannelConnectionStatus::Error {
                                *s = ChannelConnectionStatus::Connected;
                            }
                        }

                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            if let Some(updates) = body.get("result").and_then(|r| r.as_array()) {
                                if !updates.is_empty() {
                                    tracing::info!("[Telegram] got {} update(s)", updates.len());
                                }
                                for update in updates {
                                    if let Some(uid) =
                                        update.get("update_id").and_then(|u| u.as_i64())
                                    {
                                        offset = uid + 1;
                                    }
                                    if let Some(message) = update.get("message") {
                                        if !telegram_message_chat_matches(
                                            message,
                                            &configured_chat_id,
                                        ) {
                                            tracing::debug!(
                                                "[Telegram] skipped message from unconfigured chat"
                                            );
                                            continue;
                                        }

                                        if let Some(text) =
                                            message.get("text").and_then(|t| t.as_str())
                                        {
                                            // Group chat filtering: only process if @bot is mentioned
                                            let chat_type = message
                                                .pointer("/chat/type")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("private");

                                            if !telegram_should_process_text_message(
                                                chat_type,
                                                text,
                                                &bot_username,
                                                topic_mode,
                                            ) {
                                                tracing::debug!("[Telegram] skipped group msg without @bot: {text}");
                                                continue;
                                            }

                                            // Strip @bot_username from command text (case-insensitive)
                                            let clean_text = strip_bot_mention(text, &bot_username);

                                            let sender_id = message
                                                .pointer("/from/id")
                                                .and_then(json_scalar_to_string)
                                                .unwrap_or_default();
                                            let target = telegram_message_target(
                                                channel_id,
                                                &configured_chat_id,
                                                topic_mode,
                                                message,
                                            );
                                            tracing::debug!("[Telegram] dispatching: {clean_text}");
                                            let send_result = command_tx
                                                .send(IncomingCommand {
                                                    channel_id,
                                                    sender_id,
                                                    command_text: clean_text,
                                                    callback_data: None,
                                                    target,
                                                    metadata: update.clone(),
                                                })
                                                .await;
                                            if let Err(e) = send_result {
                                                tracing::error!(
                                                    "[Telegram] command_tx.send failed: {e}"
                                                );
                                            }
                                        } else {
                                            tracing::info!(
                                                "[Telegram] message update without text"
                                            );
                                        }
                                    } else if let Some(callback) = update.get("callback_query") {
                                        let Some(message) = callback.get("message") else {
                                            tracing::debug!(
                                                "[Telegram] skipped callback without message"
                                            );
                                            continue;
                                        };
                                        if !telegram_message_chat_matches(
                                            message,
                                            &configured_chat_id,
                                        ) {
                                            tracing::debug!(
                                                "[Telegram] skipped callback from unconfigured chat"
                                            );
                                            continue;
                                        }
                                        if let Some(callback_id) =
                                            callback.get("id").and_then(|v| v.as_str())
                                        {
                                            answer_callback_query(&client, &bot_token, callback_id)
                                                .await;
                                        }
                                        let Some(data) =
                                            callback.get("data").and_then(|v| v.as_str())
                                        else {
                                            tracing::debug!(
                                                "[Telegram] skipped callback without data"
                                            );
                                            continue;
                                        };
                                        let sender_id = callback
                                            .pointer("/from/id")
                                            .and_then(json_scalar_to_string)
                                            .unwrap_or_default();
                                        let target = telegram_message_target(
                                            channel_id,
                                            &configured_chat_id,
                                            topic_mode,
                                            message,
                                        );
                                        tracing::debug!("[Telegram] dispatching callback: {data}");
                                        let send_result = command_tx
                                            .send(IncomingCommand {
                                                channel_id,
                                                sender_id,
                                                command_text: data.to_string(),
                                                callback_data: Some(data.to_string()),
                                                target,
                                                metadata: update.clone(),
                                            })
                                            .await;
                                        if let Err(e) = send_result {
                                            tracing::error!(
                                                "[Telegram] command_tx.send failed: {e}"
                                            );
                                        }
                                    } else {
                                        tracing::info!(
                                            "[Telegram] update without message/callback_query"
                                        );
                                    }
                                }
                            }
                        } else {
                            tracing::error!("[Telegram] failed to parse response body");
                        }
                    }
                    Err(e) => {
                        let msg = redact_token(e.to_string(), &bot_token);
                        tracing::error!("[Telegram] polling error: {msg}");
                        *status.lock().await = ChannelConnectionStatus::Error;
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
            }
            *status.lock().await = ChannelConnectionStatus::Disconnected;
        });

        Ok(())
    }

    async fn stop(&self) -> Result<(), ChatChannelError> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(true);
        }
        *self.status.lock().await = ChannelConnectionStatus::Disconnected;
        Ok(())
    }

    async fn status(&self) -> ChannelConnectionStatus {
        *self.status.lock().await
    }

    async fn send_message(&self, text: &str) -> Result<SentMessageId, ChatChannelError> {
        self.send_text(text, None, None).await
    }

    async fn send_rich_message(
        &self,
        message: &RichMessage,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_rich_message_with_target(message, None).await
    }

    async fn send_rich_message_to(
        &self,
        message: &RichMessage,
        target: &ChannelMessageTarget,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_rich_message_with_target(message, Some(target))
            .await
    }

    async fn send_interactive_message(
        &self,
        message: &InteractiveMessage,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_interactive_message_with_target(message, None)
            .await
    }

    async fn send_interactive_message_to(
        &self,
        message: &InteractiveMessage,
        target: &ChannelMessageTarget,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_interactive_message_with_target(message, Some(target))
            .await
    }

    async fn create_thread(&self, title: &str) -> Result<ChannelMessageTarget, ChatChannelError> {
        if !self.topic_mode {
            return Err(ChatChannelError::Unsupported(
                "Telegram topic mode is not enabled".to_string(),
            ));
        }

        // Use the canonical numeric id so the binding written from the returned
        // target matches inbound updates (which are keyed by numeric /chat/id).
        let chat_id = self.canonical_chat_id().await?;
        let body = serde_json::json!({
            "chat_id": chat_id.clone(),
            "name": telegram_topic_title(title),
        });
        let resp = self
            .client
            .post(self.api_url("createForumTopic"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
            })?;
        let result: serde_json::Value = resp.json().await.map_err(|e| {
            ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
        })?;
        if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("failed to create Telegram topic");
            return Err(ChatChannelError::SendFailed(desc.to_string()));
        }
        let thread_id = result
            .pointer("/result/message_thread_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                ChatChannelError::SendFailed(
                    "Telegram createForumTopic returned no message_thread_id".to_string(),
                )
            })?;
        Ok(ChannelMessageTarget::telegram_forum_topic(
            self.channel_id,
            chat_id,
            thread_id.to_string(),
        ))
    }

    async fn edit_thread_title(
        &self,
        target: &ChannelMessageTarget,
        title: &str,
    ) -> Result<(), ChatChannelError> {
        if !target.is_telegram_forum_topic() {
            return Err(ChatChannelError::Unsupported(
                "target is not a Telegram forum topic".to_string(),
            ));
        }
        let thread_id = target
            .thread_key
            .as_deref()
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| ChatChannelError::SendFailed("invalid Telegram topic id".to_string()))?;
        let chat_id = target.chat_id.as_deref().unwrap_or(&self.chat_id);
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_thread_id": thread_id,
            "name": telegram_topic_title(title),
        });
        let resp = self
            .client
            .post(self.api_url("editForumTopic"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
            })?;
        let result: serde_json::Value = resp.json().await.map_err(|e| {
            ChatChannelError::SendFailed(redact_token(e.to_string(), &self.bot_token))
        })?;
        if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(())
        } else {
            let desc = result
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("failed to edit Telegram topic");
            Err(ChatChannelError::SendFailed(desc.to_string()))
        }
    }

    async fn test_connection(&self) -> Result<(), ChatChannelError> {
        let resp = self
            .client
            .get(self.api_url("getMe"))
            .send()
            .await
            .map_err(|e| {
                ChatChannelError::ConnectionFailed(redact_token(e.to_string(), &self.bot_token))
            })?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            ChatChannelError::ConnectionFailed(redact_token(e.to_string(), &self.bot_token))
        })?;

        if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(())
        } else {
            let desc = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("Invalid bot token");
            Err(ChatChannelError::AuthenticationFailed(desc.to_string()))
        }
    }
}

/// Scrub the bot token from an error/log string before it escapes.
///
/// Telegram API URLs embed the token in the path (`/bot<TOKEN>/…`) and
/// `reqwest::Error`'s `Display` appends `for url (<url>)`. That text is written
/// to logs and — for topic create/edit failures — sent into the chat itself, so
/// the raw token must never survive stringification. No-op on an empty token.
fn redact_token(msg: String, token: &str) -> String {
    if token.is_empty() {
        msg
    } else {
        msg.replace(token, "***")
    }
}

/// Strip `@bot_username` from text (case-insensitive).
/// Handles Telegram convention: `/command@botname args` → `/command args`
fn strip_bot_mention(text: &str, bot_username: &str) -> String {
    if bot_username.is_empty() {
        return text.to_string();
    }
    let at_bot = format!("@{}", bot_username);
    let text_lower = text.to_lowercase();
    let at_bot_lower = at_bot.to_lowercase();
    if let Some(pos) = text_lower.find(&at_bot_lower) {
        let mut result = String::with_capacity(text.len());
        result.push_str(&text[..pos]);
        result.push_str(&text[pos + at_bot.len()..]);
        result.trim().to_string()
    } else {
        text.to_string()
    }
}

async fn answer_callback_query(client: &reqwest::Client, bot_token: &str, callback_query_id: &str) {
    let body = serde_json::json!({
        "callback_query_id": callback_query_id,
    });
    let result = client
        .post(format!(
            "https://api.telegram.org/bot{}/answerCallbackQuery",
            bot_token
        ))
        .json(&body)
        .send()
        .await;
    if let Err(e) = result {
        let msg = redact_token(e.to_string(), bot_token);
        tracing::warn!("[Telegram] answerCallbackQuery failed: {msg}");
    }
}

fn telegram_message_chat_matches(message: &serde_json::Value, configured_chat_id: &str) -> bool {
    let configured = configured_chat_id.trim();
    if configured.is_empty() {
        return false;
    }

    if message
        .pointer("/chat/id")
        .and_then(json_scalar_to_string)
        .as_deref()
        == Some(configured)
    {
        return true;
    }

    let configured_username = configured.strip_prefix('@').unwrap_or(configured);
    message
        .pointer("/chat/username")
        .and_then(|v| v.as_str())
        .is_some_and(|username| username.eq_ignore_ascii_case(configured_username))
}

fn telegram_message_target(
    channel_id: i32,
    configured_chat_id: &str,
    topic_mode: bool,
    message: &serde_json::Value,
) -> ChannelMessageTarget {
    if !topic_mode {
        return ChannelMessageTarget::channel(channel_id);
    }

    let chat_id = message
        .pointer("/chat/id")
        .and_then(json_scalar_to_string)
        .unwrap_or_else(|| configured_chat_id.to_string());

    if let Some(thread_key) = message
        .pointer("/message_thread_id")
        .and_then(json_scalar_to_string)
    {
        ChannelMessageTarget::telegram_forum_topic(channel_id, chat_id, thread_key)
    } else {
        ChannelMessageTarget::telegram_general(channel_id, chat_id)
    }
}

fn telegram_should_process_text_message(
    chat_type: &str,
    text: &str,
    bot_username: &str,
    topic_mode: bool,
) -> bool {
    if topic_mode || bot_username.is_empty() || (chat_type != "group" && chat_type != "supergroup")
    {
        return true;
    }

    let at_bot = format!("@{}", bot_username);
    text.to_lowercase().contains(&at_bot.to_lowercase())
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        Some(s.to_string())
    } else if let Some(i) = value.as_i64() {
        Some(i.to_string())
    } else {
        value.as_u64().map(|u| u.to_string())
    }
}

fn telegram_topic_title(title: &str) -> String {
    let title = title.trim();
    let title = if title.is_empty() {
        "Codeg session"
    } else {
        title
    };
    title.chars().take(128).collect()
}

fn telegram_inline_keyboard(message: &InteractiveMessage) -> Option<serde_json::Value> {
    if message.buttons.is_empty() {
        return None;
    }

    let rows: Vec<serde_json::Value> = message
        .buttons
        .chunks(2)
        .map(|chunk| {
            serde_json::Value::Array(
                chunk
                    .iter()
                    .map(|button| {
                        serde_json::json!({
                            "text": button.label,
                            "callback_data": button.id,
                        })
                    })
                    .collect(),
            )
        })
        .collect();

    Some(serde_json::json!({ "inline_keyboard": rows }))
}

fn telegram_send_message_body(
    default_chat_id: &str,
    text: &str,
    parse_mode: Option<&str>,
    target: Option<&ChannelMessageTarget>,
    reply_markup: Option<serde_json::Value>,
) -> Result<serde_json::Value, ChatChannelError> {
    let chat_id = target
        .and_then(|t| t.chat_id.as_deref())
        .unwrap_or(default_chat_id);
    let mut body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    if let Some(mode) = parse_mode {
        body["parse_mode"] = serde_json::Value::String(mode.to_string());
    }
    if let Some(markup) = reply_markup {
        body["reply_markup"] = markup;
    }
    if let Some(target) = target {
        if target.is_telegram_forum_topic() {
            let thread_id = target
                .thread_key
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok())
                .ok_or_else(|| {
                    ChatChannelError::SendFailed(
                        "invalid Telegram message_thread_id target".to_string(),
                    )
                })?;
            body["message_thread_id"] = serde_json::json!(thread_id);
        }
    }

    Ok(body)
}

fn format_telegram_html(msg: &RichMessage) -> String {
    markdown_to_telegram_html(&msg.to_markdown())
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn is_fence_lang_tag(s: &str) -> bool {
    let t = s.trim();
    t.len() < 32
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_' | '.'))
}

fn is_safe_href(url: &str) -> bool {
    let u = url.trim();
    if u.contains('"') || u.contains('<') || u.contains(' ') {
        return false;
    }
    let lower = u.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn strip_fence_language(inner: &str) -> &str {
    if let Some(nl) = inner.find('\n') {
        if is_fence_lang_tag(&inner[..nl]) {
            return &inner[nl + 1..];
        }
    }
    inner
}

fn heading_text(line: &str) -> Option<&str> {
    let t = line.trim_end();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = t.get(hashes..)?;
        if rest.starts_with(' ') {
            return Some(rest.trim());
        }
    }
    None
}

fn convert_inline(s: &str) -> String {
    let mut codes: Vec<String> = Vec::new();
    let mut without_code = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        without_code.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        if let Some(end) = rest.find('`') {
            let token = format!("\u{0001}{}\u{0002}", codes.len());
            codes.push(escape_html(&rest[..end]));
            without_code.push_str(&token);
            rest = &rest[end + 1..];
        } else {
            without_code.push('`');
            without_code.push_str(rest);
            rest = "";
        }
    }
    without_code.push_str(rest);

    let mut text = escape_html(&without_code);

    static LINK_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let link_re = LINK_RE
        .get_or_init(|| regex::Regex::new(r"\[([^\]\n]+)\]\(([^)\n]+)\)").expect("link regex"));
    text = link_re
        .replace_all(&text, |caps: &regex::Captures| {
            let label = &caps[1];
            let href = &caps[2];
            if is_safe_href(href) {
                format!("<a href=\"{href}\">{label}</a>")
            } else {
                label.to_string()
            }
        })
        .into_owned();

    static BOLD_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let bold_re =
        BOLD_RE.get_or_init(|| regex::Regex::new(r"\*\*([^\n]+?)\*\*").expect("bold regex"));
    text = bold_re.replace_all(&text, "<b>$1</b>").into_owned();

    static ITALIC_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let italic_re = ITALIC_RE.get_or_init(|| {
        regex::Regex::new(r"(^|[\s(])_([^_\n]+)_([\s.,!?:;)]|$)").expect("italic regex")
    });
    text = italic_re.replace_all(&text, "$1<i>$2</i>$3").into_owned();

    for (i, code) in codes.iter().enumerate() {
        let token = format!("\u{0001}{i}\u{0002}");
        text = text.replace(&token, &format!("<code>{code}</code>"));
    }
    text
}

fn convert_markdown_segment(s: &str) -> String {
    let mut out = String::new();
    for piece in s.split_inclusive('\n') {
        let (line, nl) = match piece.strip_suffix('\n') {
            Some(line) => (line, "\n"),
            None => (piece, ""),
        };
        if let Some(heading) = heading_text(line) {
            out.push_str("<b>");
            out.push_str(&convert_inline(heading));
            out.push_str("</b>");
        } else {
            out.push_str(&convert_inline(line));
        }
        out.push_str(nl);
    }
    out
}

fn markdown_to_telegram_html(input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    loop {
        match rest.find("```") {
            None => {
                out.push_str(&convert_markdown_segment(rest));
                break;
            }
            Some(start) => {
                out.push_str(&convert_markdown_segment(&rest[..start]));
                let after_open = &rest[start + 3..];
                match after_open.find("```") {
                    Some(end) => {
                        let code = strip_fence_language(&after_open[..end]);
                        out.push_str("<pre><code>");
                        out.push_str(&escape_html(code.trim_end_matches('\n')));
                        out.push_str("</code></pre>");
                        rest = &after_open[end + 3..];
                        rest = rest.strip_prefix('\n').unwrap_or(rest);
                    }
                    None => {
                        out.push_str(&convert_markdown_segment(&rest[start..]));
                        break;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_token_scrubs_token_from_error_url() {
        let token = "123456:AAExampleSecretToken";
        // Mirrors reqwest's `Display`, which appends the request URL (with the
        // token in the path) to transport errors.
        let leaked = format!(
            "error sending request for url (https://api.telegram.org/bot{token}/createForumTopic)"
        );
        let scrubbed = redact_token(leaked, token);
        assert!(
            !scrubbed.contains(token),
            "token must be scrubbed: {scrubbed}"
        );
        assert!(scrubbed.contains("bot***/createForumTopic"), "{scrubbed}");
    }

    #[test]
    fn redact_token_is_noop_for_empty_token() {
        let msg = "some error without any token".to_string();
        assert_eq!(redact_token(msg.clone(), ""), msg);
    }

    #[tokio::test]
    async fn canonical_chat_id_passes_numeric_through_without_network() {
        // A numeric chat_id short-circuits before any getChat call, so this
        // resolves offline. (The `@username` → getChat branch is covered by
        // real-device / HTTP-mock testing.)
        let backend = TelegramBackend::new(1, "token".into(), "-100123".into(), true);
        assert_eq!(backend.canonical_chat_id().await.unwrap(), "-100123");
    }

    #[test]
    fn chat_filter_matches_configured_numeric_chat() {
        let message = serde_json::json!({
            "chat": { "id": -100123, "type": "supergroup" }
        });

        assert!(telegram_message_chat_matches(&message, "-100123"));
        assert!(!telegram_message_chat_matches(&message, "-100456"));
    }

    #[test]
    fn chat_filter_matches_configured_username_case_insensitively() {
        let message = serde_json::json!({
            "chat": { "id": -100123, "username": "CodegTopics" }
        });

        assert!(telegram_message_chat_matches(&message, "@codegtopics"));
        assert!(telegram_message_chat_matches(&message, "CODEGTOPICS"));
        assert!(!telegram_message_chat_matches(&message, "other"));
    }

    #[test]
    fn target_parser_uses_channel_target_when_topic_mode_is_disabled() {
        let message = serde_json::json!({
            "chat": { "id": -100123 },
            "message_thread_id": 8
        });

        let target = telegram_message_target(7, "-100123", false, &message);

        assert_eq!(target, ChannelMessageTarget::channel(7));
    }

    #[test]
    fn target_parser_distinguishes_general_and_forum_topics() {
        let general = serde_json::json!({ "chat": { "id": -100123 } });
        let topic = serde_json::json!({
            "chat": { "id": -100123 },
            "message_thread_id": 2
        });

        assert_eq!(
            telegram_message_target(7, "-100123", true, &general),
            ChannelMessageTarget::telegram_general(7, "-100123")
        );
        assert_eq!(
            telegram_message_target(7, "-100123", true, &topic),
            ChannelMessageTarget::telegram_forum_topic(7, "-100123", "2")
        );
    }

    #[test]
    fn text_filter_preserves_legacy_group_mention_requirement() {
        assert!(telegram_should_process_text_message(
            "supergroup",
            "/task@codeg_bot build",
            "codeg_bot",
            false
        ));
        assert!(!telegram_should_process_text_message(
            "supergroup",
            "/task build",
            "codeg_bot",
            false
        ));
    }

    #[test]
    fn text_filter_allows_unmentioned_text_in_topic_mode() {
        assert!(telegram_should_process_text_message(
            "supergroup",
            "plain follow-up",
            "codeg_bot",
            true
        ));
    }

    #[test]
    fn send_message_body_includes_forum_topic_thread_id() {
        let target = ChannelMessageTarget::telegram_forum_topic(7, "-100123", "42");
        let body = telegram_send_message_body(
            "fallback",
            "hello",
            Some("MarkdownV2"),
            Some(&target),
            Some(serde_json::json!({ "inline_keyboard": [] })),
        )
        .expect("body");

        assert_eq!(body["chat_id"], "-100123");
        assert_eq!(body["message_thread_id"], 42);
        assert_eq!(body["parse_mode"], "MarkdownV2");
        assert_eq!(
            body["reply_markup"],
            serde_json::json!({ "inline_keyboard": [] })
        );
    }

    #[test]
    fn send_message_body_rejects_invalid_forum_topic_thread_id() {
        let target = ChannelMessageTarget::telegram_forum_topic(7, "-100123", "bad");
        let err = telegram_send_message_body("fallback", "hello", None, Some(&target), None)
            .expect_err("invalid thread id should fail");

        assert!(err
            .to_string()
            .contains("invalid Telegram message_thread_id target"));
    }

    #[test]
    fn html_renders_heading_bold_code_and_link() {
        let msg = RichMessage::info("see **bold** and `code` and [x](https://e.test)")
            .with_title("Done Auth Grok");
        let html = format_telegram_html(&msg);
        assert!(html.contains("<b>Done Auth Grok</b>"), "got {html}");
        assert!(html.contains("<b>bold</b>"), "got {html}");
        assert!(html.contains("<code>code</code>"), "got {html}");
        assert!(
            html.contains("<a href=\"https://e.test\">x</a>"),
            "got {html}"
        );
        assert!(
            !html.contains("## "),
            "heading markers must not leak: {html}"
        );
    }

    #[test]
    fn html_escapes_raw_tags() {
        let msg = RichMessage::info("<script>alert(1)</script> & more");
        let html = format_telegram_html(&msg);
        assert!(html.contains("&lt;script&gt;"), "got {html}");
        assert!(html.contains("&amp; more"), "got {html}");
        assert!(!html.contains("<script>"), "got {html}");
    }

    #[test]
    fn html_preserves_fenced_code_escaped() {
        let msg = RichMessage::info("```\n<tag>\n```");
        let html = format_telegram_html(&msg);
        assert!(html.contains("<pre><code>"), "got {html}");
        assert!(html.contains("&lt;tag&gt;"), "got {html}");
        assert!(!html.contains("<tag>"), "got {html}");
    }

    #[test]
    fn html_drops_javascript_links() {
        let msg = RichMessage::info("[x](javascript:alert(1))");
        let html = format_telegram_html(&msg);
        assert!(
            !html.to_ascii_lowercase().contains("javascript:"),
            "got {html}"
        );
        assert!(html.contains("x"), "got {html}");
    }

    #[test]
    fn html_renders_italic() {
        let msg = RichMessage::info("hello _world_");
        let html = format_telegram_html(&msg);
        assert!(html.contains("<i>world</i>"), "got {html}");
    }

    #[test]
    fn inline_keyboard_uses_callback_data_in_two_button_rows() {
        let message = InteractiveMessage {
            base: RichMessage::info("Pick"),
            buttons: vec![
                MessageButton {
                    id: "cfg:folder:1".to_string(),
                    label: "One".to_string(),
                    style: ButtonStyle::Default,
                },
                MessageButton {
                    id: "cfg:folder:2".to_string(),
                    label: "Two".to_string(),
                    style: ButtonStyle::Default,
                },
                MessageButton {
                    id: "cfg:folder:3".to_string(),
                    label: "Three".to_string(),
                    style: ButtonStyle::Default,
                },
            ],
            callback_context: serde_json::json!({}),
        };

        let keyboard = telegram_inline_keyboard(&message).expect("keyboard");

        assert_eq!(keyboard["inline_keyboard"].as_array().unwrap().len(), 2);
        assert_eq!(
            keyboard["inline_keyboard"][0][0]["callback_data"],
            "cfg:folder:1"
        );
        assert_eq!(
            keyboard["inline_keyboard"][0][1]["callback_data"],
            "cfg:folder:2"
        );
        assert_eq!(
            keyboard["inline_keyboard"][1][0]["callback_data"],
            "cfg:folder:3"
        );
    }
}

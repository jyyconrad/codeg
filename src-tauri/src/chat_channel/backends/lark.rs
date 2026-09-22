use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::tungstenite;

use crate::chat_channel::error::ChatChannelError;
use crate::chat_channel::traits::ChatChannelBackend;
use crate::chat_channel::types::*;

const FEISHU_BASE_URL: &str = "https://open.feishu.cn";
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;

// ── Lark WebSocket protobuf Frame (pbbp2) ──
// Source: larksuite/oapi-sdk-go ws/pbbp2.pb.go

const FRAME_METHOD_CONTROL: i32 = 0; // Ping/Pong
const FRAME_METHOD_DATA: i32 = 1; // Event/Card

#[derive(Clone, PartialEq, ProstMessage)]
struct Frame {
    #[prost(uint64, tag = 1)]
    seq_id: u64,
    #[prost(uint64, tag = 2)]
    log_id: u64,
    #[prost(int32, tag = 3)]
    service: i32,
    #[prost(int32, tag = 4)]
    method: i32,
    #[prost(message, repeated, tag = 5)]
    headers: Vec<FrameHeader>,
    #[prost(string, tag = 6)]
    payload_encoding: String,
    #[prost(string, tag = 7)]
    payload_type: String,
    #[prost(bytes = "vec", tag = 8)]
    payload: Vec<u8>,
    #[prost(string, tag = 9)]
    log_id_new: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FrameHeader {
    #[prost(string, tag = 1)]
    key: String,
    #[prost(string, tag = 2)]
    value: String,
}

impl Frame {
    fn get_header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
    }

    fn set_header(&mut self, key: &str, value: &str) {
        if let Some(h) = self.headers.iter_mut().find(|h| h.key == key) {
            h.value = value.to_string();
        } else {
            self.headers.push(FrameHeader {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }
}

// ── Lark REST API types ──

#[derive(Deserialize)]
struct TenantAccessTokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: Option<String>,
    expire: Option<u64>,
}

#[derive(Serialize)]
struct SendMessageRequest {
    receive_id: String,
    msg_type: String,
    content: String,
}

#[derive(Deserialize)]
struct SendMessageResponse {
    code: i32,
    msg: String,
    data: Option<SendMessageData>,
}

#[derive(Deserialize)]
struct SendMessageData {
    message_id: Option<String>,
}

#[derive(Deserialize)]
struct WsConnectResponse {
    code: i32,
    msg: String,
    data: Option<WsConnectData>,
}

#[derive(Deserialize)]
struct WsConnectData {
    #[serde(rename = "URL")]
    url: Option<String>,
}

// ── Token cache ──

struct TokenCache {
    token: String,
    expires_at: Instant,
}

// ── Multi-part frame cache ──

struct PartialMessage {
    parts: HashMap<i32, Vec<u8>>,
    total: i32,
    created_at: Instant,
}

/// TTL for partial message reassembly entries. Prevents unbounded memory growth
/// if a multi-part message never completes (network issue, Lark SDK bug, etc).
const PARTIAL_MSG_TTL_SECS: u64 = 60;

// ── LarkBackend ──

pub struct LarkBackend {
    app_id: String,
    app_secret: String,
    chat_id: String,
    channel_id: i32,
    client: reqwest::Client,
    token_cache: Arc<RwLock<Option<TokenCache>>>,
    status: Arc<Mutex<ChannelConnectionStatus>>,
    shutdown_tx: Arc<Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
}

impl LarkBackend {
    pub fn new(channel_id: i32, app_id: String, app_secret: String, chat_id: String) -> Self {
        Self {
            app_id,
            app_secret,
            chat_id,
            channel_id,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            token_cache: Arc::new(RwLock::new(None)),
            status: Arc::new(Mutex::new(ChannelConnectionStatus::Disconnected)),
            shutdown_tx: Arc::new(Mutex::new(None)),
        }
    }

    async fn get_tenant_access_token(&self) -> Result<String, ChatChannelError> {
        tenant_access_token(
            &self.client,
            &self.token_cache,
            &self.app_id,
            &self.app_secret,
        )
        .await
    }

    async fn send_lark_message(
        &self,
        msg_type: &str,
        content: &str,
        receive_id: Option<&str>,
    ) -> Result<SentMessageId, ChatChannelError> {
        let token = self.get_tenant_access_token().await?;
        let receive_id = lark_receive_id(&self.chat_id, receive_id);

        let resp = self
            .client
            .post(format!(
                "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
                FEISHU_BASE_URL
            ))
            .header("Authorization", format!("Bearer {}", token))
            .json(&SendMessageRequest {
                receive_id: receive_id.to_string(),
                msg_type: msg_type.to_string(),
                content: content.to_string(),
            })
            .send()
            .await
            .map_err(|e| ChatChannelError::SendFailed(e.to_string()))?;

        let result: SendMessageResponse = resp
            .json()
            .await
            .map_err(|e| ChatChannelError::SendFailed(e.to_string()))?;

        if result.code != 0 {
            return Err(ChatChannelError::SendFailed(format!(
                "code={}, msg={}",
                result.code, result.msg
            )));
        }

        let message_id = result.data.and_then(|d| d.message_id).unwrap_or_default();
        Ok(SentMessageId(message_id))
    }

    async fn send_rich_message_with_receive_id(
        &self,
        message: &RichMessage,
        receive_id: Option<&str>,
    ) -> Result<SentMessageId, ChatChannelError> {
        let post = build_lark_post(message);
        let content = serde_json::to_string(&post)
            .map_err(|e| ChatChannelError::SendFailed(e.to_string()))?;
        match self.send_lark_message("post", &content, receive_id).await {
            Ok(id) => Ok(id),
            Err(e) => {
                tracing::warn!("[Lark] post markdown send failed: {e}, retrying as plain text");
                let text = serde_json::json!({ "text": message.to_plain_text() }).to_string();
                self.send_lark_message("text", &text, receive_id).await
            }
        }
    }

    async fn start_ws_receiver(
        &self,
        command_tx: mpsc::Sender<IncomingCommand>,
    ) -> Result<(), ChatChannelError> {
        // Verify we can get a WS URL before spawning the background task
        let _ = fetch_ws_url(&self.client, &self.app_id, &self.app_secret).await?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let channel_id = self.channel_id;
        let status = self.status.clone();
        let app_id = self.app_id.clone();
        let app_secret = self.app_secret.clone();
        let client = self.client.clone();
        let token_cache = self.token_cache.clone();

        tokio::spawn(async move {
            let mut retry_count = 0u32;

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let ws_url = match fetch_ws_url(&client, &app_id, &app_secret).await {
                    Ok(url) => url,
                    Err(e) => {
                        tracing::error!("[Lark] failed to get WS endpoint: {e}");
                        *status.lock().await = ChannelConnectionStatus::Error;
                        let delay = Duration::from_secs((2u64).pow(retry_count.min(5)));
                        retry_count += 1;
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => continue,
                            _ = shutdown_rx.changed() => break,
                        }
                    }
                };

                let ws_result = tokio_tungstenite::connect_async(&ws_url).await;
                let ws_stream = match ws_result {
                    Ok((stream, _)) => {
                        *status.lock().await = ChannelConnectionStatus::Connected;
                        retry_count = 0;
                        tracing::info!("[Lark] WebSocket connected");
                        stream
                    }
                    Err(e) => {
                        tracing::error!("[Lark] WebSocket connect failed: {e}");
                        *status.lock().await = ChannelConnectionStatus::Error;
                        let delay = Duration::from_secs((2u64).pow(retry_count.min(5)));
                        retry_count += 1;
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => continue,
                            _ = shutdown_rx.changed() => break,
                        }
                    }
                };

                let (mut write, mut read) = ws_stream.split();
                let mut partial_msgs: HashMap<String, PartialMessage> = HashMap::new();
                let mut last_partial_cleanup = Instant::now();

                loop {
                    tokio::select! {
                        msg = read.next() => {
                            match msg {
                                Some(Ok(tungstenite::Message::Binary(data))) => {
                                    match Frame::decode(data.as_ref()) {
                                        Ok(frame) => {
                                            let frame_type = frame.get_header("type").unwrap_or("").to_string();

                                            if frame.method == FRAME_METHOD_CONTROL {
                                                // Control frame: ping → respond with pong
                                                if frame_type == "ping" {
                                                    let mut pong = frame.clone();
                                                    // Clear type header and set to pong
                                                    pong.set_header("type", "pong");
                                                    pong.payload = Vec::new();
                                                    let mut buf = Vec::new();
                                                    if pong.encode(&mut buf).is_ok() {
                                                        let _ = write.send(tungstenite::Message::Binary(buf.into())).await;
                                                    }
                                                }
                                            } else if frame.method == FRAME_METHOD_DATA && frame_type == "event" {
                                                let start = Instant::now();

                                                // Multi-part reassembly
                                                let msg_id = frame.get_header("message_id").unwrap_or("").to_string();
                                                let sum: i32 = frame.get_header("sum").and_then(|s| s.parse().ok()).unwrap_or(1);
                                                let seq: i32 = frame.get_header("seq").and_then(|s| s.parse().ok()).unwrap_or(0);

                                                // Evict stale partial messages to prevent unbounded memory growth
                                if last_partial_cleanup.elapsed() > Duration::from_secs(PARTIAL_MSG_TTL_SECS) {
                                    partial_msgs.retain(|_, pm| pm.created_at.elapsed() < Duration::from_secs(PARTIAL_MSG_TTL_SECS));
                                    last_partial_cleanup = Instant::now();
                                }

                                let full_payload = if sum <= 1 {
                                                    Some(frame.payload.clone())
                                                } else {
                                                    let entry = partial_msgs.entry(msg_id.clone()).or_insert_with(|| PartialMessage {
                                                        parts: HashMap::new(),
                                                        total: sum,
                                                        created_at: Instant::now(),
                                                    });
                                                    entry.parts.insert(seq, frame.payload.clone());
                                                    if entry.parts.len() as i32 >= entry.total {
                                                        // All parts received — reassemble in order
                                                        let mut combined = Vec::new();
                                                        for i in 0..entry.total {
                                                            if let Some(part) = entry.parts.get(&i) {
                                                                combined.extend_from_slice(part);
                                                            }
                                                        }
                                                        partial_msgs.remove(&msg_id);
                                                        Some(combined)
                                                    } else {
                                                        None // Still waiting for more parts
                                                    }
                                                };

                                                if let Some(payload_bytes) = full_payload {
                                                    // Process event
                                                    if let Ok(payload_str) = std::str::from_utf8(&payload_bytes) {
                                                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(payload_str) {
                                                            handle_lark_event(
                                                                &event,
                                                                channel_id,
                                                                &command_tx,
                                                                &client,
                                                                &token_cache,
                                                                &app_id,
                                                                &app_secret,
                                                            )
                                                            .await;
                                                        } else {
                                                            tracing::info!("[Lark] event payload is not valid JSON");
                                                        }
                                                    }

                                                    // Send acknowledgment: echo frame back with {"code":200}
                                                    let elapsed_ms = start.elapsed().as_millis();
                                                    let mut ack = frame.clone();
                                                    ack.payload = br#"{"code":200}"#.to_vec();
                                                    ack.set_header("biz_rt", &elapsed_ms.to_string());
                                                    let mut buf = Vec::new();
                                                    if ack.encode(&mut buf).is_ok() {
                                                        let _ = write.send(tungstenite::Message::Binary(buf.into())).await;
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("[Lark] protobuf decode error: {e}, len={}", data.len());
                                        }
                                    }
                                }
                                Some(Ok(tungstenite::Message::Ping(data))) => {
                                    let _ = write.send(tungstenite::Message::Pong(data)).await;
                                }
                                Some(Ok(tungstenite::Message::Close(_))) | None => {
                                    tracing::info!("[Lark] WebSocket closed, will reconnect");
                                    break;
                                }
                                Some(Err(e)) => {
                                    tracing::error!("[Lark] WebSocket error: {e}");
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = shutdown_rx.changed() => {
                            let _ = write.close().await;
                            *status.lock().await = ChannelConnectionStatus::Disconnected;
                            return;
                        }
                    }
                }

                *status.lock().await = ChannelConnectionStatus::Connecting;
                let delay = Duration::from_secs(3);
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = shutdown_rx.changed() => break,
                }
            }

            *status.lock().await = ChannelConnectionStatus::Disconnected;
        });

        Ok(())
    }
}

async fn handle_lark_event(
    event: &serde_json::Value,
    channel_id: i32,
    command_tx: &mpsc::Sender<IncomingCommand>,
    client: &reqwest::Client,
    token_cache: &Arc<RwLock<Option<TokenCache>>>,
    app_id: &str,
    app_secret: &str,
) {
    let Some(mut cmd) = lark_inbound_command(event, channel_id) else {
        return;
    };

    match tenant_access_token(client, token_cache, app_id, app_secret).await {
        Ok(token) => attach_lark_resources(&mut cmd, client, &token).await,
        Err(e) => tracing::warn!("[Lark] skip attachment download, no token: {e}"),
    }

    // Keep a safe breadcrumb (who sent it) at the default level; the message
    // body itself only logs at debug so it never lands on disk by default.
    tracing::info!("[Lark] incoming message from {}", cmd.sender_id);
    tracing::debug!(
        "[Lark] incoming message from {}: {}",
        cmd.sender_id,
        cmd.command_text
    );

    let _ = command_tx.send(cmd).await;
}

async fn tenant_access_token(
    client: &reqwest::Client,
    token_cache: &Arc<RwLock<Option<TokenCache>>>,
    app_id: &str,
    app_secret: &str,
) -> Result<String, ChatChannelError> {
    {
        let cache = token_cache.read().await;
        if let Some(cached) = cache.as_ref() {
            if cached.expires_at > Instant::now() {
                return Ok(cached.token.clone());
            }
        }
    }

    let resp = client
        .post(format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            FEISHU_BASE_URL
        ))
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": app_secret,
        }))
        .send()
        .await
        .map_err(|e| ChatChannelError::AuthenticationFailed(e.to_string()))?;

    let result: TenantAccessTokenResponse = resp
        .json()
        .await
        .map_err(|e| ChatChannelError::AuthenticationFailed(e.to_string()))?;

    if result.code != 0 {
        return Err(ChatChannelError::AuthenticationFailed(format!(
            "code={}, msg={}",
            result.code, result.msg
        )));
    }

    let token = result
        .tenant_access_token
        .ok_or_else(|| ChatChannelError::AuthenticationFailed("No token in response".into()))?;
    let expire_secs = result.expire.unwrap_or(7200);

    let expires_at =
        Instant::now() + Duration::from_secs(expire_secs.saturating_sub(TOKEN_REFRESH_MARGIN_SECS));
    *token_cache.write().await = Some(TokenCache {
        token: token.clone(),
        expires_at,
    });

    Ok(token)
}

async fn attach_lark_resources(cmd: &mut IncomingCommand, client: &reqwest::Client, token: &str) {
    let Some(message_id) = cmd.provider_message_id.as_deref() else {
        return;
    };
    let content_str = cmd
        .metadata
        .pointer("/event/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(content) = serde_json::from_str::<serde_json::Value>(content_str) else {
        return;
    };
    let msg_type = cmd
        .metadata
        .pointer("/event/message/message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    for resource in lark_resource_refs(msg_type, &content) {
        match download_lark_resource(client, token, message_id, &resource).await {
            Ok(block) => cmd.extra_blocks.push(block),
            Err(e) => tracing::warn!(
                "[Lark] failed to download {} {}: {e}",
                if resource.as_image { "image" } else { "file" },
                resource.file_key
            ),
        }
    }
}

async fn download_lark_resource(
    client: &reqwest::Client,
    token: &str,
    message_id: &str,
    resource: &LarkResourceRef,
) -> Result<crate::acp::types::PromptInputBlock, String> {
    let resource_type = if resource.as_image { "image" } else { "file" };
    let url = format!(
        "{}/open-apis/im/v1/messages/{}/resources/{}?type={}",
        FEISHU_BASE_URL, message_id, resource.file_key, resource_type
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("empty resource".into());
    }
    if let Ok(err) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if let Some(code) = err.get("code").and_then(|v| v.as_i64()) {
            if code != 0 {
                let msg = err
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("download failed");
                return Err(format!("code={code}, msg={msg}"));
            }
        }
    }
    Ok(lark_attachment_block(
        &mime,
        &bytes,
        resource.file_name.as_deref(),
        resource.as_image,
    ))
}

fn lark_inbound_command(event: &serde_json::Value, channel_id: i32) -> Option<IncomingCommand> {
    let event_type = event
        .pointer("/header/event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if event_type != "im.message.receive_v1" {
        return None;
    }

    let sender_type = event
        .pointer("/event/sender/sender_type")
        .and_then(|v| v.as_str())
        .unwrap_or("user");
    if sender_type == "app" || sender_type == "bot" {
        return None;
    }

    let msg_type = event
        .pointer("/event/message/message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if msg_type == "system" {
        return None;
    }

    let quoted_message_id = lark_quoted_message_id(event);

    // Group chat: @mention, or a quote/reply to a previous message (e.g. an
    // image sent as 引用回复 to the bot).
    let chat_type = event
        .pointer("/event/message/chat_type")
        .and_then(|v| v.as_str())
        .unwrap_or("p2p");

    if chat_type == "group" {
        let mentions = event
            .pointer("/event/message/mentions")
            .and_then(|v| v.as_array());
        let mentioned = mentions.is_some_and(|m| !m.is_empty());
        if !mentioned && quoted_message_id.is_none() {
            return None;
        }
    }

    let content_str = event
        .pointer("/event/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut text = extract_lark_message_text(msg_type, content_str);
    if text.is_empty() {
        let has_resources = serde_json::from_str::<serde_json::Value>(content_str)
            .ok()
            .is_some_and(|content| !lark_resource_refs(msg_type, &content).is_empty());
        if has_resources {
            text = "[附件]".to_string();
        } else {
            return None;
        }
    }

    // Strip mention placeholders (e.g. "@_user_1") from text
    let clean_text = strip_lark_mentions(&text, event);

    if clean_text.is_empty() {
        return None;
    }

    let sender_id = event
        .pointer("/event/sender/sender_id/open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let chat_id = event
        .pointer("/event/message/chat_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let provider_message_id = event
        .pointer("/event/message/message_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let target = match chat_id {
        Some(chat_id) => ChannelMessageTarget::with_chat_id(channel_id, chat_id),
        None => ChannelMessageTarget::channel(channel_id),
    };

    Some(IncomingCommand {
        channel_id,
        sender_id,
        command_text: clean_text,
        callback_data: None,
        target,
        metadata: event.clone(),
        quoted_message_id,
        provider_message_id,
        extra_blocks: Vec::new(),
    })
}

fn extract_lark_message_text(msg_type: &str, content_str: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content_str) else {
        return if msg_type.is_empty() {
            String::new()
        } else {
            format!("[{msg_type}]")
        };
    };
    match msg_type {
        "text" => value
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "post" => flatten_lark_post(&value),
        "image" => "[图片]".to_string(),
        "sticker" => "[表情]".to_string(),
        "audio" => "[语音]".to_string(),
        "file" => format!("[文件: {}]", json_file_name(&value).unwrap_or("file")),
        "media" => format!("[视频: {}]", json_file_name(&value).unwrap_or("video")),
        "share_chat" => "[分享群]".to_string(),
        "share_user" => "[分享名片]".to_string(),
        "merge_forward" => "[合并转发]".to_string(),
        "interactive" => flatten_lark_interactive(&value),
        "location" => value
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("[位置]")
            .to_string(),
        "system" => String::new(),
        "todo" => flatten_lark_todo(&value),
        "vote" => value
            .get("topic")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("[投票: {s}]"))
            .unwrap_or_else(|| "[投票]".to_string()),
        "share_calendar_event" | "calendar" | "general_calendar" => value
            .get("summary")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("[日程: {s}]"))
            .unwrap_or_else(|| "[日程]".to_string()),
        "video_chat" => value
            .get("topic")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("[视频通话: {s}]"))
            .unwrap_or_else(|| "[视频通话]".to_string()),
        other => {
            if let Some(text) = value
                .get("text")
                .and_then(|t| t.as_str())
                .filter(|s| !s.is_empty())
            {
                return text.to_string();
            }
            let post = flatten_lark_post(&value);
            if !post.is_empty() {
                return post;
            }
            format!("[{other}]")
        }
    }
}

fn json_file_name(value: &serde_json::Value) -> Option<&str> {
    value
        .get("file_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn flatten_lark_todo(value: &serde_json::Value) -> String {
    if let Some(summary) = value.get("summary") {
        let text = flatten_lark_post(summary);
        if !text.is_empty() {
            return format!("[待办: {text}]");
        }
    }
    "[待办]".to_string()
}

fn flatten_lark_interactive(value: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(title) = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(title.to_string());
    }
    if let Some(elements) = value.get("elements") {
        let body = flatten_lark_blocks(elements);
        if !body.is_empty() {
            parts.push(body);
        }
    }
    if parts.is_empty() {
        let post = flatten_lark_post(value);
        if !post.is_empty() {
            return post;
        }
        return "[卡片]".to_string();
    }
    parts.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LarkResourceRef {
    file_key: String,
    as_image: bool,
    file_name: Option<String>,
}

fn lark_resource_refs(msg_type: &str, content: &serde_json::Value) -> Vec<LarkResourceRef> {
    let mut refs = Vec::new();
    match msg_type {
        "image" => push_resource_key(&mut refs, content.get("image_key"), true, None),
        "file" | "audio" | "folder" => push_resource_key(
            &mut refs,
            content.get("file_key"),
            false,
            json_file_name(content).map(str::to_string),
        ),
        "media" => {
            push_resource_key(
                &mut refs,
                content.get("file_key"),
                false,
                json_file_name(content).map(str::to_string),
            );
        }
        "post" | "interactive" => collect_post_resources(content, &mut refs),
        _ => {
            push_resource_key(&mut refs, content.get("image_key"), true, None);
            push_resource_key(
                &mut refs,
                content.get("file_key"),
                false,
                json_file_name(content).map(str::to_string),
            );
            collect_post_resources(content, &mut refs);
        }
    }
    refs
}

fn push_resource_key(
    refs: &mut Vec<LarkResourceRef>,
    key: Option<&serde_json::Value>,
    as_image: bool,
    file_name: Option<String>,
) {
    let Some(file_key) = key
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    if refs.iter().any(|r| r.file_key == file_key) {
        return;
    }
    refs.push(LarkResourceRef {
        file_key: file_key.to_string(),
        as_image,
        file_name,
    });
}

fn collect_post_resources(value: &serde_json::Value, refs: &mut Vec<LarkResourceRef>) {
    if let Some(arr) = value.as_array() {
        for item in arr {
            collect_post_resources(item, refs);
        }
        return;
    }
    if let Some(obj) = value.as_object() {
        let tag = obj.get("tag").and_then(|v| v.as_str()).unwrap_or("");
        match tag {
            "img" => push_resource_key(refs, obj.get("image_key"), true, None),
            "media" => {
                push_resource_key(
                    refs,
                    obj.get("file_key"),
                    false,
                    obj.get("file_name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                );
                push_resource_key(refs, obj.get("image_key"), true, None);
            }
            _ => {
                for child in obj.values() {
                    collect_post_resources(child, refs);
                }
            }
        }
    }
}

fn lark_attachment_block(
    content_type: &str,
    bytes: &[u8],
    file_name: Option<&str>,
    as_image: bool,
) -> crate::acp::types::PromptInputBlock {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let sniffed = sniff_image_mime(bytes);
    let mime = sniffed
        .or_else(|| {
            let mime = content_type.split(';').next().unwrap_or("").trim();
            if mime.starts_with("image/") {
                Some(mime)
            } else {
                None
            }
        })
        .unwrap_or(content_type.split(';').next().unwrap_or("").trim());
    if as_image || mime.starts_with("image/") {
        let mime = if mime.starts_with("image/") {
            mime.to_string()
        } else {
            sniffed.unwrap_or("image/png").to_string()
        };
        return crate::acp::types::PromptInputBlock::Image {
            data: B64.encode(bytes),
            mime_type: mime,
            uri: None,
        };
    }
    let name = file_name.unwrap_or("file");
    crate::acp::types::PromptInputBlock::Resource {
        uri: format!("feishu://{name}"),
        mime_type: Some(if mime.is_empty() {
            "application/octet-stream".to_string()
        } else {
            mime.to_string()
        }),
        text: None,
        blob: Some(B64.encode(bytes)),
    }
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn flatten_lark_post(value: &serde_json::Value) -> String {
    if let Some(text) = flatten_lark_post_body(value) {
        if !text.is_empty() {
            return text;
        }
    }
    for key in ["zh_cn", "en_us", "ja_jp"] {
        if let Some(text) = value.get(key).and_then(flatten_lark_post_body) {
            if !text.is_empty() {
                return text;
            }
        }
    }
    if let Some(obj) = value.as_object() {
        for (key, child) in obj {
            if key == "title" {
                continue;
            }
            if let Some(text) = flatten_lark_post_body(child) {
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

fn flatten_lark_post_body(body: &serde_json::Value) -> Option<String> {
    let blocks = body.get("content_v2").or_else(|| body.get("content"))?;
    let text = flatten_lark_blocks(blocks);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn flatten_lark_blocks(blocks: &serde_json::Value) -> String {
    let Some(paragraphs) = blocks.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for paragraph in paragraphs {
        let mut line = String::new();
        match paragraph.as_array() {
            Some(elements) => {
                for element in elements {
                    append_lark_post_element(element, &mut line);
                }
            }
            None => append_lark_post_element(paragraph, &mut line),
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn append_lark_post_element(element: &serde_json::Value, out: &mut String) {
    let tag = element.get("tag").and_then(|v| v.as_str()).unwrap_or("");
    match tag {
        // Quote blocks are the original message; routing uses parent_id / quote.
        "quote" | "img" | "media" | "emotion" | "hr" => {}
        "at" => {
            if let Some(user_id) = element.get("user_id").and_then(|v| v.as_str()) {
                if user_id.starts_with("@_") {
                    out.push_str(user_id);
                    out.push(' ');
                }
            }
        }
        _ => {
            if let Some(text) = element.get("text").and_then(|v| v.as_str()) {
                out.push_str(text);
            }
        }
    }
}

fn lark_quoted_message_id(event: &serde_json::Value) -> Option<String> {
    nonempty_json_str(event.pointer("/event/message/parent_id"))
        .or_else(|| nonempty_json_str(event.pointer("/event/message/quote/message_id")))
        .or_else(|| nonempty_json_str(event.pointer("/event/message/quote/id")))
}

fn nonempty_json_str(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Strip Lark mention placeholders (e.g. `@_user_1`) from the message text.
fn strip_lark_mentions(text: &str, event: &serde_json::Value) -> String {
    let mut result = text.to_string();
    if let Some(mentions) = event
        .pointer("/event/message/mentions")
        .and_then(|v| v.as_array())
    {
        for mention in mentions {
            if let Some(key) = mention.get("key").and_then(|v| v.as_str()) {
                result = result.replace(key, "");
            }
        }
    }
    result.trim().to_string()
}

/// Fetch a fresh WebSocket endpoint URL from Feishu.
async fn fetch_ws_url(
    client: &reqwest::Client,
    app_id: &str,
    app_secret: &str,
) -> Result<String, ChatChannelError> {
    let resp = client
        .post(format!("{}/callback/ws/endpoint", FEISHU_BASE_URL))
        .json(&serde_json::json!({
            "AppID": app_id,
            "AppSecret": app_secret,
        }))
        .send()
        .await
        .map_err(|e| ChatChannelError::ConnectionFailed(e.to_string()))?;

    let ws_resp: WsConnectResponse = resp
        .json()
        .await
        .map_err(|e| ChatChannelError::ConnectionFailed(e.to_string()))?;

    if ws_resp.code != 0 {
        return Err(ChatChannelError::ConnectionFailed(format!(
            "WS connect failed: code={}, msg={}",
            ws_resp.code, ws_resp.msg
        )));
    }

    ws_resp
        .data
        .and_then(|d| d.url)
        .ok_or_else(|| ChatChannelError::ConnectionFailed("No WebSocket URL returned".into()))
}

#[async_trait]
impl ChatChannelBackend for LarkBackend {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Lark
    }

    async fn start(
        &self,
        command_tx: mpsc::Sender<IncomingCommand>,
    ) -> Result<(), ChatChannelError> {
        *self.status.lock().await = ChannelConnectionStatus::Connecting;
        self.get_tenant_access_token().await?;
        *self.status.lock().await = ChannelConnectionStatus::Connected;

        if let Err(e) = self.start_ws_receiver(command_tx).await {
            tracing::error!("[Lark] WebSocket receiver failed to start: {e}");
        }

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
        let content = serde_json::json!({ "text": text }).to_string();
        self.send_lark_message("text", &content, None).await
    }

    async fn send_rich_message(
        &self,
        message: &RichMessage,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_rich_message_with_receive_id(message, None).await
    }

    async fn send_rich_message_to(
        &self,
        message: &RichMessage,
        target: &ChannelMessageTarget,
    ) -> Result<SentMessageId, ChatChannelError> {
        self.send_rich_message_with_receive_id(message, target.chat_id.as_deref())
            .await
    }

    async fn test_connection(&self) -> Result<(), ChatChannelError> {
        self.get_tenant_access_token().await?;
        Ok(())
    }
}

fn lark_receive_id<'a>(default: &'a str, override_id: Option<&'a str>) -> &'a str {
    override_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}

/// Feishu `post` + `md` tag: CommonMark 0.31 + GFM, no colored card header.
fn build_lark_post(msg: &RichMessage) -> serde_json::Value {
    serde_json::json!({
        "zh_cn": {
            "content": [[{
                "tag": "md",
                "text": sanitize_lark_markdown(&msg.to_markdown()),
            }]]
        }
    })
}

/// Strip Feishu-extended `<at>` mention tags so agent/user text cannot @everyone.
fn sanitize_lark_markdown(s: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?is)<at\b[^>]*>.*?</at\s*>|<at\b[^>]*/?>").expect("at-tag regex")
    });
    re.replace_all(s, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post_md_text(post: &serde_json::Value) -> &str {
        post["zh_cn"]["content"][0][0]["text"]
            .as_str()
            .expect("post md text")
    }

    /// Rich pushes use Feishu `post` + `md` so CommonMark/GFM in the agent
    /// body actually renders. The colored interactive-card header is gone.
    #[test]
    fn rich_message_is_post_with_md_tag() {
        let msg = RichMessage::info("done with **bold** and a list:\n- a\n- b")
            .with_title("完成 Filter UI Grok");
        let post = build_lark_post(&msg);
        let node = &post["zh_cn"]["content"][0][0];
        assert_eq!(node["tag"], "md");
        let text = node["text"].as_str().unwrap();
        assert!(text.contains("## 完成 Filter UI Grok"), "got {text}");
        assert!(text.contains("**bold**"), "got {text}");
        assert!(text.contains("- a"), "got {text}");
        assert!(post.get("header").is_none(), "must not use a card header");
        assert!(post.get("elements").is_none(), "must not use a card body");
    }

    /// `<at>` is Feishu-extended markdown that would @everyone / inject a
    /// mention. Strip it; keep the rest of the markdown (bold, links).
    #[test]
    fn strips_at_mention_tags_from_markdown() {
        let raw = "<at id=all></at> **bold** [x](http://e.test) <at user_id=\"ou_1\">Tom</at>";
        let msg = RichMessage::info(raw).with_title("User Message");
        let post = build_lark_post(&msg);
        let text = post_md_text(&post);
        assert!(!text.contains("<at"), "got {text}");
        assert!(!text.contains("id=all"), "got {text}");
        assert!(text.contains("**bold**"), "got {text}");
        assert!(text.contains("[x](http://e.test)"), "got {text}");
    }

    #[test]
    fn field_at_tags_are_stripped() {
        let injected = "Bash: echo [x](http://e.test) <at id=all></at> **b**";
        let msg = RichMessage {
            title: Some("Permission Request".into()),
            body: "An agent is waiting for approval.".into(),
            fields: vec![("Operation".into(), injected.into())],
            level: MessageLevel::Warning,
        };
        let post = build_lark_post(&msg);
        let text = post_md_text(&post);
        assert!(!text.contains("<at"), "got {text}");
        assert!(text.contains("**Operation**"), "got {text}");
        assert!(text.contains("[x](http://e.test)"), "got {text}");
        assert!(text.contains("**b**"), "got {text}");
    }

    #[test]
    fn receive_id_prefers_nonempty_override() {
        assert_eq!(
            lark_receive_id("oc_default", Some("oc_folder")),
            "oc_folder"
        );
        assert_eq!(lark_receive_id("oc_default", Some("  ")), "oc_default");
        assert_eq!(lark_receive_id("oc_default", None), "oc_default");
    }

    fn mention(key: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "key": key,
            "id": { "open_id": "ou_bot" },
            "name": name
        })
    }

    fn receive_event(
        msg_type: &str,
        content: serde_json::Value,
        extra_message: serde_json::Value,
    ) -> serde_json::Value {
        let mut message = serde_json::json!({
            "message_id": "om_child",
            "chat_id": "oc_group",
            "chat_type": "group",
            "message_type": msg_type,
            "content": content.to_string(),
            "mentions": [mention("@_user_1", "戴蒙")],
            "create_time": "1694779200000"
        });
        if let Some(obj) = extra_message.as_object() {
            for (k, v) in obj {
                message[k] = v.clone();
            }
        }
        serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": message
            }
        })
    }

    #[test]
    fn group_text_mention_is_dispatched() {
        let event = receive_event(
            "text",
            serde_json::json!({ "text": "@_user_1 继续" }),
            serde_json::json!({ "parent_id": "om_done" }),
        );
        let cmd = lark_inbound_command(&event, 7).expect("text mention should dispatch");
        assert_eq!(cmd.command_text, "继续");
        assert_eq!(cmd.quoted_message_id.as_deref(), Some("om_done"));
        assert_eq!(cmd.provider_message_id.as_deref(), Some("om_child"));
        assert_eq!(cmd.sender_id, "ou_user");
    }

    /// Quoting a bot post (markdown completion) makes Feishu deliver the
    /// follow-up as `message_type=post`. Dropping those silently is why a
    /// Feishu quote-reply never starts work.
    #[test]
    fn group_post_quote_reply_is_dispatched() {
        let event = receive_event(
            "post",
            serde_json::json!({
                "title": "",
                "content": [[
                    { "tag": "at", "user_id": "@_user_1", "user_name": "戴蒙" },
                    { "tag": "text", "text": " 下一批还有多少，还有哪些工作" }
                ]]
            }),
            serde_json::json!({ "parent_id": "om_done" }),
        );
        let cmd = lark_inbound_command(&event, 7)
            .expect("quoting a post completion must dispatch the follow-up");
        assert_eq!(cmd.command_text, "下一批还有多少，还有哪些工作");
        assert_eq!(cmd.quoted_message_id.as_deref(), Some("om_done"));
    }

    #[test]
    fn group_post_without_mentions_is_ignored() {
        let mut event = receive_event(
            "post",
            serde_json::json!({
                "content": [[{ "tag": "text", "text": "hello" }]]
            }),
            serde_json::json!({}),
        );
        event["event"]["message"]["mentions"] = serde_json::json!([]);
        assert!(lark_inbound_command(&event, 7).is_none());
    }

    #[test]
    fn quote_field_is_used_when_parent_id_is_empty() {
        let event = receive_event(
            "text",
            serde_json::json!({ "text": "@_user_1 继续" }),
            serde_json::json!({
                "quote": { "message_id": "om_quoted" }
            }),
        );
        let cmd = lark_inbound_command(&event, 7).expect("text quote should dispatch");
        assert_eq!(cmd.quoted_message_id.as_deref(), Some("om_quoted"));
    }

    #[test]
    fn zh_cn_wrapped_post_and_quote_tag_are_flattened() {
        let event = receive_event(
            "post",
            serde_json::json!({
                "zh_cn": {
                    "title": "",
                    "content": [
                        [{ "tag": "quote", "token": "om_done" }],
                        [
                            { "tag": "at", "user_id": "@_user_1", "user_name": "戴蒙" },
                            { "tag": "text", "text": "下一批还有多少" }
                        ]
                    ]
                }
            }),
            serde_json::json!({ "parent_id": "om_done" }),
        );
        let cmd = lark_inbound_command(&event, 7).expect("wrapped post should dispatch");
        assert_eq!(cmd.command_text, "下一批还有多少");
        assert_eq!(cmd.quoted_message_id.as_deref(), Some("om_done"));
    }

    #[test]
    fn group_image_mention_is_dispatched() {
        let event = receive_event(
            "image",
            serde_json::json!({ "image_key": "img_1" }),
            serde_json::json!({}),
        );
        let cmd = lark_inbound_command(&event, 7).expect("image with @ must dispatch");
        assert_eq!(cmd.command_text, "[图片]");
        assert_eq!(
            lark_resource_refs("image", &serde_json::json!({ "image_key": "img_1" })),
            vec![LarkResourceRef {
                file_key: "img_1".into(),
                as_image: true,
                file_name: None,
            }]
        );
    }

    #[test]
    fn group_file_and_media_are_dispatched() {
        let file = receive_event(
            "file",
            serde_json::json!({ "file_key": "file_1", "file_name": "spec.pdf" }),
            serde_json::json!({}),
        );
        let cmd = lark_inbound_command(&file, 7).expect("file with @ must dispatch");
        assert_eq!(cmd.command_text, "[文件: spec.pdf]");

        let media = receive_event(
            "media",
            serde_json::json!({ "file_key": "file_2", "file_name": "clip.mp4" }),
            serde_json::json!({}),
        );
        let cmd = lark_inbound_command(&media, 7).expect("video with @ must dispatch");
        assert_eq!(cmd.command_text, "[视频: clip.mp4]");
    }

    #[test]
    fn audio_sticker_share_and_interactive_are_dispatched() {
        let audio = receive_event(
            "audio",
            serde_json::json!({ "file_key": "file_a" }),
            serde_json::json!({}),
        );
        assert_eq!(
            lark_inbound_command(&audio, 7).expect("audio").command_text,
            "[语音]"
        );

        let sticker = receive_event(
            "sticker",
            serde_json::json!({ "file_key": "file_s" }),
            serde_json::json!({}),
        );
        assert_eq!(
            lark_inbound_command(&sticker, 7)
                .expect("sticker")
                .command_text,
            "[表情]"
        );

        let share = receive_event(
            "share_chat",
            serde_json::json!({ "chat_id": "oc_other" }),
            serde_json::json!({}),
        );
        assert_eq!(
            lark_inbound_command(&share, 7).expect("share").command_text,
            "[分享群]"
        );

        let card = receive_event(
            "interactive",
            serde_json::json!({
                "title": "审批",
                "elements": [[{ "tag": "text", "text": "请看截图" }]]
            }),
            serde_json::json!({}),
        );
        assert_eq!(
            lark_inbound_command(&card, 7)
                .expect("interactive")
                .command_text,
            "审批\n请看截图"
        );
    }

    #[test]
    fn group_image_without_mention_is_ignored_unless_quoted() {
        let mut event = receive_event(
            "image",
            serde_json::json!({ "image_key": "img_1" }),
            serde_json::json!({}),
        );
        event["event"]["message"]["mentions"] = serde_json::json!([]);
        assert!(lark_inbound_command(&event, 7).is_none());

        event["event"]["message"]["parent_id"] = serde_json::json!("om_done");
        let cmd =
            lark_inbound_command(&event, 7).expect("quoting the bot with an image must dispatch");
        assert_eq!(cmd.command_text, "[图片]");
        assert_eq!(cmd.quoted_message_id.as_deref(), Some("om_done"));
    }

    #[test]
    fn system_and_bot_messages_are_ignored() {
        let system = receive_event(
            "system",
            serde_json::json!({ "template": "{from_user} joined" }),
            serde_json::json!({}),
        );
        assert!(lark_inbound_command(&system, 7).is_none());

        let mut bot = receive_event(
            "text",
            serde_json::json!({ "text": "@_user_1 hi" }),
            serde_json::json!({}),
        );
        bot["event"]["sender"]["sender_type"] = serde_json::json!("app");
        assert!(lark_inbound_command(&bot, 7).is_none());
    }

    #[test]
    fn post_img_tags_are_collected_as_image_resources() {
        let content = serde_json::json!({
            "content": [
                [{ "tag": "img", "image_key": "img_in_post" }],
                [{ "tag": "text", "text": "见图" }]
            ]
        });
        assert_eq!(
            lark_resource_refs("post", &content),
            vec![LarkResourceRef {
                file_key: "img_in_post".into(),
                as_image: true,
                file_name: None,
            }]
        );
    }

    #[test]
    fn downloaded_image_bytes_become_image_prompt_block() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let block = lark_attachment_block("application/octet-stream", &png, None, true);
        match block {
            crate::acp::types::PromptInputBlock::Image {
                mime_type, data, ..
            } => {
                assert_eq!(mime_type, "image/png");
                assert!(!data.is_empty());
            }
            other => panic!("expected image block, got {other:?}"),
        }
    }
}

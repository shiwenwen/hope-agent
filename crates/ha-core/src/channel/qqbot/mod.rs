//! QQ Bot V2 channel (QQ 官方机器人).
//!
//! - **Official API**: <https://bot.q.qq.com/wiki/develop/api-v2/>
//! - **SDK / Reference**: <https://github.com/tencent-connect/botpy>
//!   (官方 Python SDK，opcode 协议 + IDENTIFY/RESUME + msg_seq 参考实现)
//! - **Protocol**: WebSocket Gateway（Discord-like opcodes）+ REST `/v2/...`，
//!   认证头 `Authorization: QQBot {access_token}` (NOT Bearer!)
//! - **Last reviewed**: 2026-05-05

pub mod api;
pub mod auth;
pub mod format;
pub mod gateway;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use self::api::{QqBotApi, QqChatScope};
use self::auth::QqBotAuth;

/// Running account state for a single QQ Bot.
struct RunningAccount {
    api: Arc<QqBotApi>,
    #[allow(dead_code)]
    bot_id: String,
    #[allow(dead_code)]
    bot_name: String,
}

/// QQ Bot channel plugin implementation.
///
/// Connects to the QQ Bot Official API via WebSocket gateway for receiving
/// events and REST API for sending messages.
pub struct QqBotPlugin {
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl QqBotPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract credentials from the JSON config blob.
    fn extract_credentials(credentials: &serde_json::Value) -> Result<(String, String)> {
        let app_id = credentials
            .get("appId")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appId' in QQ Bot credentials"))?;

        let client_secret = credentials
            .get("clientSecret")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'clientSecret' in QQ Bot credentials"))?;

        Ok((app_id, client_secret))
    }

    /// Get the API for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<QqBotApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("QQ Bot account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for QqBotPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::QqBot,
            display_name: "QQ Bot".to_string(),
            description: "QQ Official Bot".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Channel],
            supports_edit: false,
            supports_unsend: false,
            supports_reply: true,
            supports_threads: false,
            supports_typing: true,
            supports_buttons: true,
            supports_draft: false,
            supports_polls: false,
            supports_reactions: false,
            max_message_length: Some(4096),
            // 暂不声明原生媒体能力——dispatcher 的 to_outbound_media 优先
            // 给 MediaData::FilePath（hope-agent 本地缓存路径），但 QQ Bot
            // V2 上传 API 只接收公网 HTTPS URL，FilePath 会被静默跳过；同时
            // channel/dms 端点完全不开放媒体上传。声明 supports_media 反而
            // 让 dispatcher 不再追加链接文本兜底 → 媒体两头不到位。媒体能力
            // 完整补完跟踪 review-followups F-057（含本地附件中转 / channel
            // 端点替代方案）
            supports_media: Vec::new(),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (app_id, client_secret) = Self::extract_credentials(&account.credentials)?;

        let auth = Arc::new(QqBotAuth::new(&app_id, &client_secret));
        let api = Arc::new(QqBotApi::new(auth));

        // Validate by getting access token
        api.auth.get_token().await?;

        app_info!(
            "channel",
            "qqbot",
            "Bot authenticated with appId={} for account '{}'",
            app_id,
            account.id
        );

        // Store running account state (bot_id/bot_name will be populated from READY event)
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_id: String::new(),
                    bot_name: String::new(),
                },
            );
        }

        // Spawn the gateway event loop
        let account_id = account.id.clone();
        tokio::spawn(gateway::run_qq_gateway(api, account_id, inbound_tx, cancel));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        let mut accounts = self.accounts.lock().await;
        accounts.remove(account_id);
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;
        let scope = QqChatScope::parse(chat_id)?;
        let msg_id = payload.reply_to_message_id.as_deref();

        if !payload.buttons.is_empty() {
            return dispatch_buttons(&api, &scope, payload, msg_id).await;
        }
        if !payload.media.is_empty() && scope.supports_native_media() {
            if let Some(result) = dispatch_media(&api, &scope, payload, msg_id).await? {
                return Ok(result);
            }
            // 没有可发的 url 媒体 → 落到下面文本路径，由 dispatcher 的链接兜底
        }
        dispatch_text(&api, &scope, payload, msg_id).await
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        if let Ok(QqChatScope::C2c(openid)) = QqChatScope::parse(chat_id) {
            let api = self.get_api(account_id).await?;
            api.send_typing_c2c(openid).await?;
        }
        // group / channel / dms typing 不支持 — silently ignore
        Ok(())
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let (app_id, client_secret) = Self::extract_credentials(&account.credentials)?;
        let auth = Arc::new(QqBotAuth::new(&app_id, &client_secret));

        match auth.get_token().await {
            Ok(_) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(format!("QQ Bot ({})", app_id)),
            }),
            Err(e) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(e.to_string()),
                uptime_secs: None,
                bot_name: None,
            }),
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        let security = &account.security;

        match msg.chat_type {
            ChatType::Dm => match security.dm_policy {
                DmPolicy::Open => true,
                DmPolicy::Allowlist | DmPolicy::Pairing => {
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
            },
            ChatType::Group => {
                // Group policy: disabled -> deny all
                if security.group_policy == GroupPolicy::Disabled {
                    return false;
                }

                // Allowlist mode: group must be in allowlist
                if security.group_policy == GroupPolicy::Allowlist {
                    if security.groups.is_empty() {
                        if !security.group_allowlist.is_empty()
                            && !security.group_allowlist.contains(&msg.chat_id)
                        {
                            return false;
                        }
                    } else {
                        let has_config = security.groups.contains_key(&msg.chat_id)
                            || security.groups.contains_key("*");
                        if !has_config {
                            return false;
                        }
                    }
                }

                // Legacy group_allowlist backward compat
                if !security.group_allowlist.is_empty()
                    && security.groups.is_empty()
                    && !security.group_allowlist.contains(&msg.chat_id)
                {
                    return false;
                }

                // Account-level user allowlist
                if !security.user_allowlist.is_empty()
                    && !security.user_allowlist.contains(&msg.sender_id)
                    && !security.admin_ids.contains(&msg.sender_id)
                {
                    return false;
                }

                true
            }
            ChatType::Channel => {
                // Channels default to disabled unless explicitly configured
                let channel_config = security.channels.get(&msg.chat_id);
                match channel_config {
                    Some(cfg) => cfg.enabled != Some(false),
                    None => false,
                }
            }
            // QQ Bot doesn't have Forum chat type
            _ => false,
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_qqbot_text(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        crate::channel::traits::chunk_text(text, 4096)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let (app_id, client_secret) = Self::extract_credentials(credentials)?;
        let auth = Arc::new(QqBotAuth::new(&app_id, &client_secret));
        auth.get_token().await?;
        Ok(format!("QQ Bot ({})", app_id))
    }
}

fn extract_response_id(value: &serde_json::Value) -> String {
    value
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("sent")
        .to_string()
}

/// keyboard 按钮：c2c/group 走 native；channel/dms 降级 `[1]/[2]/[3]` 数字回复
/// （与 IRC / 微信 / Signal 一致）。
async fn dispatch_buttons(
    api: &QqBotApi,
    scope: &QqChatScope<'_>,
    payload: &ReplyPayload,
    msg_id: Option<&str>,
) -> Result<DeliveryResult> {
    let text_content = payload.text.as_deref().unwrap_or("");

    if scope.supports_native_keyboard() {
        let rows: Vec<_> = payload
            .buttons
            .iter()
            .map(|row| {
                let buttons: Vec<_> = row
                    .iter()
                    .map(|b| {
                        serde_json::json!({
                            "id": b.callback_id(),
                            "render_data": { "label": &b.text, "visited_label": &b.text },
                            "action": {
                                "type": 2,
                                "data": b.callback_id(),
                                "permission": { "type": 2 }
                            }
                        })
                    })
                    .collect();
                serde_json::json!({ "buttons": buttons })
            })
            .collect();
        let keyboard = serde_json::json!({ "content": { "rows": rows } });
        let result = api
            .send_message_with_keyboard(scope.clone(), text_content, keyboard, msg_id)
            .await?;
        return Ok(DeliveryResult::ok(extract_response_id(&result)));
    }

    let mut text = String::from(text_content);
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    let mut idx = 1;
    for row in &payload.buttons {
        for b in row {
            text.push_str(&format!("[{}] {}\n", idx, b.text));
            idx += 1;
        }
    }
    text.push_str("\nReply with the number to choose.");

    let result = match scope {
        QqChatScope::Channel(cid) => api.send_channel_message(cid, &text, msg_id).await?,
        QqChatScope::Dms(gid) => api.send_dms_message(gid, &text, msg_id).await?,
        // c2c / group 已在上面 supports_native_keyboard 分支处理
        _ => unreachable!(),
    };
    Ok(DeliveryResult::ok(extract_response_id(&result)))
}

/// QQ Bot V2 c2c/group 富媒体两步上传（POST /files → file_info → msg_type=7）。
/// 返回 `None` 表示 payload.media 全部不可发（无 url 来源），caller 应继续走
/// 文本路径让 dispatcher 链接兜底。
async fn dispatch_media(
    api: &QqBotApi,
    scope: &QqChatScope<'_>,
    payload: &ReplyPayload,
    msg_id: Option<&str>,
) -> Result<Option<DeliveryResult>> {
    let caption_root = payload.text.as_deref().unwrap_or("");
    let mut last_msg_id: Option<String> = None;

    for media in &payload.media {
        let url = match &media.data {
            MediaData::Url(u) => u.clone(),
            // 仅 url 来源（公网 HTTPS）支持；本地 FilePath / Bytes 由 dispatcher
            // 走链接文本兜底
            _ => continue,
        };
        let Some(file_type) = qq_file_type(&media.media_type) else {
            // Document / Sticker / 未开放类型
            continue;
        };
        let caption = media.caption.as_deref().unwrap_or(caption_root);

        let result = match scope {
            QqChatScope::C2c(openid) => {
                let file_info = api.post_c2c_files(openid, file_type, &url).await?;
                api.send_c2c_media(openid, &file_info, caption, msg_id)
                    .await?
            }
            QqChatScope::Group(gid) => {
                let file_info = api.post_group_files(gid, file_type, &url).await?;
                api.send_group_media(gid, &file_info, caption, msg_id)
                    .await?
            }
            // caller 已校验 supports_native_media
            _ => unreachable!(),
        };
        last_msg_id = Some(extract_response_id(&result));
    }
    Ok(last_msg_id.map(DeliveryResult::ok))
}

async fn dispatch_text(
    api: &QqBotApi,
    scope: &QqChatScope<'_>,
    payload: &ReplyPayload,
    msg_id: Option<&str>,
) -> Result<DeliveryResult> {
    let Some(text) = payload.text.as_deref().filter(|t| !t.is_empty()) else {
        return Ok(DeliveryResult::ok("no_content"));
    };
    let result = match scope {
        QqChatScope::C2c(openid) => api.send_c2c_message(openid, text, msg_id).await?,
        QqChatScope::Group(gid) => api.send_group_message(gid, text, msg_id).await?,
        QqChatScope::Channel(cid) => api.send_channel_message(cid, text, msg_id).await?,
        QqChatScope::Dms(gid) => api.send_dms_message(gid, text, msg_id).await?,
    };
    Ok(DeliveryResult::ok(extract_response_id(&result)))
}

/// 把 hope-agent 的 [`MediaType`] 映射到 QQ Bot V2 wire-level file_type
/// 数字。Document / Sticker 暂未开放（file_type=4 需特殊审核）→ None
/// 让 caller 走链接文本兜底。
fn qq_file_type(media_type: &MediaType) -> Option<u32> {
    match media_type {
        MediaType::Photo => Some(api::QqBotApi::FILE_TYPE_IMAGE),
        MediaType::Video | MediaType::Animation => Some(api::QqBotApi::FILE_TYPE_VIDEO),
        MediaType::Voice | MediaType::Audio => Some(api::QqBotApi::FILE_TYPE_VOICE),
        _ => None,
    }
}

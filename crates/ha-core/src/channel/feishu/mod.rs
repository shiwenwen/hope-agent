//! Feishu / Lark channel (飞书 / Lark Suite).
//!
//! - **Official API**: <https://open.feishu.cn/document/> (cn) /
//!   <https://open.larksuite.com/document/> (intl)
//! - **SDK / Reference**: <https://github.com/larksuite/oapi-sdk-nodejs>
//!   (官方 Node SDK，长连接帧协议 + 鉴权刷新参考实现)
//! - **Protocol**: WebSocket 事件订阅（pbbp2 protobuf 帧）+ REST
//!   `/open-apis/im/v1/messages` + `tenant_access_token` (TTL 7200s)
//! - **Last reviewed**: 2026-05-05

pub mod api;
pub mod auth;
pub mod data_cache;
pub mod format;
pub mod media;
pub mod proto;
pub mod ws_event;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use self::api::FeishuApi;
use self::auth::FeishuAuth;

/// Running account state for a single Feishu bot.
struct RunningAccount {
    api: Arc<FeishuApi>,
    // Diagnostics-only — retained for future filtering of bot-authored events.
    #[allow(dead_code)]
    bot_name: String,
    #[allow(dead_code)]
    bot_open_id: String,
}

/// Feishu (飞书) / Lark channel plugin implementation.
pub struct FeishuPlugin {
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl FeishuPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract credentials from the JSON config blob.
    fn extract_credentials(credentials: &serde_json::Value) -> Result<(String, String, String)> {
        let app_id = credentials
            .get("appId")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appId' in Feishu credentials"))?;

        let app_secret = credentials
            .get("appSecret")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appSecret' in Feishu credentials"))?;

        let domain = credentials
            .get("domain")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "feishu".to_string());

        Ok((app_id, app_secret, domain))
    }

    /// Get the API for a running account.
    async fn get_account(&self, account_id: &str) -> Result<Arc<FeishuApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Feishu account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for FeishuPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Feishu,
            display_name: "Feishu / Lark".to_string(),
            description: "Feishu (飞书) / Lark Bot".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group],
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: false,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
            ],
            supports_typing: false,
            supports_buttons: true,
            max_message_length: Some(4096),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (app_id, app_secret, domain) = Self::extract_credentials(&account.credentials)?;

        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = Arc::new(FeishuApi::new(auth));

        // Validate by fetching bot info
        let bot_info = api.get_bot_info().await?;
        let bot_name = bot_info.app_name.clone();
        let bot_open_id = bot_info.open_id.clone();

        app_info!(
            "channel",
            "feishu",
            "Bot authenticated: {} (open_id={})",
            bot_name,
            bot_open_id
        );

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_name: bot_name.clone(),
                    bot_open_id: bot_open_id.clone(),
                },
            );
        }

        // Spawn the gateway event loop
        let account_id = account.id.clone();
        tokio::spawn(ws_event::run_feishu_gateway(
            api,
            account_id,
            bot_open_id,
            inbound_tx,
            cancel,
        ));

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
        let api = self.get_account(account_id).await?;

        // Dispatcher 一般每次只塞一个 media（[`partition_media_by_channel`]），
        // 这里仍循环以便未来 dispatcher 改批量时不需要再改插件层。
        if !payload.media.is_empty() {
            let reply_to = payload.reply_to_message_id.as_deref();
            let mut last_id = String::from("no_content");
            for m in &payload.media {
                last_id = media::send_outbound_media(&api, chat_id, m, reply_to).await?;
            }
            if payload.text.is_none() && payload.buttons.is_empty() {
                return Ok(DeliveryResult::ok(last_id));
            }
        }

        // If buttons are present, send as an interactive card
        if !payload.buttons.is_empty() {
            let text_content = payload.text.as_deref().unwrap_or("");
            let button_elements: Vec<_> = payload
                .buttons
                .iter()
                .flatten()
                .map(|b| {
                    serde_json::json!({
                        "tag": "button",
                        "text": {"tag": "plain_text", "content": &b.text},
                        "type": "primary",
                        "value": b.callback_id(),
                    })
                })
                .collect();

            let card = serde_json::json!({
                "config": {"wide_screen_mode": true},
                "elements": [
                    {
                        "tag": "markdown",
                        "content": text_content
                    },
                    {
                        "tag": "action",
                        "actions": button_elements
                    }
                ]
            });

            let reply_to = payload.reply_to_message_id.as_deref();
            let msg_id = api.send_interactive_card(chat_id, card, reply_to).await?;
            return Ok(DeliveryResult::ok(msg_id));
        }

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let reply_to = payload.reply_to_message_id.as_deref();
            let message_id = api.send_message(chat_id, text, reply_to).await?;
            return Ok(DeliveryResult::ok(message_id));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // Feishu does not support typing indicators
        Ok(())
    }

    async fn edit_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_account(account_id).await?;

        if let Some(ref text) = payload.text {
            api.update_message(message_id, text).await?;
        }

        Ok(DeliveryResult::ok(message_id.to_string()))
    }

    async fn delete_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let api = self.get_account(account_id).await?;
        api.delete_message(message_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let (app_id, app_secret, domain) = Self::extract_credentials(&account.credentials)?;
        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = FeishuApi::new(auth);

        match api.get_bot_info().await {
            Ok(info) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(info.app_name),
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
                // Group policy: disabled → deny all
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
            // Feishu doesn't have Forum/Channel chat types
            _ => false,
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_feishu_text(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        crate::channel::traits::chunk_text(text, 4096)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let (app_id, app_secret, domain) = Self::extract_credentials(credentials)?;
        let auth = Arc::new(FeishuAuth::new(&app_id, &app_secret, &domain));
        let api = FeishuApi::new(auth);
        let info = api.get_bot_info().await?;
        Ok(info.app_name)
    }
}

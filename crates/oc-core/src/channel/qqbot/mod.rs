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

use self::api::QqBotApi;
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
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Document,
                MediaType::Voice,
            ],
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
        tokio::spawn(gateway::run_qq_gateway(
            api, app_id, account_id, inbound_tx, cancel,
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
        let api = self.get_api(account_id).await?;

        // Handle messages with inline keyboard buttons (approval prompts, etc.)
        if !payload.buttons.is_empty() {
            let text_content = payload.text.as_deref().unwrap_or("");
            let msg_id = payload.reply_to_message_id.as_deref();

            let rows: Vec<_> = payload
                .buttons
                .iter()
                .map(|row| {
                    let buttons: Vec<_> = row
                        .iter()
                        .map(|b| {
                            serde_json::json!({
                                "id": b.callback_id(),
                                "render_data": {
                                    "label": &b.text,
                                    "visited_label": &b.text,
                                },
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
                .send_message_with_keyboard(chat_id, text_content, keyboard, msg_id)
                .await?;

            let response_msg_id = result
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("sent")
                .to_string();

            return Ok(DeliveryResult::ok(response_msg_id));
        }

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let msg_id = payload.reply_to_message_id.as_deref();

            // Route to the correct endpoint based on chat_id prefix
            let result = if let Some(openid) = chat_id.strip_prefix("c2c:") {
                api.send_c2c_message(openid, text, msg_id).await?
            } else if let Some(group_openid) = chat_id.strip_prefix("group:") {
                api.send_group_message(group_openid, text, msg_id).await?
            } else if let Some(channel_id_str) = chat_id.strip_prefix("channel:") {
                api.send_channel_message(channel_id_str, text, msg_id)
                    .await?
            } else if let Some(guild_id) = chat_id.strip_prefix("dms:") {
                api.send_dms_message(guild_id, text, msg_id).await?
            } else {
                return Err(anyhow::anyhow!(
                    "Unknown QQ Bot chat_id format: {}",
                    crate::truncate_utf8(chat_id, 100)
                ));
            };

            // Extract message_id from response if available
            let response_msg_id = result
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("sent")
                .to_string();

            return Ok(DeliveryResult::ok(response_msg_id));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        // Typing indicator is only supported for C2C messages
        if let Some(openid) = chat_id.strip_prefix("c2c:") {
            let api = self.get_api(account_id).await?;
            api.send_typing_c2c(openid).await?;
        }
        // For group/channel, typing is not supported — silently ignore
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

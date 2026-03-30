pub mod api;
pub mod format;
pub mod media;
pub mod polling;

use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;
use api::TelegramBotApi;

/// Running account state.
struct RunningAccount {
    api: Arc<TelegramBotApi>,
    bot_id: i64,
    bot_username: String,
}

/// Telegram channel plugin implementation.
pub struct TelegramPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl TelegramPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract bot token from credentials JSON.
    fn extract_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("token")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'token' in Telegram credentials"))
    }

    /// Extract optional proxy URL from settings or global config.
    fn extract_proxy(settings: &serde_json::Value) -> Option<String> {
        // Check channel-level proxy first
        if let Some(proxy) = settings.get("proxy").and_then(|v| v.as_str()) {
            if !proxy.is_empty() {
                return Some(proxy.to_string());
            }
        }
        // Fall back to global proxy
        if let Ok(store) = crate::provider::load_store() {
            if matches!(store.proxy.mode, crate::provider::ProxyMode::Custom) {
                if let Some(ref url) = store.proxy.url {
                    if !url.is_empty() {
                        return Some(url.clone());
                    }
                }
            }
        }
        None
    }

    /// Get the API for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<TelegramBotApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Telegram account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for TelegramPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Telegram,
            display_name: "Telegram".to_string(),
            description: "Telegram Bot API channel".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Forum],
            supports_polls: true,
            supports_reactions: true,
            supports_draft: true,
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: true,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
                MediaType::Sticker,
                MediaType::Voice,
                MediaType::Animation,
            ],
            supports_typing: true,
            max_message_length: Some(4096),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let token = Self::extract_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api_root = account.settings.get("apiRoot").and_then(|v| v.as_str()).map(|s| s.to_string());

        let api = TelegramBotApi::new(
            &token,
            proxy.as_deref(),
            api_root.as_deref(),
        );

        // Validate token by calling getMe
        let me = api.get_me().await?;
        let bot_id = me.id.0 as i64;
        let bot_username = me.username().to_string();

        app_info!("channel", "telegram", "Bot authenticated: @{} (id={})", bot_username, bot_id);

        let api = Arc::new(api);

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(account.id.clone(), RunningAccount {
                api: api.clone(),
                bot_id,
                bot_username: bot_username.clone(),
            });
        }

        // Spawn polling loop
        let account_id = account.id.clone();
        tokio::spawn(polling::run_polling_loop(
            api,
            account_id,
            bot_id,
            bot_username,
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
        let api = self.get_api(account_id).await?;
        let chat_id_num: i64 = chat_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", chat_id))?;

        let thread_id: Option<i32> = payload.thread_id
            .as_ref()
            .and_then(|t| t.parse().ok());

        let reply_to: Option<i32> = payload.reply_to_message_id
            .as_ref()
            .and_then(|r| r.parse().ok());

        // Send text
        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let msg = api.send_text_with_fallback(
                chat_id_num,
                text,
                reply_to,
                thread_id,
            ).await?;

            return Ok(DeliveryResult::ok(msg.id.0.to_string()));
        }

        // Send media
        for m in &payload.media {
            let input_file = media::media_data_to_input_file(&m.data);
            match m.media_type {
                MediaType::Photo => {
                    let msg = api.send_photo(
                        chat_id_num,
                        input_file,
                        m.caption.as_deref(),
                        thread_id,
                    ).await?;
                    return Ok(DeliveryResult::ok(msg.id.0.to_string()));
                }
                _ => {
                    let msg = api.send_document(
                        chat_id_num,
                        input_file,
                        m.caption.as_deref(),
                        thread_id,
                    ).await?;
                    return Ok(DeliveryResult::ok(msg.id.0.to_string()));
                }
            }
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        let api = self.get_api(account_id).await?;
        let chat_id_num: i64 = chat_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", chat_id))?;
        api.send_typing(chat_id_num).await
    }

    async fn send_draft(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<()> {
        let api = self.get_api(account_id).await?;
        let chat_id_num: i64 = chat_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", chat_id))?;

        let thread_id: Option<i32> = payload.thread_id
            .as_ref()
            .and_then(|t| t.parse().ok());

        let reply_to: Option<i32> = payload.reply_to_message_id
            .as_ref()
            .and_then(|r| r.parse().ok());

        let draft_id = payload.draft_id.unwrap_or(1);

        let text = payload.text.as_deref().unwrap_or("");
        api.send_message_draft(chat_id_num, text, draft_id, reply_to, thread_id).await
    }

    async fn edit_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;
        let chat_id_num: i64 = chat_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", chat_id))?;
        let msg_id: i32 = message_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid message_id: {}", message_id))?;

        if let Some(ref text) = payload.text {
            api.edit_message_text(
                chat_id_num,
                msg_id,
                text,
                Some(teloxide::types::ParseMode::Html),
            ).await?;
        }

        Ok(DeliveryResult::ok(message_id.to_string()))
    }

    async fn delete_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let api = self.get_api(account_id).await?;
        let chat_id_num: i64 = chat_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid chat_id: {}", chat_id))?;
        let msg_id: i32 = message_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid message_id: {}", message_id))?;
        api.delete_message(chat_id_num, msg_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let token = Self::extract_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api = TelegramBotApi::new(&token, proxy.as_deref(), None);

        match api.get_me().await {
            Ok(me) => Ok(ChannelHealth {
                is_running: false, // probe doesn't check running state
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(format!("@{}", me.username())),
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
            ChatType::Dm => {
                match security.dm_policy {
                    DmPolicy::Open => true,
                    DmPolicy::Allowlist => {
                        security.user_allowlist.contains(&msg.sender_id)
                            || security.admin_ids.contains(&msg.sender_id)
                    }
                    DmPolicy::Pairing => {
                        // Pairing not yet implemented — fall back to allowlist
                        security.user_allowlist.contains(&msg.sender_id)
                            || security.admin_ids.contains(&msg.sender_id)
                    }
                }
            }
            ChatType::Group | ChatType::Forum | ChatType::Channel => {
                // Check group allowlist (by chat_id)
                if !security.group_allowlist.is_empty()
                    && !security.group_allowlist.contains(&msg.chat_id)
                {
                    return false;
                }
                // Check user allowlist within the group
                if !security.user_allowlist.is_empty()
                    && !security.user_allowlist.contains(&msg.sender_id)
                    && !security.admin_ids.contains(&msg.sender_id)
                {
                    return false;
                }
                true
            }
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_telegram_html(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        crate::channel::traits::chunk_text(text, 4096)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let token = Self::extract_token(credentials)?;
        let api = TelegramBotApi::new(&token, None, None);
        let me = api.get_me().await?;
        Ok(format!("@{}", me.username()))
    }
}

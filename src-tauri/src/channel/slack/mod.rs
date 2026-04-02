pub mod api;
pub mod format;
pub mod socket;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::{chunk_text, ChannelPlugin};
use crate::channel::types::*;
use api::SlackApi;

/// Running account state.
struct RunningAccount {
    api: Arc<SlackApi>,
    bot_id: String,
    bot_name: String,
}

/// Slack channel plugin implementation (Socket Mode).
pub struct SlackPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl SlackPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract bot token (xoxb-...) from credentials JSON.
    fn extract_bot_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("botToken")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'botToken' in Slack credentials"))
    }

    /// Extract app token (xapp-...) from credentials JSON.
    fn extract_app_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("appToken")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'appToken' in Slack credentials"))
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
    async fn get_api(&self, account_id: &str) -> Result<Arc<SlackApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Slack account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for SlackPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Slack,
            display_name: "Slack".to_string(),
            description: "Slack Bot (Socket Mode)".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Channel],
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: true,
            supports_typing: true,
            supports_draft: false,
            supports_polls: false,
            supports_reactions: false,
            max_message_length: Some(4000),
            supports_media: vec![MediaType::Photo, MediaType::Video, MediaType::Document],
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let bot_token = Self::extract_bot_token(&account.credentials)?;
        let app_token = Self::extract_app_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);

        let api = SlackApi::new(&bot_token, proxy.as_deref());

        // Validate token by calling auth.test
        let auth = api.auth_test().await?;
        let bot_id = auth.user_id.clone();
        let bot_name = auth.user.clone();

        app_info!(
            "channel",
            "slack",
            "Bot authenticated: {} (id={}, team={})",
            bot_name,
            bot_id,
            auth.team
        );

        let api = Arc::new(api);

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_id: bot_id.clone(),
                    bot_name: bot_name.clone(),
                },
            );
        }

        // Spawn Socket Mode loop
        let account_id = account.id.clone();
        tokio::spawn(socket::run_socket_mode(
            api,
            app_token,
            account_id,
            bot_id,
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

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let thread_ts = payload.thread_id.as_deref();
            let ts = api.chat_post_message(chat_id, text, thread_ts).await?;
            return Ok(DeliveryResult::ok(ts));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // Slack doesn't have a persistent typing API for bots.
        Ok(())
    }

    async fn edit_message(
        &self,
        account_id: &str,
        chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;

        if let Some(ref text) = payload.text {
            api.chat_update(chat_id, message_id, text).await?;
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
        api.chat_delete(chat_id, message_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let bot_token = Self::extract_bot_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api = SlackApi::new(&bot_token, proxy.as_deref());

        match api.auth_test().await {
            Ok(auth) => Ok(ChannelHealth {
                is_running: false, // probe doesn't check running state
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: Some(auth.user),
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
                DmPolicy::Allowlist => {
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
                DmPolicy::Pairing => {
                    // Pairing not yet implemented -- fall back to allowlist
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
            },
            ChatType::Group | ChatType::Forum => {
                // 1. Check group_policy: disabled -> deny all
                if security.group_policy == GroupPolicy::Disabled {
                    return false;
                }

                // 2. Resolve group config: exact match -> wildcard "*" -> None
                let group_config = security.groups.get(&msg.chat_id);
                let wildcard_config = security.groups.get("*");
                let effective_group_config = group_config.or(wildcard_config);

                // 3. Allowlist mode: group must be explicitly configured (or have wildcard)
                if security.group_policy == GroupPolicy::Allowlist {
                    if security.groups.is_empty() {
                        if !security.group_allowlist.is_empty()
                            && !security.group_allowlist.contains(&msg.chat_id)
                        {
                            return false;
                        }
                    } else if effective_group_config.is_none() {
                        return false;
                    }
                }

                // Legacy group_allowlist backward compatibility (for "open" policy too)
                if !security.group_allowlist.is_empty()
                    && security.groups.is_empty()
                    && !security.group_allowlist.contains(&msg.chat_id)
                {
                    return false;
                }

                // 4. Check group-level enabled flag
                if let Some(cfg) = effective_group_config {
                    if cfg.enabled == Some(false) {
                        return false;
                    }

                    // 5. Check topic-level enabled flag (if thread_id present)
                    if let Some(ref thread_id) = msg.thread_id {
                        if let Some(topic_cfg) = cfg.topics.get(thread_id) {
                            if topic_cfg.enabled == Some(false) {
                                return false;
                            }
                            // Topic-level sender allowlist
                            if !topic_cfg.allow_from.is_empty()
                                && !topic_cfg.allow_from.contains(&msg.sender_id)
                                && !security.admin_ids.contains(&msg.sender_id)
                            {
                                return false;
                            }
                        }
                    }

                    // 6. Group-level sender allowlist
                    if !cfg.allow_from.is_empty()
                        && !cfg.allow_from.contains(&msg.sender_id)
                        && !security.admin_ids.contains(&msg.sender_id)
                    {
                        return false;
                    }
                }

                // 7. Account-level user allowlist (if set)
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
                    None => false, // Not configured -> ignore
                }
            }
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_mrkdwn(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        chunk_text(text, 4000)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let bot_token = Self::extract_bot_token(credentials)?;
        let api = SlackApi::new(&bot_token, None);
        let auth = api.auth_test().await?;
        Ok(auth.user)
    }
}

pub mod api;
pub mod format;
pub mod gateway;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::{chunk_text, ChannelPlugin};
use crate::channel::types::*;
use api::DiscordApi;

/// Running account state for a Discord bot.
struct RunningAccount {
    api: Arc<DiscordApi>,
    bot_id: String,
    #[allow(dead_code)]
    bot_username: String,
    #[allow(dead_code)]
    application_id: String,
}

/// Discord channel plugin implementation.
pub struct DiscordPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl DiscordPlugin {
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
            .ok_or_else(|| anyhow::anyhow!("Missing 'token' in Discord credentials"))
    }

    /// Extract optional proxy URL from settings or global config.
    /// Same pattern as Telegram's extract_proxy.
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

    /// Sync slash commands to Discord's Application Commands API.
    /// Called once after successful authentication. Non-fatal on failure.
    async fn sync_commands_to_discord(api: &DiscordApi, application_id: &str) {
        let commands = crate::slash_commands::registry::all_commands();

        // Convert to Discord Application Command format (type 1 = CHAT_INPUT)
        let discord_commands: Vec<serde_json::Value> = commands
            .iter()
            .map(|cmd| {
                let mut command = serde_json::json!({
                    "name": cmd.name,
                    "description": cmd.description_en(),
                    "type": 1, // CHAT_INPUT
                });

                // Add string option for commands that accept arguments
                if cmd.has_args {
                    if let Some(ref options) = cmd.arg_options {
                        // Use choices for commands with predefined options
                        let choices: Vec<serde_json::Value> = options
                            .iter()
                            .map(|opt| {
                                serde_json::json!({
                                    "name": opt,
                                    "value": opt
                                })
                            })
                            .collect();
                        command["options"] = serde_json::json!([{
                            "name": "value",
                            "description": cmd.description_en(),
                            "type": 3, // STRING
                            "required": !cmd.args_optional,
                            "choices": choices
                        }]);
                    } else {
                        command["options"] = serde_json::json!([{
                            "name": "value",
                            "description": cmd.arg_placeholder.as_deref().unwrap_or("value"),
                            "type": 3, // STRING
                            "required": !cmd.args_optional,
                        }]);
                    }
                }

                command
            })
            .collect();

        let count = discord_commands.len();
        match api
            .bulk_overwrite_global_commands(application_id, discord_commands)
            .await
        {
            Ok(()) => {
                app_info!(
                    "channel",
                    "discord",
                    "Synced {} commands to Discord application",
                    count
                );
            }
            Err(e) => {
                app_warn!(
                    "channel",
                    "discord",
                    "Failed to sync Discord application commands: {}",
                    e
                );
            }
        }
    }

    /// Get the API for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<DiscordApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Discord account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for DiscordPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Discord,
            display_name: "Discord".to_string(),
            description: "Discord Bot channel".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![
                ChatType::Dm,
                ChatType::Group,
                ChatType::Forum,
                ChatType::Channel,
            ],
            supports_polls: false,
            supports_reactions: true,
            supports_draft: false,
            supports_edit: true,
            supports_unsend: true,
            supports_reply: true,
            supports_threads: true,
            supports_media: vec![
                MediaType::Photo,
                MediaType::Video,
                MediaType::Audio,
                MediaType::Document,
            ],
            supports_typing: true,
            max_message_length: Some(2000),
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

        let api = DiscordApi::new(&token, proxy.as_deref());

        // Validate token by calling GET /users/@me
        let me = api.get_current_user().await?;
        let bot_id = me["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'id' in Discord user response"))?
            .to_string();
        let bot_username = me["username"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        // Get application ID from the bot user object
        // The bot's user ID is also the application ID for bot applications
        let application_id = me
            .get("application")
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            // Fallback: for bots, application_id typically matches bot_id
            .unwrap_or_else(|| bot_id.clone());

        app_info!(
            "channel",
            "discord",
            "Bot authenticated: {} (id={})",
            bot_username,
            bot_id
        );

        // Sync slash commands to Discord
        Self::sync_commands_to_discord(&api, &application_id).await;

        let api = Arc::new(api);

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api: api.clone(),
                    bot_id: bot_id.clone(),
                    bot_username: bot_username.clone(),
                    application_id,
                },
            );
        }

        // Spawn gateway WebSocket loop
        let account_id = account.id.clone();
        tokio::spawn(gateway::run_gateway_loop(
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

        let reply_to = payload.reply_to_message_id.as_deref();
        let thread_id = payload.thread_id.as_deref();

        // Send text
        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            let msg = api
                .create_message(chat_id, text, reply_to, thread_id)
                .await?;

            let msg_id = msg["id"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            return Ok(DeliveryResult::ok(msg_id));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        let api = self.get_api(account_id).await?;
        api.trigger_typing(chat_id).await
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
            api.edit_message(chat_id, message_id, text).await?;
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
        api.delete_message(chat_id, message_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let token = Self::extract_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api = DiscordApi::new(&token, proxy.as_deref());

        match api.get_current_user().await {
            Ok(me) => {
                let name = me["username"].as_str().unwrap_or("unknown");
                Ok(ChannelHealth {
                    is_running: false,
                    last_probe: Some(chrono::Utc::now().to_rfc3339()),
                    probe_ok: Some(true),
                    error: None,
                    uptime_secs: None,
                    bot_name: Some(name.to_string()),
                })
            }
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
                    // Pairing not yet implemented — fall back to allowlist
                    security.user_allowlist.contains(&msg.sender_id)
                        || security.admin_ids.contains(&msg.sender_id)
                }
            },
            ChatType::Group | ChatType::Forum => {
                // 1. Check group_policy: disabled → deny all
                if security.group_policy == GroupPolicy::Disabled {
                    return false;
                }

                // 2. Resolve group config: exact match → wildcard "*" → None
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

                // Legacy group_allowlist backward compatibility
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
                    None => false,
                }
            }
        }
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_discord(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        chunk_text(text, 2000)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let token = Self::extract_token(credentials)?;
        let api = DiscordApi::new(&token, None);
        let me = api.get_current_user().await?;
        let username = me["username"]
            .as_str()
            .unwrap_or("unknown");
        Ok(username.to_string())
    }
}

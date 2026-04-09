pub mod api;
pub mod format;
pub mod webhook;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;
use crate::channel::webhook_server::{WebhookServer, DEFAULT_WEBHOOK_PORT};

use self::api::LineApi;
use self::webhook::ReplyTokenStore;

/// Global webhook server instance, shared across all webhook-based channels.
/// Same instance used by Google Chat, LINE, and any future webhook-based plugins.
static WEBHOOK_SERVER: tokio::sync::OnceCell<Arc<WebhookServer>> =
    tokio::sync::OnceCell::const_new();

/// Get or start the global webhook server.
async fn get_or_start_webhook_server() -> Result<Arc<WebhookServer>> {
    let server = WEBHOOK_SERVER
        .get_or_try_init(|| async { WebhookServer::start(DEFAULT_WEBHOOK_PORT).await })
        .await?;
    Ok(server.clone())
}

/// Running account state for a LINE bot.
struct RunningAccount {
    api: Arc<LineApi>,
    reply_tokens: ReplyTokenStore,
}

/// LINE Messaging API channel plugin.
///
/// Uses webhook-based inbound message delivery. The user must configure
/// a tunnel (ngrok, cloudflared) to expose the local webhook server,
/// then set the webhook URL in the LINE Developers Console.
pub struct LinePlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl LinePlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract channel access token from credentials JSON.
    fn extract_access_token(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("channelAccessToken")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("Missing 'channelAccessToken' in LINE credentials")
            })
    }

    /// Extract channel secret from credentials JSON.
    fn extract_channel_secret(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("channelSecret")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing 'channelSecret' in LINE credentials"))
    }

    /// Extract optional proxy URL from settings or global config.
    fn extract_proxy(settings: &serde_json::Value) -> Option<String> {
        if let Some(proxy) = settings.get("proxy").and_then(|v| v.as_str()) {
            if !proxy.is_empty() {
                return Some(proxy.to_string());
            }
        }
        crate::provider::active_custom_proxy_url()
    }

    /// Get the API and reply token store for a running account.
    async fn get_account_state(
        &self,
        account_id: &str,
    ) -> Result<(Arc<LineApi>, ReplyTokenStore)> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| (a.api.clone(), a.reply_tokens.clone()))
            .ok_or_else(|| anyhow::anyhow!("LINE account '{}' is not running", account_id))
    }

    /// Get or start the shared global webhook server.
    async fn ensure_webhook_server(&self) -> Result<Arc<WebhookServer>> {
        get_or_start_webhook_server().await
    }
}

#[async_trait]
impl ChannelPlugin for LinePlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::Line,
            display_name: "LINE".to_string(),
            description: "LINE Messaging API".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group],
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: false,
            supports_unsend: false,
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
            max_message_length: Some(5000),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        _cancel: CancellationToken,
    ) -> Result<()> {
        let access_token = Self::extract_access_token(&account.credentials)?;
        let channel_secret = Self::extract_channel_secret(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);

        let api = LineApi::new(&access_token, proxy.as_deref());

        // Validate credentials by getting bot info
        let bot_info = api.get_bot_info().await?;
        let bot_name = bot_info
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        app_info!(
            "channel",
            "line",
            "Bot authenticated: {} (account={})",
            bot_name,
            account.id
        );

        let api = Arc::new(api);
        let reply_tokens = webhook::new_reply_token_store();

        // Ensure webhook server is running and register handler
        let webhook_server = self.ensure_webhook_server().await?;
        let handler = webhook::create_webhook_handler(
            api.clone(),
            account.id.clone(),
            channel_secret.clone(),
            inbound_tx,
            reply_tokens.clone(),
        );
        webhook_server
            .register_handler("line", &account.id, handler)
            .await;

        app_info!(
            "channel",
            "line",
            "Webhook handler registered at /webhook/line/{}",
            account.id
        );

        // Store running account
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api,
                    reply_tokens,
                },
            );
        }

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        // Remove account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.remove(account_id);
        }

        // Unregister webhook handler
        if let Some(ws) = WEBHOOK_SERVER.get() {
            ws.unregister_handler("line", account_id).await;
        }

        app_info!(
            "channel",
            "line",
            "Stopped account '{}'",
            account_id
        );
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let (api, reply_tokens) = self.get_account_state(account_id).await?;

        let text = match &payload.text {
            Some(t) if !t.is_empty() => t.clone(),
            _ => return Ok(DeliveryResult::ok("empty")),
        };

        // Build messages: use a buttons template if buttons are present,
        // otherwise send a plain text message.
        let messages = if !payload.buttons.is_empty() {
            let actions: Vec<_> = payload
                .buttons
                .iter()
                .flatten()
                .take(3) // LINE buttons template supports at most 4 actions
                .map(|b| {
                    serde_json::json!({
                        "type": "postback",
                        "label": &b.text,
                        "data": b.callback_id(),
                    })
                })
                .collect();

            vec![serde_json::json!({
                "type": "template",
                "altText": &text,
                "template": {
                    "type": "buttons",
                    "text": crate::truncate_utf8(&text, 160),
                    "actions": actions,
                }
            })]
        } else {
            vec![serde_json::json!({
                "type": "text",
                "text": text,
            })]
        };

        // Try to use reply token first (valid for ~1 minute)
        let reply_token = {
            let mut tokens = reply_tokens.lock().await;
            if let Some((token, ts)) = tokens.remove(chat_id) {
                // Only use if less than 50 seconds old
                if ts.elapsed().as_secs() < 50 {
                    Some(token)
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(token) = reply_token {
            match api.reply_message(&token, messages.clone()).await {
                Ok(()) => {
                    app_debug!(
                        "channel",
                        "line",
                        "Replied to chat {} via reply token",
                        chat_id
                    );
                    return Ok(DeliveryResult::ok("reply"));
                }
                Err(e) => {
                    // Reply token expired or invalid, fall through to push
                    app_debug!(
                        "channel",
                        "line",
                        "Reply token failed (falling back to push): {}",
                        e
                    );
                }
            }
        }

        // Fall back to push API
        api.push_message(chat_id, messages).await?;
        app_debug!(
            "channel",
            "line",
            "Pushed message to chat {}",
            chat_id
        );
        Ok(DeliveryResult::ok("push"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // LINE does not support typing indicators via Messaging API
        Ok(())
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let access_token = Self::extract_access_token(&account.credentials)?;
        let proxy = Self::extract_proxy(&account.settings);
        let api = LineApi::new(&access_token, proxy.as_deref());

        match api.get_bot_info().await {
            Ok(info) => {
                let name = info
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
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
        crate::channel::traits::default_check_access(
            account,
            msg,
            &[ChatType::Dm, ChatType::Group],
        )
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_line(markdown)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let access_token = Self::extract_access_token(credentials)?;
        // Also validate that channel_secret is present
        Self::extract_channel_secret(credentials)?;

        let api = LineApi::new(&access_token, None);
        let info = api.get_bot_info().await?;
        let name = info
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(name.to_string())
    }
}

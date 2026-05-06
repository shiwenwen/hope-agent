//! Google Chat (Workspace) channel.
//!
//! - **Official API**: <https://developers.google.com/workspace/chat>
//! - **SDK / Reference**: <https://developers.google.com/workspace/chat/api-overview>
//! - **Protocol**: HTTPS Webhook（必须验证 Google 签发的 Bearer JWT）+ REST
//!   `chat.googleapis.com/v1/spaces/{}/messages`；Service Account JWT-bearer
//!   交换 OAuth token，scope 含 `chat.messages.create` / `chat.spaces.readonly`
//! - **Last reviewed**: 2026-05-05

pub mod api;
pub mod auth;
pub mod format;
pub mod jwt;
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
use api::GoogleChatApi;
use auth::GoogleChatAuth;

/// Global webhook server instance, shared across all webhook-based channels.
static WEBHOOK_SERVER: tokio::sync::OnceCell<Arc<WebhookServer>> =
    tokio::sync::OnceCell::const_new();

/// Get or start the global webhook server.
async fn get_or_start_webhook_server() -> Result<Arc<WebhookServer>> {
    let server = WEBHOOK_SERVER
        .get_or_try_init(|| async { WebhookServer::start(DEFAULT_WEBHOOK_PORT).await })
        .await?;
    Ok(server.clone())
}

/// Running account state for a Google Chat bot.
struct RunningAccount {
    api: Arc<GoogleChatApi>,
    cancel: CancellationToken,
}

/// Google Chat channel plugin implementation.
///
/// Uses Google Workspace service account authentication and webhook-based
/// inbound message handling via the shared webhook server.
pub struct GoogleChatPlugin {
    /// Running accounts keyed by account_id.
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl GoogleChatPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract and parse service account credentials JSON from the credentials blob.
    fn extract_credentials_json(credentials: &serde_json::Value) -> Result<String> {
        let raw = credentials
            .get("credentialsJson")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("Missing 'credentialsJson' in Google Chat credentials")
            })?;
        Ok(raw)
    }

    /// Extract the optional webhook base URL from credentials.
    fn extract_webhook_base_url(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("webhookBaseUrl")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract the bot's Google Cloud project number (used as JWT `aud` claim
    /// when verifying webhook events).
    ///
    /// **Optional**：缺失只让入站 webhook 拒绝（无法验签 JWT），出站发送
    /// 不依赖此字段——升级前保存的旧账号没有这字段，必须能继续启动出站
    /// 能力，等用户在编辑弹窗补全 projectNumber 后再开入站。
    fn extract_project_number(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("projectNumber")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Get the API for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<GoogleChatApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("Google Chat account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for GoogleChatPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::GoogleChat,
            display_name: "Google Chat".to_string(),
            description: "Google Chat (Workspace)".to_string(),
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
            supports_threads: true,
            // TODO: native Google Chat media (media:upload +
            // messages.create) not yet implemented. Dispatcher falls back to
            // a download-link text for now.
            supports_media: Vec::new(),
            supports_typing: false,
            supports_buttons: true,
            max_message_length: Some(4096),
            supports_card_stream: false,
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let cred_json = Self::extract_credentials_json(&account.credentials)?;
        let _webhook_base_url = Self::extract_webhook_base_url(&account.credentials);
        let project_number = Self::extract_project_number(&account.credentials);

        // Create auth and API instances
        let auth = GoogleChatAuth::from_json(&cred_json)?;
        let client_email = auth.client_email().to_string();
        let auth = Arc::new(auth);
        let api = Arc::new(GoogleChatApi::new(auth));

        // Validate credentials by listing spaces
        api.list_spaces()
            .await
            .map_err(|e| anyhow::anyhow!("Google Chat credential validation failed: {}", e))?;

        app_info!(
            "channel",
            "googlechat",
            "Authenticated as service account: {}",
            client_email
        );

        // Start webhook server and register handler only when projectNumber
        // 已配置；缺失时出站继续工作，入站静默 disable（不阻 start_account）
        if let Some(project_number) = project_number {
            let webhook_server = get_or_start_webhook_server().await?;
            let handler = webhook::create_webhook_handler(
                api.clone(),
                account.id.clone(),
                project_number,
                inbound_tx,
            );
            webhook_server
                .register_handler("googlechat", &account.id, handler)
                .await;

            app_info!(
                "channel",
                "googlechat",
                "Webhook handler registered at /webhook/googlechat/{}",
                account.id
            );
        } else {
            app_warn!(
                "channel",
                "googlechat",
                "Account '{}' has no projectNumber; inbound webhooks disabled. \
                 Edit account credentials to enable receiving messages.",
                account.id
            );
        }

        // Store running account state
        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(account.id.clone(), RunningAccount { api, cancel });
        }

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        // Unregister webhook handler
        if let Some(server) = WEBHOOK_SERVER.get() {
            server.unregister_handler("googlechat", account_id).await;
        }

        let mut accounts = self.accounts.lock().await;
        if let Some(account) = accounts.remove(account_id) {
            account.cancel.cancel();
        }

        app_info!("channel", "googlechat", "Stopped account '{}'", account_id);
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

            // Build Card v2 widgets for approval buttons if present
            let cards_v2 = if payload.buttons.is_empty() {
                None
            } else {
                let button_widgets: Vec<_> = payload
                    .buttons
                    .iter()
                    .flatten()
                    .map(|b| {
                        serde_json::json!({
                            "buttonList": {
                                "buttons": [{
                                    "text": &b.text,
                                    "onClick": {
                                        "action": {
                                            "function": b.callback_id(),
                                        }
                                    }
                                }]
                            }
                        })
                    })
                    .collect();

                Some(vec![serde_json::json!({
                    "cardId": "approval",
                    "card": {
                        "sections": [{
                            "widgets": button_widgets
                        }]
                    }
                })])
            };

            let thread_key = payload.thread_id.as_deref();
            let cards_ref = cards_v2.as_deref();
            let result = api
                .send_message(chat_id, text, thread_key, cards_ref)
                .await?;

            let msg_name = result
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            return Ok(DeliveryResult::ok(msg_name));
        }

        Ok(DeliveryResult::ok("no_content"))
    }

    async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
        // Google Chat does not support typing indicators for bots
        Ok(())
    }

    async fn edit_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;

        if let Some(ref text) = payload.text {
            let result = api.update_message(message_id, text).await?;
            let msg_name = result
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(message_id)
                .to_string();
            return Ok(DeliveryResult::ok(msg_name));
        }

        Ok(DeliveryResult::ok(message_id.to_string()))
    }

    async fn delete_message(
        &self,
        account_id: &str,
        _chat_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let api = self.get_api(account_id).await?;
        api.delete_message(message_id).await
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let cred_json = Self::extract_credentials_json(&account.credentials)?;
        let auth = GoogleChatAuth::from_json(&cred_json)?;
        let auth = Arc::new(auth);
        let api = GoogleChatApi::new(auth);

        match api.list_spaces().await {
            Ok(_) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(true),
                error: None,
                uptime_secs: None,
                bot_name: None,
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
        crate::channel::traits::default_check_access(account, msg, &[ChatType::Dm, ChatType::Group])
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        format::markdown_to_googlechat(markdown)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let cred_json = Self::extract_credentials_json(credentials)?;
        let auth = GoogleChatAuth::from_json(&cred_json)?;
        let client_email = auth.client_email().to_string();
        let auth = Arc::new(auth);
        let api = GoogleChatApi::new(auth);

        // Validate by calling list spaces
        api.list_spaces().await?;

        Ok(client_email)
    }
}

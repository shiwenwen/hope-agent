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
pub mod inbound_media;
pub mod jwt;
pub mod webhook;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::channel::webhook_server::{WebhookServer, DEFAULT_WEBHOOK_PORT};
use api::GoogleChatApi;
use auth::GoogleChatAuth;
use ha_core::channel::traits::ChannelPlugin;
use ha_core::channel::types::*;

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
    standard_markdown: bool,
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

    async fn get_outbound_account(&self, account_id: &str) -> Result<(Arc<GoogleChatApi>, bool)> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|account| (account.api.clone(), account.standard_markdown))
            .ok_or_else(|| anyhow::anyhow!("Google Chat account '{}' is not running", account_id))
    }
}

fn google_chat_button_cards(buttons: &[Vec<InlineButton>]) -> Vec<serde_json::Value> {
    let widgets = buttons
        .iter()
        .map(|row| {
            serde_json::json!({
                "buttonList": {
                    "buttons": row.iter().map(|button| {
                        serde_json::json!({
                            "text": &button.text,
                            "onClick": {
                                "action": {
                                    "function": button.callback_id(),
                                }
                            }
                        })
                    }).collect::<Vec<_>>()
                }
            })
        })
        .collect::<Vec<_>>();
    vec![serde_json::json!({
        "cardId": "approval",
        "card": {
            "sections": [{
                "widgets": widgets
            }]
        }
    })]
}

fn validate_google_chat_buttons(
    buttons: &[Vec<InlineButton>],
) -> std::result::Result<(), ReplyStreamError> {
    if buttons.is_empty() {
        return Ok(());
    }
    let total = buttons.iter().map(Vec::len).sum::<usize>();
    if buttons.len() > 100 || buttons.iter().any(Vec::is_empty) || total == 0 || total > 100 {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Google Chat cards require 1 to 100 buttons in non-empty rows",
        ));
    }
    for button in buttons.iter().flatten() {
        let label_chars = button.text.chars().count();
        if label_chars == 0 || label_chars > 128 || button.text.chars().any(char::is_control) {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Google Chat button labels must contain 1 to 128 printable characters",
            ));
        }
        if button.url.is_some() {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Google Chat URL actions are not enabled by this adapter",
            ));
        }
        let callback = button.callback_id();
        if callback.is_empty() || callback.len() > 256 || callback.chars().any(char::is_control) {
            return Err(ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Google Chat callback data must contain 1 to 256 UTF-8 bytes",
            ));
        }
    }
    let encoded_bytes = serde_json::to_vec(&google_chat_button_cards(buttons))
        .map_err(|_| {
            ReplyStreamError::new(
                ReplyStreamErrorKind::InvalidContent,
                "Google Chat button card could not be encoded",
            )
        })?
        .len();
    if encoded_bytes > 32 * 1024 {
        return Err(ReplyStreamError::new(
            ReplyStreamErrorKind::InvalidContent,
            "Google Chat button card exceeds the 32 KiB payload limit",
        ));
    }
    Ok(())
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
            // Google Chat media.upload is user-auth only
            // (`chat.messages.create` / `chat.messages`); this plugin uses
            // service account app-auth (`chat.bot`). Keep link fallback until
            // a separate user-OAuth credential mode exists.
            supports_media: Vec::new(),
            supports_typing: false,
            supports_buttons: true,
            streaming_preview_max_bytes: Some(4096),
            supports_card_stream: false,
            native_reply: None,
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: mpsc::Sender<InboundEvent>,
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
            accounts.insert(
                account.id.clone(),
                RunningAccount {
                    api,
                    cancel,
                    standard_markdown: account.google_chat_standard_markdown_enabled(),
                },
            );
        }

        Ok(())
    }

    async fn materialize_pending_media(
        &self,
        account: &ChannelAccountConfig,
        msg: &mut MsgContext,
    ) -> Result<()> {
        let pending = crate::channel::inbound_media_common::take_pending_refs::<
            inbound_media::ParsedMediaRef,
        >(msg);
        if pending.is_empty() {
            return Ok(());
        }
        let api = self.get_api(&account.id).await?;
        let results = futures_util::future::join_all(
            pending
                .iter()
                .map(|p| inbound_media::materialize_inbound(&api, p, &account.id)),
        )
        .await;
        for m in results.into_iter().flatten() {
            msg.media.push(m);
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
        validate_google_chat_buttons(&payload.buttons)?;
        let (api, standard_markdown) = self.get_outbound_account(account_id).await?;

        if let Some(ref text) = payload.text {
            if text.is_empty() {
                return Ok(DeliveryResult::ok("empty"));
            }

            // Build Card v2 widgets for approval buttons if present
            let cards_v2 = if payload.buttons.is_empty() {
                None
            } else {
                Some(google_chat_button_cards(&payload.buttons))
            };

            let thread_key = payload.thread_id.as_deref();
            let cards_ref = cards_v2.as_deref();
            let compiled_text = if standard_markdown {
                format::compile_standard_markdown_mentions(text)
            } else {
                format::markdown_to_googlechat(text)
            };
            let result = api
                .send_message(
                    chat_id,
                    &compiled_text,
                    thread_key,
                    cards_ref,
                    standard_markdown,
                )
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

    fn validate_reply_buttons(
        &self,
        buttons: &[Vec<InlineButton>],
    ) -> std::result::Result<(), ReplyStreamError> {
        validate_google_chat_buttons(buttons)
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
            // `markupSyntax` is create-only. Updates keep the legacy Chat
            // syntax regardless of the account's create-message flag.
            let legacy_text = format::markdown_to_googlechat(text);
            let result = api.update_message(message_id, &legacy_text).await?;
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
                capability_snapshot: None,
            }),
            Err(e) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(e.to_string()),
                uptime_secs: None,
                bot_name: None,
                capability_snapshot: None,
            }),
        }
    }

    fn check_access(&self, account: &ChannelAccountConfig, msg: &MsgContext) -> bool {
        ha_core::channel::traits::default_check_access(
            account,
            msg,
            &[ChatType::Dm, ChatType::Group],
        )
    }

    fn markdown_to_native(&self, markdown: &str) -> String {
        // Account-scoped standard/legacy compilation happens at the outbound
        // adapter boundary where the concrete account is known.
        markdown.to_string()
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

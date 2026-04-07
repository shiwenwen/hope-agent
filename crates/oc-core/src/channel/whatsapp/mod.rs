pub mod api;
pub mod format;
pub mod polling;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::channel::traits::{chunk_text, ChannelPlugin};
use crate::channel::types::*;

use self::api::WhatsAppApi;

/// Running account state for a WhatsApp bridge connection.
struct RunningAccount {
    api: Arc<WhatsAppApi>,
}

/// WhatsApp channel plugin implementation.
///
/// Connects to an external bridge HTTP API that the user deploys.
/// Follows the same bridge-polling architecture as the WeChat plugin.
///
/// Credentials JSON: `{ "baseUrl": "http://localhost:3000", "token": "optional-auth-token" }`
pub struct WhatsAppPlugin {
    accounts: Mutex<HashMap<String, RunningAccount>>,
}

impl WhatsAppPlugin {
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Extract base URL from credentials JSON.
    fn extract_base_url(credentials: &serde_json::Value) -> Result<String> {
        credentials
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("Missing 'baseUrl' in WhatsApp credentials"))
    }

    /// Extract optional auth token from credentials JSON.
    fn extract_token(credentials: &serde_json::Value) -> Option<String> {
        credentials
            .get("token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// Get the API client for a running account.
    async fn get_api(&self, account_id: &str) -> Result<Arc<WhatsAppApi>> {
        let accounts = self.accounts.lock().await;
        accounts
            .get(account_id)
            .map(|a| a.api.clone())
            .ok_or_else(|| anyhow::anyhow!("WhatsApp account '{}' is not running", account_id))
    }
}

#[async_trait]
impl ChannelPlugin for WhatsAppPlugin {
    fn meta(&self) -> ChannelMeta {
        ChannelMeta {
            id: ChannelId::WhatsApp,
            display_name: "WhatsApp".to_string(),
            description: "WhatsApp via bridge service".to_string(),
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
            supports_typing: true,
            supports_buttons: false,
            max_message_length: Some(65536),
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: tokio::sync::mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let base_url = Self::extract_base_url(&account.credentials)?;
        let token = Self::extract_token(&account.credentials);
        let api = Arc::new(WhatsAppApi::new(&base_url, token));

        // Verify bridge is reachable
        let health = api.health().await?;
        if !health.connected {
            return Err(anyhow::anyhow!(
                "WhatsApp bridge is not connected: {}",
                health
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        let account_name = health
            .account_name
            .clone()
            .unwrap_or_else(|| "WhatsApp".to_string());

        {
            let mut accounts = self.accounts.lock().await;
            accounts.insert(account.id.clone(), RunningAccount { api: api.clone() });
        }

        app_info!(
            "channel",
            "whatsapp",
            "WhatsApp account '{}' ({}) connected via bridge at {}",
            account.label,
            account_name,
            base_url
        );

        // Spawn polling loop
        tokio::spawn(polling::run_whatsapp_polling(
            api,
            account.id.clone(),
            inbound_tx,
            cancel,
        ));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        self.accounts.lock().await.remove(account_id);
        Ok(())
    }

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;

        let text = payload.text.as_deref().map(str::trim).unwrap_or("");
        if text.is_empty() {
            return Ok(DeliveryResult::ok("empty"));
        }

        let reply_to = payload.reply_to_message_id.as_deref();
        let resp = api.send_message(chat_id, text, reply_to).await?;

        if let Some(err) = resp.error {
            return Ok(DeliveryResult::err(err));
        }

        Ok(DeliveryResult::ok(
            resp.message_id.unwrap_or_else(|| "sent".to_string()),
        ))
    }

    async fn send_typing(&self, account_id: &str, chat_id: &str) -> Result<()> {
        let api = match self.get_api(account_id).await {
            Ok(api) => api,
            Err(_) => return Ok(()),
        };
        if let Err(err) = api.send_typing(chat_id).await {
            app_debug!(
                "channel",
                "whatsapp",
                "send_typing failed for '{}': {}",
                chat_id,
                err
            );
        }
        Ok(())
    }

    async fn probe(&self, account: &ChannelAccountConfig) -> Result<ChannelHealth> {
        let base_url = Self::extract_base_url(&account.credentials)?;
        let token = Self::extract_token(&account.credentials);
        let api = WhatsAppApi::new(&base_url, token);

        match api.health().await {
            Ok(health) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(health.connected),
                error: health.error,
                uptime_secs: None,
                bot_name: health
                    .account_name
                    .or(health.phone),
            }),
            Err(err) => Ok(ChannelHealth {
                is_running: false,
                last_probe: Some(chrono::Utc::now().to_rfc3339()),
                probe_ok: Some(false),
                error: Some(err.to_string()),
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
        format::markdown_to_whatsapp(markdown)
    }

    fn chunk_message(&self, text: &str) -> Vec<String> {
        chunk_text(text, 65536)
    }

    async fn validate_credentials(&self, credentials: &serde_json::Value) -> Result<String> {
        let base_url = Self::extract_base_url(credentials)?;
        let token = Self::extract_token(credentials);
        let api = WhatsAppApi::new(&base_url, token);
        let health = api.health().await?;

        if !health.connected {
            return Err(anyhow::anyhow!(
                "WhatsApp bridge is not connected: {}",
                health
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        Ok(health
            .account_name
            .or(health.phone)
            .unwrap_or_else(|| "WhatsApp".to_string()))
    }
}

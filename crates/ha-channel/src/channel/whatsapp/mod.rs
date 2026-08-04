//! WhatsApp channel via third-party bridge (HTTP polling).
//!
//! - **Official API**: WhatsApp Cloud API
//!   <https://developers.facebook.com/docs/whatsapp/cloud-api>（hope-agent 不直接对接，
//!   需用户自部署 bridge）
//! - **SDK / Reference**: <https://github.com/tulir/whatsmeow>（推荐 bridge 实现，
//!   逆向 WA 协议提供 Go API），<https://github.com/WhiskeySockets/Baileys>（Node.js）
//! - **Protocol**: 通用 bridge HTTP 长轮询 — `GET /api/messages?since={ts}` +
//!   `POST /api/send` / `POST /api/media`；timestamp 单位 = Unix 秒（UTC，bridge 实现需遵守）
//! - **Last reviewed**: 2026-05-20

pub mod api;
pub mod format;
pub mod inbound_media;
pub mod media;
pub mod polling;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use ha_core::channel::traits::{chunk_text, ChannelPlugin};
use ha_core::channel::types::*;

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
                MediaType::Sticker,
                MediaType::Voice,
                MediaType::Animation,
            ],
            supports_typing: true,
            supports_buttons: false,
            streaming_preview_max_bytes: Some(65536),
            supports_card_stream: false,
        }
    }

    async fn start_account(
        &self,
        account: &ChannelAccountConfig,
        inbound_tx: tokio::sync::mpsc::Sender<InboundEvent>,
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
                health.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }
        health.validate_security()?;

        let account_name = health
            .account_name
            .clone()
            .unwrap_or_else(|| "WhatsApp".to_string());
        let bridge_identity = match (health.implementation.as_deref(), health.version.as_deref()) {
            (Some(implementation), Some(version)) => {
                format!("{} {}", implementation.trim(), version.trim())
            }
            (Some(implementation), None) => implementation.trim().to_string(),
            _ => "legacy/undisclosed".to_string(),
        };
        let advertised_capabilities = health.capability_names();
        if health
            .implementation
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            app_warn!(
                "channel",
                "whatsapp",
                "WhatsApp bridge did not advertise implementation/version; Baileys security status cannot be verified"
            );
        }

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
            sanitized_bridge_url_for_log(&base_url)
        );
        app_info!(
            "channel",
            "whatsapp",
            "WhatsApp bridge identity={} capabilities={}",
            ha_core::truncate_utf8(&ha_core::logging::redact_sensitive(&bridge_identity), 120),
            if advertised_capabilities.is_empty() {
                "none-advertised".to_string()
            } else {
                advertised_capabilities.join(",")
            }
        );

        // Spawn polling loop
        tokio::spawn(polling::run_whatsapp_polling(
            api,
            account.id.clone(),
            account.label.clone(),
            inbound_tx,
            cancel,
        ));

        Ok(())
    }

    async fn stop_account(&self, account_id: &str) -> Result<()> {
        self.accounts.lock().await.remove(account_id);
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
        let api = {
            let accounts = self.accounts.lock().await;
            accounts
                .get(&account.id)
                .map(|a| a.api.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!("WhatsApp account '{}' is not running", account.id)
                })?
        };
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

    async fn send_message(
        &self,
        account_id: &str,
        chat_id: &str,
        payload: &ReplyPayload,
    ) -> Result<DeliveryResult> {
        let api = self.get_api(account_id).await?;

        if !payload.media.is_empty() {
            let prepared = match media::prepare_whatsapp_media(&payload.media).await {
                Ok(prepared) => prepared,
                Err(e) => return Ok(DeliveryResult::err(e.to_string())),
            };

            let mut last_msg_id = None;
            for (idx, item) in prepared.iter().enumerate() {
                let caption =
                    media::caption_text(payload.text.as_deref(), item.caption.as_deref(), idx == 0);
                let resp = api
                    .send_media(
                        chat_id,
                        item.media_type,
                        &item.media,
                        caption.as_deref(),
                        Some(&item.filename),
                        Some(&item.mime_type),
                        payload.reply_to_message_id.as_deref(),
                    )
                    .await?;
                if let Some(err) = resp.delivery_error() {
                    return Ok(DeliveryResult::err(err));
                }
                last_msg_id = Some(resp.message_id.unwrap_or_else(|| {
                    if prepared.len() == 1 {
                        "sent".to_string()
                    } else {
                        format!("sent:{}", idx + 1)
                    }
                }));
            }

            return Ok(DeliveryResult::ok(
                last_msg_id.unwrap_or_else(|| "no_content".to_string()),
            ));
        }

        let text = payload.text.as_deref().map(str::trim).unwrap_or("");
        if text.is_empty() {
            return Ok(DeliveryResult::err("WhatsApp message text is empty"));
        }

        let reply_to = payload.reply_to_message_id.as_deref();
        let resp = api.send_message(chat_id, text, reply_to).await?;

        if let Some(err) = resp.delivery_error() {
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
            Ok(health) => {
                let security_error = health.validate_security().err().map(|e| e.to_string());
                Ok(ChannelHealth {
                    is_running: false,
                    last_probe: Some(chrono::Utc::now().to_rfc3339()),
                    probe_ok: Some(health.connected && security_error.is_none()),
                    error: security_error.or(health.error),
                    uptime_secs: None,
                    bot_name: health.account_name.or(health.phone),
                })
            }
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
        ha_core::channel::traits::default_check_access(
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
                health.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }
        health.validate_security()?;

        Ok(health
            .account_name
            .or(health.phone)
            .unwrap_or_else(|| "WhatsApp".to_string()))
    }
}

fn sanitized_bridge_url_for_log(base_url: &str) -> String {
    let Ok(mut url) = url::Url::parse(base_url) else {
        return "<invalid-url>".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitized_bridge_url_for_log;

    #[test]
    fn bridge_url_log_context_drops_credentials_and_query() {
        let sanitized = sanitized_bridge_url_for_log(
            "https://user:secret@example.com/bridge?token=hidden#fragment",
        );
        assert_eq!(sanitized, "https://example.com/bridge");
    }
}

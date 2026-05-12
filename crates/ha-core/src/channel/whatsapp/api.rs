use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// WhatsApp bridge API client.
///
/// Communicates with an external bridge HTTP service (user-deployed)
/// that relays messages between WhatsApp and this plugin.
/// Follows the same bridge-polling pattern as the WeChat plugin.
#[derive(Clone)]
pub struct WhatsAppApi {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl WhatsAppApi {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            token,
        }
    }

    /// GET /api/health — check bridge connectivity and account info.
    pub async fn health(&self) -> Result<HealthResponse> {
        let raw = self.get("api/health", 10_000).await?;
        serde_json::from_str(&raw).context("Failed to decode WhatsApp bridge health response")
    }

    /// GET /api/messages?since=<timestamp> — poll for new messages.
    pub async fn poll_messages(&self, since: i64) -> Result<Vec<BridgeMessage>> {
        let endpoint = format!("api/messages?since={}", since);
        let raw = self.get(&endpoint, 35_000).await?;
        let resp: PollResponse =
            serde_json::from_str(&raw).context("Failed to decode WhatsApp poll response")?;
        Ok(resp.messages)
    }

    /// POST /api/send — send a text message.
    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<SendResponse> {
        let mut body = json!({
            "chatId": chat_id,
            "text": text,
        });
        if let Some(reply_id) = reply_to {
            body["replyTo"] = json!(reply_id);
        }
        let raw = self.post("api/send", body, 15_000).await?;
        serde_json::from_str(&raw).context("Failed to decode WhatsApp send response")
    }

    /// POST /api/typing — send a typing indicator.
    pub async fn send_typing(&self, chat_id: &str) -> Result<()> {
        self.post("api/typing", json!({ "chatId": chat_id }), 10_000)
            .await?;
        Ok(())
    }

    /// POST /api/media — send a media attachment.
    pub async fn send_media(
        &self,
        chat_id: &str,
        media_type: &str,
        data: &str,
        caption: Option<&str>,
    ) -> Result<SendResponse> {
        let mut body = json!({
            "chatId": chat_id,
            "mediaType": media_type,
            "data": data,
        });
        if let Some(cap) = caption {
            body["caption"] = json!(cap);
        }
        let raw = self.post("api/media", body, 30_000).await?;
        serde_json::from_str(&raw).context("Failed to decode WhatsApp media response")
    }

    /// Download a bridge-provided inbound attachment to `dest`. The URL
    /// can be either a bridge-side signed link (no auth) or a WhatsApp
    /// Cloud API `media_url` that needs the app access token, which the
    /// bridge surfaces via `BridgeAttachment.authBearer`. We don't pin
    /// the host because user-deployed bridges legitimately publish on
    /// arbitrary hostnames; SSRF policy still rejects metadata / private
    /// / loopback addresses by default.
    pub async fn download_attachment_to_disk(
        &self,
        url: &str,
        auth_bearer: Option<&str>,
        dest: &std::path::Path,
        cap_bytes: u64,
    ) -> Result<u64> {
        crate::security::ssrf::check_url(url, crate::security::ssrf::SsrfPolicy::Default, &[])
            .await
            .with_context(|| format!("WhatsApp attachment URL blocked: {}", url))?;

        let mut builder = self.client.get(url);
        if let Some(token) = auth_bearer {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                // Accept either a raw token or one already prefixed with
                // "Bearer " — bridges sometimes pass through verbatim.
                let header_value =
                    if trimmed.starts_with("Bearer ") || trimmed.starts_with("bearer ") {
                        trimmed.to_string()
                    } else {
                        format!("Bearer {}", trimmed)
                    };
                builder = builder.header("Authorization", header_value);
            }
        }
        crate::channel::inbound_media_common::stream_to_disk(builder, dest, cap_bytes)
            .await
            .context("WhatsApp attachment download")
    }

    // ── Internal HTTP helpers ────────────────────────────────────

    async fn get(&self, endpoint: &str, timeout_ms: u64) -> Result<String> {
        let url = join_url(&self.base_url, endpoint)?;
        let mut request = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_millis(timeout_ms));

        if let Some(ref token) = self.token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                request = request.header("Authorization", format!("Bearer {}", trimmed));
            }
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("WhatsApp GET request failed: {}", endpoint))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read WhatsApp GET response body")?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "WhatsApp GET {} failed with {}: {}",
                endpoint,
                status,
                crate::truncate_utf8(&body, 300)
            ));
        }

        Ok(body)
    }

    async fn post(
        &self,
        endpoint: &str,
        body: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<String> {
        let url = join_url(&self.base_url, endpoint)?;
        let mut request = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .json(&body);

        if let Some(ref token) = self.token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                request = request.header("Authorization", format!("Bearer {}", trimmed));
            }
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("WhatsApp POST request failed: {}", endpoint))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .context("Failed to read WhatsApp POST response body")?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "WhatsApp POST {} failed with {}: {}",
                endpoint,
                status,
                crate::truncate_utf8(&response_text, 300)
            ));
        }

        Ok(response_text)
    }
}

fn join_url(base_url: &str, endpoint: &str) -> Result<String> {
    let base = if base_url.ends_with('/') {
        base_url.to_string()
    } else {
        format!("{}/", base_url)
    };
    let url = url::Url::parse(&base)
        .with_context(|| format!("Invalid WhatsApp bridge base URL: {}", base_url))?
        .join(endpoint)
        .with_context(|| format!("Invalid WhatsApp bridge endpoint: {}", endpoint))?;
    Ok(url.to_string())
}

// ── Response types ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub account_name: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollResponse {
    #[serde(default)]
    pub messages: Vec<BridgeMessage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Whether the bot was mentioned in this message.
    #[serde(default)]
    pub was_mentioned: bool,
    /// WhatsApp message ID being replied to (if any).
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Chat title (for group chats).
    #[serde(default)]
    pub chat_title: Option<String>,
    /// Whether this is from the bot itself (echo).
    #[serde(default)]
    pub from_me: bool,
    /// Inbound attachments — empty if the bridge doesn't support media
    /// or the message has no media. Each entry must have a fetchable
    /// `url`; bridges that talk to WhatsApp Cloud API should resolve
    /// `media_id → media_url` on their side and pass the bearer in
    /// `authBearer` so this plugin only sees a download-ready record.
    /// Older bridges that don't emit this field still deserialize fine
    /// thanks to `#[serde(default)]`.
    #[serde(default)]
    pub attachments: Vec<BridgeAttachment>,
}

/// Inbound attachment transported through the bridge protocol. Optional
/// in the wire format — older bridges that omit the field deserialize
/// into an empty `attachments` vec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeAttachment {
    /// Fetchable URL — required. Either a public link or a bridge-side
    /// signed URL that resolves without auth.
    #[serde(default)]
    pub url: Option<String>,
    /// Coarse media kind (`image` / `video` / `audio` / `voice` /
    /// `document`). Used to bucket into [`MediaType`] when the MIME is
    /// missing or unhelpful.
    #[serde(default)]
    pub media_type: Option<String>,
    /// MIME type (preferred classifier).
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Optional Bearer token for the URL — populated when the bridge
    /// surfaces a WhatsApp Cloud API `media_url` that still needs the
    /// app's access token to download.
    #[serde(default)]
    pub auth_bearer: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResponse {
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

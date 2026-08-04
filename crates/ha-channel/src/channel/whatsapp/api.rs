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
    ///
    /// Canonical bridge contract:
    /// `{ chatId, mediaType, media, dataEncoding, filename?, mimeType?, caption?, replyTo? }`.
    /// `data` is sent as a legacy alias for older bridge prototypes that
    /// predate the explicit `media` field.
    pub async fn send_media(
        &self,
        chat_id: &str,
        media_type: &str,
        media: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        mime_type: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<SendResponse> {
        let mut body = json!({
            "chatId": chat_id,
            "mediaType": media_type,
            "media": media,
            "data": media,
            "dataEncoding": "data-url",
        });
        if let Some(cap) = caption {
            body["caption"] = json!(cap);
        }
        if let Some(name) = filename {
            body["filename"] = json!(name);
        }
        if let Some(mime) = mime_type {
            body["mimeType"] = json!(mime);
        }
        if let Some(reply_id) = reply_to {
            body["replyTo"] = json!(reply_id);
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
        ha_core::security::ssrf::check_url(url, ha_core::security::ssrf::SsrfPolicy::Default, &[])
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
                ha_core::truncate_utf8(&body, 300)
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
                ha_core::truncate_utf8(&response_text, 300)
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
    /// Canonical bridge implementation identifier (for example
    /// `baileys`, `whatsmeow`, or `cloud-api`). Older bridges may omit it.
    #[serde(
        default,
        alias = "bridgeImplementation",
        alias = "library",
        alias = "engine"
    )]
    pub implementation: Option<String>,
    /// Bridge/engine version. A Baileys bridge must expose this so the
    /// critical GHSA-qvv5-jq5g-4cgg minimum can be enforced at startup.
    #[serde(default, alias = "bridgeVersion", alias = "libraryVersion")]
    pub version: Option<String>,
    /// Optional runtime capabilities advertised by newer bridge versions.
    /// The current plugin keeps its conservative static capability set; the
    /// discovery data is diagnostic until account-scoped capabilities exist.
    #[serde(default)]
    pub capabilities: BridgeCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeCapabilities {
    #[serde(default)]
    pub supports_edit: bool,
    #[serde(default)]
    pub supports_unsend: bool,
    #[serde(default)]
    pub supports_buttons: bool,
    #[serde(default)]
    pub stable_user_ids: bool,
}

impl HealthResponse {
    /// Reject known-vulnerable Baileys bridges before they can ingest or emit
    /// messages. Unknown bridge implementations remain backward compatible;
    /// once a bridge identifies itself as Baileys, a parseable patched version
    /// is mandatory rather than silently assuming safety.
    pub fn validate_security(&self) -> Result<()> {
        let Some(implementation) = self
            .implementation
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let normalized = implementation.to_ascii_lowercase();
        if !normalized.contains("baileys") && !normalized.contains("whiskeysockets") {
            return Ok(());
        }

        let version = self
            .version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "WhatsApp Baileys bridge did not report its version; require >=6.7.22 or >=7.0.0-rc12"
                )
            })?;
        if baileys_version_is_patched(version) {
            return Ok(());
        }

        anyhow::bail!(
            "WhatsApp Baileys bridge version '{}' is unsupported or vulnerable; require >=6.7.22 or >=7.0.0-rc12",
            ha_core::truncate_utf8(version, 80)
        )
    }

    pub fn capability_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.capabilities.supports_edit {
            names.push("edit");
        }
        if self.capabilities.supports_unsend {
            names.push("unsend");
        }
        if self.capabilities.supports_buttons {
            names.push("buttons");
        }
        if self.capabilities.stable_user_ids {
            names.push("stable-user-ids");
        }
        names
    }
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
    pub success: Option<bool>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

impl SendResponse {
    pub fn delivery_error(&self) -> Option<String> {
        if let Some(error) = self
            .error
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(error.to_string());
        }
        if self.success == Some(false) {
            return Some("WhatsApp bridge reported success=false".to_string());
        }
        None
    }
}

fn baileys_version_is_patched(raw: &str) -> bool {
    let mut version = raw.trim();
    if let Some((_, suffix)) = version.rsplit_once('@') {
        if !suffix.is_empty() {
            version = suffix;
        }
    }
    version = version.trim_start_matches(['v', 'V', '=']);
    let version = version.split('+').next().unwrap_or(version);
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(core, pre)| (core, Some(pre)));
    let mut numbers = core.split('.');
    let Some(major) = numbers.next().and_then(|v| v.parse::<u64>().ok()) else {
        return false;
    };
    let Some(minor) = numbers.next().and_then(|v| v.parse::<u64>().ok()) else {
        return false;
    };
    let Some(patch) = numbers.next().and_then(|v| v.parse::<u64>().ok()) else {
        return false;
    };
    if numbers.next().is_some() {
        return false;
    }

    match major {
        0..=5 => false,
        6 => minor > 7 || (minor == 7 && (patch > 22 || (patch == 22 && prerelease.is_none()))),
        7 => {
            if minor > 0 || patch > 0 || prerelease.is_none() {
                return true;
            }
            let Some(pre) = prerelease else {
                return true;
            };
            let lower = pre.to_ascii_lowercase();
            let Some(rest) = lower.strip_prefix("rc") else {
                return false;
            };
            rest.trim_start_matches(['.', '-'])
                .parse::<u64>()
                .map(|rc| rc >= 12)
                .unwrap_or(false)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{baileys_version_is_patched, HealthResponse, SendResponse};

    #[test]
    fn baileys_security_floor_matches_advisory() {
        for version in ["6.7.21", "6.7.22-beta.1", "7.0.0-rc11", "n/a"] {
            assert!(!baileys_version_is_patched(version), "{version}");
        }
        for version in [
            "6.7.22",
            "6.8.0",
            "7.0.0-rc12",
            "v7.0.0-rc.14",
            "7.0.0",
            "@whiskeysockets/baileys@7.1.0",
        ] {
            assert!(baileys_version_is_patched(version), "{version}");
        }
    }

    #[test]
    fn identified_baileys_bridge_must_report_a_patched_version() {
        let missing = HealthResponse {
            implementation: Some("baileys".to_string()),
            ..Default::default()
        };
        assert!(missing.validate_security().is_err());

        let patched = HealthResponse {
            implementation: Some("WhiskeySockets/Baileys".to_string()),
            version: Some("7.0.0-rc14".to_string()),
            ..Default::default()
        };
        patched.validate_security().unwrap();
    }

    #[test]
    fn explicit_send_failure_is_not_delivery_success() {
        let response: SendResponse = serde_json::from_str(r#"{"success":false}"#).unwrap();
        assert!(response.delivery_error().is_some());
    }
}

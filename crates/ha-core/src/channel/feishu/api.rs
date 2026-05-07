use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use super::auth::FeishuAuth;
use crate::channel::types::CardStreamError;

/// Hard-coded element id used inside every streaming card we create. Lives
/// here (not in mod.rs) because both `create_streaming_card` and the
/// element-update / settings endpoints must agree on it.
pub const STREAMING_ELEMENT_ID: &str = "streaming_text";

/// Feishu bot info returned by the bot/v3/info endpoint.
#[derive(Debug, Clone)]
pub struct BotInfo {
    pub app_name: String,
    pub open_id: String,
}

/// Resolved long-connection endpoint: URL plus negotiated client params.
#[derive(Debug, Clone)]
pub struct WsEndpointInfo {
    pub url: String,
    /// Heartbeat cadence the server expects. Falls back to 120s when the
    /// `ClientConfig.PingInterval` field is missing or zero.
    pub ping_interval: Duration,
}

/// Default heartbeat used when the gateway response omits `ClientConfig` —
/// matches the documented baseline in the official SDK.
const DEFAULT_WS_PING_INTERVAL: Duration = Duration::from_secs(120);

/// Feishu REST API client.
///
/// All requests use `Authorization: Bearer {tenant_access_token}` header.
/// Responses follow the `{code: 0, msg: "ok", data: {...}}` envelope.
pub struct FeishuApi {
    client: reqwest::Client,
    auth: Arc<FeishuAuth>,
    base_url: String,
}

// ── API response envelope types ─────────────────────────────────

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    code: i64,
    msg: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct BotInfoResponse {
    code: i64,
    #[allow(dead_code)]
    msg: String,
    bot: Option<BotInfoData>,
}

#[derive(Debug, Deserialize)]
struct BotInfoData {
    app_name: String,
    open_id: String,
}

#[derive(Debug, Deserialize)]
struct SendMessageData {
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct CreateCardData {
    card_id: String,
}

#[derive(Debug, Deserialize)]
struct ImageUploadData {
    image_key: String,
}

#[derive(Debug, Deserialize)]
struct FileUploadData {
    file_key: String,
}

#[derive(Debug, Default, Deserialize)]
struct WsEndpointData {
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "ClientConfig", default)]
    client_config: Option<WsClientConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct WsClientConfig {
    /// Server-suggested heartbeat in seconds.
    #[serde(rename = "PingInterval", default)]
    ping_interval: Option<u64>,
}

impl FeishuApi {
    /// Create a new API client with the given auth manager.
    pub fn new(auth: Arc<FeishuAuth>) -> Self {
        let base_url = auth.base_url().to_string();
        Self {
            client: reqwest::Client::new(),
            auth,
            base_url,
        }
    }

    /// Get an authorized request builder with the current access token.
    async fn authorized_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<reqwest::RequestBuilder> {
        let token = self.auth.get_token().await?;
        Ok(self
            .client
            .request(method, url)
            .header("Authorization", format!("Bearer {}", token)))
    }

    /// Read `resp`, validate the standard `{code, msg, data}` envelope, and
    /// return the decoded `data` field. `label` only appears in error
    /// messages so callers can disambiguate ("card create" vs "delete
    /// message"). Returns `Ok(None)` when the response carries no `data`
    /// (some endpoints like update / delete legitimately omit it on success).
    async fn parse_envelope<T>(&self, resp: reqwest::Response, label: &str) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu {} response: {}", label, e))?;
        if !status.is_success() {
            return Err(anyhow!(
                "Feishu {} failed with HTTP {}: {}",
                label,
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }
        let parsed: ApiResponse<T> = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu {} response: {}", label, e))?;
        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu {} error (code={}): {}",
                label,
                parsed.code,
                parsed.msg
            ));
        }
        Ok(parsed.data)
    }

    /// Get bot info (app_name, open_id).
    pub async fn get_bot_info(&self) -> Result<BotInfo> {
        let url = format!("{}/open-apis/bot/v3/info/", self.base_url);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to call Feishu bot info: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu bot info response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu bot info failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        let parsed: BotInfoResponse = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu bot info: {}", e))?;

        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu bot info error (code={}): {}",
                parsed.code,
                parsed.msg
            ));
        }

        let bot = parsed
            .bot
            .ok_or_else(|| anyhow!("Feishu bot info response missing 'bot' field"))?;

        Ok(BotInfo {
            app_name: bot.app_name,
            open_id: bot.open_id,
        })
    }

    /// Send a text message to a chat.
    ///
    /// If `reply_to` is Some, sends as a reply to the specified message.
    /// Returns the message_id of the sent message.
    pub async fn send_message(
        &self,
        receive_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let content = serde_json::json!({ "text": text }).to_string();

        if let Some(reply_msg_id) = reply_to {
            // Reply to a specific message
            let url = format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                self.base_url, reply_msg_id
            );
            let body = serde_json::json!({
                "msg_type": "text",
                "content": content,
            });

            let resp = self
                .authorized_request(reqwest::Method::POST, &url)
                .await?
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("Failed to send Feishu reply: {}", e))?;

            return self.parse_send_response(resp).await;
        }

        // Send a new message
        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "text",
            "content": content,
        });

        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send Feishu message: {}", e))?;

        self.parse_send_response(resp).await
    }

    /// Send an interactive card message with action buttons.
    ///
    /// If `reply_to` is Some, sends as a reply to the specified message.
    /// Returns the message_id of the sent message.
    pub async fn send_interactive_card(
        &self,
        receive_id: &str,
        card_json: serde_json::Value,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let content = card_json.to_string();

        if let Some(reply_msg_id) = reply_to {
            // Reply to a specific message with an interactive card
            let url = format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                self.base_url, reply_msg_id
            );
            let body = serde_json::json!({
                "msg_type": "interactive",
                "content": content,
            });

            let resp = self
                .authorized_request(reqwest::Method::POST, &url)
                .await?
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("Failed to send Feishu interactive card reply: {}", e))?;

            return self.parse_send_response(resp).await;
        }

        // Send a new interactive card message
        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": content,
        });

        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send Feishu interactive card: {}", e))?;

        self.parse_send_response(resp).await
    }

    /// POST /open-apis/im/v1/images — multipart upload, returns `image_key`.
    /// `image_type` is typically `"message"` for IM-message-bound images.
    pub async fn upload_image(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        mime: &str,
        image_type: &str,
    ) -> Result<String> {
        let url = format!("{}/open-apis/im/v1/images", self.base_url);
        let form = reqwest::multipart::Form::new()
            .text("image_type", image_type.to_string())
            .part("image", build_part(bytes, filename, mime, "image")?);
        let data: ImageUploadData = self.upload_multipart(&url, form, "image").await?;
        Ok(data.image_key)
    }

    /// POST /open-apis/im/v1/files — multipart upload, returns `file_key`.
    /// `file_type` ∈ `{opus, mp4, pdf, doc, xls, ppt, stream}`.
    pub async fn upload_file(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        mime: &str,
        file_type: &str,
    ) -> Result<String> {
        let url = format!("{}/open-apis/im/v1/files", self.base_url);
        let form = reqwest::multipart::Form::new()
            .text("file_type", file_type.to_string())
            .text("file_name", filename.to_string())
            .part("file", build_part(bytes, filename, mime, "file")?);
        let data: FileUploadData = self.upload_multipart(&url, form, "file").await?;
        Ok(data.file_key)
    }

    /// Generic multipart POST: send `form`, decode `{code, msg, data}`, return `data`.
    /// `label` only appears in error messages to disambiguate image vs file uploads.
    async fn upload_multipart<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: reqwest::multipart::Form,
        label: &str,
    ) -> Result<T> {
        let resp = self
            .authorized_request(reqwest::Method::POST, url)
            .await?
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to upload Feishu {}: {}", label, e))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu {} upload response: {}", label, e))?;
        if !status.is_success() {
            return Err(anyhow!(
                "Feishu {} upload failed with HTTP {}: {}",
                label,
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }
        let parsed: ApiResponse<T> = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu {} upload response: {}", label, e))?;
        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu {} upload error (code={}): {}",
                label,
                parsed.code,
                parsed.msg
            ));
        }
        parsed
            .data
            .ok_or_else(|| anyhow!("Feishu {} upload response missing 'data' field", label))
    }

    /// Send `msg_type=image` referencing a previously uploaded `image_key`.
    pub async fn send_image_message(
        &self,
        receive_id: &str,
        image_key: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let content = serde_json::json!({ "image_key": image_key }).to_string();
        self.send_typed_message(receive_id, "image", &content, reply_to)
            .await
    }

    /// Send `msg_type=file` referencing a previously uploaded `file_key`.
    pub async fn send_file_message(
        &self,
        receive_id: &str,
        file_key: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        self.send_typed_message(receive_id, "file", &content, reply_to)
            .await
    }

    /// Shared helper: send any `msg_type` with pre-built `content` JSON string.
    async fn send_typed_message(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        if let Some(reply_msg_id) = reply_to {
            let url = format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                self.base_url, reply_msg_id
            );
            let body = serde_json::json!({
                "msg_type": msg_type,
                "content": content,
            });
            let resp = self
                .authorized_request(reqwest::Method::POST, &url)
                .await?
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("Failed to send Feishu {} reply: {}", msg_type, e))?;
            return self.parse_send_response(resp).await;
        }

        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": msg_type,
            "content": content,
        });
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send Feishu {}: {}", msg_type, e))?;
        self.parse_send_response(resp).await
    }

    /// Update an existing message.
    pub async fn update_message(&self, message_id: &str, text: &str) -> Result<()> {
        let url = format!("{}/open-apis/im/v1/messages/{}", self.base_url, message_id);
        let content = serde_json::json!({ "text": text }).to_string();
        let body = serde_json::json!({
            "msg_type": "text",
            "content": content,
        });

        let resp = self
            .authorized_request(reqwest::Method::PUT, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to update Feishu message: {}", e))?;

        let _: Option<serde_json::Value> = self.parse_envelope(resp, "update message").await?;
        Ok(())
    }

    /// Delete an existing message.
    pub async fn delete_message(&self, message_id: &str) -> Result<()> {
        let url = format!("{}/open-apis/im/v1/messages/{}", self.base_url, message_id);

        let resp = self
            .authorized_request(reqwest::Method::DELETE, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to delete Feishu message: {}", e))?;

        let _: Option<serde_json::Value> = self.parse_envelope(resp, "delete message").await?;
        Ok(())
    }

    /// Get a WebSocket endpoint and the negotiated client params.
    ///
    /// POST `/callback/ws/endpoint` with `{AppID, AppSecret}` body → returns
    /// `{data: {URL, ClientConfig: {PingInterval, ...}}}`. The handshake is
    /// unauthenticated (no tenant_access_token); credentials are passed inline
    /// in the body. `PingInterval` is honored if present; otherwise the
    /// 120-second default is used.
    pub async fn get_ws_endpoint(&self) -> Result<WsEndpointInfo> {
        let url = format!("{}/callback/ws/endpoint", self.base_url);

        let resp = self
            .client
            .post(&url)
            .header("locale", "zh")
            .json(&self.auth.ws_endpoint_credentials())
            .send()
            .await
            .map_err(|e| anyhow!("Failed to get Feishu WS endpoint: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu WS endpoint response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu WS endpoint request failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        let parsed: ApiResponse<WsEndpointData> = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu WS endpoint response: {}", e))?;

        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu WS endpoint error (code={}): {}",
                parsed.code,
                parsed.msg
            ));
        }

        let data = parsed
            .data
            .ok_or_else(|| anyhow!("Feishu WS endpoint response missing 'data' field"))?;

        let url = data
            .url
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow!("Feishu WS endpoint response missing 'URL' field"))?;

        let ping_interval = data
            .client_config
            .as_ref()
            .and_then(|c| c.ping_interval)
            .filter(|n| *n > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_WS_PING_INTERVAL);

        Ok(WsEndpointInfo { url, ping_interval })
    }

    /// Build the schema 2.0 card body used by `create_streaming_card`.
    /// Pulled out to a free helper so the JSON shape can be unit-tested
    /// without an HTTP client.
    pub(crate) fn build_streaming_card_body(initial_text: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "2.0",
            "config": {
                "streaming_mode": true,
                "summary": {"content": ""}
            },
            "body": {
                "elements": [
                    {
                        "tag": "markdown",
                        "element_id": STREAMING_ELEMENT_ID,
                        "content": initial_text
                    }
                ]
            }
        })
    }

    /// Create a streaming card on cardkit. Returns the card_id; the
    /// element_id is the constant `STREAMING_ELEMENT_ID`. Subsequent
    /// updates target that element.
    pub async fn create_streaming_card(&self, initial_text: &str) -> Result<String> {
        let url = format!("{}/open-apis/cardkit/v1/cards", self.base_url);
        let card_json = Self::build_streaming_card_body(initial_text);
        let body = serde_json::json!({
            "type": "card_json",
            "data": card_json.to_string(),
        });

        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to create Feishu streaming card: {}", e))?;

        let data: CreateCardData = self
            .parse_envelope(resp, "card create")
            .await?
            .ok_or_else(|| anyhow!("Feishu card create response missing 'data' field"))?;

        Ok(data.card_id)
    }

    /// Push a previously-created card to a chat as an interactive message
    /// referencing `card_id`. Returns the host message_id.
    pub async fn send_card_reference(
        &self,
        receive_id: &str,
        card_id: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let content_value = serde_json::json!({
            "type": "card",
            "data": {"card_id": card_id}
        });
        let content = content_value.to_string();
        let msg_type = "interactive";

        if let Some(reply_msg_id) = reply_to {
            let url = format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                self.base_url, reply_msg_id
            );
            let body = serde_json::json!({
                "msg_type": msg_type,
                "content": content,
            });
            let resp = self
                .authorized_request(reqwest::Method::POST, &url)
                .await?
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("Failed to send Feishu card reference reply: {}", e))?;
            return self.parse_send_response(resp).await;
        }

        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": msg_type,
            "content": content,
        });
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send Feishu card reference: {}", e))?;
        self.parse_send_response(resp).await
    }

    /// Append text to a streaming card element. `sequence` must be strictly
    /// increasing across all calls within one card lifetime.
    pub async fn update_card_element(
        &self,
        card_id: &str,
        element_id: &str,
        content: &str,
        sequence: i64,
    ) -> std::result::Result<(), CardStreamError> {
        let url = format!(
            "{}/open-apis/cardkit/v1/cards/{}/elements/{}/content",
            self.base_url, card_id, element_id
        );
        let body = serde_json::json!({
            "content": content,
            "sequence": sequence,
        });

        let req = self
            .authorized_request(reqwest::Method::PUT, &url)
            .await
            .map_err(|e| CardStreamError::Other(e.to_string()))?;
        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| CardStreamError::Other(format!("network: {}", e)))?;

        let status = resp.status();
        let resp_body = resp
            .text()
            .await
            .map_err(|e| CardStreamError::Other(format!("read body: {}", e)))?;

        if !status.is_success() {
            return Err(CardStreamError::Other(format!(
                "HTTP {}: {}",
                status,
                crate::truncate_utf8(&resp_body, 512)
            )));
        }

        let parsed: ApiResponse<serde_json::Value> = serde_json::from_str(&resp_body)
            .map_err(|e| CardStreamError::Other(format!("parse: {}", e)))?;

        if parsed.code != 0 {
            return Err(card_stream_error_from_code(parsed.code, &parsed.msg));
        }

        Ok(())
    }

    /// Disable streaming mode on a card via the settings endpoint.
    /// Best-effort — caller logs errors but doesn't recover.
    pub async fn close_card_streaming(&self, card_id: &str, sequence: i64) -> Result<()> {
        let url = format!(
            "{}/open-apis/cardkit/v1/cards/{}/settings",
            self.base_url, card_id
        );
        let settings_value = serde_json::json!({
            "config": {"streaming_mode": false}
        });
        let body = serde_json::json!({
            "settings": settings_value.to_string(),
            "sequence": sequence,
        });

        let resp = self
            .authorized_request(reqwest::Method::PATCH, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to close Feishu card streaming: {}", e))?;

        let _: Option<serde_json::Value> = self.parse_envelope(resp, "card close").await?;
        Ok(())
    }

    /// Parse a send/reply message response and extract the message_id.
    async fn parse_send_response(&self, resp: reqwest::Response) -> Result<String> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu send response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu send message failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        let parsed: ApiResponse<SendMessageData> = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu send response: {}", e))?;

        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu send message error (code={}): {}",
                parsed.code,
                parsed.msg
            ));
        }

        let data = parsed
            .data
            .ok_or_else(|| anyhow!("Feishu send response missing 'data' field"))?;

        Ok(data.message_id)
    }
}

fn build_part(
    bytes: Vec<u8>,
    filename: &str,
    mime: &str,
    label: &str,
) -> Result<reqwest::multipart::Part> {
    reqwest::multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str(mime)
        .map_err(|e| anyhow!("Invalid Feishu {} part mime '{}': {}", label, mime, e))
}

/// Translate a Feishu cardkit error code into the channel-agnostic
/// `CardStreamError` variants. Unknown codes fall through to `Other`
/// with the original `code= msg` formatting so logs stay greppable.
pub(crate) fn card_stream_error_from_code(code: i64, msg: &str) -> CardStreamError {
    match code {
        300317 => CardStreamError::SequenceOutOfOrder,
        200750 => CardStreamError::Expired,
        200850 => CardStreamError::TimedOut,
        300309 => CardStreamError::NotEnabled,
        300311 => CardStreamError::NoPermission,
        _ => CardStreamError::Other(format!("code={} {}", code, msg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_card_body_has_required_shape() {
        let body = FeishuApi::build_streaming_card_body("hello");
        assert_eq!(body["schema"], "2.0");
        assert_eq!(body["config"]["streaming_mode"], true);
        let elements = body["body"]["elements"].as_array().expect("elements");
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0]["tag"], "markdown");
        assert_eq!(elements[0]["element_id"], STREAMING_ELEMENT_ID);
        assert_eq!(elements[0]["content"], "hello");
    }

    #[test]
    fn card_stream_error_classifies_known_codes() {
        assert!(matches!(
            card_stream_error_from_code(300317, "out of order"),
            CardStreamError::SequenceOutOfOrder
        ));
        assert!(matches!(
            card_stream_error_from_code(200750, "expired"),
            CardStreamError::Expired
        ));
        assert!(matches!(
            card_stream_error_from_code(200850, "timed out"),
            CardStreamError::TimedOut
        ));
        assert!(matches!(
            card_stream_error_from_code(300309, "not enabled"),
            CardStreamError::NotEnabled
        ));
        assert!(matches!(
            card_stream_error_from_code(300311, "no permission"),
            CardStreamError::NoPermission
        ));
        assert!(matches!(
            card_stream_error_from_code(99999, "unknown"),
            CardStreamError::Other(_)
        ));
    }
}

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use super::auth::FeishuAuth;

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

        let status = resp.status();
        let resp_body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu update response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu update message failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&resp_body, 512)
            ));
        }

        let parsed: ApiResponse<serde_json::Value> = serde_json::from_str(&resp_body)
            .map_err(|e| anyhow!("Failed to parse Feishu update response: {}", e))?;

        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu update message error (code={}): {}",
                parsed.code,
                parsed.msg
            ));
        }

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

        let status = resp.status();
        let resp_body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu delete response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu delete message failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&resp_body, 512)
            ));
        }

        let parsed: ApiResponse<serde_json::Value> = serde_json::from_str(&resp_body)
            .map_err(|e| anyhow!("Failed to parse Feishu delete response: {}", e))?;

        if parsed.code != 0 {
            return Err(anyhow!(
                "Feishu delete message error (code={}): {}",
                parsed.code,
                parsed.msg
            ));
        }

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

        Ok(WsEndpointInfo {
            url,
            ping_interval,
        })
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

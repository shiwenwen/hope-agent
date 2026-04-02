use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::Arc;

use super::auth::FeishuAuth;

/// Feishu bot info returned by the bot/v3/info endpoint.
#[derive(Debug, Clone)]
pub struct BotInfo {
    pub app_name: String,
    pub open_id: String,
}

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

#[derive(Debug, Default, Deserialize)]
struct WsEndpointData {
    #[serde(rename = "URL")]
    url: Option<String>,
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

    /// Update an existing message.
    pub async fn update_message(&self, message_id: &str, text: &str) -> Result<()> {
        let url = format!(
            "{}/open-apis/im/v1/messages/{}",
            self.base_url, message_id
        );
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
        let url = format!(
            "{}/open-apis/im/v1/messages/{}",
            self.base_url, message_id
        );

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

    /// Get a WebSocket endpoint URL for the long-connection event subscription.
    ///
    /// POST `/open-apis/callback/ws/endpoint` → returns `{url: "wss://..."}`.
    pub async fn get_ws_endpoint(&self) -> Result<String> {
        let url = format!("{}/open-apis/callback/ws/endpoint", self.base_url);

        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&serde_json::json!({}))
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

        data.url
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow!("Feishu WS endpoint response missing 'URL' field"))
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

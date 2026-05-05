use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::time::Duration;

use super::auth::{format_auth_value, QqBotAuth};

/// QQ Bot REST API client.
///
/// Auth scheme is documented on [`super::auth::AUTH_SCHEME`]; also sends
/// `X-Union-Appid: {app_id}` header.
pub struct QqBotApi {
    client: reqwest::Client,
    pub auth: Arc<QqBotAuth>,
    base_url: String,
}

/// Response from GET /gateway.
#[derive(Debug, serde::Deserialize)]
struct GatewayResponse {
    url: String,
}

impl QqBotApi {
    /// Create a new QQ Bot API client.
    pub fn new(auth: Arc<QqBotAuth>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            auth,
            base_url: "https://api.sgroup.qq.com".to_string(),
        }
    }

    /// Make a request to the QQ Bot API with automatic auth.
    async fn qq_request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let token = self.auth.get_token().await?;
        let url = format!("{}{}", self.base_url, path);

        let mut req = self
            .client
            .request(method.clone(), &url)
            .header("Authorization", format_auth_value(&token))
            .header("X-Union-Appid", self.auth.app_id())
            .header("Content-Type", "application/json");

        if let Some(body) = body {
            req = req.json(&body);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("QQ Bot API request failed for {} {}: {}", method, path, e))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(anyhow!(
                "QQ Bot API {} {} returned HTTP {}: {}",
                method,
                path,
                status,
                crate::truncate_utf8(&body_text, 500)
            ));
        }

        serde_json::from_str(&body_text).map_err(|e| {
            anyhow!(
                "Failed to parse QQ Bot API response for {} {}: {}",
                method,
                path,
                e
            )
        })
    }

    /// Get the WebSocket gateway URL.
    ///
    /// GET /gateway -> { "url": "wss://..." }
    pub async fn get_gateway_url(&self) -> Result<String> {
        let resp: GatewayResponse = self
            .qq_request(reqwest::Method::GET, "/gateway", None)
            .await?;
        Ok(resp.url)
    }

    /// Send a message to a group.
    ///
    /// POST /v2/groups/{group_openid}/messages
    pub async fn send_group_message(
        &self,
        group_openid: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/v2/groups/{}/messages", group_openid);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 0,
        });
        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
        }
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a message to a C2C (private) user.
    ///
    /// POST /v2/users/{openid}/messages
    pub async fn send_c2c_message(
        &self,
        openid: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/v2/users/{}/messages", openid);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 0,
        });
        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
        }
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a message to a guild channel.
    ///
    /// POST /channels/{channel_id}/messages
    pub async fn send_channel_message(
        &self,
        channel_id: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/channels/{}/messages", channel_id);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 0,
        });
        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
        }
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a direct message in a guild context.
    ///
    /// POST /dms/{guild_id}/messages
    pub async fn send_dms_message(
        &self,
        guild_id: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/dms/{}/messages", guild_id);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 0,
        });
        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
        }
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a message with inline keyboard buttons (msg_type 2 = markdown with keyboard).
    ///
    /// For QQ Bot, buttons are sent as a keyboard component alongside markdown content.
    /// Only supported for C2C and group messages (not guild channels/DMs).
    pub async fn send_message_with_keyboard(
        &self,
        chat_id: &str,
        content: &str,
        keyboard: serde_json::Value,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let (path, mut body) = if let Some(openid) = chat_id.strip_prefix("c2c:") {
            (
                format!("/v2/users/{}/messages", openid),
                serde_json::json!({
                    "content": content,
                    "msg_type": 2,
                    "keyboard": keyboard,
                }),
            )
        } else if let Some(group_openid) = chat_id.strip_prefix("group:") {
            (
                format!("/v2/groups/{}/messages", group_openid),
                serde_json::json!({
                    "content": content,
                    "msg_type": 2,
                    "keyboard": keyboard,
                }),
            )
        } else {
            return Err(anyhow!(
                "Keyboard buttons not supported for this QQ Bot chat type: {}",
                crate::truncate_utf8(chat_id, 100)
            ));
        };

        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
        }

        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a typing indicator for C2C (private) messages.
    ///
    /// POST /v2/users/{openid}/input_notify
    pub async fn send_typing_c2c(&self, openid: &str) -> Result<()> {
        let path = format!("/v2/users/{}/input_notify", openid);
        let _: serde_json::Value = self
            .qq_request(reqwest::Method::POST, &path, Some(serde_json::json!({})))
            .await?;
        Ok(())
    }
}

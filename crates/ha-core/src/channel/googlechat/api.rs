use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::Duration;

use super::auth::GoogleChatAuth;

const CHAT_API_BASE: &str = "https://chat.googleapis.com/v1";

/// Google Chat REST API client.
pub struct GoogleChatApi {
    auth: Arc<GoogleChatAuth>,
    client: reqwest::Client,
}

impl GoogleChatApi {
    /// Create a new API client with the given authenticator.
    pub fn new(auth: Arc<GoogleChatAuth>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { auth, client }
    }

    /// Get an authorization header value.
    async fn auth_header(&self) -> Result<String> {
        let token = self.auth.get_access_token().await?;
        Ok(format!("Bearer {}", token))
    }

    /// Parse an error response into an anyhow error.
    async fn parse_error(resp: reqwest::Response, context: &str) -> anyhow::Error {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        anyhow!(
            "Google Chat API error {} ({}): {}",
            status,
            context,
            crate::truncate_utf8(&body, 512)
        )
    }

    // ── Spaces ─────────────────────────────────────────────────────

    /// GET /spaces — list spaces the bot has access to.
    /// Used for credential validation (probe).
    pub async fn list_spaces(&self) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let url = format!("{}?pageSize=1", CHAT_API_BASE.to_string() + "/spaces");

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| anyhow!("list_spaces request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp, "list_spaces").await);
        }

        resp.json()
            .await
            .map_err(|e| anyhow!("list_spaces parse failed: {}", e))
    }

    // ── Messages ───────────────────────────────────────────────────

    /// POST /spaces/{space}/messages — send a text message.
    ///
    /// If `thread_key` is provided, the message is sent as a reply in that thread.
    /// If `cards_v2` is provided, interactive card widgets (e.g. approval buttons)
    /// are attached to the message.
    pub async fn send_message(
        &self,
        space: &str,
        text: &str,
        thread_key: Option<&str>,
        cards_v2: Option<&[serde_json::Value]>,
    ) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;

        let mut body = serde_json::json!({ "text": text });

        // Google Chat API 区分两种 thread 引用：
        // - `thread.name` —— 已存在线程的 resource name `spaces/{}/threads/{}`
        // - `thread.threadKey` —— 开发者自定义 key（用于幂等创建/匹配）
        // 之前一律用 `name` 会让自定义 thread_key（如 cron 自动消息生成的
        // 任意字符串）触发 INVALID_ARGUMENT。这里按 thread_key 形态自动选用。
        let mut reply_option_param = "";
        if let Some(tk) = thread_key {
            if tk.starts_with("spaces/") && tk.contains("/threads/") {
                body["thread"] = serde_json::json!({ "name": tk });
                reply_option_param = "?messageReplyOption=REPLY_MESSAGE_OR_FAIL";
            } else {
                body["thread"] = serde_json::json!({ "threadKey": tk });
                reply_option_param = "?messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD";
            }
        }

        if let Some(cards) = cards_v2 {
            body["cardsV2"] = serde_json::Value::Array(cards.to_vec());
        }

        let url = format!("{}/{}/messages{}", CHAT_API_BASE, space, reply_option_param);

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("send_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp, "send_message").await);
        }

        resp.json()
            .await
            .map_err(|e| anyhow!("send_message parse failed: {}", e))
    }

    // ── Update Message ─────────────────────────────────────────────

    /// PATCH /{message_name}?updateMask=text — update a message's text.
    pub async fn update_message(
        &self,
        message_name: &str,
        text: &str,
    ) -> Result<serde_json::Value> {
        let auth = self.auth_header().await?;
        let url = format!("{}/{}?updateMask=text", CHAT_API_BASE, message_name);

        let body = serde_json::json!({ "text": text });

        let resp = self
            .client
            .patch(&url)
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("update_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp, "update_message").await);
        }

        resp.json()
            .await
            .map_err(|e| anyhow!("update_message parse failed: {}", e))
    }

    // ── Delete Message ─────────────────────────────────────────────

    /// DELETE /{message_name} — delete a message.
    pub async fn delete_message(&self, message_name: &str) -> Result<()> {
        let auth = self.auth_header().await?;
        let url = format!("{}/{}", CHAT_API_BASE, message_name);

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(|e| anyhow!("delete_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp, "delete_message").await);
        }

        Ok(())
    }
}

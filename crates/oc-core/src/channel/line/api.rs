use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

/// LINE Messaging API client.
pub struct LineApi {
    client: Client,
    base_url: String,
    channel_access_token: String,
}

impl LineApi {
    /// Create a new LINE API client.
    pub fn new(channel_access_token: &str, proxy: Option<&str>) -> Self {
        let mut builder = Client::builder();
        if let Some(proxy_url) = proxy {
            if let Ok(p) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(p);
            }
        }
        let client = builder.build().unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url: "https://api.line.me".to_string(),
            channel_access_token: channel_access_token.to_string(),
        }
    }

    /// GET /v2/bot/info - Get bot info (display name, userId, etc.)
    pub async fn get_bot_info(&self) -> Result<Value> {
        let url = format!("{}/v2/bot/info", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.channel_access_token)
            .send()
            .await
            .context("Failed to send GET /v2/bot/info")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("Failed to read bot info response")?;

        if !status.is_success() {
            anyhow::bail!(
                "LINE API GET /v2/bot/info returned {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse bot info JSON")
    }

    /// POST /v2/bot/message/reply - Reply to a message using a reply token.
    ///
    /// The reply token is valid for only ~1 minute after the webhook event.
    pub async fn reply_message(&self, reply_token: &str, messages: Vec<Value>) -> Result<()> {
        let url = format!("{}/v2/bot/message/reply", self.base_url);
        let body = serde_json::json!({
            "replyToken": reply_token,
            "messages": messages,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.channel_access_token)
            .json(&body)
            .send()
            .await
            .context("Failed to send POST /v2/bot/message/reply")?;

        let status = resp.status();
        if !status.is_success() {
            let resp_body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "LINE API POST /v2/bot/message/reply returned {}: {}",
                status,
                crate::truncate_utf8(&resp_body, 512)
            );
        }

        Ok(())
    }

    /// POST /v2/bot/message/push - Push a message to a user, group, or room.
    pub async fn push_message(&self, to: &str, messages: Vec<Value>) -> Result<()> {
        let url = format!("{}/v2/bot/message/push", self.base_url);
        let body = serde_json::json!({
            "to": to,
            "messages": messages,
        });

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.channel_access_token)
            .json(&body)
            .send()
            .await
            .context("Failed to send POST /v2/bot/message/push")?;

        let status = resp.status();
        if !status.is_success() {
            let resp_body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "LINE API POST /v2/bot/message/push returned {}: {}",
                status,
                crate::truncate_utf8(&resp_body, 512)
            );
        }

        Ok(())
    }

    /// GET /v2/bot/profile/{userId} - Get user profile.
    pub async fn get_profile(&self, user_id: &str) -> Result<Value> {
        let url = format!("{}/v2/bot/profile/{}", self.base_url, user_id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.channel_access_token)
            .send()
            .await
            .context("Failed to send GET /v2/bot/profile")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("Failed to read profile response")?;

        if !status.is_success() {
            anyhow::bail!(
                "LINE API GET /v2/bot/profile returned {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse profile JSON")
    }

    /// GET /v2/bot/group/{groupId}/member/{userId}/profile - Get group member profile.
    pub async fn get_group_member_profile(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<Value> {
        let url = format!(
            "{}/v2/bot/group/{}/member/{}/profile",
            self.base_url, group_id, user_id
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.channel_access_token)
            .send()
            .await
            .context("Failed to send GET group member profile")?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .context("Failed to read group member profile response")?;

        if !status.is_success() {
            anyhow::bail!(
                "LINE API GET group member profile returned {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse group member profile JSON")
    }
}

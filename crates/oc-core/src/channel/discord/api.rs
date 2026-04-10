use anyhow::{anyhow, Result};
use std::time::Duration;

/// Discord REST API client (v10).
pub struct DiscordApi {
    client: reqwest::Client,
    /// Already prefixed with "Bot ".
    token: String,
    base_url: String,
}

impl DiscordApi {
    /// Create a new Discord API client.
    ///
    /// `token` is the raw bot token — "Bot " is prepended internally.
    pub fn new(token: &str, proxy: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));

        if let Some(proxy_url) = proxy {
            if let Ok(p) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(p);
            }
        }

        let client = builder.build().unwrap_or_else(|_| reqwest::Client::new());
        let auth = format!("Bot {}", token.trim());

        Self {
            client,
            token: auth,
            base_url: "https://discord.com/api/v10".to_string(),
        }
    }

    /// Return the raw token (with "Bot " prefix) for gateway IDENTIFY.
    pub fn token(&self) -> &str {
        &self.token
    }

    // ── Helper ──────────────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Parse a Discord error response into an anyhow error with details.
    async fn parse_error(resp: reqwest::Response) -> anyhow::Error {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow!(
            "Discord API error {}: {}",
            status.as_u16(),
            crate::truncate_utf8(&body, 512)
        )
    }

    // ── Users ───────────────────────────────────────────────────────

    /// GET /users/@me — validate the bot token and return user object.
    pub async fn get_current_user(&self) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url("/users/@me"))
            .header("Authorization", &self.token)
            .send()
            .await
            .map_err(|e| anyhow!("get_current_user request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("get_current_user parse failed: {}", e))
    }

    // ── Gateway ─────────────────────────────────────────────────────

    /// GET /gateway/bot — get the WebSocket gateway URL and shard info.
    pub async fn get_gateway_bot(&self) -> Result<serde_json::Value> {
        let resp = self
            .client
            .get(self.url("/gateway/bot"))
            .header("Authorization", &self.token)
            .send()
            .await
            .map_err(|e| anyhow!("get_gateway_bot request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("get_gateway_bot parse failed: {}", e))
    }

    // ── Messages ────────────────────────────────────────────────────

    /// POST /channels/{channel_id}/messages — send a text message with optional components.
    pub async fn create_message(
        &self,
        channel_id: &str,
        content: &str,
        reply_to: Option<&str>,
        thread_id: Option<&str>,
        components: Option<&[serde_json::Value]>,
    ) -> Result<serde_json::Value> {
        // Build the target URL — if thread_id is provided, the message goes into that thread
        let url = match thread_id {
            Some(tid) => self.url(&format!("/channels/{}/messages", tid)),
            None => self.url(&format!("/channels/{}/messages", channel_id)),
        };

        let mut body = serde_json::json!({ "content": content });

        if let Some(ref_id) = reply_to {
            body["message_reference"] = serde_json::json!({
                "message_id": ref_id
            });
        }

        if let Some(comps) = components {
            body["components"] = serde_json::json!(comps);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("create_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("create_message parse failed: {}", e))
    }

    /// PATCH /channels/{channel_id}/messages/{message_id} — edit a message.
    pub async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<serde_json::Value> {
        let url = self.url(&format!("/channels/{}/messages/{}", channel_id, message_id));

        let body = serde_json::json!({ "content": content });

        let resp = self
            .client
            .patch(&url)
            .header("Authorization", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("edit_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("edit_message parse failed: {}", e))
    }

    /// DELETE /channels/{channel_id}/messages/{message_id} — delete a message.
    pub async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let url = self.url(&format!("/channels/{}/messages/{}", channel_id, message_id));

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", &self.token)
            .send()
            .await
            .map_err(|e| anyhow!("delete_message request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }

    // ── Interactions ─────────────────────────────────────────────────

    /// POST /interactions/{id}/{token}/callback — respond to an interaction.
    ///
    /// Common response types:
    /// - 4: CHANNEL_MESSAGE_WITH_SOURCE (send a message)
    /// - 6: DEFERRED_UPDATE_MESSAGE (ACK, edit later)
    /// - 7: UPDATE_MESSAGE (edit the original message)
    pub async fn create_interaction_response(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        response_type: u64,
        data: Option<serde_json::Value>,
    ) -> Result<()> {
        let url = format!(
            "https://discord.com/api/v10/interactions/{}/{}/callback",
            interaction_id, interaction_token
        );

        let mut body = serde_json::json!({ "type": response_type });
        if let Some(d) = data {
            body["data"] = d;
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("create_interaction_response request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }

    // ── Typing ──────────────────────────────────────────────────────

    /// POST /channels/{channel_id}/typing — trigger typing indicator.
    pub async fn trigger_typing(&self, channel_id: &str) -> Result<()> {
        let url = self.url(&format!("/channels/{}/typing", channel_id));

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &self.token)
            .header("Content-Length", "0")
            .send()
            .await
            .map_err(|e| anyhow!("trigger_typing request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }

    // ── Application Commands ────────────────────────────────────────

    /// PUT /applications/{application_id}/commands — bulk overwrite global commands.
    pub async fn bulk_overwrite_global_commands(
        &self,
        application_id: &str,
        commands: Vec<serde_json::Value>,
    ) -> Result<()> {
        let url = self.url(&format!("/applications/{}/commands", application_id));

        let resp = self
            .client
            .put(&url)
            .header("Authorization", &self.token)
            .json(&commands)
            .send()
            .await
            .map_err(|e| anyhow!("bulk_overwrite_global_commands request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }
}

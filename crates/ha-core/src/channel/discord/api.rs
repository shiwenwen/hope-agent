use anyhow::{anyhow, Result};
use std::time::Duration;

use crate::channel::rate_limit::with_rate_limit_retry;

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
        let url = self.url("/users/@me");
        let resp = with_rate_limit_retry(3, || async {
            self.client
                .get(&url)
                .header("Authorization", &self.token)
                .send()
                .await
                .map_err(|e| anyhow!("get_current_user request failed: {}", e))
        })
        .await?;

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
        let url = self.url("/gateway/bot");
        let resp = with_rate_limit_retry(3, || async {
            self.client
                .get(&url)
                .header("Authorization", &self.token)
                .send()
                .await
                .map_err(|e| anyhow!("get_gateway_bot request failed: {}", e))
        })
        .await?;

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
            // fail_if_not_exists=false: 引用消息已删除时降级为普通消息而不是整条 fail
            body["message_reference"] = serde_json::json!({
                "message_id": ref_id,
                "fail_if_not_exists": false,
            });
        }

        if let Some(comps) = components {
            body["components"] = serde_json::json!(comps);
        }

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .post(&url)
                .header("Authorization", &self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("create_message request failed: {}", e))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("create_message parse failed: {}", e))
    }

    /// POST /channels/{channel_id}/messages with multipart/form-data attachments.
    /// Each file becomes a `files[N]` part paired with an entry in the JSON
    /// `attachments` array. `files` is consumed by-value so payload bytes can be
    /// moved into multipart Parts without an extra copy.
    pub async fn create_message_with_attachments(
        &self,
        channel_id: &str,
        content: Option<&str>,
        reply_to: Option<&str>,
        thread_id: Option<&str>,
        components: Option<&[serde_json::Value]>,
        files: Vec<crate::channel::media_helpers::MaterializedMedia>,
    ) -> Result<serde_json::Value> {
        let url = match thread_id {
            Some(tid) => self.url(&format!("/channels/{}/messages", tid)),
            None => self.url(&format!("/channels/{}/messages", channel_id)),
        };

        let mut payload = serde_json::Map::new();
        if let Some(text) = content {
            payload.insert(
                "content".to_string(),
                serde_json::Value::String(text.to_string()),
            );
        }
        if let Some(ref_id) = reply_to {
            payload.insert(
                "message_reference".to_string(),
                serde_json::json!({
                    "message_id": ref_id,
                    "fail_if_not_exists": false,
                }),
            );
        }
        if let Some(comps) = components {
            payload.insert("components".to_string(), serde_json::json!(comps));
        }
        let attachments: Vec<serde_json::Value> = files
            .iter()
            .enumerate()
            .map(|(i, m)| serde_json::json!({ "id": i, "filename": m.filename }))
            .collect();
        if !attachments.is_empty() {
            payload.insert(
                "attachments".to_string(),
                serde_json::Value::Array(attachments),
            );
        }

        let payload_json = serde_json::to_string(&serde_json::Value::Object(payload))
            .map_err(|e| anyhow!("Failed to serialize Discord payload_json: {}", e))?;

        let mut form = reqwest::multipart::Form::new().part(
            "payload_json",
            reqwest::multipart::Part::text(payload_json)
                .mime_str("application/json")
                .map_err(|e| anyhow!("Invalid payload_json mime: {}", e))?,
        );
        for (i, m) in files.into_iter().enumerate() {
            let part = reqwest::multipart::Part::bytes(m.bytes)
                .file_name(m.filename)
                .mime_str(&m.mime)
                .map_err(|e| anyhow!("Invalid attachment mime '{}': {}", m.mime, e))?;
            form = form.part(format!("files[{}]", i), part);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", &self.token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow!("create_message_with_attachments request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("create_message_with_attachments parse failed: {}", e))
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

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .patch(&url)
                .header("Authorization", &self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("edit_message request failed: {}", e))
        })
        .await?;

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

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .delete(&url)
                .header("Authorization", &self.token)
                .send()
                .await
                .map_err(|e| anyhow!("delete_message request failed: {}", e))
        })
        .await?;

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

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .post(&url)
                .header("Authorization", &self.token)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("create_interaction_response request failed: {}", e))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }

    // ── Typing ──────────────────────────────────────────────────────

    /// POST /channels/{channel_id}/typing — trigger typing indicator.
    pub async fn trigger_typing(&self, channel_id: &str) -> Result<()> {
        let url = self.url(&format!("/channels/{}/typing", channel_id));

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .post(&url)
                .header("Authorization", &self.token)
                .header("Content-Length", "0")
                .send()
                .await
                .map_err(|e| anyhow!("trigger_typing request failed: {}", e))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }

    // ── Channels ────────────────────────────────────────────────────

    /// GET /channels/{channel_id} — fetch channel object (used to map Discord
    /// `type` enum to ChatType for forum threads / guild text channels).
    pub async fn get_channel(&self, channel_id: &str) -> Result<serde_json::Value> {
        let url = self.url(&format!("/channels/{}", channel_id));
        let resp = with_rate_limit_retry(3, || async {
            self.client
                .get(&url)
                .header("Authorization", &self.token)
                .send()
                .await
                .map_err(|e| anyhow!("get_channel request failed: {}", e))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        resp.json()
            .await
            .map_err(|e| anyhow!("get_channel parse failed: {}", e))
    }

    // ── Application Commands ────────────────────────────────────────

    /// PUT /applications/{application_id}/commands — bulk overwrite global commands.
    pub async fn bulk_overwrite_global_commands(
        &self,
        application_id: &str,
        commands: Vec<serde_json::Value>,
    ) -> Result<()> {
        let url = self.url(&format!("/applications/{}/commands", application_id));

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .put(&url)
                .header("Authorization", &self.token)
                .json(&commands)
                .send()
                .await
                .map_err(|e| anyhow!("bulk_overwrite_global_commands request failed: {}", e))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(Self::parse_error(resp).await);
        }
        Ok(())
    }
}

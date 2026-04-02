use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

/// Slack Web API client.
///
/// Uses the bot token (xoxb-...) for all API calls except `connections_open`,
/// which requires the app token (xapp-...).
pub struct SlackApi {
    client: reqwest::Client,
    bot_token: String,
}

/// Generic Slack API response wrapper.
/// All Slack API responses have `{ok: bool, error?: string, ...}`.
#[derive(Debug, Deserialize)]
struct SlackResponse<T> {
    ok: bool,
    error: Option<String>,
    #[serde(flatten)]
    data: Option<T>,
}

/// Response from `auth.test`.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthTestResponse {
    pub user_id: String,
    pub user: String,
    pub team_id: String,
    pub team: String,
}

/// Response from `chat.postMessage` (partial).
#[derive(Debug, Deserialize)]
struct PostMessageData {
    ts: Option<String>,
}

/// Response from `apps.connections.open`.
#[derive(Debug, Deserialize)]
struct ConnectionsOpenData {
    url: Option<String>,
}

impl SlackApi {
    /// Create a new Slack API client with the given bot token and optional proxy.
    pub fn new(bot_token: &str, proxy: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));

        if let Some(proxy_url) = proxy {
            if let Ok(p) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(p);
            }
        }

        let client = builder
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            bot_token: bot_token.to_string(),
        }
    }

    /// Make a POST request to a Slack Web API method with the bot token.
    async fn slack_post<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        self.slack_post_with_token(method, &self.bot_token, body)
            .await
    }

    /// Make a POST request to a Slack Web API method with a specified token.
    async fn slack_post_with_token<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let url = format!("https://slack.com/api/{}", method);

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Slack API request failed for {}: {}", method, e))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Slack API {} returned HTTP {}: {}",
                method,
                status,
                crate::truncate_utf8(&body_text, 500)
            ));
        }

        let slack_resp: SlackResponse<T> = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Slack API response for {}: {}", method, e))?;

        if !slack_resp.ok {
            let error = slack_resp.error.unwrap_or_else(|| "unknown_error".into());
            return Err(anyhow!("Slack API {} error: {}", method, error));
        }

        slack_resp
            .data
            .ok_or_else(|| anyhow!("Slack API {} returned ok but no data", method))
    }

    /// Call `auth.test` to validate the bot token.
    /// Returns bot user info including user_id, user (name), team_id, team.
    pub async fn auth_test(&self) -> Result<AuthTestResponse> {
        self.slack_post("auth.test", serde_json::json!({})).await
    }

    /// Post a message to a channel.
    /// Returns the message timestamp (ts) which serves as the message ID.
    pub async fn chat_post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        let data: PostMessageData = self.slack_post("chat.postMessage", body).await?;
        data.ts
            .ok_or_else(|| anyhow!("chat.postMessage returned ok but no ts"))
    }

    /// Update an existing message.
    pub async fn chat_update(&self, channel: &str, ts: &str, text: &str) -> Result<()> {
        let body = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
        });

        let _: serde_json::Value = self.slack_post("chat.update", body).await?;
        Ok(())
    }

    /// Delete an existing message.
    pub async fn chat_delete(&self, channel: &str, ts: &str) -> Result<()> {
        let body = serde_json::json!({
            "channel": channel,
            "ts": ts,
        });

        let _: serde_json::Value = self.slack_post("chat.delete", body).await?;
        Ok(())
    }

    /// Open a Socket Mode connection.
    /// Uses the **app token** (xapp-...), not the bot token.
    /// Returns a one-time-use WebSocket URL.
    pub async fn connections_open(&self, app_token: &str) -> Result<String> {
        let data: ConnectionsOpenData = self
            .slack_post_with_token("apps.connections.open", app_token, serde_json::json!({}))
            .await?;

        data.url
            .ok_or_else(|| anyhow!("apps.connections.open returned ok but no URL"))
    }
}

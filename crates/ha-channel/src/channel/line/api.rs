use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::fmt;

use crate::channel::rate_limit::with_rate_limit_retry;

/// Failure classification for the non-idempotent LINE reply endpoint.
///
/// A push fallback is safe only when LINE explicitly rejects the reply token.
/// Transport failures and all other responses have an ambiguous delivery
/// outcome and must not trigger a second send.
#[derive(Debug, PartialEq, Eq)]
pub enum LineReplyError {
    InvalidReplyToken { detail: String },
    Ambiguous { detail: String },
}

impl LineReplyError {
    pub fn allows_push_fallback(&self) -> bool {
        matches!(self, Self::InvalidReplyToken { .. })
    }

    fn ambiguous(detail: impl Into<String>) -> Self {
        Self::Ambiguous {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for LineReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReplyToken { detail } => {
                write!(f, "LINE explicitly rejected the reply token: {detail}")
            }
            Self::Ambiguous { detail } => {
                write!(f, "LINE reply delivery outcome is ambiguous: {detail}")
            }
        }
    }
}

impl std::error::Error for LineReplyError {}

fn classify_reply_response_error(status: reqwest::StatusCode, body: &str) -> LineReplyError {
    let explicitly_invalid_reply_token = status == reqwest::StatusCode::BAD_REQUEST
        && serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("Invalid reply token");

    let detail = format!(
        "LINE API POST /v2/bot/message/reply returned {}: {}",
        status,
        ha_core::truncate_utf8(body, 512)
    );
    if explicitly_invalid_reply_token {
        LineReplyError::InvalidReplyToken { detail }
    } else {
        LineReplyError::ambiguous(detail)
    }
}

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
                ha_core::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse bot info JSON")
    }

    /// POST /v2/bot/message/reply - Reply to a message using a reply token.
    ///
    /// The reply token is valid for only ~1 minute after the webhook event.
    pub async fn reply_message(
        &self,
        reply_token: &str,
        messages: Vec<Value>,
    ) -> std::result::Result<(), LineReplyError> {
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
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|error| {
                LineReplyError::ambiguous(format!(
                    "failed to send POST /v2/bot/message/reply: {error}"
                ))
            })?;

        let status = resp.status();
        if !status.is_success() {
            let resp_body = resp.text().await.map_err(|error| {
                LineReplyError::ambiguous(format!(
                    "failed to read LINE reply error response ({status}): {error}"
                ))
            })?;
            return Err(classify_reply_response_error(status, &resp_body));
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

        let retry_key = line_retry_key();
        let resp = with_rate_limit_retry(3, || {
            let req = self
                .client
                .post(&url)
                .bearer_auth(&self.channel_access_token)
                .header("X-Line-Retry-Key", &retry_key)
                .json(&body);
            async move {
                req.send()
                    .await
                    .context("Failed to send POST /v2/bot/message/push")
            }
        })
        .await?;

        let status = resp.status();
        if !status.is_success() {
            let resp_body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "LINE API POST /v2/bot/message/push returned {}: {}",
                status,
                ha_core::truncate_utf8(&resp_body, 512)
            );
        }

        Ok(())
    }

    /// POST /v2/bot/chat/loading/start — display a best-effort waiting
    /// animation in a one-on-one chat. LINE rejects group/room IDs.
    pub async fn start_loading(&self, user_id: &str, loading_seconds: u8) -> Result<()> {
        let body = loading_request_body(user_id, loading_seconds)?;
        let url = format!("{}/v2/bot/chat/loading/start", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.channel_access_token)
            .json(&body)
            .send()
            .await
            .context("Failed to send POST /v2/bot/chat/loading/start")?;
        let status = resp.status();
        if !status.is_success() {
            let resp_body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "LINE API POST /v2/bot/chat/loading/start returned {}: {}",
                status,
                ha_core::truncate_utf8(&resp_body, 512)
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
                ha_core::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse profile JSON")
    }

    /// GET /v2/bot/group/{groupId}/member/{userId}/profile - Get group member profile.
    pub async fn get_group_member_profile(&self, group_id: &str, user_id: &str) -> Result<Value> {
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
                ha_core::truncate_utf8(&body, 512)
            );
        }

        serde_json::from_str(&body).context("Failed to parse group member profile JSON")
    }

    /// Download a LINE message attachment to disk using the Content API.
    /// LINE serves message content (image / video / audio / file) on a
    /// dedicated host: `https://api-data.line.me/v2/bot/message/{id}/content`
    /// — separate from the main `api.line.me` host that serves replies.
    pub async fn download_message_content_to_disk(
        &self,
        message_id: &str,
        dest: &std::path::Path,
        cap_bytes: u64,
    ) -> Result<u64> {
        if message_id.is_empty() {
            anyhow::bail!("Empty LINE message id");
        }
        let url = format!(
            "https://api-data.line.me/v2/bot/message/{}/content",
            urlencoding::encode(message_id)
        );
        let builder = self
            .client
            .get(&url)
            .bearer_auth(&self.channel_access_token);
        crate::channel::inbound_media_common::stream_to_disk(builder, dest, cap_bytes)
            .await
            .context("LINE content download")
    }
}

fn line_retry_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn loading_request_body(user_id: &str, loading_seconds: u8) -> Result<Value> {
    if !user_id.starts_with('U') {
        anyhow::bail!("LINE loading animation requires a one-on-one user id");
    }
    if !(5..=60).contains(&loading_seconds) || loading_seconds % 5 != 0 {
        anyhow::bail!("LINE loadingSeconds must be a multiple of 5 from 5 through 60");
    }
    Ok(serde_json::json!({
        "chatId": user_id,
        "loadingSeconds": loading_seconds,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_retry_key_is_uuid() {
        let key = line_retry_key();
        let parsed = uuid::Uuid::parse_str(&key).unwrap();
        assert_eq!(parsed.to_string(), key);
    }

    #[test]
    fn loading_animation_is_limited_to_dm_ids_and_valid_duration() {
        let body = loading_request_body("U123", 60).unwrap();
        assert_eq!(body["chatId"], "U123");
        assert_eq!(body["loadingSeconds"], 60);
        assert!(loading_request_body("C123", 60).is_err());
        assert!(loading_request_body("U123", 7).is_err());
    }

    #[test]
    fn invalid_reply_token_is_the_only_response_that_allows_push_fallback() {
        let error = classify_reply_response_error(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"message":"Invalid reply token"}"#,
        );

        assert!(error.allows_push_fallback());
        assert!(matches!(error, LineReplyError::InvalidReplyToken { .. }));
    }

    #[test]
    fn other_reply_rejections_have_ambiguous_delivery_outcomes() {
        let cases = [
            (
                reqwest::StatusCode::BAD_REQUEST,
                r#"{"message":"The request body has 1 error(s)"}"#,
            ),
            (
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"message":"Invalid reply token"}"#,
            ),
            (reqwest::StatusCode::BAD_REQUEST, "not-json"),
        ];

        for (status, body) in cases {
            let error = classify_reply_response_error(status, body);
            assert!(!error.allows_push_fallback());
            assert!(matches!(error, LineReplyError::Ambiguous { .. }));
        }
    }
}

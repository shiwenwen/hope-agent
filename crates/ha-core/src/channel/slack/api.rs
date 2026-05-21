use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

use crate::channel::media_helpers::MaterializedMedia;
use crate::channel::rate_limit::with_rate_limit_retry;

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

#[derive(Debug, Deserialize)]
struct UploadUrlData {
    upload_url: Option<String>,
    file_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompleteUploadData {
    files: Option<Vec<CompletedFile>>,
}

#[derive(Debug, Deserialize)]
struct CompletedFile {
    id: Option<String>,
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

        let client = builder.build().unwrap_or_else(|_| reqwest::Client::new());

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
    ///
    /// Slack Web API tier-based rate limits（chat.postMessage tier 4 ≈ 1
    /// msg/sec/channel）通过 HTTP 429 + `Retry-After` header 通知；用
    /// `with_rate_limit_retry` 自动尊重退避。
    async fn slack_post_with_token<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let url = format!("https://slack.com/api/{}", method);
        let auth_header = format!("Bearer {}", token);

        let resp = with_rate_limit_retry(3, || async {
            self.client
                .post(&url)
                .header("Authorization", &auth_header)
                .header("Content-Type", "application/json; charset=utf-8")
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow!("Slack API request failed for {}: {}", method, e))
        })
        .await?;

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
    ///
    /// If `blocks` is provided, they are sent as Slack Block Kit blocks alongside
    /// the `text` (which becomes the fallback for notifications/accessibility).
    pub async fn chat_post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
        blocks: Option<&[serde_json::Value]>,
    ) -> Result<String> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::Value::String(ts.to_string());
        }

        if let Some(blocks) = blocks {
            body["blocks"] = serde_json::Value::Array(blocks.to_vec());
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

    /// Upload and share files using Slack's external upload flow:
    /// files.getUploadURLExternal → POST upload_url → files.completeUploadExternal.
    pub async fn upload_files_external(
        &self,
        channel_id: &str,
        thread_ts: Option<&str>,
        initial_comment: Option<&str>,
        files: Vec<MaterializedMedia>,
    ) -> Result<String> {
        if files.is_empty() {
            return Err(anyhow!("Slack upload_files_external called with no files"));
        }

        let mut complete_files = Vec::with_capacity(files.len());
        for file in files {
            if file.bytes.is_empty() {
                return Err(anyhow!("Slack does not accept zero-length file uploads"));
            }

            let ticket = self
                .files_get_upload_url_external(&file.filename, file.bytes.len())
                .await?;
            self.upload_to_external_url(&ticket.upload_url, file)
                .await?;
            complete_files.push(serde_json::json!({
                "id": ticket.file_id,
                "title": ticket.title,
            }));
        }

        let body = complete_upload_body(channel_id, thread_ts, initial_comment, complete_files);
        let data: CompleteUploadData = self
            .slack_post("files.completeUploadExternal", body)
            .await?;
        let first_id = data
            .files
            .and_then(|files| files.into_iter().find_map(|file| file.id))
            .unwrap_or_else(|| "file_uploaded".to_string());
        Ok(first_id)
    }

    async fn files_get_upload_url_external(
        &self,
        filename: &str,
        length: usize,
    ) -> Result<UploadTicket> {
        let data: UploadUrlData = self
            .slack_post(
                "files.getUploadURLExternal",
                serde_json::json!({
                    "filename": filename,
                    "length": length,
                }),
            )
            .await?;
        Ok(UploadTicket {
            upload_url: data
                .upload_url
                .ok_or_else(|| anyhow!("files.getUploadURLExternal returned no upload_url"))?,
            file_id: data
                .file_id
                .ok_or_else(|| anyhow!("files.getUploadURLExternal returned no file_id"))?,
            title: filename.to_string(),
        })
    }

    async fn upload_to_external_url(
        &self,
        upload_url: &str,
        file: MaterializedMedia,
    ) -> Result<()> {
        validate_slack_upload_url(upload_url)?;
        let part = reqwest::multipart::Part::bytes(file.bytes)
            .file_name(file.filename.clone())
            .mime_str(&file.mime)
            .map_err(|e| anyhow!("Invalid Slack upload mime '{}': {}", file.mime, e))?;
        let form = reqwest::multipart::Form::new().part("filename", part);
        let resp = self
            .client
            .post(upload_url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow!("Slack external upload request failed: {}", e))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Slack external upload returned HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 500)
            ));
        }
        Ok(())
    }

    /// Download a Slack-hosted file (`url_private` / `url_private_download`)
    /// to `dest` using the bot token. Slack's private file URLs return a
    /// login page (HTTP 200, HTML body) when fetched without
    /// `Authorization: Bearer xoxb-…`, so the LLM's `web_fetch` can't
    /// reach them — this method is the only way inbound Slack attachments
    /// become locally readable.
    ///
    /// Validates the URL host is `*.slack.com` (Slack's own CDN) so a
    /// poisoned event payload can't redirect the bot's token to an
    /// attacker-controlled host. SSRF check runs as well for the IP
    /// classification layer.
    pub async fn download_file_to_disk(
        &self,
        url: &str,
        dest: &std::path::Path,
        cap_bytes: u64,
    ) -> Result<u64> {
        // Host pin first — cheap, catches the obvious injection.
        let parsed_url = url::Url::parse(url).map_err(|e| anyhow!("Invalid Slack URL: {}", e))?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("Slack URL has no host: {}", url))?;
        let host_ok =
            host == "files.slack.com" || host == "slack.com" || host.ends_with(".slack.com");
        if !host_ok {
            return Err(anyhow!(
                "Refusing to download with bot token from non-Slack host: {}",
                host
            ));
        }
        // SSRF layer — pinned host could in theory still DNS-resolve to a
        // private IP (e.g. internal split-horizon DNS); the policy check
        // refuses metadata/private/loopback by default.
        crate::security::ssrf::check_url(url, crate::security::ssrf::SsrfPolicy::Default, &[])
            .await
            .map_err(|e| anyhow!("Slack file URL blocked: {}", e))?;

        let builder = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.bot_token));
        crate::channel::inbound_media_common::stream_to_disk(builder, dest, cap_bytes).await
    }
}

struct UploadTicket {
    upload_url: String,
    file_id: String,
    title: String,
}

fn complete_upload_body(
    channel_id: &str,
    thread_ts: Option<&str>,
    initial_comment: Option<&str>,
    files: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel_id": channel_id,
        "files": files,
    });
    if let Some(thread_ts) = thread_ts.filter(|s| !s.is_empty()) {
        body["thread_ts"] = serde_json::Value::String(thread_ts.to_string());
    }
    if let Some(comment) = initial_comment.filter(|s| !s.is_empty()) {
        body["initial_comment"] = serde_json::Value::String(comment.to_string());
    }
    body
}

fn validate_slack_upload_url(upload_url: &str) -> Result<()> {
    let parsed =
        url::Url::parse(upload_url).map_err(|e| anyhow!("Invalid Slack upload URL: {}", e))?;
    if parsed.scheme() != "https" {
        return Err(anyhow!("Slack upload URL must use https"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("Slack upload URL has no host"))?;
    if host == "slack.com" || host == "files.slack.com" || host.ends_with(".slack.com") {
        Ok(())
    } else {
        Err(anyhow!(
            "Refusing Slack upload URL from non-Slack host: {}",
            host
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_upload_body, validate_slack_upload_url};

    #[test]
    fn complete_upload_body_includes_share_target_and_comment() {
        let body = complete_upload_body(
            "C123",
            Some("1710000000.000100"),
            Some("hello"),
            vec![serde_json::json!({"id": "F123", "title": "cat.png"})],
        );

        assert_eq!(body["channel_id"], "C123");
        assert_eq!(body["thread_ts"], "1710000000.000100");
        assert_eq!(body["initial_comment"], "hello");
        assert_eq!(body["files"][0]["id"], "F123");
    }

    #[test]
    fn validate_slack_upload_url_requires_https_slack_host() {
        assert!(validate_slack_upload_url("https://files.slack.com/upload/v1/abc").is_ok());
        assert!(validate_slack_upload_url("http://files.slack.com/upload/v1/abc").is_err());
        assert!(validate_slack_upload_url("https://example.com/upload/v1/abc").is_err());
    }
}

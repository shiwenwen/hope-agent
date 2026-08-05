use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::time::Duration;

use crate::channel::media_helpers::MaterializedMedia;
use crate::channel::rate_limit::with_rate_limit_retry;

pub(crate) const SLACK_STREAM_MARKDOWN_MAX_CHARS: usize = 12_000;
const SLACK_STREAM_FIELD_MAX_CHARS: usize = 256;

/// Slack-native stream chunks. Keeping this enum private to the Slack adapter
/// prevents Block Kit/Web API JSON from leaking into the channel contract.
#[derive(Debug, Clone)]
pub enum SlackStreamChunk {
    MarkdownText(String),
    PlanUpdate {
        title: String,
    },
    TaskUpdate {
        id: String,
        title: String,
        status: SlackStreamTaskStatus,
        details: Option<String>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum SlackStreamTaskStatus {
    Pending,
    InProgress,
    Complete,
    Error,
}

impl SlackStreamTaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Complete => "complete",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackStreamApiErrorKind {
    Unsupported,
    InvalidTarget,
    InvalidContent,
    Rejected,
    RateLimited,
    Transient,
    Expired,
    Ambiguous,
}

pub struct SlackStreamApiError {
    pub kind: SlackStreamApiErrorKind,
    message: String,
}

impl SlackStreamApiError {
    fn new(kind: SlackStreamApiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: ha_core::logging::redact_sensitive(&message.into()),
        }
    }
}

impl std::fmt::Debug for SlackStreamApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackStreamApiError")
            .field("kind", &self.kind)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

impl std::fmt::Display for SlackStreamApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SlackStreamApiError {}

pub type SlackStreamApiResult<T> = std::result::Result<T, SlackStreamApiError>;

#[derive(Debug, Clone)]
pub struct SlackStreamMessage {
    pub channel: String,
    pub ts: String,
}

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

/// Shared response fields from chat.startStream/appendStream/stopStream.
#[derive(Debug, Deserialize)]
struct StreamMessageData {
    channel: Option<String>,
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

fn redact_slack_error(message: &str, token: &str) -> String {
    let message = ha_core::logging::redact_sensitive(message);
    if token.is_empty() {
        message
    } else {
        message.replace(token, "[REDACTED]")
    }
}

fn truncate_stream_field(value: &str) -> String {
    value.chars().take(SLACK_STREAM_FIELD_MAX_CHARS).collect()
}

fn split_stream_markdown(markdown: &str) -> Vec<String> {
    if markdown.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for ch in markdown.chars() {
        if current_chars == SLACK_STREAM_MARKDOWN_MAX_CHARS {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(ch);
        current_chars += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn stream_chunks_json(chunks: &[SlackStreamChunk]) -> Vec<serde_json::Value> {
    let mut values = Vec::new();
    for chunk in chunks {
        match chunk {
            SlackStreamChunk::MarkdownText(markdown) => {
                values.extend(split_stream_markdown(markdown).into_iter().map(|part| {
                    serde_json::json!({
                        "type": "markdown_text",
                        "text": part,
                    })
                }));
            }
            SlackStreamChunk::PlanUpdate { title } => {
                let title = truncate_stream_field(title);
                if !title.is_empty() {
                    values.push(serde_json::json!({
                        "type": "plan_update",
                        "title": title,
                    }));
                }
            }
            SlackStreamChunk::TaskUpdate {
                id,
                title,
                status,
                details,
            } => {
                let id = truncate_stream_field(id);
                let title = truncate_stream_field(title);
                if id.is_empty() || title.is_empty() {
                    continue;
                }
                let mut value = serde_json::json!({
                    "type": "task_update",
                    "id": id,
                    "title": title,
                    "status": status.as_str(),
                });
                if let Some(details) = details.as_deref().filter(|value| !value.is_empty()) {
                    value["details"] = serde_json::Value::String(truncate_stream_field(details));
                }
                values.push(value);
            }
        }
    }
    values
}

fn stop_stream_body(
    channel: &str,
    ts: &str,
    chunks: &[serde_json::Value],
    blocks: &[serde_json::Value],
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "channel": channel,
        "ts": ts,
    });
    if !chunks.is_empty() {
        body["chunks"] = serde_json::Value::Array(chunks.to_vec());
    }
    if !blocks.is_empty() {
        body["blocks"] = serde_json::Value::Array(blocks.to_vec());
    }
    body
}

fn classify_stream_error_code(code: &str) -> SlackStreamApiErrorKind {
    match code {
        "deprecated_endpoint"
        | "enterprise_is_restricted"
        | "method_deprecated"
        | "method_not_supported_for_channel_type"
        | "channel_type_not_supported"
        | "not_supported" => SlackStreamApiErrorKind::Unsupported,
        "channel_not_found"
        | "not_in_channel"
        | "thread_not_found"
        | "invalid_thread_ts"
        | "missing_recipient_user_id"
        | "missing_recipient_team_id" => SlackStreamApiErrorKind::InvalidTarget,
        "invalid_arguments"
        | "invalid_blocks"
        | "invalid_chunks"
        | "msg_too_long"
        | "no_text"
        | "msg_blocks_invalid"
        | "msg_blocks_too_long"
        | "msg_blocks_too_many" => SlackStreamApiErrorKind::InvalidContent,
        "message_not_found" | "message_stream_not_found" | "stream_not_found" => {
            SlackStreamApiErrorKind::Expired
        }
        "rate_limited" | "ratelimited" => SlackStreamApiErrorKind::RateLimited,
        "access_denied"
        | "account_inactive"
        | "ekm_access_denied"
        | "invalid_auth"
        | "missing_scope"
        | "no_permission"
        | "not_allowed_token_type"
        | "not_authed"
        | "org_login_required"
        | "restricted_action"
        | "team_access_not_granted"
        | "token_expired"
        | "token_revoked" => SlackStreamApiErrorKind::Rejected,
        "fatal_error" | "internal_error" | "request_timeout" | "service_unavailable" => {
            SlackStreamApiErrorKind::Ambiguous
        }
        // Unknown Slack failures cannot prove that the mutating request had
        // no effect. Treating them as retryable/fallback-safe risks duplicate
        // replies when Slack accepted the stream before returning the error.
        _ => SlackStreamApiErrorKind::Ambiguous,
    }
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
                .map_err(|error| {
                    anyhow!(
                        "Slack API request failed for {}: {}",
                        method,
                        redact_slack_error(&error.to_string(), token)
                    )
                })
        })
        .await?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let body_text = redact_slack_error(&body_text, token);
            return Err(anyhow!(
                "Slack API {} returned HTTP {}: {}",
                method,
                status,
                ha_core::truncate_utf8(&body_text, 500)
            ));
        }

        let slack_resp: SlackResponse<T> = resp.json().await.map_err(|error| {
            anyhow!(
                "Failed to parse Slack API response for {}: {}",
                method,
                redact_slack_error(&error.to_string(), token)
            )
        })?;

        if !slack_resp.ok {
            let error = slack_resp.error.unwrap_or_else(|| "unknown_error".into());
            return Err(anyhow!(
                "Slack API {} error: {}",
                method,
                redact_slack_error(&error, token)
            ));
        }

        slack_resp
            .data
            .ok_or_else(|| anyhow!("Slack API {} returned ok but no data", method))
    }

    /// Stream API requests share the ordinary Slack 429 handling, but retain
    /// an explicit delivery-ambiguity boundary for non-idempotent appends.
    async fn slack_stream_post<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: serde_json::Value,
        ambiguous_message: &'static str,
    ) -> SlackStreamApiResult<T> {
        let url = format!("https://slack.com/api/{}", method);
        let auth_header = format!("Bearer {}", self.bot_token);
        let response = with_rate_limit_retry(3, || async {
            self.client
                .post(&url)
                .header("Authorization", &auth_header)
                .header("Content-Type", "application/json; charset=utf-8")
                .json(&body)
                .send()
                .await
                .map_err(|error| {
                    anyhow!(
                        "Slack stream request failed for {}: {}",
                        method,
                        redact_slack_error(&error.to_string(), &self.bot_token)
                    )
                })
        })
        .await
        .map_err(|error| {
            SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                format!(
                    "{}: {}",
                    ambiguous_message,
                    redact_slack_error(&error.to_string(), &self.bot_token)
                ),
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            let kind = if status.as_u16() == 429 {
                SlackStreamApiErrorKind::RateLimited
            } else if status == reqwest::StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
                // A mutating stream call may already have been applied before
                // Slack generated its 5xx response. Replaying it is unsafe.
                SlackStreamApiErrorKind::Ambiguous
            } else {
                SlackStreamApiErrorKind::Rejected
            };
            return Err(SlackStreamApiError::new(
                kind,
                format!("Slack API {} returned HTTP {}", method, status),
            ));
        }

        let slack_response: SlackResponse<T> = response.json().await.map_err(|error| {
            // A 2xx response whose representation cannot be decoded gives us
            // no safe basis for replaying a mutating stream call.
            SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                format!(
                    "{}: {}",
                    ambiguous_message,
                    redact_slack_error(&error.to_string(), &self.bot_token)
                ),
            )
        })?;

        if !slack_response.ok {
            let code = slack_response
                .error
                .unwrap_or_else(|| "unknown_error".to_string());
            return Err(SlackStreamApiError::new(
                classify_stream_error_code(&code),
                format!("Slack API {} error: {}", method, code),
            ));
        }

        slack_response.data.ok_or_else(|| {
            SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                format!("{}: response contained no data", ambiguous_message),
            )
        })
    }

    /// Start a Slack-native reply stream. Markdown is sent as canonical
    /// Markdown, not converted to Slack's legacy mrkdwn dialect.
    pub async fn chat_start_stream(
        &self,
        channel: &str,
        thread_ts: &str,
        recipient_user_id: &str,
        recipient_team_id: &str,
        chunks: &[SlackStreamChunk],
    ) -> SlackStreamApiResult<SlackStreamMessage> {
        let chunks = stream_chunks_json(chunks);
        if chunks.is_empty() {
            return Err(SlackStreamApiError::new(
                SlackStreamApiErrorKind::InvalidContent,
                "Slack chat.startStream requires at least one non-empty chunk",
            ));
        }
        let body = serde_json::json!({
            "channel": channel,
            "thread_ts": thread_ts,
            "recipient_user_id": recipient_user_id,
            "recipient_team_id": recipient_team_id,
            "task_display_mode": "dense",
            "chunks": chunks,
        });
        let data: StreamMessageData = self
            .slack_stream_post(
                "chat.startStream",
                body,
                "Slack chat.startStream outcome is ambiguous",
            )
            .await?;
        let ts = data.ts.ok_or_else(|| {
            SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                "Slack chat.startStream outcome is ambiguous: response contained no ts",
            )
        })?;
        Ok(SlackStreamMessage {
            channel: data.channel.unwrap_or_else(|| channel.to_string()),
            ts,
        })
    }

    /// Append only newly accepted content. A transport failure is ambiguous:
    /// Slack may have appended the suffix even though its response was lost,
    /// so this method deliberately does not retry such failures.
    pub async fn chat_append_stream(
        &self,
        channel: &str,
        ts: &str,
        chunks: &[SlackStreamChunk],
    ) -> SlackStreamApiResult<()> {
        let chunks = stream_chunks_json(chunks);
        if chunks.is_empty() {
            return Ok(());
        }
        let body = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "chunks": chunks,
        });
        let data: StreamMessageData = self
            .slack_stream_post(
                "chat.appendStream",
                body,
                "Slack chat.appendStream outcome is ambiguous",
            )
            .await?;
        if data.ts.is_none() {
            return Err(SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                "Slack chat.appendStream outcome is ambiguous: response contained no ts",
            ));
        }
        Ok(())
    }

    /// Stop a stream, optionally flushing remaining chunks and rendering final
    /// Block Kit actions below the streamed Markdown.
    pub async fn chat_stop_stream(
        &self,
        channel: &str,
        ts: &str,
        chunks: &[SlackStreamChunk],
        blocks: &[serde_json::Value],
    ) -> SlackStreamApiResult<SlackStreamMessage> {
        let chunks = stream_chunks_json(chunks);
        let body = stop_stream_body(channel, ts, &chunks, blocks);

        let data: StreamMessageData = self
            .slack_stream_post(
                "chat.stopStream",
                body,
                "Slack chat.stopStream outcome is ambiguous",
            )
            .await?;
        let response_ts = data.ts.ok_or_else(|| {
            SlackStreamApiError::new(
                SlackStreamApiErrorKind::Ambiguous,
                "Slack chat.stopStream outcome is ambiguous: response contained no ts",
            )
        })?;
        Ok(SlackStreamMessage {
            channel: data.channel.unwrap_or_else(|| channel.to_string()),
            ts: response_ts,
        })
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

    /// Update an existing message, explicitly replacing its blocks when the
    /// caller supplies a block list (including an empty list).
    pub async fn chat_update(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: Option<&[serde_json::Value]>,
    ) -> Result<()> {
        let mut body = serde_json::json!({
            "channel": channel,
            "ts": ts,
            "text": text,
        });
        if let Some(blocks) = blocks {
            body["blocks"] = serde_json::Value::Array(blocks.to_vec());
        }

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
            // The reqwest error can include Slack's one-time signed upload
            // URL. Keep that capability out of surfaced/logged error text.
            .map_err(|_| anyhow!("Slack external upload request failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let body = ha_core::logging::redact_sensitive(&body);
            return Err(anyhow!(
                "Slack external upload returned HTTP {}: {}",
                status,
                ha_core::truncate_utf8(&body, 500)
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
        ha_core::security::ssrf::check_url(url, ha_core::security::ssrf::SsrfPolicy::Default, &[])
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
    use super::{
        classify_stream_error_code, complete_upload_body, split_stream_markdown, stop_stream_body,
        stream_chunks_json, validate_slack_upload_url, SlackResponse, SlackStreamApiErrorKind,
        SlackStreamChunk, SlackStreamTaskStatus, StreamMessageData,
        SLACK_STREAM_MARKDOWN_MAX_CHARS,
    };

    #[test]
    fn stream_markdown_chunks_are_unicode_safe() {
        let markdown = format!("{}🙂中文", "a".repeat(SLACK_STREAM_MARKDOWN_MAX_CHARS - 1));
        let chunks = split_stream_markdown(&markdown);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), SLACK_STREAM_MARKDOWN_MAX_CHARS);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.is_char_boundary(chunk.len())));
        assert_eq!(chunks.concat(), markdown);
    }

    #[test]
    fn stream_chunks_preserve_markdown_and_encode_task_plan() {
        let values = stream_chunks_json(&[
            SlackStreamChunk::MarkdownText("**bold** `code`".to_string()),
            SlackStreamChunk::PlanUpdate {
                title: "检查实现".to_string(),
            },
            SlackStreamChunk::TaskUpdate {
                id: "tool-1".to_string(),
                title: "读取文件".to_string(),
                status: SlackStreamTaskStatus::InProgress,
                details: Some("处理中".to_string()),
            },
        ]);

        assert_eq!(values[0]["type"], "markdown_text");
        assert_eq!(values[0]["text"], "**bold** `code`");
        assert_eq!(values[1]["type"], "plan_update");
        assert_eq!(values[1]["title"], "检查实现");
        assert_eq!(values[2]["type"], "task_update");
        assert_eq!(values[2]["id"], "tool-1");
        assert_eq!(values[2]["status"], "in_progress");
        assert_eq!(values[2]["details"], "处理中");
    }

    #[test]
    fn stream_error_codes_are_classified_for_worker_fallback() {
        for code in [
            "deprecated_endpoint",
            "enterprise_is_restricted",
            "method_deprecated",
        ] {
            assert_eq!(
                classify_stream_error_code(code),
                SlackStreamApiErrorKind::Unsupported,
                "{code} must allow a safe legacy fallback"
            );
        }
        assert_eq!(
            classify_stream_error_code("method_not_supported_for_channel_type"),
            SlackStreamApiErrorKind::Unsupported
        );
        assert_eq!(
            classify_stream_error_code("channel_not_found"),
            SlackStreamApiErrorKind::InvalidTarget
        );
        assert_eq!(
            classify_stream_error_code("invalid_chunks"),
            SlackStreamApiErrorKind::InvalidContent
        );
        assert_eq!(
            classify_stream_error_code("message_stream_not_found"),
            SlackStreamApiErrorKind::Expired
        );
        assert_eq!(
            classify_stream_error_code("ratelimited"),
            SlackStreamApiErrorKind::RateLimited
        );
        assert_eq!(
            classify_stream_error_code("internal_error"),
            SlackStreamApiErrorKind::Ambiguous
        );
        assert_eq!(
            classify_stream_error_code("missing_recipient_team_id"),
            SlackStreamApiErrorKind::InvalidTarget
        );
        assert_eq!(
            classify_stream_error_code("msg_blocks_too_many"),
            SlackStreamApiErrorKind::InvalidContent
        );
        assert_eq!(
            classify_stream_error_code("missing_scope"),
            SlackStreamApiErrorKind::Rejected
        );
        assert_eq!(
            classify_stream_error_code("future_unknown_error"),
            SlackStreamApiErrorKind::Ambiguous
        );
    }

    #[test]
    fn stop_stream_body_omits_empty_optional_fields() {
        let empty = stop_stream_body("C123", "1710000000.000100", &[], &[]);
        assert!(empty.get("chunks").is_none());
        assert!(empty.get("blocks").is_none());

        let chunks = vec![serde_json::json!({
            "type": "markdown_text",
            "text": "done",
        })];
        let blocks = vec![serde_json::json!({"type": "actions", "elements": []})];
        let populated = stop_stream_body("C123", "1710000000.000100", &chunks, &blocks);
        assert_eq!(populated["chunks"], serde_json::Value::Array(chunks));
        assert_eq!(populated["blocks"], serde_json::Value::Array(blocks));
    }

    #[test]
    fn stream_response_flattens_channel_and_ts() {
        let response: SlackResponse<StreamMessageData> =
            serde_json::from_value(serde_json::json!({
                "ok": true,
                "channel": "C123",
                "ts": "1710000000.000100",
            }))
            .expect("stream response should deserialize");
        let data = response.data.expect("stream response should contain data");
        assert_eq!(data.channel.as_deref(), Some("C123"));
        assert_eq!(data.ts.as_deref(), Some("1710000000.000100"));
    }

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

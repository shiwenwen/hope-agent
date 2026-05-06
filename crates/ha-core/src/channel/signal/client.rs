use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::channel::types::*;

/// JSON-RPC + SSE client for the signal-cli HTTP daemon.
pub struct SignalClient {
    client: Client,
    base_url: String,
    account: String,
}

impl SignalClient {
    /// Create a new client targeting the signal-cli HTTP daemon.
    pub fn new(port: u16, account: String) -> Self {
        Self {
            client: Client::new(),
            // 与 daemon.rs 端绑定地址保持一致（127.0.0.1 而非 localhost）
            base_url: format!("http://127.0.0.1:{}", port),
            account,
        }
    }

    /// Make a JSON-RPC 2.0 call to the signal-cli daemon.
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let url = format!("{}/api/v1/rpc", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .with_context(|| format!("Signal RPC request failed for method '{}'", method))?;

        let status = resp.status();

        // 201 means success with no body (e.g. send)
        if status.as_u16() == 201 {
            return Ok(Value::Null);
        }

        let text = resp
            .text()
            .await
            .with_context(|| format!("Failed to read Signal RPC response for '{}'", method))?;

        if text.is_empty() {
            anyhow::bail!(
                "Signal RPC empty response (status {}) for '{}'",
                status,
                method
            );
        }

        let parsed: Value = serde_json::from_str(&text).with_context(|| {
            format!(
                "Signal RPC malformed JSON (status {}) for '{}'",
                status, method
            )
        })?;

        // Check for JSON-RPC error
        if let Some(err) = parsed.get("error") {
            let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("Signal RPC error {}: {}", code, msg);
        }

        // Return the result field
        Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Send a text message to a recipient (phone number or group ID).
    ///
    /// `quote_timestamp` + `quote_author` 必须成对——signal-cli `send` 文档
    /// 要求 reply 必须同时提供 `quoteTimestamp` + `quoteAuthor`（最少这两
    /// 字段），缺一即被忽略，发出去就是普通消息。
    pub async fn send_message(
        &self,
        recipient: &str,
        message: &str,
        _attachments: &[String],
        quote_timestamp: Option<i64>,
        quote_author: Option<&str>,
    ) -> Result<Value> {
        let mut params = serde_json::json!({
            "account": self.account,
            "message": message,
        });

        // Determine if this is a group or DM
        if is_group_id(recipient) {
            params["groupId"] = Value::String(recipient.to_string());
        } else {
            params["recipient"] = serde_json::json!([recipient]);
        }

        // signal-cli quote 必须有 timestamp + author 才生效
        if let (Some(ts), Some(author)) = (quote_timestamp, quote_author) {
            params["quoteTimestamp"] = Value::Number(serde_json::Number::from(ts));
            params["quoteAuthor"] = Value::String(author.to_string());
        }

        self.rpc("send", params).await
    }

    /// Send a typing indicator to a recipient.
    pub async fn send_typing(&self, recipient: &str) -> Result<()> {
        let mut params = serde_json::json!({
            "account": self.account,
        });

        if is_group_id(recipient) {
            params["groupId"] = Value::String(recipient.to_string());
        } else {
            params["recipient"] = Value::String(recipient.to_string());
        }

        self.rpc("sendTyping", params).await?;
        Ok(())
    }

    /// Delete (remote-delete) a previously sent message.
    pub async fn delete_message(&self, recipient: &str, timestamp: i64) -> Result<()> {
        let mut params = serde_json::json!({
            "account": self.account,
            "targetTimestamp": timestamp,
        });

        if is_group_id(recipient) {
            params["groupId"] = Value::String(recipient.to_string());
        } else {
            params["recipient"] = Value::String(recipient.to_string());
        }

        self.rpc("remoteDelete", params).await?;
        Ok(())
    }

    /// List registered identities (used for validation / probe).
    pub async fn list_identities(&self) -> Result<Value> {
        let params = serde_json::json!({
            "account": self.account,
        });
        self.rpc("listIdentities", params).await
    }

    /// Run the SSE event loop, parsing inbound messages and sending them
    /// through `inbound_tx`. Reconnects with exponential backoff on disconnect.
    pub async fn run_sse_loop(
        &self,
        account_id: String,
        inbound_tx: mpsc::Sender<MsgContext>,
        cancel: CancellationToken,
    ) {
        let backoff_secs = [1u64, 2, 5, 10, 30, 60];
        let mut attempt = 0usize;

        loop {
            if cancel.is_cancelled() {
                break;
            }

            let url = format!("{}/api/v1/events?account={}", self.base_url, self.account);

            app_info!(
                "channel",
                "signal-sse",
                "Connecting to SSE endpoint: {}",
                url
            );

            match self
                .connect_sse(&url, &account_id, &inbound_tx, &cancel)
                .await
            {
                Ok(()) => {
                    // Clean exit (cancel was triggered)
                    break;
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let delay = backoff_secs[attempt.min(backoff_secs.len() - 1)];
                    app_warn!(
                        "channel",
                        "signal-sse",
                        "SSE connection lost: {}. Reconnecting in {}s (attempt {})",
                        e,
                        delay,
                        attempt + 1
                    );
                    attempt += 1;

                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {}
                        _ = cancel.cancelled() => break,
                    }
                }
            }
        }

        app_info!(
            "channel",
            "signal-sse",
            "SSE loop exiting for account {}",
            account_id
        );
    }

    /// Connect to the SSE endpoint and process events until disconnect or cancel.
    async fn connect_sse(
        &self,
        url: &str,
        account_id: &str,
        inbound_tx: &mpsc::Sender<MsgContext>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        use futures_util::StreamExt;

        let resp = self
            .client
            .get(url)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .context("Failed to connect to Signal SSE endpoint")?;

        if !resp.status().is_success() {
            anyhow::bail!("Signal SSE failed: HTTP {}", resp.status());
        }

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut current_event = String::new();
        let mut current_data = String::new();

        loop {
            tokio::select! {
                chunk = stream.next() => {
                    match chunk {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));

                            // Process complete lines
                            while let Some(line_end) = buffer.find('\n') {
                                let line = buffer[..line_end].trim_end_matches('\r').to_string();
                                buffer = buffer[line_end + 1..].to_string();

                                if line.is_empty() {
                                    // Empty line = event boundary
                                    if !current_data.is_empty() {
                                        if current_event == "receive" || current_event.is_empty() {
                                            if let Err(e) = self.handle_sse_data(
                                                &current_data,
                                                account_id,
                                                inbound_tx,
                                            ).await {
                                                app_warn!(
                                                    "channel",
                                                    "signal-sse",
                                                    "Failed to handle SSE event: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    current_event.clear();
                                    current_data.clear();
                                } else if line.starts_with(':') {
                                    // SSE comment, ignore
                                } else if let Some(value) = line.strip_prefix("event:") {
                                    current_event = value.trim().to_string();
                                } else if let Some(value) = line.strip_prefix("data:") {
                                    let value = value.strip_prefix(' ').unwrap_or(value);
                                    if current_data.is_empty() {
                                        current_data = value.to_string();
                                    } else {
                                        current_data.push('\n');
                                        current_data.push_str(value);
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            anyhow::bail!("SSE stream error: {}", e);
                        }
                        None => {
                            // Stream ended
                            anyhow::bail!("SSE stream ended unexpectedly");
                        }
                    }
                }
                _ = cancel.cancelled() => {
                    return Ok(());
                }
            }
        }
    }

    /// Parse an SSE data payload from the `receive` event and convert to MsgContext.
    async fn handle_sse_data(
        &self,
        data: &str,
        account_id: &str,
        inbound_tx: &mpsc::Sender<MsgContext>,
    ) -> Result<()> {
        let envelope: Value =
            serde_json::from_str(data).context("Failed to parse SSE event data as JSON")?;

        // The envelope structure from signal-cli:
        // { "envelope": { "source": "+123...", "sourceName": "Alice", "dataMessage": { ... }, ... } }
        let env = envelope.get("envelope").unwrap_or(&envelope);

        let data_message = match env.get("dataMessage") {
            Some(dm) => dm,
            None => return Ok(()), // Not a data message (could be receipt, typing, etc.)
        };

        // Extract sender info
        let sender_id = env
            .get("sourceNumber")
            .or_else(|| env.get("source"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if sender_id.is_empty() {
            return Ok(());
        }

        let sender_name = env
            .get("sourceName")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract message text
        let text = data_message
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract timestamp as message_id
        let timestamp = data_message
            .get("timestamp")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let message_id = timestamp.to_string();

        // Determine chat type and chat_id
        let group_info = data_message.get("groupInfo");
        let (chat_type, chat_id, chat_title) = if let Some(gi) = group_info {
            let gid = gi
                .get("groupId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = gi
                .get("groupName")
                .or_else(|| gi.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (ChatType::Group, gid, title)
        } else {
            (ChatType::Dm, sender_id.clone(), None)
        };

        if chat_id.is_empty() {
            return Ok(());
        }

        // Check if bot was mentioned
        let was_mentioned = self.check_mentioned(data_message);

        // Extract reply-to
        let reply_to = data_message
            .get("quote")
            .and_then(|q| q.get("id"))
            .and_then(|v| v.as_i64())
            .map(|ts| ts.to_string());

        // Extract inbound media (attachments)
        let media = self.extract_media(data_message);

        let msg = MsgContext {
            channel_id: ChannelId::Signal,
            account_id: account_id.to_string(),
            sender_id,
            sender_name,
            sender_username: None,
            chat_id,
            chat_type,
            chat_title,
            thread_id: None,
            message_id,
            text,
            media,
            reply_to_message_id: reply_to,
            timestamp: chrono::Utc::now(),
            was_mentioned,
            raw: envelope.clone(),
        };

        if inbound_tx.send(msg).await.is_err() {
            app_warn!(
                "channel",
                "signal-sse",
                "Inbound channel closed, dropping message"
            );
        }

        Ok(())
    }

    /// Check if the bot account phone number appears in the message mentions.
    fn check_mentioned(&self, data_message: &Value) -> bool {
        let mentions = match data_message.get("mentions") {
            Some(Value::Array(arr)) => arr,
            _ => return false,
        };

        for mention in mentions {
            let number = mention.get("number").and_then(|v| v.as_str()).unwrap_or("");
            if number == self.account {
                return true;
            }
        }

        false
    }

    /// Extract attachment metadata from the data message.
    fn extract_media(&self, data_message: &Value) -> Vec<InboundMedia> {
        let attachments = match data_message.get("attachments") {
            Some(Value::Array(arr)) => arr,
            _ => return Vec::new(),
        };

        attachments
            .iter()
            .filter_map(|att| {
                let id = att.get("id").and_then(|v| v.as_str())?.to_string();
                let content_type = att
                    .get("contentType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let file_size = att.get("size").and_then(|v| v.as_u64());

                let media_type = match content_type.as_deref() {
                    Some(ct) if ct.starts_with("image/") => MediaType::Photo,
                    Some(ct) if ct.starts_with("video/") => MediaType::Video,
                    Some(ct) if ct.starts_with("audio/ogg") => MediaType::Voice,
                    Some(ct) if ct.starts_with("audio/") => MediaType::Audio,
                    _ => MediaType::Document,
                };

                Some(InboundMedia {
                    media_type,
                    file_id: id,
                    file_url: None,
                    mime_type: content_type,
                    file_size,
                    caption: None,
                })
            })
            .collect()
    }
}

/// 判断 recipient 是否是 Signal 群组 ID。
///
/// Signal recipient 形态：
/// - `+E164` 电话号（例如 `+15551234567`）
/// - UUID（v4，含 `-`，36 chars）—— 新版用户 identifier
/// - `u:username` —— signal-cli 0.13+ username 形式
/// - 群 ID v1：纯 base64 字符串，长度 22 或 44 chars，不含 `-` `:` `+` `@`
/// - 群 ID v2：以 `group.` 前缀（base64 字串）
///
/// 之前 `!starts_with('+')` 在 UUID 时代会把单聊 UUID 当作群发给 signal-cli，
/// 直接返回 `Group with id ... not found`。
fn is_group_id(recipient: &str) -> bool {
    if recipient.starts_with('+') {
        return false;
    }
    if recipient.starts_with("u:") || recipient.starts_with('@') {
        return false;
    }
    if recipient.starts_with("group.") {
        return true;
    }
    // UUID 含 `-`，例如 `a3b1f5e8-...`
    if recipient.contains('-') {
        return false;
    }
    // 纯 base64 形态群 ID（v1）：22 或 44 字符无破折号
    let len = recipient.len();
    (len == 22 || len == 44)
        && recipient
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_group_id_recognizes_phones() {
        assert!(!is_group_id("+15551234567"));
    }

    #[test]
    fn is_group_id_recognizes_uuid() {
        assert!(!is_group_id("a3b1f5e8-1234-4abc-9def-0123456789ab"));
    }

    #[test]
    fn is_group_id_recognizes_username() {
        assert!(!is_group_id("u:alice"));
        assert!(!is_group_id("@alice"));
    }

    #[test]
    fn is_group_id_recognizes_group_v2() {
        assert!(is_group_id("group.AbCdEfGh1234567890=="));
    }

    #[test]
    fn is_group_id_recognizes_base64_v1() {
        // 22 chars base64
        assert!(is_group_id("AbCdEfGhIjKlMnOpQrStUv"));
        // 44 chars base64
        assert!(is_group_id("AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGh"));
    }

    #[test]
    fn is_group_id_rejects_random_strings() {
        // 短而不像 group id
        assert!(!is_group_id("short"));
    }
}

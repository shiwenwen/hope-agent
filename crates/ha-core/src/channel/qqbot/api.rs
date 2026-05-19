use anyhow::{anyhow, Result};
use lru::LruCache;
use serde::de::DeserializeOwned;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::auth::{format_auth_value, QqBotAuth};

/// QQ Bot V2 多端点 chat 命名空间，由 hope-agent 在 inbound MsgContext 时
/// 把平台原始 ID 编码为 `<scope>:<id>` 字串（见 [`gateway`](super::gateway)），
/// 出站 dispatch 时再解回选择正确的 send endpoint。
///
/// - `C2c` 私聊：`POST /v2/users/{openid}/messages`，仅此端点支持 keyboard
/// - `Group` 群：`POST /v2/groups/{group_openid}/messages`，支持 keyboard 与 media
/// - `Channel` 频道：`POST /channels/{channel_id}/messages`
/// - `Dms` 频道私信：`POST /dms/{guild_id}/messages`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QqChatScope<'a> {
    C2c(&'a str),
    Group(&'a str),
    Channel(&'a str),
    Dms(&'a str),
}

impl<'a> QqChatScope<'a> {
    pub fn parse(chat_id: &'a str) -> Result<Self> {
        if let Some(id) = chat_id.strip_prefix("c2c:") {
            Ok(QqChatScope::C2c(id))
        } else if let Some(id) = chat_id.strip_prefix("group:") {
            Ok(QqChatScope::Group(id))
        } else if let Some(id) = chat_id.strip_prefix("channel:") {
            Ok(QqChatScope::Channel(id))
        } else if let Some(id) = chat_id.strip_prefix("dms:") {
            Ok(QqChatScope::Dms(id))
        } else {
            Err(anyhow!(
                "Unknown QQ Bot chat_id format (expected 'c2c:'/'group:'/'channel:'/'dms:' prefix): {}",
                crate::truncate_utf8(chat_id, 100)
            ))
        }
    }

    /// keyboard / media 仅 c2c/group 端点 V2 接口支持；channel/dms 须降级到
    /// 文本格式（数字回复 / 链接）。
    pub fn supports_native_keyboard(&self) -> bool {
        matches!(self, QqChatScope::C2c(_) | QqChatScope::Group(_))
    }

    pub fn supports_native_media(&self) -> bool {
        matches!(self, QqChatScope::C2c(_) | QqChatScope::Group(_))
    }
}

/// QQ Bot REST API client.
///
/// Auth scheme is documented on [`super::auth::AUTH_SCHEME`]; also sends
/// `X-Union-Appid: {app_id}` header.
///
/// **`msg_seq` 状态机**：QQ Bot V2 文档明确"相同的 msg_id + msg_seq 重复发送
/// 会失败"——同一 inbound msg_id 的多次被动回复（流式分段、逐 chunk 发送）
/// 必须每次递增 msg_seq。`msg_seq_map` 维护 per-msg_id 的 counter，下一次
/// 发同一 msg_id 时调 [`Self::next_msg_seq`] 拿单调递增值；LRU cap 1024 自然
/// 驱逐过期条目（msg_id 服务端 5 分钟有效，足够）。
pub struct QqBotApi {
    client: reqwest::Client,
    pub auth: Arc<QqBotAuth>,
    base_url: String,
    msg_seq_map: Mutex<LruCache<String, u32>>,
    passive_reply_contexts: Mutex<LruCache<String, PassiveReplyContext>>,
}

pub const QQBOT_API_BASE_URL: &str = "https://api.sgroup.qq.com";
pub const QQBOT_SANDBOX_API_BASE_URL: &str = "https://sandbox.api.sgroup.qq.com";

/// Response from GET /gateway/bot.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GatewayBotInfo {
    pub url: String,
    #[serde(default = "default_gateway_shards")]
    pub shards: u64,
    #[serde(default)]
    pub session_start_limit: GatewaySessionStartLimit,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct GatewaySessionStartLimit {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub remaining: u64,
    #[serde(default)]
    pub reset_after: u64,
    #[serde(default)]
    pub max_concurrency: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PassiveReplyContext {
    Message {
        msg_id: String,
        event_id: Option<String>,
    },
    Event {
        event_id: String,
    },
}

fn default_gateway_shards() -> u64 {
    1
}

impl QqBotApi {
    /// Create a new QQ Bot API client.
    pub fn new(auth: Arc<QqBotAuth>) -> Self {
        Self::new_with_base_url(auth, QQBOT_API_BASE_URL)
    }

    /// Create a new QQ Bot API client with an explicit production/sandbox base URL.
    pub fn new_with_base_url(auth: Arc<QqBotAuth>, base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let base_url = base_url.into().trim_end_matches('/').to_string();

        Self {
            client,
            auth,
            base_url,
            msg_seq_map: Mutex::new(LruCache::new(
                NonZeroUsize::new(1024).expect("1024 is non-zero"),
            )),
            passive_reply_contexts: Mutex::new(LruCache::new(
                NonZeroUsize::new(1024).expect("1024 is non-zero"),
            )),
        }
    }

    /// 取下一个 `msg_seq` 值（per-msg_id 单调递增，从 1 开始）。
    ///
    /// 同一 msg_id 多次发送（如流式分段）必须使用不同 msg_seq；首次发送
    /// 返回 1，下一次返回 2，依此类推。msg_id 为 None（纯主动消息）时
    /// caller 不应调用此方法（主动消息无 msg_id）。
    pub async fn next_msg_seq(&self, msg_id: &str) -> u32 {
        let mut map = self.msg_seq_map.lock().await;
        if let Some(seq) = map.get_mut(msg_id) {
            *seq += 1;
            *seq
        } else {
            map.put(msg_id.to_string(), 1);
            1
        }
    }

    pub async fn remember_message_reply_context(&self, msg_id: &str, event_id: Option<&str>) {
        if msg_id.is_empty() {
            return;
        }
        let event_id = event_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mut contexts = self.passive_reply_contexts.lock().await;
        contexts.put(
            msg_id.to_string(),
            PassiveReplyContext::Message {
                msg_id: msg_id.to_string(),
                event_id,
            },
        );
    }

    pub async fn remember_event_reply_context(&self, key: &str, event_id: Option<&str>) {
        let Some(event_id) = event_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        if key.is_empty() {
            return;
        }
        let mut contexts = self.passive_reply_contexts.lock().await;
        contexts.put(
            key.to_string(),
            PassiveReplyContext::Event {
                event_id: event_id.to_string(),
            },
        );
    }

    async fn passive_reply_context(&self, reply_to_id: &str) -> Option<PassiveReplyContext> {
        let mut contexts = self.passive_reply_contexts.lock().await;
        contexts.get(reply_to_id).cloned()
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

    /// Get the WebSocket gateway URL and sharding/session-start metadata.
    ///
    /// GET /gateway/bot -> { "url": "wss://...", "shards": 1, ... }
    pub async fn get_gateway_bot_info(&self) -> Result<GatewayBotInfo> {
        self.qq_request(reqwest::Method::GET, "/gateway/bot", None)
            .await
    }

    /// 在 body 上注入被动回复上下文。
    ///
    /// 普通消息回复使用 `msg_id` + 自增 `msg_seq`（per-msg_id 单调递增），
    /// 并在 gateway 顶层 `id` 可用时同时带 `event_id`；按钮等事件回调
    /// 只带 `event_id`。主动消息（reply_to_id=None）不注入任何被动字段。
    async fn inject_passive_reply_meta(
        &self,
        body: &mut serde_json::Value,
        reply_to_id: Option<&str>,
    ) {
        let Some(reply_to_id) = reply_to_id.filter(|id| !id.is_empty()) else {
            return;
        };

        match self.passive_reply_context(reply_to_id).await {
            Some(PassiveReplyContext::Message { msg_id, event_id }) => {
                body["msg_id"] = serde_json::Value::String(msg_id.clone());
                let seq = self.next_msg_seq(&msg_id).await;
                body["msg_seq"] = serde_json::Value::Number(seq.into());
                if let Some(event_id) = event_id {
                    body["event_id"] = serde_json::Value::String(event_id);
                }
            }
            Some(PassiveReplyContext::Event { event_id }) => {
                body["event_id"] = serde_json::Value::String(event_id);
            }
            None => {
                body["msg_id"] = serde_json::Value::String(reply_to_id.to_string());
                let seq = self.next_msg_seq(reply_to_id).await;
                body["msg_seq"] = serde_json::Value::Number(seq.into());
            }
        }
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
        self.inject_passive_reply_meta(&mut body, msg_id).await;
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
        self.inject_passive_reply_meta(&mut body, msg_id).await;
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
        self.inject_passive_reply_meta(&mut body, msg_id).await;
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
        self.inject_passive_reply_meta(&mut body, msg_id).await;
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    // ── Rich Media (V2 two-step) ────────────────────────────────────
    //
    // QQ Bot V2 富媒体走两步流程（参考
    // <https://bot.q.qq.com/wiki/develop/api-v2/server-inter/message/send-receive/rich-media.html>）：
    //
    // 1. POST `/v2/groups/{group_openid}/files` 或 `/v2/users/{openid}/files`
    //    body: `{ file_type, url, srv_send_msg: false }` → 返回 `file_info` + `ttl`
    // 2. 用 `msg_type=7 + media: { file_info }` 调常规 send_*_message 即可
    //
    // **重要约束**：file_info 上传到群文件接口 → 只能群里发；上传到 c2c
    // 文件接口 → 只能 c2c 里发，不可跨用。

    /// File type codes for `/v2/groups/.../files` and `/v2/users/.../files`.
    /// 1=image (png/jpg)、2=video (mp4)、3=voice (silk)、4=file（暂未开放）
    pub const FILE_TYPE_IMAGE: u32 = 1;
    pub const FILE_TYPE_VIDEO: u32 = 2;
    pub const FILE_TYPE_VOICE: u32 = 3;

    /// Upload a media URL for use in a subsequent group message.
    ///
    /// Returns the `file_info` token to embed in `media` of the send message.
    /// `file_info` 有 ttl，过期需要重新上传。
    pub async fn post_group_files(
        &self,
        group_openid: &str,
        file_type: u32,
        url: &str,
    ) -> Result<String> {
        let path = format!("/v2/groups/{}/files", group_openid);
        let body = serde_json::json!({
            "file_type": file_type,
            "url": url,
            "srv_send_msg": false,
        });
        let resp: serde_json::Value = self
            .qq_request(reqwest::Method::POST, &path, Some(body))
            .await?;
        resp.get("file_info")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("post_group_files response missing file_info"))
    }

    /// Upload a media URL for use in a subsequent C2C message.
    pub async fn post_c2c_files(&self, openid: &str, file_type: u32, url: &str) -> Result<String> {
        let path = format!("/v2/users/{}/files", openid);
        let body = serde_json::json!({
            "file_type": file_type,
            "url": url,
            "srv_send_msg": false,
        });
        let resp: serde_json::Value = self
            .qq_request(reqwest::Method::POST, &path, Some(body))
            .await?;
        resp.get("file_info")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("post_c2c_files response missing file_info"))
    }

    /// Send a media message (`msg_type=7`) to a group with a previously uploaded
    /// `file_info` and optional caption (`content`).
    pub async fn send_group_media(
        &self,
        group_openid: &str,
        file_info: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/v2/groups/{}/messages", group_openid);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 7,
            "media": { "file_info": file_info },
        });
        self.inject_passive_reply_meta(&mut body, msg_id).await;
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a media message (`msg_type=7`) to a C2C user.
    pub async fn send_c2c_media(
        &self,
        openid: &str,
        file_info: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = format!("/v2/users/{}/messages", openid);
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 7,
            "media": { "file_info": file_info },
        });
        self.inject_passive_reply_meta(&mut body, msg_id).await;
        self.qq_request(reqwest::Method::POST, &path, Some(body))
            .await
    }

    /// Send a message with inline keyboard buttons (msg_type 2 = markdown with keyboard).
    /// Only supported for [`QqChatScope::C2c`] / [`QqChatScope::Group`].
    pub async fn send_message_with_keyboard(
        &self,
        scope: QqChatScope<'_>,
        content: &str,
        keyboard: serde_json::Value,
        msg_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let path = match scope {
            QqChatScope::C2c(openid) => format!("/v2/users/{}/messages", openid),
            QqChatScope::Group(gid) => format!("/v2/groups/{}/messages", gid),
            _ => {
                return Err(anyhow!("Keyboard buttons not supported for {:?}", scope));
            }
        };
        let mut body = serde_json::json!({
            "content": content,
            "msg_type": 2,
            "keyboard": keyboard,
        });
        self.inject_passive_reply_meta(&mut body, msg_id).await;
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

    /// Acknowledge an INTERACTION_CREATE (button click) event.
    ///
    /// POST /interactions/{interaction_id} with `code: 0` (success). Tencent
    /// expects the ack within 5 s; without it the gateway considers the
    /// callback failed and may resend the same `INTERACTION_CREATE`. Other
    /// codes (1=async, 2=invalid, 3=hidden, 4=quiet failure, 5=visible
    /// failure) are reserved for richer flows we don't yet need.
    pub async fn ack_interaction(&self, interaction_id: &str) -> Result<()> {
        let path = format!("/interactions/{}", interaction_id);
        let _: serde_json::Value = self
            .qq_request(
                reqwest::Method::PUT,
                &path,
                Some(serde_json::json!({"code": 0})),
            )
            .await?;
        Ok(())
    }

    /// Download a QQ Bot CDN attachment URL to disk. URLs in inbound
    /// `attachments[]` are short-lived signed Tencent CDN links that
    /// resolve **without** auth headers — the signature is in the URL
    /// itself. We still pin the host to `*.qq.com` / `*.qpic.cn` etc. so
    /// a poisoned gateway payload can't redirect us, and run the SSRF
    /// classifier on the resolved IP.
    pub async fn download_cdn_to_disk(
        &self,
        url: &str,
        dest: &std::path::Path,
        cap_bytes: u64,
    ) -> Result<u64> {
        let parsed_url = url::Url::parse(url).map_err(|e| anyhow!("Invalid QQ URL: {}", e))?;
        let host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("QQ URL has no host: {}", url))?;
        let host_ok = host.ends_with(".qq.com")
            || host.ends_with(".qpic.cn")
            || host.ends_with(".gtimg.cn")
            || host.ends_with(".myqcloud.com");
        if !host_ok {
            return Err(anyhow!("Refusing to download from non-QQ host: {}", host));
        }
        crate::security::ssrf::check_url(url, crate::security::ssrf::SsrfPolicy::Default, &[])
            .await
            .map_err(|e| anyhow!("QQ CDN URL blocked: {}", e))?;

        let builder = self.client.get(url);
        crate::channel::inbound_media_common::stream_to_disk(builder, dest, cap_bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_msg_seq_increments_per_msg_id() {
        let auth = Arc::new(QqBotAuth::new("appid", "secret"));
        let api = QqBotApi::new(auth);

        // 同一 msg_id 多次调返回单调递增
        assert_eq!(api.next_msg_seq("msg-1").await, 1);
        assert_eq!(api.next_msg_seq("msg-1").await, 2);
        assert_eq!(api.next_msg_seq("msg-1").await, 3);

        // 不同 msg_id 各自独立
        assert_eq!(api.next_msg_seq("msg-2").await, 1);
        assert_eq!(api.next_msg_seq("msg-2").await, 2);
        assert_eq!(api.next_msg_seq("msg-1").await, 4);
    }

    #[tokio::test]
    async fn passive_reply_meta_uses_message_context_event_id() {
        let auth = Arc::new(QqBotAuth::new("appid", "secret"));
        let api = QqBotApi::new(auth);
        api.remember_message_reply_context("msg-1", Some("event-1"))
            .await;

        let mut body = serde_json::json!({"content": "hello"});
        api.inject_passive_reply_meta(&mut body, Some("msg-1"))
            .await;

        assert_eq!(body["msg_id"], "msg-1");
        assert_eq!(body["msg_seq"], 1);
        assert_eq!(body["event_id"], "event-1");
    }

    #[tokio::test]
    async fn passive_reply_meta_uses_event_context_without_msg_seq() {
        let auth = Arc::new(QqBotAuth::new("appid", "secret"));
        let api = QqBotApi::new(auth);
        api.remember_event_reply_context("interaction-1", Some("event-1"))
            .await;

        let mut body = serde_json::json!({"content": "hello"});
        api.inject_passive_reply_meta(&mut body, Some("interaction-1"))
            .await;

        assert_eq!(body["event_id"], "event-1");
        assert!(body.get("msg_id").is_none());
        assert!(body.get("msg_seq").is_none());
    }

    #[tokio::test]
    async fn passive_reply_meta_falls_back_to_msg_id() {
        let auth = Arc::new(QqBotAuth::new("appid", "secret"));
        let api = QqBotApi::new(auth);

        let mut body = serde_json::json!({"content": "hello"});
        api.inject_passive_reply_meta(&mut body, Some("msg-1"))
            .await;

        assert_eq!(body["msg_id"], "msg-1");
        assert_eq!(body["msg_seq"], 1);
        assert!(body.get("event_id").is_none());
    }
}

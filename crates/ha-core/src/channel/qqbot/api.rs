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
}

/// Response from GET /gateway.
#[derive(Debug, serde::Deserialize)]
struct GatewayResponse {
    url: String,
}

impl QqBotApi {
    /// Create a new QQ Bot API client.
    pub fn new(auth: Arc<QqBotAuth>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            auth,
            base_url: "https://api.sgroup.qq.com".to_string(),
            msg_seq_map: Mutex::new(LruCache::new(
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

    /// Get the WebSocket gateway URL.
    ///
    /// GET /gateway -> { "url": "wss://..." }
    pub async fn get_gateway_url(&self) -> Result<String> {
        let resp: GatewayResponse = self
            .qq_request(reqwest::Method::GET, "/gateway", None)
            .await?;
        Ok(resp.url)
    }

    /// 在 body 上注入 `msg_id` + 自增 `msg_seq`（per-msg_id 单调递增）。
    /// 主动消息（msg_id=None）不注入 seq——主动消息无 msg_id 关联，QQ
    /// 服务端不要求 seq；空 msg_id 也用同款逻辑（产生 seq=1 的兜底）。
    async fn inject_passive_reply_meta(&self, body: &mut serde_json::Value, msg_id: Option<&str>) {
        if let Some(id) = msg_id {
            body["msg_id"] = serde_json::Value::String(id.to_string());
            let seq = self.next_msg_seq(id).await;
            body["msg_seq"] = serde_json::Value::Number(seq.into());
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
}

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

pub const DEFAULT_WECHAT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub const DEFAULT_WECHAT_CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";
pub const DEFAULT_WECHAT_BOT_TYPE: &str = "3";
const ILINK_APP_ID: &str = "bot";
const DEFAULT_LONG_POLL_TIMEOUT_MS: u64 = 35_000;

pub const MESSAGE_TYPE_USER: i32 = 1;
pub const MESSAGE_TYPE_BOT: i32 = 2;
pub const MESSAGE_ITEM_TYPE_TEXT: i32 = 1;
pub const MESSAGE_ITEM_TYPE_IMAGE: i32 = 2;
pub const MESSAGE_ITEM_TYPE_VOICE: i32 = 3;
pub const MESSAGE_ITEM_TYPE_FILE: i32 = 4;
pub const MESSAGE_ITEM_TYPE_VIDEO: i32 = 5;
pub const TYPING_STATUS_TYPING: i32 = 1;
pub const TYPING_STATUS_CANCEL: i32 = 2;

#[derive(Clone)]
pub struct WeChatApi {
    client: Client,
    base_url: String,
    token: Option<String>,
}

impl WeChatApi {
    pub fn new(token: Option<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            token,
        }
    }

    pub async fn fetch_login_qr(bot_type: &str) -> Result<QrCodeResponse> {
        let raw = Self::get_text(
            DEFAULT_WECHAT_BASE_URL,
            &format!(
                "ilink/bot/get_bot_qrcode?bot_type={}",
                urlencoding::encode(bot_type)
            ),
            5_000,
        )
        .await?;
        serde_json::from_str(&raw).context("Failed to decode WeChat QR code response")
    }

    pub async fn poll_login_status(
        base_url: &str,
        qrcode: &str,
        timeout_ms: u64,
    ) -> Result<QrStatusResponse> {
        match Self::get_text(
            base_url,
            &format!(
                "ilink/bot/get_qrcode_status?qrcode={}",
                urlencoding::encode(qrcode)
            ),
            timeout_ms,
        )
        .await
        {
            Ok(raw) => {
                serde_json::from_str(&raw).context("Failed to decode WeChat QR status response")
            }
            Err(err) if is_timeout(&err) => Ok(QrStatusResponse {
                status: Some("wait".to_string()),
                ..Default::default()
            }),
            Err(err) => Err(err),
        }
    }

    pub async fn get_updates(
        &self,
        get_updates_buf: &str,
        timeout_ms: Option<u64>,
    ) -> Result<GetUpdatesResponse> {
        match self
            .post_json(
                "ilink/bot/getupdates",
                json!({
                    "get_updates_buf": get_updates_buf,
                    "base_info": base_info(),
                }),
                timeout_ms.unwrap_or(DEFAULT_LONG_POLL_TIMEOUT_MS),
            )
            .await
        {
            Ok(raw) => {
                serde_json::from_str(&raw).context("Failed to decode WeChat getUpdates response")
            }
            Err(err) if is_timeout(&err) => Ok(GetUpdatesResponse {
                ret: Some(0),
                get_updates_buf: Some(get_updates_buf.to_string()),
                ..Default::default()
            }),
            Err(err) => Err(err),
        }
    }

    pub async fn send_text(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: Option<&str>,
    ) -> Result<String> {
        self.send_message_items(
            to_user_id,
            vec![json!({
                "type": MESSAGE_ITEM_TYPE_TEXT,
                "text_item": { "text": text }
            })],
            context_token,
        )
        .await
    }

    pub async fn send_message_items(
        &self,
        to_user_id: &str,
        item_list: Vec<serde_json::Value>,
        context_token: Option<&str>,
    ) -> Result<String> {
        let message_id = format!("hope-agent-wechat-{}", Uuid::new_v4().simple());
        self.post_json(
            "ilink/bot/sendmessage",
            json!({
                "msg": {
                    "from_user_id": "",
                    "to_user_id": to_user_id,
                    "client_id": message_id,
                    "message_type": MESSAGE_TYPE_BOT,
                    "message_state": 2,
                    "item_list": item_list,
                    "context_token": context_token,
                },
                "base_info": base_info(),
            }),
            15_000,
        )
        .await?;
        Ok(message_id)
    }

    pub async fn get_upload_url(&self, body: serde_json::Value) -> Result<GetUploadUrlResponse> {
        let raw = self
            .post_json("ilink/bot/getuploadurl", body, 15_000)
            .await?;
        serde_json::from_str(&raw).context("Failed to decode WeChat getUploadUrl response")
    }

    pub async fn get_config(
        &self,
        ilink_user_id: &str,
        context_token: Option<&str>,
    ) -> Result<GetConfigResponse> {
        let raw = self
            .post_json(
                "ilink/bot/getconfig",
                json!({
                    "ilink_user_id": ilink_user_id,
                    "context_token": context_token,
                    "base_info": base_info(),
                }),
                10_000,
            )
            .await?;
        serde_json::from_str(&raw).context("Failed to decode WeChat getConfig response")
    }

    pub async fn send_typing(
        &self,
        ilink_user_id: &str,
        typing_ticket: &str,
        status: i32,
    ) -> Result<()> {
        self.post_json(
            "ilink/bot/sendtyping",
            json!({
                "ilink_user_id": ilink_user_id,
                "typing_ticket": typing_ticket,
                "status": status,
                "base_info": base_info(),
            }),
            10_000,
        )
        .await?;
        Ok(())
    }

    pub async fn probe(&self) -> Result<()> {
        let response = self.get_updates("", Some(1_000)).await?;
        let ret = response.ret.unwrap_or(0);
        let errcode = response.errcode.unwrap_or(0);
        if ret == 0 && errcode == 0 {
            return Ok(());
        }

        Err(anyhow::anyhow!(
            "WeChat probe failed: ret={} errcode={} errmsg={}",
            ret,
            errcode,
            response
                .errmsg
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    }

    async fn get_text(base_url: &str, endpoint: &str, timeout_ms: u64) -> Result<String> {
        let client = Client::new();
        let url = join_url(base_url, endpoint)?;
        let response = client
            .get(url.clone())
            .header("iLink-App-Id", ILINK_APP_ID)
            .header("iLink-App-ClientVersion", client_version().to_string())
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .send()
            .await
            .with_context(|| format!("WeChat GET request failed: {}", endpoint))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read WeChat GET response body")?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "WeChat GET {} failed with {}: {}",
                endpoint,
                status,
                crate::truncate_utf8(&body, 300)
            ));
        }

        Ok(body)
    }

    async fn post_json(
        &self,
        endpoint: &str,
        body: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<String> {
        let url = join_url(&self.base_url, endpoint)?;
        let mut request = self
            .client
            .post(url.clone())
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("X-WECHAT-UIN", random_wechat_uin())
            .header("iLink-App-Id", ILINK_APP_ID)
            .header("iLink-App-ClientVersion", client_version().to_string())
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .json(&body);

        if let Some(token) = self
            .token
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("WeChat POST request failed: {}", endpoint))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .context("Failed to read WeChat POST response body")?;

        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "WeChat POST {} failed with {}: {}",
                endpoint,
                status,
                crate::truncate_utf8(&response_text, 300)
            ));
        }

        Ok(response_text)
    }
}

fn join_url(base_url: &str, endpoint: &str) -> Result<String> {
    let base = if base_url.ends_with('/') {
        base_url.to_string()
    } else {
        format!("{}/", base_url)
    };
    let url = url::Url::parse(&base)
        .with_context(|| format!("Invalid WeChat base URL: {}", base_url))?
        .join(endpoint)
        .with_context(|| format!("Invalid WeChat endpoint: {}", endpoint))?;
    Ok(url.to_string())
}

fn client_version() -> u32 {
    let parts: Vec<u32> = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect();
    let major = *parts.first().unwrap_or(&0);
    let minor = *parts.get(1).unwrap_or(&0);
    let patch = *parts.get(2).unwrap_or(&0);
    ((major & 0xff) << 16) | ((minor & 0xff) << 8) | (patch & 0xff)
}

fn base_info() -> serde_json::Value {
    json!({
        "channel_version": format!("hope-agent/{}", env!("CARGO_PKG_VERSION")),
    })
}

fn random_wechat_uin() -> String {
    use base64::Engine as _;
    let value = rand::random::<u32>().to_string();
    base64::engine::general_purpose::STANDARD.encode(value)
}

fn is_timeout(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .map(|req_err| req_err.is_timeout())
            .unwrap_or(false)
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QrCodeResponse {
    pub qrcode: String,
    #[serde(default)]
    pub qrcode_img_content: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct QrStatusResponse {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub ilink_bot_id: Option<String>,
    #[serde(default)]
    pub baseurl: Option<String>,
    #[serde(default)]
    pub ilink_user_id: Option<String>,
    #[serde(default)]
    pub redirect_host: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetUpdatesResponse {
    #[serde(default)]
    pub ret: Option<i32>,
    #[serde(default)]
    pub errcode: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub msgs: Vec<WeChatMessage>,
    #[serde(default)]
    pub get_updates_buf: Option<String>,
    #[serde(default)]
    pub longpolling_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeChatMessage {
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub message_id: Option<i64>,
    #[serde(default)]
    pub from_user_id: Option<String>,
    #[serde(default)]
    pub to_user_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub create_time_ms: Option<i64>,
    #[serde(default)]
    pub update_time_ms: Option<i64>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub message_type: Option<i32>,
    #[serde(default)]
    pub message_state: Option<i32>,
    #[serde(default)]
    pub item_list: Vec<MessageItem>,
    #[serde(default)]
    pub context_token: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageItem {
    #[serde(rename = "type", default)]
    pub item_type: i32,
    #[serde(default)]
    pub text_item: Option<TextItem>,
    #[serde(default)]
    pub image_item: Option<ImageItem>,
    #[serde(default)]
    pub voice_item: Option<VoiceItem>,
    #[serde(default)]
    pub file_item: Option<FileItem>,
    #[serde(default)]
    pub video_item: Option<VideoItem>,
    #[serde(default)]
    pub ref_msg: Option<RefMessage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextItem {
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CdnMedia {
    #[serde(default)]
    pub encrypt_query_param: Option<String>,
    #[serde(default)]
    pub aes_key: Option<String>,
    #[serde(default)]
    pub encrypt_type: Option<i32>,
    #[serde(default)]
    pub full_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageItem {
    #[serde(default)]
    pub media: Option<CdnMedia>,
    #[serde(default)]
    pub aeskey: Option<String>,
    #[serde(default)]
    pub mid_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceItem {
    #[serde(default)]
    pub media: Option<CdnMedia>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileItem {
    #[serde(default)]
    pub media: Option<CdnMedia>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub len: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoItem {
    #[serde(default)]
    pub media: Option<CdnMedia>,
    #[serde(default)]
    pub video_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefMessage {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message_item: Option<Box<MessageItem>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetUploadUrlResponse {
    #[serde(default)]
    pub upload_param: Option<String>,
    #[serde(default)]
    pub thumb_upload_param: Option<String>,
    #[serde(default)]
    pub upload_full_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GetConfigResponse {
    #[serde(default)]
    pub ret: Option<i32>,
    #[serde(default)]
    pub errmsg: Option<String>,
    #[serde(default)]
    pub typing_ticket: Option<String>,
}

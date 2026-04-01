use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::api::{WeChatApi, DEFAULT_WECHAT_BOT_TYPE, DEFAULT_WECHAT_BASE_URL};

const LOGIN_TTL: Duration = Duration::minutes(8); // Official: 480 seconds

#[derive(Debug, Clone)]
struct ActiveLogin {
    qrcode: String,
    qrcode_url: String,
    started_at: DateTime<Utc>,
    current_api_base_url: String,
    refresh_count: u32,
}

static ACTIVE_LOGINS: Lazy<Mutex<HashMap<String, ActiveLogin>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeChatLoginStart {
    pub qrcode_url: Option<String>,
    pub session_key: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeChatLoginWait {
    pub connected: bool,
    pub status: Option<String>,
    pub bot_token: Option<String>,
    pub remote_account_id: Option<String>,
    pub base_url: Option<String>,
    pub user_id: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qrcode_url: Option<String>,
}

pub async fn start_login(account_id: Option<&str>) -> Result<WeChatLoginStart> {
    let session_key = account_id
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    {
        let logins = ACTIVE_LOGINS.lock().await;
        if let Some(existing) = logins.get(&session_key) {
            if Utc::now() - existing.started_at < LOGIN_TTL {
                return Ok(WeChatLoginStart {
                    qrcode_url: Some(existing.qrcode_url.clone()),
                    session_key,
                    message: "二维码已就绪，请使用微信扫码。".to_string(),
                });
            }
        }
    }

    let qr_response = WeChatApi::fetch_login_qr(DEFAULT_WECHAT_BOT_TYPE).await?;

    // qrcode_img_content is a URL to be encoded into a QR code (not an image itself).
    // The frontend renders the QR code using qrcode.react.
    let qr_url = qr_response.qrcode_img_content.clone();

    app_info!(
        "channel",
        "wechat::login",
        "QR login started: qrcode='{}', url='{}'",
        crate::truncate_utf8(&qr_response.qrcode, 60),
        crate::truncate_utf8(&qr_url, 120)
    );

    let login = ActiveLogin {
        qrcode: qr_response.qrcode,
        qrcode_url: qr_url.clone(),
        started_at: Utc::now(),
        current_api_base_url: DEFAULT_WECHAT_BASE_URL.to_string(),
        refresh_count: 0,
    };

    ACTIVE_LOGINS.lock().await.insert(session_key.clone(), login);

    Ok(WeChatLoginStart {
        qrcode_url: Some(qr_url),
        session_key,
        message: "请使用微信扫描二维码，并在手机上确认登录。".to_string(),
    })
}

pub async fn wait_login(session_key: &str, timeout_ms: Option<u64>) -> Result<WeChatLoginWait> {
    let login = {
        let logins = ACTIVE_LOGINS.lock().await;
        logins.get(session_key).cloned()
    };

    let Some(login) = login else {
        return Ok(WeChatLoginWait {
            connected: false,
            status: Some("idle".to_string()),
            bot_token: None,
            remote_account_id: None,
            base_url: None,
            user_id: None,
            message: "当前没有进行中的微信登录。".to_string(),
            qrcode_url: None,
        });
    };

    if Utc::now() - login.started_at >= LOGIN_TTL {
        ACTIVE_LOGINS.lock().await.remove(session_key);
        return Ok(WeChatLoginWait {
            connected: false,
            status: Some("expired".to_string()),
            bot_token: None,
            remote_account_id: None,
            base_url: None,
            user_id: None,
            message: "二维码已过期，请重新生成。".to_string(),
            qrcode_url: None,
        });
    }

    let timeout = timeout_ms.unwrap_or(3_000).clamp(1_000, 5_000);
    let status = WeChatApi::poll_login_status(&login.current_api_base_url, &login.qrcode, timeout)
        .await?;

    match status.status.as_deref().unwrap_or("wait") {
        "confirmed" => {
            ACTIVE_LOGINS.lock().await.remove(session_key);
            Ok(WeChatLoginWait {
                connected: true,
                status: Some("connected".to_string()),
                bot_token: status.bot_token,
                remote_account_id: status.ilink_bot_id,
                base_url: status.baseurl,
                user_id: status.ilink_user_id,
                message: "微信连接成功。".to_string(),
                qrcode_url: None,
            })
        }
        "scaned" => Ok(WeChatLoginWait {
            connected: false,
            status: Some("scanned".to_string()),
            bot_token: None,
            remote_account_id: None,
            base_url: None,
            user_id: None,
            message: "已扫码，请在微信里确认授权。".to_string(),
            qrcode_url: None,
        }),
        "scaned_but_redirect" => {
            if let Some(redirect_host) = status.redirect_host.as_deref().filter(|v| !v.is_empty()) {
                if let Some(active) = ACTIVE_LOGINS.lock().await.get_mut(session_key) {
                    active.current_api_base_url = format!("https://{}", redirect_host);
                }
            }
            Ok(WeChatLoginWait {
                connected: false,
                status: Some("wait".to_string()),
                bot_token: None,
                remote_account_id: None,
                base_url: None,
                user_id: None,
                message: "微信正在切换登录节点，请稍候。".to_string(),
                qrcode_url: None,
            })
        }
        "expired" => {
            // Auto-refresh QR code (max 3 times)
            let should_refresh = {
                let logins = ACTIVE_LOGINS.lock().await;
                logins
                    .get(session_key)
                    .map(|l| l.refresh_count < 3)
                    .unwrap_or(false)
            };

            if should_refresh {
                if let Ok(qr_response) =
                    WeChatApi::fetch_login_qr(DEFAULT_WECHAT_BOT_TYPE).await
                {
                    let qr_url = qr_response.qrcode_img_content.clone();
                    let refresh_count = {
                        let mut logins = ACTIVE_LOGINS.lock().await;
                        if let Some(active) = logins.get_mut(session_key) {
                            active.qrcode = qr_response.qrcode;
                            active.qrcode_url = qr_url.clone();
                            active.started_at = Utc::now();
                            active.refresh_count += 1;
                            active.current_api_base_url =
                                DEFAULT_WECHAT_BASE_URL.to_string();
                            active.refresh_count
                        } else {
                            0
                        }
                    };
                    return Ok(WeChatLoginWait {
                        connected: false,
                        status: Some("refreshed".to_string()),
                        bot_token: None,
                        remote_account_id: None,
                        base_url: None,
                        user_id: None,
                        message: format!(
                            "二维码已过期，已自动刷新 ({}/3)。",
                            refresh_count
                        ),
                        qrcode_url: Some(qr_url),
                    });
                }
            }

            ACTIVE_LOGINS.lock().await.remove(session_key);
            Ok(WeChatLoginWait {
                connected: false,
                status: Some("expired".to_string()),
                bot_token: None,
                remote_account_id: None,
                base_url: None,
                user_id: None,
                message: "二维码已过期，请重新生成。".to_string(),
                qrcode_url: None,
            })
        }
        _ => Ok(WeChatLoginWait {
            connected: false,
            status: Some("wait".to_string()),
            bot_token: None,
            remote_account_id: None,
            base_url: None,
            user_id: None,
            message: "等待扫码中。".to_string(),
            qrcode_url: None,
        }),
    }
}

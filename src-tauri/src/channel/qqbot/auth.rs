use anyhow::{anyhow, Result};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Cached access token with expiration time.
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// QQ Bot authentication manager.
///
/// Handles access token acquisition via the QQ Bot App Access Token API.
/// The token expires every 2 hours; we refresh 5 minutes before expiry.
pub struct QqBotAuth {
    app_id: String,
    client_secret: String,
    client: reqwest::Client,
    cached_token: Mutex<Option<CachedToken>>,
}

/// Response from the getAppAccessToken API.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: String,
}

impl QqBotAuth {
    /// Create a new auth manager with the given appId and clientSecret.
    pub fn new(app_id: &str, client_secret: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            client_secret: client_secret.to_string(),
            client: reqwest::Client::new(),
            cached_token: Mutex::new(None),
        }
    }

    /// Get the app_id (needed for gateway identify and API headers).
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Get a valid access token.
    ///
    /// Returns a cached token if it's still valid (with a 5-minute safety buffer).
    /// Otherwise, requests a new token from the QQ Bot API.
    pub async fn get_token(&self) -> Result<String> {
        // Check cache first
        {
            let cached = self.cached_token.lock().await;
            if let Some(ref ct) = *cached {
                let buffer = std::time::Duration::from_secs(5 * 60);
                if ct.expires_at > Instant::now() + buffer {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Request new token
        let resp = self
            .client
            .post("https://bots.qq.com/app/getAppAccessToken")
            .json(&serde_json::json!({
                "appId": self.app_id,
                "clientSecret": self.client_secret,
            }))
            .send()
            .await
            .map_err(|e| anyhow!("Failed to request QQ Bot token: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read QQ Bot token response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "QQ Bot token request failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse QQ Bot token response: {}", e))?;

        let expire_secs: u64 = token_resp.expires_in.parse().unwrap_or(7200);

        // Cache the token
        {
            let mut cached = self.cached_token.lock().await;
            *cached = Some(CachedToken {
                token: token_resp.access_token.clone(),
                expires_at: Instant::now() + std::time::Duration::from_secs(expire_secs),
            });
        }

        app_info!(
            "channel",
            "qqbot::auth",
            "Acquired new access token (expires in {}s)",
            expire_secs
        );

        Ok(token_resp.access_token)
    }
}

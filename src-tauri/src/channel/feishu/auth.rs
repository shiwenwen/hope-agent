use anyhow::{anyhow, Result};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Cached tenant access token with expiration time.
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Feishu authentication manager.
///
/// Handles tenant_access_token acquisition and automatic refresh.
/// The token expires every 2 hours; we refresh 5 minutes before expiry.
pub struct FeishuAuth {
    app_id: String,
    app_secret: String,
    base_url: String,
    client: reqwest::Client,
    cached_token: Mutex<Option<CachedToken>>,
}

/// Response from the tenant_access_token API.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    code: i64,
    msg: String,
    tenant_access_token: Option<String>,
    expire: Option<u64>,
}

impl FeishuAuth {
    /// Create a new auth manager.
    ///
    /// `domain` can be:
    /// - `"feishu"` or empty → `https://open.feishu.cn`
    /// - `"lark"` → `https://open.larksuite.com`
    /// - A URL starting with `"http"` → used as-is (custom private deployment)
    pub fn new(app_id: &str, app_secret: &str, domain: &str) -> Self {
        let base_url = resolve_base_url(domain);
        Self {
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            base_url,
            client: reqwest::Client::new(),
            cached_token: Mutex::new(None),
        }
    }

    /// Get the base URL for API requests.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get a valid tenant access token.
    ///
    /// Returns a cached token if it's still valid (with a 5-minute safety buffer).
    /// Otherwise, requests a new token from the Feishu API.
    pub async fn get_token(&self) -> Result<String> {
        // Check cache first
        {
            let cached = self.cached_token.lock().await;
            if let Some(ref ct) = *cached {
                // Return cached if not expiring within 5 minutes
                let buffer = std::time::Duration::from_secs(5 * 60);
                if ct.expires_at > Instant::now() + buffer {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Request new token
        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal/",
            self.base_url
        );

        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await
            .map_err(|e| anyhow!("Failed to request Feishu token: {}", e))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu token response: {}", e))?;

        if !status.is_success() {
            return Err(anyhow!(
                "Feishu token request failed with HTTP {}: {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| anyhow!("Failed to parse Feishu token response: {}", e))?;

        if token_resp.code != 0 {
            return Err(anyhow!(
                "Feishu token error (code={}): {}",
                token_resp.code,
                token_resp.msg
            ));
        }

        let token = token_resp
            .tenant_access_token
            .ok_or_else(|| anyhow!("Feishu token response missing tenant_access_token"))?;

        let expire_secs = token_resp.expire.unwrap_or(7200);

        // Cache the token
        {
            let mut cached = self.cached_token.lock().await;
            *cached = Some(CachedToken {
                token: token.clone(),
                expires_at: Instant::now() + std::time::Duration::from_secs(expire_secs),
            });
        }

        app_info!(
            "channel",
            "feishu:auth",
            "Acquired new tenant_access_token (expires in {}s)",
            expire_secs
        );

        Ok(token)
    }
}

/// Resolve domain string to base URL.
fn resolve_base_url(domain: &str) -> String {
    let trimmed = domain.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("feishu") {
        "https://open.feishu.cn".to_string()
    } else if trimmed.eq_ignore_ascii_case("lark") {
        "https://open.larksuite.com".to_string()
    } else if trimmed.starts_with("http") {
        // Custom private deployment URL — use as-is, strip trailing slash
        trimmed.trim_end_matches('/').to_string()
    } else {
        // Fallback to feishu
        "https://open.feishu.cn".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_base_url_feishu() {
        assert_eq!(resolve_base_url("feishu"), "https://open.feishu.cn");
        assert_eq!(resolve_base_url("Feishu"), "https://open.feishu.cn");
        assert_eq!(resolve_base_url(""), "https://open.feishu.cn");
        assert_eq!(resolve_base_url("  "), "https://open.feishu.cn");
    }

    #[test]
    fn test_resolve_base_url_lark() {
        assert_eq!(resolve_base_url("lark"), "https://open.larksuite.com");
        assert_eq!(resolve_base_url("Lark"), "https://open.larksuite.com");
    }

    #[test]
    fn test_resolve_base_url_custom() {
        assert_eq!(
            resolve_base_url("https://my-feishu.example.com"),
            "https://my-feishu.example.com"
        );
        assert_eq!(
            resolve_base_url("https://my-feishu.example.com/"),
            "https://my-feishu.example.com"
        );
        assert_eq!(
            resolve_base_url("http://localhost:8080"),
            "http://localhost:8080"
        );
    }

    #[test]
    fn test_resolve_base_url_unknown_fallback() {
        assert_eq!(resolve_base_url("unknown"), "https://open.feishu.cn");
    }
}

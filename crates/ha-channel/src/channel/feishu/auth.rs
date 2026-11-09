use anyhow::{anyhow, Result};
use serde::Deserialize;
use tokio::sync::RwLock;
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
///
/// `cached_token` is an `RwLock` so concurrent tool calls all share a read
/// lock on the cache hit path (the common case — refresh runs ~once every
/// 7140s). The refresh path takes a write lock, which serializes refreshes
/// (singleflight) and briefly blocks readers — a fair trade-off given how
/// rare refresh is.
pub struct FeishuAuth {
    app_id: String,
    app_secret: String,
    base_url: String,
    client: reqwest::Client,
    cached_token: RwLock<Option<CachedToken>>,
}

/// Response from the tenant_access_token API.
#[derive(Deserialize)]
struct TokenResponse {
    code: i64,
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
            cached_token: RwLock::new(None),
        }
    }

    /// Get the base URL for API requests.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build the request body for the WS endpoint handshake. Lives here (not
    /// in `api.rs`) so the app secret never leaves this module.
    pub(crate) fn ws_endpoint_credentials(&self) -> serde_json::Value {
        serde_json::json!({
            "AppID": self.app_id,
            "AppSecret": self.app_secret,
        })
    }

    /// Get a valid tenant access token.
    ///
    /// Returns a cached token if it's still valid (with a 5-minute safety buffer).
    /// Otherwise, requests a new token from the Feishu API.
    pub async fn get_token(&self) -> Result<String> {
        let buffer = std::time::Duration::from_secs(5 * 60);

        // Hot path: read lock — concurrent tool calls all share the read lock
        // and return the cached token without serializing on a mutex.
        {
            let guard = self.cached_token.read().await;
            if let Some(ct) = guard.as_ref() {
                if ct.expires_at > Instant::now() + buffer {
                    return Ok(ct.token.clone());
                }
            }
        }

        // Refresh path: write lock + double-check (singleflight). Holding the
        // write lock across the HTTP request briefly blocks readers, but it
        // ensures concurrent refreshers serialize and the second-onwards waiter
        // sees the now-fresh token via the double-check.
        let mut guard = self.cached_token.write().await;
        if let Some(ct) = guard.as_ref() {
            if ct.expires_at > Instant::now() + buffer {
                return Ok(ct.token.clone());
            }
        }

        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
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
            .map_err(|e| anyhow!("Failed to request Feishu token: {}", e.without_url()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| anyhow!("Failed to read Feishu token response: {}", e.without_url()))?;

        if !status.is_success() {
            return Err(anyhow!("Feishu token request failed with HTTP {}", status));
        }

        // An upstream error (including a JSON type error) may reflect a
        // credential in arbitrary text. Preserve status/code, never its body.
        let token_resp: TokenResponse = serde_json::from_str(&body)
            .map_err(|_| anyhow!("Failed to parse Feishu token response"))?;

        if token_resp.code != 0 {
            return Err(anyhow!("Feishu token error (code={})", token_resp.code));
        }

        let token = token_resp
            .tenant_access_token
            .ok_or_else(|| anyhow!("Feishu token response missing tenant_access_token"))?;

        let expire_secs = token_resp.expire.unwrap_or(7200);
        *guard = Some(CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + std::time::Duration::from_secs(expire_secs),
        });

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

    #[tokio::test]
    async fn token_errors_do_not_reflect_credentials() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let canary = format!("synthetic-secret-canary.{}", "x".repeat(4096));
        for (status, body) in [
            (
                401,
                serde_json::json!({"msg": canary, "tenant_access_token": canary}),
            ),
            (
                200,
                serde_json::json!({"code": 99991663, "msg": canary, "tenant_access_token": canary}),
            ),
            (
                200,
                serde_json::json!({"code": canary, "tenant_access_token": canary}),
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;
            let auth = FeishuAuth::new("cli_synthetic", &canary, &server.uri());
            let error = auth.get_token().await.unwrap_err().to_string();
            assert!(
                !error.contains("synthetic-secret-canary"),
                "credential reflection"
            );
            assert!(auth.cached_token.read().await.is_none());
        }
    }

    #[tokio::test]
    async fn long_credentials_and_tokens_remain_account_scoped() {
        use crate::channel::feishu::api::FeishuApi;
        use ha_core::channel::types::ChannelAccountConfig;
        use std::sync::Arc;
        use wiremock::matchers::{body_partial_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let mut apis = Vec::new();
        for id in ["a", "b"] {
            let secret = format!("synthetic-config-{id}.{}", id.repeat(4096));
            let token = format!("eyJhbGciOiJub25lIn0.{}.synthetic-{id}", id.repeat(4096));
            let config: ChannelAccountConfig = serde_json::from_value(serde_json::json!({
                "id": id, "channelId": "feishu", "label": id,
                "credentials": {"appId": id, "appSecret": secret, "domain": server.uri()}
            }))
            .unwrap();
            let config: ChannelAccountConfig =
                serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
            assert_eq!(
                config.credentials["appSecret"].as_str(),
                Some(secret.as_str())
            );
            Mock::given(method("POST"))
                .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
                .and(body_partial_json(
                    serde_json::json!({"app_id": id, "app_secret": secret}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": 0, "msg": "ok", "tenant_access_token": token, "expire": 7200
                })))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/open-apis/bot/v3/info/"))
                .and(header("authorization", format!("Bearer {token}").as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "code": 0, "msg": "ok", "bot": {"app_name": id, "open_id": id}
                })))
                .expect(2)
                .mount(&server)
                .await;
            apis.push((
                id,
                FeishuApi::new(Arc::new(FeishuAuth::new(
                    config.credentials["appId"].as_str().unwrap(),
                    config.credentials["appSecret"].as_str().unwrap(),
                    config.credentials["domain"].as_str().unwrap(),
                ))),
            ));
        }
        // Interleave two accounts and then exercise each account's cache.
        for _ in 0..2 {
            for (id, api) in &apis {
                assert_eq!(api.get_bot_info().await.unwrap().open_id, *id);
            }
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn concurrent_get_token_singleflights_refresh() {
        use std::sync::Arc;
        use std::time::Duration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(50))
                    .set_body_json(serde_json::json!({
                        "code": 0,
                        "msg": "ok",
                        "tenant_access_token": "t-fake-token",
                        "expire": 7200
                    })),
            )
            .mount(&server)
            .await;

        let auth = Arc::new(FeishuAuth::new("cli_test", "secret_test", &server.uri()));
        let tasks = (0..10).map(|_| {
            let auth = auth.clone();
            tokio::spawn(async move { auth.get_token().await })
        });

        for result in futures_util::future::join_all(tasks).await {
            assert_eq!(result.unwrap().unwrap(), "t-fake-token");
        }

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
    }

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

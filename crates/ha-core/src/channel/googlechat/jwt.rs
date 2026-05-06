//! Google Chat webhook JWT 验签。
//!
//! Google Chat 在向 bot endpoint 发请求时附带 `Authorization: Bearer <JWT>`，
//! 由 `chat@system.gserviceaccount.com` 签发。bot 必须验：
//!
//! 1. 签名（用 Google 公钥 PEM）
//! 2. `iss == "chat@system.gserviceaccount.com"`
//! 3. `aud == <bot 的 project number>`（用户在凭据中提供）
//! 4. 未过期（`exp`）
//!
//! 任何一项不通过，请求都视为伪造（任何人能 reach 隧道 URL 的人都可以伪造
//! `MESSAGE` / `CARD_CLICKED` 事件 → 劫持 LLM 会话或触发任意工具审批）。
//!
//! - 公钥源：<https://www.googleapis.com/service_accounts/v1/metadata/x509/chat@system.gserviceaccount.com>
//! - 算法：RS256
//! - 公钥缓存：1 小时（与 Google 文档约定一致）

use anyhow::{anyhow, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const ISSUER: &str = "chat@system.gserviceaccount.com";
const PUBLIC_KEYS_URL: &str =
    "https://www.googleapis.com/service_accounts/v1/metadata/x509/chat@system.gserviceaccount.com";
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// JWT claims we care about (rest ignored).
#[derive(Debug, serde::Deserialize)]
struct GoogleChatClaims {
    iss: String,
    aud: String,
    // exp / iat 由 jsonwebtoken 自动校验（Validation::set_audience + leeway）
}

/// Cached public keys, keyed by `kid` (X.509 PEM string per kid).
#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, String>,
    fetched_at: Option<Instant>,
}

/// Public-key fetcher with TTL cache. Shared singleton.
pub struct GoogleChatJwtVerifier {
    http_client: reqwest::Client,
    cache: Arc<Mutex<KeyCache>>,
}

impl GoogleChatJwtVerifier {
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http_client,
            cache: Arc::new(Mutex::new(KeyCache::default())),
        }
    }

    /// 从 `Authorization: Bearer <JWT>` 头中提取 token 并完整验签。
    ///
    /// `expected_aud` 是 bot 的 Google Cloud project number（用户在凭据中
    /// 配置）。验签失败返回 Err，调用方应回 401/403。
    pub async fn verify_authz_header(
        &self,
        authz_header: Option<&str>,
        expected_aud: &str,
    ) -> Result<()> {
        let header_value =
            authz_header.ok_or_else(|| anyhow!("Missing Authorization header"))?;
        let token = header_value
            .strip_prefix("Bearer ")
            .ok_or_else(|| anyhow!("Authorization header is not Bearer"))?;
        self.verify_token(token, expected_aud).await
    }

    /// 直接对一个 raw JWT token 字符串验签。
    pub async fn verify_token(&self, token: &str, expected_aud: &str) -> Result<()> {
        // 1. 读 header 拿 kid
        let header = decode_header(token).map_err(|e| anyhow!("Invalid JWT header: {}", e))?;
        let kid = header
            .kid
            .ok_or_else(|| anyhow!("JWT header missing 'kid'"))?;

        // 2. 拿对应 PEM 公钥（必要时刷新缓存）
        let pem = self.get_key(&kid).await?;

        // 3. 构造 Validation，指定 aud / iss / 算法
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[expected_aud]);
        validation.set_issuer(&[ISSUER]);
        // exp 默认会被校验；不设置 leeway（jsonwebtoken 默认 0）

        let key = DecodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| anyhow!("Failed to parse Google public key PEM: {}", e))?;

        let token_data = decode::<GoogleChatClaims>(token, &key, &validation)
            .map_err(|e| anyhow!("JWT verification failed: {}", e))?;

        // 4. 双保险（Validation 已经 set_issuer + set_audience，但显式校验
        //    防止 future 版本默认行为漂移）
        if token_data.claims.iss != ISSUER {
            return Err(anyhow!(
                "JWT iss mismatch: expected '{}', got '{}'",
                ISSUER,
                token_data.claims.iss
            ));
        }
        if token_data.claims.aud != expected_aud {
            return Err(anyhow!(
                "JWT aud mismatch: expected '{}', got '{}'",
                expected_aud,
                token_data.claims.aud
            ));
        }

        Ok(())
    }

    /// 取 kid 对应的 PEM 公钥；过期 / miss 时全量拉取一次。
    async fn get_key(&self, kid: &str) -> Result<String> {
        // 先看缓存
        {
            let cache = self.cache.lock().await;
            if let Some(fetched_at) = cache.fetched_at {
                if fetched_at.elapsed() < CACHE_TTL {
                    if let Some(pem) = cache.keys.get(kid) {
                        return Ok(pem.clone());
                    }
                }
            }
        }

        // 重新拉
        self.refresh_keys().await?;

        let cache = self.cache.lock().await;
        cache
            .keys
            .get(kid)
            .cloned()
            .ok_or_else(|| anyhow!("Google public key not found for kid '{}'", kid))
    }

    async fn refresh_keys(&self) -> Result<()> {
        let resp = self
            .http_client
            .get(PUBLIC_KEYS_URL)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to fetch Google public keys: {}", e))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "Google public keys endpoint returned {}",
                resp.status().as_u16()
            ));
        }
        // Response 形如 `{"<kid>": "-----BEGIN CERTIFICATE-----..."}`
        let body: HashMap<String, String> = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse Google public keys JSON: {}", e))?;
        let mut cache = self.cache.lock().await;
        cache.keys = body;
        cache.fetched_at = Some(Instant::now());
        Ok(())
    }
}

impl Default for GoogleChatJwtVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_authz_returns_err() {
        let v = GoogleChatJwtVerifier::new();
        let r = v.verify_authz_header(None, "12345").await;
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("Missing Authorization"));
    }

    #[tokio::test]
    async fn non_bearer_returns_err() {
        let v = GoogleChatJwtVerifier::new();
        let r = v
            .verify_authz_header(Some("Basic abc=="), "12345")
            .await;
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("not Bearer"));
    }

    #[tokio::test]
    async fn malformed_token_returns_err() {
        let v = GoogleChatJwtVerifier::new();
        let r = v.verify_token("not-a-jwt", "12345").await;
        assert!(r.is_err());
    }
}

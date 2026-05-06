use anyhow::{anyhow, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// Google Chat **app authentication**（service account JWT-bearer）必须用
/// `chat.bot` scope；`chat.messages.create` / `chat.spaces.readonly` 是
/// **user authentication** 专用 scope（OAuth 用户授权流），不能用于本仓
/// 的 service account 流程，否则 spaces.list / messages.create 会
/// `PERMISSION_DENIED`。
///
/// 参考：<https://developers.google.com/workspace/chat/authenticate-authorize-chat-app>
const CHAT_SCOPE: &str = "https://www.googleapis.com/auth/chat.bot";
/// Buffer before expiry to refresh the token (5 minutes).
const EXPIRY_BUFFER_SECS: u64 = 300;

/// Parsed service account credentials.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccountCredentials {
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

/// JWT claims for Google service account auth.
#[derive(Debug, Serialize)]
struct JwtClaims {
    iss: String,
    scope: String,
    aud: String,
    exp: u64,
    iat: u64,
}

/// Cached access token.
struct CachedToken {
    access_token: String,
    expires_at: u64,
}

/// Google Chat service account authenticator.
///
/// Manages JWT-based authentication for Google Chat API access.
/// Caches access tokens and refreshes them before expiry.
pub struct GoogleChatAuth {
    credentials: ServiceAccountCredentials,
    http_client: reqwest::Client,
    cached_token: Arc<Mutex<Option<CachedToken>>>,
}

/// Token exchange response from Google's token endpoint.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[allow(dead_code)]
    token_type: Option<String>,
}

impl GoogleChatAuth {
    /// Create a new authenticator from raw service account JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        let credentials: ServiceAccountCredentials = serde_json::from_str(json_str)
            .map_err(|e| anyhow!("Failed to parse service account JSON: {}", e))?;

        if credentials.client_email.is_empty() {
            return Err(anyhow!("Service account missing client_email"));
        }
        if credentials.private_key.is_empty() {
            return Err(anyhow!("Service account missing private_key"));
        }
        if credentials.token_uri.is_empty() {
            return Err(anyhow!("Service account missing token_uri"));
        }

        let http_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Ok(Self {
            credentials,
            http_client,
            cached_token: Arc::new(Mutex::new(None)),
        })
    }

    /// Get a valid access token, refreshing if needed.
    pub async fn get_access_token(&self) -> Result<String> {
        // Check cached token
        {
            let cache = self.cached_token.lock().await;
            if let Some(ref cached) = *cache {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if now < cached.expires_at.saturating_sub(EXPIRY_BUFFER_SECS) {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        // Generate new JWT and exchange for access token
        let token_response = self.exchange_jwt_for_token().await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expires_at = now + token_response.expires_in;

        // Cache the token
        {
            let mut cache = self.cached_token.lock().await;
            *cache = Some(CachedToken {
                access_token: token_response.access_token.clone(),
                expires_at,
            });
        }

        Ok(token_response.access_token)
    }

    /// Generate a signed JWT and exchange it for an access token.
    async fn exchange_jwt_for_token(&self) -> Result<TokenResponse> {
        let jwt = self.create_signed_jwt()?;

        let resp = self
            .http_client
            .post(&self.credentials.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| anyhow!("Token exchange request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Token exchange failed ({}): {}",
                status,
                crate::truncate_utf8(&body, 512)
            ));
        }

        resp.json::<TokenResponse>()
            .await
            .map_err(|e| anyhow!("Failed to parse token response: {}", e))
    }

    /// Create a signed JWT for the service account.
    fn create_signed_jwt(&self) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let claims = JwtClaims {
            iss: self.credentials.client_email.clone(),
            scope: CHAT_SCOPE.to_string(),
            aud: self.credentials.token_uri.clone(),
            exp: now + 3600,
            iat: now,
        };

        let header = Header::new(Algorithm::RS256);

        let encoding_key = EncodingKey::from_rsa_pem(self.credentials.private_key.as_bytes())
            .map_err(|e| anyhow!("Failed to parse RSA private key: {}", e))?;

        encode(&header, &claims, &encoding_key).map_err(|e| anyhow!("Failed to sign JWT: {}", e))
    }

    /// Get the client email (useful for display/logging).
    pub fn client_email(&self) -> &str {
        &self.credentials.client_email
    }
}

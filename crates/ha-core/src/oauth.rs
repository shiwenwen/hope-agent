use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_PORT: u16 = 1455;
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPES: &str = "openid profile email offline_access";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// JWT payload for extracting chatgpt_account_id
#[derive(Deserialize)]
struct JwtPayload {
    #[serde(default)]
    exp: Option<u64>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<JwtAuth>,
}

#[derive(Deserialize)]
struct JwtAuth {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

/// Extract chatgpt_account_id from JWT access token (public for use in agent/lib)
pub fn extract_account_id(token: &str) -> Option<String> {
    decode_jwt_payload(token)?
        .auth
        .and_then(|a| a.chatgpt_account_id)
}

fn decode_jwt_payload(token: &str) -> Option<JwtPayload> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&payload_bytes).ok()
}

fn access_token_expires_at_ms(token: &str) -> Option<u64> {
    decode_jwt_payload(token)?.exp?.checked_mul(1_000)
}

/// Margin before absolute expiry to treat the token as expired.
/// Consumers refreshing on demand (HTTP round-trip + JWT parse typically < 1s)
/// don't need a large window — 30s is enough to absorb clock skew and network
/// jitter without burning the last minute of a still-valid token.
const REFRESH_MARGIN_MS: u64 = 30_000;
const EVAL_TOKEN_EXPIRY_MARGIN_MS: u64 = 60_000;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodexEvaluationToken {
    pub access_token: String,
    pub account_id: String,
    pub expires_at_ms: u64,
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn effective_token_expiry(token: &TokenData) -> Option<u64> {
    match (
        token.expires_at,
        access_token_expires_at_ms(&token.access_token),
    ) {
        (Some(stored), Some(jwt)) => Some(stored.min(jwt)),
        (Some(stored), None) => Some(stored),
        (None, Some(jwt)) => Some(jwt),
        (None, None) => None,
    }
}

fn expiry_covers(expires_at_ms: u64, required_validity_secs: u64) -> bool {
    let required_ms = required_validity_secs
        .saturating_mul(1_000)
        .saturating_add(EVAL_TOKEN_EXPIRY_MARGIN_MS);
    now_unix_ms().saturating_add(required_ms) < expires_at_ms
}

fn evaluation_token_from_parts(
    access_token: String,
    account_id: String,
    expires_at_ms: u64,
    required_validity_secs: u64,
) -> Result<CodexEvaluationToken> {
    if account_id.trim().is_empty() {
        bail!("Codex OAuth account id is unavailable (authentication). Please sign in again.");
    }
    if !expiry_covers(expires_at_ms, required_validity_secs) {
        let available_secs = expires_at_ms
            .saturating_sub(now_unix_ms())
            .saturating_sub(EVAL_TOKEN_EXPIRY_MARGIN_MS)
            / 1_000;
        bail!(
            "Codex OAuth access token cannot cover the requested evaluation duration of {required_validity_secs}s (safe remaining lifetime: {available_secs}s). Reduce the evaluation duration or sign in again."
        );
    }
    Ok(CodexEvaluationToken {
        access_token,
        account_id,
        expires_at_ms,
    })
}

/// Resolve a Codex access token whose remaining lifetime covers the entire
/// local evaluation campaign. Refresh happens only in the owner process; the
/// isolated runtime still receives no refresh token or OAuth file.
pub(crate) async fn load_codex_token_for_evaluation(
    required_validity_secs: u64,
) -> Result<CodexEvaluationToken> {
    if crate::eval_context::model_eval_mode_enabled() {
        bail!("Codex evaluation credentials must be resolved by the owner process");
    }

    let cached = match crate::get_codex_token_cache() {
        Some(cache) => cache.lock().await.clone(),
        None => None,
    };
    if let Some((access_token, account_id)) = cached {
        if let Some(expires_at_ms) = access_token_expires_at_ms(&access_token) {
            if expiry_covers(expires_at_ms, required_validity_secs) {
                return evaluation_token_from_parts(
                    access_token,
                    account_id,
                    expires_at_ms,
                    required_validity_secs,
                );
            }
        }
    }

    let disk = load_token()
        .context("reading Codex OAuth for local evaluation")?
        .ok_or_else(|| {
            anyhow!("Codex OAuth token not found (authentication). Please sign in again.")
        })?;
    if let Some(expires_at_ms) = effective_token_expiry(&disk) {
        if expiry_covers(expires_at_ms, required_validity_secs) {
            let account_id = disk
                .account_id
                .clone()
                .or_else(|| extract_account_id(&disk.access_token))
                .unwrap_or_default();
            return evaluation_token_from_parts(
                disk.access_token,
                account_id,
                expires_at_ms,
                required_validity_secs,
            );
        }
    }

    let refresh = disk.refresh_token.as_deref().ok_or_else(|| {
        anyhow!(
            "Codex OAuth token does not cover the requested evaluation duration and has no refresh token (authentication). Please sign in again."
        )
    })?;
    let refreshed = refresh_access_token(refresh).await.map_err(|error| {
        anyhow!(
            "Codex OAuth refresh failed before local evaluation (authentication): {error}. Please sign in again."
        )
    })?;
    let expires_at_ms = effective_token_expiry(&refreshed)
        .ok_or_else(|| anyhow!("refreshed Codex OAuth token has no verifiable expiration time"))?;
    let account_id = refreshed
        .account_id
        .clone()
        .or_else(|| extract_account_id(&refreshed.access_token))
        .unwrap_or_default();
    evaluation_token_from_parts(
        refreshed.access_token,
        account_id,
        expires_at_ms,
        required_validity_secs,
    )
}

/// 已按 `model-eval-codex-oauth.v1` schema 编码、可交给隔离评测运行时的 Codex 凭据。
///
/// **`secret` 是明文 JSON，内含 raw access token**——
/// `config::encode_model_eval_codex_secret` 只做 schema 封装与有效性校验，
/// 不加密。别把这个类型当成脱敏边界。
///
/// 它收窄的是**装配职责**而非凭据可见性：特征 crate 不再需要认识
/// [`CodexEvaluationToken`]、不再自己决定编码方式与摘要口径，因此
/// [`CodexEvaluationToken`] / [`load_codex_token_for_evaluation`] /
/// `config::encode_model_eval_codex_secret` 三者得以保持 `pub(crate)`。
/// 凭据本身照样流向 `ha-eval-runtime` 并进隔离运行时的 config——那条路径的
/// 把关点在 `evaluation/provider_resolution.rs`（见 CODEOWNERS）。
///
/// 刻意不 derive `Debug`：避免被顺手 `{:?}` 进日志（AGENTS「凭据禁入日志」红线）。
pub struct CodexEvaluationSecret {
    /// `config::encode_model_eval_codex_secret` 的产物，直接进隔离 `config.json`。
    /// **含 raw access token 明文**，见类型文档。
    pub secret: String,
    /// 账号标识的摘要，只用于「同一 Provider 不得混用不同凭据」的一致性校验。
    pub account_id_digest: String,
}

/// 为本地真实模型评测铸一份 Codex 凭据。`required_validity_secs` 必须覆盖整个
/// campaign——隔离运行时拿不到 refresh token，中途过期无法续期。
///
/// 与迁移前 `ha-eval-runtime` 侧自己拼 token → encode → digest 三步**逐位等价**，
/// 只是把三步收进 kernel，让那三个 `pub(crate)` 项不必为跨 crate 调用放开。
pub async fn mint_codex_evaluation_secret(
    required_validity_secs: u64,
) -> Result<CodexEvaluationSecret> {
    let token = load_codex_token_for_evaluation(required_validity_secs).await?;
    let secret = crate::config::encode_model_eval_codex_secret(
        &token.access_token,
        &token.account_id,
        token.expires_at_ms,
    )?;
    let account_id_digest = ha_eval_spec::digest_serializable(&token.account_id)?;
    Ok(CodexEvaluationSecret {
        secret,
        account_id_digest,
    })
}

/// Check if token is expired (or within `REFRESH_MARGIN_MS` of expiry).
pub fn is_token_expired(token: &TokenData) -> bool {
    match token.expires_at {
        Some(expires_at) => {
            let now_ms = now_unix_ms();
            now_ms + REFRESH_MARGIN_MS >= expires_at
        }
        None => false, // If no expiry info, assume valid
    }
}

/// Generates a random code_verifier for PKCE (43-128 chars, URL-safe)
fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Derives code_challenge from code_verifier using S256
fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

/// Returns the path to the auth token file
fn auth_file_path() -> Result<PathBuf> {
    crate::paths::auth_path()
}

/// Save token to disk
pub fn save_token(token: &TokenData) -> Result<()> {
    let path = auth_file_path()?;
    let json = serde_json::to_string_pretty(token)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load token from disk
pub fn load_token() -> Result<Option<TokenData>> {
    let path = auth_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(path)?;
    let token: TokenData = serde_json::from_str(&json)?;
    Ok(Some(token))
}

/// Delete saved token
pub fn clear_token() -> Result<()> {
    let path = auth_file_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    // SessionEnd(logout) hook (observation, app-global). Per-session fan-out is
    // a later refinement; this fires one representative event. (The matching
    // auth_success Notification fires from the OAuth-flow completion site, not
    // here and not from `save_token` — which also runs on token refresh.)
    crate::hooks::fire_session_end("", "logout");
    Ok(())
}

/// Start the OAuth PKCE flow:
/// 1. Generate PKCE verifier + challenge
/// 2. Start a local HTTP server to receive the callback
/// 3. Open the browser to the authorization URL
/// 4. Wait for the callback with the authorization code
/// 5. Exchange the code for tokens
/// Returns the TokenData on success.
pub async fn start_oauth_flow(auth_result: Arc<Mutex<Option<Result<TokenData>>>>) -> Result<()> {
    start_oauth_flow_with_auth_url(auth_result, true)
        .await
        .map(|_| ())
}

/// Start the OAuth PKCE flow and return the authorization URL.
///
/// `open_browser=false` is used by terminal/headless entry points so they can
/// print the URL and let the operator open it manually.
pub async fn start_oauth_flow_with_auth_url(
    auth_result: Arc<Mutex<Option<Result<TokenData>>>>,
    open_browser: bool,
) -> Result<String> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = uuid::Uuid::new_v4().to_string();

    // Build the authorization URL with PKCE challenge.
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=hope-agent",
        AUTH_URL,
        CLIENT_ID,
        urlencoding(REDIRECT_URI),
        urlencoding(SCOPES),
        code_challenge,
        state,
    );

    // Start local HTTP server in a blocking task
    let state_clone = state.clone();
    let verifier_clone = code_verifier.clone();
    let result_clone = auth_result.clone();

    tokio::task::spawn_blocking(move || {
        match run_callback_server(&state_clone, &verifier_clone) {
            Ok(token) => {
                // Save token to disk
                if let Err(e) = save_token(&token) {
                    app_error!("auth", "oauth", "Failed to save token: {}", e);
                }
                // Notification(auth_success): fire from the OAuth-flow
                // completion site (a fresh login), NOT from `save_token` —
                // which also runs on silent token refresh. App-global
                // representative event.
                crate::hooks::fire_notification("", "auth_success", "Signed in successfully.");
                let mut lock = result_clone.blocking_lock();
                *lock = Some(Ok(token));
            }
            Err(e) => {
                let mut lock = result_clone.blocking_lock();
                *lock = Some(Err(e));
            }
        }
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    if open_browser {
        open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;
    }

    Ok(auth_url)
}

/// Runs a tiny HTTP server that listens for the OAuth callback
fn run_callback_server(expected_state: &str, code_verifier: &str) -> Result<TokenData> {
    // Bind to loopback only so the callback is never exposed externally.
    let addr = format!("127.0.0.1:{}", REDIRECT_PORT);
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow!("Failed to start callback server on {}: {}", addr, e))?;

    app_info!(
        "auth",
        "oauth",
        "OAuth callback server listening on {}",
        addr
    );

    // Wait for the callback request (with a timeout)
    let timeout = std::time::Duration::from_secs(300); // 5 minutes
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(anyhow!("OAuth callback timed out after 5 minutes"));
        }

        // Receive with a short timeout so we can check the overall timeout
        match server.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(Some(request)) => {
                let url = request.url().to_string();

                // Check if this is our callback
                if !url.starts_with("/auth/callback") {
                    let response =
                        tiny_http::Response::from_string("Not found").with_status_code(404);
                    let _ = request.respond(response);
                    continue;
                }

                // Parse query parameters
                let query = url.split('?').nth(1).unwrap_or("");
                let params: std::collections::HashMap<String, String> = query
                    .split('&')
                    .filter_map(|pair| {
                        let mut parts = pair.splitn(2, '=');
                        let key = parts.next()?.to_string();
                        let value = parts.next().unwrap_or("").to_string();
                        Some((key, value))
                    })
                    .collect();

                // Verify state
                let received_state = params.get("state").map(|s| s.as_str()).unwrap_or("");
                if received_state != expected_state {
                    let response = tiny_http::Response::from_string("Invalid state parameter")
                        .with_status_code(400);
                    let _ = request.respond(response);
                    return Err(anyhow!("OAuth state mismatch"));
                }

                // Check for errors
                if let Some(error) = params.get("error") {
                    let desc = params.get("error_description").cloned().unwrap_or_default();
                    let response =
                        tiny_http::Response::from_string(format!("Error: {} - {}", error, desc))
                            .with_status_code(400);
                    let _ = request.respond(response);
                    return Err(anyhow!("OAuth error: {} - {}", error, desc));
                }

                // Get the authorization code
                let code = params
                    .get("code")
                    .ok_or_else(|| anyhow!("No authorization code in callback"))?
                    .clone();

                // Send success response to browser
                let html = r#"<!DOCTYPE html>
<html><head><title>Hope Agent</title>
<style>
  body { font-family: -apple-system, sans-serif; display: flex; justify-content: center;
         align-items: center; height: 100vh; margin: 0; background: #0f0f0f; color: #e0e0e0; }
  .container { text-align: center; }
  h1 { font-size: 24px; margin-bottom: 8px; }
  p { color: #888; }
</style></head>
<body><div class="container">
  <h1>✅ 登录成功</h1>
  <p>你可以关闭此页面，回到 Hope Agent 应用。</p>
</div></body></html>"#;

                let response = tiny_http::Response::from_string(html).with_header(
                    "Content-Type: text/html; charset=utf-8"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                );
                let _ = request.respond(response);

                // Exchange the code for tokens
                return exchange_code_for_token(&code, code_verifier);
            }
            Ok(None) => continue,
            Err(_) => continue,
        }
    }
}

/// Exchange authorization code for access token
fn exchange_code_for_token(code: &str, code_verifier: &str) -> Result<TokenData> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", code_verifier),
        ])
        .send()
        .map_err(|e| anyhow!("Token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("Token exchange failed ({}): {}", status, body));
    }

    let mut token: TokenData = response
        .json()
        .map_err(|e| anyhow!("Failed to parse token response: {}", e))?;

    // Extract account_id from JWT and compute absolute expiry
    token.account_id = extract_account_id(&token.access_token);
    if let Some(expires_in) = token.expires_in {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        token.expires_at = Some(now_ms + expires_in * 1000);
    }

    Ok(token)
}

/// Load a usable Codex OAuth token, refreshing the on-disk token if expired.
/// Returns `(access_token, account_id)`.
///
/// An isolated local model-evaluation server has no OAuth files or refresh
/// token by design. In that mode the short-lived pair seeded during process
/// bootstrap is the only accepted source. This common entry point keeps main
/// chat, Side Query, automation, and nested Agent paths on the same boundary.
///
/// Error messages embed the literal `authentication` so
/// [`crate::failover::classify_error`] returns `FailoverReason::Auth`.
pub async fn load_fresh_codex_token() -> Result<(String, String)> {
    if crate::eval_context::model_eval_mode_enabled() {
        let cached = match crate::get_codex_token_cache() {
            Some(cache) => cache.lock().await.clone(),
            None => None,
        };
        return cached.ok_or_else(|| {
            anyhow!(
                "Codex OAuth token is unavailable in the isolated evaluation process (authentication)"
            )
        });
    }

    let disk = load_token()
        .map_err(|e| {
            anyhow!(
                "Codex OAuth token read failed (authentication): {}. Please sign in again.",
                e
            )
        })?
        .ok_or_else(|| {
            anyhow!("Codex OAuth token not found (authentication). Please sign in again.")
        })?;

    if !is_token_expired(&disk) {
        let account_id = disk
            .account_id
            .clone()
            .or_else(|| extract_account_id(&disk.access_token))
            .unwrap_or_default();
        return Ok((disk.access_token, account_id));
    }

    let refresh = disk.refresh_token.as_ref().ok_or_else(|| {
        anyhow!(
            "Codex OAuth token expired and no refresh token on disk (authentication). \
             Please sign in again."
        )
    })?;

    let new_token = refresh_access_token(refresh).await.map_err(|e| {
        anyhow!(
            "Codex OAuth token refresh failed (authentication): {}. Please sign in again.",
            e
        )
    })?;

    let account_id = new_token
        .account_id
        .clone()
        .or_else(|| extract_account_id(&new_token.access_token))
        .unwrap_or_default();

    app_info!(
        "auth",
        "codex",
        "On-demand Codex token refresh via load_fresh_codex_token"
    );

    Ok((new_token.access_token, account_id))
}

/// Return a fresh `(access_token, account_id)` pair when the caller's
/// in-memory `current_access_token` is stale or missing relative to disk:
/// - `None` — nothing changed; caller keeps what it has.
/// - `Some((token, account_id))` — either (a) disk has a newer token (another
///   process refreshed it), or (b) disk token was near expiry and we just
///   refreshed it here.
///
/// Called before each Codex chat request so a barely-expired token doesn't
/// produce a visible 401 mid-conversation.
pub async fn ensure_fresh_codex_token(current_access_token: &str) -> Option<(String, String)> {
    let disk = tokio::task::spawn_blocking(|| load_token().ok().flatten())
        .await
        .ok()
        .flatten()?;

    if !is_token_expired(&disk) {
        if disk.access_token == current_access_token {
            return None;
        }
        let account_id = disk
            .account_id
            .clone()
            .or_else(|| extract_account_id(&disk.access_token))?;
        return Some((disk.access_token, account_id));
    }

    let refresh = disk.refresh_token.as_ref()?;
    match refresh_access_token(refresh).await {
        Ok(new_token) => {
            let account_id = new_token
                .account_id
                .clone()
                .or_else(|| extract_account_id(&new_token.access_token))
                .unwrap_or_default();
            app_info!("auth", "codex", "Pre-refreshed Codex OAuth token");
            Some((new_token.access_token, account_id))
        }
        Err(e) => {
            app_warn!(
                "auth",
                "codex",
                "Proactive Codex token refresh failed: {}",
                e
            );
            None
        }
    }
}

/// Refresh access token using refresh_token
pub async fn refresh_access_token(refresh_token: &str) -> Result<TokenData> {
    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|e| anyhow!("Token refresh request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Token refresh failed ({}): {}", status, body));
    }

    let mut token: TokenData = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse refresh token response: {}", e))?;

    // OAuth providers may rotate the refresh token or omit it when the
    // existing token remains valid. Never erase the still-usable token when
    // the response omits a replacement.
    if token.refresh_token.is_none() {
        token.refresh_token = Some(refresh_token.to_string());
    }

    // Extract account_id from new JWT and compute absolute expiry
    token.account_id = extract_account_id(&token.access_token);
    if let Some(expires_in) = token.expires_in {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        token.expires_at = Some(now_ms + expires_in * 1000);
    }

    save_token(&token)?;
    Ok(token)
}

/// Simple URL encoding for known safe strings
fn urlencoding(s: &str) -> String {
    s.replace(' ', "+").replace(':', "%3A").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failover::{classify_error, FailoverReason};

    // Error messages from load_fresh_codex_token must classify as Auth.

    #[test]
    fn token_not_found_message_classifies_as_auth() {
        let err = anyhow!("Codex OAuth token not found (authentication). Please sign in again.");
        assert_eq!(classify_error(&err.to_string()), FailoverReason::Auth);
    }

    #[test]
    fn token_expired_without_refresh_classifies_as_auth() {
        let err = anyhow!(
            "Codex OAuth token expired and no refresh token on disk (authentication). \
             Please sign in again."
        );
        assert_eq!(classify_error(&err.to_string()), FailoverReason::Auth);
    }

    #[test]
    fn token_refresh_failure_classifies_as_auth() {
        let inner = "401 Unauthorized from auth.openai.com";
        let err = anyhow!(
            "Codex OAuth token refresh failed (authentication): {}. Please sign in again.",
            inner
        );
        assert_eq!(classify_error(&err.to_string()), FailoverReason::Auth);
    }

    #[test]
    fn evaluation_token_must_cover_the_whole_campaign() {
        let expires_at_ms = now_unix_ms() + 120_000;
        assert!(evaluation_token_from_parts(
            "codex-evaluation-access-token".to_string(),
            "account-eval".to_string(),
            expires_at_ms,
            30,
        )
        .is_ok());

        let error = evaluation_token_from_parts(
            "codex-evaluation-access-token".to_string(),
            "account-eval".to_string(),
            expires_at_ms,
            90,
        )
        .err()
        .expect("expiry margin must reject a campaign longer than the token lifetime");
        assert!(error
            .to_string()
            .contains("cannot cover the requested evaluation duration"));
    }

    #[test]
    fn effective_expiry_uses_the_earliest_verifiable_boundary() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"exp":100}"#);
        let token = TokenData {
            access_token: format!("header.{payload}.signature"),
            refresh_token: None,
            expires_in: None,
            token_type: None,
            account_id: None,
            expires_at: Some(123_000),
        };
        assert_eq!(effective_token_expiry(&token), Some(100_000));
    }

    // **AGENTS.md「凭据禁入日志」红线**：`CodexEvaluationSecret` 明确不
    // derive `Debug` 以防被 `{:?}` 顺手打到日志（见类型 doc）。这里用
    // `static_assertions::assert_not_impl_all!` 做 compile-time 守卫——
    // 若将来给它 derive 了 `Debug`，本文件会**编译失败**（不是运行时失败，
    // 是直接 build 断），比人肉 review 兜底更可靠。
    // 具体到 `mint_codex_evaluation_secret_bails_inside_model_eval_mode`：
    // 由于返回 Ok 侧不 Debug，测试也不能用 `.expect_err(...)`——手动 match
    // 是本 assertion 的必然副产物，且合意（防凭据顺手 pretty-print 到日志）。
    static_assertions::assert_not_impl_all!(CodexEvaluationSecret: std::fmt::Debug);

    /// **`mint_codex_evaluation_secret` 有效性契约的下界守卫**：
    /// `load_codex_token_for_evaluation` 在检测到 `HA_MODEL_EVAL_MODE=1`
    /// 时**必须立刻 bail**——`oauth.rs:145-147` 明说「Codex evaluation
    /// credentials must be resolved by the owner process」，即隔离评测运行
    /// 时里的 Codex 凭据面**永远不能**在这条路径产生。
    ///
    /// 若这条早退失守（比如未来重构不小心把它挪到 owner check 之下），隔
    /// 离运行时会试图读 owner 侧的 OAuth 文件、甚至走 refresh token——
    /// 直接违反「HA_MODEL_EVAL_MODE 下不得读凭据」的隔离契约。这里就地拦。
    #[tokio::test]
    async fn mint_codex_evaluation_secret_bails_inside_model_eval_mode() {
        let result = crate::test_support::with_env_vars_async(
            &[("HA_MODEL_EVAL_MODE", std::path::Path::new("1"))],
            || async { mint_codex_evaluation_secret(60).await },
        )
        .await;
        // Deliberately avoid `.expect_err(...)`：`CodexEvaluationSecret` 刻意
        // 不 impl Debug（见上方 `assert_not_impl_all!`），Result 的 `Ok` 侧无
        // Debug 就用不了 `expect_err`——手动 match 是刚才那条守卫的必然副产物，
        // 且合意：mint 若真的返回了 Ok，我们希望 test 干净地 panic 而不是把
        // 凭据 pretty-print 到测试日志。
        match result {
            Err(err) => assert!(
                err.to_string()
                    .contains("must be resolved by the owner process"),
                "错误信息应指出 mint 的边界，实际：{err}"
            ),
            Ok(_) => panic!("隔离评测运行时里 mint 必须立刻 bail，绝不去读 owner OAuth 文件"),
        }
    }
}

use anyhow::{anyhow, Result};
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
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let payload: JwtPayload = serde_json::from_slice(&payload_bytes).ok()?;
    payload.auth.and_then(|a| a.chatgpt_account_id)
}

/// Check if token is expired
pub fn is_token_expired(token: &TokenData) -> bool {
    match token.expires_at {
        Some(expires_at) => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            // Consider expired if within 60 seconds of expiry
            now_ms + 60_000 >= expires_at
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
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = uuid::Uuid::new_v4().to_string();

    // Build the authorization URL (matching OpenClaw's createAuthorizationFlow)
    let auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=opencomputer",
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
                    log::error!("Failed to save token: {}", e);
                }
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

    // Open the browser
    open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;

    Ok(())
}

/// Runs a tiny HTTP server that listens for the OAuth callback
fn run_callback_server(expected_state: &str, code_verifier: &str) -> Result<TokenData> {
    // Bind to 127.0.0.1 (matching OpenClaw's security practice)
    let addr = format!("127.0.0.1:{}", REDIRECT_PORT);
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow!("Failed to start callback server on {}: {}", addr, e))?;

    log::info!("OAuth callback server listening on {}", addr);

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
                    let response = tiny_http::Response::from_string("Not found")
                        .with_status_code(404);
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
                    let response = tiny_http::Response::from_string(format!("Error: {} - {}", error, desc))
                        .with_status_code(400);
                    let _ = request.respond(response);
                    return Err(anyhow!("OAuth error: {} - {}", error, desc));
                }

                // Get the authorization code
                let code = params.get("code")
                    .ok_or_else(|| anyhow!("No authorization code in callback"))?
                    .clone();

                // Send success response to browser
                let html = r#"<!DOCTYPE html>
<html><head><title>OpenComputer</title>
<style>
  body { font-family: -apple-system, sans-serif; display: flex; justify-content: center;
         align-items: center; height: 100vh; margin: 0; background: #0f0f0f; color: #e0e0e0; }
  .container { text-align: center; }
  h1 { font-size: 24px; margin-bottom: 8px; }
  p { color: #888; }
</style></head>
<body><div class="container">
  <h1>✅ 登录成功</h1>
  <p>你可以关闭此页面，回到 OpenComputer 应用。</p>
</div></body></html>"#;

                let response = tiny_http::Response::from_string(html)
                    .with_header("Content-Type: text/html; charset=utf-8".parse::<tiny_http::Header>().unwrap());
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
    s.replace(' ', "+")
        .replace(':', "%3A")
        .replace('/', "%2F")
}

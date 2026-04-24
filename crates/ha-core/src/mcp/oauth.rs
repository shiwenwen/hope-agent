//! OAuth 2.1 + PKCE flow for networked MCP servers.
//!
//! Why we don't use `rmcp::auth_client`:
//! 1. rmcp pulls its own reqwest-0.13 client; ha-core is on 0.12. Mixing at
//!    the type level triggers trait-resolution conflicts (the same reason
//!    `transport.rs` hides rmcp's client behind `build_http_client`).
//! 2. We want event-driven UX: emit `mcp:auth_required` so the desktop
//!    shell opens the browser, the server mode prints to stdout, and the
//!    HTTP mode tunnels the URL to the web UI — all through the same
//!    `EventBus` contract, without reaching into rmcp internals.
//!
//! The flow:
//!
//! ```text
//!   begin ── start loopback listener (127.0.0.1:0)
//!        ── discover .well-known/oauth-authorization-server
//!        ── dynamic client registration if needed (RFC 7591)
//!        ── build authorize URL with PKCE (S256) + CSRF state
//!        ── emit `mcp:auth_required { authUrl }`
//!        ── await callback (deadline = AUTHORIZATION_TIMEOUT)
//!        ── POST /token with code + code_verifier
//!        ── persist credentials (0600) + set ServerState = Idle
//!        ── emit `mcp:auth_completed`
//! ```
//!
//! Every outbound HTTP (discovery, registration, token, refresh) runs
//! through `security::ssrf::check_url` with the `Default` policy. The
//! `redirect_uri` is always a loopback IP on a wildcard port per RFC 8252,
//! so the OAuth server has no attack surface pointing back at our host.
//!
//! Log hygiene: every `app_*!` call that touches a token-bearing payload
//! pipes through `redact_sensitive` first. Raw tokens never land in the
//! text log, the SQLite store, or the event bus.

use std::collections::HashMap;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::config::McpOAuthConfig;
use super::credentials::{self, McpCredentials};
use super::errors::{McpError, McpResult};
use super::events::{emit_auth_completed, emit_auth_required};
use crate::logging::redact_sensitive;
use crate::security::ssrf::{check_url, SsrfPolicy};

/// How long we wait for the user to finish the browser flow before
/// tearing down the callback listener and surfacing a timeout. Ten
/// minutes matches Claude Desktop / Cursor behavior.
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(600);

/// Request timeout for discovery / registration / token endpoints.
/// Short enough that a hung OAuth server fails fast, long enough for
/// slow public endpoints under high latency links.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// PKCE code-verifier length. The spec allows 43–128 characters from the
/// `[A-Z][a-z][0-9]-._~` alphabet; 64 characters (48 random bytes →
/// base64url) sits comfortably in the middle.
const PKCE_VERIFIER_BYTES: usize = 48;

/// CSRF-protection `state` parameter size.
const STATE_BYTES: usize = 32;

/// Local callback path. Fixed so the DCR record matches exactly.
const CALLBACK_PATH: &str = "/callback";

/// DCR default client name — shows up in OAuth server admin consoles.
const DCR_CLIENT_NAME: &str = "Hope Agent (MCP)";

// ── Public types ──────────────────────────────────────────────────

/// PKCE material. `challenge` is what we send on the authorize URL;
/// `verifier` is what we POST to the token endpoint. Never log either.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge_s256: String,
}

/// Subset of `.well-known/oauth-authorization-server` that we need.
/// Additional fields in the JSON are tolerated and discarded.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DiscoveredMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

/// Response of a successful Dynamic Client Registration (RFC 7591).
#[derive(Debug, Clone, Deserialize)]
pub struct ClientRegistration {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Normalized token endpoint response. The spec allows `expires_in` to be
/// absent — in that case we store `expires_at = 0` meaning "never refresh
/// proactively", and rely on a 401 from the resource server to trigger
/// re-auth.
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

// ── Pure helpers ──────────────────────────────────────────────────

impl Pkce {
    /// Generate a fresh PKCE pair. Uses the OS CSPRNG; deterministic tests
    /// feed their own [`Pkce`] instances rather than seeding this function.
    pub fn generate() -> Self {
        let mut buf = [0u8; PKCE_VERIFIER_BYTES];
        rand::rng().fill_bytes(&mut buf);
        let verifier = URL_SAFE_NO_PAD.encode(buf);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge_s256 = URL_SAFE_NO_PAD.encode(digest);
        Pkce {
            verifier,
            challenge_s256,
        }
    }
}

/// Generate a CSRF-resistant opaque `state` value. Returned as URL-safe
/// base64 without padding so it survives being a query-string parameter.
pub fn generate_state() -> String {
    let mut buf = [0u8; STATE_BYTES];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Compute the `.well-known/oauth-authorization-server` URL from a server
/// base URL. Strips the path component — MCP servers host the endpoint
/// discovery metadata at the origin root per RFC 8414.
fn discovery_url(server_url: &str) -> McpResult<String> {
    let mut parsed = url::Url::parse(server_url)
        .map_err(|e| McpError::Config(format!("bad server URL: {e}")))?;
    parsed.set_path("/.well-known/oauth-authorization-server");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

/// Build the browser-facing authorize URL with all PKCE + state
/// parameters attached. The server ignores unknown params, so this also
/// forwards `oauth_cfg.extra_params` verbatim.
fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    code_challenge: &str,
    extra_params: &std::collections::BTreeMap<String, String>,
) -> McpResult<String> {
    let mut url = url::Url::parse(authorization_endpoint)
        .map_err(|e| McpError::Config(format!("bad authorization_endpoint: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", client_id);
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", state);
        if !scopes.is_empty() {
            q.append_pair("scope", &scopes.join(" "));
        }
        for (k, v) in extra_params {
            q.append_pair(k, v);
        }
    }
    Ok(url.to_string())
}

// ── HTTP helpers ──────────────────────────────────────────────────

/// Build a `reqwest::Client` configured for OAuth traffic with the
/// app's proxy policy applied — matches every other public-internet
/// dial-out site (weather / web_fetch / LLM providers). Skipping
/// `apply_proxy` silently breaks OAuth behind a corporate proxy.
fn http_client() -> McpResult<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .timeout(HTTP_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5));
    crate::provider::apply_proxy(builder)
        .build()
        .map_err(|e| McpError::Transport {
            server: "oauth".into(),
            source: format!("build http client: {e}"),
        })
}

/// SSRF-gate an outbound URL, then return the parsed form. Every OAuth
/// network hop (discovery / registration / token / refresh) funnels
/// through here so a rogue authorization-server URL cannot punch through
/// to metadata IPs or arbitrary local services.
async fn guard_url(server_name: &str, url: &str) -> McpResult<url::Url> {
    let app_cfg = crate::config::cached_config();
    let trusted = app_cfg.ssrf.trusted_hosts.clone();
    // Always use `Default` for OAuth endpoints — they're public
    // internet services by definition. `Strict` would reject every
    // real-world OAuth provider (public IPs ≠ metadata IPs), defeating
    // the feature entirely.
    check_url(url, SsrfPolicy::Default, &trusted)
        .await
        .map_err(|e| McpError::Blocked {
            server: server_name.to_string(),
            reason: format!("SSRF blocked OAuth URL: {e}"),
        })
}

// ── Discovery ─────────────────────────────────────────────────────

/// Fetch + parse `.well-known/oauth-authorization-server`. Returns
/// `McpError::Auth` when discovery fails — the natural "server doesn't
/// speak OAuth" signal flows through the same NeedsAuth surfacing path.
pub async fn discover_metadata(
    server_name: &str,
    server_url: &str,
) -> McpResult<DiscoveredMetadata> {
    let url = discovery_url(server_url)?;
    guard_url(server_name, &url).await?;
    let client = http_client()?;
    let resp = client
        .get(&url)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| McpError::Transport {
            server: server_name.to_string(),
            source: format!("discovery GET: {e}"),
        })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(McpError::Auth {
            server: server_name.to_string(),
            message: format!("discovery {status} at {url}"),
        });
    }
    let body = resp.text().await.map_err(|e| McpError::Transport {
        server: server_name.to_string(),
        source: format!("discovery read body: {e}"),
    })?;
    let meta: DiscoveredMetadata = serde_json::from_str(&body).map_err(|e| McpError::Auth {
        server: server_name.to_string(),
        message: format!("discovery JSON parse: {e}"),
    })?;
    // Spec requires S256; refuse plain `plain` to stop a downgrade.
    if !meta.code_challenge_methods_supported.is_empty()
        && !meta
            .code_challenge_methods_supported
            .iter()
            .any(|m| m.eq_ignore_ascii_case("S256"))
    {
        return Err(McpError::Auth {
            server: server_name.to_string(),
            message: "server does not support S256 code_challenge_method".into(),
        });
    }
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "Discovered OAuth endpoints: authorize={}, token={}",
        meta.authorization_endpoint,
        meta.token_endpoint
    );
    Ok(meta)
}

// ── Dynamic Client Registration (RFC 7591) ────────────────────────

/// Register an OAuth client on servers that support DCR. The returned
/// credentials are stored alongside user tokens so refresh works without
/// re-registration; if the user removes the server record, the adjacent
/// credentials file disappears with it.
pub async fn register_dynamic_client(
    server_name: &str,
    registration_endpoint: &str,
    redirect_uri: &str,
    scopes: &[String],
) -> McpResult<ClientRegistration> {
    guard_url(server_name, registration_endpoint).await?;
    let client = http_client()?;
    let mut body = serde_json::json!({
        "client_name": DCR_CLIENT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    if !scopes.is_empty() {
        body["scope"] = serde_json::Value::String(scopes.join(" "));
    }
    let resp = client
        .post(registration_endpoint)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| McpError::Transport {
            server: server_name.to_string(),
            source: format!("DCR POST: {e}"),
        })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(McpError::Auth {
            server: server_name.to_string(),
            message: format!("DCR {status}: {}", redact_sensitive(&text)),
        });
    }
    let reg: ClientRegistration = serde_json::from_str(&text).map_err(|e| McpError::Auth {
        server: server_name.to_string(),
        message: format!("DCR response parse: {e}"),
    })?;
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "Dynamic client registration succeeded (client_id length={})",
        reg.client_id.len()
    );
    Ok(reg)
}

// ── Token endpoint: exchange + refresh ────────────────────────────

/// Exchange an authorization code for an access/refresh token pair.
/// `client_secret` is only attached when the server registered a
/// confidential client — public clients (the common MCP case) rely on
/// PKCE alone to bind the code to this flow.
pub async fn exchange_code_for_tokens(
    server_name: &str,
    authorization_endpoint: &str,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> McpResult<McpCredentials> {
    guard_url(server_name, token_endpoint).await?;
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let token = post_token_form(server_name, token_endpoint, &form).await?;
    Ok(creds_from_token(
        client_id,
        client_secret,
        token_endpoint,
        authorization_endpoint,
        &token,
    ))
}

/// Use a refresh token to mint a new access token without user
/// interaction. On success, the returned credentials record replaces the
/// previous one (refresh tokens can rotate — honor the new one when the
/// server sends it, fall back to the prior otherwise).
pub async fn refresh_access_token(
    server_name: &str,
    prior: &McpCredentials,
) -> McpResult<McpCredentials> {
    let refresh = prior
        .refresh_token
        .as_deref()
        .ok_or_else(|| McpError::Auth {
            server: server_name.to_string(),
            message: "no refresh_token on record; re-authorize required".into(),
        })?;
    guard_url(server_name, &prior.token_endpoint).await?;
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", &prior.client_id),
    ];
    if let Some(secret) = prior.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let token = post_token_form(server_name, &prior.token_endpoint, &form).await?;
    let mut creds = creds_from_token(
        &prior.client_id,
        prior.client_secret.as_deref(),
        &prior.token_endpoint,
        &prior.authorization_endpoint,
        &token,
    );
    // Preserve the prior refresh_token if the server didn't rotate.
    if creds.refresh_token.is_none() {
        creds.refresh_token = prior.refresh_token.clone();
    }
    Ok(creds)
}

/// Common POST-form-with-JSON-response helper shared by authorize and
/// refresh grants. Body + error text go through `redact_sensitive` before
/// landing in logs.
async fn post_token_form(
    server_name: &str,
    token_endpoint: &str,
    form: &[(&str, &str)],
) -> McpResult<TokenFields> {
    let client = http_client()?;
    let resp = client
        .post(token_endpoint)
        .header("accept", "application/json")
        .form(form)
        .send()
        .await
        .map_err(|e| McpError::Transport {
            server: server_name.to_string(),
            source: format!("token POST: {e}"),
        })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(McpError::Auth {
            server: server_name.to_string(),
            message: format!("token {status}: {}", redact_sensitive(&text)),
        });
    }
    let parsed: TokenResponse = serde_json::from_str(&text).map_err(|e| McpError::Auth {
        server: server_name.to_string(),
        message: format!("token response parse: {e}"),
    })?;
    Ok(TokenFields {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_in: parsed.expires_in,
        scope: parsed.scope,
    })
}

/// Separate from the `serde`-only `TokenResponse` so callers of the
/// exchange/refresh functions get a `Clone` + `Debug` struct they can
/// pass around without dragging the private parser type.
#[derive(Debug, Clone)]
pub struct TokenFields {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
    pub scope: Option<String>,
}

/// Convert an OAuth token response into a persisted [`McpCredentials`]
/// record. Timestamps are computed at the call site so a slow disk
/// write doesn't skew `issued_at`.
fn creds_from_token(
    client_id: &str,
    client_secret: Option<&str>,
    token_endpoint: &str,
    authorization_endpoint: &str,
    token: &TokenFields,
) -> McpCredentials {
    let now = chrono::Utc::now().timestamp();
    let expires_at = token.expires_in.map(|s| now + s).unwrap_or(0);
    let scopes = token
        .scope
        .as_deref()
        .map(|s| {
            s.split_whitespace()
                .map(|w| w.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    McpCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.map(|s| s.to_string()),
        access_token: token.access_token.clone(),
        refresh_token: token.refresh_token.clone(),
        expires_at,
        token_endpoint: token_endpoint.to_string(),
        authorization_endpoint: authorization_endpoint.to_string(),
        granted_scopes: scopes,
        issued_at: now,
    }
}

// ── Callback server ───────────────────────────────────────────────

/// What the loopback listener extracted from the browser redirect.
/// The `state` parameter is verified inline in the connection handler
/// (mismatched state is treated as "ignored prefetch"), so the
/// orchestrator only needs the code.
#[derive(Debug)]
struct CallbackResult {
    code: String,
}

/// Bind `127.0.0.1:0`, return the concrete port + a oneshot future that
/// resolves when the browser hits `/callback`. The listener closes after
/// the first request or on timeout, whichever fires first.
///
/// We use raw `tokio::net` + `hyper::service::service_fn` rather than a
/// full `axum::Router` to keep the dependency footprint small (hyper is
/// already pulled in via reqwest) and because we only accept one request
/// ever — an `axum::Router` is overkill for that. Response HTML is
/// hard-coded.
async fn start_callback_listener(
    expected_state: String,
) -> McpResult<(u16, oneshot::Receiver<McpResult<CallbackResult>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| McpError::Transport {
            server: "oauth-callback".into(),
            source: format!("bind loopback listener: {e}"),
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| McpError::Transport {
            server: "oauth-callback".into(),
            source: format!("local_addr: {e}"),
        })?
        .port();
    let (mut tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        // Loop because a link preview / browser prefetch can fire the
        // callback URL before the real navigation, and we want to match
        // the `state` we actually issued.
        let deadline = tokio::time::Instant::now() + AUTHORIZATION_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = tx.send(Err(McpError::Auth {
                    server: "oauth-callback".into(),
                    message: format!(
                        "no callback within {}s (authorize flow abandoned?)",
                        AUTHORIZATION_TIMEOUT.as_secs()
                    ),
                }));
                return;
            }
            // Cancel immediately when the orchestrator drops its receiver
            // (timeout / early error / caller gave up). Without this the
            // listener would sit on the bound port until its own deadline,
            // wasting a local port for up to AUTHORIZATION_TIMEOUT.
            let (mut stream, _peer) = tokio::select! {
                _ = tx.closed() => return,
                res = tokio::time::timeout(remaining, listener.accept()) => match res {
                    Ok(Ok(pair)) => pair,
                    Ok(Err(e)) => {
                        let _ = tx.send(Err(McpError::Transport {
                            server: "oauth-callback".into(),
                            source: format!("accept: {e}"),
                        }));
                        return;
                    }
                    Err(_) => {
                        let _ = tx.send(Err(McpError::Auth {
                            server: "oauth-callback".into(),
                            message: "callback timeout".into(),
                        }));
                        return;
                    }
                }
            };
            match handle_callback_connection(&mut stream, &expected_state).await {
                Ok(CallbackOutcome::Matched(result)) => {
                    let _ = tx.send(Ok(result));
                    return;
                }
                Ok(CallbackOutcome::Ignored) => continue,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        }
    });

    Ok((port, rx))
}

enum CallbackOutcome {
    Matched(CallbackResult),
    /// Something hit our port but either wasn't the OAuth callback or
    /// carried the wrong `state`. Keep listening rather than abort — a
    /// browser prefetch can produce these without user intent.
    Ignored,
}

/// Parse a single minimal HTTP/1.1 request line, respond with a fixed
/// HTML page, and extract `code` + `state` from the query string.
/// Unknown or OAuth-error paths still return `200 OK` so the user sees
/// the same "you can close this tab" page regardless — otherwise some
/// browsers display their default error UI which is more confusing.
async fn handle_callback_connection(
    stream: &mut tokio::net::TcpStream,
    expected_state: &str,
) -> McpResult<CallbackOutcome> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // We only need the first line of the request. Cap the buffer to
    // avoid a malicious client flooding memory. 8 KiB covers the spec
    // upper bound on a sane authorize callback URL comfortably.
    //
    // Per-read timeout: without it a malicious localhost client (rogue
    // browser extension, misbehaving app listening on 127.0.0.1) could
    // `connect` + dribble one byte at a time and stall the entire
    // authorize flow by holding the listener busy up to
    // AUTHORIZATION_TIMEOUT.
    const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    let mut buf = [0u8; 8192];
    let mut read_total = 0usize;
    loop {
        let n = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf[read_total..])).await
        {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                return Err(McpError::Transport {
                    server: "oauth-callback".into(),
                    source: format!("read: {e}"),
                })
            }
            Err(_) => return Ok(CallbackOutcome::Ignored),
        };
        if n == 0 {
            break;
        }
        read_total += n;
        if read_total >= buf.len() {
            break;
        }
        if buf[..read_total].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request_line = std::str::from_utf8(&buf[..read_total])
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    // Path is the second space-separated token: `GET /callback?... HTTP/1.1`.
    let path_and_query = request_line.split_whitespace().nth(1).unwrap_or("");

    let html = b"<!doctype html><html><head><meta charset=\"utf-8\">\
        <title>Hope Agent: OAuth</title>\
        <style>body{font-family:system-ui;margin:4rem;text-align:center;color:#222}</style>\
        </head><body><h2>Authorization complete</h2>\
        <p>You can close this tab and return to Hope Agent.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(html).await;
    let _ = stream.shutdown().await;

    // Not our path — a prefetcher hitting `/` or `/favicon.ico`.
    if !path_and_query.starts_with(CALLBACK_PATH) {
        return Ok(CallbackOutcome::Ignored);
    }

    let query = path_and_query.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();

    if let Some(err) = params.get("error") {
        let desc = params
            .get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("");
        return Err(McpError::Auth {
            server: "oauth-callback".into(),
            message: format!("authorization server returned error: {err} ({desc})"),
        });
    }
    let code = match params.get("code") {
        Some(c) if !c.is_empty() => c.clone(),
        _ => return Ok(CallbackOutcome::Ignored),
    };
    let state = params.get("state").cloned().unwrap_or_default();
    if state != expected_state {
        // Almost always CSRF — but a browser preload could have queued
        // a stale URL. Ignore rather than abort the whole flow.
        crate::app_warn!(
            "mcp",
            "oauth:callback",
            "state mismatch on /callback; ignoring (possible CSRF or stale prefetch)"
        );
        return Ok(CallbackOutcome::Ignored);
    }
    Ok(CallbackOutcome::Matched(CallbackResult { code }))
}

// ── Orchestrator ──────────────────────────────────────────────────

/// Run the full PKCE + loopback-callback authorization flow for a
/// networked MCP server. On success the credentials file is persisted
/// (0600) and the returned record is ready to attach to the transport's
/// `Authorization: Bearer ...` header.
///
/// Side effects:
/// * Emits `mcp:auth_required` once the authorize URL is built so the
///   desktop shell / CLI / HTTP client can open the browser.
/// * Emits `mcp:auth_completed` on both success and failure.
/// * Writes `~/.hope-agent/credentials/mcp/{server_id}.json` on success.
pub async fn authorize_server(
    server_id: &str,
    server_name: &str,
    server_url: &str,
    oauth_cfg: &McpOAuthConfig,
) -> McpResult<McpCredentials> {
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "Starting OAuth authorization for MCP server"
    );
    credentials::ensure_dir().map_err(|e| McpError::Config(e.to_string()))?;

    // 1. Bind callback listener first so the redirect_uri is concrete
    //    before discovery / DCR.
    let state = generate_state();
    let pkce = Pkce::generate();
    let (port, callback_rx) = start_callback_listener(state.clone()).await?;
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");

    // 2. Resolve endpoints — user-pinned values win over discovery.
    let meta = match (
        oauth_cfg.authorization_endpoint.as_deref(),
        oauth_cfg.token_endpoint.as_deref(),
    ) {
        (Some(auth_ep), Some(token_ep)) => DiscoveredMetadata {
            authorization_endpoint: auth_ep.to_string(),
            token_endpoint: token_ep.to_string(),
            registration_endpoint: None,
            scopes_supported: vec![],
            code_challenge_methods_supported: vec!["S256".into()],
        },
        _ => discover_metadata(server_name, server_url).await?,
    };

    // 3. Resolve the client id — DCR when config didn't provide one.
    let (client_id, client_secret) = match &oauth_cfg.client_id {
        Some(id) => (id.clone(), oauth_cfg.client_secret.clone()),
        None => {
            let reg_ep = meta
                .registration_endpoint
                .as_ref()
                .ok_or_else(|| McpError::Auth {
                    server: server_name.to_string(),
                    message: "no client_id configured and server has no registration_endpoint"
                        .into(),
                })?;
            let reg =
                register_dynamic_client(server_name, reg_ep, &redirect_uri, &oauth_cfg.scopes)
                    .await?;
            (reg.client_id, reg.client_secret)
        }
    };

    // 4. Build the authorize URL + nudge the user to open it.
    let auth_url = build_authorize_url(
        &meta.authorization_endpoint,
        &client_id,
        &redirect_uri,
        &oauth_cfg.scopes,
        &state,
        &pkce.challenge_s256,
        &oauth_cfg.extra_params,
    )?;
    emit_auth_required(server_id, server_name, &auth_url);
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "Awaiting user authorization at {}",
        auth_url
    );
    // Open the default browser directly from the backend so the flow
    // works identically in the Tauri shell and the HTTP server (whose
    // caller may not even be a browser). `open::that` is async-safe and
    // never blocks the flow — a failure here just means the user has to
    // click the URL from the `mcp:auth_required` event payload.
    if let Err(e) = open::that(&auth_url) {
        crate::app_warn!(
            "mcp",
            &format!("{server_name}:oauth"),
            "Failed to auto-open browser; user must open the URL manually: {e}"
        );
    }

    // 5. Wait for the browser callback, with an overall timeout so a
    //    dropped tab doesn't pin a listener forever. Listener shares
    //    the same deadline (`start_callback_listener` binds it to
    //    `AUTHORIZATION_TIMEOUT` internally), so either path wins the
    //    race cleanly.
    let cb_result = match timeout(AUTHORIZATION_TIMEOUT, callback_rx).await {
        Ok(Ok(Ok(cb))) => Ok(cb),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_canceled)) => Err(McpError::Auth {
            server: server_name.to_string(),
            message: "callback listener closed unexpectedly".into(),
        }),
        Err(_elapsed) => Err(McpError::Auth {
            server: server_name.to_string(),
            message: format!(
                "user did not complete authorization within {}s",
                AUTHORIZATION_TIMEOUT.as_secs()
            ),
        }),
    };
    let cb_result = cb_result.inspect_err(|e| {
        emit_auth_completed(server_id, server_name, false, Some(&e.to_string()));
    })?;

    // 6. Exchange the code for tokens.
    let creds = exchange_code_for_tokens(
        server_name,
        &meta.authorization_endpoint,
        &meta.token_endpoint,
        &client_id,
        client_secret.as_deref(),
        &cb_result.code,
        &redirect_uri,
        &pkce.verifier,
    )
    .await
    .inspect_err(|e| {
        emit_auth_completed(server_id, server_name, false, Some(&e.to_string()));
    })?;

    // 7. Persist under the secure-file path + emit success.
    credentials::save(server_id, &creds).map_err(|e| {
        let err = McpError::Auth {
            server: server_name.to_string(),
            message: format!("persist credentials: {e}"),
        };
        emit_auth_completed(server_id, server_name, false, Some(&err.to_string()));
        err
    })?;
    emit_auth_completed(server_id, server_name, true, None);
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "OAuth authorization succeeded; credentials persisted"
    );
    Ok(creds)
}

/// Refresh the stored tokens if they're about to expire (60s safety
/// margin) or already did. Returns the freshest credentials record —
/// which the caller must re-persist when it differs from the input.
/// On refresh failure surfaces `McpError::Auth` so the transport layer
/// can drop the server into `NeedsAuth`.
pub async fn refresh_if_stale(
    server_id: &str,
    server_name: &str,
    current: &McpCredentials,
) -> McpResult<McpCredentials> {
    if !current.needs_refresh() {
        return Ok(current.clone());
    }
    let refreshed = refresh_access_token(server_name, current)
        .await
        .map_err(|e| {
            crate::app_warn!(
                "mcp",
                &format!("{server_name}:oauth"),
                "refresh_access_token failed: {e}"
            );
            e
        })?;
    credentials::save(server_id, &refreshed).map_err(|e| McpError::Auth {
        server: server_name.to_string(),
        message: format!("persist refreshed credentials: {e}"),
    })?;
    crate::app_info!(
        "mcp",
        &format!("{server_name}:oauth"),
        "Refreshed access token (expires_at={})",
        refreshed.expires_at
    );
    Ok(refreshed)
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = Pkce::generate();
        let digest = Sha256::digest(p.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(p.challenge_s256, expected);
        // URL-safe base64 (no padding, no `+` / `/`).
        assert!(!p.verifier.contains('='));
        assert!(!p.verifier.contains('+'));
        assert!(!p.verifier.contains('/'));
    }

    #[test]
    fn state_is_url_safe_and_has_entropy() {
        let a = generate_state();
        let b = generate_state();
        assert_ne!(a, b);
        assert!(!a.contains('='));
        assert!(!a.contains('/'));
        assert!(!a.contains('+'));
        // 32 bytes → 43 base64url chars.
        assert_eq!(a.len(), 43);
    }

    #[test]
    fn discovery_url_strips_path_and_query() {
        let u = discovery_url("https://example.com/mcp/v1?token=abc#x").unwrap();
        assert_eq!(
            u,
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn authorize_url_emits_required_params() {
        let extra: std::collections::BTreeMap<String, String> =
            [("audience".into(), "mcp".into())].into_iter().collect();
        let out = build_authorize_url(
            "https://example.com/oauth/authorize",
            "client-123",
            "http://127.0.0.1:51234/callback",
            &["read".into(), "write".into()],
            "state-xyz",
            "chal-abc",
            &extra,
        )
        .unwrap();
        assert!(out.contains("response_type=code"));
        assert!(out.contains("client_id=client-123"));
        assert!(out.contains("code_challenge=chal-abc"));
        assert!(out.contains("code_challenge_method=S256"));
        assert!(out.contains("state=state-xyz"));
        assert!(out.contains("scope=read+write"));
        assert!(out.contains("audience=mcp"));
    }
}

//! Transport factories — wire an [`McpTransportSpec`] up to an rmcp client.
//!
//! Phase 2 shipped stdio only. Phase 4 adds Streamable HTTP (the spec's
//! preferred remote transport), a best-effort SSE fallback routed through
//! the same client (rmcp 1.5 retired the standalone SSE client in favor
//! of Streamable HTTP's SSE sub-protocol), and WebSocket via
//! `tokio-tungstenite` bridged into rmcp's `SinkStreamTransport`.
//!
//! Every networked transport goes through the project SSRF policy
//! (`security::ssrf::check_url`) BEFORE we touch the network, so a
//! misconfigured private-network URL cannot exfiltrate through a rogue
//! `Authorization` header. The `ws(s)://` scheme is rewritten to
//! `http(s)://` for the SSRF gate — host/port classification is
//! identical, the scheme itself is only a port-default hint.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use rmcp::service::RunningService;
use rmcp::transport::child_process::ConfigureCommandExt;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use tokio::process::{ChildStderr, Command};

use super::config::{expand_placeholders, McpServerConfig, McpTransportSpec, McpTrustLevel};
use super::credentials;
use super::errors::{McpError, McpResult};
use super::oauth;

/// Minimal list of env vars inherited from the parent process when we
/// spawn a subprocess. Stops surprises like "works on my machine because
/// I have `AWS_PROFILE` in my shell" from making MCP servers behave
/// differently between the desktop GUI and the HTTP server mode.
///
/// Anything the server genuinely needs must be declared in the server's
/// [`McpServerConfig::env`] block.
const INHERITED_ENV_WHITELIST: &[&str] = &[
    "HOME", "USER", "PATH", "LANG", "LC_ALL", "TZ", "TMPDIR", "TEMP", "TMP",
];

/// Build a `tokio::process::Command` from a `Stdio` transport spec, applying:
/// * env placeholder expansion (`${VAR}` / `$VAR`), looking up in the
///   server's own `env` block first, then falling back to the real env.
/// * env whitelisting — only whitelisted vars inherit from the parent,
///   plus the server's explicit `env` entries on top.
/// * optional `cwd`.
fn build_stdio_command(cfg: &McpServerConfig) -> McpResult<Command> {
    let (command, args, cwd) = match &cfg.transport {
        McpTransportSpec::Stdio { command, args, cwd } => (command, args, cwd),
        _ => unreachable!("build_stdio_command called on non-stdio transport"),
    };

    // 1. Expand `${VAR}` in the server's env values using the process
    //    env as fallback. Keys are never expanded (they're identifiers).
    let expanded_env: BTreeMap<String, String> = cfg
        .env
        .iter()
        .map(|(k, v)| {
            let expanded = expand_placeholders(v, |name| std::env::var(name).ok());
            (k.clone(), expanded)
        })
        .collect();

    // 2. Build the final env map: whitelist inherit + expanded overrides.
    //    Explicit server entries win over the inherited defaults.
    let mut final_env: BTreeMap<String, String> = BTreeMap::new();
    for key in INHERITED_ENV_WHITELIST {
        if let Ok(v) = std::env::var(key) {
            final_env.insert((*key).to_string(), v);
        }
    }
    for (k, v) in expanded_env {
        final_env.insert(k, v);
    }

    // 3. Expand `${VAR}` in each argv slot using the *final* env first,
    //    then the process env. This lets users template the real command
    //    on values they just declared in `env`.
    let expanded_args: Vec<String> = args
        .iter()
        .map(|a| {
            expand_placeholders(a, |name| {
                final_env
                    .get(name)
                    .cloned()
                    .or_else(|| std::env::var(name).ok())
            })
        })
        .collect();

    // 4. Expand cwd similarly. Unknown variable → empty substring, which
    //    should produce a visible error from the OS (ENOENT) rather than
    //    silent failure.
    let expanded_cwd = cwd.as_ref().map(|c| {
        expand_placeholders(c, |name| {
            final_env
                .get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
        })
    });

    let mut cmd = Command::new(command);
    cmd.args(&expanded_args).env_clear();
    for (k, v) in final_env {
        cmd.env(k, v);
    }
    if let Some(dir) = expanded_cwd {
        if !dir.is_empty() {
            cmd.current_dir(dir);
        }
    }
    // Stdio MCP servers (node/npx/python/uvx…) are real console processes on
    // Windows — suppress the console window so connecting one never flashes.
    crate::platform::hide_console_tokio(&mut cmd);
    Ok(cmd)
}

/// A fully-served MCP client plus any side-channel handles the caller
/// needs to drive after the fact. `stderr` is `Some` only for stdio.
///
/// We construct the rmcp transport **and** call `.serve()` internally
/// here so the concrete reqwest-0.13 `Client` type rmcp uses for
/// Streamable HTTP never escapes this module (ha-core itself depends
/// on reqwest 0.12 through other call sites; mixing the two at the
/// type level causes a trait-resolution conflict).
pub struct ConnectedClient {
    pub running: RunningService<RoleClient, ()>,
    pub stderr: Option<ChildStderr>,
}

/// Spawn the subprocess described by a stdio transport spec and return
/// the connected rmcp client + stderr pipe. Caller must drain the
/// stderr pipe — otherwise a verbose server can fill its buffer and
/// block.
pub async fn build_stdio_client(cfg: &McpServerConfig) -> McpResult<ConnectedClient> {
    let cmd = build_stdio_command(cfg)?;
    let (proc, stderr) = TokioChildProcess::builder(cmd.configure(|_| {}))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| McpError::Transport {
            server: cfg.name.clone(),
            source: format!("spawn failed: {e}"),
        })?;
    let running = ().serve(proc).await.map_err(|e| McpError::Transport {
        server: cfg.name.clone(),
        source: format!("handshake failed: {e}"),
    })?;
    Ok(ConnectedClient { running, stderr })
}

/// SSRF-gate a networked transport URL before constructing the client.
/// Trusted servers use the app-level default policy; untrusted use
/// `Strict`. Callers pass the `http(s)://` form — WS callers rewrite
/// `ws(s)://` to the `http(s)://` equivalent first, because
/// `security::ssrf` only classifies those schemes.
async fn ssrf_gate_url(cfg: &McpServerConfig, http_equiv_url: &str) -> McpResult<()> {
    let app_cfg = crate::config::cached_config();
    let trusted_hosts = app_cfg.ssrf.trusted_hosts.clone();
    let policy = match cfg.trust_level {
        McpTrustLevel::Trusted => app_cfg.ssrf.default_policy,
        McpTrustLevel::Untrusted => crate::security::ssrf::SsrfPolicy::Strict,
    };
    crate::security::ssrf::check_url(http_equiv_url, policy, &trusted_hosts)
        .await
        .map_err(|e| McpError::Blocked {
            server: cfg.name.clone(),
            reason: format!(
                "SSRF policy blocked {} URL: {e}",
                cfg.transport.kind_label()
            ),
        })?;
    Ok(())
}

/// Build the final request-header map for a networked transport:
/// user-provided headers (with `${ENV}` expansion) + OAuth Bearer token
/// (if `cfg.oauth` is set AND disk has credentials AND the user didn't
/// already pin an `Authorization` header themselves).
///
/// A missing credentials file is NOT an error — the handshake's 401
/// surface path is what flips the server into `NeedsAuth`.
async fn authorized_headers(cfg: &McpServerConfig) -> McpResult<HashMap<HeaderName, HeaderValue>> {
    let mut headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
    let mut user_set_authorization = false;
    for (k, v) in &cfg.headers {
        let expanded = expand_placeholders(v, |name| std::env::var(name).ok());
        let name = HeaderName::from_str(k).map_err(|e| {
            McpError::Config(format!(
                "invalid header name '{k}' for server '{srv}': {e}",
                srv = cfg.name
            ))
        })?;
        let value = HeaderValue::from_str(&expanded).map_err(|e| {
            McpError::Config(format!(
                "invalid header value for '{k}' on server '{srv}': {e}",
                srv = cfg.name
            ))
        })?;
        if name == http::header::AUTHORIZATION {
            user_set_authorization = true;
        }
        headers.insert(name, value);
    }

    // A user-provided `Authorization` header always wins: some setups
    // pre-bake a long-lived PAT / service token that our OAuth flow
    // shouldn't overwrite.
    if cfg.oauth.is_some() && !user_set_authorization {
        match credentials::load(&cfg.id) {
            Ok(Some(creds)) => {
                let fresh = oauth::refresh_if_stale(&cfg.id, &cfg.name, &creds).await?;
                let bearer = format!("Bearer {}", fresh.access_token);
                let value = HeaderValue::from_str(&bearer).map_err(|e| McpError::Auth {
                    server: cfg.name.clone(),
                    message: format!("invalid access_token in stored credentials: {e}"),
                })?;
                headers.insert(http::header::AUTHORIZATION, value);
            }
            Ok(None) => {
                crate::app_info!(
                    "mcp",
                    &format!("{}:oauth", cfg.name),
                    "No stored OAuth credentials; handshake will trigger NeedsAuth on 401"
                );
            }
            Err(e) => {
                crate::app_warn!(
                    "mcp",
                    &format!("{}:oauth", cfg.name),
                    "Failed to load stored OAuth credentials: {e}"
                );
            }
        }
    }
    Ok(headers)
}

/// True when `host` is loopback / private / link-local, i.e. a proxy must be
/// bypassed to reach it. Remote hosts return `false` (the proxy still applies).
///
/// Why: a user who runs a system / env HTTP proxy to reach remote LLM APIs will
/// otherwise have that proxy hijack a local MCP server — `http://localhost:PORT`
/// gets routed through the proxy, which can't dial loopback and returns a 503 /
/// hangs. reqwest picks up the OS proxy but does NOT honor its bypass list
/// (macOS `ExceptionsList` / `ExcludeSimpleHostnames`), so we decide the bypass
/// ourselves from the target host. Applied on the SSE path (ha-core's own
/// reqwest); the Streamable HTTP path is on rmcp's reqwest and can't share it.
fn host_bypasses_proxy(host: &str) -> bool {
    use std::net::IpAddr;
    // `url::Host` never yields brackets, but strip them defensively.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let lower = bare.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        Ok(IpAddr::V6(v6)) => {
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
        // A resolvable remote hostname — keep the proxy (it may be how the user
        // reaches the outside world). SSRF classification stays the security
        // boundary; this only decides proxy routing.
        Err(_) => false,
    }
}

/// Build the reqwest client for the hand-rolled SSE transport. Mirrors rmcp's
/// default client (`pool_max_idle_per_host(0)` to dodge TCP Delayed-ACK stalls)
/// and additionally bypasses the system / env proxy when the target host is
/// loopback / private (see [`host_bypasses_proxy`]).
fn build_mcp_http_client(target_url: &str, server_name: &str) -> McpResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder().pool_max_idle_per_host(0);
    let bypass_proxy = url::Url::parse(target_url)
        .ok()
        .and_then(|u| u.host_str().map(host_bypasses_proxy))
        .unwrap_or(false);
    if bypass_proxy {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| McpError::Transport {
        server: server_name.to_string(),
        source: format!("failed to build MCP HTTP client: {e}"),
    })
}

/// Classify a networked-transport error as `Auth` vs `Transport` using
/// [`is_auth_challenge`]. The `verb` is the user-visible slice of the
/// error message that names the phase that failed (e.g. `"handshake"`
/// or `"WebSocket handshake"`) so the GUI row's `reason` stays
/// self-explanatory.
fn classify_network_error(cfg_name: &str, verb: &str, e: impl std::fmt::Display) -> McpError {
    let msg = e.to_string();
    if is_auth_challenge(&msg) {
        McpError::Auth {
            server: cfg_name.to_string(),
            message: format!("{verb} rejected by server: {msg}"),
        }
    } else {
        McpError::Transport {
            server: cfg_name.to_string(),
            source: format!("{verb} failed: {msg}"),
        }
    }
}

/// Build a Streamable HTTP (or SSE → Streamable HTTP fallback) client
/// transport and complete the initial handshake. Runs the SSRF policy
/// check before constructing the underlying reqwest client so a
/// misconfigured private-network URL never dials out.
pub async fn build_http_client(cfg: &McpServerConfig, url: &str) -> McpResult<ConnectedClient> {
    // Expand `${VAR}` in URL so SSRF check sees the real destination.
    let expanded_url = expand_placeholders(url, |name| std::env::var(name).ok());
    ssrf_gate_url(cfg, &expanded_url).await?;
    let headers = authorized_headers(cfg).await?;

    // NOTE: this path uses rmcp's own reqwest (0.13) client via `from_config`.
    // We can't inject a proxy-bypassing client here the way `build_sse_client`
    // does (ha-core is on reqwest 0.12; rmcp's `StreamableHttpClient` is impl'd
    // only for its 0.13 `reqwest::Client`, and rmcp exposes no proxy knob), so a
    // system proxy set for remote APIs can still hijack a *local* Streamable HTTP
    // MCP server. Local MCP is overwhelmingly stdio / SSE, so this is left as a
    // known limitation rather than pulling a second reqwest major into ha-core.
    let http_cfg =
        StreamableHttpClientTransportConfig::with_uri(expanded_url).custom_headers(headers);
    let transport = StreamableHttpClientTransport::from_config(http_cfg);
    let running = ()
        .serve(transport)
        .await
        .map_err(|e| classify_network_error(&cfg.name, "handshake", e))?;
    Ok(ConnectedClient {
        running,
        stderr: None,
    })
}

/// Heuristic: does this rmcp handshake error describe an HTTP 401/403 /
/// OAuth auth challenge? rmcp's error type doesn't expose the underlying
/// status cleanly, so we substring-match on common response shapes. Any
/// hit flips the server into `NeedsAuth` which is a recoverable state;
/// false negatives just degrade to `Transport`, which is also safe.
fn is_auth_challenge(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid_token")
        || lower.contains("invalid_grant")
}

/// Build a WebSocket MCP client. Bridges `tokio-tungstenite`'s
/// `WebSocketStream` into rmcp's generic `SinkStreamTransport` via the
/// [`WsJsonRpcTransport`] adapter defined below — text / binary frames
/// carry JSON-RPC payloads, ping / pong / close frames are handled by
/// tungstenite and never reach rmcp.
pub async fn build_ws_client(cfg: &McpServerConfig, url: &str) -> McpResult<ConnectedClient> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let expanded_url = expand_placeholders(url, |name| std::env::var(name).ok());
    // `security::ssrf::check_url` only classifies http/https. ws→http,
    // wss→https is semantically identical for host/port classification.
    let http_equiv = ws_to_http_equiv(&expanded_url)?;
    ssrf_gate_url(cfg, &http_equiv).await?;

    let headers = authorized_headers(cfg).await?;
    let mut request = expanded_url.as_str().into_client_request().map_err(|e| {
        McpError::Config(format!(
            "invalid WebSocket URL for server '{}': {e}",
            cfg.name
        ))
    })?;
    for (name, value) in headers {
        request.headers_mut().insert(name, value);
    }

    // Cap incoming frames so a malicious / misconfigured MCP server
    // can't OOM us with a multi-GB text frame. tungstenite's defaults
    // (64 MiB message / 16 MiB frame) are appropriate for general
    // WebSocket traffic but wasteful for JSON-RPC. 4 MiB / 1 MiB leaves
    // generous headroom over realistic MCP payloads.
    //
    // `connect_async` does NOT follow HTTP redirects — RFC 6455 requires
    // the upgrade response to be 101 Switching Protocols, so any 3xx
    // kills the handshake. Our single SSRF gate above therefore covers
    // the only dial-out this function makes.
    let ws_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(4 * 1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (ws, _resp) = tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false)
        .await
        .map_err(|e| classify_network_error(&cfg.name, "WebSocket handshake", e))?;

    // rmcp's `IntoTransport for (Si, St)` expects the two halves as a
    // tuple; `StreamExt::split` on our `Sink + Stream` adapter yields
    // exactly that shape.
    let (sink, stream) = WsJsonRpcTransport::new(ws).split();
    let running = ()
        .serve((sink, stream))
        .await
        .map_err(|e| classify_network_error(&cfg.name, "WebSocket handshake", e))?;
    Ok(ConnectedClient {
        running,
        stderr: None,
    })
}

/// Adapter bridging `tokio-tungstenite::WebSocketStream` to rmcp's
/// `Sink<TxJsonRpcMessage<RoleClient>> + Stream<Item = RxJsonRpcMessage<RoleClient>>`
/// contract.
///
/// Implemented as a manual `Sink` / `Stream` pair rather than
/// `SinkExt::with` + `StreamExt::filter_map` because those combinators
/// produce types whose `Unpin` bound depends on the captured future's
/// auto-trait inference — `async move { ... }` closures are
/// conservatively `!Unpin`, and rmcp's `IntoTransport for (Si, St)`
/// bound requires both halves to be `Unpin`. Manual impl sidesteps
/// the whole category of errors.
struct WsJsonRpcTransport<S> {
    ws: S,
}

impl<S> WsJsonRpcTransport<S> {
    fn new(ws: S) -> Self {
        Self { ws }
    }
}

// `Unpin` is auto-derived: the only field is `S`, so `WsJsonRpcTransport<S>`
// is `Unpin` iff `S` is. tokio-tungstenite's `WebSocketStream` is always
// `Unpin`, so rmcp can hold the adapter by `&mut`.

impl<S> futures_util::Sink<rmcp::service::TxJsonRpcMessage<RoleClient>> for WsJsonRpcTransport<S>
where
    S: futures_util::Sink<
            tokio_tungstenite::tungstenite::protocol::Message,
            Error = tokio_tungstenite::tungstenite::Error,
        > + Unpin,
{
    type Error = tokio_tungstenite::tungstenite::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::pin::Pin::new(&mut self.get_mut().ws).poll_ready(cx)
    }

    fn start_send(
        self: std::pin::Pin<&mut Self>,
        item: rmcp::service::TxJsonRpcMessage<RoleClient>,
    ) -> Result<(), Self::Error> {
        let json = serde_json::to_string(&item)
            .map_err(|e| tokio_tungstenite::tungstenite::Error::Io(std::io::Error::other(e)))?;
        std::pin::Pin::new(&mut self.get_mut().ws).start_send(
            tokio_tungstenite::tungstenite::protocol::Message::Text(json.into()),
        )
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::pin::Pin::new(&mut self.get_mut().ws).poll_flush(cx)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::pin::Pin::new(&mut self.get_mut().ws).poll_close(cx)
    }
}

impl<S> futures_util::Stream for WsJsonRpcTransport<S>
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::protocol::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + Unpin,
{
    type Item = rmcp::service::RxJsonRpcMessage<RoleClient>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use tokio_tungstenite::tungstenite::protocol::Message;
        // Cooperative-yield budget: cap consecutive non-data frames
        // (pings, pongs, malformed JSON) per poll so a misbehaving or
        // malicious server can't starve the scheduler by flooding us
        // with frames we'd silently discard. After the budget is spent,
        // we wake our own task and return `Pending` — the runtime picks
        // us back up on the next tick with a clean budget.
        const MAX_DROPPED_FRAMES_PER_POLL: usize = 64;
        let this = self.get_mut();
        let mut dropped = 0usize;
        loop {
            match std::pin::Pin::new(&mut this.ws).poll_next(cx) {
                std::task::Poll::Pending => return std::task::Poll::Pending,
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(None),
                // Transport error from tungstenite ends the stream.
                // rmcp treats this the same as a clean close and will
                // surface a Service-level error on the next operation.
                std::task::Poll::Ready(Some(Err(_))) => return std::task::Poll::Ready(None),
                std::task::Poll::Ready(Some(Ok(msg))) => {
                    let parsed = match msg {
                        Message::Text(txt) => serde_json::from_str(&txt).ok(),
                        Message::Binary(bin) => serde_json::from_slice(&bin).ok(),
                        // Ping / Pong / Close / Frame: handled inside
                        // tungstenite; nothing for rmcp to see.
                        _ => None,
                    };
                    if let Some(m) = parsed {
                        return std::task::Poll::Ready(Some(m));
                    }
                    // Malformed JSON OR control frame: drop and keep
                    // polling. Persistent garbage is bounded by the
                    // budget below rather than killing the transport
                    // on the first bad frame.
                    dropped += 1;
                    if dropped >= MAX_DROPPED_FRAMES_PER_POLL {
                        cx.waker().wake_by_ref();
                        return std::task::Poll::Pending;
                    }
                }
            }
        }
    }
}

/// ws → http / wss → https for SSRF classification. Anything else is a
/// config error — the schema-level validator should have already caught
/// it, but defensive here.
fn ws_to_http_equiv(url: &str) -> McpResult<String> {
    let mut parsed =
        url::Url::parse(url).map_err(|e| McpError::Config(format!("invalid ws URL: {e}")))?;
    let new_scheme = match parsed.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => {
            return Err(McpError::Config(format!(
                "unsupported WebSocket scheme: {other}"
            )))
        }
    };
    parsed
        .set_scheme(new_scheme)
        .map_err(|_| McpError::Config("WebSocket scheme rewrite failed".into()))?;
    Ok(parsed.to_string())
}

/// Resolve the server-provided SSE `endpoint` value against the SSE base URL.
/// The endpoint is usually a relative path carrying `?session_id=…`; absolute
/// URLs are honored as-is. Extracted as a pure function so the resolution +
/// same-origin behavior is unit-testable without real sockets.
fn resolve_sse_endpoint(base_url: &str, endpoint: &str, server_name: &str) -> McpResult<url::Url> {
    let base = url::Url::parse(base_url).map_err(|e| {
        McpError::Config(format!("invalid SSE URL for server '{server_name}': {e}"))
    })?;
    base.join(endpoint.trim()).map_err(|e| McpError::Transport {
        server: server_name.to_string(),
        source: format!("SSE server returned an unresolvable endpoint '{endpoint}': {e}"),
    })
}

/// Build a legacy MCP HTTP+SSE (2024-11-05 spec) client and complete the
/// handshake. This is a DIFFERENT wire protocol from Streamable HTTP: the
/// client GETs the SSE URL, the server replies with an `endpoint` event whose
/// data is a session-scoped POST URL, and JSON-RPC then flows client→server via
/// POST to that URL and server→client over the open SSE `message` stream.
///
/// rmcp 1.5 retired its standalone SSE client (only the `client-side-sse` frame
/// parser remains), and routing `Sse` through the Streamable HTTP client fails
/// because that client POSTs `initialize` directly and the legacy server rejects
/// it with `400 session_id is required`. So we hand-roll the handshake and
/// bridge it into rmcp's generic `(Sink, Stream)` serve path — exactly like
/// `build_ws_client`.
///
/// SSRF is enforced twice: on the GET URL, and again on the server-provided
/// `endpoint` URL before the first POST (server-controlled data is a fresh
/// outbound surface the GET gate never saw).
pub async fn build_sse_client(cfg: &McpServerConfig, url: &str) -> McpResult<ConnectedClient> {
    use futures_util::StreamExt;
    use sse_stream::SseStream;

    let expanded_url = expand_placeholders(url, |name| std::env::var(name).ok());
    // SSRF gate #1: the GET URL, before any dial.
    ssrf_gate_url(cfg, &expanded_url).await?;

    // One reqwest client shared by the long-lived GET stream and the outbound
    // POSTs. Auth headers (OAuth Bearer / user-pinned) are shared by both.
    // `build_mcp_http_client` bypasses the system proxy for loopback / private
    // targets so a proxy set for remote APIs can't hijack a local SSE server.
    let auth_headers = authorized_headers(cfg).await?;
    let mut header_map = reqwest::header::HeaderMap::with_capacity(auth_headers.len() + 1);
    for (name, value) in auth_headers {
        header_map.insert(name, value);
    }
    let client = build_mcp_http_client(&expanded_url, &cfg.name)?;

    // 1) Open the SSE stream.
    let response = client
        .get(&expanded_url)
        .headers(header_map.clone())
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| classify_network_error(&cfg.name, "SSE handshake", e))?;

    // Box-pin the `!Unpin` parser so it can be driven by `.next()` here and then
    // handed to `filter_map` without pin plumbing leaking into the Stream half.
    let mut sse = Box::pin(SseStream::from_byte_stream(response.bytes_stream()));

    // 2) Read up to the `endpoint` event to learn the session-scoped POST URL.
    //    The whole handshake runs under `do_connect`'s connect_timeout, so a
    //    server that never sends `endpoint` surfaces as a Timeout, not a hang.
    let endpoint_data = loop {
        match sse.next().await {
            Some(Ok(frame)) if frame.event.as_deref() == Some("endpoint") => {
                break frame.data.unwrap_or_default();
            }
            // Keep-alive comments / other pre-handshake frames: ignore and wait.
            Some(Ok(_)) => continue,
            Some(Err(e)) => {
                return Err(McpError::Transport {
                    server: cfg.name.clone(),
                    source: format!("SSE handshake failed reading endpoint event: {e}"),
                })
            }
            None => {
                return Err(McpError::Transport {
                    server: cfg.name.clone(),
                    source: "SSE stream closed before sending an endpoint event".into(),
                })
            }
        }
    };

    // 3) Resolve the endpoint (usually relative) against the SSE base, then SSRF
    //    gate #2 — the endpoint is server-controlled and could point at an
    //    internal host the GET gate never vetted.
    let endpoint = resolve_sse_endpoint(&expanded_url, &endpoint_data, &cfg.name)?;
    ssrf_gate_url(cfg, endpoint.as_str()).await?;
    let endpoint = endpoint.to_string();

    // 4) Stream half (server→client): remaining SSE `message` frames parsed as
    //    JSON-RPC. Non-message frames (later endpoint pings, comments) are dropped.
    let stream_server = cfg.name.clone();
    let stream = sse.filter_map(move |frame| {
        let server = stream_server.clone();
        async move {
            let frame = frame.ok()?;
            // Per the SSE spec a missing / empty `event:` field defaults to "message".
            if !matches!(frame.event.as_deref(), None | Some("") | Some("message")) {
                return None;
            }
            let data = frame.data?;
            match serde_json::from_str::<rmcp::service::RxJsonRpcMessage<RoleClient>>(&data) {
                Ok(message) => Some(message),
                Err(e) => {
                    crate::app_warn!(
                        "mcp",
                        &format!("{server}:sse"),
                        "dropping unparseable SSE message frame: {e}"
                    );
                    None
                }
            }
        }
    });
    let stream: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = rmcp::service::RxJsonRpcMessage<RoleClient>> + Send>,
    > = Box::pin(stream);

    // 5) Sink half (client→server): each JSON-RPC message is POSTed to the
    //    session endpoint. `Sink::start_send` is sync but the POST is async, so
    //    route through a bounded channel drained by a background task and expose
    //    the sender as a `Sink` via `PollSender` (natural backpressure). The task
    //    exits when the sink is dropped (rmcp closing the transport).
    let (tx, mut rx) =
        tokio::sync::mpsc::channel::<rmcp::service::TxJsonRpcMessage<RoleClient>>(64);
    let post_client = client;
    let post_headers = header_map;
    let post_server = cfg.name.clone();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let body = match serde_json::to_vec(&message) {
                Ok(body) => body,
                Err(e) => {
                    crate::app_warn!(
                        "mcp",
                        &format!("{post_server}:sse"),
                        "failed to serialize outbound SSE message: {e}"
                    );
                    continue;
                }
            };
            let result = post_client
                .post(&endpoint)
                .headers(post_headers.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await;
            match result {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => crate::app_warn!(
                    "mcp",
                    &format!("{post_server}:sse"),
                    "SSE message POST rejected with HTTP {}",
                    response.status()
                ),
                Err(e) => crate::app_warn!(
                    "mcp",
                    &format!("{post_server}:sse"),
                    "SSE message POST failed: {e}"
                ),
            }
        }
    });
    let sink = tokio_util::sync::PollSender::new(tx);

    let running = ()
        .serve((sink, stream))
        .await
        .map_err(|e| classify_network_error(&cfg.name, "SSE handshake", e))?;
    Ok(ConnectedClient {
        running,
        stderr: None,
    })
}

/// Entry point used by `client::do_connect`. Dispatches on the transport
/// kind, runs any gating policy (SSRF), constructs the appropriate
/// rmcp transport, and returns a connected client ready for
/// `list_tools` / `call_tool` round-trips.
pub async fn build_transport_for(cfg: &McpServerConfig) -> McpResult<ConnectedClient> {
    match &cfg.transport {
        McpTransportSpec::Stdio { .. } => build_stdio_client(cfg).await,
        McpTransportSpec::StreamableHttp { url } => build_http_client(cfg, url).await,
        McpTransportSpec::Sse { url } => build_sse_client(cfg, url).await,
        McpTransportSpec::WebSocket { url } => build_ws_client(cfg, url).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::config::{McpServerConfig, McpTransportSpec, McpTrustLevel};

    fn stdio_cfg(command: &str) -> McpServerConfig {
        McpServerConfig {
            id: "id-1".into(),
            name: "t".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: command.into(),
                args: vec!["-x".into(), "${FOO}".into()],
                cwd: None,
            },
            env: [("FOO".into(), "from-env".into())].into_iter().collect(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    #[test]
    fn build_stdio_command_expands_args_from_env_block() {
        let cfg = stdio_cfg("echo");
        let cmd = build_stdio_command(&cfg).unwrap();
        // `std::process::Command` (via tokio wrapper) doesn't expose
        // its argv publicly; we use `get_args()` on the std command.
        let std_cmd: &std::process::Command = cmd.as_std();
        let args: Vec<_> = std_cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["-x".to_string(), "from-env".to_string()]);
    }

    #[test]
    fn build_stdio_command_whitelists_env() {
        let cfg = stdio_cfg("echo");
        let cmd = build_stdio_command(&cfg).unwrap();
        let std_cmd: &std::process::Command = cmd.as_std();
        let envs: std::collections::HashMap<String, Option<String>> = std_cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|s| s.to_string_lossy().into_owned()),
                )
            })
            .collect();
        // FOO must have been passed through.
        assert!(envs.contains_key("FOO"));
        // A variable that is NOT in the whitelist and NOT in cfg.env
        // should not have been forwarded. We use `PWD` as a probe since
        // it's almost always present in the parent env but we deliberately
        // left it off the whitelist.
        assert!(!envs.contains_key("PWD"));
    }

    #[tokio::test]
    async fn websocket_transport_honors_ssrf_policy() {
        // Untrusted + private-network ws:// URL → Blocked. Exercises
        // the ws→http scheme rewrite on the way into
        // `security::ssrf::check_url`.
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::WebSocket {
            url: "ws://127.0.0.1:9999/mcp".into(),
        };
        cfg.trust_level = McpTrustLevel::Untrusted;
        match build_transport_for(&cfg).await {
            Err(McpError::Blocked { .. }) => {}
            Err(other) => panic!("expected Blocked, got: {other:?}"),
            Ok(_) => panic!("expected Blocked for private ws URL under Strict policy"),
        }
    }

    #[test]
    fn ws_to_http_equiv_rewrites_scheme() {
        assert_eq!(
            ws_to_http_equiv("ws://example.com:9000/mcp").unwrap(),
            "http://example.com:9000/mcp"
        );
        assert_eq!(
            ws_to_http_equiv("wss://example.com/").unwrap(),
            "https://example.com/"
        );
        assert!(ws_to_http_equiv("http://example.com/").is_err());
    }

    #[tokio::test]
    async fn http_transport_honors_ssrf_policy() {
        // Untrusted + private-network URL → Blocked. This guards the
        // regression where the SSRF gate was skipped on MCP dial-out.
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::StreamableHttp {
            url: "http://127.0.0.1:9999/mcp".into(),
        };
        cfg.trust_level = McpTrustLevel::Untrusted;
        match build_transport_for(&cfg).await {
            Err(McpError::Blocked { .. }) => {}
            Err(other) => panic!("expected Blocked, got: {other:?}"),
            Ok(_) => panic!("expected Blocked for private URL under Strict policy"),
        }
    }

    #[tokio::test]
    async fn sse_transport_honors_ssrf_policy() {
        // Untrusted + private-network SSE GET URL → Blocked, before any dial.
        // The server-provided endpoint is re-gated separately (integration),
        // but the GET URL must be gated up front like HTTP/WS.
        let mut cfg = stdio_cfg("echo");
        cfg.transport = McpTransportSpec::Sse {
            url: "http://127.0.0.1:9999/sse".into(),
        };
        cfg.trust_level = McpTrustLevel::Untrusted;
        match build_transport_for(&cfg).await {
            Err(McpError::Blocked { .. }) => {}
            Err(other) => panic!("expected Blocked, got: {other:?}"),
            Ok(_) => panic!("expected Blocked for private SSE URL under Strict policy"),
        }
    }

    #[test]
    fn resolve_sse_endpoint_relative_and_absolute() {
        // Relative endpoint (the common shape) keeps the base origin + carries
        // the session_id query the legacy handshake mints.
        let resolved =
            resolve_sse_endpoint("https://host.example/sse", "/messages?session_id=abc", "t")
                .unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://host.example/messages?session_id=abc"
        );

        // Relative without a leading slash resolves against the base path dir.
        let resolved =
            resolve_sse_endpoint("https://host.example/mcp/sse", "message?session_id=x", "t")
                .unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://host.example/mcp/message?session_id=x"
        );

        // An absolute endpoint is honored verbatim (it is still SSRF re-gated
        // by the caller before any POST).
        let resolved = resolve_sse_endpoint(
            "https://host.example/sse",
            "https://other.example/inbox?session_id=z",
            "t",
        )
        .unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://other.example/inbox?session_id=z"
        );
    }

    #[test]
    fn host_bypasses_proxy_covers_loopback_and_private() {
        // Loopback / localhost → bypass the proxy (local MCP servers).
        for host in [
            "localhost",
            "app.localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "[::1]",
        ] {
            assert!(host_bypasses_proxy(host), "{host} should bypass proxy");
        }
        // Private / link-local ranges (LAN MCP servers) → bypass.
        for host in [
            "10.0.0.5",
            "192.168.1.20",
            "172.16.0.1",
            "172.31.255.254",
            "169.254.1.1",
            "fd00::1", // unique-local IPv6
            "fe80::1", // link-local IPv6
        ] {
            assert!(host_bypasses_proxy(host), "{host} should bypass proxy");
        }
        // Public hosts → keep the proxy (it may be how the user reaches them).
        for host in [
            "example.com",
            "api.openai.com",
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",   // just outside 172.16/12
            "2606:4700::1", // public IPv6
        ] {
            assert!(!host_bypasses_proxy(host), "{host} should keep proxy");
        }
    }
}

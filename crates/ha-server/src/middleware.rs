use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const BROWSER_SESSION_COOKIE: &str = "ha_session";
pub const EVENT_TICKET_PROTOCOL_PREFIX: &str = "ha-events.";
const MAX_BOUND_FILE_TICKETS: usize = 4_096;

#[derive(Clone)]
pub struct BoundFile {
    pub path: PathBuf,
    pub download: bool,
    pub mime: String,
    pub file: Arc<File>,
    pub len: u64,
}

#[derive(Clone)]
struct BoundFileEntry {
    resource: BoundFile,
    expires_at: u64,
}

/// Shared owner-token authentication state. The root token can be rotated
/// without restarting the server; browser sessions are stateless HMAC tokens
/// derived from it, so rotation invalidates every prior session immediately.
#[derive(Clone)]
pub struct AuthState {
    owner_token: Arc<RwLock<Option<String>>>,
    access_ticket_key: Arc<RwLock<[u8; 32]>>,
    owner_changes: tokio::sync::watch::Sender<u64>,
    knowledge_agent_read_token: Option<String>,
    externally_managed: bool,
    login_failures: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
    bound_file_tickets: Arc<Mutex<HashMap<String, BoundFileEntry>>>,
}

impl AuthState {
    pub fn new(
        owner_token: Option<String>,
        knowledge_agent_read_token: Option<String>,
        externally_managed: bool,
    ) -> Self {
        let (owner_changes, _) = tokio::sync::watch::channel(0);
        Self {
            owner_token: Arc::new(RwLock::new(owner_token.filter(|token| !token.is_empty()))),
            access_ticket_key: Arc::new(RwLock::new(
                ha_core::server_auth::generate_access_ticket_signing_key(),
            )),
            owner_changes,
            knowledge_agent_read_token,
            externally_managed,
            login_failures: Arc::new(Mutex::new(HashMap::new())),
            bound_file_tickets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn auth_required(&self) -> bool {
        match self.owner_token.read() {
            Ok(token) => token.as_deref().is_some_and(|token| !token.is_empty()),
            // Authentication state corruption must never open the protected
            // router. Follow-up checks will reject every credential.
            Err(_) => true,
        }
    }

    pub fn externally_managed(&self) -> bool {
        self.externally_managed
    }

    pub fn owner_fingerprint(&self) -> Option<String> {
        self.owner_token.read().ok().and_then(|token| {
            token
                .as_deref()
                .map(ha_core::server_auth::token_fingerprint)
        })
    }

    pub fn check_owner_token(&self, candidate: &[u8]) -> bool {
        self.owner_token.read().ok().is_some_and(|token| {
            token
                .as_deref()
                .is_some_and(|owner| constant_time_eq(candidate, owner.as_bytes()))
        })
    }

    pub fn create_browser_session(&self, ttl_secs: u64) -> anyhow::Result<String> {
        let owner = self
            .owner_token
            .read()
            .map_err(|_| anyhow::anyhow!("owner-token state is unavailable"))?;
        let owner = owner
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("owner-token authentication is disabled"))?;
        ha_core::server_auth::create_browser_session(owner, ttl_secs, unix_time())
    }

    pub fn check_browser_session(&self, session: &str) -> bool {
        self.owner_token.read().ok().is_some_and(|token| {
            token.as_deref().is_some_and(|owner| {
                ha_core::server_auth::verify_browser_session(owner, session, unix_time())
            })
        })
    }

    pub fn create_access_ticket(&self, scope: &str, ttl_secs: u64) -> anyhow::Result<String> {
        let owner = self
            .owner_token
            .read()
            .map_err(|_| anyhow::anyhow!("owner-token state is unavailable"))?;
        if owner.as_deref().is_none_or(str::is_empty) {
            anyhow::bail!("owner-token authentication is disabled");
        }
        let signing_key = self
            .access_ticket_key
            .read()
            .map_err(|_| anyhow::anyhow!("access-ticket signing state is unavailable"))?;
        ha_core::server_auth::create_scoped_access_ticket(
            signing_key.as_ref(),
            scope,
            ttl_secs,
            unix_time(),
        )
    }

    pub fn create_transport_access_tickets(
        &self,
        ttl_secs: u64,
    ) -> anyhow::Result<(String, String)> {
        let owner = self
            .owner_token
            .read()
            .map_err(|_| anyhow::anyhow!("owner-token state is unavailable"))?;
        if owner.as_deref().is_none_or(str::is_empty) {
            anyhow::bail!("owner-token authentication is disabled");
        }
        let signing_key = self
            .access_ticket_key
            .read()
            .map_err(|_| anyhow::anyhow!("access-ticket signing state is unavailable"))?;
        let now = unix_time();
        let resources = ha_core::server_auth::create_scoped_access_ticket(
            signing_key.as_ref(),
            "resources",
            ttl_secs,
            now,
        )?;
        let events = ha_core::server_auth::create_scoped_access_ticket(
            signing_key.as_ref(),
            "events",
            ttl_secs,
            now,
        )?;
        Ok((resources, events))
    }

    pub fn check_access_ticket(&self, ticket: &str, scope: &str) -> bool {
        let Ok(owner) = self.owner_token.read() else {
            return false;
        };
        if owner.as_deref().is_none_or(str::is_empty) {
            return false;
        }
        let Ok(signing_key) = self.access_ticket_key.read() else {
            return false;
        };
        ha_core::server_auth::verify_scoped_access_ticket(
            signing_key.as_ref(),
            ticket,
            scope,
            unix_time(),
        )
    }

    /// Mint a short-lived capability bound to an open handle for one
    /// already-authorized canonical file. The stable handle prevents a
    /// writable workspace from swapping the path to a symlink or a different
    /// inode between ticket creation and the later public GET.
    pub async fn create_bound_file_ticket(
        &self,
        path: PathBuf,
        download: bool,
        ttl_secs: u64,
    ) -> anyhow::Result<String> {
        let open_path = path.clone();
        let (file, metadata, mime) = tokio::task::spawn_blocking(move || {
            let file = open_bound_file(&open_path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "bound path is not a regular file",
                ));
            }
            let mime = mime_for_open_bound_file(&open_path, &file);
            Ok::<_, std::io::Error>((file, metadata, mime))
        })
        .await
        .map_err(|error| anyhow::anyhow!("bound file-open task failed: {error}"))??;
        let ticket = self.create_access_ticket("bound_file", ttl_secs)?;
        let now = unix_time();
        let expires_at = now.saturating_add(ttl_secs.clamp(60, 60 * 60));
        let mut tickets = self
            .bound_file_tickets
            .lock()
            .map_err(|_| anyhow::anyhow!("bound file-ticket state is unavailable"))?;
        tickets.retain(|_, entry| entry.expires_at >= now);
        if tickets.len() >= MAX_BOUND_FILE_TICKETS {
            anyhow::bail!("too many active bound file tickets");
        }
        tickets.insert(
            ticket.clone(),
            BoundFileEntry {
                resource: BoundFile {
                    path,
                    download,
                    mime,
                    file: Arc::new(file),
                    len: metadata.len(),
                },
                expires_at,
            },
        );
        drop(tickets);

        // Expired capabilities own OS file handles, so unlike ordinary signed
        // tickets they must be removed even if no later request happens to
        // trigger lazy pruning.
        let expiring_ticket = ticket.clone();
        let bound_file_tickets = self.bound_file_tickets.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(expires_at.saturating_sub(now))).await;
            if let Ok(mut tickets) = bound_file_tickets.lock() {
                let should_remove = tickets
                    .get(&expiring_ticket)
                    .is_some_and(|entry| entry.expires_at <= unix_time());
                if should_remove {
                    tickets.remove(&expiring_ticket);
                }
            }
        });
        Ok(ticket)
    }

    pub fn resolve_bound_file_ticket(&self, ticket: &str) -> Option<BoundFile> {
        if !self.check_access_ticket(ticket, "bound_file") {
            return None;
        }
        let now = unix_time();
        let mut tickets = self.bound_file_tickets.lock().ok()?;
        tickets.retain(|_, entry| entry.expires_at >= now);
        tickets.get(ticket).map(|entry| entry.resource.clone())
    }

    pub fn headers_are_owner_authenticated(&self, headers: &axum::http::HeaderMap) -> bool {
        if !self.auth_required() {
            return true;
        }
        bearer_header_token(headers)
            .as_deref()
            .is_some_and(|token| self.check_owner_token(token))
            || browser_session_cookie_value(headers)
                .as_deref()
                .is_some_and(|session| self.check_browser_session(session))
    }

    pub fn replace_owner_token(&self, token: Option<String>) -> anyhow::Result<()> {
        let mut owner = self
            .owner_token
            .write()
            .map_err(|_| anyhow::anyhow!("owner-token state is unavailable"))?;
        let mut signing_key = self
            .access_ticket_key
            .write()
            .map_err(|_| anyhow::anyhow!("access-ticket signing state is unavailable"))?;
        *owner = token.filter(|value| !value.is_empty());
        *signing_key = ha_core::server_auth::generate_access_ticket_signing_key();
        if let Ok(mut tickets) = self.bound_file_tickets.lock() {
            tickets.clear();
        }
        self.owner_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        Ok(())
    }

    /// Subscribe before revalidating a WebSocket upgrade so a concurrent
    /// Owner Token replacement cannot leave an authenticated event stream
    /// alive under the previous credential.
    pub fn subscribe_owner_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.owner_changes.subscribe()
    }

    pub fn login_allowed(&self, peer: IpAddr) -> bool {
        let Ok(mut by_peer) = self.login_failures.lock() else {
            return false;
        };
        let cutoff = Instant::now() - Duration::from_secs(60);
        let Some(failures) = by_peer.get_mut(&peer) else {
            return true;
        };
        while failures
            .front()
            .is_some_and(|failed_at| *failed_at < cutoff)
        {
            failures.pop_front();
        }
        failures.len() < 10
    }

    pub fn record_login_failure(&self, peer: IpAddr) {
        if let Ok(mut by_peer) = self.login_failures.lock() {
            if by_peer.len() >= 2_048 && !by_peer.contains_key(&peer) {
                by_peer.retain(|_, failures| {
                    failures
                        .back()
                        .is_some_and(|failed_at| failed_at.elapsed() < Duration::from_secs(60))
                });
                if by_peer.len() >= 2_048 {
                    return;
                }
            }
            let failures = by_peer.entry(peer).or_default();
            failures.push_back(Instant::now());
        }
    }

    pub fn clear_login_failures(&self, peer: IpAddr) {
        if let Ok(mut by_peer) = self.login_failures.lock() {
            by_peer.remove(&peer);
        }
    }
}

fn open_bound_file(path: &std::path::Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);

    // The authorization layer hands us a canonical regular-file path. Refuse
    // a last-component link introduced in the narrow hand-off window before
    // opening the stable capability handle.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    options.open(path)
}

fn mime_for_open_bound_file(path: &std::path::Path, file: &File) -> String {
    let ext = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if let Some(mime) = ext
        .as_deref()
        .and_then(ha_core::attachments::mime_from_extension)
    {
        return mime.to_string();
    }

    let mut head = vec![0_u8; 512];
    let read = file
        .try_clone()
        .and_then(|mut cloned| cloned.read(&mut head))
        .unwrap_or(0);
    head.truncate(read);
    ha_core::attachments::sniff_mime(&head, path)
}

fn active_auth_state() -> &'static RwLock<Option<AuthState>> {
    static ACTIVE: OnceLock<RwLock<Option<AuthState>>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(None))
}

pub fn register_active_auth_state(state: AuthState) {
    if let Ok(mut active) = active_auth_state().write() {
        *active = Some(state);
    }
}

pub fn replace_active_owner_token(token: Option<String>) -> anyhow::Result<()> {
    let active = active_auth_state()
        .read()
        .map_err(|_| anyhow::anyhow!("active authentication state is unavailable"))?;
    if let Some(state) = active.as_ref() {
        state.replace_owner_token(token)?;
    }
    Ok(())
}

/// Return non-secret runtime auth metadata for settings validation.
pub fn active_auth_status() -> anyhow::Result<Option<(bool, bool)>> {
    let active = active_auth_state()
        .read()
        .map_err(|_| anyhow::anyhow!("active authentication state is unavailable"))?;
    Ok(active
        .as_ref()
        .map(|state| (state.auth_required(), state.externally_managed())))
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Constant-time byte comparison. Guards against timing side-channels when
/// comparing owner tokens — never use `==` for secret comparisons. A length
/// mismatch short-circuits to `false`; equal-length inputs XOR-fold into a
/// single byte to produce a branch-free answer.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Middleware that validates requests against an optional Owner Token.
///
/// - If `api_key` is `None`, all requests pass through (no-auth mode).
/// - If `api_key` is `Some`, checks in order:
///   1. `Authorization: Bearer <token>` header (for HTTP requests)
///   2. Signed HttpOnly browser-session cookie (HTTP, media, and WebSocket).
/// - All comparisons are constant-time to avoid timing side-channels.
/// - Returns 401 on failure.
pub async fn require_api_key(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    if !state.auth_required() {
        // A scoped Knowledge Agent token only makes sense alongside owner API-key
        // protection. Without an owner key the server is intentionally in no-auth
        // mode; do not let a read token alone lock every other endpoint into an
        // inaccessible state.
        return next.run(request).await;
    }
    let path = request.uri().path().to_string();
    if let Some(token) = bearer_token(&request) {
        if state.check_owner_token(&token) {
            return next.run(request).await;
        }
        if let Some(read_token) = state
            .knowledge_agent_read_token
            .as_deref()
            .filter(|token| !token.is_empty())
        {
            if constant_time_eq(&token, read_token.as_bytes()) {
                if is_knowledge_agent_read_path(&path) {
                    return next.run(request).await;
                } else {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({
                            "error": "Forbidden: knowledge agent read token can only access read-only /api/knowledge/agent endpoints"
                        })),
                    )
                        .into_response();
                }
            }
        }
    }
    if browser_session_cookie(&request)
        .as_deref()
        .is_some_and(|session| state.check_browser_session(session))
        && cookie_origin_is_safe(&request)
    {
        return next.run(request).await;
    }
    if path == "/ws/events"
        && event_ticket_from_headers(request.headers())
            .as_deref()
            .is_some_and(|ticket| state.check_access_ticket(ticket, "events"))
    {
        return next.run(request).await;
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Unauthorized: invalid or missing owner token" })),
    )
        .into_response()
}

fn bearer_token(request: &Request) -> Option<Vec<u8>> {
    bearer_header_token(request.headers())
}

fn bearer_header_token(headers: &axum::http::HeaderMap) -> Option<Vec<u8>> {
    if let Some(auth_header) = headers.get("authorization") {
        if let Ok(value) = auth_header.to_str() {
            if let Some((scheme, token)) = value.split_once(' ') {
                if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
                    return None;
                }
                return Some(token.as_bytes().to_vec());
            }
        }
    }
    None
}

pub fn event_ticket_protocol(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| {
            protocols.split(',').find_map(|protocol| {
                let protocol = protocol.trim();
                protocol
                    .strip_prefix(EVENT_TICKET_PROTOCOL_PREFIX)
                    .filter(|ticket| !ticket.is_empty())
                    .map(|_| protocol.to_string())
            })
        })
}

fn event_ticket_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    event_ticket_protocol(headers).and_then(|protocol| {
        protocol
            .strip_prefix(EVENT_TICKET_PROTOCOL_PREFIX)
            .map(str::to_string)
    })
}

pub fn browser_session_cookie(request: &Request) -> Option<String> {
    browser_session_cookie_value(request.headers())
}

fn browser_session_cookie_value(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == BROWSER_SESSION_COOKIE).then(|| value.to_string())
            })
        })
}

fn cookie_origin_is_safe(request: &Request) -> bool {
    let Some(origin) = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = request
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|uri| {
            uri.authority()
                .map(|authority| authority.as_str().to_string())
        })
        .is_some_and(|authority| authority.eq_ignore_ascii_case(host))
}

fn is_knowledge_agent_read_path(path: &str) -> bool {
    matches!(
        path,
        "/api/knowledge/agent/search"
            | "/api/knowledge/agent/read"
            | "/api/knowledge/agent/expand"
            | "/api/knowledge/agent/sources"
    )
}

/// Per-request access log. Query strings are intentionally never logged.
pub async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = redact_access_path(request.uri().path());
    let start = std::time::Instant::now();
    let response = next.run(request).await;
    ha_core::app_info!(
        "http",
        "access",
        "{} {} {} {}ms",
        response.status().as_u16(),
        method,
        path,
        start.elapsed().as_millis()
    );
    response
}

pub async fn security_headers(request: Request, next: Next) -> Response {
    let scoped_resource = request.uri().path().starts_with("/api/resource/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    let content_security_policy = if scoped_resource {
        // Scoped resources are the only surface intentionally frameable by a
        // cross-origin Tauri/remote GUI. The short-lived read-only capability
        // is the authorization boundary; iframe callers still apply sandbox.
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https: http:; font-src 'self' data:; media-src 'self' data: blob:; connect-src 'self' ws: wss:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'"
    } else {
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https: http:; font-src 'self' data:; media-src 'self' data: blob:; connect-src 'self' ws: wss:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'"
    };
    let defaults = [
        ("content-security-policy", content_security_policy),
        ("x-content-type-options", "nosniff"),
        ("strict-transport-security", "max-age=31536000"),
        ("referrer-policy", "no-referrer"),
        (
            "permissions-policy",
            "camera=(), geolocation=(), microphone=(self)",
        ),
    ];
    for (name, value) in defaults {
        if !headers.contains_key(name) {
            if let (Ok(name), Ok(value)) = (
                name.parse::<axum::http::HeaderName>(),
                value.parse::<axum::http::HeaderValue>(),
            ) {
                headers.insert(name, value);
            }
        }
    }
    if !scoped_resource && !headers.contains_key("x-frame-options") {
        headers.insert(
            "x-frame-options",
            axum::http::HeaderValue::from_static("SAMEORIGIN"),
        );
    }
    response
}

fn redact_access_path(path: &str) -> String {
    const RESOURCE_PREFIX: &str = "/api/resource/";
    if let Some(ticket_and_path) = path.strip_prefix(RESOURCE_PREFIX) {
        if let Some((_, resource_path)) = ticket_and_path.split_once('/') {
            return format!("{RESOURCE_PREFIX}[redacted]/{resource_path}");
        }
        return format!("{RESOURCE_PREFIX}[redacted]");
    }
    const PREFIX: &str = "/api/pets/import/previews/";
    const SUFFIX: &str = "/thumbnail";
    if let Some(token_and_suffix) = path.strip_prefix(PREFIX) {
        if let Some(token) = token_and_suffix.strip_suffix(SUFFIX) {
            if !token.contains('/') {
                return format!("{PREFIX}[redacted]{SUFFIX}");
            }
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::{get, post};
    use axum::Router;
    use tower::ServiceExt;

    #[test]
    fn constant_time_eq_matches_equal_inputs() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_rejects_unequal_length() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abc", b""));
    }

    #[test]
    fn constant_time_eq_rejects_different_content() {
        assert!(!constant_time_eq(b"abc", b"abd"));
    }

    #[test]
    fn knowledge_agent_read_token_paths_are_exact() {
        assert!(is_knowledge_agent_read_path("/api/knowledge/agent/search"));
        assert!(is_knowledge_agent_read_path("/api/knowledge/agent/sources"));
        assert!(!is_knowledge_agent_read_path(
            "/api/knowledge/agent/compile/propose"
        ));
        assert!(!is_knowledge_agent_read_path("/api/knowledge"));
        assert!(!is_knowledge_agent_read_path(
            "/api/knowledge/agent/search/extra"
        ));
    }

    #[test]
    fn access_log_redacts_pet_preview_capabilities() {
        assert_eq!(
            redact_access_path("/api/pets/import/previews/secret-token/thumbnail"),
            "/api/pets/import/previews/[redacted]/thumbnail"
        );
        assert_eq!(
            redact_access_path("/api/pets/import/preview/cancel"),
            "/api/pets/import/preview/cancel"
        );
    }

    #[test]
    fn access_log_redacts_scoped_resource_tickets() {
        assert_eq!(
            redact_access_path("/api/resource/v1.resources.secret/canvas/projects/p1/index.html"),
            "/api/resource/[redacted]/canvas/projects/p1/index.html"
        );
    }

    #[test]
    fn websocket_event_ticket_uses_a_subprotocol_not_the_url() {
        let auth = AuthState::new(Some("owner-token".into()), None, false);
        let ticket = auth.create_access_ticket("events", 900).unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "sec-websocket-protocol",
            format!("chat, {EVENT_TICKET_PROTOCOL_PREFIX}{ticket}")
                .parse()
                .unwrap(),
        );
        assert_eq!(
            event_ticket_from_headers(&headers).as_deref(),
            Some(ticket.as_str())
        );
        assert!(auth.check_access_ticket(&ticket, "events"));
        assert!(!auth.check_access_ticket(&ticket, "resources"));
    }

    #[tokio::test]
    async fn owner_token_replacement_revokes_existing_event_streams() {
        let auth = AuthState::new(Some("owner-token".into()), None, false);
        let mut owner_changes = auth.subscribe_owner_changes();
        let old_ticket = auth.create_access_ticket("events", 900).unwrap();

        auth.replace_owner_token(Some("new-owner-token".into()))
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), owner_changes.changed())
            .await
            .expect("owner-token change notification")
            .expect("owner-token watch remains open");
        assert!(!auth.check_access_ticket(&old_ticket, "events"));
        let new_ticket = auth.create_access_ticket("events", 900).unwrap();
        assert!(auth.check_access_ticket(&new_ticket, "events"));
    }

    #[tokio::test]
    async fn bound_file_tickets_are_handle_bound_and_revoked_on_owner_rotation() {
        let auth = AuthState::new(Some("owner-token".into()), None, false);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("visible.html");
        std::fs::write(&path, "visible").unwrap();
        let ticket = auth
            .create_bound_file_ticket(path.clone(), false, 900)
            .await
            .unwrap();

        let bound = auth.resolve_bound_file_ticket(&ticket).unwrap();
        assert_eq!(bound.path, path);
        assert!(!bound.download);
        assert_eq!(bound.mime, "text/html");
        assert_eq!(bound.len, 7);
        assert!(auth
            .resolve_bound_file_ticket(&auth.create_access_ticket("resources", 900).unwrap())
            .is_none());

        auth.replace_owner_token(Some("rotated-owner-token".into()))
            .unwrap();
        assert!(auth.resolve_bound_file_ticket(&ticket).is_none());
    }

    #[tokio::test]
    async fn scoped_resources_can_be_framed_but_owner_pages_cannot() {
        let app = Router::new()
            .route(
                "/api/resource/ticket/canvas/index.html",
                get(|| async { "ok" }),
            )
            .route("/settings", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(security_headers));

        let resource = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/resource/ticket/canvas/index.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resource.headers().get("x-frame-options").is_none());
        assert!(!resource
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("frame-ancestors"));

        let owner = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(owner.headers()["x-frame-options"], "SAMEORIGIN");
        assert!(owner.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'self'"));
    }

    #[tokio::test]
    async fn read_token_allows_knowledge_agent_read_path() {
        let app = auth_test_router();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/search")
                    .header("authorization", "Bearer read-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn read_token_cannot_call_compile_propose() {
        let app = auth_test_router();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/compile/propose")
                    .header("authorization", "Bearer read-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn owner_token_can_call_compile_propose() {
        let app = auth_test_router();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/compile/propose")
                    .header("authorization", "Bearer owner-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn read_token_without_owner_key_keeps_no_auth_mode() {
        let app = auth_test_router_with(None, Some("read-token"));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/compile/propose")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn query_string_owner_token_is_rejected() {
        let response = auth_test_router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/search?token=owner-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn signed_same_origin_browser_session_is_accepted() {
        let auth_state = AuthState::new(Some("owner-token".into()), None, false);
        let session = auth_state.create_browser_session(3_600).unwrap();
        let response = auth_test_router_with_state(auth_state)
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/search")
                    .header("host", "localhost:8420")
                    .header("origin", "http://localhost:8420")
                    .header("cookie", format!("{BROWSER_SESSION_COOKIE}={session}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn browser_session_rejects_cross_origin_requests() {
        let auth_state = AuthState::new(Some("owner-token".into()), None, false);
        let session = auth_state.create_browser_session(3_600).unwrap();
        let response = auth_test_router_with_state(auth_state)
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/knowledge/agent/search")
                    .header("host", "localhost:8420")
                    .header("origin", "https://attacker.example")
                    .header("cookie", format!("{BROWSER_SESSION_COOKIE}={session}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn responses_receive_browser_security_headers() {
        let response = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(security_headers))
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert!(response.headers().contains_key("content-security-policy"));
        assert!(response.headers().contains_key("referrer-policy"));
    }

    fn auth_test_router() -> Router {
        auth_test_router_with(Some("owner-token"), Some("read-token"))
    }

    fn auth_test_router_with(
        api_key: Option<&str>,
        knowledge_agent_read_token: Option<&str>,
    ) -> Router {
        let auth_state = AuthState::new(
            api_key.map(str::to_string),
            knowledge_agent_read_token.map(str::to_string),
            false,
        );
        auth_test_router_with_state(auth_state)
    }

    fn auth_test_router_with_state(auth_state: AuthState) -> Router {
        Router::new()
            .route("/api/knowledge/agent/search", post(|| async { "ok" }))
            .route(
                "/api/knowledge/agent/compile/propose",
                post(|| async { "ok" }),
            )
            .route_layer(axum::middleware::from_fn_with_state(
                auth_state,
                require_api_key,
            ))
    }
}

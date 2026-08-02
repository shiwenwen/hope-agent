use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const BROWSER_SESSION_COOKIE: &str = "ha_session";

/// Shared owner-token authentication state. The root token can be rotated
/// without restarting the server; browser sessions are stateless HMAC tokens
/// derived from it, so rotation invalidates every prior session immediately.
#[derive(Clone)]
pub struct AuthState {
    owner_token: Arc<RwLock<Option<String>>>,
    knowledge_agent_read_token: Option<String>,
    externally_managed: bool,
    login_failures: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
}

impl AuthState {
    pub fn new(
        owner_token: Option<String>,
        knowledge_agent_read_token: Option<String>,
        externally_managed: bool,
    ) -> Self {
        Self {
            owner_token: Arc::new(RwLock::new(owner_token.filter(|token| !token.is_empty()))),
            knowledge_agent_read_token,
            externally_managed,
            login_failures: Arc::new(Mutex::new(HashMap::new())),
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
        *owner = token.filter(|value| !value.is_empty());
        Ok(())
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
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    let defaults = [
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https: http:; font-src 'self' data:; media-src 'self' data: blob:; connect-src 'self' ws: wss:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'",
        ),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "SAMEORIGIN"),
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
    response
}

fn redact_access_path(path: &str) -> String {
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

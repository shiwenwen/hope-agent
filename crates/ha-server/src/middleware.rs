use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// State for the API key authentication middleware.
#[derive(Clone)]
pub struct ApiKeyState {
    pub api_key: Option<String>,
    pub knowledge_agent_read_token: Option<String>,
}

/// Constant-time byte comparison. Guards against timing side-channels when
/// comparing API keys — never use `==` for secret comparisons. A length
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

/// `application/x-www-form-urlencoded` value decoder: treats `+` as space
/// and `%XX` as a byte; anything else passes through. Returns the raw
/// decoded bytes so comparison stays byte-for-byte.
fn percent_decode_form_value(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h = (bytes[i + 1] as char).to_digit(16);
                let l = (bytes[i + 2] as char).to_digit(16);
                match (h, l) {
                    (Some(h), Some(l)) => {
                        out.push((h as u8) * 16 + l as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Middleware that validates requests against an optional API key.
///
/// - If `api_key` is `None`, all requests pass through (no-auth mode).
/// - If `api_key` is `Some`, checks in order:
///   1. `Authorization: Bearer <token>` header (for HTTP requests)
///   2. `?token=<token>` query parameter (for browser WebSocket connections).
///      Values are percent-decoded so keys containing reserved characters
///      match correctly when the client URL-encodes them.
/// - All comparisons are constant-time to avoid timing side-channels.
/// - Returns 401 on failure.
pub async fn require_api_key(
    State(state): State<ApiKeyState>,
    request: Request,
    next: Next,
) -> Response {
    let owner_key = state.api_key.as_deref().filter(|k| !k.is_empty());
    if owner_key.is_none() {
        // A scoped Knowledge Agent token only makes sense alongside owner API-key
        // protection. Without an owner key the server is intentionally in no-auth
        // mode; do not let a read token alone lock every other endpoint into an
        // inaccessible state.
        return next.run(request).await;
    }
    let read_token = state
        .knowledge_agent_read_token
        .as_deref()
        .filter(|k| !k.is_empty());

    let path = request.uri().path().to_string();
    if let Some(token) = request_auth_token(&request) {
        if let Some(owner_key) = owner_key {
            if constant_time_eq(&token, owner_key.as_bytes()) {
                return next.run(request).await;
            }
        }
        if let Some(read_token) = read_token {
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

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Unauthorized: invalid or missing API key" })),
    )
        .into_response()
}

fn request_auth_token(request: &Request) -> Option<Vec<u8>> {
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(value) = auth_header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token.as_bytes().to_vec());
            }
        }
    }
    request.uri().query().and_then(|query| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix("token=").map(percent_decode_form_value))
    })
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

/// Per-request access log. Query string is intentionally dropped so the
/// `?token=<api-key>` WebSocket auth fallback can't leak into any log stream.
pub async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::post;
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
    fn percent_decode_handles_encoded_symbols() {
        assert_eq!(percent_decode_form_value("hello%20world"), b"hello world");
        assert_eq!(percent_decode_form_value("a%2Bb%3Dc"), b"a+b=c");
        assert_eq!(percent_decode_form_value("plain"), b"plain");
        // `+` decodes to space per application/x-www-form-urlencoded.
        assert_eq!(percent_decode_form_value("a+b"), b"a b");
    }

    #[test]
    fn percent_decode_tolerates_bad_sequences() {
        // Malformed `%Q1` must not crash; passes through as literal.
        assert_eq!(percent_decode_form_value("%Q1"), b"%Q1");
        // Trailing `%` with no digits passes through.
        assert_eq!(percent_decode_form_value("abc%"), b"abc%");
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

    fn auth_test_router() -> Router {
        auth_test_router_with(Some("owner-token"), Some("read-token"))
    }

    fn auth_test_router_with(
        api_key: Option<&str>,
        knowledge_agent_read_token: Option<&str>,
    ) -> Router {
        let auth_state = ApiKeyState {
            api_key: api_key.map(str::to_string),
            knowledge_agent_read_token: knowledge_agent_read_token.map(str::to_string),
        };
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

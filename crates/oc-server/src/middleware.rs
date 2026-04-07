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
}

/// Middleware that validates requests against an optional API key.
///
/// - If `api_key` is `None`, all requests pass through (no-auth mode).
/// - If `api_key` is `Some`, checks in order:
///   1. `Authorization: Bearer <token>` header (for HTTP requests)
///   2. `?token=<token>` query parameter (for browser WebSocket connections)
/// - Returns 401 on failure.
pub async fn require_api_key(
    State(state): State<ApiKeyState>,
    request: Request,
    next: Next,
) -> Response {
    let expected = match &state.api_key {
        Some(key) => key,
        None => return next.run(request).await,
    };

    // Check Authorization header first
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(value) = auth_header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                if token == expected {
                    return next.run(request).await;
                }
            }
        }
    }

    // Fallback: check ?token= query parameter (for browser WebSocket)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=") {
                if token == expected {
                    return next.run(request).await;
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

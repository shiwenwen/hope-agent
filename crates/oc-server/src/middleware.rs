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
    let expected = match &state.api_key {
        Some(key) => key,
        None => return next.run(request).await,
    };
    let expected_bytes = expected.as_bytes();

    // Check Authorization header first
    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(value) = auth_header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                if constant_time_eq(token.as_bytes(), expected_bytes) {
                    return next.run(request).await;
                }
            }
        }
    }

    // Fallback: check ?token= query parameter (for browser WebSocket)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=") {
                let decoded = percent_decode_form_value(token);
                if constant_time_eq(&decoded, expected_bytes) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

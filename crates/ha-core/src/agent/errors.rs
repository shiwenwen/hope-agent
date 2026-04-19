use super::api_types::ApiErrorResponse;

/// Check if an HTTP error is retryable (rate limit or server error)
pub(super) fn is_retryable_error(status: u16, error_text: &str) -> bool {
    if matches!(status, 429 | 500 | 502 | 503 | 504) {
        return true;
    }
    let lower = error_text.to_lowercase();
    lower.contains("rate") && lower.contains("limit")
        || lower.contains("overloaded")
        || lower.contains("service unavailable")
        || lower.contains("upstream connect")
        || lower.contains("connection refused")
}

/// Parse error response and return a user-friendly message
pub(super) fn parse_error_response(status: u16, raw: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<ApiErrorResponse>(raw) {
        if let Some(detail) = &parsed.detail {
            if let Some(s) = detail.as_str() {
                return format!("Codex API 错误 ({}): {}", status, s);
            }
        }

        if let Some(err) = parsed.error {
            let code = err
                .code
                .as_deref()
                .or(err.error_type.as_deref())
                .unwrap_or("");

            if code.contains("usage_limit_reached")
                || code.contains("usage_not_included")
                || code.contains("rate_limit_exceeded")
                || status == 429
            {
                let plan = err
                    .plan_type
                    .as_ref()
                    .map(|p| format!(" ({} plan)", p.to_lowercase()))
                    .unwrap_or_default();

                let when = if let Some(resets_at) = err.resets_at {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as f64;
                    let mins = ((resets_at - now_secs) / 60.0).max(0.0).round() as u64;
                    format!(" 大约 {} 分钟后可重试。", mins)
                } else {
                    String::new()
                };

                return format!("您已达到 ChatGPT 使用限额{}。{}", plan, when);
            }

            if let Some(msg) = err.message {
                return format!("Codex API 错误 ({}): {}", status, msg);
            }
        }
    }

    format!("Codex API 错误 ({}): {}", status, raw)
}

/// Get OS version string.
///
/// Routed through [`crate::platform::os_version_string`] so error reports
/// carry the real Windows / Linux version instead of `"unknown"`.
pub(super) fn os_version() -> String {
    crate::platform::os_version_string()
}

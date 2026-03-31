// ── Model Failover: Error Classification ──────────────────────────
//
//  Classifies API errors to determine whether to retry the same model,
//  fall back to the next model, or surface the error directly.
//  Inspired by OpenClaw's failover-error.ts.

use serde::Serialize;

/// Why a model request failed — drives retry / fallback decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailoverReason {
    /// 429 Too Many Requests — retryable on same model
    RateLimit,
    /// 503 Service Unavailable / overloaded — retryable on same model
    Overloaded,
    /// Request timeout or connection error — retryable on same model
    Timeout,
    /// 401 Unauthorized / invalid API key — skip to next model
    Auth,
    /// 402 Payment Required / quota exhausted — skip to next model
    Billing,
    /// 404 Model not found — skip to next model
    ModelNotFound,
    /// Context window exceeded — NOT fallback-able (smaller model would be worse)
    ContextOverflow,
    /// Unrecognized error — skip to next model
    Unknown,
}

impl FailoverReason {
    /// Whether this error class should be retried on the **same** model
    /// (with backoff) before moving to the next model in the chain.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimit | Self::Overloaded | Self::Timeout)
    }

    /// Whether this error should immediately surface to the user
    /// without trying any fallback models.
    /// Note: ContextOverflow is no longer terminal — it triggers compaction first.
    pub fn is_terminal(&self) -> bool {
        false
    }

    /// Whether this error should trigger context compaction before retry.
    pub fn needs_compaction(&self) -> bool {
        matches!(self, Self::ContextOverflow)
    }
}

// ── Error Classification ──────────────────────────────────────────

/// Regex-style patterns for error classification.
/// We use simple substring matching for performance.

/// Classify an API error message into a `FailoverReason`.
///
/// Checks HTTP-style status codes and well-known error patterns from
/// Anthropic, OpenAI, Google, and other LLM APIs.
pub fn classify_error(error_msg: &str) -> FailoverReason {
    let lower = error_msg.to_lowercase();

    // ── Context overflow (terminal — never fallback) ──────────────
    if is_context_overflow(&lower) {
        return FailoverReason::ContextOverflow;
    }

    // ── Rate limit (retryable) ────────────────────────────────────
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("resource_exhausted")
        || lower.contains("throttl")
    {
        return FailoverReason::RateLimit;
    }

    // ── Overloaded (retryable) ────────────────────────────────────
    if lower.contains("503")
        || lower.contains("overloaded")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("502")  // Bad Gateway
        || lower.contains("521")  // Cloudflare origin down
        || lower.contains("522")  // Cloudflare connection timed out
        || lower.contains("524")
    // Cloudflare timeout
    {
        return FailoverReason::Overloaded;
    }

    // ── Timeout (retryable) ───────────────────────────────────────
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("etimedout")
        || lower.contains("econnreset")
        || lower.contains("econnrefused")
        || lower.contains("econnaborted")
        || lower.contains("enetunreach")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("broken pipe")
    {
        return FailoverReason::Timeout;
    }

    // ── Auth (skip to next model) ─────────────────────────────────
    if lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("authentication")
        || lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("permission denied")
    {
        return FailoverReason::Auth;
    }

    // ── Billing (skip to next model) ──────────────────────────────
    if lower.contains("402")
        || lower.contains("payment required")
        || lower.contains("billing")
        || lower.contains("quota")
        || lower.contains("insufficient_quota")
        || lower.contains("exceeded your current quota")
    {
        return FailoverReason::Billing;
    }

    // ── Model not found (skip to next model) ──────────────────────
    if lower.contains("404")
        || lower.contains("model not found")
        || lower.contains("model_not_found")
        || lower.contains("does not exist")
        || lower.contains("not_found_error")
    {
        return FailoverReason::ModelNotFound;
    }

    FailoverReason::Unknown
}

/// Check if an error message indicates context window overflow.
/// These errors should NEVER trigger model fallback — a smaller context
/// window model would produce an even worse result.
fn is_context_overflow(lower: &str) -> bool {
    lower.contains("context length exceeded")
        || lower.contains("context_length_exceeded")
        || lower.contains("context window")
        || lower.contains("maximum context length")
        || lower.contains("prompt is too long")
        || lower.contains("token limit")
        || lower.contains("max_tokens") && (lower.contains("exceed") || lower.contains("too large"))
        || lower.contains("input too long")
        || lower.contains("request too large")
}

// ── Retry with Backoff ────────────────────────────────────────────

/// Compute delay for retry attempt `attempt` (0-indexed).
/// Uses exponential backoff: base_ms * 2^attempt, clamped to max_ms,
/// plus random jitter up to ±10%.
pub fn retry_delay_ms(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let delay = base_ms.saturating_mul(2u64.saturating_pow(attempt));
    let clamped = delay.min(max_ms);
    // Simple jitter: ±10%
    let jitter_range = clamped / 10;
    if jitter_range == 0 {
        return clamped;
    }
    let jitter = (rand_simple() % (jitter_range * 2 + 1)) as i64 - jitter_range as i64;
    (clamped as i64 + jitter).max(0) as u64
}

/// Simple pseudo-random number (no external crate needed).
fn rand_simple() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit() {
        assert_eq!(
            classify_error("429 Too Many Requests"),
            FailoverReason::RateLimit
        );
        assert_eq!(
            classify_error("Rate limit exceeded"),
            FailoverReason::RateLimit
        );
        assert_eq!(
            classify_error("RESOURCE_EXHAUSTED"),
            FailoverReason::RateLimit
        );
    }

    #[test]
    fn test_overloaded() {
        assert_eq!(
            classify_error("503 Service Unavailable"),
            FailoverReason::Overloaded
        );
        assert_eq!(
            classify_error("The server is overloaded"),
            FailoverReason::Overloaded
        );
        assert_eq!(
            classify_error("502 Bad Gateway"),
            FailoverReason::Overloaded
        );
    }

    #[test]
    fn test_timeout() {
        assert_eq!(classify_error("request timed out"), FailoverReason::Timeout);
        assert_eq!(classify_error("ETIMEDOUT"), FailoverReason::Timeout);
        assert_eq!(
            classify_error("connection reset by peer"),
            FailoverReason::Timeout
        );
    }

    #[test]
    fn test_auth() {
        assert_eq!(classify_error("401 Unauthorized"), FailoverReason::Auth);
        assert_eq!(classify_error("Invalid API key"), FailoverReason::Auth);
        assert_eq!(classify_error("403 Forbidden"), FailoverReason::Auth);
    }

    #[test]
    fn test_billing() {
        assert_eq!(
            classify_error("402 Payment Required"),
            FailoverReason::Billing
        );
        assert_eq!(
            classify_error("You exceeded your current quota"),
            FailoverReason::Billing
        );
    }

    #[test]
    fn test_model_not_found() {
        assert_eq!(
            classify_error("404 Not Found"),
            FailoverReason::ModelNotFound
        );
        assert_eq!(
            classify_error("model_not_found"),
            FailoverReason::ModelNotFound
        );
        assert_eq!(
            classify_error("The model does not exist"),
            FailoverReason::ModelNotFound
        );
    }

    #[test]
    fn test_context_overflow() {
        assert_eq!(
            classify_error("This model's maximum context length is 200000 tokens"),
            FailoverReason::ContextOverflow
        );
        assert_eq!(
            classify_error("context_length_exceeded"),
            FailoverReason::ContextOverflow
        );
    }

    #[test]
    fn test_unknown() {
        assert_eq!(classify_error("some random error"), FailoverReason::Unknown);
    }

    #[test]
    fn test_retryable() {
        assert!(FailoverReason::RateLimit.is_retryable());
        assert!(FailoverReason::Overloaded.is_retryable());
        assert!(FailoverReason::Timeout.is_retryable());
        assert!(!FailoverReason::Auth.is_retryable());
        assert!(!FailoverReason::ContextOverflow.is_retryable());
    }

    #[test]
    fn test_terminal() {
        assert!(FailoverReason::ContextOverflow.is_terminal());
        assert!(!FailoverReason::RateLimit.is_terminal());
        assert!(!FailoverReason::Unknown.is_terminal());
    }

    #[test]
    fn test_retry_delay() {
        let d0 = retry_delay_ms(0, 1000, 10000);
        assert!(d0 >= 900 && d0 <= 1100); // ~1000 ±10%

        let d1 = retry_delay_ms(1, 1000, 10000);
        assert!(d1 >= 1800 && d1 <= 2200); // ~2000 ±10%

        let d_max = retry_delay_ms(10, 1000, 10000);
        assert!(d_max >= 9000 && d_max <= 11000); // clamped to ~10000
    }
}

// ── Model Failover: Error Classification & Auth Profile Rotation ───
//
//  Classifies API errors to determine whether to retry the same model,
//  fall back to the next model, or surface the error directly.
//  Also provides per-profile cooldown tracking and session-sticky
//  profile selection for multi-key rotation within a single provider.
//
//  ## Submodules
//
//  - [`executor`] (Phase 3): generic `execute_with_failover` wrapper that
//    lifts the inline rotation + retry + cooldown orchestration out of
//    `chat_engine` so one-shot paths (side_query / summarize_direct) can
//    opt in too.

pub mod executor;

use serde::Serialize;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use crate::provider::{AuthProfile, ProviderConfig};

/// Structured provider failure retained across `anyhow` wrapping. Provider
/// adapters must construct this from the decoded HTTP/SSE error envelope so
/// destructive recovery never has to trust arbitrary display text.
#[derive(Debug, Clone)]
pub(crate) struct ProviderApiError {
    provider: String,
    status: Option<u16>,
    code: Option<String>,
    error_type: Option<String>,
    message: Option<String>,
    retry_after_ms: Option<u64>,
    transport: ProviderErrorTransport,
    display: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderErrorTransport {
    HttpResponse,
    StreamEvent,
}

impl ProviderApiError {
    /// Whether the Provider supplied an explicit terminal rejection rather
    /// than the client merely observing an interrupted response body.
    ///
    /// Both a complete non-2xx HTTP response and a decoded terminal SSE error
    /// (`error` / `response.failed`) prove that the claimed request reached a
    /// known Provider outcome. Callers may therefore close that exact request
    /// before preparing a fallback; transport/decode failures without one of
    /// these typed envelopes remain dispatch-unknown.
    pub(crate) fn is_explicit_rejection(&self) -> bool {
        matches!(
            self.transport,
            ProviderErrorTransport::HttpResponse | ProviderErrorTransport::StreamEvent
        )
    }

    pub(crate) fn from_http_response(provider: &str, status: u16, raw_body: &str) -> Self {
        let decoded = serde_json::from_str::<serde_json::Value>(raw_body).ok();
        let code = decoded.as_ref().and_then(|value| {
            json_string_at(value, &["/error/code", "/code", "/error/status", "/status"])
        });
        let error_type = decoded.as_ref().and_then(|value| {
            json_string_at(value, &["/error/type", "/type", "/error/error_type"])
        });
        let message = decoded
            .as_ref()
            .and_then(|value| json_string_at(value, &["/error/message", "/message", "/detail"]));
        let public_detail = message
            .as_deref()
            .or(code.as_deref())
            .or(error_type.as_deref())
            .unwrap_or("request failed");
        // Keep the decoded envelope for typed classification, but never echo
        // an arbitrary response body into user-facing errors/logs: a proxy may
        // include request headers or credentials in that body.
        let display = format!(
            "{provider} API error ({status}): {}",
            crate::truncate_utf8(public_detail, 1_024)
        );
        Self {
            provider: provider.to_string(),
            status: Some(status),
            code,
            error_type,
            message,
            retry_after_ms: None,
            transport: ProviderErrorTransport::HttpResponse,
            display,
        }
    }

    pub(crate) fn from_stream_event(
        provider: &str,
        code: Option<&str>,
        error_type: Option<&str>,
        message: Option<&str>,
        display: String,
    ) -> Self {
        Self {
            provider: provider.to_string(),
            status: None,
            code: code.map(ToOwned::to_owned),
            error_type: error_type.map(ToOwned::to_owned),
            message: message.map(ToOwned::to_owned),
            retry_after_ms: None,
            transport: ProviderErrorTransport::StreamEvent,
            display,
        }
    }

    /// Preserve an existing user-facing provider error without discarding the
    /// structured fields used by recovery classification.
    pub(crate) fn with_display(mut self, display: String) -> Self {
        self.display = display;
        self
    }

    /// Preserve a bounded `Retry-After` delta-seconds hint without retaining
    /// arbitrary response headers. HTTP-date values are deliberately ignored:
    /// wall-clock rollback must not turn a retry hint into an unbounded wait.
    pub(crate) fn with_retry_after_header(mut self, value: Option<&str>) -> Self {
        self.retry_after_ms = value
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .map(|seconds| seconds.saturating_mul(1_000));
        self
    }

    pub(crate) fn retry_after_ms(&self) -> Option<u64> {
        self.retry_after_ms
    }
}

impl std::fmt::Display for ProviderApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display)
    }
}

impl std::error::Error for ProviderApiError {}

fn json_string_at(value: &serde_json::Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

/// Evidence that an error may represent an input-context overflow.
///
/// [`Self::LocalPreflight`] and [`Self::StructuredProvider`] are both strong
/// enough to classify the failure as overflow. Destructive Tier-4 recovery is
/// narrower: it additionally requires a [`Self::LocalPreflight`] carrying an
/// immutable complete-request capacity proof. [`Self::TextHint`] exists only
/// for diagnostics and telemetry; arbitrary provider/proxy text can never
/// trigger destructive compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextOverflowEvidence {
    LocalPreflight {
        input_tokens: u64,
        max_input_tokens: u64,
        source: crate::token_accounting::TokenCountSource,
        capacity_proof: Option<crate::token_accounting::PreflightCapacityProof>,
    },
    StructuredProvider {
        provider: String,
        status: Option<u16>,
        code: Option<String>,
        error_type: Option<String>,
        message_pattern: Option<String>,
    },
    TextHint {
        pattern: String,
    },
}

impl ContextOverflowEvidence {
    pub fn is_high_confidence(&self) -> bool {
        matches!(
            self,
            Self::LocalPreflight { .. } | Self::StructuredProvider { .. }
        )
    }
}

/// Why a model request failed — drives retry / fallback decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailoverReason {
    /// A protected evaluation run reached an immutable trial ceiling. This is
    /// an application-level terminal outcome, never a Provider retry/failover.
    EvaluationBudget,
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
    /// Even the current tool-result group's cheapest protocol-legal envelope
    /// cannot fit after the configured old-history recovery ladder. This is an
    /// application-level terminal outcome, not a Provider failure.
    CurrentToolGroupOverflow,
    /// The exact request crossed the durable dispatch claim but no response
    /// proof was obtained. Retrying may duplicate billing or Provider-side
    /// work, so automatic retry/profile/model rotation is forbidden.
    DispatchUnknown,
    /// Unrecognized error — retry with a small budget because opaque provider / proxy errors
    /// are often transient, then skip to the next model.
    Unknown,
}

impl FailoverReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EvaluationBudget => "evaluation_budget",
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::Timeout => "timeout",
            Self::Auth => "auth",
            Self::Billing => "billing",
            Self::ModelNotFound => "model_not_found",
            Self::ContextOverflow => "context_overflow",
            Self::CurrentToolGroupOverflow => "current_tool_group_overflow",
            Self::DispatchUnknown => "dispatch_unknown",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this error class should be retried on the **same** model
    /// (with backoff) before moving to the next model in the chain.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Overloaded | Self::Timeout | Self::Unknown
        )
    }

    /// Whether this error should immediately surface to the user
    /// without trying any fallback models.
    /// Note: ContextOverflow is no longer terminal — it triggers compaction first.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::EvaluationBudget | Self::CurrentToolGroupOverflow | Self::DispatchUnknown
        )
    }

    /// Whether this error should trigger context compaction before retry.
    pub fn needs_compaction(&self) -> bool {
        matches!(self, Self::ContextOverflow)
    }

    /// Whether this error class should trigger rotation to the next auth profile
    /// within the same provider before falling through to model-level failover.
    pub fn is_profile_rotatable(&self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Overloaded | Self::Auth | Self::Billing
        )
    }

    /// Get the cooldown duration (in seconds) for this error type when applied
    /// to a per-profile cooldown.
    pub fn profile_cooldown_secs(&self) -> u64 {
        match self {
            Self::Overloaded => 30,
            Self::RateLimit => 60,
            Self::Auth => 300,
            Self::Billing => 600,
            _ => 0,
        }
    }
}

// ── Error Classification ──────────────────────────────────────────

// Known semantic phrases use substring matching. Ambiguous generic 500/504
// codes require explicit HTTP/API-status context so token counts and request
// IDs cannot accidentally trigger retries or profile cooldowns.

static HTTP_STATUS_CODE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?ix)
        (?:
            \bhttp(?:/\d(?:\.\d)?)?\s* |
            \bstatus(?:\s+code)?\s*[:=]?\s* |
            \bresponse\s+code\s*[:=]?\s* |
            \bapi\s+(?:error|错误)\s*[\(:=]?\s* |
            ["']status["']\s*:\s*
        )
        (?P<code>[1-5][0-9]{2})\b
        "#,
    )
    .expect("valid HTTP status code regex")
});

fn contains_http_status(error_msg: &str, expected: &str) -> bool {
    HTTP_STATUS_CODE_RE
        .captures_iter(error_msg)
        .any(|captures| {
            captures
                .name("code")
                .is_some_and(|code| code.as_str() == expected)
        })
}

fn exact_overflow_code(code: &str) -> bool {
    matches!(
        code.trim().to_ascii_lowercase().as_str(),
        "context_length_exceeded"
            | "context_window_exceeded"
            | "prompt_too_long"
            | "input_too_long"
    )
}

fn invalid_request_type(error_type: &str) -> bool {
    matches!(
        error_type.trim().to_ascii_lowercase().as_str(),
        "invalid_request_error" | "invalid_request" | "bad_request_error" | "bad_request"
    )
}

fn high_confidence_overflow_message(lower: &str) -> Option<&'static str> {
    let direct = [
        "context length exceeded",
        "context_length_exceeded",
        "context window exceeded",
        "exceeds the context window",
        "exceeded the context window",
        "prompt is too long",
        "input is too long for this model",
    ]
    .into_iter()
    .find(|pattern| lower.contains(pattern));
    direct.or_else(|| {
        (lower.contains("maximum context length")
            && [
                "messages resulted in",
                "message resulted in",
                "input resulted in",
                "tokens in the messages",
            ]
            .into_iter()
            .any(|pattern| lower.contains(pattern)))
        .then_some("maximum context length with input token count")
    })
}

fn low_confidence_overflow_hint(lower: &str) -> Option<&'static str> {
    high_confidence_overflow_message(lower).or_else(|| {
        [
            "context window",
            "maximum context length",
            "token limit",
            "input too long",
            "request too large",
            "max_tokens",
        ]
        .into_iter()
        .find(|pattern| lower.contains(pattern))
    })
}

fn structured_provider_overflow_evidence(
    error: &ProviderApiError,
) -> Option<ContextOverflowEvidence> {
    let exact_code = error
        .code
        .as_deref()
        .filter(|code| exact_overflow_code(code));
    let exact_type = error
        .error_type
        .as_deref()
        .filter(|error_type| exact_overflow_code(error_type));
    let message_pattern = error
        .message
        .as_deref()
        .and_then(|message| high_confidence_overflow_message(&message.to_ascii_lowercase()));
    let has_invalid_request_type = error
        .error_type
        .as_deref()
        .is_some_and(invalid_request_type);
    let structured_message = match error.transport {
        ProviderErrorTransport::HttpResponse => {
            matches!(error.status, Some(400 | 413 | 422))
                && has_invalid_request_type
                && message_pattern.is_some()
        }
        ProviderErrorTransport::StreamEvent => {
            has_invalid_request_type && message_pattern.is_some()
        }
    };
    if exact_code.is_none() && exact_type.is_none() && !structured_message {
        return None;
    }

    Some(ContextOverflowEvidence::StructuredProvider {
        provider: error.provider.clone(),
        status: error.status,
        code: error.code.clone(),
        error_type: error.error_type.clone(),
        message_pattern: message_pattern.map(ToOwned::to_owned),
    })
}

/// Recover high-confidence typed evidence when available, otherwise retain a
/// low-confidence text hint for diagnostics only.
pub(crate) fn context_overflow_evidence(error: &anyhow::Error) -> Option<ContextOverflowEvidence> {
    if let Some(preflight) = error.downcast_ref::<crate::token_accounting::PreflightOverflow>() {
        return Some(ContextOverflowEvidence::LocalPreflight {
            input_tokens: preflight.input_tokens,
            max_input_tokens: preflight.max_input_tokens,
            source: preflight.source,
            capacity_proof: preflight.capacity_proof.clone(),
        });
    }
    if let Some(provider) = error.downcast_ref::<ProviderApiError>() {
        if let Some(evidence) = structured_provider_overflow_evidence(provider) {
            return Some(evidence);
        }
        if let Some(pattern) = provider
            .message
            .as_deref()
            .and_then(|message| low_confidence_overflow_hint(&message.to_ascii_lowercase()))
        {
            return Some(ContextOverflowEvidence::TextHint {
                pattern: pattern.to_string(),
            });
        }
    }

    low_confidence_overflow_hint(&error.to_string().to_ascii_lowercase()).map(|pattern| {
        ContextOverflowEvidence::TextHint {
            pattern: pattern.to_string(),
        }
    })
}

/// Classification for an executable model attempt. Tier 4 is authorized only
/// by high-confidence typed evidence; free-form text falls through to normal
/// retry/failover classification.
pub fn classify_error_with_evidence(
    error: &anyhow::Error,
) -> (FailoverReason, Option<ContextOverflowEvidence>) {
    if error
        .downcast_ref::<crate::agent::ProviderDispatchUnknown>()
        .is_some()
    {
        return (FailoverReason::DispatchUnknown, None);
    }
    if error
        .downcast_ref::<crate::context_compact::group_admission::CurrentToolGroupEnvelopeOverflowError>()
        .is_some()
    {
        return (FailoverReason::CurrentToolGroupOverflow, None);
    }
    let evidence = context_overflow_evidence(error);
    let reason = if evidence
        .as_ref()
        .is_some_and(ContextOverflowEvidence::is_high_confidence)
    {
        FailoverReason::ContextOverflow
    } else {
        classify_error(&error.to_string())
    };
    (reason, evidence)
}

/// Classify an API error message into a `FailoverReason`.
///
/// Checks HTTP-style status codes and well-known error patterns from
/// Anthropic, OpenAI, Google, and other LLM APIs.
pub fn classify_error(error_msg: &str) -> FailoverReason {
    let lower = error_msg.to_lowercase();

    if lower.contains("evaluation budget exhausted") {
        return FailoverReason::EvaluationBudget;
    }

    // ── Rate limit (retryable) ────────────────────────────────────
    if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("resource_exhausted")
        || lower.contains("throttl")
        || lower.contains("requestbursttoofast")
    {
        return FailoverReason::RateLimit;
    }

    // ── Overloaded (retryable) ────────────────────────────────────
    if contains_http_status(&lower, "500") // Internal Server Error
        || lower.contains("503")
        || lower.contains("overloaded")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
        || lower.contains("server_error")
        || lower.contains("internal server error")
        || lower.contains("an error occurred while processing your request")
        || lower.contains("502")  // Bad Gateway
        || contains_http_status(&lower, "504") // Gateway Timeout
        || lower.contains("521")  // Cloudflare origin down
        || lower.contains("522")  // Cloudflare connection timed out
        || lower.contains("524")
    // Cloudflare timeout
    {
        return FailoverReason::Overloaded;
    }

    // ── Timeout / transport error (retryable) ─────────────────────
    // Includes reqwest/hyper decode failures which typically occur when
    // the server closes a chunked/SSE response body mid-stream after
    // returning 200 headers (seen on dashscope-coding under load).
    if lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("etimedout")
        || lower.contains("econnreset")
        || lower.contains("econnrefused")
        || lower.contains("econnaborted")
        || lower.contains("enetunreach")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection error")
        || lower.contains("network error")
        || lower.contains("network unreachable")
        || lower.contains("error sending request")
        || lower.contains("error trying to connect")
        || lower.contains("dns error")
        || lower.contains("failed to lookup address information")
        || lower.contains("tcp connect error")
        || lower.contains("broken pipe")
        || lower.contains("error decoding response body")
        || lower.contains("error reading a body from connection")
        || lower.contains("incomplete message")
        || lower.contains("unexpected eof")
        || lower.contains("connection closed before message completed")
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
        || lower.contains("provider not found")
        || lower.contains("does not exist")
        || lower.contains("not_found_error")
    {
        return FailoverReason::ModelNotFound;
    }

    FailoverReason::Unknown
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

/// Combine local exponential backoff with a Provider `Retry-After` hint while
/// preserving the caller's single bounded retry budget.
pub fn effective_retry_delay_ms(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    retry_after_ms: Option<u64>,
) -> u64 {
    retry_delay_ms(attempt, base_ms, max_ms).max(retry_after_ms.unwrap_or(0).min(max_ms))
}

/// Simple pseudo-random number (no external crate needed).
/// Uses both nanos and a thread-local counter to avoid bias from
/// rapid successive calls that may share the same nanosecond value.
fn rand_simple() -> u64 {
    use std::cell::Cell;
    use std::time::SystemTime;
    thread_local! {
        static COUNTER: Cell<u64> = const { Cell::new(0) };
    }
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let count = COUNTER.with(|c| {
        let v = c.get().wrapping_add(1);
        c.set(v);
        v
    });
    // Mix nanos with counter using a simple hash-like operation
    nanos ^ (count.wrapping_mul(6364136223846793005))
}

// ── Auth Profile Cooldown Tracking ────────────────────────────────
//
//  Runtime-only state (not persisted). Tracks per-profile cooldowns after
//  rate-limit / auth / billing errors to avoid retrying a known-bad key.

struct CooldownEntry {
    until: Instant,
}

/// Global per-profile cooldown tracker.
pub struct ProfileCooldownTracker {
    entries: Mutex<HashMap<String, CooldownEntry>>,
}

impl ProfileCooldownTracker {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Mark a profile as cooled down for the given duration.
    pub fn mark_cooldown(&self, profile_id: &str, reason: &FailoverReason) {
        let secs = reason.profile_cooldown_secs();
        if secs == 0 {
            return;
        }
        if let Ok(mut map) = self.entries.lock() {
            // Prune expired entries opportunistically (cap at 100 to avoid unbounded growth)
            if map.len() > 100 {
                let now = Instant::now();
                map.retain(|_, e| now < e.until);
            }
            map.insert(
                profile_id.to_string(),
                CooldownEntry {
                    until: Instant::now() + std::time::Duration::from_secs(secs),
                },
            );
        }
    }

    /// Check if a profile is available (not in cooldown).
    pub fn is_available(&self, profile_id: &str) -> bool {
        if let Ok(map) = self.entries.lock() {
            match map.get(profile_id) {
                Some(entry) => Instant::now() >= entry.until,
                None => true,
            }
        } else {
            true
        }
    }

    /// Filter a list of profiles to only those not currently in cooldown.
    /// Acquires the lock once for all profiles.
    pub fn filter_available(&self, profiles: &[AuthProfile]) -> Vec<AuthProfile> {
        let now = Instant::now();
        if let Ok(map) = self.entries.lock() {
            profiles
                .iter()
                .filter(|p| map.get(&p.id).map_or(true, |e| now >= e.until))
                .cloned()
                .collect()
        } else {
            profiles.to_vec()
        }
    }

    /// Clear the cooldown for a profile (e.g. on success).
    pub fn clear(&self, profile_id: &str) {
        if let Ok(mut map) = self.entries.lock() {
            map.remove(profile_id);
        }
    }
}

pub static PROFILE_COOLDOWNS: LazyLock<ProfileCooldownTracker> =
    LazyLock::new(ProfileCooldownTracker::new);

// ── Session Profile Stickiness ───────────────────────────────────
//
//  Maps (provider_id, session_id) → last-successful profile_id.
//  Ensures cache-friendly behavior by preferring the same key across turns.

/// Per-provider LRU of (session_id → profile_id). We need insertion-order
/// tracking for O(1) "evict oldest" semantics without pulling a full LRU
/// crate, so sessions live in a side `VecDeque` alongside the map: `get`
/// looks up in the map, `set` promotes the key to the back, eviction drops
/// the front. Keeps the whole map bounded without the old "blow away
/// everything at 500" behavior that destroyed session stickiness for
/// every long-running process.
#[derive(Default)]
struct StickyShard {
    map: HashMap<String, String>,
    order: std::collections::VecDeque<String>,
}

impl StickyShard {
    fn promote(&mut self, session_id: &str) {
        if let Some(pos) = self.order.iter().position(|s| s == session_id) {
            self.order.remove(pos);
        }
        self.order.push_back(session_id.to_string());
    }

    fn insert(&mut self, session_id: &str, profile_id: &str, max: usize) {
        self.map
            .insert(session_id.to_string(), profile_id.to_string());
        self.promote(session_id);
        while self.order.len() > max {
            if let Some(evicted) = self.order.pop_front() {
                self.map.remove(&evicted);
            }
        }
    }
}

pub struct ProfileStickyMap {
    map: Mutex<HashMap<String, StickyShard>>,
}

/// Cap per-provider session entries to prevent unbounded growth.
const STICKY_MAX_SESSIONS_PER_PROVIDER: usize = 500;

impl ProfileStickyMap {
    fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Get the sticky profile ID for a provider+session pair.
    pub fn get(&self, provider_id: &str, session_id: &str) -> Option<String> {
        let mut guard = self.map.lock().ok()?;
        let shard = guard.get_mut(provider_id)?;
        let profile = shard.map.get(session_id).cloned();
        if profile.is_some() {
            shard.promote(session_id);
        }
        profile
    }

    /// Set the sticky profile ID after a successful request.
    /// Uses LRU semantics so hitting the cap evicts the single oldest
    /// session entry instead of wiping every existing stickiness.
    pub fn set(&self, provider_id: &str, session_id: &str, profile_id: &str) {
        if let Ok(mut map) = self.map.lock() {
            let shard = map.entry(provider_id.to_string()).or_default();
            shard.insert(session_id, profile_id, STICKY_MAX_SESSIONS_PER_PROVIDER);
        }
    }
}

pub static PROFILE_STICKY: LazyLock<ProfileStickyMap> = LazyLock::new(ProfileStickyMap::new);

// ── Profile Selection ────────────────────────────────────────────

/// Select the best auth profile for a provider+session combination.
///
/// Priority:
/// 1. Sticky profile from the same session (if still available)
/// 2. First available (non-cooled-down, enabled) profile
/// 3. None (all profiles exhausted)
pub fn select_profile(provider: &ProviderConfig, session_id: &str) -> Option<AuthProfile> {
    let profiles = provider.effective_profiles();
    if profiles.is_empty() {
        return None;
    }

    // Try sticky profile first
    if let Some(sticky_id) = PROFILE_STICKY.get(&provider.id, session_id) {
        if let Some(p) = profiles.iter().find(|p| p.id == sticky_id) {
            if PROFILE_COOLDOWNS.is_available(&p.id) {
                return Some(p.clone());
            }
        }
    }

    // Fall back to first available
    PROFILE_COOLDOWNS
        .filter_available(&profiles)
        .into_iter()
        .next()
}

/// Get the next profile to try after a failure, excluding already-tried profiles.
pub fn next_profile(provider: &ProviderConfig, tried: &[String]) -> Option<AuthProfile> {
    let profiles = provider.effective_profiles();
    PROFILE_COOLDOWNS
        .filter_available(&profiles)
        .into_iter()
        .find(|p| !tried.contains(&p.id))
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
        assert_eq!(
            classify_error("RequestBurstTooFast"),
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
        assert_eq!(
            classify_error("Codex API 错误 (500):"),
            FailoverReason::Overloaded
        );
        assert_eq!(
            classify_error("Codex API 错误 (504):"),
            FailoverReason::Overloaded
        );
        assert_eq!(
            classify_error("request failed with status: 500"),
            FailoverReason::Overloaded
        );
        assert_eq!(classify_error("server_error"), FailoverReason::Overloaded);
        assert_eq!(
            classify_error("An error occurred while processing your request. Please include the request ID 8d46da73-d9c2-44d5-af24-707fb7680aad in your message."),
            FailoverReason::Overloaded
        );
    }

    #[test]
    fn status_codes_require_http_context() {
        assert_eq!(
            classify_error("maximum output tokens is 500"),
            FailoverReason::Unknown
        );
        assert_eq!(
            classify_error("request ID abc-500-def"),
            FailoverReason::Unknown
        );
        assert_eq!(
            classify_error("using model v504-preview"),
            FailoverReason::Unknown
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
        assert_eq!(
            classify_error("error decoding response body"),
            FailoverReason::Timeout
        );
        assert_eq!(
            classify_error("error reading a body from connection"),
            FailoverReason::Timeout
        );
        assert_eq!(
            classify_error("connection closed before message completed"),
            FailoverReason::Timeout
        );
        assert_eq!(
            classify_error(
                "Codex API request failed: error sending request for url \
                 (https://chatgpt.com/backend-api/codex/responses)"
            ),
            FailoverReason::Timeout
        );
        assert_eq!(
            classify_error("OpenAI Chat API request failed: error trying to connect: dns error"),
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
        assert_eq!(
            classify_error("Provider not found: removed-provider for model example-model"),
            FailoverReason::ModelNotFound
        );
    }

    #[test]
    fn free_text_context_hints_never_authorize_compaction() {
        for message in [
            "This model's maximum context length is 200000 tokens",
            "context_length_exceeded",
            "prompt is too long",
        ] {
            let error = anyhow::anyhow!(message);
            let (reason, evidence) = classify_error_with_evidence(&error);
            assert_eq!(reason, FailoverReason::Unknown);
            assert!(matches!(
                evidence,
                Some(ContextOverflowEvidence::TextHint { .. })
            ));
        }
    }

    #[test]
    fn known_provider_overflow_shapes_are_high_confidence() {
        let fixtures = [
            ProviderApiError::from_http_response(
                "OpenAI Responses",
                400,
                r#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 129000 tokens."}}"#,
            ),
            ProviderApiError::from_http_response(
                "Anthropic",
                400,
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 201000 tokens > 200000 maximum"}}"#,
            ),
            ProviderApiError::from_http_response(
                "Codex",
                400,
                r#"{"error":{"type":"invalid_request_error","code":"context_window_exceeded","message":"Your input exceeds the context window of this model."}}"#,
            )
            .with_display("Codex API 错误 (400): 请求无法完成".to_string()),
            ProviderApiError::from_stream_event(
                "OpenAI Responses/Codex",
                Some("context_length_exceeded"),
                Some("invalid_request_error"),
                Some("input rejected"),
                "input rejected".to_string(),
            ),
        ];

        for provider_error in fixtures {
            let error = anyhow::Error::new(provider_error);
            let (reason, evidence) = classify_error_with_evidence(&error);
            assert_eq!(reason, FailoverReason::ContextOverflow);
            assert!(evidence.is_some_and(|value| value.is_high_confidence()));
        }
    }

    #[test]
    fn local_preflight_overflow_is_high_confidence() {
        let error = anyhow::Error::new(crate::token_accounting::PreflightOverflow {
            input_tokens: 129_000,
            max_input_tokens: 128_000,
            source: crate::token_accounting::TokenCountSource::LocalTokenizer,
            capacity_proof: None,
        });
        let (reason, evidence) = classify_error_with_evidence(&error);
        assert_eq!(reason, FailoverReason::ContextOverflow);
        assert!(matches!(
            evidence,
            Some(ContextOverflowEvidence::LocalPreflight {
                input_tokens: 129_000,
                max_input_tokens: 128_000,
                ..
            })
        ));
    }

    #[test]
    fn current_tool_group_c0_overflow_is_typed_terminal_not_provider_overflow() {
        let error = anyhow::Error::new(
            crate::context_compact::group_admission::CurrentToolGroupEnvelopeOverflowError {
                capacity: crate::context_compact::group_admission::RequestCapacityCount::new(
                    129_000, 8_000,
                ),
                context_window: 128_000,
                safety_headroom: 2_000,
            },
        );

        let (reason, evidence) = classify_error_with_evidence(&error);

        assert_eq!(reason, FailoverReason::CurrentToolGroupOverflow);
        assert!(reason.is_terminal());
        assert!(!reason.is_retryable());
        assert!(!reason.needs_compaction());
        assert!(evidence.is_none());
    }

    #[test]
    fn ambiguous_token_and_request_limits_do_not_authorize_compaction() {
        let cases = [
            (
                "429 rate limit reached for this token limit tier",
                FailoverReason::RateLimit,
            ),
            (
                "reverse proxy rejected request too large",
                FailoverReason::Unknown,
            ),
            (
                "max_tokens exceeds the output token limit",
                FailoverReason::Unknown,
            ),
        ];
        for (message, expected_reason) in cases {
            let error = anyhow::anyhow!(message);
            let (reason, evidence) = classify_error_with_evidence(&error);
            assert_eq!(reason, expected_reason, "{message}");
            assert!(evidence.is_some_and(|value| !value.is_high_confidence()));
        }

        let proxy_error = anyhow::Error::new(ProviderApiError::from_http_response(
            "OpenAI-compatible proxy",
            413,
            r#"{"error":{"type":"request_too_large","message":"request too large"}}"#,
        ));
        let (reason, evidence) = classify_error_with_evidence(&proxy_error);
        assert_eq!(reason, FailoverReason::Unknown);
        assert!(evidence.is_some_and(|value| !value.is_high_confidence()));

        let untyped_bad_request = anyhow::Error::new(ProviderApiError::from_http_response(
            "opaque proxy",
            400,
            r#"{"error":{"message":"prompt is too long"}}"#,
        ));
        let (reason, evidence) = classify_error_with_evidence(&untyped_bad_request);
        assert_eq!(reason, FailoverReason::Unknown);
        assert!(evidence.is_some_and(|value| !value.is_high_confidence()));
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
        assert!(FailoverReason::Unknown.is_retryable());
        assert!(!FailoverReason::Auth.is_retryable());
        assert!(!FailoverReason::ContextOverflow.is_retryable());
        assert!(!FailoverReason::CurrentToolGroupOverflow.is_retryable());
        assert!(!FailoverReason::DispatchUnknown.is_retryable());
    }

    #[test]
    fn test_terminal() {
        // ContextOverflow is no longer terminal — it triggers compaction first.
        assert_eq!(
            classify_error("evaluation budget exhausted: model_calls"),
            FailoverReason::EvaluationBudget
        );
        assert!(FailoverReason::EvaluationBudget.is_terminal());
        assert!(!FailoverReason::EvaluationBudget.is_retryable());
        assert!(!FailoverReason::EvaluationBudget.is_profile_rotatable());
        assert!(!FailoverReason::ContextOverflow.is_terminal());
        assert!(FailoverReason::CurrentToolGroupOverflow.is_terminal());
        assert!(FailoverReason::DispatchUnknown.is_terminal());
        assert!(!FailoverReason::DispatchUnknown.is_retryable());
        assert!(!FailoverReason::DispatchUnknown.is_profile_rotatable());
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

    #[test]
    fn retry_after_hint_shares_the_bounded_client_retry_delay() {
        let hinted = effective_retry_delay_ms(0, 1000, 10_000, Some(7_000));
        assert_eq!(hinted, 7_000);

        let capped = effective_retry_delay_ms(0, 1000, 10_000, Some(60_000));
        assert_eq!(capped, 10_000);

        let provider_error = ProviderApiError::from_http_response(
            "BytePlus",
            429,
            r#"{"error":{"code":"RequestBurstTooFast"}}"#,
        )
        .with_retry_after_header(Some("3"));
        assert_eq!(provider_error.retry_after_ms(), Some(3_000));
        assert_eq!(
            classify_error_with_evidence(&anyhow::Error::new(provider_error)).0,
            FailoverReason::RateLimit
        );
    }

    // ── Profile rotation tests ──────────────────────────────────

    #[test]
    fn test_is_profile_rotatable() {
        assert!(FailoverReason::RateLimit.is_profile_rotatable());
        assert!(FailoverReason::Overloaded.is_profile_rotatable());
        assert!(FailoverReason::Auth.is_profile_rotatable());
        assert!(FailoverReason::Billing.is_profile_rotatable());
        assert!(!FailoverReason::Timeout.is_profile_rotatable());
        assert!(!FailoverReason::ModelNotFound.is_profile_rotatable());
        assert!(!FailoverReason::ContextOverflow.is_profile_rotatable());
        assert!(!FailoverReason::DispatchUnknown.is_profile_rotatable());
        assert!(!FailoverReason::Unknown.is_profile_rotatable());
    }

    #[test]
    fn test_profile_cooldown_secs() {
        assert_eq!(FailoverReason::Overloaded.profile_cooldown_secs(), 30);
        assert_eq!(FailoverReason::RateLimit.profile_cooldown_secs(), 60);
        assert_eq!(FailoverReason::Auth.profile_cooldown_secs(), 300);
        assert_eq!(FailoverReason::Billing.profile_cooldown_secs(), 600);
        assert_eq!(FailoverReason::Timeout.profile_cooldown_secs(), 0);
    }

    #[test]
    fn test_cooldown_tracker_basic() {
        let tracker = ProfileCooldownTracker::new();
        assert!(tracker.is_available("p1"));

        tracker.mark_cooldown("p1", &FailoverReason::RateLimit);
        assert!(!tracker.is_available("p1"));
        assert!(tracker.is_available("p2"));

        tracker.clear("p1");
        assert!(tracker.is_available("p1"));
    }

    #[test]
    fn test_cooldown_zero_duration_not_tracked() {
        let tracker = ProfileCooldownTracker::new();
        tracker.mark_cooldown("p1", &FailoverReason::Timeout); // 0 secs
        assert!(tracker.is_available("p1"));
    }

    #[test]
    fn test_sticky_map_basic() {
        let sticky = ProfileStickyMap::new();
        assert!(sticky.get("prov1", "sess1").is_none());

        sticky.set("prov1", "sess1", "profile-a");
        assert_eq!(sticky.get("prov1", "sess1").as_deref(), Some("profile-a"));
        assert!(sticky.get("prov1", "sess2").is_none());
    }

    #[test]
    fn test_sticky_map_lru_eviction_preserves_recent() {
        // Hit the cap + 1 with distinct sessions; oldest is evicted but
        // newer ones must survive (old `clear()` wiped everything).
        let sticky = ProfileStickyMap::new();
        for i in 0..STICKY_MAX_SESSIONS_PER_PROVIDER {
            sticky.set("prov1", &format!("sess{}", i), "profile-a");
        }
        // One past the cap triggers eviction of sess0.
        sticky.set(
            "prov1",
            &format!("sess{}", STICKY_MAX_SESSIONS_PER_PROVIDER),
            "profile-a",
        );
        assert!(
            sticky.get("prov1", "sess0").is_none(),
            "oldest entry should have been evicted"
        );
        // Recently used entries must still be present.
        assert_eq!(
            sticky.get("prov1", "sess1").as_deref(),
            Some("profile-a"),
            "recent entries must not be wiped by cap enforcement"
        );
        assert_eq!(
            sticky
                .get(
                    "prov1",
                    &format!("sess{}", STICKY_MAX_SESSIONS_PER_PROVIDER)
                )
                .as_deref(),
            Some("profile-a"),
            "newest entry must be present"
        );
    }

    #[test]
    fn test_sticky_map_lru_promotes_on_get() {
        let sticky = ProfileStickyMap::new();
        // Fill up to the cap so the next insert triggers exactly one
        // eviction. Seed the two oldest entries first so we can observe
        // the promotion effect before fillers arrive.
        sticky.set("prov1", "sess-a", "profile-a");
        sticky.set("prov1", "sess-b", "profile-b");
        for i in 0..(STICKY_MAX_SESSIONS_PER_PROVIDER - 2) {
            sticky.set("prov1", &format!("filler{}", i), "profile-a");
        }
        // Promote sess-a so sess-b is now the oldest.
        assert_eq!(sticky.get("prov1", "sess-a").as_deref(), Some("profile-a"));
        // Next insert overflows the cap by one → pop_front evicts sess-b.
        sticky.set("prov1", "trigger", "profile-a");
        assert_eq!(
            sticky.get("prov1", "sess-a").as_deref(),
            Some("profile-a"),
            "promoted entry must survive eviction"
        );
        assert!(
            sticky.get("prov1", "sess-b").is_none(),
            "untouched older entry should have been evicted"
        );
    }

    #[test]
    fn test_select_profile_basic() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut cfg = ProviderConfig::new(
            "test".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            String::new(),
        );
        cfg.auth_profiles = vec![
            AuthProfile::new("A".into(), "key-a".into(), None),
            AuthProfile::new("B".into(), "key-b".into(), None),
        ];

        let selected = select_profile(&cfg, "sess1");
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().api_key, "key-a");
    }

    #[test]
    fn test_next_profile_excludes_tried() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut cfg = ProviderConfig::new(
            "test".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            String::new(),
        );
        let p1 = AuthProfile::new("A".into(), "key-a".into(), None);
        let p1_id = p1.id.clone();
        let p2 = AuthProfile::new("B".into(), "key-b".into(), None);
        cfg.auth_profiles = vec![p1, p2];

        let next = next_profile(&cfg, &[p1_id]);
        assert!(next.is_some());
        assert_eq!(next.unwrap().api_key, "key-b");
    }

    #[test]
    fn test_next_profile_all_tried() {
        use crate::provider::{ApiType, AuthProfile, ProviderConfig};
        let mut cfg = ProviderConfig::new(
            "test".into(),
            ApiType::Anthropic,
            "https://api.anthropic.com".into(),
            String::new(),
        );
        let p1 = AuthProfile::new("A".into(), "key-a".into(), None);
        let p1_id = p1.id.clone();
        let p2 = AuthProfile::new("B".into(), "key-b".into(), None);
        let p2_id = p2.id.clone();
        cfg.auth_profiles = vec![p1, p2];

        let next = next_profile(&cfg, &[p1_id, p2_id]);
        assert!(next.is_none());
    }
}

// ── Model Failover: Error Classification & Auth Profile Rotation ───
//
//  Classifies API errors to determine whether to retry the same model,
//  fall back to the next model, or surface the error directly.
//  Also provides per-profile cooldown tracking and session-sticky
//  profile selection for multi-key rotation within a single provider.

use serde::Serialize;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use crate::provider::{AuthProfile, ProviderConfig};

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
        // ContextOverflow is no longer terminal — it triggers compaction first.
        assert!(!FailoverReason::ContextOverflow.is_terminal());
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
                .get("prov1", &format!("sess{}", STICKY_MAX_SESSIONS_PER_PROVIDER))
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

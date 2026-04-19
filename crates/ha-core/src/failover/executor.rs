//! Generic failover executor: profile rotation + retry-with-backoff +
//! cooldown + sticky bookkeeping wrapped around any async operation.
//!
//! Phase 3 of the LLM call unification: lifts the failover orchestration
//! that used to be hand-rolled inline in [`crate::chat_engine::engine`]
//! into a reusable executor that one-shot paths ([`crate::agent::side_query`]
//! and [`crate::agent::context::summarize_direct`]) can also opt into.
//!
//! ## Design
//!
//! - Generic `execute_with_failover<T, F, Fut>` function, not a trait — each
//!   caller's "operation" is a closure over `&AssistantAgent` /
//!   `&LlmProvider`, so a trait would just pile on associated types and
//!   `Send` bounds without any reuse benefit.
//! - Uses the same [`super::FailoverReason`] / [`super::PROFILE_COOLDOWNS`] /
//!   [`super::PROFILE_STICKY`] / [`super::select_profile`] / [`super::next_profile`]
//!   primitives as the inline implementation, so error classification and
//!   cooldown semantics are shared.
//! - Emergency compaction is **not** handled inside the executor. Closures
//!   borrowing `&mut AssistantAgent` for compact + retry would conflict with
//!   the operation closure's borrow; instead the executor returns
//!   [`ExecutorError::NeedsCompaction`] carrying the profile that just hit
//!   ContextOverflow so the outer chat_engine can compact, write that profile
//!   back into [`super::PROFILE_STICKY`] (so retry hits the same key →
//!   prompt cache prefix preserved), and call the executor again.

use std::future::Future;
use std::time::Duration;

use crate::provider::{ApiType, AuthProfile, ProviderConfig};

use super::{
    classify_error, next_profile, retry_delay_ms, select_profile, FailoverReason,
    PROFILE_COOLDOWNS, PROFILE_STICKY,
};

/// Per-call-site failover behavior — different code paths want different
/// retry / rotation policies (chat_engine vs side_query vs summarize).
#[derive(Debug, Clone)]
pub struct FailoverPolicy {
    /// Maximum same-profile retry attempts for retryable errors (RateLimit /
    /// Overloaded / Timeout) before giving up.
    pub max_retries: u32,
    /// Whether `is_profile_rotatable` errors should rotate to the next
    /// auth profile. Set `false` for paths that need to fail fast (e.g.
    /// summarize_direct → CompactionProvider fallback chain).
    pub allow_profile_rotation: bool,
    /// Base delay for exponential backoff (ms).
    pub retry_base_ms: u64,
    /// Max clamped delay for exponential backoff (ms).
    pub retry_max_ms: u64,
}

impl FailoverPolicy {
    /// chat_engine main loop default (matches pre-Phase-3 hand-rolled values).
    pub fn chat_engine_default() -> Self {
        Self {
            max_retries: 2,
            allow_profile_rotation: true,
            retry_base_ms: 1000,
            retry_max_ms: 10000,
        }
    }

    /// side_query default — allow profile rotation but limit retries (low-frequency
    /// path, multi-profile rotation acceptable but multi-second backoff is not).
    pub fn side_query_default() -> Self {
        Self {
            max_retries: 1,
            allow_profile_rotation: true,
            retry_base_ms: 1000,
            retry_max_ms: 10000,
        }
    }

    /// Tier 3 dedicated summarize default — fail fast (no profile rotation),
    /// so the upper layer drops to the side_query fallback or emergency
    /// compaction quickly. The caller's `DedicatedModelProvider` is bound to
    /// one specific provider:model pair, and rotating through that pair's
    /// profiles when a budget summary is in progress just adds latency to
    /// the user's main turn.
    pub fn summarize_default() -> Self {
        Self {
            max_retries: 2,
            allow_profile_rotation: false,
            retry_base_ms: 1000,
            retry_max_ms: 10000,
        }
    }
}

/// Executor outcome on failure. Successful operations return `Ok(T)` directly.
#[derive(Debug)]
pub enum ExecutorError {
    /// `ContextOverflow` was hit — the outer chat_engine should run an
    /// emergency compaction and call the executor again. `last_profile` carries
    /// the profile that just failed so the outer can write it back into
    /// `PROFILE_STICKY` before retry, ensuring retry lands on the same key
    /// (preserves the Anthropic prompt-cache prefix that the upcoming compact
    /// will not invalidate).
    NeedsCompaction { last_profile: Option<AuthProfile> },
    /// All available retries / profile rotations exhausted. Last classification
    /// + raw error message preserved for the outer to log / surface.
    Exhausted {
        last_reason: FailoverReason,
        last_error: String,
    },
    /// Provider has no usable auth profile (all in cooldown, all `enabled=false`,
    /// or `auth_profiles` empty + `api_key` empty). Distinct from `Exhausted`
    /// because no operation was ever attempted.
    NoProfileAvailable,
}

impl ExecutorError {
    /// Pretty-print for log output. Equivalent to the `Display` impl;
    /// kept as a named method for callers that want to be explicit.
    pub fn describe(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsCompaction { .. } => write!(f, "context overflow → needs compaction"),
            Self::Exhausted {
                last_reason,
                last_error,
            } => write!(f, "exhausted ({:?}): {}", last_reason, last_error),
            Self::NoProfileAvailable => write!(f, "no auth profile available"),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Run an async operation with profile rotation + retry-with-backoff.
///
/// `operation` is called as `operation(Some(&profile))` for providers with
/// auth profiles, or `operation(None)` for Codex / OAuth providers whose
/// `effective_profiles()` returns an empty `Vec`.
///
/// On success: marks `PROFILE_STICKY` for next-turn affinity and clears
/// `PROFILE_COOLDOWNS` for that profile.
///
/// On `is_profile_rotatable` error (and `policy.allow_profile_rotation`):
/// marks cooldown, calls `on_profile_rotation` callback, picks the next
/// profile via `next_profile`, retry count resets.
///
/// On `is_retryable` error: exponential backoff up to `policy.max_retries`.
///
/// On `needs_compaction` error: returns `NeedsCompaction` carrying the
/// profile that failed (outer handles compact + retry).
///
/// Codex / OAuth providers have `allow_profile_rotation` forced to `false`
/// regardless of `policy` — `effective_profiles()` returns empty so there's
/// nothing to rotate to.
pub async fn execute_with_failover<T, F, Fut>(
    provider: &ProviderConfig,
    session_id: &str,
    policy: FailoverPolicy,
    on_profile_rotation: Option<&(dyn Fn(&AuthProfile, &AuthProfile, &FailoverReason) + Sync)>,
    mut operation: F,
) -> Result<T, ExecutorError>
where
    F: FnMut(Option<&AuthProfile>) -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    // Codex defense-in-depth: even if the caller passes
    // `allow_profile_rotation: true` we force it off because Codex's
    // `effective_profiles()` is always empty by design and rotation would
    // immediately bail out anyway.
    let allow_rotation = policy.allow_profile_rotation && provider.api_type != ApiType::Codex;

    let mut current_profile = select_profile(provider, session_id);
    let mut tried_profiles: Vec<String> = Vec::new();
    if let Some(ref p) = current_profile {
        tried_profiles.push(p.id.clone());
    }
    let mut retry_count: u32 = 0;

    // Codex case: effective_profiles() is empty → current_profile is None.
    // We still want to call operation(None) at least once. The loop below
    // handles this naturally: rotation/sticky paths are no-ops without a
    // profile, retry / classify paths still run.

    loop {
        match operation(current_profile.as_ref()).await {
            Ok(value) => {
                if let Some(ref profile) = current_profile {
                    PROFILE_COOLDOWNS.clear(&profile.id);
                    PROFILE_STICKY.set(&provider.id, session_id, &profile.id);
                }
                return Ok(value);
            }
            Err(e) => {
                let err_str = e.to_string();
                let reason = classify_error(&err_str);

                if reason.needs_compaction() {
                    return Err(ExecutorError::NeedsCompaction {
                        last_profile: current_profile,
                    });
                }

                if reason.is_terminal() {
                    return Err(ExecutorError::Exhausted {
                        last_reason: reason,
                        last_error: err_str,
                    });
                }

                // Profile rotation path
                if reason.is_profile_rotatable() && allow_rotation {
                    if let Some(ref profile) = current_profile {
                        PROFILE_COOLDOWNS.mark_cooldown(&profile.id, &reason);
                    }
                    if let Some(next) = next_profile(provider, &tried_profiles) {
                        if let Some(ref cb) = on_profile_rotation {
                            if let Some(ref prev) = current_profile {
                                cb(prev, &next, &reason);
                            }
                        }
                        tried_profiles.push(next.id.clone());
                        current_profile = Some(next);
                        retry_count = 0;
                        continue;
                    }
                    // No more profiles to try → fall through to exhausted.
                    return Err(ExecutorError::Exhausted {
                        last_reason: reason,
                        last_error: err_str,
                    });
                }

                // Retry-on-same-profile path (only for retryable errors that
                // either aren't profile-rotatable or whose rotation we already
                // skipped because policy.allow_profile_rotation is false).
                if reason.is_retryable() && retry_count < policy.max_retries {
                    let delay =
                        retry_delay_ms(retry_count, policy.retry_base_ms, policy.retry_max_ms);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    retry_count += 1;
                    continue;
                }

                // Non-retryable, non-rotatable, non-compactable → give up.
                return Err(ExecutorError::Exhausted {
                    last_reason: reason,
                    last_error: err_str,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ApiType, AuthProfile};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Helper: build a provider with N enabled auth profiles.
    fn make_provider(n_profiles: usize) -> (ProviderConfig, Vec<String>) {
        let mut cfg = ProviderConfig::new(
            format!("test-prov-{}", uuid::Uuid::new_v4()),
            ApiType::Anthropic,
            "https://api.test/".into(),
            String::new(),
        );
        let profiles: Vec<AuthProfile> = (0..n_profiles)
            .map(|i| AuthProfile::new(format!("P{}", i), format!("key-{}", i), None))
            .collect();
        let ids: Vec<String> = profiles.iter().map(|p| p.id.clone()).collect();
        cfg.auth_profiles = profiles;
        (cfg, ids)
    }

    #[tokio::test]
    async fn first_attempt_success_marks_sticky() {
        let (cfg, ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |profile| {
                let has = profile.is_some();
                async move {
                    assert!(has);
                    Ok("ok".to_string())
                }
            },
        )
        .await;

        assert!(matches!(result, Ok(s) if s == "ok"));
        // Sticky should be the first profile.
        assert_eq!(
            PROFILE_STICKY.get(&cfg.id, &session).as_deref(),
            Some(ids[0].as_str())
        );
    }

    #[tokio::test]
    async fn auth_error_rotates_to_next_profile() {
        let (cfg, ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);
        let observed_rotation = Arc::new(std::sync::Mutex::new(Vec::new()));

        let observed_clone = observed_rotation.clone();
        let cb = move |from: &AuthProfile, to: &AuthProfile, reason: &FailoverReason| {
            observed_clone
                .lock()
                .unwrap()
                .push((from.id.clone(), to.id.clone(), reason.clone()));
        };

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            Some(&cb),
            |profile| {
                let n = attempt.fetch_add(1, Ordering::SeqCst);
                let key = profile.map(|p| p.api_key.clone()).unwrap_or_default();
                async move {
                    if n == 0 {
                        // First attempt → 401
                        Err(anyhow::anyhow!("401 Unauthorized for key {}", key))
                    } else {
                        // Second attempt → success
                        Ok(format!("ok with {}", key))
                    }
                }
            },
        )
        .await;

        assert!(matches!(result, Ok(s) if s == "ok with key-1"));
        let rotations = observed_rotation.lock().unwrap();
        assert_eq!(rotations.len(), 1);
        assert_eq!(rotations[0].0, ids[0]);
        assert_eq!(rotations[0].1, ids[1]);
        assert_eq!(rotations[0].2, FailoverReason::Auth);
        // After all attempts, sticky should track the second profile.
        assert_eq!(
            PROFILE_STICKY.get(&cfg.id, &session).as_deref(),
            Some(ids[1].as_str())
        );
    }

    #[tokio::test]
    async fn rate_limit_retries_then_exhausts() {
        let (cfg, _ids) = make_provider(1);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        // policy with retries=1 + retry_base=10ms (fast test)
        let policy = FailoverPolicy {
            max_retries: 1,
            allow_profile_rotation: false, // force retry path, not rotation
            retry_base_ms: 10,
            retry_max_ms: 20,
        };

        let result: Result<String, _> =
            execute_with_failover(&cfg, &session, policy, None, |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("429 Too Many Requests")) }
            })
            .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::RateLimit,
                ..
            })
        ));
        // 1 initial + 1 retry = 2 attempts.
        assert_eq!(attempt.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn context_overflow_returns_needs_compaction_with_profile() {
        let (cfg, ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |_profile| async move { Err(anyhow::anyhow!("context_length_exceeded")) },
        )
        .await;

        match result {
            Err(ExecutorError::NeedsCompaction { last_profile }) => {
                assert!(last_profile.is_some());
                assert_eq!(last_profile.unwrap().id, ids[0]);
            }
            other => panic!("expected NeedsCompaction, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn auth_error_with_rotation_disabled_exhausts_immediately() {
        let (cfg, _ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());

        let policy = FailoverPolicy {
            max_retries: 0,
            allow_profile_rotation: false,
            retry_base_ms: 1,
            retry_max_ms: 1,
        };

        let attempt = AtomicU32::new(0);
        let result: Result<String, _> =
            execute_with_failover(&cfg, &session, policy, None, |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("401 Unauthorized")) }
            })
            .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::Auth,
                ..
            })
        ));
        assert_eq!(attempt.load(Ordering::SeqCst), 1, "no rotation, no retry");
    }

    #[tokio::test]
    async fn all_profiles_auth_failed_returns_exhausted() {
        let (cfg, _ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("401 Unauthorized")) }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::Auth,
                ..
            })
        ));
        // 2 profiles tried.
        assert_eq!(attempt.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn model_not_found_does_not_rotate_or_retry() {
        let (cfg, _ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("404 model not found")) }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::ModelNotFound,
                ..
            })
        ));
        assert_eq!(attempt.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_profiles_returns_no_profile_available_for_codex() {
        let mut cfg = ProviderConfig::new(
            format!("codex-{}", uuid::Uuid::new_v4()),
            ApiType::Codex,
            "https://chatgpt.com/".into(),
            String::new(),
        );
        cfg.auth_profiles = Vec::new();
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        // Codex op succeeds with None profile (uses OAuth out-of-band).
        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                let has_profile = profile.is_some();
                async move {
                    assert!(!has_profile, "Codex always gets None profile");
                    Ok("codex_ok".to_string())
                }
            },
        )
        .await;

        assert!(matches!(result, Ok(s) if s == "codex_ok"));
        assert_eq!(attempt.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn codex_auth_error_does_not_rotate_even_if_policy_allows() {
        let mut cfg = ProviderConfig::new(
            format!("codex-{}", uuid::Uuid::new_v4()),
            ApiType::Codex,
            "https://chatgpt.com/".into(),
            String::new(),
        );
        // Even if someone weirdly added auth_profiles to a Codex config,
        // executor must NOT rotate them.
        cfg.auth_profiles = vec![
            AuthProfile::new("A".into(), "a".into(), None),
            AuthProfile::new("B".into(), "b".into(), None),
        ];
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(), // allow_profile_rotation=true
            None,
            |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("401 Unauthorized")) }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::Auth,
                ..
            })
        ));
        // Only 1 attempt — Codex defense-in-depth blocked rotation.
        assert_eq!(attempt.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_error_exhausts_immediately() {
        let (cfg, _ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("some random gibberish")) }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::Unknown,
                ..
            })
        ));
        assert_eq!(attempt.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn timeout_retries_then_succeeds() {
        let (cfg, _ids) = make_provider(1);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let policy = FailoverPolicy {
            max_retries: 2,
            allow_profile_rotation: false,
            retry_base_ms: 5,
            retry_max_ms: 10,
        };

        let result: Result<String, _> =
            execute_with_failover(&cfg, &session, policy, None, |_profile| {
                let n = attempt.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        Err(anyhow::anyhow!("request timed out"))
                    } else {
                        Ok("ok".to_string())
                    }
                }
            })
            .await;

        assert!(matches!(result, Ok(s) if s == "ok"));
        assert_eq!(attempt.load(Ordering::SeqCst), 3); // 1 + 2 retries
    }

    #[tokio::test]
    async fn billing_error_rotates_then_exhausts() {
        let (cfg, ids) = make_provider(2);
        let session = format!("sess-{}", uuid::Uuid::new_v4());
        let attempt = AtomicU32::new(0);

        let result: Result<String, _> = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |_profile| {
                attempt.fetch_add(1, Ordering::SeqCst);
                async move { Err(anyhow::anyhow!("402 payment required quota exceeded")) }
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(ExecutorError::Exhausted {
                last_reason: FailoverReason::Billing,
                ..
            })
        ));
        assert_eq!(attempt.load(Ordering::SeqCst), 2); // both profiles tried
                                                       // Both should be in cooldown after billing failures.
        assert!(!PROFILE_COOLDOWNS.is_available(&ids[0]));
        assert!(!PROFILE_COOLDOWNS.is_available(&ids[1]));
        // Cleanup so other tests don't see these in cooldown.
        PROFILE_COOLDOWNS.clear(&ids[0]);
        PROFILE_COOLDOWNS.clear(&ids[1]);
    }

    #[tokio::test]
    async fn sticky_returned_on_subsequent_calls() {
        let (cfg, ids) = make_provider(3);
        let session = format!("sess-{}", uuid::Uuid::new_v4());

        // First call: rotate from p0 (auth fail) → p1 (success).
        let attempt1 = AtomicU32::new(0);
        let _ = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |profile| {
                let n = attempt1.fetch_add(1, Ordering::SeqCst);
                let key = profile.map(|p| p.api_key.clone()).unwrap_or_default();
                async move {
                    if n == 0 {
                        Err(anyhow::anyhow!("401 Unauthorized"))
                    } else {
                        Ok(key)
                    }
                }
            },
        )
        .await;

        // Sticky should now point at p1.
        assert_eq!(
            PROFILE_STICKY.get(&cfg.id, &session).as_deref(),
            Some(ids[1].as_str())
        );

        // Cleanup p0 cooldown so it's not still cooled in subsequent tests.
        PROFILE_COOLDOWNS.clear(&ids[0]);

        // Second call should land on p1 first (sticky).
        let attempt2 = AtomicU32::new(0);
        let observed_first_key = std::sync::Mutex::new(String::new());
        let _ = execute_with_failover(
            &cfg,
            &session,
            FailoverPolicy::chat_engine_default(),
            None,
            |profile| {
                let n = attempt2.fetch_add(1, Ordering::SeqCst);
                let key = profile.map(|p| p.api_key.clone()).unwrap_or_default();
                if n == 0 {
                    *observed_first_key.lock().unwrap() = key.clone();
                }
                async move { Ok(key) }
            },
        )
        .await;

        assert_eq!(*observed_first_key.lock().unwrap(), "key-1");
    }
}

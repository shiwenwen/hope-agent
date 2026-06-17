//! R7.4 retry policy for backgrounded tool jobs.
//!
//! A backgrounded job that fails may be retried with exponential backoff — but
//! **conservatively, and OPT-IN by default** (`async_tools.retry_enabled`
//! defaults to `false`). Two safety rails:
//!
//! 1. **Eligibility is a CODE-LEVEL allowlist** ([`is_retry_eligible`]), not a
//!    user knob — only *idempotent, re-runnable* tools (`web_search` /
//!    `web_fetch`) qualify. `exec` is excluded (a shell command may have applied
//!    a partial side effect before failing — a blind re-run could repeat it);
//!    `image_generate` is excluded (re-running yields a *different*
//!    non-deterministic image, so a retry isn't "the same operation").
//! 2. **Default off.** Even an eligible tool re-RUNS on retry, and the eligible
//!    network tools are often backed by *paid* providers (e.g. `web_search` via
//!    Brave / Tavily / Google). The job layer can't reliably tell a transient
//!    failure (429 / 5xx — worth retrying) from a deterministic one (400 bad
//!    query — a wasted re-bill), so retry is opt-in to avoid surprise charges.
//!
//! Only `JobError::Failed` is ever retried (a clean dispatch error). Cancels,
//! policy denials, and timeouts never retry (see [`decide`]).
//!
//! This module is the pure decision layer: [`decide`] takes the tool name, the
//! attempt number, the terminal [`JobError`], and the resolved [`RetryConfig`],
//! and returns whether to retry (and the backoff). The worker loop in
//! [`super::spawn`] drives it. Keeping the policy pure makes it unit-testable
//! without a runtime.

use super::error::JobError;

/// Fixed exponential-backoff base. Not a user knob — the user-facing tunables
/// are the master switch + attempt count; the timing curve is fixed so a typo
/// can't turn retries into a multi-minute stall.
const BASE_BACKOFF_MS: u64 = 500;

/// Cap on the backoff shift so `1 << n` can never overflow / explode for a
/// pathological `max_retry_attempts`.
const MAX_BACKOFF_SHIFT: u32 = 6; // 500ms * 2^6 = 32s ceiling

/// Hard upper bound on total attempts, regardless of the configured
/// `max_retry_attempts`. A retrying job pins a concurrency slot for its whole
/// multi-attempt lifetime, so an absurd config value can't be allowed to retry
/// a persistently-failing (and possibly paid) tool hundreds of times.
const MAX_ATTEMPTS_CAP: u32 = 10;

/// Resolved retry knobs (snapshot of `async_tools.{retry_enabled,max_retry_attempts}`).
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub enabled: bool,
    /// Total attempts (1 = no retry; the initial try counts).
    pub max_attempts: u32,
}

impl RetryConfig {
    /// Read the live config snapshot.
    pub fn current() -> Self {
        let cfg = crate::config::cached_config();
        Self {
            enabled: cfg.async_tools.retry_enabled,
            max_attempts: cfg.async_tools.max_retry_attempts,
        }
    }
}

/// What to do after an attempt failed.
#[derive(Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Settle the job with the failure — no (more) retries.
    Stop,
    /// Retry after sleeping `backoff_ms` (cancellable).
    Retry { backoff_ms: u64 },
}

/// Tools that are *idempotent / safe to re-run* — re-executing the same call
/// neither corrupts external state nor produces a logically different operation,
/// so a backgrounded job MAY auto-retry them (when retry is enabled).
///
/// Deliberately NOT here (never auto-retried, by design):
/// - `exec` — a shell command may have applied a partial side effect before
///   failing; a blind re-run could repeat it (not idempotent).
/// - `image_generate` — re-running returns a *different* non-deterministic image
///   and re-bills; the retry isn't "the same operation that failed".
///
/// NOTE on cost: an eligible tool still re-RUNS, and `web_search` is often a
/// *paid* per-query provider — that cost is the reason retry defaults OFF (see
/// the module doc), not a reason to drop eligibility (re-running a search is
/// state-safe; the user opts into the small re-bill). Only async-capable tools
/// ever reach the job path; today that's `web_search` among the eligible set
/// (`web_fetch` is listed for forward-compatibility but is not async-capable
/// yet, so it never becomes a background job).
///
/// **Adding a tool here:** it MUST be idempotent AND must not register an
/// `output_tail` ring (only `exec` does today) — a retried tail tool would
/// re-stream into the once-registered ring. [`super::spawn::run_tool_with_retry`]
/// `debug_assert`s the latter.
pub fn is_retry_eligible(tool_name: &str) -> bool {
    matches!(
        tool_name,
        crate::tools::TOOL_WEB_SEARCH | crate::tools::TOOL_WEB_FETCH
    )
}

/// Decide whether to retry tool `tool_name` after its `attempt`-th run (1-based)
/// ended with `error`.
///
/// Rules:
/// - Retry only [`JobError::Failed`] (a transient tool error where the dispatch
///   returned cleanly — e.g. a network blip / rate-limit from `web_search`), and
///   only when retries are enabled, the tool is [`is_retry_eligible`], and
///   `attempt < max_attempts`. Backoff is exponential from [`BASE_BACKOFF_MS`].
/// - **Never** retry [`JobError::Cancelled`] (terminal user/session cancel),
///   [`JobError::DeniedByUser`] (deterministic — a re-run won't be approved), or
///   [`JobError::TimedOut`]. Timeout is deliberately excluded: the per-job
///   timeout cancels the *shared* job token (so the worker can't cleanly start a
///   fresh attempt without per-attempt token plumbing), and a tool that
///   exhausted its time budget is likely to time out again. Failed covers the
///   dominant transient case (API errors), which is the safe, useful scope.
pub fn decide(tool_name: &str, attempt: u32, error: &JobError, cfg: &RetryConfig) -> RetryDecision {
    match error {
        // The only retryable class: a clean dispatch error (token untouched).
        JobError::Failed { .. } => {}
        // Terminal regardless of policy (see fn doc for why TimedOut is here).
        JobError::Cancelled | JobError::DeniedByUser { .. } | JobError::TimedOut { .. } => {
            return RetryDecision::Stop
        }
    }
    if !cfg.enabled || cfg.max_attempts <= 1 || !is_retry_eligible(tool_name) {
        return RetryDecision::Stop;
    }
    // Clamp the configured attempt count to a sane ceiling (a retrying job holds
    // a concurrency slot for its whole multi-attempt life — an absurd value
    // mustn't be honored verbatim).
    let max_attempts = cfg.max_attempts.min(MAX_ATTEMPTS_CAP);
    if attempt >= max_attempts {
        return RetryDecision::Stop;
    }
    // attempt is 1-based; the 1st failure backs off 500ms, 2nd 1s, …
    let shift = (attempt - 1).min(MAX_BACKOFF_SHIFT);
    RetryDecision::Retry {
        backoff_ms: BASE_BACKOFF_MS.saturating_mul(1u64 << shift),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool, max: u32) -> RetryConfig {
        RetryConfig {
            enabled,
            max_attempts: max,
        }
    }
    fn failed() -> JobError {
        JobError::Failed {
            message: "boom".into(),
        }
    }

    #[test]
    fn eligible_tool_retries_until_attempts_exhausted() {
        let c = cfg(true, 3);
        assert_eq!(
            decide("web_search", 1, &failed(), &c),
            RetryDecision::Retry { backoff_ms: 500 }
        );
        assert_eq!(
            decide("web_search", 2, &failed(), &c),
            RetryDecision::Retry { backoff_ms: 1000 }
        );
        // 3rd attempt is the last (max_attempts=3) → stop.
        assert_eq!(decide("web_search", 3, &failed(), &c), RetryDecision::Stop);
    }

    #[test]
    fn side_effect_tools_never_retry() {
        let c = cfg(true, 3);
        assert_eq!(decide("exec", 1, &failed(), &c), RetryDecision::Stop);
        assert_eq!(
            decide("image_generate", 1, &failed(), &c),
            RetryDecision::Stop
        );
    }

    #[test]
    fn cancelled_and_denied_never_retry_even_for_eligible_tool() {
        let c = cfg(true, 5);
        assert_eq!(
            decide("web_search", 1, &JobError::Cancelled, &c),
            RetryDecision::Stop
        );
        let denied = JobError::DeniedByUser {
            rejection: crate::tools::rejection::ToolRejection::DeniedByUser {
                name: "web_search".into(),
            },
        };
        assert_eq!(decide("web_search", 1, &denied, &c), RetryDecision::Stop);
    }

    #[test]
    fn timeout_never_retries_even_for_eligible_tool() {
        // TimedOut is deliberately excluded (see decide() doc): the per-job
        // timeout cancels the shared job token, and a budget-exhausted tool
        // tends to time out again. Only Failed retries.
        let c = cfg(true, 3);
        assert_eq!(
            decide("web_search", 1, &JobError::TimedOut { max_secs: 30 }, &c),
            RetryDecision::Stop
        );
        assert_eq!(
            decide("exec", 1, &JobError::TimedOut { max_secs: 30 }, &c),
            RetryDecision::Stop
        );
    }

    #[test]
    fn disabled_or_single_attempt_never_retries() {
        assert_eq!(
            decide("web_search", 1, &failed(), &cfg(false, 3)),
            RetryDecision::Stop,
            "master switch off"
        );
        assert_eq!(
            decide("web_search", 1, &failed(), &cfg(true, 1)),
            RetryDecision::Stop,
            "max_attempts=1 means no retry"
        );
    }

    #[test]
    fn backoff_is_shift_capped() {
        // A high attempt index must not overflow the shift; backoff saturates at
        // the 2^6 ceiling. attempt 9 is the last retrying attempt under the
        // MAX_ATTEMPTS_CAP (10), and (9-1).min(6) = 6 → 500ms * 2^6.
        let c = cfg(true, MAX_ATTEMPTS_CAP);
        assert_eq!(
            decide("web_search", 9, &failed(), &c),
            RetryDecision::Retry {
                backoff_ms: 500 * (1u64 << MAX_BACKOFF_SHIFT)
            }
        );
    }

    #[test]
    fn max_attempts_is_clamped_to_cap() {
        // An absurd configured value must not retry past the hard cap.
        let c = cfg(true, 4_000_000_000);
        assert_eq!(
            decide("web_search", MAX_ATTEMPTS_CAP, &failed(), &c),
            RetryDecision::Stop,
            "attempt == cap must stop even when config asks for billions"
        );
        // ...but it still retries up to the cap.
        assert!(matches!(
            decide("web_search", MAX_ATTEMPTS_CAP - 1, &failed(), &c),
            RetryDecision::Retry { .. }
        ));
    }
}

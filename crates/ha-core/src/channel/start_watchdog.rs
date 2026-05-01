//! Background watchdog that retries failed channel auto-starts.
//!
//! The startup handshake (e.g. Telegram `getMe`, Slack `auth.test`,
//! Feishu WS endpoint discovery) used to be a one-shot: if the very first
//! attempt failed because the VPN/system proxy/Wi-Fi wasn't ready yet,
//! the channel stayed dead until the user noticed and clicked Restart.
//!
//! This module keeps a small in-memory map of accounts that failed to
//! start and retries each one on a backoff schedule until either the
//! handshake succeeds or the user explicitly stops/disables/removes the
//! account.
//!
//! Public API:
//! - [`register_failure`] — log + enqueue (or update) a retry entry.
//!   Single source for the failure log line so call sites stay one-liners.
//! - [`cancel_pending`] — drop a pending entry; user intent always wins.
//! - [`mark_success`] — drop the entry and log a recovery message.
//! - [`spawn_loop`] — spawn the retry task once during init; idempotent.
//!
//! `mark_success` and `cancel_pending` are on the hot path (every UI
//! Start/Stop) so they short-circuit via [`PENDING_COUNT`] when the map
//! is empty — the common case is "no failures ever", and that case must
//! not pay a mutex acquire.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

use super::registry::ChannelRegistry;
use super::types::ChannelAccountConfig;
use crate::failover::retry_delay_ms;

#[derive(Debug, Clone, Copy)]
struct PendingEntry {
    next_attempt: Instant,
    attempt_count: u32,
    /// Whether a `channel:auth_failed` event has already been emitted for
    /// this pending sequence. Used to fire the desktop alert exactly once
    /// per "needs user action" episode — flipping back to false only
    /// happens implicitly when the entry is removed (success / cancel).
    auth_alerted: bool,
}

static PENDING: Lazy<Arc<Mutex<HashMap<String, PendingEntry>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Lock-free fast path so `mark_success` / `cancel_pending` skip the
/// mutex when nothing's pending. Kept in sync with `PENDING` size.
static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);

static LOOP_SPAWNED: AtomicBool = AtomicBool::new(false);

/// Sweep cadence — how often we check for due entries when at least one
/// is pending. Per-entry retry delay is governed by [`backoff_for`].
const SWEEP_INTERVAL: Duration = Duration::from_secs(15);

/// Backoff schedule: 30s, 60s, 120s, 240s, then capped at 5m.
///
/// Reuses [`failover::retry_delay_ms`] so we get the same ±10% jitter as
/// the LLM retry path — important here too, because a shared VPN/proxy
/// flap can fail every channel at the same instant and unjittered fixed
/// delays would synchronize their retry storms.
fn backoff_for(attempt_count: u32) -> Duration {
    // attempt_count is 1-based (first failure → 1); retry_delay_ms is 0-based.
    let exp_attempt = attempt_count.saturating_sub(1);
    Duration::from_millis(retry_delay_ms(exp_attempt, 30_000, 300_000))
}

/// Log a failure and enqueue (or refresh) a retry entry.
///
/// All call sites collapse to one line; the log content (label,
/// channel_id, error chain, classifier hint, attempt count, next delay)
/// is constructed here so it stays consistent across boot / add / update.
pub async fn register_failure(account: &ChannelAccountConfig, error: &anyhow::Error) {
    let chain = format!("{:#}", error);
    let hint = classify_channel_error(&chain);
    let now = Instant::now();
    let needs_alert = needs_user_action(hint);

    let (attempt_count, backoff, should_emit) = {
        let mut pending = PENDING.lock().await;
        let was_new = !pending.contains_key(&account.id);
        let entry = pending
            .entry(account.id.clone())
            .or_insert_with(|| PendingEntry {
                next_attempt: now,
                attempt_count: 0,
                auth_alerted: false,
            });
        entry.attempt_count = entry.attempt_count.saturating_add(1);
        let backoff = backoff_for(entry.attempt_count);
        entry.next_attempt = now + backoff;
        let count = entry.attempt_count;
        // Emit the alert the first time auth/forbidden surfaces for this
        // account, regardless of whether earlier attempts failed for
        // recoverable reasons (network / DNS / proxy). Watchdog can heal
        // those on its own; auth needs the user. Once we've alerted, stay
        // quiet for the rest of this pending sequence to avoid spam.
        let should_emit = needs_alert && !entry.auth_alerted;
        if should_emit {
            entry.auth_alerted = true;
        }
        if was_new {
            PENDING_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        (count, backoff, should_emit)
    };

    crate::app_error!(
        "channel",
        "watchdog",
        "Channel auto-start failed for '{}' [{}/{}] (attempt {}): {} | hint: {} | next retry in {}s",
        account.label,
        account.channel_id,
        account.id,
        attempt_count,
        chain,
        hint,
        backoff.as_secs(),
    );

    if should_emit {
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "channel:auth_failed",
                serde_json::json!({
                    "accountId": account.id,
                    "label": account.label,
                    "channelId": account.channel_id,
                    "hint": hint,
                }),
            );
        }
    }
}

/// True for hints that the watchdog cannot fix on its own — only the user
/// can re-supply credentials or unblock the bot. Substring-matched against
/// the classifier output so the two stay in sync without a brittle enum.
fn needs_user_action(hint: &str) -> bool {
    hint.starts_with("auth/") || hint.starts_with("forbidden")
}

pub async fn cancel_pending(account_id: &str) {
    if PENDING_COUNT.load(Ordering::Relaxed) == 0 {
        return;
    }
    let mut pending = PENDING.lock().await;
    if pending.remove(account_id).is_some() {
        PENDING_COUNT.fetch_sub(1, Ordering::Relaxed);
        crate::app_info!(
            "channel",
            "watchdog",
            "Cancelled pending auto-start retries for '{}' (user action)",
            account_id
        );
    }
}

pub async fn mark_success(account_id: &str) {
    if PENDING_COUNT.load(Ordering::Relaxed) == 0 {
        return;
    }
    let mut pending = PENDING.lock().await;
    if let Some(entry) = pending.remove(account_id) {
        PENDING_COUNT.fetch_sub(1, Ordering::Relaxed);
        crate::app_info!(
            "channel",
            "watchdog",
            "Channel '{}' recovered after {} retry attempt(s)",
            account_id,
            entry.attempt_count,
        );
    }
}

pub fn spawn_loop(registry: Arc<ChannelRegistry>) {
    if LOOP_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        crate::app_info!(
            "channel",
            "watchdog",
            "Channel auto-start retry watchdog started (sweep every {}s)",
            SWEEP_INTERVAL.as_secs(),
        );
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await; // skip the immediate first tick
        loop {
            tick.tick().await;
            if PENDING_COUNT.load(Ordering::Relaxed) == 0 {
                continue;
            }
            for account_id in collect_due_ids().await {
                retry_one(&registry, &account_id).await;
            }
        }
    });
}

async fn collect_due_ids() -> Vec<String> {
    let now = Instant::now();
    let pending = PENDING.lock().await;
    pending
        .iter()
        .filter(|(_, entry)| entry.next_attempt <= now)
        .map(|(id, _)| id.clone())
        .collect()
}

async fn retry_one(registry: &Arc<ChannelRegistry>, account_id: &str) {
    // Re-fetch config — user may have edited credentials, disabled, or
    // deleted the account between failure and retry.
    let cfg = crate::config::cached_config();
    let Some(account) = cfg.channels.find_account(account_id) else {
        cancel_pending(account_id).await;
        return;
    };
    if !account.enabled {
        cancel_pending(account_id).await;
        return;
    }

    if registry.health(account_id).await.is_running {
        mark_success(account_id).await;
        return;
    }

    let owned = account.clone();
    crate::app_info!(
        "channel",
        "watchdog",
        "Retrying channel auto-start for '{}' [{}/{}]",
        owned.label,
        owned.channel_id,
        account_id,
    );

    match registry.start_account(&owned).await {
        Ok(()) => mark_success(account_id).await,
        Err(e) => register_failure(&owned, &e).await,
    }
}

/// Heuristic mapping from raw SDK error chain to a one-line user-facing
/// hint. Substring-matched (not typed) because each channel SDK
/// (teloxide, serenity, slack-morphism, ...) reports differently.
/// Order matters: more specific signals before broader ones.
fn classify_channel_error(s: &str) -> &'static str {
    let lower = s.to_ascii_lowercase();

    if lower.contains("certificate")
        || lower.contains("tls handshake")
        || lower.contains("handshake failed")
        || lower.contains("self-signed")
        || lower.contains("self signed")
    {
        return "TLS/cert error — proxy may be intercepting HTTPS, or system CA trust is misconfigured";
    }
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("invalid token") {
        return "auth/token rejected — verify credentials are correct and not revoked";
    }
    if lower.contains("403") || lower.contains("forbidden") {
        return "forbidden — bot may be blocked or lack required permissions";
    }
    if lower.contains("404") || lower.contains("not found") {
        return "endpoint not found — verify apiRoot / channel base URL";
    }
    if lower.contains("connection refused") {
        return "connection refused — proxy/local service is down or wrong port";
    }
    if lower.contains("dns")
        || lower.contains("name resolution")
        || lower.contains("failed to lookup")
    {
        return "DNS resolution failed — likely no internet or DNS proxy down";
    }
    if lower.contains("proxy") {
        return "proxy error — verify the configured proxy URL is reachable";
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return "request timed out — slow network, firewall drop, or proxy not yet up";
    }
    if lower.contains("error sending request") || lower.contains("connect") {
        return "network unreachable — check VPN/proxy and try again once online";
    }
    "unknown — see full error chain above"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_progression_caps_at_five_minutes() {
        // attempt 1 → ~30s, attempt 2 → ~60s, … cap at 300s.
        // ±10% jitter means we check ranges, not equality.
        let in_range = |actual: Duration, want_secs: u64| {
            let lo = (want_secs as f64 * 0.9 - 1.0) as u64;
            let hi = (want_secs as f64 * 1.1 + 1.0) as u64;
            (lo..=hi).contains(&actual.as_secs())
        };
        assert!(in_range(backoff_for(1), 30));
        assert!(in_range(backoff_for(2), 60));
        assert!(in_range(backoff_for(3), 120));
        assert!(in_range(backoff_for(4), 240));
        assert!(in_range(backoff_for(5), 300));
        assert!(in_range(backoff_for(50), 300));
    }

    #[test]
    fn classifier_recognises_telegram_network_error() {
        let s = "getMe failed: A network error: error sending request for url \
                 (https://api.telegram.org/token:redacted/GetMe)";
        let hint = classify_channel_error(s);
        assert!(hint.contains("network") || hint.contains("timed out"));
    }

    #[test]
    fn classifier_recognises_auth_failure() {
        assert!(classify_channel_error("HTTP 401 Unauthorized").contains("auth"));
        assert!(classify_channel_error("invalid token").contains("auth"));
    }

    #[test]
    fn classifier_recognises_dns_failure() {
        assert!(
            classify_channel_error("failed to lookup address: nodename nor servname")
                .contains("DNS")
        );
    }

    #[test]
    fn classifier_recognises_proxy_failure() {
        assert!(classify_channel_error("proxy connect failed").contains("proxy"));
    }

    #[test]
    fn classifier_falls_back_to_unknown() {
        assert!(classify_channel_error("something completely weird").contains("unknown"));
    }
}

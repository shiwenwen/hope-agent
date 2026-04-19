//! Per-session triggers for `run_review_cycle`.
//!
//! Fires when: cooldown elapsed AND (tokens_since_last >= threshold OR
//! messages_since_last >= threshold). Concurrency is bounded by a per-session
//! `AtomicBool` guard (an `AutoReviewGate`) so two tool loops in the same
//! session cannot spawn overlapping reviews.
//!
//! Uses `std::sync::Mutex` rather than `tokio::Mutex`: all critical sections
//! touch only in-memory HashMaps (no `.await` inside), and the `touch` +
//! `maybe_trigger` hot path runs on every chat turn, so avoiding an async
//! scheduler hop matters.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;

use super::config::SkillsAutoReviewConfig;

/// Per-session activity accounting since the last review fire.
#[derive(Default, Debug, Clone, Copy)]
struct TurnStats {
    tokens: usize,
    messages: usize,
    last_review_at: u64, // unix seconds; 0 if never run
}

/// Global registry of per-session turn stats and guards.
#[derive(Default)]
struct Registry {
    stats: HashMap<String, TurnStats>,
    guards: HashMap<String, Arc<AtomicBool>>,
}

static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(|| Mutex::new(Registry::default()));

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record turn activity and, if thresholds fire, acquire the per-session gate
/// in a single registry lock. Returns `Some(gate)` when the caller should run
/// `run_review_cycle` (gate drops on scope exit). Returns `None` when
/// disabled / cooldown / thresholds not met / another review already running.
///
/// Merging touch + trigger saves a second lock round-trip per chat turn.
pub fn touch_and_maybe_trigger(
    session_id: &str,
    turn_tokens: usize,
    new_messages: usize,
    cfg: &SkillsAutoReviewConfig,
) -> Option<AutoReviewGate> {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_secs();

    let (cooldown_elapsed, threshold_met);
    {
        let stats = reg.stats.entry(session_id.to_string()).or_default();
        stats.tokens = stats.tokens.saturating_add(turn_tokens);
        stats.messages = stats.messages.saturating_add(new_messages);
        cooldown_elapsed = stats.last_review_at == 0
            || now.saturating_sub(stats.last_review_at) >= cfg.cooldown_secs;
        threshold_met =
            stats.tokens >= cfg.token_threshold || stats.messages >= cfg.message_threshold;
    }
    if !(cfg.enabled && cooldown_elapsed && threshold_met) {
        return None;
    }

    let guard = reg
        .guards
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    if guard
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        // Another review is already running for this session; let it finish.
        return None;
    }

    if let Some(stats) = reg.stats.get_mut(session_id) {
        stats.tokens = 0;
        stats.messages = 0;
        stats.last_review_at = now;
    }

    Some(AutoReviewGate {
        session_id: session_id.to_string(),
        flag: guard,
    })
}

/// Manual trigger path — bypass thresholds but still honour the per-session
/// running guard. Used by the "Run review now" UI button.
pub fn acquire_manual(session_id: &str) -> Option<AutoReviewGate> {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let guard = reg
        .guards
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    if guard
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    if let Some(stats) = reg.stats.get_mut(session_id) {
        stats.last_review_at = now_secs();
    }
    Some(AutoReviewGate {
        session_id: session_id.to_string(),
        flag: guard,
    })
}

/// Drop stats + guards for sessions we haven't seen activity on in the last
/// `retention_secs` and whose accounting is empty. Keeps the registry bounded
/// under heavy IM/subagent multi-session workloads. Called periodically from
/// the same post-turn hook.
pub fn sweep_stale(retention_secs: u64) {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let cutoff = now_secs().saturating_sub(retention_secs);
    let stale: Vec<String> = reg
        .stats
        .iter()
        .filter(|(_, stats)| {
            stats.last_review_at < cutoff && stats.tokens == 0 && stats.messages == 0
        })
        .map(|(sid, _)| sid.clone())
        .collect();
    for sid in stale {
        reg.stats.remove(&sid);
        reg.guards.remove(&sid);
    }
}

/// RAII guard that clears the per-session running flag on drop.
pub struct AutoReviewGate {
    session_id: String,
    flag: Arc<AtomicBool>,
}

impl AutoReviewGate {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for AutoReviewGate {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

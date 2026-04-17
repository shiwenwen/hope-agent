//! Per-session triggers for `run_review_cycle`.
//!
//! Fires when: cooldown elapsed AND (tokens_since_last >= threshold OR
//! messages_since_last >= threshold). Concurrency is bounded by a per-session
//! `AtomicBool` guard (an `AutoReviewGate`) so two tool loops in the same
//! session cannot spawn overlapping reviews.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

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

/// Record activity for a session. Called from the agent's post-turn hook.
pub async fn touch_turn_stats(session_id: &str, turn_tokens: usize, new_messages: usize) {
    let mut reg = REGISTRY.lock().await;
    let entry = reg.stats.entry(session_id.to_string()).or_default();
    entry.tokens = entry.tokens.saturating_add(turn_tokens);
    entry.messages = entry.messages.saturating_add(new_messages);
}

/// Check thresholds and, if they fire, acquire the per-session gate. Returns
/// `Some(gate)` when the caller should run `run_review_cycle` (gate drops on
/// scope exit). Returns `None` when: disabled / cooldown / thresholds not met
/// / another review already running.
pub async fn maybe_trigger_post_turn(
    session_id: &str,
    cfg: &SkillsAutoReviewConfig,
) -> Option<AutoReviewGate> {
    if !cfg.enabled {
        return None;
    }
    let mut reg = REGISTRY.lock().await;
    let now = now_secs();

    // Phase 1: inspect thresholds under a short-lived stats borrow.
    let (cooldown_elapsed, threshold_met) = {
        let stats = reg.stats.entry(session_id.to_string()).or_default();
        let elapsed = stats.last_review_at == 0
            || now.saturating_sub(stats.last_review_at) >= cfg.cooldown_secs;
        let met = stats.tokens >= cfg.token_threshold
            || stats.messages >= cfg.message_threshold;
        (elapsed, met)
    };
    if !(cooldown_elapsed && threshold_met) {
        return None;
    }

    // Phase 2: try to acquire the per-session gate.
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

    // Phase 3: reset accounting + stamp last_review_at now so later fires wait cooldown.
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

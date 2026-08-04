//! Gate 1 of the five-gate waterfall: per-session triggers for
//! `run_review_cycle`.
//!
//! Fires when, after a chat turn:
//!   cooldown elapsed AND enabled AND any of:
//!     * `tool_use_count_since_last >= tool_use_threshold` (the dominant
//!       hard gate that keeps pure-chat turns from triggering review)
//!     * `token_threshold_met` AND at least one tool use this round (so a
//!       long text-only ramble doesn't fire) — modulated by
//!       `require_tool_use`
//!     * `correction_signal` raised by the caller (two user messages
//!       within 30s)
//!
//! Concurrency is bounded by a per-session `AtomicBool` guard so two tool
//! loops in the same session cannot spawn overlapping reviews.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;

use super::config::SkillsAutoReviewConfig;

/// Per-turn signals fed into the trigger. Aggregated by the chat engine
/// from this round's tool calls and message timestamps.
#[derive(Debug, Default, Clone, Copy)]
pub struct TriggerSignals {
    /// Sum of input+output tokens for the round.
    pub turn_tokens: usize,
    /// Number of new role-bearing messages appended this round.
    pub new_messages: usize,
    /// Number of tool_use blocks emitted this round.
    pub tool_use_count: usize,
    /// True when two user messages arrived within 30 seconds — a rough
    /// proxy for "user is correcting me / changing course".
    pub user_correction: bool,
}

/// Per-session activity accounting since the last review fire.
#[derive(Default, Debug, Clone, Copy)]
struct TurnStats {
    tokens: usize,
    messages: usize,
    tool_use: usize,
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

/// Record turn activity and, if thresholds fire, acquire the per-session
/// gate in a single registry lock. Returns `Some(gate)` when the caller
/// should run `run_review_cycle` (gate drops on scope exit). Returns
/// `None` when disabled / cooldown not elapsed / thresholds not met /
/// another review already running.
pub fn touch_and_maybe_trigger(
    session_id: &str,
    signals: TriggerSignals,
    cfg: &SkillsAutoReviewConfig,
) -> Option<AutoReviewGate> {
    if !cfg.enabled {
        return None;
    }

    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_secs();

    let (cooldown_elapsed, fire, fire_reason);
    {
        let stats = reg.stats.entry(session_id.to_string()).or_default();
        stats.tokens = stats.tokens.saturating_add(signals.turn_tokens);
        stats.messages = stats.messages.saturating_add(signals.new_messages);
        stats.tool_use = stats.tool_use.saturating_add(signals.tool_use_count);
        cooldown_elapsed = stats.last_review_at == 0
            || now.saturating_sub(stats.last_review_at) >= cfg.cooldown_secs;

        let tool_use_hit = cfg.tool_use_threshold > 0 && stats.tool_use >= cfg.tool_use_threshold;
        let token_hit = stats.tokens >= cfg.token_threshold;
        let message_hit = stats.messages >= cfg.message_threshold;
        let correction_hit = cfg.correction_signal_enabled && signals.user_correction;

        // If the user requires tool use, suppress token/message-only fires
        // (the correction signal still wins — it's an explicit user signal).
        let bulk_hit = if cfg.require_tool_use {
            (token_hit || message_hit) && stats.tool_use >= 1
        } else {
            token_hit || message_hit
        };

        fire = tool_use_hit || bulk_hit || correction_hit;
        fire_reason = if tool_use_hit {
            "tool_use"
        } else if correction_hit {
            "correction"
        } else if bulk_hit {
            "bulk"
        } else {
            "none"
        };
    }

    if !(cooldown_elapsed && fire) {
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
        stats.tool_use = 0;
        stats.last_review_at = now;
    }

    Some(AutoReviewGate {
        session_id: session_id.to_string(),
        fire_reason: fire_reason.to_string(),
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
        fire_reason: "manual".to_string(),
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
            stats.last_review_at < cutoff
                && stats.tokens == 0
                && stats.messages == 0
                && stats.tool_use == 0
        })
        .map(|(sid, _)| sid.clone())
        .collect();
    for sid in stale {
        reg.stats.remove(&sid);
        reg.guards.remove(&sid);
    }
}

/// RAII guard that clears the per-session running flag on drop. Records
/// the trigger reason for downstream `learning_events` logging.
pub struct AutoReviewGate {
    session_id: String,
    fire_reason: String,
    flag: Arc<AtomicBool>,
}

impl AutoReviewGate {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Short string explaining why this gate fired:
    /// `tool_use` | `bulk` | `correction` | `manual`.
    pub fn fire_reason(&self) -> &str {
        &self.fire_reason
    }
}

impl Drop for AutoReviewGate {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fresh_session() -> String {
        format!("test-{}", Uuid::new_v4())
    }

    fn cfg_default() -> SkillsAutoReviewConfig {
        SkillsAutoReviewConfig::default()
    }

    #[test]
    fn pure_chat_does_not_trigger_with_require_tool_use() {
        let sid = fresh_session();
        let cfg = cfg_default();
        // many messages and tokens but zero tool use
        let gate = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 100_000,
                new_messages: 50,
                tool_use_count: 0,
                user_correction: false,
            },
            &cfg,
        );
        assert!(gate.is_none(), "pure-chat must not trigger by default");
    }

    #[test]
    fn tool_use_threshold_fires() {
        let sid = fresh_session();
        let cfg = cfg_default();
        let gate = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 0,
                new_messages: 2,
                tool_use_count: 3,
                user_correction: false,
            },
            &cfg,
        );
        let g = gate.expect("tool_use_count >= threshold should fire");
        assert_eq!(g.fire_reason(), "tool_use");
    }

    #[test]
    fn correction_alone_fires() {
        let sid = fresh_session();
        let cfg = cfg_default();
        let gate = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 0,
                new_messages: 2,
                tool_use_count: 0,
                user_correction: true,
            },
            &cfg,
        );
        let g = gate.expect("correction should fire even without tool_use");
        assert_eq!(g.fire_reason(), "correction");
    }

    #[test]
    fn disabled_short_circuits() {
        let sid = fresh_session();
        let mut cfg = cfg_default();
        cfg.enabled = false;
        let gate = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 100_000,
                new_messages: 50,
                tool_use_count: 10,
                user_correction: true,
            },
            &cfg,
        );
        assert!(gate.is_none());
    }

    #[test]
    fn cooldown_blocks_back_to_back() {
        let sid = fresh_session();
        let cfg = cfg_default();
        let first = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 0,
                new_messages: 0,
                tool_use_count: 5,
                user_correction: false,
            },
            &cfg,
        );
        assert!(first.is_some());
        drop(first);
        let second = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: 0,
                new_messages: 0,
                tool_use_count: 5,
                user_correction: false,
            },
            &cfg,
        );
        assert!(second.is_none(), "cooldown should block immediate re-fire");
    }

    #[test]
    fn require_tool_use_off_lets_bulk_fire() {
        let sid = fresh_session();
        let mut cfg = cfg_default();
        cfg.require_tool_use = false;
        cfg.tool_use_threshold = 100; // make sure bulk path is what fires
        let gate = touch_and_maybe_trigger(
            &sid,
            TriggerSignals {
                turn_tokens: cfg.token_threshold + 1,
                new_messages: 0,
                tool_use_count: 0,
                user_correction: false,
            },
            &cfg,
        );
        let g = gate.expect("bulk path should fire when require_tool_use is off");
        assert_eq!(g.fire_reason(), "bulk");
    }
}

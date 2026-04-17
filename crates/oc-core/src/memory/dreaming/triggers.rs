//! Triggers — wire the Dreaming cycle into the app's background
//! machinery (Guardian heartbeat / Cron scheduler / manual API).

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use serde::{Deserialize, Serialize};

use super::config::DreamingConfig;
use super::pipeline;
use super::types::DreamReport;

/// Which trigger fired a given cycle. Serialised into `DreamReport` and
/// Dashboard filters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DreamTrigger {
    Idle,
    Cron,
    Manual,
}

impl DreamTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            DreamTrigger::Idle => "idle",
            DreamTrigger::Cron => "cron",
            DreamTrigger::Manual => "manual",
        }
    }
}

/// Global mutex-as-AtomicBool serialising cycles across all triggers.
/// We prefer a bool + CAS over a tokio `Mutex` so the Guardian path
/// (which is synchronous) can peek + bail without needing to `.await`.
static DREAMING_RUNNING: AtomicBool = AtomicBool::new(false);

/// Wall-clock timestamp of the last user-facing activity (epoch seconds).
/// Updated by `touch_activity()` from the chat path; consumed by
/// `check_idle_trigger()` from Guardian.
static LAST_ACTIVITY_EPOCH_SECS: AtomicI64 = AtomicI64::new(0);

/// Wall-clock timestamp of the last dreaming cycle. Used to prevent
/// multiple idle triggers firing in the same idle window when Guardian
/// heartbeats more often than expected.
static LAST_CYCLE_EPOCH_SECS: AtomicI64 = AtomicI64::new(0);

/// Read-only peek at whether a dreaming cycle is in progress. Callers
/// use this for UI state (Dashboard button disabled while running).
pub fn dreaming_running() -> bool {
    DREAMING_RUNNING.load(Ordering::Acquire)
}

/// Try to claim the global running flag. Returns a guard that resets
/// the flag on drop.
pub(super) fn try_claim() -> Option<RunningGuard> {
    DREAMING_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| RunningGuard)
}

pub(super) struct RunningGuard;
impl Drop for RunningGuard {
    fn drop(&mut self) {
        DREAMING_RUNNING.store(false, Ordering::Release);
        LAST_CYCLE_EPOCH_SECS.store(now_secs(), Ordering::Release);
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Record that the user just sent a message / interacted with the app.
/// Called from the chat entry points so `check_idle_trigger` has a
/// consistent "last activity" timestamp.
pub fn touch_activity() {
    LAST_ACTIVITY_EPOCH_SECS.store(now_secs(), Ordering::Release);
}

/// Guardian heartbeat hook. Returns `true` when the app has been idle
/// long enough to fire a dreaming cycle AND a cycle isn't already
/// running AND we haven't just finished one.
///
/// Caller (Guardian) then kicks off `manual_run(DreamTrigger::Idle)` in
/// a detached task.
pub fn check_idle_trigger(cfg: &DreamingConfig) -> bool {
    if !cfg.enabled || !cfg.idle_trigger.enabled {
        return false;
    }
    if dreaming_running() {
        return false;
    }
    let now = now_secs();
    let last_activity = LAST_ACTIVITY_EPOCH_SECS.load(Ordering::Acquire);
    if last_activity == 0 {
        // Never saw activity — avoid surprise cycles right after boot.
        return false;
    }
    let idle_secs = (cfg.idle_trigger.idle_minutes.max(1) as i64) * 60;
    if now - last_activity < idle_secs {
        return false;
    }
    // Debounce: don't fire again within the same idle window. The
    // cycle-end timestamp is set when the RunningGuard drops.
    let last_cycle = LAST_CYCLE_EPOCH_SECS.load(Ordering::Acquire);
    if last_cycle != 0 && now - last_cycle < idle_secs {
        return false;
    }
    true
}

/// Explicit trigger — called from the manual HTTP/Tauri command, from
/// the Cron job callback, and from Guardian once `check_idle_trigger`
/// returns true. Runs the cycle inline (`await`) and returns the report.
pub async fn manual_run(trigger: DreamTrigger) -> DreamReport {
    pipeline::run_cycle(trigger).await
}

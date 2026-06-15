//! Agent self-scheduled wakeups (R10 — aligns with Claude Code's
//! `ScheduleWakeup`).
//!
//! Lets an agent ask to be woken back into **the current session** after a
//! delay — to poll an external state the harness can't track (a CI run, a
//! remote queue), or to re-check something later. The agent calls
//! `schedule_wakeup(delay_secs, note)`, ends its turn, and at `fire_at` a
//! `<wakeup>` message is injected back through the **shared injection pipeline**
//! (`subagent::injection::inject_and_run_parent`) — so it inherits R2's
//! idle-gating (waits for a live turn to finish), cancellation, and retry, and
//! runs a fresh parent turn carrying the note.
//!
//! This is deliberately NOT cron: cron is user-configured, periodic, and may
//! target a separate session with delivery fan-out; a wakeup is agent-initiated,
//! one-shot, and continues the originating session's context. The two don't
//! share an entry point.
//!
//! ## Lifecycle & cross-process model
//! - **Creation** persists a row (unless incognito) and arms a process-local
//!   timer in the *creating* process.
//! - **Restart recovery** (`replay_pending`) is **Primary-only** (mirrors
//!   `async_jobs::replay_pending_jobs`): it re-arms unfired rows; past-due ones
//!   fire immediately. Secondary processes don't re-arm shared rows.
//! - **Delivery** marks the row fired only when the injection actually lands
//!   (via the `on_injected` callback). An abandoned injection (parent never went
//!   idle within the announce window) leaves the row unfired for the next
//!   replay. Cross-process double-delivery is best-effort-deduped the same way
//!   the async-job injection pipeline is (per-process in-flight set).
//! - **Incognito** wakeups are in-memory only (no row) — close-and-burn.

pub(crate) mod db;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

pub use db::{Wakeup, WakeupDB};

/// Lower bound on the wakeup delay (seconds). Guards against busy-polling — an
/// agent that wants "almost immediately" should just keep working this turn.
pub const MIN_DELAY_SECS: i64 = 10;
/// Upper bound on the wakeup delay (seconds, 24h). Guards against zombie timers
/// pinning a session indefinitely; longer cadences belong to cron.
pub const MAX_DELAY_SECS: i64 = 86_400;
/// Per-session cap on pending wakeups (structural reject — exceeding errors,
/// it does NOT queue). Guards against an agent self-scheduling a flood of
/// billed turns.
pub const MAX_PENDING_PER_SESSION: usize = 5;

static WAKEUP_DB: OnceLock<Arc<WakeupDB>> = OnceLock::new();

/// Set the global wakeup DB. Called once during app initialization.
pub fn set_wakeup_db(db: Arc<WakeupDB>) {
    let _ = WAKEUP_DB.set(db);
}

/// Get the global wakeup DB (None until init / if it failed to open).
pub fn get_wakeup_db() -> Option<&'static Arc<WakeupDB>> {
    WAKEUP_DB.get()
}

struct ArmedTimer {
    session_id: String,
    abort: tokio::task::AbortHandle,
}

/// Live process-local timers, keyed by wakeup id. Used to count per-session
/// pending wakeups (the cap source of truth, covering both persisted and
/// incognito) and to cancel timers on session delete / burn.
static ARMED_TIMERS: LazyLock<Mutex<HashMap<String, ArmedTimer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-process in-flight delivery set, mirroring `async_jobs::injection`'s
/// dedup so a replay + a late timer can't double-inject the same wakeup.
static DELIVERING: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn count_pending_for_session(session_id: &str) -> usize {
    ARMED_TIMERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|t| t.session_id == session_id)
        .count()
}

/// Outcome of a successful schedule call (returned to the tool layer).
#[derive(Debug, Clone)]
pub struct ScheduleOutcome {
    pub id: String,
    pub fire_at: i64,
    pub delay_secs: i64,
}

/// Why a schedule request was rejected (structural — never queued).
#[derive(Debug)]
pub enum ScheduleError {
    /// The per-session pending cap (`MAX_PENDING_PER_SESSION`) is reached.
    TooManyPending { limit: usize },
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::TooManyPending { limit } => write!(
                f,
                "too many pending wakeups for this session (limit {limit}); \
                 wait for one to fire or cancel before scheduling another"
            ),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Schedule a one-shot wakeup for `session_id`. `delay_secs` is clamped to
/// `[MIN_DELAY_SECS, MAX_DELAY_SECS]`. Persists a row unless `incognito`, then
/// arms a process-local timer. Returns `Err` if the per-session pending cap is
/// hit (structural reject).
pub fn schedule(
    session_id: &str,
    agent_id: &str,
    delay_secs: i64,
    note: Option<String>,
    incognito: bool,
) -> Result<ScheduleOutcome, ScheduleError> {
    if count_pending_for_session(session_id) >= MAX_PENDING_PER_SESSION {
        return Err(ScheduleError::TooManyPending {
            limit: MAX_PENDING_PER_SESSION,
        });
    }

    let delay = delay_secs.clamp(MIN_DELAY_SECS, MAX_DELAY_SECS);
    let now = now_secs();
    let fire_at = now.saturating_add(delay);
    let id = format!("wakeup_{}", uuid::Uuid::new_v4().simple());

    if !incognito {
        // Best-effort persistence: if the DB is missing we still arm the live
        // timer so the wakeup works this session — it just won't survive a
        // restart (degrades to incognito-like behavior).
        if let Some(db) = get_wakeup_db() {
            let row = Wakeup {
                id: id.clone(),
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                note: note.clone(),
                fire_at,
                created_at: now,
            };
            if let Err(e) = db.insert(&row) {
                app_warn!(
                    "wakeup",
                    "schedule",
                    "Failed to persist wakeup {} (arming in-memory only): {}",
                    id,
                    e
                );
            }
        }
    }

    arm_timer(
        id.clone(),
        session_id.to_string(),
        agent_id.to_string(),
        note,
        fire_at,
        incognito,
    );

    app_info!(
        "wakeup",
        "schedule",
        "Scheduled wakeup {} for session {} in {}s (incognito={})",
        id,
        session_id,
        delay,
        incognito
    );

    Ok(ScheduleOutcome {
        id,
        fire_at,
        delay_secs: delay,
    })
}

/// Spawn the live timer task and register it in `ARMED_TIMERS`. The map lock is
/// held across `tokio::spawn` so a delay==0 task (past-due replay) can't remove
/// itself before this insert lands (its `remove_armed` blocks on the same lock).
fn arm_timer(
    id: String,
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    incognito: bool,
) {
    let delay = (fire_at - now_secs()).max(0) as u64;
    // Clone the session id for the map entry before the rest moves into the task.
    let session_for_map = session_id.clone();
    let task_id = id.clone();
    // Hold the map lock across spawn so a delay==0 task can't `remove_armed`
    // before we insert (its removal blocks on this same lock).
    let mut map = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let handle = tokio::spawn(async move {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        remove_armed(&task_id);
        fire(task_id, session_id, agent_id, note, incognito);
    });
    map.insert(
        id,
        ArmedTimer {
            session_id: session_for_map,
            abort: handle.abort_handle(),
        },
    );
}

fn remove_armed(id: &str) {
    ARMED_TIMERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id);
}

/// Deliver a fired wakeup: inject a `<wakeup>` message back into the session via
/// the shared injection pipeline. Runs on a detached thread (its own runtime),
/// exactly like `async_jobs::injection::dispatch_injection`.
fn fire(id: String, session_id: String, agent_id: String, note: Option<String>, incognito: bool) {
    // Per-process in-flight dedup.
    {
        let mut g = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
        if !g.insert(id.clone()) {
            return;
        }
    }

    let session_db = match crate::get_session_db() {
        Some(db) => db.clone(),
        None => {
            release_delivering(&id);
            return;
        }
    };

    let push_message = build_wakeup_message(note.as_deref());
    let id_for_mark = id.clone();
    let id_for_release = id.clone();

    std::thread::spawn(move || {
        struct DeliverGuard(String);
        impl Drop for DeliverGuard {
            fn drop(&mut self) {
                release_delivering(&self.0);
            }
        }
        let _guard = DeliverGuard(id_for_release);

        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let on_injected: crate::subagent::injection::OnInjected = {
                    let jid = id_for_mark.clone();
                    let persisted = !incognito;
                    Arc::new(move || {
                        if persisted {
                            if let Some(db) = get_wakeup_db() {
                                let _ = db.mark_fired(&jid);
                            }
                        }
                    })
                };
                let outcome = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    session_id,
                    agent_id,
                    "wakeup".to_string(),
                    id,
                    push_message,
                    session_db,
                    Some(on_injected),
                ));
                if matches!(
                    outcome,
                    crate::subagent::injection::InjectionOutcome::Abandoned
                ) {
                    app_warn!(
                        "wakeup",
                        "fire",
                        "Wakeup {} abandoned (parent never went idle); left for restart replay",
                        id_for_mark
                    );
                }
            }
            Err(e) => app_error!("wakeup", "fire", "Failed to build runtime: {}", e),
        }
    });
}

fn release_delivering(id: &str) {
    DELIVERING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id);
}

/// Build the injected `<wakeup>` user message carrying the agent's own note.
pub(crate) fn build_wakeup_message(note: Option<&str>) -> String {
    let note_block = note
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("<note>\n{}\n</note>\n", escape_xml(s)))
        .unwrap_or_default();
    format!(
        "<wakeup>\n\
         A wakeup you scheduled earlier has fired. Continue the work you set this \
         timer for. Your note to self:\n\
         {note_block}\
         </wakeup>"
    )
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Re-arm unfired wakeups after a restart. **Primary-only** — call sites are
/// gated by `runtime_lock::is_primary()`. Past-due wakeups fire promptly.
pub fn replay_pending() {
    let Some(db) = get_wakeup_db() else {
        return;
    };
    let pending = match db.list_pending() {
        Ok(p) => p,
        Err(e) => {
            app_error!("wakeup", "replay", "Failed to list pending wakeups: {}", e);
            return;
        }
    };
    let count = pending.len();
    for w in pending {
        arm_timer(w.id, w.session_id, w.agent_id, w.note, w.fire_at, false);
    }
    if count > 0 {
        app_info!("wakeup", "replay", "Re-armed {} pending wakeup(s)", count);
    }
}

/// Cancel & delete all wakeups for a session (session delete / incognito burn).
pub fn purge_for_session(session_id: &str) {
    // Abort live timers for this session.
    let aborted: Vec<String> = {
        let mut map = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, t)| t.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            if let Some(t) = map.remove(id) {
                t.abort.abort();
            }
        }
        ids
    };
    if let Some(db) = get_wakeup_db() {
        if let Err(e) = db.delete_for_session(session_id) {
            app_warn!(
                "wakeup",
                "purge",
                "Failed to delete wakeups for session {}: {}",
                session_id,
                e
            );
        }
    }
    if !aborted.is_empty() {
        app_info!(
            "wakeup",
            "purge",
            "Cancelled {} wakeup(s) for session {}",
            aborted.len(),
            session_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_wakeup_message_escapes_and_wraps_note() {
        let msg = build_wakeup_message(Some("check <CI> & retry"));
        assert!(msg.starts_with("<wakeup>"));
        assert!(msg.contains("<note>\ncheck &lt;CI&gt; &amp; retry\n</note>"));
        assert!(msg.trim_end().ends_with("</wakeup>"));
    }

    #[test]
    fn build_wakeup_message_handles_empty_note() {
        let msg = build_wakeup_message(None);
        assert!(msg.starts_with("<wakeup>"));
        assert!(!msg.contains("<note>"));
    }

    #[tokio::test]
    async fn schedule_clamps_delay_and_enforces_per_session_cap() {
        // Unique session id isolates the global ARMED_TIMERS count from other
        // parallel tests. Incognito → no DB needed (in-memory timers only).
        let sid = "test-wakeup-cap-session";
        purge_for_session(sid); // ensure clean slate

        // Sub-minimum delay is clamped up to MIN_DELAY_SECS (not busy-polled).
        let out = schedule(sid, "ha-main", 2, Some("note".into()), true).unwrap();
        assert_eq!(out.delay_secs, MIN_DELAY_SECS);

        // Fill to the cap (we already armed 1).
        for _ in 1..MAX_PENDING_PER_SESSION {
            schedule(sid, "ha-main", 60, None, true).unwrap();
        }
        assert_eq!(count_pending_for_session(sid), MAX_PENDING_PER_SESSION);

        // One past the cap is a structural reject (not queued).
        let err = schedule(sid, "ha-main", 60, None, true).unwrap_err();
        assert!(matches!(err, ScheduleError::TooManyPending { .. }));

        // Purge aborts every live timer and frees the session's budget.
        purge_for_session(sid);
        assert_eq!(count_pending_for_session(sid), 0);
        // Scheduling works again after purge.
        schedule(sid, "ha-main", 60, None, true).unwrap();
        assert_eq!(count_pending_for_session(sid), 1);
        purge_for_session(sid); // leave no lingering timers for sibling tests
    }

    #[tokio::test]
    async fn schedule_clamps_oversized_delay_to_max() {
        let sid = "test-wakeup-maxclamp-session";
        purge_for_session(sid);
        let out = schedule(sid, "ha-main", MAX_DELAY_SECS + 10_000, None, true).unwrap();
        assert_eq!(out.delay_secs, MAX_DELAY_SECS);
        purge_for_session(sid);
    }
}

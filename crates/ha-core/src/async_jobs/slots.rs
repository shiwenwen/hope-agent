//! Concurrency scheduler for backgrounded tool jobs (MISC-5 + R7.1 queue).
//!
//! When the configured cap (`async_tools.max_concurrent_jobs`) is full, a new
//! background job no longer hard-rejects — it QUEUES (status `Queued`) and a
//! **per-process** scheduler task ([`run_scheduler`], tier-agnostic — each
//! process drains only its OWN in-memory queue) promotes queued jobs per-session
//! round-robin as slots free. The queue holds each job's live
//! [`ToolExecContext`] in memory (it cannot be persisted), so queued jobs do
//! NOT survive a restart — they are recovered as `Interrupted` like running
//! jobs (see `replay_pending_jobs`).
//!
//! Accounting model: every *running* job holds exactly one [`SlotReservation`],
//! granted by [`try_reserve`] (immediate path), [`try_take_next`] (promote
//! path), or [`reserve_forced`] (an already-running auto-backgrounded job that
//! detached and must be counted retroactively). Its `Drop` decrements the counts
//! and wakes the scheduler, so a freed slot immediately pulls the next queued
//! job. `max_concurrent_jobs == 0` removes the *global* cap; the per-session cap
//! below still applies (a session past it still queues). Both caps `== 0` ⇒ truly
//! unlimited (no cap, no queueing).
//!
//! Fairness has two tiers (R7.1):
//! - **Hard per-session cap** (`max_concurrent_jobs_per_session`, 0 = unlimited):
//!   a session may hold at most this many concurrent slots. A reservation for a
//!   session already at its cap is refused even when the global pool has room, so
//!   its extra jobs QUEUE — one busy session (or IM chat) can't fill every global
//!   slot and starve the others. Auto-backgrounded jobs are *counted* against
//!   both caps (so they push subsequent reservations toward queueing) but, being
//!   already-running threads, are never *refused* — a burst of auto-detaches can
//!   transiently exceed either cap (see [`reserve_forced`]).
//! - **Round-robin promotion** ([`pick_fair_index`]): among queued jobs whose
//!   session is still BELOW its per-session cap, promote the one whose session
//!   currently has the FEWEST running jobs (ties → oldest). A session sitting at
//!   its cap is skipped until one of its slots frees.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tools::ToolExecContext;

/// Safe band for the configurable wait-queue ceiling (R9). Each queued job pins
/// its `ToolExecContext` in RAM, so the queue MUST stay bounded — the configured
/// `async_tools.max_queued_jobs` is clamped here so a `0`/huge value can neither
/// disable the bound nor blow up memory. Default lives in config (256).
const MAX_QUEUED_JOBS_FLOOR: usize = 1;
const MAX_QUEUED_JOBS_CEILING: usize = 4096;

/// Everything needed to start a backgrounded job later, parked in the queue
/// while it waits for a slot. Holds the live ctx (not persistable), so it lives
/// only in the Primary process's memory.
pub struct PreparedJob {
    pub job_id: String,
    pub tool_name: String,
    pub args: Value,
    pub ctx: ToolExecContext,
    pub max_secs: u64,
    pub preview_bytes: usize,
    pub cancel_token: CancellationToken,
}

impl PreparedJob {
    fn session_key(&self) -> String {
        self.ctx.session_id.clone().unwrap_or_default()
    }
}

/// Per-session round-robin: index of the queued job whose session currently has
/// the fewest running jobs; ties broken by queue position (oldest first).
/// Sessions already at the per-session cap (`per_session_cap`, 0 = unlimited)
/// are SKIPPED — promoting them would breach the cap, so they wait for one of
/// their own slots to free. Pure (no globals) so the fairness rule is
/// unit-testable. `None` ⇒ nothing eligible (empty, or every queued job's
/// session is at its cap).
fn pick_fair_index<'a>(
    queue_sessions: impl Iterator<Item = &'a str>,
    per_session: &HashMap<String, usize>,
    per_session_cap: usize,
) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None; // (index, running_count)
    for (i, sess) in queue_sessions.enumerate() {
        let running = *per_session.get(sess).unwrap_or(&0);
        if per_session_cap != 0 && running >= per_session_cap {
            continue; // session at its per-session cap — not eligible yet
        }
        match best {
            Some((_, best_running)) if running >= best_running => {}
            _ => best = Some((i, running)),
        }
    }
    best.map(|(i, _)| i)
}

struct SlotManager {
    total: usize,
    per_session: HashMap<String, usize>,
    queue: VecDeque<PreparedJob>,
}

impl SlotManager {
    fn new() -> Self {
        Self {
            total: 0,
            per_session: HashMap::new(),
            queue: VecDeque::new(),
        }
    }

    /// True iff `session_key` is already at the per-session cap (`0` ⇒ no limit).
    fn session_at_cap(&self, per_session_cap: usize, session_key: &str) -> bool {
        per_session_cap != 0 && *self.per_session.get(session_key).unwrap_or(&0) >= per_session_cap
    }

    /// Reserve a slot if there is room — both the global cap (`cap == 0` ⇒
    /// unlimited) AND the per-session cap (`per_session_cap == 0` ⇒ unlimited)
    /// must have headroom. Increments the counts and returns true; false ⇒ caller
    /// should enqueue.
    fn reserve_inner(&mut self, cap: usize, per_session_cap: usize, session_key: &str) -> bool {
        if cap != 0 && self.total >= cap {
            return false;
        }
        if self.session_at_cap(per_session_cap, session_key) {
            return false;
        }
        self.reserve_forced_inner(session_key);
        true
    }

    /// Increment the counts unconditionally (no cap gate). Used by
    /// [`reserve_forced`] for an auto-backgrounded job that already detached and
    /// is running — it can't be queued or refused, only accounted for so fresh
    /// reservations see the pool as fuller. May briefly push `total` AND the
    /// session's count past their respective caps (a live thread can't be undone).
    fn reserve_forced_inner(&mut self, session_key: &str) {
        self.total += 1;
        *self.per_session.entry(session_key.to_string()).or_insert(0) += 1;
    }

    /// Pick + remove the next queued job if a slot is free, incrementing counts.
    /// Honors both the global cap and the per-session cap (skips sessions at cap).
    fn take_next_inner(&mut self, cap: usize, per_session_cap: usize) -> Option<PreparedJob> {
        if cap != 0 && self.total >= cap {
            return None;
        }
        let idx = pick_fair_index(
            self.queue.iter().map(|j| j.session_key_ref()),
            &self.per_session,
            per_session_cap,
        )?;
        let job = self.queue.remove(idx)?;
        self.reserve_forced_inner(&job.session_key());
        Some(job)
    }

    /// Release a running slot.
    fn release(&mut self, session_key: &str) {
        self.total = self.total.saturating_sub(1);
        if let Some(c) = self.per_session.get_mut(session_key) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.per_session.remove(session_key);
            }
        }
    }
}

impl PreparedJob {
    /// Borrowed session key for fairness scanning without allocation.
    fn session_key_ref(&self) -> &str {
        self.ctx.session_id.as_deref().unwrap_or("")
    }
}

static MANAGER: LazyLock<Mutex<SlotManager>> = LazyLock::new(|| Mutex::new(SlotManager::new()));
static SCHED_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

fn cap() -> usize {
    crate::config::cached_config()
        .async_tools
        .max_concurrent_jobs
}

fn per_session_cap() -> usize {
    crate::config::cached_config()
        .async_tools
        .max_concurrent_jobs_per_session
}

/// Clamp a configured wait-queue ceiling to the safe band. Pure (testable
/// without touching the global config cache).
fn clamp_queued(raw: usize) -> usize {
    raw.clamp(MAX_QUEUED_JOBS_FLOOR, MAX_QUEUED_JOBS_CEILING)
}

/// Configurable wait-queue ceiling (R9), clamped to the safe band. `0` does NOT
/// mean "unlimited" here (the queue pins live ctxs in RAM) — it clamps up to the
/// floor.
fn max_queued() -> usize {
    clamp_queued(crate::config::cached_config().async_tools.max_queued_jobs)
}

fn lock() -> std::sync::MutexGuard<'static, SlotManager> {
    MANAGER.lock().unwrap_or_else(|p| p.into_inner())
}

/// RAII reservation for one running slot. `Drop` decrements the counts and wakes
/// the scheduler, so a freed slot immediately promotes the next queued job. The
/// runner thread holds this for the job's whole lifetime (panic-safe release).
#[must_use = "dropping the reservation releases the slot immediately"]
pub struct SlotReservation {
    session_key: String,
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        lock().release(&self.session_key);
        SCHED_NOTIFY.notify_one();
    }
}

/// Try to reserve a slot for immediate execution. `None` ⇒ no headroom (global
/// or per-session cap full, or jobs already waiting) and the caller should
/// [`enqueue`] instead. Both caps `== 0` ⇒ unlimited.
pub fn try_reserve(session_key: &str) -> Option<SlotReservation> {
    let mut m = lock();
    // FIFO fairness: if jobs are already waiting, a fresh spawn must queue behind
    // them rather than jump the line — even when a slot is technically free. The
    // scheduler drains the queue (per-session round-robin, skipping sessions at
    // their per-session cap) into freed slots, so a flooding session's backlog
    // can't starve others on the dequeue side.
    if !m.queue.is_empty() {
        return None;
    }
    if m.reserve_inner(cap(), per_session_cap(), session_key) {
        Some(SlotReservation {
            session_key: session_key.to_string(),
        })
    } else {
        None
    }
}

/// Reserve a slot for an auto-backgrounded job that already detached and is
/// running on its own thread. The job can't be queued or refused (it's live), so
/// this counts it unconditionally — even if that briefly pushes the pool past the
/// global OR the per-session cap — so subsequent [`try_reserve`] calls see the
/// slot as occupied. Held by the runner thread; `Drop` releases it when the job
/// ends. (Consequence: the per-session cap bounds *admission* of new background
/// jobs, not the count of already-running auto-detached ones.)
pub fn reserve_forced(session_key: &str) -> SlotReservation {
    lock().reserve_forced_inner(session_key);
    SlotReservation {
        session_key: session_key.to_string(),
    }
}

/// Park a job in the wait queue. `false` ⇒ the queue itself is full (the job is
/// dropped here; the caller hard-rejects and cleans up its row/token by id).
pub fn enqueue(job: PreparedJob) -> bool {
    let mut m = lock();
    if m.queue.len() >= max_queued() {
        return false;
    }
    m.queue.push_back(job);
    true
}

/// Scheduler step: if a slot is free and the queue is non-empty, pick the next
/// job per-session round-robin, reserve its slot, and return it for the
/// scheduler to start. `None` ⇒ nothing to promote right now.
pub fn try_take_next() -> Option<(PreparedJob, SlotReservation)> {
    let mut m = lock();
    let job = m.take_next_inner(cap(), per_session_cap())?;
    let session_key = job.session_key();
    Some((job, SlotReservation { session_key }))
}

/// Remove a still-queued job by id (used by cancel). `Some` ⇒ it was waiting in
/// the queue and is now removed (no slot was held); `None` ⇒ not queued (already
/// running, already taken by the scheduler, or never queued).
pub fn remove_queued(job_id: &str) -> Option<PreparedJob> {
    let mut m = lock();
    let pos = m.queue.iter().position(|j| j.job_id == job_id)?;
    m.queue.remove(pos)
}

/// Remove ALL still-queued jobs for a session (incognito burn / session purge):
/// drops their in-memory `PreparedJob`s — and the live ctx those pin (which may
/// carry the incognito session's sensitive args) — not just the DB rows. Returns
/// how many were removed. A dropped queued job is never promoted (its row is also
/// being deleted by the caller).
pub fn remove_queued_for_session(session_id: &str) -> usize {
    let mut m = lock();
    let before = m.queue.len();
    m.queue.retain(|j| j.session_key_ref() != session_id);
    before - m.queue.len()
}

/// Wait for the next "slot freed / job enqueued" wakeup (the scheduler task
/// parks here). `Notify` stores one permit, so a wake that arrives between
/// drains is not lost.
pub async fn scheduler_notified() {
    SCHED_NOTIFY.notified().await;
}

/// Wake the scheduler — call after enqueueing so a currently-free slot promotes
/// the new job even though nothing finished.
pub fn wake_scheduler() {
    SCHED_NOTIFY.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_fair_index_prefers_session_with_fewest_running() {
        // queue sessions: [A, A, B]; A has 2 running, B has 0 → pick B (idx 2).
        let mut per = HashMap::new();
        per.insert("A".to_string(), 2usize);
        per.insert("B".to_string(), 0usize);
        let q = ["A", "A", "B"];
        assert_eq!(pick_fair_index(q.iter().copied(), &per, 0), Some(2));
    }

    #[test]
    fn pick_fair_index_breaks_ties_by_oldest() {
        // No one running → all tie at 0 → oldest (idx 0) wins.
        let per = HashMap::new();
        let q = ["A", "B", "C"];
        assert_eq!(pick_fair_index(q.iter().copied(), &per, 0), Some(0));
    }

    #[test]
    fn pick_fair_index_empty_is_none() {
        let per = HashMap::new();
        let q: [&str; 0] = [];
        assert_eq!(pick_fair_index(q.iter().copied(), &per, 0), None);
    }

    #[test]
    fn clamp_queued_keeps_the_bound_safe() {
        // R9: the configured queue ceiling is clamped to a safe band — `0` is NOT
        // "unlimited" (the queue pins live ctxs in RAM), and an absurd value is
        // capped to protect memory.
        assert_eq!(clamp_queued(0), MAX_QUEUED_JOBS_FLOOR);
        assert_eq!(clamp_queued(256), 256);
        assert_eq!(clamp_queued(usize::MAX), MAX_QUEUED_JOBS_CEILING);
    }

    #[test]
    fn pick_fair_index_skips_session_at_per_session_cap() {
        // A is at cap (2), B is below (1). Even though A's job is older and A is
        // "fewest" only if not capped, A must be skipped and B's job (idx 2) wins.
        let mut per = HashMap::new();
        per.insert("A".to_string(), 2usize);
        per.insert("B".to_string(), 1usize);
        let q = ["A", "A", "B"];
        assert_eq!(pick_fair_index(q.iter().copied(), &per, 2), Some(2));
    }

    #[test]
    fn pick_fair_index_all_capped_is_none() {
        // Every queued job's session is at the per-session cap → nothing eligible.
        let mut per = HashMap::new();
        per.insert("A".to_string(), 3usize);
        per.insert("B".to_string(), 3usize);
        let q = ["A", "B", "A"];
        assert_eq!(pick_fair_index(q.iter().copied(), &per, 3), None);
    }

    #[test]
    fn reserve_respects_cap_and_release_frees() {
        let mut m = SlotManager::new();
        assert!(m.reserve_inner(2, 0, "a"));
        assert!(m.reserve_inner(2, 0, "b"));
        assert!(!m.reserve_inner(2, 0, "c"), "cap 2 is full");
        m.release("a");
        assert!(m.reserve_inner(2, 0, "c"), "slot freed → room again");
        assert_eq!(m.total, 2);
    }

    #[test]
    fn reserve_cap_zero_is_unlimited() {
        let mut m = SlotManager::new();
        for i in 0..100 {
            assert!(
                m.reserve_inner(0, 0, &format!("s{i}")),
                "cap 0 never blocks"
            );
        }
        assert_eq!(m.total, 100);
    }

    #[test]
    fn reserve_respects_per_session_cap_with_global_headroom() {
        // Global cap 10 (plenty), per-session cap 2: one session can take only 2
        // even though the global pool is nearly empty — its 3rd must queue.
        let mut m = SlotManager::new();
        assert!(m.reserve_inner(10, 2, "a"));
        assert!(m.reserve_inner(10, 2, "a"));
        assert!(
            !m.reserve_inner(10, 2, "a"),
            "session at per-session cap 2 is refused despite global headroom"
        );
        // A different session still has room (fairness).
        assert!(m.reserve_inner(10, 2, "b"), "other session unaffected");
        m.release("a");
        assert!(
            m.reserve_inner(10, 2, "a"),
            "freed a slot → session a can run again"
        );
        assert_eq!(m.total, 3);
    }

    #[test]
    fn reserve_forced_bypasses_both_caps() {
        // An auto-backgrounded detach is already running; it must always count,
        // even past the global and per-session caps.
        let mut m = SlotManager::new();
        assert!(m.reserve_inner(1, 1, "a"));
        assert!(!m.reserve_inner(1, 1, "a"), "caps are full");
        m.reserve_forced_inner("a"); // detach: counts anyway
        assert_eq!(m.total, 2, "forced reservation pushed total past cap 1");
        assert_eq!(*m.per_session.get("a").unwrap(), 2);
    }

    #[test]
    fn take_next_inner_skips_capped_session() {
        // Two queued jobs: session "a" (at per-session cap 1) and "b" (idle).
        // take_next must promote b, not a.
        let mut m = SlotManager::new();
        m.reserve_forced_inner("a"); // a now has 1 running == cap
        let mk = |sess: &str| PreparedJob {
            job_id: format!("j-{sess}"),
            tool_name: "exec".into(),
            args: serde_json::json!({}),
            ctx: ToolExecContext {
                session_id: Some(sess.to_string()),
                ..Default::default()
            },
            max_secs: 0,
            preview_bytes: 0,
            cancel_token: CancellationToken::new(),
        };
        m.queue.push_back(mk("a"));
        m.queue.push_back(mk("b"));
        let promoted = m.take_next_inner(10, 1).expect("b is eligible");
        assert_eq!(promoted.job_id, "j-b");
        // a stays queued (still at cap); nothing else eligible now.
        assert!(m.take_next_inner(10, 1).is_none(), "a still capped → none");
    }

    #[test]
    fn release_unknown_session_is_saturating_noop() {
        let mut m = SlotManager::new();
        m.release("never-reserved"); // must not underflow
        assert_eq!(m.total, 0);
    }
}

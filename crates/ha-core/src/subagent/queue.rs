//! R7.2 reject→queue unification for background sub-agents.
//!
//! When a parent session is at its per-session sub-agent concurrency limit
//! (`max_concurrent_for_agent`, default 8), a new spawn no longer hard-rejects —
//! it PARKS here (the run row carries [`SubagentStatus::Queued`]) and a
//! per-process scheduler ([`run_subagent_scheduler`]) promotes parked spawns as
//! running children settle. This mirrors R7.1's tool-job queue
//! (`async_jobs::slots`), but is a focused, subagent-specific queue rather than
//! generifying the tool-job path: the sub-agent limit is per-parent-session (no
//! global pool) and runs launch via `tokio::spawn` running `run_chat_engine`.
//!
//! **Structural limits (depth, batch size, agent/session/capability) still
//! hard-reject** — a structural breach can't become legal by waiting; only the
//! resource limit (over-concurrency) queues.
//!
//! ## Lifecycle
//! - **Park**: [`enqueue`] holds the live [`SpawnParams`] (incl. attachments) in
//!   RAM, like the tool-job queue pins `ToolExecContext`. Bounded by
//!   [`MAX_QUEUED_SUBAGENTS`]; a full queue makes the caller hard-reject. The
//!   cancel flag is registered HERE (at park) and REUSED on promotion, so a
//!   cancel arriving in the park→launch window stays visible to the engine.
//! - **Promote**: the scheduler wakes on [`wake_subagent_scheduler`] (fired from
//!   the terminal status choke point — a slot may have freed) or a 5s fallback
//!   tick, and for each session under its limit promotes the oldest queued spawn
//!   via a guarded `Queued → Spawning` CAS, then
//!   [`super::spawn::launch_subagent_run`].
//! - **Cancel**: [`super::request_cancel_run`] claims the parked entry via
//!   [`remove_for_run`] — winning the queue mutex makes it authoritative, so it
//!   stamps `Killed`. If the scheduler already dequeued it, the reused cancel
//!   flag is tripped so the launched engine aborts. Together with the promote's
//!   guarded CAS (a terminal row can't transition to `Spawning`), a cancelled
//!   run can never be resurrected into a running child — the subagent analogue of
//!   R7.1's atomic dequeue-claim.
//! - **Session delete / incognito burn**: [`purge_for_session`] drops every
//!   parked entry for the session (the entry is the only place an incognito
//!   spawn's sensitive `SpawnParams` live — dropping it IS the burn).
//! - **Restart**: the in-memory queue is lost; `Queued` rows are swept to
//!   Orphaned by `cleanup_orphan_subagent_runs` (mirrors tool-job
//!   Queued→Interrupted).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use super::types::{SpawnParams, SubagentStatus};

/// Hard cap on the in-memory sub-agent wait queue. Each entry pins a live
/// `SpawnParams` (incl. attachments) in RAM, so the queue must stay bounded;
/// past this an over-limit spawn hard-rejects (the model waits / kills some).
const MAX_QUEUED_SUBAGENTS: usize = 256;

/// A sub-agent spawn parked while its parent session is at the concurrency
/// limit. The run row + projection already exist (status `Queued`); only the
/// launch is deferred. Holds the live `SpawnParams` (not persistable), so it
/// lives only in this process's memory.
pub struct PendingSubagentSpawn {
    pub params: SpawnParams,
    pub run_id: String,
    pub child_session_id: String,
    /// `Some` only when the child is grouped (R5) AND its projection was created
    /// — threaded to `launch_subagent_run` so a promoted grouped child still
    /// suppresses its individual injection.
    pub effective_group_id: Option<String>,
    pub enqueued_at: std::time::Instant,
    /// Registered when the child is admitted, not when it is promoted, so a
    /// queued child keeps the parent trial open for its full queue+run life.
    pub eval_guard: Option<crate::eval_context::EvalSessionGuard>,
}

static QUEUE: LazyLock<Mutex<VecDeque<PendingSubagentSpawn>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
static SCHED_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

fn lock() -> std::sync::MutexGuard<'static, VecDeque<PendingSubagentSpawn>> {
    QUEUE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Whether the queue is at capacity (caller hard-rejects instead of parking).
pub fn is_full() -> bool {
    lock().len() >= MAX_QUEUED_SUBAGENTS
}

/// Park a spawn. `false` ⇒ the queue is full (caller must hard-reject). On
/// success wakes the scheduler in case a slot is already free.
pub fn enqueue(pending: PendingSubagentSpawn) -> bool {
    {
        let mut q = lock();
        if q.len() >= MAX_QUEUED_SUBAGENTS {
            return false;
        }
        q.push_back(pending);
    }
    wake_subagent_scheduler();
    true
}

/// Wake the scheduler to re-evaluate promotions (a slot freed / config raised).
pub fn wake_subagent_scheduler() {
    SCHED_NOTIFY.notify_one();
}

/// Remove a parked spawn by run id (cancel of a queued run). Returns it so the
/// caller can drop the live `SpawnParams`.
pub fn remove_for_run(run_id: &str) -> Option<PendingSubagentSpawn> {
    let mut q = lock();
    let pos = q.iter().position(|p| p.run_id == run_id)?;
    q.remove(pos)
}

/// Drop every parked spawn for a session (session delete / incognito burn).
/// Returns the dropped run ids so the caller can stamp their rows terminal.
pub fn purge_for_session(session_id: &str) -> Vec<String> {
    let mut q = lock();
    let mut removed = Vec::new();
    q.retain(|p| {
        if p.params.parent_session_id == session_id {
            removed.push(p.run_id.clone());
            false
        } else {
            true
        }
    });
    removed
}

/// Distinct (parent_session_id, parent_agent_id) pairs currently parked. The
/// scheduler uses these to compute per-session availability without holding the
/// queue lock across DB calls.
fn queued_session_keys() -> Vec<(String, String)> {
    let q = lock();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in q.iter() {
        if seen.insert(p.params.parent_session_id.clone()) {
            out.push((
                p.params.parent_session_id.clone(),
                p.params.parent_agent_id.clone(),
            ));
        }
    }
    out
}

/// Pop the oldest parked spawn for a session (FIFO within a session). `None` ⇒
/// no parked spawn for that session.
fn take_for_session(session_id: &str) -> Option<PendingSubagentSpawn> {
    let mut q = lock();
    let pos = q
        .iter()
        .position(|p| p.params.parent_session_id == session_id)?;
    q.remove(pos)
}

/// Per-process sub-agent promotion scheduler. Idempotent (at most one loop per
/// process; both `app_init` paths may call it). Wakes on [`SCHED_NOTIFY`]
/// (terminal status freed a slot / enqueue) with a 5s fallback tick (config
/// raise / missed wake), and for each session under its concurrency limit
/// promotes parked spawns until full. Mirrors `async_jobs::spawn::run_scheduler`.
pub async fn run_subagent_scheduler() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    loop {
        tokio::select! {
            _ = SCHED_NOTIFY.notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        }
        let Some(db) = crate::globals::get_session_db() else {
            continue;
        };
        let Some(registry) = crate::get_subagent_cancels() else {
            continue;
        };
        for (session, parent_agent_id) in queued_session_keys() {
            let max = super::max_concurrent_for_agent(&parent_agent_id);
            let active = match db.count_active_subagent_runs(&session) {
                Ok(n) => n,
                Err(e) => {
                    crate::app_warn!(
                        "subagent",
                        "scheduler",
                        "Failed to count active runs for session {}: {}",
                        session,
                        e
                    );
                    continue;
                }
            };
            let mut free = max.saturating_sub(active);
            while free > 0 {
                let Some(pending) = take_for_session(&session) else {
                    break;
                };
                // Guarded promote: flip `Queued → Spawning` atomically, then
                // launch. A no-op CAS means a concurrent cancel/purge already
                // stamped the row terminal — drop the spawn WITHOUT spending a
                // slot (don't decrement `free`), so a killed run is never
                // resurrected into a running child. The queue mutex already
                // serialized this dequeue against cancel's `remove_for_run`; the
                // CAS closes the remaining promote-vs-cancel window. This is the
                // subagent analogue of R7.1's atomic dequeue-claim in
                // `async_jobs::slots`.
                if promote(pending, &db, &registry) {
                    free -= 1;
                }
            }
        }
    }
}

/// Flip a parked spawn `Queued → Spawning` via a guarded CAS and launch it.
/// Returns `false` (and does NOT launch) when the row is no longer `Queued` — a
/// concurrent cancel already stamped it terminal, so launching would run a child
/// the user already killed. Runs on the scheduler task.
fn promote(
    pending: PendingSubagentSpawn,
    db: &std::sync::Arc<crate::session::SessionDB>,
    registry: &std::sync::Arc<super::SubagentCancelRegistry>,
) -> bool {
    match db.try_transition_subagent_status(
        &pending.run_id,
        SubagentStatus::Queued,
        SubagentStatus::Spawning,
    ) {
        Ok(true) => {}
        Ok(false) => return false, // lost to a concurrent cancel/purge
        Err(e) => {
            crate::app_warn!(
                "subagent",
                "scheduler",
                "Failed to promote queued run {}: {}",
                pending.run_id,
                e
            );
            return false;
        }
    }
    super::spawn::launch_subagent_run(
        pending.params,
        pending.run_id,
        pending.child_session_id,
        pending.effective_group_id,
        pending.eval_guard,
        pending
            .enqueued_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64,
        db.clone(),
        registry.clone(),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(session: &str, agent: &str) -> SpawnParams {
        SpawnParams {
            task: "t".into(),
            agent_id: "helper".into(),
            parent_session_id: session.into(),
            parent_agent_id: agent.into(),
            depth: 1,
            timeout_secs: None,
            model_override: None,
            label: None,
            isolate_worktree: false,
            attachments: Vec::new(),
            plan_agent_mode: None,
            plan_mode_allow_paths: Vec::new(),
            lock_plan_agent_mode: false,
            skip_parent_injection: false,
            extra_system_context: None,
            skill_allowed_tools: Vec::new(),
            reasoning_effort: None,
            skill_name: None,
            origin_source: None,
            origin_channel_kb_context: None,
            group_id: None,
        }
    }

    fn pending(run_id: &str, session: &str) -> PendingSubagentSpawn {
        PendingSubagentSpawn {
            params: params(session, "ha-main"),
            run_id: run_id.into(),
            child_session_id: format!("child-{run_id}"),
            effective_group_id: None,
            enqueued_at: std::time::Instant::now(),
            eval_guard: None,
        }
    }

    #[test]
    fn enqueue_remove_and_purge() {
        // Unique session ids isolate this test from the process-global queue.
        let s = "queue-test-A";
        assert!(enqueue(pending("rA1", s)));
        assert!(enqueue(pending("rA2", s)));
        // remove a specific run
        assert!(remove_for_run("rA1").is_some());
        assert!(remove_for_run("rA1").is_none(), "already removed");
        // purge the rest for the session
        let purged = purge_for_session(s);
        assert_eq!(purged, vec!["rA2".to_string()]);
        assert!(purge_for_session(s).is_empty());
    }

    #[test]
    fn take_for_session_is_fifo_and_scoped() {
        let s1 = "queue-test-B1";
        let s2 = "queue-test-B2";
        enqueue(pending("rB1", s1));
        enqueue(pending("rB2", s2));
        enqueue(pending("rB3", s1));
        // FIFO within s1: rB1 before rB3; s2 untouched.
        assert_eq!(take_for_session(s1).unwrap().run_id, "rB1");
        assert_eq!(take_for_session(s1).unwrap().run_id, "rB3");
        assert!(take_for_session(s1).is_none());
        assert_eq!(take_for_session(s2).unwrap().run_id, "rB2");
        // cleanup
        purge_for_session(s1);
        purge_for_session(s2);
    }

    #[test]
    fn queued_session_keys_dedups() {
        let s = "queue-test-C";
        enqueue(pending("rC1", s));
        enqueue(pending("rC2", s));
        let keys = queued_session_keys();
        let count = keys.iter().filter(|(sess, _)| sess == s).count();
        assert_eq!(count, 1, "one entry per distinct session");
        purge_for_session(s);
    }

    #[test]
    fn queued_child_keeps_eval_trace_open_until_removed() {
        let parent = "queue-eval-parent";
        let child = "queue-eval-child";
        let trial = "mtrial_queue_eval";
        let context = crate::eval_context::EvalRunContext {
            evidence_kind: "model_campaign".into(),
            campaign_id: "mcampaign_queue_eval".into(),
            plan_digest: "1".repeat(64),
            suite_id: "hope-core".into(),
            suite_version: "1.0.0".into(),
            suite_digest: "2".repeat(64),
            case_id: "HA-QUEUE-EVAL".into(),
            case_digest: "3".repeat(64),
            trial_id: trial.into(),
            trial_index: 0,
            arm: "control".into(),
            fault_profile: "clean".into(),
            orchestration_profile: None,
            trace_id: "trace_queue_eval".into(),
            root_span_id: "span_queue_eval".into(),
            model_role: "anchor".into(),
            seed: 1,
            source: "local_cli".into(),
            commit_sha: "4".repeat(40),
            dirty: true,
            app_version: "0.17.0".into(),
            required_signals: Vec::new(),
            faults: Vec::new(),
            budget: crate::eval_context::EvalBudgetLimits::default(),
        };
        let root = crate::eval_context::register_root_session(parent, context.clone()).unwrap();
        let child_guard =
            crate::eval_context::register_child_session_from_parent(parent, child, context)
                .unwrap();
        let mut item = pending("r-eval-queued", parent);
        item.child_session_id = child.into();
        item.eval_guard = Some(child_guard);
        assert!(enqueue(item));

        drop(root);
        let queued = crate::eval_context::telemetry_snapshot(trial).unwrap();
        assert_eq!(queued["trace"]["closed"], false);

        drop(remove_for_run("r-eval-queued"));
        let closed = crate::eval_context::telemetry_snapshot(trial).unwrap();
        assert_eq!(closed["trace"]["closed"], true);
    }
}

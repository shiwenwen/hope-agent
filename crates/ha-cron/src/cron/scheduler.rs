use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cron::executor::{
    execute_claimed_job, pump_session_turns, spawn_session_turn_pump_once, stage_session_turn,
};
use ha_core::cron::CronDB;
use ha_core::cron_defs::CronJob;

/// §9 (C6): if the previous scheduler's last heartbeat is older than this at
/// startup, warn that the (Primary-only) scheduler was offline for a stretch —
/// a multi-minute silent gap usually means the Primary process died while the
/// host stayed up. Catch-up still recovers due runs; this is pure observability.
const HEARTBEAT_STALE_WARN_SECS: i64 = 300;

// ── Concurrency gate (§4) ───────────────────────────────────────

/// How many more cron jobs may be dispatched this pass, given the configured
/// cap and the count already running. `max == None` (unlimited) returns `None`
/// (unbounded); a full cap returns `Some(0)`. Pure so the slot arithmetic is
/// unit-testable without a scheduler or DB.
fn available_slots(max: Option<usize>, running: usize) -> Option<usize> {
    max.map(|m| m.saturating_sub(running))
}

/// Claim and spawn due jobs up to the available concurrency budget, acquiring a
/// slot **before** the claim that advances `next_run_at` (slot-before-claim):
/// jobs beyond the cap keep their schedule untouched and are retried on the next
/// tick instead of silently skipping the occurrence. Manual `run now` runs
/// bypass this gate but their running markers are counted by `count_running`, so
/// the scheduler won't over-spawn while a manual run is in flight. Returns the
/// number of jobs actually dispatched.
fn dispatch_due_jobs(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<ha_core::session::SessionDB>,
    due_jobs: Vec<CronJob>,
) -> usize {
    if due_jobs.is_empty() {
        return 0;
    }
    let (session_turns, due_jobs): (Vec<_>, Vec<_>) = due_jobs
        .into_iter()
        .partition(|job| matches!(job.payload, ha_core::cron::CronPayload::SessionTurn { .. }));
    let mut dispatched = 0usize;
    for job in session_turns {
        match stage_session_turn(cron_db, session_db, &job, false) {
            Ok(Some(_)) => dispatched += 1,
            Ok(None) => {}
            Err(error) => app_error!(
                "cron",
                "scheduler",
                "Failed to queue SessionTurn job '{}': {error:#}",
                job.name
            ),
        }
    }
    if dispatched > 0 {
        spawn_session_turn_pump_once(cron_db.clone(), session_db.clone());
    }
    if due_jobs.is_empty() {
        return dispatched;
    }
    let max = ha_core::config::cached_config()
        .cron
        .effective_max_concurrent();
    // Fail closed on a count error: skip this pass rather than risk an unbounded
    // spawn (a poisoned lock would fail the claims below anyway; next tick retries).
    let running = match cron_db.count_running() {
        Ok(n) => n,
        Err(e) => {
            app_error!(
                "cron",
                "scheduler",
                "Failed to count running jobs; deferring this pass: {}",
                e
            );
            return 0;
        }
    };
    let mut available = available_slots(max, running);
    let total = due_jobs.len();
    for (idx, job) in due_jobs.into_iter().enumerate() {
        if available == Some(0) {
            app_info!(
                "cron",
                "scheduler",
                "At cron concurrency cap ({} running); deferring {} due job(s) to next tick",
                running,
                total - idx
            );
            break;
        }
        let foreground_stop_admission = match session_db.foreground_stop_admission(None) {
            Ok(admission) => admission,
            Err(error) => {
                app_error!(
                    "cron",
                    "scheduler",
                    "Failed to capture the Global Stop fence before claiming job '{}': {error:#}",
                    job.name
                );
                continue;
            }
        };
        match ha_core::agent_lifecycle::with_lifecycle_gate(|| {
            cron_db.claim_scheduled_job_for_execution(&job)
        }) {
            Ok(Some(claimed)) => {
                if let Some(a) = available.as_mut() {
                    *a -= 1;
                }
                dispatched += 1;
                let db = cron_db.clone();
                let sdb = session_db.clone();
                tokio::spawn(async move {
                    execute_claimed_job(&db, &sdb, claimed, foreground_stop_admission).await;
                });
            }
            // Lost the race / already claimed elsewhere — do NOT consume a slot.
            Ok(None) => {
                app_debug!(
                    "cron",
                    "scheduler",
                    "Job '{}' already claimed, skipping",
                    job.name
                );
            }
            Err(e) => {
                app_error!(
                    "cron",
                    "scheduler",
                    "Failed to claim job '{}': {}",
                    job.name,
                    e
                );
            }
        }
    }
    dispatched
}

fn recover_session_turn_boundaries(
    cron_db: &Arc<CronDB>,
    session_db: &Arc<ha_core::session::SessionDB>,
) -> anyhow::Result<()> {
    // Session and Cron use separate SQLite ledgers. A crash after queue commit
    // but after/before a failed Cron publication can leave a durable Scheduled
    // row whose exact occurrence is terminal or gone. Sweep by immutable queue
    // id so deleting a page cannot skip later rows; live envelopes are retained.
    let mut after_queue_row_id = 0;
    loop {
        let identities =
            session_db.list_scheduled_turn_queue_identities(after_queue_row_id, 128)?;
        let page_len = identities.len();
        if let Some(last) = identities.last() {
            after_queue_row_id = last.queue_row_id;
        }
        for identity in identities {
            if cron_db.session_turn_queue_envelope_is_live(
                identity.run_log_id,
                &identity.request_id,
                &identity.session_id,
            )? {
                continue;
            }
            let source_ref = identity.run_log_id.to_string();
            if !session_db.cancel_scheduled_turn_message(&identity.request_id, &source_ref)? {
                match session_db.get_scheduled_turn_message(&source_ref)? {
                    None => {}
                    Some(_) => {
                        anyhow::bail!("scheduled orphan queue custody changed during recovery")
                    }
                }
            }
        }
        if page_len < 128 {
            break;
        }
    }

    // A previous process may have committed the ordinary ChatTurn immediately
    // before crashing. ChatTurn.user_message_id is the durable commit marker
    // used to repair the exact history anchor before the open run is closed.
    let mut after_id = 0;
    loop {
        let commits = cron_db.list_unprojected_session_turn_commits(after_id, 128)?;
        let page_len = commits.len();
        if let Some(last) = commits.last() {
            after_id = last.0;
        }
        for (run_log_id, request_id, turn_id) in commits {
            let committed = session_db
                .get_chat_turn(&turn_id)?
                .and_then(|turn| turn.user_message_id);
            if let Some(message_id) = committed {
                if !cron_db.set_session_turn_target_message(
                    run_log_id,
                    &request_id,
                    &turn_id,
                    message_id,
                )? {
                    anyhow::bail!("scheduled commit projection changed during recovery");
                }
            } else {
                let source_ref = run_log_id.to_string();
                if !session_db.cancel_scheduled_turn_message(&request_id, &source_ref)? {
                    match session_db.get_scheduled_turn_message(&source_ref)? {
                        None => {}
                        Some(_) => {
                            anyhow::bail!("scheduled commit boundary custody is uncertain")
                        }
                    }
                }
            }
        }
        if page_len < 128 {
            break;
        }
    }
    // `preparing` is invisible to the pump. Publish it only if the exact queue
    // row survived; otherwise terminalize it without executing.
    after_id = 0;
    loop {
        let preparing = cron_db.list_stale_preparing_session_turn_runs(after_id, 128)?;
        let page_len = preparing.len();
        if let Some(last) = preparing.last() {
            after_id = last.run_log_id;
        }
        for run in preparing {
            let source_ref = run.run_log_id.to_string();
            match session_db.get_scheduled_turn_message(&source_ref)? {
                Some(row)
                    if row.request_id == run.request_id && row.session_id == run.session_id =>
                {
                    if !cron_db.mark_session_turn_queued(run.run_log_id, &run.request_id)? {
                        anyhow::bail!("scheduled preparing publication changed during recovery");
                    }
                }
                Some(_) => anyhow::bail!("scheduled preparing queue identity mismatch"),
                None => {
                    if !cron_db.finish_pending_session_turn(
                        run.run_log_id,
                        &run.request_id,
                        "error",
                        Some("Interrupted before session queue publication"),
                    )? {
                        anyhow::bail!("scheduled preparing custody changed during recovery");
                    }
                }
            }
        }
        if page_len < 128 {
            break;
        }
    }
    Ok(())
}

// ── Scheduler ───────────────────────────────────────────────────

/// Start the background cron scheduler on a dedicated OS thread with its own tokio runtime.
/// This avoids requiring an existing tokio runtime at call time (e.g. during Tauri .setup()).
pub fn start_scheduler(
    cron_db: Arc<CronDB>,
    session_db: Arc<ha_core::session::SessionDB>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("cron-scheduler".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create cron tokio runtime");

            rt.block_on(async move {
                // §9 (C6): scheduler liveness. Surface a long offline gap from the
                // previous (Primary) scheduler before recording our own heartbeat.
                match cron_db.last_scheduler_heartbeat() {
                    Ok(Some(prev)) => {
                        let gap_secs = Utc::now().signed_duration_since(prev).num_seconds();
                        if gap_secs >= HEARTBEAT_STALE_WARN_SECS {
                            app_warn!(
                                "cron",
                                "scheduler",
                                "Scheduler was offline ~{}s (last heartbeat {}); due runs are caught up per grace below",
                                gap_secs,
                                prev.to_rfc3339()
                            );
                        }
                    }
                    Ok(None) => {} // fresh DB / first ever run
                    Err(e) => {
                        app_error!("cron", "scheduler", "Failed to read scheduler heartbeat: {}", e)
                    }
                }

                // Startup recovery
                if let Err(error) = recover_session_turn_boundaries(&cron_db, &session_db) {
                    app_error!("cron", "scheduler", "SessionTurn recovery stopped fail-closed: {error:#}");
                }
                if let Err(e) = cron_db.recover_orphaned_runs() {
                    app_error!("cron", "scheduler", "Failed to recover orphaned runs: {}", e);
                }
                // Ungated on purpose: this is the only sweep that clears a marker
                // written before its run log existed, and it never runs again.
                match cron_db.clear_stale_running().map(Some) {
                    Ok(Some(n)) if n > 0 => app_warn!(
                        "cron",
                        "scheduler",
                        "Cleared {} stale running markers from previous session",
                        n
                    ),
                    Err(e) => app_error!(
                        "cron",
                        "scheduler",
                        "Failed to clear stale running markers: {}",
                        e
                    ),
                    _ => {}
                }
                if let Err(error) = crate::cron::workspace::reconcile_workspaces(
                    &cron_db,
                    &session_db,
                )
                .await
                {
                    app_error!("cron", "workspace_reconcile", "Startup reconcile failed: {error:#}");
                }
                // Mark un-fireable At jobs missed BEFORE catch-up: those past the
                // late-fire grace window (or claimed-then-crashed) are taken out
                // so catch-up only late-fires At jobs still within grace (§7).
                let at_grace_secs = ha_core::config::cached_config()
                    .cron
                    .effective_at_grace_secs();
                if let Err(e) = cron_db.mark_missed_at_jobs(at_grace_secs) {
                    app_error!("cron", "scheduler", "Failed to mark missed at jobs: {}", e);
                }

                // Run catch-up for overdue recurring jobs (slot-before-claim cap applies).
                if let Ok(due_jobs) = cron_db.get_due_jobs(&Utc::now()) {
                    if !due_jobs.is_empty() {
                        app_info!(
                            "cron",
                            "scheduler",
                            "Catch-up: {} overdue jobs found at startup",
                            due_jobs.len()
                        );
                        dispatch_due_jobs(&cron_db, &session_db, due_jobs);
                    }
                }

                // SessionTurn readiness changes when any ordinary turn releases,
                // independently of the 15s schedule cadence. A small bounded
                // Primary pump keeps that FIFO handoff responsive.
                let pump_db = cron_db.clone();
                let pump_sessions = session_db.clone();
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_secs(1));
                    loop {
                        interval.tick().await;
                        pump_session_turns(&pump_db, &pump_sessions).await;
                    }
                });

                // §9 (C6): record liveness only AFTER startup recovery completes.
                // Writing it earlier would let a crash confined to the recovery
                // phase refresh the heartbeat on every boot, masking the "scheduler
                // was offline ~Ns" warning in a startup crash-loop. Recovery itself
                // is idempotent and re-runs each boot regardless; the per-tick
                // refresh below keeps it current once the loop is running.
                let _ = cron_db.record_scheduler_heartbeat();

                app_info!("cron", "scheduler", "Scheduler started");
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
                let tick_running = Arc::new(AtomicBool::new(false));

                loop {
                    interval.tick().await;

                    // §9 (C6): refresh liveness heartbeat every tick (cheap UPSERT),
                    // even when the tick is skipped below — it records the scheduler
                    // is alive, not that work happened.
                    let _ = cron_db.record_scheduler_heartbeat();

                    // Scheduler-level guard: skip this tick only if the previous
                    // tick's claim/dispatch pass hasn't returned yet. It guards
                    // the dispatch loop against overlapping itself — NOT job
                    // execution (jobs run on their own spawned tasks and the §4
                    // slot cap bounds their concurrency, see dispatch_due_jobs).
                    if tick_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        app_debug!("cron", "scheduler", "Previous tick still running, skipping");
                        continue;
                    }

                    let now = Utc::now();

                    // Atomic terminal/accounting failures hand the still-open
                    // exact occurrence to this recovery pass. Run it before the
                    // At missed sweep so a completed/interrupted one-shot is
                    // settled by its marker owner, never reclassified as missed.
                    if let Err(error) = cron_db.recover_orphaned_runs() {
                        app_error!(
                            "cron",
                            "scheduler",
                            "Failed to recover unsettled cron occurrences: {error:#}"
                        );
                    }

                    if let Err(error) = crate::cron::workspace::reconcile_workspaces(
                        &cron_db,
                        &session_db,
                    )
                    .await
                    {
                        app_error!("cron", "workspace_reconcile", "Tick reconcile failed: {error:#}");
                    }

                    // §7 (review fix): reap un-fireable `At` jobs every tick, not
                    // only at startup. An `At` left within the late-fire grace
                    // window at startup but never claimed (e.g. sustained
                    // concurrency-cap pressure) ages past grace while the loop runs;
                    // without a per-tick reap it stays `active` and is re-evaluated
                    // forever instead of terminalizing to `missed`. Cheap UPDATE;
                    // the cutoff is recomputed from `now` each call, and it must run
                    // before dispatch so an aged-out `At` isn't picked as due.
                    let at_grace_secs = ha_core::config::cached_config()
                        .cron
                        .effective_at_grace_secs();
                    if let Err(e) = cron_db.mark_missed_at_jobs(at_grace_secs) {
                        app_error!("cron", "scheduler", "Failed to mark missed at jobs: {}", e);
                    }

                    match cron_db.get_due_jobs(&now) {
                        // Slot-before-claim: dispatch only up to the configured
                        // concurrency cap; remaining due jobs stay due and are
                        // retried next tick (their schedule is untouched).
                        Ok(due_jobs) => {
                            dispatch_due_jobs(&cron_db, &session_db, due_jobs);
                        }
                        Err(e) => {
                            app_error!("cron", "scheduler", "Failed to query due jobs: {}", e);
                        }
                    }

                    tick_running.store(false, Ordering::Release);
                }
            });
        })
        .expect("Failed to spawn cron scheduler thread")
}

#[cfg(test)]
mod tests {
    use super::{available_slots, recover_session_turn_boundaries};
    use ha_core::cron::{CronDB, CronPayload, CronSchedule, NewCronJob};
    use ha_core::session::{NewScheduledTurnMessage, SessionDB};
    use std::sync::Arc;

    #[test]
    fn available_slots_unlimited_is_unbounded() {
        // max=None (max_concurrent == 0) → never gated, regardless of running.
        assert_eq!(available_slots(None, 0), None);
        assert_eq!(available_slots(None, 1000), None);
    }

    #[test]
    fn available_slots_caps_at_remaining_budget() {
        assert_eq!(available_slots(Some(5), 0), Some(5));
        assert_eq!(available_slots(Some(5), 3), Some(2));
        assert_eq!(available_slots(Some(5), 5), Some(0));
        // Over-occupancy (e.g. manual run-now pushed past the cap) saturates at 0,
        // never underflows — the scheduler simply dispatches nothing this pass.
        assert_eq!(available_slots(Some(5), 8), Some(0));
    }

    #[test]
    fn startup_recovery_removes_only_terminal_scheduled_queue_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let cron_path = dir.path().join("cron.db");
        let session_db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).unwrap());
        let session_id = session_db.create_session("ha-main").unwrap().id;
        let (terminal_run_id, live_run_id) = {
            let cron_db = CronDB::open(&cron_path).unwrap();
            let job = cron_db
                .add_job(&NewCronJob {
                    name: "scheduled conversation".into(),
                    description: None,
                    project_id: None,
                    workspace_policy: Default::default(),
                    schedule: CronSchedule::Every {
                        interval_ms: 60_000,
                        start_at: None,
                    },
                    payload: CronPayload::SessionTurn {
                        session_id: session_id.clone(),
                        prompt: "continue".into(),
                    },
                    max_failures: None,
                    notify_on_complete: None,
                    delivery_targets: None,
                    prefix_delivery_with_name: None,
                    job_timeout_secs: None,
                    permission_mode_override: None,
                    sandbox_mode_override: None,
                })
                .unwrap();
            let terminal = cron_db
                .claim_session_turn_for_preparing(&job, "terminal-request", true)
                .unwrap()
                .unwrap();
            let terminal_admission = session_db
                .foreground_stop_admission(Some(&session_id))
                .unwrap();
            session_db
                .enqueue_scheduled_turn_message(
                    NewScheduledTurnMessage {
                        request_id: terminal.request_id.clone(),
                        session_id: session_id.clone(),
                        source_ref: terminal.run_log_id.to_string(),
                        message: "terminal".into(),
                    },
                    terminal_admission,
                )
                .unwrap();
            assert!(cron_db
                .finish_pending_session_turn(
                    terminal.run_log_id,
                    &terminal.request_id,
                    "error",
                    Some("publication failed"),
                )
                .unwrap());

            let live = cron_db
                .claim_session_turn_for_preparing(&job, "live-request", true)
                .unwrap()
                .unwrap();
            let live_admission = session_db
                .foreground_stop_admission(Some(&session_id))
                .unwrap();
            session_db
                .enqueue_scheduled_turn_message(
                    NewScheduledTurnMessage {
                        request_id: live.request_id.clone(),
                        session_id: session_id.clone(),
                        source_ref: live.run_log_id.to_string(),
                        message: "live".into(),
                    },
                    live_admission,
                )
                .unwrap();
            (terminal.run_log_id, live.run_log_id)
        };

        let cron_db = Arc::new(CronDB::open(&cron_path).unwrap());
        recover_session_turn_boundaries(&cron_db, &session_db).unwrap();

        assert!(session_db
            .get_scheduled_turn_message(&terminal_run_id.to_string())
            .unwrap()
            .is_none());
        assert!(session_db
            .get_scheduled_turn_message(&live_run_id.to_string())
            .unwrap()
            .is_some());
        assert_eq!(
            cron_db.get_run_log(live_run_id).unwrap().unwrap().status,
            "queued"
        );
    }
}

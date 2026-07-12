use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::db::CronDB;
use super::executor::execute_claimed_job;
use super::types::CronJob;

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
    session_db: &Arc<crate::session::SessionDB>,
    due_jobs: Vec<CronJob>,
) -> usize {
    if due_jobs.is_empty() {
        return 0;
    }
    let max = crate::config::cached_config()
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
    let mut dispatched = 0usize;
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
        match crate::agent_lifecycle::with_lifecycle_gate(|| {
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
                    execute_claimed_job(&db, &sdb, claimed).await;
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

// ── Scheduler ───────────────────────────────────────────────────

/// Start the background cron scheduler on a dedicated OS thread with its own tokio runtime.
/// This avoids requiring an existing tokio runtime at call time (e.g. during Tauri .setup()).
pub fn start_scheduler(
    cron_db: Arc<CronDB>,
    session_db: Arc<crate::session::SessionDB>,
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
                if let Err(e) = cron_db.recover_orphaned_runs() {
                    app_error!(
                        "cron",
                        "scheduler",
                        "Failed to recover orphaned runs: {}",
                        e
                    );
                }
                match cron_db.clear_all_running() {
                    Ok(n) if n > 0 => app_warn!(
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
                // Mark un-fireable At jobs missed BEFORE catch-up: those past the
                // late-fire grace window (or claimed-then-crashed) are taken out
                // so catch-up only late-fires At jobs still within grace (§7).
                let at_grace_secs = crate::config::cached_config()
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

                    // §7 (review fix): reap un-fireable `At` jobs every tick, not
                    // only at startup. An `At` left within the late-fire grace
                    // window at startup but never claimed (e.g. sustained
                    // concurrency-cap pressure) ages past grace while the loop runs;
                    // without a per-tick reap it stays `active` and is re-evaluated
                    // forever instead of terminalizing to `missed`. Cheap UPDATE;
                    // the cutoff is recomputed from `now` each call, and it must run
                    // before dispatch so an aged-out `At` isn't picked as due.
                    let at_grace_secs = crate::config::cached_config()
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
    use super::available_slots;

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
}

//! Background jobs (R1 unified model): detach long-running work into background
//! jobs, return a synthetic `job_id` to the LLM immediately, and inject the real
//! result back into the parent session when ready.
//!
//! [`JobManager`] is the single production entry point (spawn / cancel / list /
//! inspect / replay / schedule). Today only [`JobKind::Tool`] is wired — the
//! `Subagent` (R6) and `Group` (R5) kinds extend `JobManager`, not parallel
//! APIs. The module keeps the historical `async_jobs/` name + log category per
//! the PRD's "evolve the lineage in place" contract; the persisted table / DB
//! file are `background_jobs`.
//!
//! See `docs/architecture/tool-system.md` and AGENTS.md for the higher-level
//! design. The user-facing entry points are:
//!
//! - `run_in_background: true` on any `async_capable` tool
//! - Agent `capabilities.async_tool_policy = "always-background"`
//! - The auto-background budget (`config.async_tools.auto_background_secs`)
//!   for sync calls of async-capable tools
//!
//! The `job_status` meta tool is a snapshot/status escape hatch; completion
//! primarily flows through auto-injected `<task-notification>` messages.

pub(crate) mod approval_bridge;
pub(crate) mod approval_projection_watcher;
pub(crate) mod cancel;
pub(crate) mod db;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod injection;
pub(crate) mod manager;
pub(crate) mod output_tail;
pub(crate) mod retention;
pub(crate) mod retry;
pub(crate) mod slots;
pub(crate) mod spawn;
pub(crate) mod types;
pub(crate) mod wait;

use std::sync::{Arc, OnceLock};

pub use db::{JobsDB, PurgeStats};
// R1: `JobManager` is the single production entry point for background-job
// operations (spawn / cancel / list / replay / schedule). The spawn / scheduler
// / cancel / retention helpers are now `pub(crate)` internals reached only
// through it. `synthetic_started_result` stays re-exported as a pure formatter.
pub use manager::JobManager;
pub use spawn::synthetic_started_result;
pub use types::{BackgroundJob, BackgroundJobSnapshot, JobKind, JobOrigin, JobStatus};

static ASYNC_JOBS_DB: OnceLock<Arc<JobsDB>> = OnceLock::new();

/// Set the global background-jobs database. Called once during app
/// initialization (`app_init`).
pub(crate) fn set_async_jobs_db(db: Arc<JobsDB>) {
    let _ = ASYNC_JOBS_DB.set(db);
}

/// Get the global background-jobs database (None until initialization
/// completes). The white-box read accessor used by `job_status` /
/// `runtime_tasks` / tests; production *operations* go through [`JobManager`].
pub(crate) fn get_async_jobs_db() -> Option<&'static Arc<JobsDB>> {
    ASYNC_JOBS_DB.get()
}

/// Best-effort cancellation for an async tool job. Returns the updated job
/// snapshot when the job exists.
pub(crate) fn cancel_job(job_id: &str) -> anyhow::Result<Option<BackgroundJob>> {
    let Some(db) = get_async_jobs_db() else {
        return Ok(None);
    };
    let Some(job) = db.load(job_id)? else {
        return Ok(None);
    };
    if job.status.is_terminal() {
        return Ok(Some(job));
    }

    // R6: a `kind=subagent` projection's cancel routes to the subagent cancel
    // registry (`subagent_runs` is the truth source). The run's own terminal
    // handling stamps it Killed and syncs the projection → Cancelled via
    // `update_subagent_status`. We do NOT run the tool-job cancel dance
    // (terminal hooks / injection) here — the subagent fires its own
    // SubagentStop hook + `inject_and_run_parent`.
    if job.kind == JobKind::Subagent {
        // Optimistically reflect `cancelling` so the caller sees progress.
        // `None`, not a message: a projection is status/lifecycle-only and must
        // never hold run content in `error` (that lives in subagent_runs). Do
        // this BEFORE request_cancel_run so the projection still passes through
        // `cancelling` even when the no-flag fallback stamps the run Killed
        // synchronously (which would otherwise sync us straight to Cancelled).
        let _ = db.mark_cancelling(job_id, None);
        if let Some(run_id) = &job.subagent_run_id {
            crate::subagent::request_cancel_run(run_id);
        }
        return db.load(job_id);
    }

    if job.kind == JobKind::Monitor {
        crate::loop_control::cancel_loop_monitor_by_job_id(job_id, job.tool_call_id.as_deref())?;
        let cancelled_now = db.update_terminal(
            job_id,
            JobStatus::Cancelled,
            None,
            None,
            Some("Monitor cancelled by user"),
            chrono::Utc::now().timestamp(),
        )?;
        if cancelled_now {
            events::emit_completed(
                job_id,
                JobKind::Monitor,
                &job.tool_name,
                JobStatus::Cancelled.as_str(),
                job.session_id.as_deref(),
            );
        }
        return db.load(job_id);
    }

    // R5: cancelling a `Group` marks the group terminal FIRST, THEN cancels each
    // child. Order is load-bearing: `request_cancel_run`'s no-flag fallback
    // stamps a child `Killed` *synchronously*, which flows through
    // `update_subagent_status` → `try_complete_group` on this very thread — if
    // the group were still `Running` at that point, the join's single-winner CAS
    // would win and fire a merged injection for a batch the user just cancelled.
    // Marking the group `Cancelled` first makes that CAS lose. The children's
    // own terminal sync (Killed→Cancelled on their projections) still runs as
    // usual. The group carries no run content, so a lifecycle message on its
    // `error` column is fine (unlike a subagent projection).
    if job.kind == JobKind::Group {
        // Only emit terminal if THIS call actually transitioned the group — if
        // the join coordinator already completed it (CAS won the race), the
        // update no-ops and we must not emit a spurious second terminal event.
        let cancelled_now = db
            .update_terminal(
                job_id,
                JobStatus::Cancelled,
                None,
                None,
                Some("Cancelled by user"),
                chrono::Utc::now().timestamp(),
            )
            .unwrap_or(false);
        if cancelled_now {
            events::emit_completed(
                job_id,
                JobKind::Group,
                &job.tool_name,
                JobStatus::Cancelled.as_str(),
                job.session_id.as_deref(),
            );
        }
        if let Ok(children) = db.group_children(job_id) {
            for child in &children {
                if let Some(run_id) = &child.subagent_run_id {
                    crate::subagent::request_cancel_run(run_id);
                }
            }
        }
        return db.load(job_id);
    }

    // R7.1: a job still waiting in the in-memory scheduler queue has no runner
    // that will ever settle it — pull it out and finalize `Cancelled` directly.
    // The queue lock serializes this against the scheduler, so `Some` here means
    // the scheduler did NOT take it and it can never be promoted.
    if slots::remove_queued(job_id).is_some() {
        let _ = cancel::cancel_job(job_id); // defensive token trip
        cancel::remove_job(job_id);
        // R3: a queued exec job registered a tail ring at spawn but never ran;
        // drop it here since this path bypasses finalize_job.
        output_tail::remove(job_id);
        const QUEUED_MSG: &str = "Cancelled while queued, before a slot freed";
        let _ = db.update_terminal(
            job_id,
            JobStatus::Cancelled,
            None,
            None,
            Some(QUEUED_MSG),
            chrono::Utc::now().timestamp(),
        )?;
        let (is_error, is_interrupt) = JobStatus::Cancelled.terminal_hook_flags();
        crate::hooks::fire_async_job_terminal(
            job.session_id.as_deref(),
            job.agent_id.as_deref(),
            &job.tool_name,
            job.tool_call_id.as_deref(),
            job_id,
            is_error,
            is_interrupt,
            QUEUED_MSG,
        );
        let _ = db.mark_injected(job_id);
        wait::notify_completion(job_id);
        events::emit_completed(
            job_id,
            job.kind,
            &job.tool_name,
            JobStatus::Cancelled.as_str(),
            job.session_id.as_deref(),
        );
        return db.load(job_id);
    }

    // I4: persist the cross-process cancel flag FIRST — it now also covers a row
    // still in the spawn window (`queued` in the DB but not yet handed to the
    // scheduler queue) so it is cancelled once it runs and polls the flag. The
    // in-memory token signal below only reaches a runner in THIS process. A
    // runner owning this job in ANOTHER process observes the flag on its poll.
    if let Err(e) = db.set_cancel_requested(job_id) {
        app_warn!(
            "async_jobs",
            "cancel",
            "Failed to set cancel_requested flag for {}: {}",
            job_id,
            e
        );
    }
    let signalled = cancel::cancel_job(job_id);

    // R8: if this job is parked on a human approval, dismiss that dialog NOW
    // (drops the pending sender → the parked `rx.await` wakes so the dispatch
    // winds down within the cancel grace instead of dead-waiting the full 5s,
    // and closes the window where an Allow click during the grace would run the
    // just-cancelled command). The command gate sees the dropped sender as a
    // cancellation, so it never approves; the job still settles Cancelled via the
    // token trip above. `on_resume` would also dismiss when the future finally
    // drops, but that is up to 5s later — and never fires at all for a
    // cross-process cancel that only sets the DB flag, so do it here too.
    // remove-if-present + emit-if-removed makes a later/duplicate dismiss a no-op.
    if let Some(request_id) = approval_bridge::parked_request_id(job_id) {
        crate::tools::approval::dismiss_parked_job_approval(&request_id, job.session_id.as_deref());
    }

    if !db.mark_cancelling(job_id, Some("Cancellation requested"))? {
        // Not running/cancelling (still `queued` in the spawn window, or it just
        // settled). The cancel flag set above cancels it once it runs; nothing
        // to force here.
        return db.load(job_id);
    }
    if !signalled {
        // No in-process runner owns this job id. Mark it terminal so callers
        // are not left with an un-cancellable row forever; any late runner
        // completion is ignored by `update_terminal`'s active-status guard.
        const NO_RUNNER_MSG: &str = "Cancelled; no active runner handle was found in this process";
        let _ = db.update_terminal(
            job_id,
            JobStatus::Cancelled,
            None,
            None,
            Some(NO_RUNNER_MSG),
            chrono::Utc::now().timestamp(),
        )?;
        // review#5: this is a real terminal settle, so it must be visible to
        // PostToolUse/PostToolUseFailure hooks like finalize_job and
        // replay_pending_jobs (H4 contract: cancelled jobs fire the hook,
        // is_interrupt=true). mark_injected below closes off the replay
        // fallback, so fire it here. Deliberately NOT routed through
        // dispatch_injection (cancel comes from turn-cancel/session-delete;
        // injecting would spawn an unwanted parent turn / hit a ghost session).
        let (is_error, is_interrupt) = JobStatus::Cancelled.terminal_hook_flags();
        crate::hooks::fire_async_job_terminal(
            job.session_id.as_deref(),
            job.agent_id.as_deref(),
            &job.tool_name,
            job.tool_call_id.as_deref(),
            job_id,
            is_error,
            is_interrupt,
            NO_RUNNER_MSG,
        );
        let _ = db.mark_injected(job_id);
        wait::notify_completion(job_id);
        events::emit_completed(
            job_id,
            job.kind,
            &job.tool_name,
            JobStatus::Cancelled.as_str(),
            job.session_id.as_deref(),
        );
    } else {
        wait::notify_completion(job_id);
        events::emit_updated(
            job_id,
            job.kind,
            &job.tool_name,
            JobStatus::Cancelling.as_str(),
            job.session_id.as_deref(),
        );
    }
    db.load(job_id)
}

/// Cancel every active (`running`/`cancelling`/`awaiting_approval`) job owned
/// by `session_id`. Called by the session cleanup watcher when a session is
/// deleted or purged so abandoned background jobs don't run on forever
/// (DELETE-4). Returns the number of jobs cancelled.
pub(crate) fn cancel_jobs_for_session(session_id: &str) -> usize {
    let Some(db) = get_async_jobs_db() else {
        return 0;
    };
    let jobs = match db.list_active_by_session(session_id) {
        Ok(j) => j,
        Err(e) => {
            app_warn!(
                "async_jobs",
                "cleanup",
                "list_active_by_session failed for {}: {}",
                session_id,
                e
            );
            return 0;
        }
    };
    let mut cancelled = 0;
    for job in jobs {
        if cancel_job(&job.job_id).is_ok() {
            cancelled += 1;
        }
    }
    if cancelled > 0 {
        app_info!(
            "async_jobs",
            "cleanup",
            "cancelled {} active job(s) for removed session {}",
            cancelled,
            session_id
        );
    }
    cancelled
}

/// Physically delete **all** job rows for `session_id` and their spool files.
/// Called by the session cleanup watcher on **purge** (incognito burn-on-close)
/// — incognito jobs already skip the spool ([`spawn::record_running_job`] +
/// `persist_result`), so this is a backstop that also drops the redacted job
/// rows themselves so nothing about the burned session lingers. Returns the
/// number of rows deleted. Epic E (INCOG-2).
pub(crate) fn purge_jobs_for_session(session_id: &str) -> u64 {
    let Some(db) = get_async_jobs_db() else {
        return 0;
    };
    // R7.1: drop any still-queued jobs for this session from the in-memory
    // scheduler queue too — otherwise their `PreparedJob` (which pins the burned
    // session's live ctx, incl. sensitive incognito args) would linger in RAM
    // after the DB rows are deleted. Removing them also guarantees they are never
    // promoted (their row is being deleted here anyway).
    let drained = slots::remove_queued_for_session(session_id);
    if drained > 0 {
        app_info!(
            "async_jobs",
            "cleanup",
            "dropped {} queued job(s) from scheduler queue for burned session {}",
            drained,
            session_id
        );
    }
    // R4: also drop any completions buffered in the merge window for this session
    // — their PendingJobInjection holds tool name / preview / error in RAM and
    // would otherwise linger until the timer fires (incognito burn-on-close).
    let unbuffered = injection::remove_buffered_for_session(session_id);
    if unbuffered > 0 {
        app_info!(
            "async_jobs",
            "cleanup",
            "dropped {} buffered injection(s) from merge window for burned session {}",
            unbuffered,
            session_id
        );
    }
    match db.purge_jobs_for_session(session_id) {
        Ok(stats) => {
            if stats.rows_deleted > 0 || stats.spool_files_deleted > 0 {
                app_info!(
                    "async_jobs",
                    "cleanup",
                    "purged {} job row(s) + {} spool file(s) for burned session {}",
                    stats.rows_deleted,
                    stats.spool_files_deleted,
                    session_id
                );
            }
            stats.rows_deleted
        }
        Err(e) => {
            app_warn!(
                "async_jobs",
                "cleanup",
                "purge_jobs_for_session failed for {}: {}",
                session_id,
                e
            );
            0
        }
    }
}

/// Replay logic invoked from `start_background_tasks`:
///   1. Mark every job left in `running` as `interrupted` (the underlying
///      process did not survive the restart).
///   2. Re-dispatch any terminal-but-not-injected jobs back to their parent
///      sessions.
pub(crate) fn replay_pending_jobs() {
    let db = match get_async_jobs_db() {
        Some(db) => db.clone(),
        None => return,
    };

    let now = chrono::Utc::now().timestamp();
    match db.list_running() {
        Ok(rows) => {
            for job in rows {
                // I3: a job left `running` at restart may have a child process
                // tree that outlived the crash (a backgrounded exec). If we
                // recorded its pid and that pid is still alive, it's an orphan
                // with no remaining owner — terminate the whole group before
                // marking the row interrupted. (pid-reuse risk is bounded: the
                // restart window is short and exec children run in their own
                // process group; logged for audit.)
                if let Some(pid) = job.pid {
                    if pid > 0 && crate::platform::pid_alive(pid as u32) {
                        app_warn!(
                            "async_jobs",
                            "replay",
                            "Terminating orphaned process tree pid={} for interrupted job {}",
                            pid,
                            &job.job_id
                        );
                        crate::platform::terminate_process_tree(pid as u32);
                    }
                }
                if let Err(e) = db.update_terminal(
                    &job.job_id,
                    JobStatus::Interrupted,
                    None,
                    None,
                    Some("interrupted by application restart"),
                    now,
                ) {
                    app_warn!(
                        "async_jobs",
                        "replay",
                        "Failed to mark job {} interrupted: {}",
                        &job.job_id,
                        e
                    );
                }
            }
        }
        Err(e) => app_warn!(
            "async_jobs",
            "replay",
            "Failed to list running jobs on startup: {}",
            e
        ),
    }

    match db.list_pending_injection() {
        Ok(rows) => {
            for job in rows {
                // H6: this row is terminal but un-injected — it never had its
                // terminal hook fired (process died before finalize, or it was
                // just marked `interrupted` above). Fire it now so async
                // terminals stay visible to hooks across restarts (HOOKS-1/4).
                // Not double-fired in the normal path: a finalized job is
                // injected=true and excluded by `list_pending_injection`; only
                // crash/restart survivors reach here.
                {
                    let (is_error, is_interrupt) = job.status.terminal_hook_flags();
                    let detail = if is_error {
                        job.error.as_deref().unwrap_or("")
                    } else {
                        job.result_preview.as_deref().unwrap_or("")
                    };
                    crate::hooks::fire_async_job_terminal(
                        job.session_id.as_deref(),
                        job.agent_id.as_deref(),
                        &job.tool_name,
                        job.tool_call_id.as_deref(),
                        &job.job_id,
                        is_error,
                        is_interrupt,
                        detail,
                    );
                }

                if job.status == JobStatus::Cancelled {
                    let _ = db.mark_injected(&job.job_id);
                    continue;
                }
                let Some(session_id) = job.session_id.clone() else {
                    let _ = db.mark_injected(&job.job_id);
                    continue;
                };
                injection::dispatch_injection(
                    session_id,
                    job.agent_id.clone(),
                    job.job_id.clone(),
                    job.tool_name.clone(),
                    job.tool_call_id.clone(),
                    job.status,
                    job.result_preview.clone(),
                    job.result_path.clone(),
                    job.error.clone(),
                );
            }
        }
        Err(e) => app_warn!(
            "async_jobs",
            "replay",
            "Failed to list pending injections on startup: {}",
            e
        ),
    }
}

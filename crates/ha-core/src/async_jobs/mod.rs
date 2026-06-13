//! Async tool execution: detach long-running tool calls into background jobs,
//! return a synthetic `job_id` to the LLM immediately, and inject the real
//! result back into the parent session when ready.
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

pub(crate) mod cancel;
pub(crate) mod db;
pub(crate) mod error;
pub(crate) mod injection;
pub(crate) mod retention;
pub(crate) mod spawn;
pub(crate) mod types;
pub(crate) mod wait;

use std::sync::{Arc, OnceLock};

pub use db::{AsyncJobsDB, PurgeStats};
pub use retention::{
    run_once as run_retention_once, spawn_background_loop as spawn_retention_loop,
};
pub use spawn::{dispatch_with_auto_background, spawn_explicit_job, synthetic_started_result};
pub use types::{AsyncJob, AsyncJobStatus, JobOrigin};

static ASYNC_JOBS_DB: OnceLock<Arc<AsyncJobsDB>> = OnceLock::new();

/// Set the global async jobs database. Called once during app initialization.
pub fn set_async_jobs_db(db: Arc<AsyncJobsDB>) {
    let _ = ASYNC_JOBS_DB.set(db);
}

/// Get the global async jobs database (None until initialization completes).
pub fn get_async_jobs_db() -> Option<&'static Arc<AsyncJobsDB>> {
    ASYNC_JOBS_DB.get()
}

/// Best-effort cancellation for an async tool job. Returns the updated job
/// snapshot when the job exists.
pub fn cancel_job(job_id: &str) -> anyhow::Result<Option<AsyncJob>> {
    let Some(db) = get_async_jobs_db() else {
        return Ok(None);
    };
    let Some(job) = db.load(job_id)? else {
        return Ok(None);
    };
    if job.status.is_terminal() {
        return Ok(Some(job));
    }

    if !db.mark_cancelling(job_id, Some("Cancellation requested"))? {
        return db.load(job_id);
    }
    // I4: persist the cross-process cancel flag so a runner owning this job in
    // ANOTHER process (desktop + headless server share async_jobs.db) observes
    // it on its next poll and aborts the work — the in-memory token signal
    // below only reaches a runner in THIS process. Best-effort.
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
    if !signalled {
        // No in-process runner owns this job id. Mark it terminal so callers
        // are not left with an un-cancellable row forever; any late runner
        // completion is ignored by `update_terminal`'s active-status guard.
        const NO_RUNNER_MSG: &str = "Cancelled; no active runner handle was found in this process";
        let _ = db.update_terminal(
            job_id,
            AsyncJobStatus::Cancelled,
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
        let (is_error, is_interrupt) = AsyncJobStatus::Cancelled.terminal_hook_flags();
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
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "async_tool_job:completed",
                serde_json::json!({
                    "job_id": job_id,
                    "tool": job.tool_name,
                    "status": AsyncJobStatus::Cancelled.as_str(),
                }),
            );
        }
    } else {
        wait::notify_completion(job_id);
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "async_tool_job:updated",
                serde_json::json!({
                    "job_id": job_id,
                    "tool": job.tool_name,
                    "status": AsyncJobStatus::Cancelling.as_str(),
                }),
            );
        }
    }
    db.load(job_id)
}

/// Cancel every active (`running`/`cancelling`/`awaiting_approval`) job owned
/// by `session_id`. Called by the session cleanup watcher when a session is
/// deleted or purged so abandoned background jobs don't run on forever
/// (DELETE-4). Returns the number of jobs cancelled.
pub fn cancel_jobs_for_session(session_id: &str) -> usize {
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

/// Replay logic invoked from `start_background_tasks`:
///   1. Mark every job left in `running` as `interrupted` (the underlying
///      process did not survive the restart).
///   2. Re-dispatch any terminal-but-not-injected jobs back to their parent
///      sessions.
pub fn replay_pending_jobs() {
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
                    AsyncJobStatus::Interrupted,
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

                if job.status == AsyncJobStatus::Cancelled {
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

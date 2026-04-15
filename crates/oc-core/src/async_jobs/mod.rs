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
//! The `job_status` deferred tool lets the model actively wait for results.

pub(crate) mod db;
pub(crate) mod injection;
pub(crate) mod retention;
pub(crate) mod spawn;
pub(crate) mod types;

use std::sync::{Arc, OnceLock};

pub use db::{AsyncJobsDB, PurgeStats};
pub use retention::{run_once as run_retention_once, spawn_background_loop as spawn_retention_loop};
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
                let Some(session_id) = job.session_id.clone() else {
                    let _ = db.mark_injected(&job.job_id);
                    continue;
                };
                injection::dispatch_injection(
                    session_id,
                    job.agent_id.clone(),
                    job.job_id.clone(),
                    job.tool_name.clone(),
                    job.status,
                    job.result_preview.clone(),
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

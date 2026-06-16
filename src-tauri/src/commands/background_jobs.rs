//! R4 owner-plane background-jobs panel commands.
//!
//! Thin wrappers over [`ha_core::async_jobs::JobManager`] owner-plane reads. The
//! desktop shell is host-trusted, so there is no scope/auth here — the session
//! id is the only filter (a session sees its own jobs). Cancellation reuses the
//! existing `cancel_runtime_task(kind = async_job, …)` path, so no cancel command
//! is added here.

use crate::commands::CmdError;
use ha_core::async_jobs::{BackgroundJobSnapshot, JobManager};

/// List a session's background jobs (active + recent terminal) for the panel.
#[tauri::command]
pub async fn list_background_jobs(
    session_id: String,
) -> Result<Vec<BackgroundJobSnapshot>, CmdError> {
    JobManager::list_session_snapshots(&session_id).map_err(Into::into)
}

/// Snapshot a single background job — includes the live running-output tail for
/// a backgrounded `exec`. `Ok(None)` when the job is unknown.
#[tauri::command]
pub async fn get_background_job(
    job_id: String,
) -> Result<Option<BackgroundJobSnapshot>, CmdError> {
    JobManager::get_job_snapshot(&job_id).map_err(Into::into)
}

//! R8: bridge a background-job runner to the approval engine.
//!
//! A backgrounded tool job runs its dispatch on a dedicated OS thread (see
//! [`super::spawn::start_runner`]). When that dispatch reaches an *attended*
//! approval gate (`exec`'s command-level gate, now run on the job thread for the
//! explicit-background path), the dispatch future blocks on the approval
//! engine's oneshot — the job is genuinely **parked waiting for a human**, not
//! running. This module installs a thread-local [`BackgroundApprovalBridge`]
//! (defined in `tools::approval` so `tools` keeps zero dependency on
//! `async_jobs`) whose closures flip the job row `Running ⇄ AwaitingApproval`
//! around that wait, and records the pending `request_id` so a cancel of the
//! parked job can dismiss the now-orphaned dialog.
//!
//! Scope: only the explicit / policy `ImmediateBackground` exec path parks here
//! (its approval reorder is deferred to the job thread). Auto-background and
//! sync exec resolve approval up front (unchanged). A background *subagent*'s
//! inner tool approvals run on the subagent's own runtime — the bridge is not
//! installed there, so its projection job's status is unaffected (its
//! block-and-wait already works; only the projection label lags — a documented
//! follow-up).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use super::db::JobsDB;
use super::types::JobKind;
use crate::tools::approval::{BackgroundApprovalBridge, BackgroundApprovalScope};

/// `job_id → request_id` for jobs currently parked on an approval. Populated
/// when a job parks (`on_park`), cleared when it resumes (`on_resume`). Read by
/// `cancel_job` so cancelling a parked job can dismiss its approval dialog. Tiny
/// (only parked jobs); a plain `std::sync::Mutex` since every access is sync.
static PARKED_JOB_REQUESTS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Install the approval bridge on the current job-runner thread for `job_id`.
/// Returns a [`BackgroundApprovalScope`] RAII guard the caller holds for the
/// whole dispatch; dropping it clears the thread-local so the thread can't leak
/// a stale bridge.
pub(crate) fn install(
    db: Arc<JobsDB>,
    job_id: String,
    tool_name: String,
    session_id: Option<String>,
) -> BackgroundApprovalScope {
    let park_db = db.clone();
    let park_job = job_id.clone();
    let park_tool = tool_name.clone();
    let park_session = session_id.clone();
    let on_park = Box::new(move |request_id: &str| {
        // Record first so a cancel racing the park can always find the request
        // id to dismiss. Harmless if the DB flip below no-ops.
        PARKED_JOB_REQUESTS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(park_job.clone(), request_id.to_string());
        match park_db.mark_awaiting_approval(&park_job) {
            Ok(true) => {
                super::events::emit_updated(
                    &park_job,
                    JobKind::Tool,
                    &park_tool,
                    super::types::JobStatus::AwaitingApproval.as_str(),
                    park_session.as_deref(),
                );
                app_info!(
                    "async_jobs",
                    "approval",
                    "Background job {} parked awaiting approval (request {})",
                    park_job,
                    request_id
                );
            }
            // Not running (already cancelled / settled) — leave it; the runner
            // will settle it terminal. Don't force it into awaiting_approval.
            Ok(false) => {}
            Err(e) => app_warn!(
                "async_jobs",
                "approval",
                "Failed to park job {} awaiting approval: {}",
                park_job,
                e
            ),
        }
    });

    let resume_db = db;
    let resume_job = job_id.clone();
    let resume_tool = tool_name;
    let resume_session = session_id;
    let on_resume = Box::new(move |origin: Option<crate::tools::approval::ApprovalOrigin>| {
        // Take the recorded request id (also the cancel-dismiss key below).
        let request_id = PARKED_JOB_REQUESTS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&resume_job);
        // Guarded `awaiting_approval → running`: a no-op (false) if a concurrent
        // cancel already moved the row to cancelling/terminal, so we never
        // clobber a cancel back to running.
        match resume_db.resume_from_awaiting_approval(&resume_job) {
            Ok(true) => {
                // F6 audit: correct the placeholder origin recorded at spawn (the
                // command gate had not run yet) with the real decision.
                if let Some(o) = origin {
                    let _ = resume_db.set_approval_origin(&resume_job, o.as_str());
                }
                super::events::emit_updated(
                    &resume_job,
                    JobKind::Tool,
                    &resume_tool,
                    super::types::JobStatus::Running.as_str(),
                    resume_session.as_deref(),
                );
            }
            Ok(false) => {}
            Err(e) => app_warn!(
                "async_jobs",
                "approval",
                "Failed to resume job {} after approval: {}",
                resume_job,
                e
            ),
        }
        // R8: dismiss the orphaned dialog IFF the job was cancelled while parked
        // — `dismiss_parked_job_approval` removes the pending entry only if it is
        // still present (a no-op + no broadcast for approve/deny/timeout, which
        // already cleared it). Runs after the dispatch future was dropped by the
        // cancel, so it cannot race the `select!` into a spurious completion.
        if let Some(request_id) = request_id {
            crate::tools::approval::dismiss_parked_job_approval(
                &request_id,
                resume_session.as_deref(),
            );
        }
    });

    BackgroundApprovalScope::new(BackgroundApprovalBridge { on_park, on_resume })
}

/// Forget any parked-approval record for `job_id` (terminal cleanup). Idempotent.
pub(crate) fn forget(job_id: &str) {
    PARKED_JOB_REQUESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(job_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_jobs::types::{BackgroundJob, JobStatus};
    use crate::tools::approval::{
        test_drive_bridge_park, test_drive_bridge_resume, ApprovalOrigin,
    };

    fn running_job(id: &str) -> BackgroundJob {
        BackgroundJob {
            job_id: id.to_string(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: Some("s1".into()),
            agent_id: None,
            tool_name: "exec".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: 0,
            completed_at: None,
            injected: false,
            origin: "explicit".into(),
            // Spawn-time placeholder (the deferred command gate hasn't run yet).
            approval_origin: Some("policy_allow".into()),
            incognito: false,
            pid: None,
            cancel_requested: false,
        }
    }

    // `PARKED_JOB_REQUESTS` is a process-global shared by every test, so each
    // test uses a unique job id and asserts only on its own key (cargo runs
    // tests in parallel).
    fn is_parked(job_id: &str) -> bool {
        PARKED_JOB_REQUESTS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(job_id)
    }

    #[test]
    fn install_parks_then_resumes_a_real_job_row_with_origin_correction() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(JobsDB::open(&dir.path().join("background_jobs.db")).unwrap());
        db.insert(&running_job("park-resume-j")).unwrap();

        let scope = install(
            db.clone(),
            "park-resume-j".into(),
            "exec".into(),
            Some("s1".into()),
        );

        // The runner's dispatch hits an attended approval → park.
        test_drive_bridge_park("req-1");
        assert_eq!(
            db.load("park-resume-j").unwrap().unwrap().status,
            JobStatus::AwaitingApproval,
            "park flips the real row to AwaitingApproval"
        );
        assert!(is_parked("park-resume-j"), "request id recorded for cancel-dismiss");

        // User approves → resume to Running, placeholder origin corrected.
        test_drive_bridge_resume(Some(ApprovalOrigin::User));
        let loaded = db.load("park-resume-j").unwrap().unwrap();
        assert_eq!(loaded.status, JobStatus::Running);
        assert_eq!(
            loaded.approval_origin.as_deref(),
            Some("user"),
            "F6: real decision corrects the spawn-time placeholder"
        );
        assert!(!is_parked("park-resume-j"), "resume clears the parked record");
        drop(scope);
    }

    #[test]
    fn resume_does_not_clobber_a_concurrent_cancel() {
        // R8: if a cancel moved the parked row to `cancelling` before the resume
        // guard fires (cancel-drop), `resume_from_awaiting_approval` must no-op so
        // the cancel is not reverted back to running.
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(JobsDB::open(&dir.path().join("background_jobs.db")).unwrap());
        db.insert(&running_job("cancel-clobber-j")).unwrap();
        let scope = install(
            db.clone(),
            "cancel-clobber-j".into(),
            "exec".into(),
            Some("s1".into()),
        );

        test_drive_bridge_park("req-1");
        // Simulate cancel_job: awaiting_approval -> cancelling.
        assert!(db
            .mark_cancelling("cancel-clobber-j", Some("Cancellation requested"))
            .unwrap());

        // Cancel drops the dispatch future → resume guard fires with origin=None.
        test_drive_bridge_resume(None);
        assert_eq!(
            db.load("cancel-clobber-j").unwrap().unwrap().status,
            JobStatus::Cancelling,
            "resume must not revert a cancel back to running"
        );
        assert!(
            !is_parked("cancel-clobber-j"),
            "parked record cleared on resume regardless"
        );
        drop(scope);
    }
}

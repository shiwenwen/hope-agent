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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::db::JobsDB;
use super::types::JobKind;
use crate::tools::approval::{BackgroundApprovalBridge, BackgroundApprovalScope};

/// Per-job-thread accounting of time spent parked on a human approval, so the
/// job's execution budget (`max_job_secs`) can EXCLUDE the approval wait
/// (ASYNC-2: the human wait is not execution time). Lives on the job-runner
/// thread — the same thread as the bridge's park/resume callbacks AND the budget
/// timer in `run_tool_once`. On any thread without a bridge (e.g. the
/// auto-background worker) it stays at its default (no extension), so the timer
/// there is unchanged.
struct ParkTiming {
    accum: Duration,
    park_start: Option<Instant>,
}

thread_local! {
    static PARK_TIMING: RefCell<ParkTiming> =
        const { RefCell::new(ParkTiming { accum: Duration::ZERO, park_start: None }) };
}

/// Reset the current job thread's park accounting. Called once at the start of
/// each job's dispatch so a (theoretical) thread reuse can't leak stale timing.
pub(crate) fn reset_park_timing() {
    PARK_TIMING.with(|t| {
        *t.borrow_mut() = ParkTiming {
            accum: Duration::ZERO,
            park_start: None,
        };
    });
}

fn park_timing_enter() {
    PARK_TIMING.with(|t| {
        let mut t = t.borrow_mut();
        if t.park_start.is_none() {
            t.park_start = Some(Instant::now());
        }
    });
}

fn park_timing_exit() {
    PARK_TIMING.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(start) = t.park_start.take() {
            t.accum += start.elapsed();
        }
    });
}

/// Total time the current job thread has spent parked on approval so far,
/// INCLUDING an in-progress park. The budget timer in `run_tool_once` adds this
/// to the job's deadline so the human wait never counts against `max_job_secs`
/// (and, because an ongoing park keeps growing this value, the timer never fires
/// the timeout WHILE the job is parked).
pub(crate) fn parked_budget_extension() -> Duration {
    PARK_TIMING.with(|t| {
        let t = t.borrow();
        t.accum + t.park_start.map(|s| s.elapsed()).unwrap_or_default()
    })
}

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
        // Start excluding this approval wait from the job's execution budget.
        park_timing_enter();
        // Record next so a cancel racing the park can always find the request
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
    let on_resume = Box::new(
        move |origin: Option<crate::tools::approval::ApprovalOrigin>| {
            // Stop excluding (fold this park's duration into the accumulated total
            // the budget timer adds to the deadline).
            park_timing_exit();
            // Take the recorded request id (also the cancel-dismiss key below).
            let request_id = PARKED_JOB_REQUESTS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&resume_job);
            // Revert to Running ONLY on a proceed outcome (`origin = Some`: approve /
            // timeout-proceed) — that is when the dispatch actually continues and the
            // row should show Running again. On deny / timeout-deny / cancel-drop
            // (`origin = None`) the dispatch does NOT continue: leave the row
            // `awaiting_approval` and let the terminal settle take it straight to its
            // terminal state (`update_terminal` accepts `awaiting_approval`). Emitting
            // a `job:updated(Running)` here would broadcast a spurious "running" for a
            // job that never resumed (UI flicker + wrong event for raw `job:*`
            // consumers). Guarded `awaiting_approval → running` is also a no-op if a
            // concurrent cancel already moved the row to cancelling/terminal.
            //
            // Accepted limitation (audit): a DENIED / timeout-denied parked job keeps
            // the spawn-time placeholder `approval_origin` (e.g. `policy_allow`) — the
            // F6 correction only runs on a proceed. The denial is authoritative in the
            // terminal `error` column (DeniedByUser → "denied by user"), and the job
            // never ran, so the audit origin of a non-executed Failed job is moot.
            if let Some(o) = origin {
                match resume_db.resume_from_awaiting_approval(&resume_job) {
                    Ok(true) => {
                        // F6 audit: correct the placeholder origin recorded at spawn
                        // (the command gate had not run yet) with the real decision.
                        let _ = resume_db.set_approval_origin(&resume_job, o.as_str());
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
        },
    );

    BackgroundApprovalScope::new(BackgroundApprovalBridge { on_park, on_resume })
}

/// The approval `request_id` a job is currently parked on, if any. Used by
/// `cancel_job` to proactively dismiss the orphaned dialog the instant a parked
/// job is cancelled (rather than waiting for the dispatch future to drop) — which
/// also drops the pending sender, waking the parked `rx.await` so the job winds
/// down within the cancel grace instead of dead-waiting it, and closes the window
/// where an Allow click during that grace would run the just-cancelled command.
pub(crate) fn parked_request_id(job_id: &str) -> Option<String> {
    PARKED_JOB_REQUESTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(job_id)
        .cloned()
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
        assert!(
            is_parked("park-resume-j"),
            "request id recorded for cancel-dismiss"
        );

        // User approves → resume to Running, placeholder origin corrected.
        test_drive_bridge_resume(Some(ApprovalOrigin::User));
        let loaded = db.load("park-resume-j").unwrap().unwrap();
        assert_eq!(loaded.status, JobStatus::Running);
        assert_eq!(
            loaded.approval_origin.as_deref(),
            Some("user"),
            "F6: real decision corrects the spawn-time placeholder"
        );
        assert!(
            !is_parked("park-resume-j"),
            "resume clears the parked record"
        );
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

    #[test]
    fn deny_leaves_row_awaiting_without_a_spurious_running_emit() {
        // R8 review fix B: on a DENY (origin=None) the resume must NOT revert the
        // row to running (which would broadcast a spurious job:updated{running}
        // for a job that never resumed). The row stays `awaiting_approval` and the
        // terminal settle (update_terminal, which accepts awaiting_approval) takes
        // it straight to Failed.
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(JobsDB::open(&dir.path().join("background_jobs.db")).unwrap());
        db.insert(&running_job("deny-j")).unwrap();
        let scope = install(
            db.clone(),
            "deny-j".into(),
            "exec".into(),
            Some("s1".into()),
        );

        test_drive_bridge_park("req-1");
        assert_eq!(
            db.load("deny-j").unwrap().unwrap().status,
            JobStatus::AwaitingApproval
        );
        // Deny → guard drops with origin=None.
        test_drive_bridge_resume(None);
        assert_eq!(
            db.load("deny-j").unwrap().unwrap().status,
            JobStatus::AwaitingApproval,
            "deny must leave the row awaiting (no revert to running)"
        );
        assert!(!is_parked("deny-j"), "parked record cleared on resume");
        // The real terminal settle then takes the still-parked row to Failed.
        assert!(db
            .update_terminal("deny-j", JobStatus::Failed, None, None, Some("denied"), 1)
            .unwrap());
        assert_eq!(
            db.load("deny-j").unwrap().unwrap().status,
            JobStatus::Failed
        );
        drop(scope);
    }

    #[test]
    fn park_timing_excludes_the_human_wait_from_the_budget() {
        // R8 review fix A: the park-timing accounting that the budget timer adds to
        // the job deadline. Runs on the test thread (same thread-local model as a
        // job runner). Reset → 0; an in-progress park is counted live; after exit
        // the accumulated value is fixed and does not keep growing.
        reset_park_timing();
        assert_eq!(parked_budget_extension(), Duration::ZERO);

        park_timing_enter();
        std::thread::sleep(Duration::from_millis(30));
        let during = parked_budget_extension();
        assert!(
            during >= Duration::from_millis(25),
            "an in-progress park is counted live (got {during:?})"
        );

        park_timing_exit();
        let after = parked_budget_extension();
        assert!(
            after >= during,
            "accumulated park time is retained after exit"
        );
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            parked_budget_extension(),
            after,
            "once resumed, the extension is fixed (post-approval execution gets the full budget)"
        );

        // Clean up the thread-local so a reused test thread starts fresh.
        reset_park_timing();
    }
}

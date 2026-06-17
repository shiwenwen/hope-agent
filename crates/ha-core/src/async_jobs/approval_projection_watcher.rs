//! R8 follow-up: reflect a background subagent's INNER-tool approval on its
//! Background Job projection label.
//!
//! R8 landed `AwaitingApproval` for background `exec` jobs via a thread-local
//! bridge installed on the job's own OS thread ([`super::approval_bridge`]). A
//! background *subagent* doesn't run through that path — it runs its own turns in
//! a child session, so its inner-tool approvals never touch the job thread's
//! thread-local. This watcher closes that gap WITHOUT touching execution: it
//! subscribes to the EventBus and, on an approval being requested / resolved in a
//! subagent's child session, flips that run's projection row
//! `running ⇄ awaiting_approval` so the panel / `job_status` show "等待审批"
//! instead of "运行中", mirroring R8's background-`exec` behaviour.
//!
//! Subscriber path:
//! 1. [`crate::tools::approval::check_and_request_approval`] emits
//!    `approval_required` when an attended approval is requested and
//!    `approval:resolved` when it settles — both carry the requesting session.
//! 2. This watcher maps that session → an active *projected* subagent run via
//!    [`super::JobManager::reflect_subagent_inner_approval`] and parks / resumes
//!    the projection. Non-subagent approvals (foreground, R8's background `exec`
//!    whose approval carries its *parent* session, unprojected internal /
//!    incognito runs) fall through as no-ops.
//!
//! The watcher is **pure projection** — it never gates execution; the inner
//! approval still block-and-waits in the child session exactly as before.

use crate::async_jobs::JobManager;
use crate::tools::approval::{EVENT_APPROVAL_REQUIRED, EVENT_APPROVAL_RESOLVED};

static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Spawn the EventBus subscriber that mirrors subagent inner approvals onto their
/// projection labels. Idempotent — at most one task per process (both `app_init`
/// paths call this; the loser returns immediately). No-op (and re-armable) when
/// the event bus isn't up yet.
pub fn spawn_subagent_approval_projection_watcher() {
    use std::sync::atomic::Ordering;
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(bus) = crate::globals::get_event_bus() else {
        // Bus not initialised yet (mainly unit-test contexts); let a later call retry.
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    crate::app_warn!(
                        "async_jobs",
                        "approval_projection",
                        "Lagged {} EventBus events; some subagent approval labels may lag",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            let parked = if event.name == EVENT_APPROVAL_REQUIRED {
                true
            } else if event.name == EVENT_APPROVAL_RESOLVED {
                false
            } else {
                continue;
            };

            // `approval_required` carries `session_id` (snake_case, serialized
            // `ApprovalRequest`); `approval:resolved` carries `sessionId`
            // (camelCase, hand-built). Accept either spelling.
            let Some(session_id) = event
                .payload
                .get("session_id")
                .or_else(|| event.payload.get("sessionId"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };

            JobManager::reflect_subagent_inner_approval(session_id, parked);
        }
    });
}

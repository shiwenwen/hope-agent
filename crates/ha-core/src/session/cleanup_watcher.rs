//! `session:deleted` / `session:purged` watcher — fans a session-lifecycle
//! event out to every in-memory subsystem holding a reference to the session,
//! so deleting/purging a session triggers coordinated cleanup instead of
//! leaking:
//!   - pending approvals → deny + broadcast resolved (A-9)
//!   - async jobs → cancel running/awaiting (A-8)
//!   - IM `TEXT_PENDING` → drop the session's stack (A-9)
//!   - live turn → cancel (A-9)
//!   - per-session allowlist rules → clear (A-9)
//!
//! Mirrors [`crate::channel::worker::eviction_watcher`]: one EventBus
//! subscriber, name-filtered, each fan-out step best-effort so a single failing
//! subsystem can't block the rest.
//!
//! Spawned from both `start_background_tasks` and
//! `start_minimal_background_tasks` tier-agnostic sections. It must NOT live
//! inside `spawn_channel_listeners` — server / ACP have no channel registry but
//! still delete sessions and need this cleanup.

use crate::session::events::{session_event_keys, EVENT_SESSION_DELETED, EVENT_SESSION_PURGED};

/// Spawn the EventBus subscriber that cleans up in-memory state when a session
/// is deleted or purged. No-op when the event bus isn't initialised yet (e.g.
/// unit-test contexts; desktop / server / ACP bring the bus up first).
pub fn spawn_session_cleanup_watcher() {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let mut rx = bus.subscribe();

    tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Loud: a dropped session:deleted/purged means that session's
                    // cleanup never runs — pending approvals stay hung, background
                    // jobs keep running, and (purge) incognito artifacts aren't
                    // scrubbed. The fan-out below is spawned off this loop to keep
                    // this drain fast and make lagging effectively impossible.
                    app_error!(
                        "session",
                        "cleanup_watcher",
                        "Lagged {} EventBus event(s); those session cleanups are missed (approvals left hung / jobs uncancelled / incognito artifacts unscrubbed)",
                        n
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            };

            if event.name != EVENT_SESSION_DELETED && event.name != EVENT_SESSION_PURGED {
                continue;
            }

            let Some(session_id) = event
                .payload
                .get(session_event_keys::SESSION_ID)
                .and_then(|v| v.as_str())
            else {
                app_warn!(
                    "session",
                    "cleanup_watcher",
                    "{} payload missing sessionId: {}",
                    event.name,
                    event.payload
                );
                continue;
            };

            // Purge (incognito burn-on-close) additionally scrubs on-disk
            // artifacts that a plain delete leaves to age-based GC.
            let is_purge = event.name == EVENT_SESSION_PURGED;
            // Run the fan-out OFF the receive loop: cleanup_session does several
            // DB queries + global-lock scans, and awaiting it inline would let a
            // burst of deletes back up the broadcast buffer and trigger Lagged
            // (dropping later cleanups). Each step is best-effort and idempotent
            // per subsystem, so concurrent runs for distinct sessions are safe.
            let sid = session_id.to_string();
            tokio::spawn(async move {
                cleanup_session(&sid, is_purge).await;
            });
        }
    });
}

/// Fan out cleanup for one removed session. Each step is best-effort and
/// independent so a failure in one subsystem can't block the rest. When
/// `is_purge` (incognito burn-on-close), also physically scrub on-disk
/// artifacts (tool-result spills + async-job rows/spool) so the burned session
/// leaves no trace — a plain delete leaves these to age-based GC. Epic E.
async fn cleanup_session(session_id: &str, is_purge: bool) {
    // A-8: cancel active / awaiting-approval background jobs (DELETE-4).
    let cancelled_jobs = crate::async_jobs::cancel_jobs_for_session(session_id);

    // A-9: deny + resolve pending approvals so a blocked tool turn unblocks and
    // every surface dismisses its dialog (DELETE-1 / INCOG-4).
    let denied = crate::tools::deny_pending_for_session(
        session_id,
        crate::tools::ApprovalResolutionSource::SessionDeleted,
    )
    .await;

    // A-9: drop stale IM text-reply approval state for this session (SURFACE-2).
    crate::channel::worker::approval::drop_pending_for_session(session_id).await;

    // A-9: clear per-session allowlist rules so they don't linger (INCOG-7).
    crate::permission::allowlist::clear_session_rules(session_id);

    // A-9: live-cancel any in-flight turn (DELETE-5 / INCOG-1 in-flight turn).
    if let Some(snapshot) = crate::chat_engine::active_turn::current(session_id) {
        snapshot
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    // E3/E4 (INCOG-2/5): on incognito burn, scrub the session's on-disk
    // artifacts. Incognito tool results / job spools are skipped at write time,
    // so these are backstops — but they also drop the (redacted) async-job rows
    // and any pre-flag spills so the burned session leaves nothing behind. A
    // plain delete leaves these to age-based retention; only purge scrubs now.
    if is_purge {
        crate::tools::purge_tool_results_for_session(session_id);
        crate::async_jobs::purge_jobs_for_session(session_id);
    }

    if cancelled_jobs > 0 || denied > 0 {
        app_debug!(
            "session",
            "cleanup_watcher",
            "fanned out cleanup for {} (jobs={}, approvals={})",
            session_id,
            cancelled_jobs,
            denied
        );
    }
}

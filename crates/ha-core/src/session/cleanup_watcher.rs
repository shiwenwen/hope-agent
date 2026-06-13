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
                    app_warn!(
                        "session",
                        "cleanup_watcher",
                        "Lagged {} EventBus events; some session cleanups may be missed",
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

            cleanup_session(session_id);
        }
    });
}

/// Fan out cleanup for one removed session. Each step is best-effort and
/// independent so a failure in one subsystem can't block the rest.
///
/// Fan-out targets are stubbed for now — wired by later Epic A subtasks:
///   - A-8: `async_jobs::cancel_jobs_for_session`
///   - A-9: approval `deny_pending_for_session` / channel
///     `drop_pending_for_session` / allowlist `clear_session_rules` /
///     `active_turn` live-cancel
fn cleanup_session(session_id: &str) {
    // TODO(A-8/A-9): wire real fan-out. Logged for now to confirm end-to-end
    // event delivery while the cleanup targets land.
    app_debug!(
        "session",
        "cleanup_watcher",
        "session lifecycle event for {} — fan-out pending (A-8/A-9)",
        session_id
    );
}

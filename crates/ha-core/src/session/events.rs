//! Session lifecycle domain events.
//!
//! Emitted when a session is deleted (explicit user action / startup orphan
//! sweep) or purged (incognito burn-on-close). Subscribed by
//! [`crate::session::cleanup_watcher`], which fans the event out to every
//! in-memory subsystem holding a reference to the session (pending approvals,
//! async jobs, IM `TEXT_PENDING`, live turns, per-session allowlist rules) so a
//! delete/purge triggers coordinated cleanup instead of leaking.
//!
//! Mirrors the [`crate::channel::db`] eviction-event pattern: payload key names
//! live in [`session_event_keys`] and are shared between the emit site
//! (`session::db`) and the subscriber so a rename can't drift the two halves
//! out of sync.

use super::SessionMeta;

/// One event per deleted session (explicit user delete / startup orphan sweep).
pub const EVENT_SESSION_DELETED: &str = "session:deleted";

/// One event per purged incognito session (burn-on-close). Physically distinct
/// from [`EVENT_SESSION_DELETED`] so audit and subscribers can tell a normal
/// delete from an incognito purge.
pub const EVENT_SESSION_PURGED: &str = "session:purged";

/// JSON payload keys for [`EVENT_SESSION_DELETED`] / [`EVENT_SESSION_PURGED`].
/// Shared between the emit site (`session::db`) and the subscriber
/// (`session::cleanup_watcher`) so a rename can't drift the two halves apart.
pub mod session_event_keys {
    pub const SESSION_ID: &str = "sessionId";
    pub const AGENT_ID: &str = "agentId";
    pub const INCOGNITO: &str = "incognito";
    pub const REASON: &str = "reason";
}

/// Why a session row was removed. Carried in the event payload so subscribers
/// and audit can distinguish the trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDeleteReason {
    /// Explicit user / API delete.
    UserDelete,
    /// Incognito burn-on-close purge (navigated away from an incognito chat).
    IncognitoPurge,
    /// Startup sweep of orphaned incognito sessions left by a previous run.
    OrphanSweep,
}

impl SessionDeleteReason {
    /// Stable snake_case string for the payload `reason` field and logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserDelete => "user_delete",
            Self::IncognitoPurge => "incognito_purge",
            Self::OrphanSweep => "orphan_sweep",
        }
    }

    /// Whether this reason is an incognito purge — purges emit
    /// [`EVENT_SESSION_PURGED`] instead of [`EVENT_SESSION_DELETED`].
    pub fn is_purge(&self) -> bool {
        matches!(self, Self::IncognitoPurge)
    }
}

/// Emit the appropriate session-lifecycle event for a removed session.
///
/// `meta` must be the **pre-delete** snapshot — by emit time the row is gone,
/// so callers capture it (via `get_session`) before deleting. No-op when no
/// event bus is registered (e.g. early startup / tests).
pub fn emit_session_deleted(meta: &SessionMeta, reason: SessionDeleteReason) {
    let Some(bus) = crate::globals::get_event_bus() else {
        return;
    };
    let name = if reason.is_purge() {
        EVENT_SESSION_PURGED
    } else {
        EVENT_SESSION_DELETED
    };
    bus.emit(
        name,
        serde_json::json!({
            session_event_keys::SESSION_ID: meta.id,
            session_event_keys::AGENT_ID: meta.agent_id,
            session_event_keys::INCOGNITO: meta.incognito,
            session_event_keys::REASON: reason.as_str(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_as_str_is_stable() {
        assert_eq!(SessionDeleteReason::UserDelete.as_str(), "user_delete");
        assert_eq!(
            SessionDeleteReason::IncognitoPurge.as_str(),
            "incognito_purge"
        );
        assert_eq!(SessionDeleteReason::OrphanSweep.as_str(), "orphan_sweep");
    }

    #[test]
    fn only_incognito_purge_reports_is_purge() {
        assert!(SessionDeleteReason::IncognitoPurge.is_purge());
        assert!(!SessionDeleteReason::UserDelete.is_purge());
        assert!(!SessionDeleteReason::OrphanSweep.is_purge());
    }
}

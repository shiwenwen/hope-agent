//! Unattended-approval surface detection (Epic D, DEADLOCK-1..5).
//!
//! When the permission engine decides a tool needs an `Ask`, the approval
//! blocks waiting for a human to click Allow/Deny. On some entries **no human
//! can ever respond**: a cron run firing at 3am, a headless `server` with no
//! web client and no IM-attached chat, an ACP editor that never declared a
//! permission capability, or a subagent whose parent chain has no surface.
//! Historically those turns hung forever (or a generic whole-job timeout masked
//! the real cause). This module decides, *before* blocking, whether the current
//! turn has any approval surface at all, so [`crate::tools::approval`] can
//! fail-closed (or auto-proceed, per config) with a structured reason instead.
//!
//! ## Conservative red line
//!
//! Return [`ApprovalSurface::Unattended`] **only when we are certain no human
//! can approve**. Any plausible surface (desktop window, connected web client,
//! IM-attached chat) yields [`ApprovalSurface::Attended`] so a legitimate
//! interactive approval is never silently denied. The one deliberate exception
//! is **cron**: cron sessions are excluded from the desktop's interactive
//! approval prompt (it filters by the current session id), so a cron approval
//! has no reliable interactive surface even on desktop — cron is treated as
//! unattended regardless, matching the DEADLOCK-4 recommendation. Users who
//! want privileged cron/headless runs set `unattendedApprovalAction = proceed`
//! or give that agent YOLO / `auto_approve_tools`.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the ACP (`hope-agent acp`) client declared a permission capability
/// it can use to surface approvals. Default `false` → ACP approvals are
/// unattended (fail-closed) until the client advertises one. Set by the ACP
/// `do_initialize` handler (D7). Irrelevant outside ACP mode.
static ACP_PERMISSION_CAPABLE: AtomicBool = AtomicBool::new(false);

/// Record whether the connected ACP client can surface permission requests.
/// Called from the ACP initialize handler (D7); no-op effect outside ACP mode
/// because [`evaluate_approval_surface`] only reads it when [`crate::app_init::is_acp`].
pub fn set_acp_permission_capable(capable: bool) {
    ACP_PERMISSION_CAPABLE.store(capable, Ordering::SeqCst);
}

fn acp_permission_capable() -> bool {
    ACP_PERMISSION_CAPABLE.load(Ordering::SeqCst)
}

/// Why a turn has no human who can answer an approval prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnattendedReason {
    /// Scheduled cron run — isolated session, no synchronous watcher, and
    /// excluded from the desktop's interactive approval prompt.
    Cron,
    /// Headless `server` (or non-desktop) with no connected web client and no
    /// IM-attached chat — the `approval_required` broadcast reaches nobody.
    HeadlessNoClient,
    /// ACP stdio bridge whose client never declared a permission capability,
    /// so there is no channel to forward the approval over.
    AcpNoPermissionCapability,
    /// Subagent whose parent chain exposes no surface (headless parent, or a
    /// cron/agent root) — the child approval can't bubble anywhere visible.
    SubagentNoParentSurface,
}

impl UnattendedReason {
    /// Stable snake_case tag for logs / audit / the model-facing reason string.
    pub fn as_str(self) -> &'static str {
        match self {
            UnattendedReason::Cron => "cron_unattended",
            UnattendedReason::HeadlessNoClient => "headless_no_client",
            UnattendedReason::AcpNoPermissionCapability => "acp_no_permission_capability",
            UnattendedReason::SubagentNoParentSurface => "subagent_no_parent_surface",
        }
    }

    /// One-line human explanation embedded in the fail-closed tool result so
    /// the model (and the operator reading logs) understands why it was denied.
    pub fn explain(self) -> &'static str {
        match self {
            UnattendedReason::Cron => {
                "this is a scheduled cron run with no one watching to approve it"
            }
            UnattendedReason::HeadlessNoClient => {
                "this is a headless server turn with no connected client and no IM chat to approve it"
            }
            UnattendedReason::AcpNoPermissionCapability => {
                "the ACP client did not advertise a permission capability, so approvals cannot be shown"
            }
            UnattendedReason::SubagentNoParentSurface => {
                "this subagent's parent conversation has no surface that can show an approval"
            }
        }
    }
}

/// Result of the surface check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSurface {
    /// A human can plausibly respond — proceed with the normal approval prompt.
    Attended,
    /// No human can respond — caller applies `unattendedApprovalAction`.
    Unattended(UnattendedReason),
}

/// Decide whether the turn owning `session_id` has any approval surface.
///
/// Only reads global runtime state + the session row (+ the channel attach
/// table); cheap enough to run on the rare approval path. See the module-level
/// conservative red line.
pub fn evaluate_approval_surface(session_id: Option<&str>) -> ApprovalSurface {
    use ApprovalSurface::{Attended, Unattended};

    let meta = session_id.and_then(load_session_meta);

    // 1. Cron — unattended by definition (see module note), regardless of any
    //    desktop window, because cron sessions never reach the interactive prompt.
    if meta.as_ref().is_some_and(|m| m.is_cron) {
        return Unattended(UnattendedReason::Cron);
    }

    // 2. Subagent child session: the approval has to bubble to its parent chain.
    if meta
        .as_ref()
        .and_then(|m| m.parent_session_id.as_deref())
        .is_some()
    {
        // A desktop window / connected web client surfaces child approvals via
        // OS notification + the child-session badge (and D6 parent bubbling), so
        // the user can still reach them — only a fully headless parent leaves it
        // unreachable.
        if crate::app_init::desktop_client_present() {
            return Attended;
        }
        if subagent_chain_has_im_surface(meta.as_ref()) {
            return Attended;
        }
        return Unattended(UnattendedReason::SubagentNoParentSurface);
    }

    // 3. Top-level turn. IM-attached chat → the IM user can approve via buttons.
    if let Some(sid) = session_id {
        if session_is_im_attached(sid, meta.as_ref()) {
            return Attended;
        }
    }

    // 4. Desktop window or connected web client present.
    if crate::app_init::desktop_client_present() {
        return Attended;
    }

    // 5. ACP stdio bridge — attended only if the client advertised a capability.
    if crate::app_init::is_acp() {
        return if acp_permission_capable() {
            Attended
        } else {
            Unattended(UnattendedReason::AcpNoPermissionCapability)
        };
    }

    // 6. Headless server / non-desktop with no client and no IM chat.
    Unattended(UnattendedReason::HeadlessNoClient)
}

fn load_session_meta(session_id: &str) -> Option<crate::session::SessionMeta> {
    crate::get_session_db()
        .and_then(|db| db.get_session(session_id).ok().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattended_reason_tags_are_distinct_and_nonempty() {
        let all = [
            UnattendedReason::Cron,
            UnattendedReason::HeadlessNoClient,
            UnattendedReason::AcpNoPermissionCapability,
            UnattendedReason::SubagentNoParentSurface,
        ];
        let tags: Vec<&str> = all.iter().map(|r| r.as_str()).collect();
        for r in all {
            assert!(!r.as_str().is_empty());
            assert!(!r.explain().is_empty());
        }
        // All tags unique (used as stable audit/log keys).
        let mut sorted = tags.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), tags.len(), "reason tags must be unique");
    }

    #[test]
    fn no_session_no_client_is_headless_unattended() {
        // In ha-core unit tests nothing increments the events-WS counter and the
        // runtime role is not "desktop"/"acp", so desktop_client_present() is
        // false and a session-less approval has no surface.
        assert!(!crate::app_init::is_desktop());
        assert!(!crate::app_init::is_acp());
        assert_eq!(
            evaluate_approval_surface(None),
            ApprovalSurface::Unattended(UnattendedReason::HeadlessNoClient)
        );
    }

    #[test]
    fn acp_capability_toggle_flips_acp_surface() {
        // Pure toggle round-trip of the D7 capability flag (independent of mode).
        set_acp_permission_capable(true);
        assert!(acp_permission_capable());
        set_acp_permission_capable(false);
        assert!(!acp_permission_capable());
    }
}

/// True iff `session_id` is currently attached to an IM channel conversation
/// (the authoritative 1:1 attach table is the source of truth; falls back to the
/// denormalized `channel_info` on the session row if the channel DB is absent).
fn session_is_im_attached(session_id: &str, meta: Option<&crate::session::SessionMeta>) -> bool {
    if let Some(db) = crate::get_channel_db() {
        if let Ok(Some(_conv)) = db.get_conversation_by_session(session_id) {
            return true;
        }
        // channel DB present but no row → genuinely not attached.
        return meta.is_some_and(|m| m.channel_info.is_some());
    }
    meta.is_some_and(|m| m.channel_info.is_some())
}

/// Walk a subagent's parent chain looking for an IM-attached ancestor whose
/// user could answer a bubbled approval. Bounded so a corrupt parent cycle
/// can't loop forever; a cron ancestor ends the walk (cron is never a surface).
fn subagent_chain_has_im_surface(child: Option<&crate::session::SessionMeta>) -> bool {
    const MAX_DEPTH: usize = 8;
    let Some(db) = crate::get_session_db() else {
        return false;
    };
    let mut next_parent = child.and_then(|m| m.parent_session_id.clone());
    for _ in 0..MAX_DEPTH {
        let Some(parent_id) = next_parent.take() else {
            return false;
        };
        let Ok(Some(parent)) = db.get_session(&parent_id) else {
            return false;
        };
        if parent.is_cron {
            return false;
        }
        if session_is_im_attached(&parent_id, Some(&parent)) {
            return true;
        }
        next_parent = parent.parent_session_id.clone();
    }
    false
}

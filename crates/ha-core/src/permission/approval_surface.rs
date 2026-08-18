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
//! is **cron**: an occurrence fires on its own schedule with nobody watching it,
//! so it is treated as unattended even on desktop, matching the DEADLOCK-4
//! recommendation. The signal is the live turn (see [`session_runs_cron_turn`]),
//! not the Session row — a scheduled run now lives in an ordinary chat, and the
//! user's own later turns there stay attended. Users who
//! want privileged cron/headless runs set `unattendedApprovalAction = proceed`
//! or give that agent YOLO / `auto_approve_tools`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Whether the ACP (`hope-agent acp`) client declared a permission capability
/// it can use to surface approvals. Default `false` → ACP approvals are
/// unattended (fail-closed) until the client advertises one. Set by the ACP
/// `do_initialize` handler (D7). Irrelevant outside ACP mode.
static ACP_PERMISSION_CAPABLE: AtomicBool = AtomicBool::new(false);

/// Verified first-party HTTP UI turns remain reachable after their initiating
/// request returns: pending approvals are durable for same-process reload and
/// the browser can reconnect to `/ws/events`. Track that capability per
/// session; a process-wide boolean would incorrectly make unrelated headless
/// automation look attended.
static REATTACHABLE_UI_SESSIONS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn reattachable_ui_sessions() -> &'static Mutex<HashMap<String, usize>> {
    REATTACHABLE_UI_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug)]
pub struct ReattachableUiSessionGuard {
    session_id: String,
    released: bool,
}

impl Clone for ReattachableUiSessionGuard {
    fn clone(&self) -> Self {
        register_reattachable_ui_session(&self.session_id)
    }
}

impl ReattachableUiSessionGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut sessions = reattachable_ui_sessions()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = sessions.get_mut(&self.session_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                sessions.remove(&self.session_id);
            }
        }
        self.released = true;
    }
}

impl Drop for ReattachableUiSessionGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// Mark one server-owned, first-party UI turn as reattachable. This does not
/// approve anything: it only preserves the normal Ask path while the browser
/// is disconnected, so the user can reopen the UI and answer. Cron remains
/// unattended by the earlier hard gate below.
pub fn register_reattachable_ui_session(session_id: &str) -> ReattachableUiSessionGuard {
    let mut sessions = reattachable_ui_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *sessions.entry(session_id.to_string()).or_insert(0) += 1;
    ReattachableUiSessionGuard {
        session_id: session_id.to_string(),
        released: false,
    }
}

/// Propagate a verified first-party UI reachability lease from a parent turn
/// to one background child. The returned guard must live for the child's full
/// queue + execution lifecycle; otherwise a child that outlives the parent
/// turn could be reclassified as unattended before a later approval request.
///
/// This copies only an already-live capability. It never creates an attended
/// surface for an unrelated headless/cron parent.
pub fn register_reattachable_ui_child_session(
    parent_session_id: &str,
    child_session_id: &str,
) -> Option<ReattachableUiSessionGuard> {
    let parent_meta =
        crate::get_session_db().and_then(|db| db.get_session(parent_session_id).ok().flatten());
    if session_runs_cron_turn(parent_session_id, parent_meta.as_ref()) {
        return None;
    }
    if session_has_reattachable_ui_surface(parent_session_id)
        || subagent_chain_has_reattachable_ui_surface(parent_meta.as_ref())
    {
        return Some(register_reattachable_ui_session(child_session_id));
    }
    None
}

fn session_has_reattachable_ui_surface(session_id: &str) -> bool {
    reattachable_ui_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .is_some_and(|count| *count > 0)
}

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
    //    desktop window: nobody is watching a 3am occurrence even when the app is
    //    open, so the decision must be deterministic rather than a hung prompt.
    if session_id.is_some_and(|sid| session_runs_cron_turn(sid, meta.as_ref())) {
        return Unattended(UnattendedReason::Cron);
    }

    // 2. Subagent child session: the approval has to bubble to its parent chain.
    if meta
        .as_ref()
        .and_then(|m| m.parent_session_id.as_deref())
        .is_some()
    {
        // C03: a subagent spawned inside a cron run inherits cron's
        // non-interactivity, so it must be classified BEFORE the desktop
        // short-circuit below. A cron turn has no human at the keyboard even when
        // a desktop window is open, and the cron + subagent sessions are hidden
        // from the sidebar (and useApprovals filters by current session), so the
        // approval dialog would never render — Attended here means a silent hang
        // until the approval timeout. Treating it as Unattended(Cron) applies the
        // fail-closed unattended policy (deny by default) immediately. (The cron
        // root's chain is never IM-attached, so this can't mask a real surface.)
        if subagent_chain_roots_at_cron(meta.as_ref()) {
            return Unattended(UnattendedReason::Cron);
        }
        if session_id.is_some_and(session_has_reattachable_ui_surface)
            || subagent_chain_has_reattachable_ui_surface(meta.as_ref())
        {
            return Attended;
        }
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
        if session_has_reattachable_ui_surface(sid) {
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
    crate::get_session_db().and_then(|db| db.get_session(session_id).ok().flatten())
}

/// Is this session running a scheduled occurrence right now?
///
/// A scheduled run is an ordinary Session (`is_cron = 0`), so cron-ness lives on
/// the turn, not the row: the live turn's `ChatSource::Cron` is the signal, and
/// it is necessarily live while its own tool asks for approval. Turn-scoped on
/// purpose — the user's own later turns in that same chat are Desktop/HTTP and
/// stay attended. `is_cron` still covers legacy hidden cron sessions.
/// (`SessionMeta.origin` is display-only and must never gate permissions.)
fn session_runs_cron_turn(session_id: &str, meta: Option<&crate::session::SessionMeta>) -> bool {
    if meta.is_some_and(|m| m.is_cron) {
        return true;
    }
    crate::chat_engine::active_turn::current(session_id)
        .is_some_and(|turn| turn.source == crate::chat_engine::ChatSource::Cron)
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
        if session_runs_cron_turn(&parent_id, Some(&parent)) {
            return false;
        }
        if session_is_im_attached(&parent_id, Some(&parent)) {
            return true;
        }
        next_parent = parent.parent_session_id.clone();
    }
    false
}

/// A child approval can bubble back to a currently-running first-party HTTP UI
/// parent even while that browser is disconnected. This is session-scoped and
/// bounded like the IM ancestry walk above.
fn subagent_chain_has_reattachable_ui_surface(child: Option<&crate::session::SessionMeta>) -> bool {
    const MAX_DEPTH: usize = 8;
    let Some(db) = crate::get_session_db() else {
        return false;
    };
    let mut next_parent = child.and_then(|meta| meta.parent_session_id.clone());
    for _ in 0..MAX_DEPTH {
        let Some(parent_id) = next_parent.take() else {
            return false;
        };
        if session_has_reattachable_ui_surface(&parent_id) {
            return true;
        }
        let Ok(Some(parent)) = db.get_session(&parent_id) else {
            return false;
        };
        if session_runs_cron_turn(&parent_id, Some(&parent)) {
            return false;
        }
        next_parent = parent.parent_session_id;
    }
    false
}

/// C03: does this subagent's parent chain reach a cron session? A subagent
/// spawned inside a cron run is just as non-interactive as the cron session
/// itself. Live wrapper over the global session DB; the walk logic lives in the
/// pure [`chain_roots_at_cron_with`] so it stays unit-testable without the global.
fn subagent_chain_roots_at_cron(child: Option<&crate::session::SessionMeta>) -> bool {
    let Some(db) = crate::get_session_db() else {
        return false;
    };
    chain_roots_at_cron_with(child.and_then(|m| m.parent_session_id.clone()), |id| {
        db.get_session(id).ok().flatten().map(|parent| {
            (
                session_runs_cron_turn(id, Some(&parent)),
                parent.parent_session_id,
            )
        })
    })
}

/// Pure core of the cron-root walk: starting from `start_parent`, follow the
/// chain via `lookup` (returning `(is_cron, next_parent_id)` for a session id, or
/// `None` if it's gone) and report whether any ancestor is a cron session.
/// Bounded against a corrupt parent cycle, mirroring `subagent_chain_has_im_surface`.
fn chain_roots_at_cron_with<F>(start_parent: Option<String>, mut lookup: F) -> bool
where
    F: FnMut(&str) -> Option<(bool, Option<String>)>,
{
    const MAX_DEPTH: usize = 8;
    let mut next_parent = start_parent;
    for _ in 0..MAX_DEPTH {
        let Some(parent_id) = next_parent.take() else {
            return false;
        };
        let Some((is_cron, grandparent)) = lookup(&parent_id) else {
            return false;
        };
        if is_cron {
            return true;
        }
        next_parent = grandparent;
    }
    false
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
    fn reattachable_ui_guard_preserves_ask_surface_without_a_live_socket() {
        let session_id = format!("reattachable-ui-{}", uuid::Uuid::new_v4());
        assert!(!session_has_reattachable_ui_surface(&session_id));
        let first = register_reattachable_ui_session(&session_id);
        let second = register_reattachable_ui_session(&session_id);
        assert!(session_has_reattachable_ui_surface(&session_id));
        assert_eq!(
            evaluate_approval_surface(Some(&session_id)),
            ApprovalSurface::Attended
        );
        drop(first);
        assert_eq!(
            evaluate_approval_surface(Some(&session_id)),
            ApprovalSurface::Attended,
            "the per-session registration is reference counted"
        );
        drop(second);
        assert!(!session_has_reattachable_ui_surface(&session_id));
    }

    #[test]
    fn reattachable_ui_guard_is_retained_by_background_child() {
        let parent_id = format!("reattachable-parent-{}", uuid::Uuid::new_v4());
        let child_id = format!("reattachable-child-{}", uuid::Uuid::new_v4());
        let parent_guard = register_reattachable_ui_session(&parent_id);
        let child_guard = register_reattachable_ui_child_session(&parent_id, &child_id)
            .expect("an active parent UI lease must propagate to its child");

        drop(parent_guard);
        assert_eq!(
            evaluate_approval_surface(Some(&child_id)),
            ApprovalSurface::Attended,
            "the child must remain reachable after its parent turn completes"
        );

        drop(child_guard);
        assert!(!session_has_reattachable_ui_surface(&child_id));
    }

    #[test]
    fn a_live_scheduled_turn_is_unattended_even_in_an_ordinary_session() {
        // A scheduled run is an ordinary Session (`is_cron = 0`), so cron-ness has
        // to come from the live turn — otherwise the 3am occurrence would wait on
        // an approval dialog instead of deciding fail-closed.
        let session_id = format!("scheduled-run-{}", uuid::Uuid::new_v4());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Cron,
            "turn-cron".to_string(),
            cancel.clone(),
        )
        .expect("acquire scheduled turn");
        assert!(session_runs_cron_turn(&session_id, None));
        assert_eq!(
            evaluate_approval_surface(Some(&session_id)),
            ApprovalSurface::Unattended(UnattendedReason::Cron)
        );
        // A cron parent never lends its UI lease to a background child.
        let child_id = format!("scheduled-child-{}", uuid::Uuid::new_v4());
        let _lease = register_reattachable_ui_session(&session_id);
        assert!(register_reattachable_ui_child_session(&session_id, &child_id).is_none());
        drop(guard);
    }

    #[test]
    fn the_users_own_turn_in_that_chat_stays_attended() {
        // Turn-scoped, not session-scoped: replying in the same chat is a Desktop
        // turn with a human right there, so it must keep the normal Ask path.
        let session_id = format!("scheduled-run-{}", uuid::Uuid::new_v4());
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            "turn-user".to_string(),
            cancel.clone(),
        )
        .expect("acquire user turn");
        assert!(!session_runs_cron_turn(&session_id, None));
        let _lease = register_reattachable_ui_session(&session_id);
        assert_eq!(
            evaluate_approval_surface(Some(&session_id)),
            ApprovalSurface::Attended
        );
        drop(guard);
    }

    #[test]
    fn chain_roots_at_cron_detects_cron_ancestor() {
        // C03: a subagent whose parent chain reaches a cron session is cron-rooted
        // (→ Unattended(Cron), classified before the desktop short-circuit).
        // child -> mid (not cron) -> cron-root.
        let with_cron = |id: &str| -> Option<(bool, Option<String>)> {
            match id {
                "mid" => Some((false, Some("cron-root".into()))),
                "cron-root" => Some((true, None)),
                _ => None,
            }
        };
        assert!(chain_roots_at_cron_with(Some("mid".into()), with_cron));

        // A chain with no cron ancestor is not cron-rooted.
        let no_cron = |id: &str| -> Option<(bool, Option<String>)> {
            match id {
                "mid" => Some((false, Some("top".into()))),
                "top" => Some((false, None)),
                _ => None,
            }
        };
        assert!(!chain_roots_at_cron_with(Some("mid".into()), no_cron));

        // Broken chain (missing parent) → not cron-rooted, no panic.
        assert!(!chain_roots_at_cron_with(Some("ghost".into()), |_| None));

        // A corrupt self-referential cycle terminates at the depth bound.
        assert!(!chain_roots_at_cron_with(Some("a".into()), |_| Some((
            false,
            Some("a".into())
        ))));

        // No parent at all (not a subagent) → not cron-rooted.
        assert!(!chain_roots_at_cron_with(None, |_| Some((true, None))));
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

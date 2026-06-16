use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use crate::process_registry::create_session_id;

// Shared tool state lives in process-global `OnceLock<TokioMutex<…>>` cells
// in this module on purpose: see the concurrency contract on
// [`super::execution::ToolExecContext`]. The tool loop clones the per-call
// context for every concurrent branch, so any mutable state that must be
// observed across concurrent tools or across rounds has to sit outside the
// context struct. Add new shared state here (or in a sibling module) rather
// than reaching for `Mutex<…>` inside `ToolExecContext`.
//
// Per-session permission mode (Default / Smart / Yolo) lives in the SQLite
// `sessions.permission_mode` column and is read into [`ToolExecContext.session_mode`]
// by the agent setup path. The legacy process-global `TOOL_PERMISSION_MODE`
// static was removed in the permission system v2 redesign.

// ── Command Approval System ───────────────────────────────────────

/// Approval request sent to frontend and IM channel approval listeners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub command: String,
    pub cwd: String,
    /// Session ID for correlating with IM channel conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional reason emitted by the permission engine. The frontend
    /// renders a colored banner and disables AllowAlways for strict reasons
    /// (`protected_path` / `dangerous_command`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ApprovalReasonPayload>,
    /// When true, the owning session is incognito — the frontend hides the
    /// AllowAlways button and shows a notice, because a persistent grant would
    /// outlive the burn-on-close and break the no-trace guarantee. The backend
    /// independently forces any AllowAlways to in-memory session scope
    /// ([`crate::permission::allowlist`] `choose_scope`); this is the UX half.
    /// Epic E (INCOG-6). Skipped on the wire when false to keep normal payloads
    /// unchanged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
}

/// Reason payload — flat shape so the frontend can switch on `kind` without
/// running a full enum matcher. Mirrors [`crate::permission::AskReason`] but
/// strips internal struct fields the UI doesn't need.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApprovalReasonPayload {
    pub kind: ApprovalReasonKind,
    /// Human-readable detail (matched pattern, path, rationale…). Optional —
    /// `edit_tool` carries no extra detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// KEEP IN SYNC with the TS string union in
/// [`src/components/chat/ApprovalDialog.tsx`] (`ApprovalRequest.reason.kind`).
/// Adding a variant here without updating that union leaves the frontend
/// without a banner — TS won't catch the drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReasonKind {
    EditTool,
    EditCommand,
    DangerousCommand,
    ProtectedPath,
    AgentCustomList,
    SmartJudge,
    BrowserEvaluate,
    MacControlAction,
    MacControlDangerousAction,
    PlanModeAsk,
}

impl ApprovalReasonKind {
    /// Whether this is a **strict** reason: it always demands a per-call human
    /// decision, bars AllowAlways, and must never auto-proceed unattended (Epic
    /// F / TIMEOUT-1 — a strict approval that times out is force-denied even when
    /// `approval_timeout_action=proceed`).
    ///
    /// KEEP IN SYNC with [`crate::permission::AskReason::forbids_allow_always`],
    /// the canonical source — `reason_kind_is_strict_matches_ask_reason` asserts
    /// the two agree across every variant.
    pub fn is_strict(self) -> bool {
        matches!(
            self,
            Self::ProtectedPath
                | Self::DangerousCommand
                | Self::MacControlDangerousAction
                | Self::PlanModeAsk
        )
    }
}

impl From<&crate::permission::AskReason> for ApprovalReasonPayload {
    fn from(value: &crate::permission::AskReason) -> Self {
        use crate::permission::AskReason::*;
        match value {
            EditTool => Self {
                kind: ApprovalReasonKind::EditTool,
                detail: None,
            },
            EditCommand { matched_pattern } => Self {
                kind: ApprovalReasonKind::EditCommand,
                detail: Some(matched_pattern.clone()),
            },
            DangerousCommand { matched_pattern } => Self {
                kind: ApprovalReasonKind::DangerousCommand,
                detail: Some(matched_pattern.clone()),
            },
            ProtectedPath { matched_path } => Self {
                kind: ApprovalReasonKind::ProtectedPath,
                detail: Some(matched_path.clone()),
            },
            AgentCustomList => Self {
                kind: ApprovalReasonKind::AgentCustomList,
                detail: None,
            },
            SmartJudge { rationale } => Self {
                kind: ApprovalReasonKind::SmartJudge,
                detail: Some(rationale.clone()),
            },
            BrowserEvaluate { script_preview } => Self {
                kind: ApprovalReasonKind::BrowserEvaluate,
                detail: Some(script_preview.clone()),
            },
            MacControlAction { action } => Self {
                kind: ApprovalReasonKind::MacControlAction,
                detail: Some(action.clone()),
            },
            MacControlDangerousAction { action } => Self {
                kind: ApprovalReasonKind::MacControlDangerousAction,
                detail: Some(action.clone()),
            },
            PlanModeAsk => Self {
                kind: ApprovalReasonKind::PlanModeAsk,
                detail: None,
            },
        }
    }
}

/// Approval response from frontend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ApprovalResponse {
    AllowOnce,
    AllowAlways, // adds command pattern to allowlist
    Deny,
}

/// Where an approval decision came from. Carried in the
/// [`EVENT_APPROVAL_RESOLVED`] broadcast so every surface (GUI / IM / HTTP) can
/// dismiss its dialog and tell "decided elsewhere" apart from "I decided this".
/// Variants grow as each resolution path lands (timeout → F, session delete →
/// A-9, eviction → G).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResolutionSource {
    /// Desktop GUI (Tauri command).
    Gui,
    /// HTTP / WS client.
    Http,
    /// IM channel (button or text reply).
    Im,
    /// Auto-denied because the owning session was deleted / purged (A-9).
    SessionDeleted,
    /// Approval dialog timed out and was resolved to **deny** — either
    /// `approval_timeout_action=deny` or a strict reason force-denied (F2/F3).
    TimeoutDeny,
    /// Approval dialog timed out and was resolved to **proceed**
    /// (`approval_timeout_action=proceed`, non-strict reason).
    TimeoutProceed,
    /// Auto-denied because the IM chat that owned the prompt was taken over /
    /// evicted while the session stayed active (G5 / SURFACE-4).
    Eviction,
    /// Dismissed because the backgrounded job parked on this approval was
    /// cancelled (R8) — the job settles `Cancelled` via its own runner; this
    /// only clears the now-orphaned dialog on every surface.
    JobCancelled,
}

impl ApprovalResolutionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gui => "gui",
            Self::Http => "http",
            Self::Im => "im",
            Self::SessionDeleted => "session_deleted",
            Self::TimeoutDeny => "timeout_deny",
            Self::TimeoutProceed => "timeout_proceed",
            Self::Eviction => "eviction",
            Self::JobCancelled => "job_cancelled",
        }
    }
}

/// How a backgrounded tool call got authorized to run — the persistent audit
/// counterpart to [`ApprovalResolutionSource`] (transient broadcast), sharing
/// the same snake_case word table. Stored in the async-job `approval_origin`
/// column so audits can tell a real human grant apart from a weaker
/// timeout-proceed (TIMEOUT-2). Written by the exec async approval-reorder; the
/// sync exec path / other origins are wired by later subtasks (F6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOrigin {
    /// User clicked Approve (once or always), or a prior AllowAlways prefix
    /// matched — a real human grant.
    User,
    /// Approval dialog timed out and `approval_timeout_action=proceed` — a
    /// weaker authorization than an explicit click.
    TimeoutProceed,
    /// An unattended surface (cron / headless-no-client / ACP-no-capability /
    /// subagent-no-parent-surface) auto-proceeded because
    /// `unattendedApprovalAction=proceed` — a weaker, non-human authorization,
    /// recorded distinctly from a real `User` grant. A strict reason can never
    /// reach here (it is force-denied). Epic D / F (TIMEOUT-1).
    UnattendedProceed,
    /// A YOLO session or global dangerous-skip bypassed the gate.
    Yolo,
    /// IM auto-approve account / slash-skill execution skipped all gates.
    AutoApprove,
    /// Async-job re-entry pre-approved at the outer engine gate.
    ExternalPreApproved,
    /// The permission engine allowed the command without prompting (safe for
    /// the current session preset, not via YOLO).
    PolicyAllow,
}

impl ApprovalOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::TimeoutProceed => "timeout_proceed",
            Self::UnattendedProceed => "unattended_proceed",
            Self::Yolo => "yolo",
            Self::AutoApprove => "auto_approve",
            Self::ExternalPreApproved => "external_pre_approved",
            Self::PolicyAllow => "policy_allow",
        }
    }
}

/// Why a [`submit_approval_response`] call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSubmitError {
    /// No pending approval matched the request id — already resolved, timed
    /// out, evicted, or never existed. HTTP maps this to 410 Gone (MISC-18).
    NotPending,
    /// The pending entry existed but its receiver was already dropped (the
    /// awaiting tool future is gone). The decision had no effect — surfaces
    /// should report "approval no longer active" instead of a false success
    /// (PENDING-1).
    NoLongerActive,
}

impl std::fmt::Display for ApprovalSubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPending => write!(f, "No pending approval request"),
            Self::NoLongerActive => write!(f, "Approval is no longer active"),
        }
    }
}

impl std::error::Error for ApprovalSubmitError {}

/// Broadcast when an approval is resolved by any surface, so all surfaces can
/// dismiss their dialog and (for non-originating ones) indicate it was handled
/// elsewhere. Payload: `{ requestId, sessionId?, decision, source }`.
pub const EVENT_APPROVAL_RESOLVED: &str = "approval:resolved";

/// Broadcast when an attended approval is REQUESTED (surfaces the dialog).
/// Payload is a serialized [`ApprovalRequest`] (`session_id` is snake_case here,
/// unlike [`EVENT_APPROVAL_RESOLVED`]'s camelCase `sessionId`). The R8-follow-up
/// subagent approval-projection watcher subscribes to both to flip a background
/// subagent's projection label running ⇄ awaiting_approval.
pub const EVENT_APPROVAL_REQUIRED: &str = "approval_required";

fn approval_decision_str(response: ApprovalResponse) -> &'static str {
    match response {
        ApprovalResponse::AllowOnce => "allow_once",
        ApprovalResponse::AllowAlways => "allow_always",
        ApprovalResponse::Deny => "deny",
    }
}

/// Emit the [`EVENT_APPROVAL_RESOLVED`] broadcast. No-op without an event bus.
pub fn emit_approval_resolved(
    request_id: &str,
    session_id: Option<&str>,
    decision: &str,
    source: ApprovalResolutionSource,
) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            EVENT_APPROVAL_RESOLVED,
            serde_json::json!({
                "requestId": request_id,
                "sessionId": session_id,
                "decision": decision,
                "source": source.as_str(),
            }),
        );
    }
}

/// In-memory entry for a pending approval. Stores the oneshot sender plus the
/// originating session id so we can aggregate counts per session for the
/// sidebar "needs your response" indicator.
struct PendingApprovalEntry {
    sender: tokio::sync::oneshot::Sender<ApprovalResponse>,
    session_id: Option<String>,
}

/// Global approval request registry
static PENDING_APPROVALS: OnceLock<TokioMutex<HashMap<String, PendingApprovalEntry>>> =
    OnceLock::new();

fn get_pending_approvals() -> &'static TokioMutex<HashMap<String, PendingApprovalEntry>> {
    PENDING_APPROVALS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

// ── R8: background-job approval bridge ────────────────────────────────────
// A backgrounded tool job runs its dispatch on a dedicated OS thread with a
// current-thread runtime (`async_jobs::spawn::start_runner`). When that
// dispatch reaches an *attended* approval gate it blocks on the oneshot below —
// the job is genuinely parked waiting for a human, not "running". This
// thread-local lets the job runner observe that park/resume around the wait so
// it can flip the job row Running ⇄ AwaitingApproval (R8) WITHOUT `tools` taking
// a dependency on `async_jobs`: the runner installs closures that call back into
// the job DB. Only set on a job-runner thread (`BackgroundApprovalScope`); a
// no-op everywhere else (foreground turns, subagent runtimes), so foreground and
// subagent approvals are unaffected.

/// Park/resume hooks a background-job runner installs for the duration of its
/// dispatch. See module-level R8 note.
pub struct BackgroundApprovalBridge {
    /// Called right before blocking on an attended approval, with the pending
    /// `request_id` (so a later cancel of the parked job can dismiss the
    /// orphaned dialog). Flips the owning job Running → AwaitingApproval.
    pub on_park: Box<dyn Fn(&str)>,
    /// Called once the wait ends (resolved, timed out, or the future was dropped
    /// by a cancel — the [`BgResumeGuard`] Drop is the single revert point so a
    /// cancel that drops the future mid-await still un-parks the row). `origin`
    /// is `Some` only on a proceed outcome so the runner can correct the job's
    /// `approval_origin` audit column; `None` on deny / timeout-deny / drop (the
    /// job settles terminal anyway). Reverts AwaitingApproval → Running.
    pub on_resume: Box<dyn Fn(Option<ApprovalOrigin>)>,
}

thread_local! {
    static BG_APPROVAL_BRIDGE: std::cell::RefCell<Option<BackgroundApprovalBridge>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII installer for the background-approval bridge on the current job-runner
/// thread. Cleared on drop so a thread can never leak a stale bridge. Held for
/// the whole `run_job_to_completion` body by the job runner.
pub struct BackgroundApprovalScope;

impl BackgroundApprovalScope {
    pub fn new(bridge: BackgroundApprovalBridge) -> Self {
        BG_APPROVAL_BRIDGE.with(|c| *c.borrow_mut() = Some(bridge));
        Self
    }
}

impl Drop for BackgroundApprovalScope {
    fn drop(&mut self) {
        BG_APPROVAL_BRIDGE.with(|c| *c.borrow_mut() = None);
    }
}

fn notify_bg_park(request_id: &str) {
    BG_APPROVAL_BRIDGE.with(|c| {
        if let Some(b) = c.borrow().as_ref() {
            (b.on_park)(request_id);
        }
    });
}

fn notify_bg_resume(origin: Option<ApprovalOrigin>) {
    BG_APPROVAL_BRIDGE.with(|c| {
        if let Some(b) = c.borrow().as_ref() {
            (b.on_resume)(origin);
        }
    });
}

/// RAII guard that calls [`notify_bg_resume`] exactly once on drop — covering
/// the resolved / timeout paths (which set `origin` first) AND the cancel path
/// (the awaiting future is dropped mid-poll, so no match arm runs; the guard's
/// Drop still un-parks the job). A no-op when no bridge is installed.
struct BgResumeGuard {
    origin: Option<ApprovalOrigin>,
}

impl BgResumeGuard {
    fn new() -> Self {
        Self { origin: None }
    }
}

impl Drop for BgResumeGuard {
    fn drop(&mut self) {
        notify_bg_resume(self.origin);
    }
}

/// R8: dismiss the now-orphaned approval dialog for a backgrounded job that was
/// cancelled while parked. Called from the job runner's `on_resume` (which fires
/// when the parked dispatch future is dropped by the cancel) — i.e. AFTER the
/// `select!` has already chosen the cancel branch, so removing the registry
/// entry here can never race the dispatch into a spurious "denied" completion.
///
/// Removes the pending entry **only if still present**: on an approve / deny /
/// timeout the entry was already cleared by `submit_approval_response` / the
/// timeout path, so this is a no-op and emits nothing. It is present only on the
/// cancel path — there we remove it (clearing the "needs your response" badge),
/// then broadcast `approval:resolved` so every surface dismisses its dialog and
/// the IM listener clears its `TEXT_PENDING` entry. Returns whether it dismissed.
///
/// Best-effort under `try_lock` (the caller is a sync Drop on the job thread); a
/// contended lock leaves the inert entry to be GC'd on the next submit/cleanup.
pub fn dismiss_parked_job_approval(request_id: &str, session_id: Option<&str>) -> bool {
    let removed = match PENDING_APPROVALS.get() {
        Some(approvals) => match approvals.try_lock() {
            Ok(mut pending) => pending.remove(request_id).is_some(),
            Err(_) => false,
        },
        None => false,
    };
    if removed {
        emit_approval_resolved(
            request_id,
            session_id,
            "deny",
            ApprovalResolutionSource::JobCancelled,
        );
        emit_pending_interactions_changed(session_id);
    }
    removed
}

/// True iff an approval is currently registered and awaiting a human decision
/// for `session_id`. Used by `subagent` spawn_and_wait to tell the parent the
/// child is **paused on an approval** rather than merely "backgrounded" (D6 /
/// DEADLOCK-5). A pending child approval only persists where it can actually be
/// answered — unattended surfaces fail-close instead of registering one.
pub(crate) async fn session_has_pending_approval(session_id: &str) -> bool {
    let pending = get_pending_approvals().lock().await;
    pending
        .values()
        .any(|e| e.session_id.as_deref() == Some(session_id))
}

/// Return the request ids of every pending approval owned by `session_id`.
/// Used by the IM eviction watcher (G5 / SURFACE-4) to deny each pending
/// approval when the owning chat is taken over — there is no reverse index, so
/// this scans the registry (the pending set is tiny in practice).
pub async fn pending_request_ids_for_session(session_id: &str) -> Vec<String> {
    let pending = get_pending_approvals().lock().await;
    pending
        .iter()
        .filter(|(_, e)| e.session_id.as_deref() == Some(session_id))
        .map(|(rid, _)| rid.clone())
        .collect()
}

/// Count pending approvals grouped by session id. Approvals registered without
/// a session id (e.g. global commands triggered outside any chat) are skipped.
pub async fn pending_approvals_per_session() -> HashMap<String, i64> {
    let pending = get_pending_approvals().lock().await;
    let mut out: HashMap<String, i64> = HashMap::new();
    for entry in pending.values() {
        if let Some(sid) = entry.session_id.as_ref() {
            *out.entry(sid.clone()).or_insert(0) += 1;
        }
    }
    out
}

/// Return the originating session id for a pending approval request.
///
/// Used by IM button callbacks to verify that the click came from the same
/// channel conversation that received the approval prompt before submitting
/// the tool response.
pub async fn pending_approval_session_id(request_id: &str) -> Result<Option<String>> {
    let pending = get_pending_approvals().lock().await;
    pending
        .get(request_id)
        .map(|entry| entry.session_id.clone())
        .ok_or_else(|| anyhow::anyhow!("No pending approval request: {}", request_id))
}

/// Submit an approval response from a given surface (GUI / HTTP / IM).
///
/// On success broadcasts [`EVENT_APPROVAL_RESOLVED`] so other surfaces dismiss
/// their dialog. Returns:
/// - `Err(NotPending)` when no pending approval matches (already resolved /
///   timed out / evicted) — HTTP maps to 410.
/// - `Err(NoLongerActive)` when the entry existed but its receiver was already
///   dropped (the awaiting tool future is gone): the decision had no effect, so
///   surface a clear error instead of a false success (PENDING-1).
pub async fn submit_approval_response(
    request_id: &str,
    response: ApprovalResponse,
    source: ApprovalResolutionSource,
) -> std::result::Result<(), ApprovalSubmitError> {
    let mut pending = get_pending_approvals().lock().await;
    let Some(entry) = pending.remove(request_id) else {
        return Err(ApprovalSubmitError::NotPending);
    };
    let session_id = entry.session_id.clone();
    let delivered = entry.sender.send(response).is_ok();
    drop(pending);
    emit_pending_interactions_changed(session_id.as_deref());
    // The entry was removed above, so every surface showing this request must
    // dismiss — broadcast resolved even when the receiver was already gone
    // (NoLongerActive). Otherwise a dialog mirrored on another surface (IM / a
    // second GUI) lingers until an unrelated event. The submitter still gets the
    // NoLongerActive error below so it knows its click had no effect (PENDING-1).
    emit_approval_resolved(
        request_id,
        session_id.as_deref(),
        approval_decision_str(response),
        source,
    );
    if !delivered {
        return Err(ApprovalSubmitError::NoLongerActive);
    }
    Ok(())
}

/// Deny and resolve every pending approval owned by `session_id`. Called by the
/// session cleanup watcher when a session is deleted / purged so the blocked
/// tool turn unblocks instead of hanging forever, and every surface dismisses
/// its dialog (DELETE-1 / INCOG-4). Returns the number of approvals denied.
pub async fn deny_pending_for_session(session_id: &str, source: ApprovalResolutionSource) -> usize {
    // Drain matching entries under the lock, then send/emit after releasing it
    // (the receiver side and bus subscribers must not run while we hold it).
    let drained: Vec<(String, PendingApprovalEntry)> = {
        let mut pending = get_pending_approvals().lock().await;
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, e)| e.session_id.as_deref() == Some(session_id))
            .map(|(k, _)| k.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| pending.remove(&id).map(|e| (id, e)))
            .collect()
    };
    if drained.is_empty() {
        return 0;
    }
    let count = drained.len();
    for (request_id, entry) in drained {
        let _ = entry.sender.send(ApprovalResponse::Deny);
        emit_approval_resolved(&request_id, Some(session_id), "deny", source);
    }
    emit_pending_interactions_changed(Some(session_id));
    count
}

/// Broadcast to the frontend that some session's pending-interaction count
/// likely changed so the sidebar should reload its list. Payload carries an
/// optional `session_id` for clients that want to optimise; a missing id means
/// "any session, please refresh".
pub fn emit_pending_interactions_changed(session_id: Option<&str>) {
    if let Some(bus) = crate::globals::get_event_bus() {
        let payload = match session_id {
            Some(sid) => serde_json::json!({ "sessionId": sid }),
            None => serde_json::json!({}),
        };
        bus.emit("session_pending_interactions_changed", payload);
    }
}

/// Allowlist: command prefixes that are auto-approved
static COMMAND_ALLOWLIST: OnceLock<TokioMutex<Vec<String>>> = OnceLock::new();

fn get_allowlist() -> &'static TokioMutex<Vec<String>> {
    COMMAND_ALLOWLIST.get_or_init(|| {
        let list = load_allowlist().unwrap_or_default();
        TokioMutex::new(list)
    })
}

fn allowlist_path() -> std::path::PathBuf {
    crate::paths::root_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("exec-approvals.json")
}

fn load_allowlist() -> Result<Vec<String>> {
    let path = allowlist_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&data)?)
    } else {
        Ok(Vec::new())
    }
}

/// Check if command is in the allowlist
pub(crate) async fn is_command_allowed(command: &str) -> bool {
    let list = get_allowlist().lock().await;
    let cmd_trimmed = command.trim();
    list.iter()
        .any(|pattern| cmd_trimmed.starts_with(pattern) || cmd_trimmed == *pattern)
}

pub(crate) fn approval_timeout_secs() -> u64 {
    let cfg = crate::config::cached_config();
    if cfg.permission.approval_timeout_enabled {
        cfg.permission.approval_timeout_secs
    } else {
        0
    }
}

pub(crate) fn approval_timeout_action() -> crate::config::ApprovalTimeoutAction {
    crate::config::cached_config()
        .permission
        .approval_timeout_action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalCheckError {
    RequestSerialization,
    EventBusUnavailable,
    Cancelled,
    TimedOut {
        timeout_secs: u64,
        /// The approval reason was strict (forbids AllowAlways). F2/F3 force a
        /// deny on timeout regardless of `approval_timeout_action` — a strict
        /// reason must never auto-proceed unattended. Epic F (TIMEOUT-1).
        strict: bool,
    },
    /// No human could answer this prompt (cron / headless-no-client / ACP-no-
    /// capability / subagent-no-parent-surface) and `unattendedApprovalAction`
    /// is `deny`. Fail-closed without blocking (Epic D). Callers render it via
    /// [`crate::tools::ToolRejection::denied_unattended`].
    Unattended {
        reason: crate::permission::UnattendedReason,
    },
    /// No human could answer, `unattendedApprovalAction=proceed`, and the reason
    /// was NOT strict — auto-proceed with a weaker-than-click origin. A strict
    /// reason on an unattended surface returns [`Self::Unattended`] (fail-closed
    /// deny) instead, never this. Epic D / F (TIMEOUT-1).
    UnattendedProceed {
        reason: crate::permission::UnattendedReason,
    },
}

impl fmt::Display for ApprovalCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestSerialization => write!(f, "Failed to serialize approval request"),
            Self::EventBusUnavailable => write!(f, "EventBus not available for approval events"),
            Self::Cancelled => write!(f, "Approval request cancelled"),
            Self::TimedOut {
                timeout_secs,
                strict,
            } => {
                if *strict {
                    write!(
                        f,
                        "Approval request timed out ({}s); strict reason — denied",
                        timeout_secs
                    )
                } else {
                    write!(f, "Approval request timed out ({}s)", timeout_secs)
                }
            }
            Self::Unattended { reason } => {
                write!(f, "No one available to approve ({})", reason.explain())
            }
            Self::UnattendedProceed { reason } => {
                write!(
                    f,
                    "No one available to approve; auto-proceeding ({})",
                    reason.explain()
                )
            }
        }
    }
}

/// Whether an unattended approval surface should auto-proceed: only when the
/// user configured `proceed` AND the reason is not strict. A strict reason
/// (forbids AllowAlways — protected path / dangerous command / mac-dangerous /
/// plan-ask) must NEVER auto-proceed when no human can answer; it is force-denied
/// regardless of the configured action, mirroring the strict-timeout rule so the
/// "auto-allow always needs a human" invariant holds on every path (TIMEOUT-1).
/// Pure helper so the security-critical decision is unit-testable without global
/// config / runtime-role state.
fn unattended_effective_proceed(
    action: crate::permission::UnattendedApprovalAction,
    strict: bool,
) -> bool {
    matches!(action, crate::permission::UnattendedApprovalAction::Proceed) && !strict
}

/// Request approval from the user for a command.
/// Emits an EventBus event and waits for the response via oneshot channel.
/// `session_id` is used by the IM channel approval listener to route the
/// request to the correct chat.
pub(crate) async fn check_and_request_approval(
    command: &str,
    cwd: &str,
    session_id: Option<&str>,
    reason: Option<ApprovalReasonPayload>,
) -> std::result::Result<ApprovalResponse, ApprovalCheckError> {
    // Epic D (DEADLOCK-1..5): an `Ask` was decided, but on some entries no human
    // can ever answer it. Resolve the surface BEFORE registering a pending entry
    // or blocking on the oneshot — otherwise the turn / HTTP request / cron run
    // hangs forever and a generic timeout later masks the real cause. This is the
    // single chokepoint for both the exec command gate and the engine Ask path.
    // F1 (TIMEOUT-1) / Epic D: capture strictness up front. A strict reason
    // (protected path / dangerous command / mac-dangerous / plan-ask) must NEVER
    // auto-proceed when no human can answer — whether the surface is unattended
    // up front (here) or the prompt later times out (F2/F3 below). `reason` is
    // moved into the outbound `ApprovalRequest`, so read it before that.
    let strict = reason.as_ref().map(|r| r.kind.is_strict()).unwrap_or(false);

    if let crate::permission::ApprovalSurface::Unattended(unattended) =
        crate::permission::evaluate_approval_surface(session_id)
    {
        let action = crate::config::cached_config()
            .permission
            .unattended_approval_action;
        // A strict reason overrides `proceed` to a fail-closed deny, mirroring the
        // strict-timeout rule (F2/F3) so "auto-allow always needs a human" holds
        // on every path, not just the timeout one (TIMEOUT-1).
        let effective_proceed = unattended_effective_proceed(action, strict);
        // Structured signal for any in-app consumer (cron run telemetry / dashboard
        // / future UI), parallel to `approval_required`. The whole-job timeout no
        // longer masks the cause (D2/D3 fail-close instead of hanging); this makes
        // "a tool needed an approval no one could give" observable, not just prose
        // in the model's reply. `effective` reflects the strict override.
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "approval:unattended",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": unattended.as_str(),
                    "action": match action {
                        crate::permission::UnattendedApprovalAction::Proceed => "proceed",
                        crate::permission::UnattendedApprovalAction::Deny => "deny",
                    },
                    "strict": strict,
                    "effective": if effective_proceed { "proceed" } else { "deny" },
                    "command": command,
                }),
            );
        }
        if effective_proceed {
            app_warn!(
                "tool",
                "approval",
                "Unattended approval surface ({}) for '{}' (session={:?}) — auto-proceeding per unattendedApprovalAction=proceed",
                unattended.as_str(),
                command,
                session_id
            );
            // Weaker-than-click authorization: the caller records it distinctly
            // from a real User grant (audit) and still runs the tool.
            return Err(ApprovalCheckError::UnattendedProceed { reason: unattended });
        }
        if strict && matches!(action, crate::permission::UnattendedApprovalAction::Proceed) {
            app_warn!(
                "permission",
                "strict_unattended_deny",
                "Unattended approval surface ({}) for '{}' (session={:?}) — reason is strict; forcing deny despite unattendedApprovalAction=proceed",
                unattended.as_str(),
                command,
                session_id
            );
        } else {
            app_warn!(
                "tool",
                "approval",
                "Unattended approval surface ({}) for '{}' (session={:?}) — fail-closed deny (no one could approve)",
                unattended.as_str(),
                command,
                session_id
            );
        }
        // Observation hook parity with the user-decline path.
        crate::hooks::fire_permission_denied(session_id, command, unattended.as_str(), None);
        return Err(ApprovalCheckError::Unattended { reason: unattended });
    }

    let request_id = create_session_id();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let timeout_secs = approval_timeout_secs();
    // `strict` was captured above (before the unattended check) and is reused
    // here for the timeout force-deny (F2/F3).

    // Register the pending approval
    {
        let mut pending = get_pending_approvals().lock().await;
        pending.insert(
            request_id.clone(),
            PendingApprovalEntry {
                sender: tx,
                session_id: session_id.map(|s| s.to_string()),
            },
        );
    }

    // Emit event to frontend
    let request = ApprovalRequest {
        request_id: request_id.clone(),
        command: command.to_string(),
        cwd: cwd.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        reason,
        // E5 (INCOG-6): tell the surface to hide AllowAlways for incognito turns.
        incognito: crate::session::is_session_incognito(session_id),
    };

    if let Some(bus) = crate::globals::get_event_bus() {
        let event_data = match serde_json::to_value(&request) {
            Ok(value) => value,
            Err(_) => {
                let mut pending = get_pending_approvals().lock().await;
                pending.remove(&request_id);
                return Err(ApprovalCheckError::RequestSerialization);
            }
        };
        bus.emit(EVENT_APPROVAL_REQUIRED, event_data);
        // Notification hook (observation): bridge the permission prompt to user
        // scripts / desktop notifications. Fire-and-forget.
        crate::hooks::fire_notification(
            session_id.unwrap_or_default(),
            "permission_prompt",
            command,
        );
        // PermissionRequest hook (observation): the structured permission event,
        // matchable on the command. Single chokepoint for every approval prompt.
        crate::hooks::fire_permission_request(session_id, command, None);
        app_info!(
            "tool",
            "approval",
            "Approval requested for command: {} (id: {})",
            command,
            request_id
        );
    } else {
        // No EventBus available, clean up and return error
        let mut pending = get_pending_approvals().lock().await;
        pending.remove(&request_id);
        return Err(ApprovalCheckError::EventBusUnavailable);
    }

    // R8: if this approval is being awaited inside a background-job runner, the
    // job is genuinely parked on a human decision — flip its row to
    // AwaitingApproval for the duration of the wait. `resume_guard`'s Drop is the
    // single revert point (Running on a proceed outcome carried by `origin`,
    // else still reverted to Running and the runner settles it terminal): it
    // fires on resolve, timeout, AND a cancel that drops this future mid-await.
    // Both are no-ops on a foreground / subagent thread (no bridge installed).
    notify_bg_park(&request_id);
    let mut resume_guard = BgResumeGuard::new();

    let wait_result = if timeout_secs == 0 {
        rx.await.map_err(|_| "cancelled")
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err("cancelled"),
            Err(_) => Err("timeout"),
        }
    };

    match wait_result {
        Ok(response) => {
            // R8: a user grant (allow once/always) is the real audit origin —
            // hand it to the runner so the job's placeholder `approval_origin`
            // (set at spawn before the gate ran) is corrected. A Deny leaves
            // `origin = None`: the job settles terminal, origin is moot.
            if !matches!(response, ApprovalResponse::Deny) {
                resume_guard.origin = Some(ApprovalOrigin::User);
            }
            if let Some(logger) = crate::get_logger() {
                let response_str = match &response {
                    ApprovalResponse::AllowOnce => "allow_once",
                    ApprovalResponse::AllowAlways => "allow_always",
                    ApprovalResponse::Deny => "deny",
                };
                logger.log("info", "tool", "approval::response",
                    &format!("Approval response: {} for '{}'", response_str, command),
                    Some(serde_json::json!({"command": command, "response": response_str, "request_id": request_id}).to_string()),
                    None, None);
            }
            // PermissionDenied hook (observation): the user declined the prompt.
            // Single chokepoint for every user-facing decline.
            if matches!(response, ApprovalResponse::Deny) {
                crate::hooks::fire_permission_denied(session_id, command, "user_declined", None);
            }
            Ok(response)
        }
        Err("cancelled") => {
            // Drop any IM-side pending entry — if this approval was being
            // surfaced on a channel without buttons, the user would
            // otherwise see the prompt linger forever.
            crate::channel::worker::approval::drop_pending_by_request_id(&request_id).await;
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "tool",
                    "approval::cancelled",
                    &format!("Approval cancelled for '{}'", command),
                    None,
                    None,
                    None,
                );
            }
            Err(ApprovalCheckError::Cancelled)
        }
        Err("timeout") => {
            // Timeout — clean up
            {
                let mut pending = get_pending_approvals().lock().await;
                pending.remove(&request_id);
            }
            emit_pending_interactions_changed(session_id);
            // Drop the IM-side `TEXT_PENDING` entry. The companion
            // `approval_timed_out` event below only carries the user-facing
            // "timed out" notification; cleanup is unconditional so cancel-
            // path and timeout-path stay symmetric.
            crate::channel::worker::approval::drop_pending_by_request_id(&request_id).await;
            // Notify subscribers so IM and desktop clients can clear stale
            // UI and tell the user the approval expired.
            if let Some(bus) = crate::globals::get_event_bus() {
                bus.emit(
                    "approval_timed_out",
                    serde_json::json!({
                        "request_id": request_id,
                        "session_id": session_id,
                        "timeout_secs": timeout_secs,
                        "timeout_action": approval_timeout_action(),
                    }),
                );
            }
            // F4 (TIMEOUT-3 / SURFACE-1): also emit the unified `approval:resolved`
            // so every surface dismisses its dialog symmetrically with the submit
            // path (G6). The effective decision mirrors what F2/F3 enforce: a
            // strict reason OR `action=deny` resolves to deny, else proceed.
            let resolved_deny = strict
                || matches!(
                    approval_timeout_action(),
                    crate::config::ApprovalTimeoutAction::Deny
                );
            let (decision, resolution_source) = if resolved_deny {
                ("deny", ApprovalResolutionSource::TimeoutDeny)
            } else {
                // R8: a non-strict timeout-proceed authorizes a parked job to run
                // — record the weaker-than-click origin for audit (F6 / TIMEOUT-2).
                resume_guard.origin = Some(ApprovalOrigin::TimeoutProceed);
                ("allow_once", ApprovalResolutionSource::TimeoutProceed)
            };
            emit_approval_resolved(&request_id, session_id, decision, resolution_source);
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "tool",
                    "approval::timeout",
                    &format!(
                        "Approval timed out for '{}' after {}s",
                        command, timeout_secs
                    ),
                    None,
                    None,
                    None,
                );
            }
            Err(ApprovalCheckError::TimedOut {
                timeout_secs,
                strict,
            })
        }
        Err(_) => unreachable!(),
    }
}

/// Test-only drivers for the R8 background-approval bridge. The real driver is
/// `check_and_request_approval`'s attended wait; these let `async_jobs` tests
/// exercise the installed bridge (park → resume) without standing up the full
/// approval flow (event bus + a responder + a pending registry entry).
#[cfg(test)]
pub(crate) fn test_drive_bridge_park(request_id: &str) {
    notify_bg_park(request_id);
}

#[cfg(test)]
pub(crate) fn test_drive_bridge_resume(origin: Option<ApprovalOrigin>) {
    // Mirror the production path: the resume always fires through a guard drop.
    let mut guard = BgResumeGuard::new();
    guard.origin = origin;
    drop(guard);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::AskReason;

    /// R8: the background-approval bridge is a thread-local that only fires while
    /// a `BackgroundApprovalScope` is installed (a job-runner thread) and is
    /// cleared on scope drop, so foreground / subagent threads never park a job.
    #[test]
    fn background_approval_bridge_thread_local_park_resume_and_clear() {
        use std::cell::RefCell;
        use std::rc::Rc;

        // No bridge installed → notify is a silent no-op (foreground turns).
        test_drive_bridge_park("none");
        test_drive_bridge_resume(None);

        let parked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let resumed: Rc<RefCell<Vec<Option<&'static str>>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let p = parked.clone();
            let r = resumed.clone();
            let _scope = BackgroundApprovalScope::new(BackgroundApprovalBridge {
                on_park: Box::new(move |rid: &str| p.borrow_mut().push(rid.to_string())),
                on_resume: Box::new(move |o: Option<ApprovalOrigin>| {
                    r.borrow_mut().push(o.map(|o| o.as_str()))
                }),
            });
            test_drive_bridge_park("req-1");
            test_drive_bridge_resume(Some(ApprovalOrigin::User));
            assert_eq!(parked.borrow().as_slice(), &["req-1".to_string()]);
            assert_eq!(resumed.borrow().as_slice(), &[Some("user")]);
        }
        // Scope dropped → thread-local cleared → further notifies are no-ops.
        test_drive_bridge_park("after");
        test_drive_bridge_resume(Some(ApprovalOrigin::User));
        assert_eq!(parked.borrow().len(), 1, "no park after scope drop");
        assert_eq!(resumed.borrow().len(), 1, "no resume after scope drop");
    }

    /// F1 (TIMEOUT-1): `ApprovalReasonKind::is_strict()` is a serializable mirror
    /// of the canonical `AskReason::forbids_allow_always()`. Assert they agree for
    /// EVERY reason variant so the strict set can never silently drift between the
    /// two representations.
    #[test]
    fn reason_kind_is_strict_matches_ask_reason() {
        let all = [
            AskReason::EditTool,
            AskReason::EditCommand {
                matched_pattern: "rm".into(),
            },
            AskReason::DangerousCommand {
                matched_pattern: "rm -rf".into(),
            },
            AskReason::ProtectedPath {
                matched_path: "/etc".into(),
            },
            AskReason::AgentCustomList,
            AskReason::SmartJudge {
                rationale: "x".into(),
            },
            AskReason::BrowserEvaluate {
                script_preview: "x".into(),
            },
            AskReason::MacControlAction {
                action: "click".into(),
            },
            AskReason::MacControlDangerousAction {
                action: "quit".into(),
            },
            AskReason::PlanModeAsk,
        ];
        for reason in &all {
            let kind = ApprovalReasonPayload::from(reason).kind;
            assert_eq!(
                kind.is_strict(),
                reason.forbids_allow_always(),
                "strict mismatch for {:?}",
                reason
            );
        }
    }

    /// Red line (TIMEOUT-1): a strict reason must NEVER auto-proceed on an
    /// unattended surface, even when the user configured
    /// `unattendedApprovalAction=proceed` — it is force-denied, exactly like the
    /// strict-timeout path. Non-strict reasons still honor the configured action.
    #[test]
    fn strict_reason_never_auto_proceeds_unattended() {
        use crate::permission::UnattendedApprovalAction::{Deny, Proceed};
        // Non-strict: honor whatever the user configured.
        assert!(unattended_effective_proceed(Proceed, false));
        assert!(!unattended_effective_proceed(Deny, false));
        // Strict: force-denied regardless of the configured action.
        assert!(!unattended_effective_proceed(Proceed, true));
        assert!(!unattended_effective_proceed(Deny, true));
    }
}

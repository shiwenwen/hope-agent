//! Tool permission / approval system.
//!
//! Single-source-of-truth for "should this tool call be approved by the user?"
//!
//! ## Architecture
//!
//! The decision is driven by:
//! 1. **Plan Mode** (highest priority) — overrides everything
//! 2. **YOLO** (global / session) — bypasses all approvals (with audit warn)
//! 3. **Protected paths / dangerous commands** — force-ask in non-YOLO modes
//! 4. **AllowAlways accumulated rules** (project / session / agent_home / global)
//! 5. **Session mode preset** (Default / Smart)
//! 6. **Default fallback** — allow
//!
//! Public entry point: [`engine::resolve`].

pub mod allowlist;
pub mod approval_surface;
pub mod config;
pub mod dangerous_commands;
pub mod edit_commands;
pub mod engine;
pub mod judge;
pub(crate) mod list_store;
pub mod mode;
pub(crate) mod pattern_match;
pub mod protected_paths;
pub mod rules;
pub mod session_edits;

pub use approval_surface::{evaluate_approval_surface, ApprovalSurface, UnattendedReason};
pub use config::{ApprovalTimeoutAction, PermissionGlobalConfig, UnattendedApprovalAction};
pub use mode::{JudgeModelConfig, SandboxMode, SessionMode, SmartModeConfig, SmartStrategy};

/// Final disposition of a tool call after the engine consumes all rule layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Tool runs without prompting the user.
    Allow,
    /// Tool prompts the user with an approval dialog.
    Ask {
        /// Why approval is needed (shown in the dialog).
        reason: AskReason,
    },
    /// Tool is blocked outright — user is told why and the call returns an error.
    Deny {
        /// Why the call was rejected.
        reason: String,
    },
}

/// Why the engine raised an approval prompt (drives dialog UX).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskReason {
    /// Hardcoded edit-class tool (`write` / `edit` / `apply_patch`).
    EditTool,
    /// `exec` matched the edit-command pattern list.
    EditCommand { matched_pattern: String },
    /// `exec` matched the dangerous-command pattern list.
    DangerousCommand { matched_pattern: String },
    /// Path argument matched a protected-path entry.
    ProtectedPath { matched_path: String },
    /// User explicitly added this tool to the agent's `custom_approval_tools`.
    AgentCustomList,
    /// Smart mode `judge_model` returned `ask`.
    SmartJudge { rationale: String },
    /// Browser `control.evaluate` is about to execute arbitrary JavaScript in
    /// the active tab.
    BrowserEvaluate { script_preview: String },
    /// Browser `control.raw_cdp` is about to send a raw Chrome DevTools
    /// Protocol method to a real Chrome tab.
    BrowserRawCdp { method: String },
    /// Browser control is about to inspect or take control of the user's real
    /// Chrome state through the extension backend.
    BrowserChromeAccess { action: String },
    /// Browser control is about to interrupt or mutate a real Chrome download.
    BrowserDownloadAction { action: String },
    /// Native macOS control action that mutates desktop focus/state.
    MacControlAction { action: String },
    /// Native macOS control action with destructive potential.
    MacControlDangerousAction { action: String },
    /// Plan Mode `ask_tools` list — tool is whitelisted but flagged as
    /// "needs explicit confirmation before each call". The default plan
    /// agent uses this for `exec` so a planning subagent can't quietly run
    /// commands while the plan is still being drafted.
    PlanModeAsk,
    /// `manage_cron action=delete` — the model is about to permanently remove a
    /// user's scheduled task. Deliberately **non-strict** (not in
    /// `forbids_allow_always`): unlike the exfil-class strict reasons, deleting a
    /// cron job is a controlled, owner-scoped integrity action, so AllowAlways /
    /// YOLO may bypass it and Smart mode may self-decide. Default mode still
    /// prompts; an unattended surface fail-closes per `unattended_approval_action`.
    CronDelete,
}

impl AskReason {
    /// `true` if this reason forbids `Allow Always` — protected paths,
    /// dangerous commands, and raw CDP against the user's real Chrome always
    /// need a per-call confirmation. `BrowserRawCdp` is strict because a single
    /// "Allow Always" would otherwise permanently grant arbitrary DevTools
    /// Protocol access (cookies, storage, navigation) to the logged-in browser.
    pub fn forbids_allow_always(&self) -> bool {
        matches!(
            self,
            AskReason::ProtectedPath { .. }
                | AskReason::DangerousCommand { .. }
                | AskReason::MacControlDangerousAction { .. }
                | AskReason::BrowserRawCdp { .. }
                | AskReason::PlanModeAsk
        )
    }
}

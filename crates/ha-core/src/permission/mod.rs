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
pub mod config;
pub mod dangerous_commands;
pub mod edit_commands;
pub mod engine;
pub mod judge;
pub mod mode;
pub mod protected_paths;
pub mod rules;

pub use config::{ApprovalTimeoutAction, PermissionGlobalConfig};
pub use mode::{JudgeModelConfig, SessionMode, SmartModeConfig, SmartStrategy};
pub use rules::{ArgMatcher, PermissionRules, RuleSpec};

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
}

impl AskReason {
    /// `true` if this reason forbids `Allow Always` — protected paths and
    /// dangerous commands always need a per-call confirmation.
    pub fn forbids_allow_always(&self) -> bool {
        matches!(
            self,
            AskReason::ProtectedPath { .. } | AskReason::DangerousCommand { .. }
        )
    }
}

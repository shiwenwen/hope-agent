//! Core hook data structures: events, inputs, outputs, decisions.
//!
//! Field-level aligned with the Claude Code hooks protocol (see
//! `docs/plans/hooks-system-design.md` §3.4 / §8). Event names use PascalCase
//! (`SessionStart`, `PreToolUse`); the common input fields are flattened into
//! each event's JSON payload using snake_case; hook stdout (`HookOutput`) uses
//! camelCase to match the official body schema.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::permission::SessionMode;
use crate::plan::PlanModeState;

/// All 28 hook events. Declared in one shot (even those not yet wired) so the
/// enum never needs a breaking change as later phases light up more events.
/// See design doc §2 for the per-event landing schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    UserPromptExpansion,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PostToolBatch,
    PermissionRequest,
    PermissionDenied,
    Stop,
    StopFailure,
    PreCompact,
    PostCompact,
    Notification,
    SubagentStart,
    SubagentStop,
    TaskCreated,
    TaskCompleted,
    TeammateIdle,
    ConfigChange,
    CwdChanged,
    FileChanged,
    InstructionsLoaded,
    Elicitation,
    ElicitationResult,
    WorktreeCreate,
    WorktreeRemove,
}

impl HookEvent {
    /// The PascalCase literal used for `hook_event_name` and for indexing the
    /// `HooksConfig` matcher groups.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::UserPromptExpansion => "UserPromptExpansion",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::PostToolBatch => "PostToolBatch",
            Self::PermissionRequest => "PermissionRequest",
            Self::PermissionDenied => "PermissionDenied",
            Self::Stop => "Stop",
            Self::StopFailure => "StopFailure",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::Notification => "Notification",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::TaskCreated => "TaskCreated",
            Self::TaskCompleted => "TaskCompleted",
            Self::TeammateIdle => "TeammateIdle",
            Self::ConfigChange => "ConfigChange",
            Self::CwdChanged => "CwdChanged",
            Self::FileChanged => "FileChanged",
            Self::InstructionsLoaded => "InstructionsLoaded",
            Self::Elicitation => "Elicitation",
            Self::ElicitationResult => "ElicitationResult",
            Self::WorktreeCreate => "WorktreeCreate",
            Self::WorktreeRemove => "WorktreeRemove",
        }
    }

    /// Whether this event is observation-only — it cannot gate execution, so a
    /// hook returning `block`/`deny` for it is downgraded to non-blocking +
    /// logged (design §5.1.1). Lists exactly the events fired this phase;
    /// blocking-capable events (PreToolUse / Stop / PreCompact / …) land with
    /// their decision semantics in later phases and are deliberately absent so
    /// their decisions are never neutralized here.
    pub fn is_observation_only(&self) -> bool {
        matches!(
            self,
            Self::SessionStart
                | Self::SessionEnd
                | Self::Notification
                | Self::PostToolUse
                | Self::PostToolUseFailure
                | Self::PostCompact
        )
    }
}

/// Protocol-facing permission mode string.
///
/// The official protocol allows `default|plan|acceptEdits|auto|dontAsk|
/// bypassPermissions`; we land `default|plan|bypassPermissions` plus an
/// `other` fallback (design doc §2.4). This is **not** the same as the
/// internal [`SessionMode`] — it folds in Plan Mode state, which lives in a
/// separate slot. Build it via [`PermissionMode::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PermissionMode {
    #[default]
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "plan")]
    Plan,
    #[serde(rename = "bypassPermissions")]
    BypassPermissions,
    #[serde(rename = "other")]
    Other,
}

impl PermissionMode {
    /// Map the internal `(SessionMode, PlanModeState)` pair to the protocol
    /// string. Plan Mode being active takes precedence over the session mode
    /// (an active plan implies user-gated tools regardless of YOLO).
    pub fn resolve(session_mode: SessionMode, plan_state: PlanModeState) -> Self {
        if matches!(
            plan_state,
            PlanModeState::Planning | PlanModeState::Review | PlanModeState::Executing
        ) {
            return Self::Plan;
        }
        match session_mode {
            SessionMode::Default => Self::Default,
            SessionMode::Yolo => Self::BypassPermissions,
            // Smart mode has no official equivalent.
            SessionMode::Smart => Self::Other,
        }
    }
}

/// Fields shared by every hook event's input payload. Serialized flat into the
/// per-event JSON (so a hook script sees `session_id`, `cwd`, … alongside the
/// event-specific fields). Field names are snake_case per design doc §8.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonHookInput {
    pub session_id: String,
    pub transcript_path: PathBuf,
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// Source for a `SessionStart` event (design doc §5.1.1 matcher target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

impl SessionStartSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Resume => "resume",
            Self::Clear => "clear",
            Self::Compact => "compact",
        }
    }
}

/// Trigger for a compaction event (`PreCompact` / `PostCompact`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactTrigger {
    Manual,
    Auto,
}

impl CompactTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

/// Per-event input payload. Variants for events this phase fires are
/// fully-typed; later phases add more variants (untagged enum growth is not a
/// breaking change). Each variant flattens [`CommonHookInput`] to the top
/// level of the emitted JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum HookInput {
    SessionStart {
        #[serde(flatten)]
        common: CommonHookInput,
        source: SessionStartSource,
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
    },
    SessionEnd {
        #[serde(flatten)]
        common: CommonHookInput,
        source: String,
    },
    UserPromptSubmit {
        #[serde(flatten)]
        common: CommonHookInput,
        prompt: String,
    },
    PreToolUse {
        #[serde(flatten)]
        common: CommonHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
    },
    PostToolUse {
        #[serde(flatten)]
        common: CommonHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_response: serde_json::Value,
        tool_use_id: String,
    },
    PostToolUseFailure {
        #[serde(flatten)]
        common: CommonHookInput,
        tool_name: String,
        tool_input: serde_json::Value,
        tool_use_id: String,
        error: String,
        is_interrupt: bool,
        duration_ms: u64,
    },
    PreCompact {
        #[serde(flatten)]
        common: CommonHookInput,
        trigger: CompactTrigger,
        /// Pre-compaction context fill ratio (tokens / window).
        usage_ratio: f64,
    },
    PostCompact {
        #[serde(flatten)]
        common: CommonHookInput,
        trigger: CompactTrigger,
        tier: u8,
        usage_ratio: f64,
    },
    Notification {
        #[serde(flatten)]
        common: CommonHookInput,
        notification_type: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    PostToolBatch {
        #[serde(flatten)]
        common: CommonHookInput,
        /// Zero-based API round index.
        round: u32,
        /// Names of every tool that ran in this round.
        tool_names: Vec<String>,
    },
    SubagentStart {
        #[serde(flatten)]
        common: CommonHookInput,
        /// The spawned sub-agent's id (also the matcher target).
        subagent_id: String,
        run_id: String,
    },
    SubagentStop {
        #[serde(flatten)]
        common: CommonHookInput,
        subagent_id: String,
        run_id: String,
        /// Terminal status: `completed` / `failed` / `cancelled` / …
        status: String,
    },
}

impl HookInput {
    /// Borrow the common fields regardless of variant.
    pub fn common(&self) -> &CommonHookInput {
        match self {
            Self::SessionStart { common, .. }
            | Self::SessionEnd { common, .. }
            | Self::UserPromptSubmit { common, .. }
            | Self::PreToolUse { common, .. }
            | Self::PostToolUse { common, .. }
            | Self::PostToolUseFailure { common, .. }
            | Self::PreCompact { common, .. }
            | Self::PostCompact { common, .. }
            | Self::Notification { common, .. }
            | Self::PostToolBatch { common, .. }
            | Self::SubagentStart { common, .. }
            | Self::SubagentStop { common, .. } => common,
        }
    }

    /// The matcher target for this input (design doc §6.2): tool name / source
    /// / notification_type / trigger. `None` means "no target → only wildcard
    /// matchers fire".
    pub fn matcher_target(&self) -> Option<&str> {
        match self {
            Self::SessionStart { source, .. } => Some(source.as_str()),
            Self::SessionEnd { source, .. } => Some(source.as_str()),
            Self::PreToolUse { tool_name, .. }
            | Self::PostToolUse { tool_name, .. }
            | Self::PostToolUseFailure { tool_name, .. } => Some(tool_name.as_str()),
            Self::PreCompact { trigger, .. } | Self::PostCompact { trigger, .. } => {
                Some(trigger.as_str())
            }
            Self::Notification {
                notification_type, ..
            } => Some(notification_type.as_str()),
            Self::SubagentStart { subagent_id, .. } | Self::SubagentStop { subagent_id, .. } => {
                Some(subagent_id.as_str())
            }
            // No matcher target → only wildcard matchers fire.
            Self::UserPromptSubmit { .. } | Self::PostToolBatch { .. } => None,
        }
    }
}

/// `hookSpecificOutput` object inside a hook's JSON stdout. Field names are
/// camelCase per design doc §8.2/§8.4. Only the fields used by landed events
/// are typed; unknown fields are ignored at parse time (a warn is logged by
/// the runner per §8.4).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    #[serde(default)]
    pub additional_context: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(default)]
    pub permission_decision: Option<String>,
    #[serde(default)]
    pub permission_decision_reason: Option<String>,
    #[serde(default)]
    pub updated_input: Option<serde_json::Value>,
}

/// Parsed JSON stdout of a hook (design doc §8.2). `continue` is a Rust
/// keyword so it is renamed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    #[serde(default = "default_true", rename = "continue")]
    pub continue_execution: bool,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub suppress_output: bool,
    #[serde(default)]
    pub system_message: Option<String>,
    /// Top-level `decision` (used by Stop / PostToolUse / ConfigChange / …).
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

fn default_true() -> bool {
    true
}

/// Aggregated decision across all matched hooks for one event. Observation
/// events only ever produce `Allow` (with optional `additional_context`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HookDecision {
    #[default]
    Allow,
    Ask,
    Defer,
    Deny {
        reason: String,
    },
    /// `decision: "block"` — non-blocking for observation events, degraded to
    /// log + notify (design doc §5.1.1).
    Block {
        reason: String,
    },
}

/// Placeholder for `updatedPermissions` entries (PermissionRequest, later PR).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionUpdate {
    #[serde(flatten)]
    pub raw: serde_json::Value,
}

/// The outcome the dispatcher hands back to a business call site. Defaults to
/// a no-op (`Allow`, continue, nothing injected) so callers can treat "no
/// hooks configured / all failed" as "nothing happened".
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub decision: HookDecision,
    /// `true` only when a hook returned an explicit `permissionDecision:"allow"`
    /// (PreToolUse), letting the tool gate skip the user approval prompt — as
    /// opposed to the default `Allow` a context-only hook produces, which must
    /// not skip anything.
    pub permission_allow: bool,
    pub continue_execution: bool,
    pub stop_reason: Option<String>,
    pub system_message: Option<String>,
    pub additional_context: Vec<String>,
    pub updated_input: Option<serde_json::Value>,
    pub updated_mcp_output: Option<serde_json::Value>,
    pub updated_permissions: Vec<PermissionUpdate>,
    pub session_title: Option<String>,
    pub retry: bool,
}

impl Default for HookOutcome {
    fn default() -> Self {
        Self::noop()
    }
}

impl HookOutcome {
    /// The "nothing happened" outcome.
    pub fn noop() -> Self {
        Self {
            decision: HookDecision::Allow,
            permission_allow: false,
            continue_execution: true,
            stop_reason: None,
            system_message: None,
            additional_context: Vec::new(),
            updated_input: None,
            updated_mcp_output: None,
            updated_permissions: Vec::new(),
            session_title: None,
            retry: false,
        }
    }

    /// Concatenate all injected context into one string (ordered, `---`
    /// separated), or `None` if nothing was injected.
    pub fn merged_additional_context(&self) -> Option<String> {
        if self.additional_context.is_empty() {
            return None;
        }
        Some(self.additional_context.join("\n\n---\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_pascal_case_roundtrip() {
        for (event, literal) in [
            (HookEvent::SessionStart, "\"SessionStart\""),
            (HookEvent::PostToolBatch, "\"PostToolBatch\""),
            (HookEvent::UserPromptExpansion, "\"UserPromptExpansion\""),
            (HookEvent::WorktreeRemove, "\"WorktreeRemove\""),
        ] {
            let json = serde_json::to_string(&event).unwrap();
            assert_eq!(json, literal);
            let back: HookEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
            assert_eq!(format!("\"{}\"", event.as_str()), literal);
        }
    }

    #[test]
    fn permission_mode_resolve_precedence() {
        // Plan active wins over session mode (even YOLO).
        assert_eq!(
            PermissionMode::resolve(SessionMode::Yolo, PlanModeState::Planning),
            PermissionMode::Plan
        );
        assert_eq!(
            PermissionMode::resolve(SessionMode::Default, PlanModeState::Executing),
            PermissionMode::Plan
        );
        // No active plan → session mode maps directly.
        assert_eq!(
            PermissionMode::resolve(SessionMode::Default, PlanModeState::Off),
            PermissionMode::Default
        );
        assert_eq!(
            PermissionMode::resolve(SessionMode::Yolo, PlanModeState::Completed),
            PermissionMode::BypassPermissions
        );
        assert_eq!(
            PermissionMode::resolve(SessionMode::Smart, PlanModeState::Off),
            PermissionMode::Other
        );
    }

    #[test]
    fn permission_mode_serializes_to_protocol_strings() {
        assert_eq!(
            serde_json::to_string(&PermissionMode::BypassPermissions).unwrap(),
            "\"bypassPermissions\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionMode::Plan).unwrap(),
            "\"plan\""
        );
    }

    #[test]
    fn common_fields_flatten_into_event_json() {
        let input = HookInput::PostToolUse {
            common: CommonHookInput {
                session_id: "s1".into(),
                transcript_path: PathBuf::from("/tmp/t.jsonl"),
                cwd: PathBuf::from("/work"),
                permission_mode: PermissionMode::Default,
                hook_event_name: "PostToolUse".into(),
                agent_id: Some("ha-main".into()),
                agent_type: None,
            },
            tool_name: "Write".into(),
            tool_input: serde_json::json!({"path": "/a"}),
            tool_response: serde_json::json!("ok"),
            tool_use_id: "call_1".into(),
        };
        let v = serde_json::to_value(&input).unwrap();
        // Common fields are flattened to the top level.
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["hook_event_name"], "PostToolUse");
        assert_eq!(v["permission_mode"], "default");
        assert_eq!(v["tool_name"], "Write");
        assert_eq!(v["tool_use_id"], "call_1");
        // agent_type is None → omitted.
        assert!(v.get("agent_type").is_none());
    }

    #[test]
    fn matcher_target_per_event() {
        let common = CommonHookInput {
            session_id: "s".into(),
            transcript_path: PathBuf::new(),
            cwd: PathBuf::new(),
            permission_mode: PermissionMode::Default,
            hook_event_name: "x".into(),
            agent_id: None,
            agent_type: None,
        };
        let pre = HookInput::PreToolUse {
            common: common.clone(),
            tool_name: "Bash".into(),
            tool_input: serde_json::Value::Null,
            tool_use_id: "c".into(),
        };
        assert_eq!(pre.matcher_target(), Some("Bash"));
        let ups = HookInput::UserPromptSubmit {
            common,
            prompt: "hi".into(),
        };
        assert_eq!(ups.matcher_target(), None);
    }

    #[test]
    fn hook_output_defaults_continue_true() {
        let out: HookOutput = serde_json::from_str("{}").unwrap();
        assert!(out.continue_execution);
        assert!(out.decision.is_none());
        let out2: HookOutput =
            serde_json::from_str(r#"{"continue": false, "stopReason": "stop"}"#).unwrap();
        assert!(!out2.continue_execution);
        assert_eq!(out2.stop_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn hook_specific_output_camel_case() {
        let out: HookOutput = serde_json::from_str(
            r#"{"hookSpecificOutput": {"additionalContext": "ctx", "sessionTitle": "T"}}"#,
        )
        .unwrap();
        let hso = out.hook_specific_output.unwrap();
        assert_eq!(hso.additional_context.as_deref(), Some("ctx"));
        assert_eq!(hso.session_title.as_deref(), Some("T"));
    }

    #[test]
    fn outcome_noop_is_inert() {
        let n = HookOutcome::noop();
        assert_eq!(n.decision, HookDecision::Allow);
        assert!(n.continue_execution);
        assert!(n.additional_context.is_empty());
        assert!(n.merged_additional_context().is_none());
    }
}

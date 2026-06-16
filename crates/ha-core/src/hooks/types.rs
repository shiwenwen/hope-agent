//! Core hook data structures: events, inputs, outputs, decisions.
//!
//! Field-level aligned with the Claude Code hooks protocol (see
//! `docs/architecture/hooks.md`). Event names use PascalCase
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

    /// Whether this event's matcher target is a tool name (so
    /// `matcher::compile_for_event` can normalize Claude Code aliases like
    /// `Bash`/`Write` to Hope Agent's internal `exec`/`write` before compile).
    /// Pure metadata; keep in sync with [`HookInput::matcher_target`].
    pub fn uses_tool_name_matcher(&self) -> bool {
        matches!(
            self,
            Self::PreToolUse | Self::PostToolUse | Self::PostToolUseFailure
        )
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
                | Self::PostToolBatch
                | Self::SubagentStart
                | Self::SubagentStop
                // Stop / StopFailure fire fire-and-forget this phase (no
                // block-to-continue), so a stray blocking decision is
                // downgraded + logged like any other observation event. They
                // move out of this list when block-to-continue lands.
                | Self::Stop
                | Self::StopFailure
                | Self::TaskCreated
                | Self::TaskCompleted
                | Self::ConfigChange
                | Self::CwdChanged
                | Self::FileChanged
                | Self::PermissionRequest
                | Self::PermissionDenied
                | Self::UserPromptExpansion
                | Self::Elicitation
                | Self::ElicitationResult
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
        /// `Some` only for an async tool job's *terminal* fire (HOOKS-1). Lets a
        /// hook tell this apart from the `started`-time `PreToolUse` carrying the
        /// same `tool_use_id` (the synthetic "started" result is returned at
        /// submission; this fires later when the background work settles).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
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
        /// `Some` only for an async tool job's terminal failure fire (HOOKS-1/4);
        /// see [`Self::PostToolUse::job_id`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
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
    Stop {
        #[serde(flatten)]
        common: CommonHookInput,
        /// Terminal turn status: `completed` / `interrupted`.
        status: String,
        /// Claude Code's `stop_hook_active` — `true` when a Stop hook is
        /// already in the continue loop. Block-to-continue is not implemented
        /// yet, so always `false`; the field keeps the payload field-aligned.
        stop_hook_active: bool,
    },
    StopFailure {
        #[serde(flatten)]
        common: CommonHookInput,
        /// Failure category (matcher target): `provider_failed` /
        /// `compaction_failed` / `shutdown` / `crash` / `no_profile` / `other`.
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    TaskCreated {
        #[serde(flatten)]
        common: CommonHookInput,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_form: Option<String>,
        /// Shared id for every task created in one `task_create` call.
        batch_id: String,
    },
    TaskCompleted {
        #[serde(flatten)]
        common: CommonHookInput,
        task_id: i64,
        content: String,
    },
    ConfigChange {
        #[serde(flatten)]
        common: CommonHookInput,
        /// Config category (matcher target): `hooks` / `permission` /
        /// `web_search` / `app` / …
        category: String,
        /// Who triggered the change (`user` / `skill` / `reload` / …).
        source: String,
    },
    CwdChanged {
        #[serde(flatten)]
        common: CommonHookInput,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        new_cwd: Option<String>,
    },
    FileChanged {
        #[serde(flatten)]
        common: CommonHookInput,
        /// Absolute path of the changed file (matcher target — a regex matcher
        /// like `.*\.rs$` scopes the hook to a file pattern).
        path: String,
        /// `create` / `edit` / `delete` / `patch`.
        action: String,
    },
    PermissionRequest {
        #[serde(flatten)]
        common: CommonHookInput,
        /// The command / tool being approved (matcher target). For `exec` this
        /// is the shell command; for the tool gate, a `tool: <name> <args>` desc.
        command: String,
        /// The tool_use this approval belongs to, for correlation with
        /// PreToolUse/PostToolUse. `None` when the call site has no id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        /// `Some` when the approval gates a backgrounded call, for async-job
        /// correlation. Currently always `None` (approval runs before detach).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
    },
    PermissionDenied {
        #[serde(flatten)]
        common: CommonHookInput,
        /// The command / tool that was denied (matcher target).
        command: String,
        /// Why: `user_declined` (the user said no) / `policy` (engine auto-deny).
        reason: String,
        /// The tool_use this denial belongs to, for correlation. `None` when the
        /// call site has no id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        /// `Some` when the denied call would have backgrounded. Currently always
        /// `None` (approval runs before detach).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        job_id: Option<String>,
    },
    UserPromptExpansion {
        #[serde(flatten)]
        common: CommonHookInput,
        /// The slash command name (matcher target, e.g. `plan`, `deploy`).
        command: String,
        /// The full raw command text including args.
        command_text: String,
    },
    Elicitation {
        #[serde(flatten)]
        common: CommonHookInput,
        /// `ask_user_question` group id (correlates with ElicitationResult).
        request_id: String,
        question_count: usize,
    },
    ElicitationResult {
        #[serde(flatten)]
        common: CommonHookInput,
        request_id: String,
        /// `answered` / `cancelled` / `timeout`.
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
            | Self::SubagentStop { common, .. }
            | Self::Stop { common, .. }
            | Self::StopFailure { common, .. }
            | Self::TaskCreated { common, .. }
            | Self::TaskCompleted { common, .. }
            | Self::ConfigChange { common, .. }
            | Self::CwdChanged { common, .. }
            | Self::FileChanged { common, .. }
            | Self::PermissionRequest { common, .. }
            | Self::PermissionDenied { common, .. }
            | Self::UserPromptExpansion { common, .. }
            | Self::Elicitation { common, .. }
            | Self::ElicitationResult { common, .. } => common,
        }
    }

    /// Whether this input fires on a gate-capable event whose degraded
    /// delivery paths must fail **closed**. Lists exactly the events that flow
    /// through a blocking decision sink (PreToolUse gate, UserPromptSubmit
    /// preflight, PreCompact). For these, a handler that can't run at all
    /// (spawn failure, IO error, timeout, unreachable endpoint) must be
    /// treated as `Block` rather than silently inert — a silent fall-through
    /// to `Allow` would defeat the very gate the hook exists to enforce.
    /// Observation-only events are excluded: `Block` from them is downgraded
    /// by [`HookEvent::is_observation_only`] anyway, and fail-closing them
    /// would hide real errors without buying any security. Every runner
    /// (`command`, `http`) consults this on its infra-failure branches.
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            Self::PreToolUse { .. } | Self::UserPromptSubmit { .. } | Self::PreCompact { .. }
        )
    }

    /// The tool name for tool-lifecycle events (`PreToolUse` / `PostToolUse` /
    /// `PostToolUseFailure`); `None` for every other event. Used by the `if`
    /// condition gate — a non-tool event can't satisfy a `ToolName(...)` rule.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::PreToolUse { tool_name, .. }
            | Self::PostToolUse { tool_name, .. }
            | Self::PostToolUseFailure { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        }
    }

    /// The tool input args for tool-lifecycle events; `None` otherwise.
    pub fn tool_input(&self) -> Option<&serde_json::Value> {
        match self {
            Self::PreToolUse { tool_input, .. }
            | Self::PostToolUse { tool_input, .. }
            | Self::PostToolUseFailure { tool_input, .. } => Some(tool_input),
            _ => None,
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
            // StopFailure matches on its failure category (`provider_failed`, …).
            Self::StopFailure { reason, .. } => Some(reason.as_str()),
            // ConfigChange matches on the config category (`hooks`, `permission`, …).
            Self::ConfigChange { category, .. } => Some(category.as_str()),
            // FileChanged matches on the file path (regex matcher → file pattern).
            Self::FileChanged { path, .. } => Some(path.as_str()),
            // Permission events match on the command / tool being gated.
            Self::PermissionRequest { command, .. } | Self::PermissionDenied { command, .. } => {
                Some(command.as_str())
            }
            // UserPromptExpansion matches on the slash command name.
            Self::UserPromptExpansion { command, .. } => Some(command.as_str()),
            // No matcher target → only wildcard matchers fire. Task content is
            // freeform; elicitation ids are UUIDs — all match wildcard only.
            Self::UserPromptSubmit { .. }
            | Self::PostToolBatch { .. }
            | Self::Stop { .. }
            | Self::TaskCreated { .. }
            | Self::TaskCompleted { .. }
            | Self::CwdChanged { .. }
            | Self::Elicitation { .. }
            | Self::ElicitationResult { .. } => None,
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
            job_id: None,
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
            common: common.clone(),
            prompt: "hi".into(),
        };
        assert_eq!(ups.matcher_target(), None);
        // Stop has no matcher target; StopFailure matches on its category.
        let stop = HookInput::Stop {
            common: common.clone(),
            status: "completed".into(),
            stop_hook_active: false,
        };
        assert_eq!(stop.matcher_target(), None);
        let fail = HookInput::StopFailure {
            common,
            reason: "provider_failed".into(),
            error: Some("boom".into()),
        };
        assert_eq!(fail.matcher_target(), Some("provider_failed"));
    }

    #[test]
    fn stop_failure_omits_none_error() {
        let common = CommonHookInput {
            session_id: "s".into(),
            transcript_path: PathBuf::new(),
            cwd: PathBuf::new(),
            permission_mode: PermissionMode::Default,
            hook_event_name: "StopFailure".into(),
            agent_id: None,
            agent_type: None,
        };
        let fail = HookInput::StopFailure {
            common,
            reason: "shutdown".into(),
            error: None,
        };
        let v = serde_json::to_value(&fail).unwrap();
        assert_eq!(v["reason"], "shutdown");
        assert!(v.get("error").is_none());
        assert_eq!(v["session_id"], "s");
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

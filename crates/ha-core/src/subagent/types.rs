use serde::{Deserialize, Serialize};

// ── Data Structures ─────────────────────────────────────────────

/// Sub-agent run status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    /// R7.2 reject→queue unification: parked because the parent session is at its
    /// per-session subagent concurrency limit. NOT a slot holder and NOT terminal
    /// — promoted to `Spawning` by the subagent scheduler when a slot frees.
    /// Deliberately excluded from the `count_active_subagent_runs` active set so a
    /// queued run can't inflate the count and deadlock its own promotion.
    Queued,
    Spawning,
    Running,
    Completed,
    Error,
    Timeout,
    Killed,
    /// The process/runner that owned this attempt disappeared before it could
    /// produce a trustworthy terminal result. Unlike `Error`, this is an
    /// infrastructure interruption and may be suitable for an explicit
    /// continuation in the same thread.
    Interrupted,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Spawning => "spawning",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Killed => "killed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "spawning" => Self::Spawning,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "error" => Self::Error,
            "timeout" => Self::Timeout,
            "killed" => Self::Killed,
            "interrupted" => Self::Interrupted,
            _ => Self::Error,
        }
    }

    /// Whether this status represents a terminal (finished) state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Error | Self::Timeout | Self::Killed | Self::Interrupted
        )
    }
}

/// Stable control-plane owner for a sub-agent thread.
///
/// Ownership is deliberately separate from `parent_session_id`: Workflow,
/// Team, and internal helper children can share a parent chat while retaining
/// a narrower control surface. A plain parent-session tool call must never take
/// over one of those threads merely because it knows a run id.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentOwnerKind {
    ParentSession,
    Workflow,
    Team,
    Internal,
}

impl SubagentOwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentSession => "parent_session",
            Self::Workflow => "workflow",
            Self::Team => "team",
            Self::Internal => "internal",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "parent_session" => Self::ParentSession,
            "workflow" => Self::Workflow,
            "team" => Self::Team,
            _ => Self::Internal,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentThreadState {
    Open,
    UserStopped,
    Quarantined,
    Closed,
}

impl SubagentThreadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::UserStopped => "user_stopped",
            Self::Quarantined => "quarantined",
            Self::Closed => "closed",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "open" => Self::Open,
            "user_stopped" => Self::UserStopped,
            "quarantined" => Self::Quarantined,
            _ => Self::Closed,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentDeliveryKind {
    Parent,
    Group,
    Workflow,
    None,
}

impl SubagentDeliveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Group => "group",
            Self::Workflow => "workflow",
            Self::None => "none",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "parent" => Self::Parent,
            "group" => Self::Group,
            "workflow" => Self::Workflow,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTerminalReason {
    Success,
    ProviderExhausted,
    /// The active tool-result group cannot fit even in its protocol-minimal
    /// envelope after bounded old-history recovery. This is not a provider
    /// outage and must never enter automatic provider-recovery continuation.
    CurrentToolGroupOverflow,
    /// A Provider request crossed its dispatch claim, but receipt/processing
    /// could not be proven. Automatic continuation could duplicate work or
    /// billing, so only an explicit manual continuation is appropriate.
    DispatchUnknown,
    ModelError,
    ToolError,
    DeadlineExceeded,
    ProcessInterrupted,
    RunnerPanic,
    InvalidTypedOutput,
    ApprovalDenied,
    UserKilled,
    ParentCancelled,
    WorkflowCancelled,
    /// A session-level Stop interrupted this immutable attempt without closing
    /// its child thread. Explicit Continue may create a fresh continuation.
    SessionPaused,
    QueuePayloadUnavailable,
    Unknown,
}

impl SubagentTerminalReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ProviderExhausted => "provider_exhausted",
            Self::CurrentToolGroupOverflow => "current_tool_group_overflow",
            Self::DispatchUnknown => "dispatch_unknown",
            Self::ModelError => "model_error",
            Self::ToolError => "tool_error",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::ProcessInterrupted => "process_interrupted",
            Self::RunnerPanic => "runner_panic",
            Self::InvalidTypedOutput => "invalid_typed_output",
            Self::ApprovalDenied => "approval_denied",
            Self::UserKilled => "user_killed",
            Self::ParentCancelled => "parent_cancelled",
            Self::WorkflowCancelled => "workflow_cancelled",
            Self::SessionPaused => "session_paused",
            Self::QueuePayloadUnavailable => "queue_payload_unavailable",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "success" => Self::Success,
            "provider_exhausted" => Self::ProviderExhausted,
            "current_tool_group_overflow" => Self::CurrentToolGroupOverflow,
            "dispatch_unknown" => Self::DispatchUnknown,
            "model_error" => Self::ModelError,
            "tool_error" => Self::ToolError,
            "deadline_exceeded" => Self::DeadlineExceeded,
            "process_interrupted" => Self::ProcessInterrupted,
            "runner_panic" => Self::RunnerPanic,
            "invalid_typed_output" => Self::InvalidTypedOutput,
            "approval_denied" => Self::ApprovalDenied,
            "user_killed" => Self::UserKilled,
            "parent_cancelled" => Self::ParentCancelled,
            "workflow_cancelled" => Self::WorkflowCancelled,
            "session_paused" => Self::SessionPaused,
            "queue_payload_unavailable" => Self::QueuePayloadUnavailable,
            _ => Self::Unknown,
        }
    }

    /// Diagnostic hint only. Callers must still re-check ownership, thread
    /// state, permissions, and their own bounded retry policy.
    pub fn resume_recommended(self) -> bool {
        matches!(
            self,
            Self::ProviderExhausted
                | Self::DeadlineExceeded
                | Self::ProcessInterrupted
                | Self::SessionPaused
        )
    }

    pub fn resume_allowed(self) -> bool {
        !matches!(
            self,
            Self::ApprovalDenied
                | Self::UserKilled
                | Self::ParentCancelled
                | Self::WorkflowCancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentThread {
    pub thread_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub depth: u32,
    pub owner_kind: SubagentOwnerKind,
    pub owner_id: String,
    pub lifecycle_state: SubagentThreadState,
    pub current_run_id: Option<String>,
    pub lease_epoch: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// A sub-agent run record persisted in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub run_id: String,
    /// Stable public identity for the child conversation. This is equal to
    /// `child_session_id`; both are returned during the compatibility window.
    pub thread_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task: String,
    pub status: SubagentStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub depth: u32,
    pub model_used: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    /// Optional display label for tracking
    pub label: Option<String>,
    /// Number of file attachments passed to the sub-agent
    pub attachment_count: u32,
    /// Input token usage (if available)
    pub input_tokens: Option<u64>,
    /// Output token usage (if available)
    pub output_tokens: Option<u64>,
    /// Immutable predecessor in the same thread, if this is a continuation.
    pub continuation_of_run_id: Option<String>,
    /// Stable audit source: spawn / parent_followup / workflow_resume / ...
    pub trigger_kind: String,
    pub terminal_reason: Option<SubagentTerminalReason>,
    pub runner_owner: Option<String>,
    pub lease_epoch: u64,
    pub last_heartbeat_at: Option<String>,
    pub delivery_kind: SubagentDeliveryKind,
    /// Non-sensitive execution recipe used for deterministic continuation and
    /// recovery. Provider credentials and resolved profile state are never
    /// persisted here.
    pub launch_spec_json: Option<String>,
    pub owner_kind: SubagentOwnerKind,
    pub owner_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SubagentProviderRecovery {
    pub run_id: String,
    pub thread_id: String,
    pub owner_kind: SubagentOwnerKind,
    pub owner_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
    pub next_attempt_at: String,
}

impl Default for SubagentRun {
    fn default() -> Self {
        Self {
            run_id: String::new(),
            thread_id: String::new(),
            parent_session_id: String::new(),
            parent_agent_id: String::new(),
            child_agent_id: String::new(),
            child_session_id: String::new(),
            task: String::new(),
            status: SubagentStatus::Error,
            result: None,
            error: None,
            depth: 0,
            model_used: None,
            started_at: String::new(),
            finished_at: None,
            duration_ms: None,
            label: None,
            attachment_count: 0,
            input_tokens: None,
            output_tokens: None,
            continuation_of_run_id: None,
            trigger_kind: "spawn".to_string(),
            terminal_reason: None,
            runner_owner: None,
            lease_epoch: 1,
            last_heartbeat_at: None,
            delivery_kind: SubagentDeliveryKind::Parent,
            launch_spec_json: None,
            owner_kind: SubagentOwnerKind::ParentSession,
            owner_id: String::new(),
        }
    }
}

/// Parameters for spawning a sub-agent.
#[derive(Debug, Clone)]
pub struct SpawnParams {
    pub task: String,
    pub agent_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub depth: u32,
    pub timeout_secs: Option<u64>,
    pub model_override: Option<String>,
    /// Optional display label for tracking
    pub label: Option<String>,
    /// Create a managed git worktree for this child session when possible.
    /// User-delegated subagents enable this for file isolation; internal
    /// helper spawns leave it off unless they explicitly need isolation.
    pub isolate_worktree: bool,
    /// File attachments to pass to the sub-agent
    pub attachments: Vec<crate::agent::Attachment>,
    /// Plan agent mode to configure on the sub-agent (None = normal sub-agent)
    pub plan_agent_mode: Option<crate::agent::PlanAgentMode>,
    /// Path allow-list for plan mode file writes (plans/ directory)
    pub plan_mode_allow_paths: Vec<String>,
    /// True when the spawn caller is the source of truth for `plan_agent_mode`
    /// (set by `spawn_plan_subagent`). The streaming loop's mid-turn probe
    /// will skip overwriting this with the child session's backend plan
    /// state — without the flag, the probe sees `Off` in the freshly-created
    /// child session and clobbers the explicit `PlanAgent` mode that the
    /// spawn caller configured, breaking the plan-creation subagent.
    pub lock_plan_agent_mode: bool,
    /// If true, skip automatic result injection into parent conversation
    pub skip_parent_injection: bool,
    /// Additional platform-maintained run instruction for the child. The child
    /// task itself remains the user message and must not be copied here.
    pub run_instruction_context: Option<String>,
    /// User/model-authored metadata associated with the child run (for example
    /// a Team roster or shared context). Adapters always keep this in the
    /// untrusted run-data lane, never in system/developer authority.
    pub run_data_context: Option<String>,
    /// Skill-level tool restriction inherited from parent skill activation.
    /// When non-empty, the sub-agent only has access to these tools.
    pub skill_allowed_tools: Vec<String>,
    /// Reasoning / thinking effort forwarded to the provider on the sub-agent's
    /// `chat` call. Skills set this from their `effort:` frontmatter; other
    /// callers leave `None` to fall back to provider/agent defaults.
    pub reasoning_effort: Option<String>,
    /// Skill name when spawned by a `context: fork` skill — propagated to
    /// `SubagentEvent.skill_name` so the frontend can pick the dedicated
    /// SkillProgressBlock renderer. `None` for every other caller.
    pub skill_name: Option<String>,
    /// Parent turn's KB-access origin (design D10), forwarded to the child's
    /// `ChatEngineParams.origin_source` so an IM-origin chain can't reacquire KB
    /// access through the neutral `Subagent` source. The `subagent` tool sets it
    /// from the parent `ToolExecContext`; system-initiated spawns (plan / team /
    /// hooks / fork skill) leave it `None` and rely on subagent session
    /// isolation (fresh child session, no project) for the same guarantee.
    pub origin_source: Option<crate::knowledge::KbAccessSource>,
    /// IM origin identity (WS8), forwarded to the child's
    /// `ChatEngineParams.channel_kb_context` so an IM-origin subagent's KB opt-in
    /// is judged against the account/chat that started the chain — not the
    /// neutral `Subagent` source. The `subagent` tool sets it from the parent
    /// `ToolExecContext`; system-initiated spawns leave it `None`.
    pub origin_channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
    /// R5 (Group fan-out): when this spawn is one child of a `batch_spawn`
    /// Group, the owning Group's `job_id`. The child's `background_jobs`
    /// projection records it, and the child's individual completion injection is
    /// **suppressed** — the Group fires ONE merged injection when all children
    /// settle. `None` for a standalone spawn (per-child injection, unchanged).
    pub group_id: Option<String>,
    /// Durable control-plane owner of the child thread.
    pub owner_kind: SubagentOwnerKind,
    pub owner_id: String,
    /// Durable delivery domain. This is persisted on every attempt and is the
    /// execution-layer source of truth; `skip_parent_injection` remains only as
    /// a compatibility input while callers migrate.
    pub delivery_kind: SubagentDeliveryKind,
}

/// Event payload for streaming parent agent responses back to frontend.
/// Emitted when a sub-agent completes and the backend auto-injects the result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentAgentStreamEvent {
    pub event_type: String, // "started" | "delta" | "done" | "error"
    pub parent_session_id: String,
    pub run_id: String,
    pub push_message: Option<String>, // only for "started"
    pub delta: Option<String>,        // raw JSON delta string, only for "delta"
    pub error: Option<String>,        // only for "error"
}

#[cfg(test)]
mod status_tests {
    use super::{SubagentStatus, SubagentTerminalReason};

    #[test]
    fn queued_round_trips_and_is_non_terminal() {
        // R7.2: the parked status must serialize stably (it's persisted to the
        // `subagent_runs.status` TEXT column and read back) and must NOT be
        // terminal — a terminal Queued would freeze the projection and the
        // active-count exclusion would be meaningless.
        assert_eq!(SubagentStatus::Queued.as_str(), "queued");
        assert_eq!(SubagentStatus::from_str("queued"), SubagentStatus::Queued);
        assert!(!SubagentStatus::Queued.is_terminal());
        // Unknown still falls back to Error (unchanged).
        assert_eq!(SubagentStatus::from_str("bogus"), SubagentStatus::Error);
    }

    #[test]
    fn current_tool_group_overflow_is_durable_and_not_auto_recommended() {
        let reason = SubagentTerminalReason::CurrentToolGroupOverflow;
        assert_eq!(reason.as_str(), "current_tool_group_overflow");
        assert_eq!(SubagentTerminalReason::from_str(reason.as_str()), reason);
        assert!(!reason.resume_recommended());
        assert!(
            reason.resume_allowed(),
            "an explicit continuation may compact the now-historical group"
        );
    }

    #[test]
    fn dispatch_unknown_is_durable_and_not_auto_recommended() {
        let reason = SubagentTerminalReason::DispatchUnknown;
        assert_eq!(reason.as_str(), "dispatch_unknown");
        assert_eq!(SubagentTerminalReason::from_str(reason.as_str()), reason);
        assert!(!reason.resume_recommended());
        assert!(
            reason.resume_allowed(),
            "a user may explicitly continue after checking Provider activity"
        );
    }
}

/// Event payload emitted to the frontend via Tauri events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub event_type: String,
    pub run_id: String,
    pub parent_session_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task_preview: String,
    pub status: SubagentStatus,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    /// Optional display label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Input tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Full result text — included only in terminal events for push delivery.
    /// Frontend uses this to auto-inject the result into the parent agent's conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_full: Option<String>,
    /// Skill name when this sub-agent was spawned by a `context: fork` skill.
    /// The frontend uses it to mount the dedicated SkillProgressBlock renderer
    /// instead of the generic SubagentGroup. `None` for `/subagent` spawns,
    /// team members, and every other caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
}

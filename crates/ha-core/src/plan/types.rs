use serde::{Deserialize, Serialize};

// ── Plan Mode State ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeState {
    #[default]
    Off,
    Planning,
    Review,
    Executing,
    Paused,
    Completed,
}

impl PlanModeState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::Planning => "planning",
            Self::Review => "review",
            Self::Executing => "executing",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "planning" => Self::Planning,
            "review" => Self::Review,
            "executing" => Self::Executing,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            _ => Self::Off,
        }
    }

    /// Whether `self → next` is a legal Plan Mode state transition.
    ///
    /// Keeps the six-state machine well-formed so concurrent writers can't
    /// flip `Completed → Executing` and re-run already-done steps, or skip
    /// straight to `Executing` without going through a `Review` checkpoint.
    /// Same-state "transitions" (e.g. re-asserting `Planning` after a
    /// persistence round-trip) are always allowed.
    pub fn is_valid_transition(&self, next: &PlanModeState) -> bool {
        if self == next {
            return true;
        }
        // Entering or leaving Plan Mode entirely is always valid — callers
        // need an escape hatch for cancelled / deleted sessions.
        if matches!(next, PlanModeState::Off) || matches!(self, PlanModeState::Off) {
            return true;
        }
        match (self, next) {
            // Normal forward flow.
            (PlanModeState::Planning, PlanModeState::Review) => true,
            (PlanModeState::Review, PlanModeState::Planning) => true,
            (PlanModeState::Review, PlanModeState::Executing) => true,
            (PlanModeState::Executing, PlanModeState::Paused) => true,
            (PlanModeState::Executing, PlanModeState::Completed) => true,
            (PlanModeState::Paused, PlanModeState::Executing) => true,
            (PlanModeState::Paused, PlanModeState::Planning) => true,
            // Post-completion revisions are allowed back into Planning only.
            (PlanModeState::Completed, PlanModeState::Planning) => true,
            _ => false,
        }
    }
}

// ── Plan Step ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
    Failed,
}

impl PlanStepStatus {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Skipped | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub index: usize,
    pub phase: String,
    pub title: String,
    pub description: String,
    pub status: PlanStepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

// ── Plan Metadata ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub file_path: String,
    pub state: PlanModeState,
    pub steps: Vec<PlanStep>,
    pub created_at: String,
    pub updated_at: String,
    /// Step index where execution was paused (for Paused state)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_at_step: Option<usize>,
    /// Plan version counter (incremented on each save/edit)
    #[serde(default = "default_version")]
    pub version: u32,
    /// Git checkpoint reference (branch or stash) created before execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<String>,
}

fn default_version() -> u32 {
    1
}

impl PlanMeta {
    #[allow(dead_code)]
    pub fn completed_count(&self) -> usize {
        self.steps.iter().filter(|s| s.status.is_terminal()).count()
    }

    pub fn all_terminal(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.status.is_terminal())
    }
}

/// Info about a plan version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVersionInfo {
    pub version: u32,
    pub file_path: String,
    pub modified_at: String,
    pub is_current: bool,
}

// ── Plan Agent / Executing Agent Configuration ─────────────────────

/// Declarative configuration for the Plan Agent (Planning/Review states).
/// Uses an **allow-list** approach: only listed tools are available.
pub struct PlanAgentConfig {
    /// Tool allow-list: only these tools are available to the Plan Agent
    pub allowed_tools: Vec<String>,
    /// Path restrictions for write/edit (only .md in plans/ directory)
    pub plan_mode_allow_paths: Vec<String>,
    /// Tools that require user approval (e.g., exec)
    pub ask_tools: Vec<String>,
}

impl PlanAgentConfig {
    pub fn default_config() -> Self {
        Self {
            allowed_tools: vec![
                // Read-only exploration tools
                "read",
                "ls",
                "grep",
                "find",
                "glob",
                "web_search",
                "web_fetch",
                // Restricted execution (requires approval)
                "exec",
                // Plan-specific tools
                "ask_user_question",
                "submit_plan",
                // Path-restricted write tools (only plans/ directory)
                "write",
                "edit",
                // Memory and delegation
                "recall_memory",
                "memory_get",
                "subagent",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            plan_mode_allow_paths: vec!["plans".into()],
            ask_tools: vec!["exec".into()],
        }
    }
}

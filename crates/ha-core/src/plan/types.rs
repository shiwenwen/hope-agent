use serde::{Deserialize, Serialize};

// ── Plan Mode State ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeState {
    #[default]
    Off,
    Planning,
    Review,
    Executing,
    Completed,
}

impl PlanModeState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::Planning => "planning",
            Self::Review => "review",
            Self::Executing => "executing",
            Self::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "planning" => Self::Planning,
            "review" => Self::Review,
            "executing" => Self::Executing,
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
            (PlanModeState::Executing, PlanModeState::Completed) => true,
            // Re-entry: Executing/Completed back into Planning to revise the
            // approved plan (replaces the deleted amend_plan tool path).
            (PlanModeState::Executing, PlanModeState::Planning) => true,
            (PlanModeState::Completed, PlanModeState::Planning) => true,
            _ => false,
        }
    }
}

// ── Plan Metadata ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub file_path: String,
    pub state: PlanModeState,
    pub created_at: String,
    pub updated_at: String,
    /// Plan version counter (incremented on each save/edit)
    #[serde(default = "default_version")]
    pub version: u32,
    /// Git checkpoint reference (branch or stash) created before execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_ref: Option<String>,
    /// Wall-clock RFC3339 timestamp set when the plan most recently entered
    /// the Executing state. Used by `maybe_complete_plan` to scope the
    /// "all tasks done" check to tasks created after execution started, so
    /// pre-existing pending tasks (or tasks from a prior plan run) don't
    /// block / falsely trigger auto-completion.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub executing_started_at: Option<String>,
}

fn default_version() -> u32 {
    1
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

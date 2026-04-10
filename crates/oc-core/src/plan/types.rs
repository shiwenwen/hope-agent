use serde::{Deserialize, Serialize};

// ── Plan Mode State ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanModeState {
    Off,
    Planning,
    Review,
    Executing,
    Paused,
    Completed,
}

impl Default for PlanModeState {
    fn default() -> Self {
        Self::Off
    }
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

// ── Ask User Question (Interactive Q&A, legacy name: Plan Question) ──
//
// These types back the generic `ask_user_question` tool. The struct names keep
// the `PlanQuestion*` prefix for backwards compatibility with serialized
// session history and the long-standing `plan_question_request` event, but the
// feature is now available outside Plan Mode.

/// A single question option for the user to choose from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionOption {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether this option is recommended/suggested as the default choice.
    #[serde(default)]
    pub recommended: bool,
    /// Optional rich preview body rendered when this option is focused.
    /// Supports markdown (code blocks, tables), image URLs, or mermaid diagrams.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Preview content kind: `markdown` (default), `image`, or `mermaid`.
    #[serde(skip_serializing_if = "Option::is_none", rename = "previewKind")]
    pub preview_kind: Option<String>,
}

/// A structured question sent by LLM to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestion {
    pub question_id: String,
    pub text: String,
    pub options: Vec<PlanQuestionOption>,
    #[serde(default = "crate::default_true")]
    pub allow_custom: bool,
    #[serde(default)]
    pub multi_select: bool,
    /// Optional question template/category (e.g., "scope", "tech_choice", "priority")
    /// Used to render category-specific UI styling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Very short chip label (max ~12 chars) displayed next to the question text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Per-question timeout in seconds. 0 or missing = inherit group / global default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Values automatically selected when the question times out. Each entry must
    /// match an option value, or can be a free-form string for custom input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_values: Vec<String>,
}

/// A group of questions sent together.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionGroup {
    pub request_id: String,
    pub session_id: String,
    pub questions: Vec<PlanQuestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Where this question originated from: "plan" | "normal" | skill id.
    /// Used by the UI and listeners for routing / styling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// UNIX timestamp (seconds) after which pending answers auto-fall back to defaults.
    /// `None` means no overall timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_at: Option<u64>,
}

/// User's answer to a single question
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQuestionAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_input: Option<String>,
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

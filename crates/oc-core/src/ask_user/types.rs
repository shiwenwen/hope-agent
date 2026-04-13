use serde::{Deserialize, Serialize};

// ── Ask User Question (Interactive Q&A) ──

/// A single question option for the user to choose from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionOption {
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
pub struct AskUserQuestion {
    pub question_id: String,
    pub text: String,
    pub options: Vec<AskUserQuestionOption>,
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
pub struct AskUserQuestionGroup {
    pub request_id: String,
    pub session_id: String,
    pub questions: Vec<AskUserQuestion>,
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
pub struct AskUserQuestionAnswer {
    pub question_id: String,
    pub selected: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_input: Option<String>,
}

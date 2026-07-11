use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ── Localizable Text ─────────────────────────────────────────────

/// Text shown in an ask-user prompt.
///
/// Backward compatibility is intentional here: older tool calls and persisted
/// rows store plain strings, while backend-owned prompts can now send an i18n
/// key plus interpolation params for the desktop/web UI. Non-i18n surfaces
/// such as IM channels use `fallback_text()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AskUserText {
    Plain(String),
    I18n(AskUserI18nText),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserI18nText {
    pub key: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

impl AskUserText {
    pub fn plain(value: impl Into<String>) -> Self {
        Self::Plain(value.into())
    }

    pub fn fallback_text(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::I18n(value) => value.fallback.as_deref().unwrap_or(&value.key),
        }
    }
}

impl From<String> for AskUserText {
    fn from(value: String) -> Self {
        Self::Plain(value)
    }
}

impl From<&str> for AskUserText {
    fn from(value: &str) -> Self {
        Self::Plain(value.to_string())
    }
}

// ── Ask User Question (Interactive Q&A) ──

/// A single question option for the user to choose from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionOption {
    pub value: String,
    pub label: AskUserText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<AskUserText>,
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
    pub text: AskUserText,
    pub options: Vec<AskUserQuestionOption>,
    /// Whether to render a free-form custom input alongside the options.
    ///
    /// 保留该字段以维持原有的开关能力。当前在工具入口强制覆盖为 `true`，
    /// 因为模型往往给不出完整覆盖用户意图的选项，强制留一个自由文本
    /// 通道可以避免用户被迫二选一。等未来模型提问质量更稳定后可以把
    /// 覆盖逻辑摘掉，恢复模型自主控制。
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
    pub header: Option<AskUserText>,
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
    pub context: Option<AskUserText>,
    /// Where this question originated from: "plan" | "normal" | skill id.
    /// Used by the UI and listeners for routing / styling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// UNIX timestamp (seconds) after which pending answers auto-fall back to defaults.
    /// `None` means no overall timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_at: Option<u64>,
    /// Effective wall-clock timeout used to derive `timeout_at`. Persisting it
    /// lets owner-plane questions emit an accurate timeout event after restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Optional owner-plane response handler. Tool-created ask_user requests
    /// wait on an in-memory oneshot; owner-created requests instead complete
    /// by recording durable evidence when the user answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_response: Option<AskUserOwnerResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserOwnerResponse {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_evidence: Option<crate::domain_workflow::RecordDomainEvidenceInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOwnerAskUserQuestionInput {
    pub session_id: String,
    pub questions: Vec<AskUserQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<AskUserText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    pub owner_response: AskUserOwnerResponse,
}

/// Event payload emitted when a live ask_user_question request expires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserTimedOutPayload {
    pub request_id: String,
    pub session_id: String,
    pub timeout_secs: u64,
    pub used_default_values: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_preview: Option<String>,
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

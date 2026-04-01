use serde::{Deserialize, Serialize};

/// Category of a slash command, used for grouping in UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommandCategory {
    Session,
    Model,
    Memory,
    Agent,
    Utility,
    Skill,
}

/// A slash command definition (sent to frontend for menu rendering).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandDef {
    /// Command name without the "/" prefix, e.g. "new"
    pub name: String,
    /// Category for grouping
    pub category: CommandCategory,
    /// i18n key for the description, e.g. "slashCommands.new.description"
    pub description_key: String,
    /// Whether this command accepts arguments
    pub has_args: bool,
    /// Whether arguments are optional (command works with or without args)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub args_optional: bool,
    /// Placeholder text for args, e.g. "<title>"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg_placeholder: Option<String>,
    /// Fixed argument choices for hints (e.g. ["off","low","medium","high"])
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg_options: Option<Vec<String>>,
    /// Raw description string for skill commands (no i18n key).
    /// When set, frontend should display this directly instead of looking up description_key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_raw: Option<String>,
}

/// Channel-agnostic result of executing a slash command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    /// Text to display to the user (Markdown format).
    pub content: String,
    /// Side-effect action that the channel/frontend should perform.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<CommandAction>,
}

/// Side-effect actions returned by command execution.
/// Each channel (desktop UI, Telegram, Discord, etc.) handles these
/// appropriately for its context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CommandAction {
    /// A new session was created.
    NewSession { session_id: String },
    /// Model was switched.
    SwitchModel {
        provider_id: String,
        model_id: String,
    },
    /// Reasoning effort was changed.
    SetEffort { effort: String },
    /// Agent was switched (new session created).
    SwitchAgent {
        agent_id: String,
        session_id: String,
    },
    /// Stop the current streaming response.
    StopStream,
    /// Trigger context compaction (frontend should call compact_context_now).
    Compact,
    /// Session messages were cleared.
    SessionCleared,
    /// Do not intercept — pass message through to LLM as a normal user message.
    PassThrough { message: String },
    /// Export: content is the file data, filename is the suggested name.
    ExportFile { content: String, filename: String },
    /// Set tool permission mode for current session.
    SetToolPermission { mode: String },
    /// No side-effect, just display the `content` field.
    DisplayOnly,
    /// Show an interactive model picker card.
    /// Desktop: renders a clickable card; Telegram: sends inline buttons.
    ShowModelPicker {
        models: Vec<ModelPickerItem>,
        active_provider_id: Option<String>,
        active_model_id: Option<String>,
    },
    /// Enter plan mode for the current session.
    EnterPlanMode,
    /// Exit plan mode (optionally with plan content).
    ExitPlanMode { plan_content: Option<String> },
    /// Approve plan and start execution.
    ApprovePlan { plan_content: Option<String> },
    /// Show plan content in the plan panel.
    ShowPlan { plan_content: String },
    /// Pause plan execution.
    PausePlan,
    /// Resume plan execution.
    ResumePlan,
    /// Open system prompt viewer.
    ViewSystemPrompt,
}

/// A single model entry for the model picker card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPickerItem {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    pub model_name: String,
}

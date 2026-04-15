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
    /// Skill fork: the skill was dispatched to a sub-agent.
    /// The frontend should show a "skill running in background" indicator.
    SkillFork { run_id: String, skill_name: String },
    /// Recap: a recap report is being generated in the background.
    /// Frontend renders a streaming card subscribed to WS `recap_progress`
    /// events filtered by this `report_id`.
    RecapCard { report_id: String },
    /// Navigate to a specific Dashboard tab (e.g. `"recap"`, `"insights"`).
    OpenDashboardTab { tab: String },
    /// Show a structured context-window breakdown card.
    /// Desktop: renders a segmented bar + per-category detail card.
    /// IM channels: falls back to `CommandResult.content` markdown.
    ShowContextBreakdown { breakdown: ContextBreakdown },
}

/// Structured per-category context window usage snapshot.
/// All token counts use the char/4 heuristic consistent with
/// `context_compact::estimate_tokens`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdown {
    /// Maximum context window for the active model (tokens).
    pub context_window: u32,
    /// Reserved output budget (tokens).
    pub max_output_tokens: u32,
    /// Base system prompt excluding memory/skills/tool-descriptions sections.
    pub system_prompt_tokens: u32,
    /// JSON tool schemas sent to the API (`tools:` array).
    pub tool_schemas_tokens: u32,
    /// Tool descriptions embedded inside the system prompt.
    pub tool_descriptions_tokens: u32,
    /// Memory section injected into the system prompt (core + SQLite + guidelines).
    pub memory_tokens: u32,
    /// Skill descriptions injected into the system prompt.
    pub skill_tokens: u32,
    /// Conversation history (user/assistant messages + tool results).
    pub messages_tokens: u32,
    /// Total used (sum of the categories above + reserved output).
    pub used_total: u32,
    /// Free space (context_window - used_total). Saturates at 0.
    pub free_space: u32,
    /// Usage percentage (0.0–100.0).
    pub usage_pct: f32,
    /// Tier of the most recent compaction (None = never compacted this session).
    pub last_compact_tier: Option<u8>,
    /// Seconds since the most recent Tier 2+ compaction.
    pub last_compact_secs_ago: Option<u64>,
    /// Seconds until the next Tier 2+ compaction is allowed (cache TTL throttle).
    /// `Some(0)` or `None` means no cooldown.
    pub next_compact_allowed_in_secs: Option<u64>,
    /// Active model display label (e.g. "claude-sonnet-4-6").
    pub active_model: String,
    /// Active model provider display name.
    pub active_provider: String,
    /// Active agent ID.
    pub active_agent: String,
    /// Number of messages in the active conversation history.
    pub message_count: u32,
}

impl SlashCommandDef {
    /// Return an English description for use in channel APIs (e.g. Telegram Bot Menu).
    ///
    /// For skill commands, uses `description_raw`. For built-in commands, maps
    /// the command name to a hardcoded English string (matching en.json values).
    pub fn description_en(&self) -> String {
        if let Some(ref raw) = self.description_raw {
            return raw.clone();
        }
        match self.name.as_str() {
            "new" => "Start a new chat",
            "clear" => "Clear conversation",
            "compact" => "Compress context",
            "stop" => "Stop current reply",
            "rename" => "Rename session",
            "model" => "Switch model",
            "models" => "List all available models",
            "think" => "Set thinking effort",
            "remember" => "Save a memory",
            "forget" => "Delete a memory",
            "memories" => "List memories",
            "agent" => "Switch agent",
            "agents" => "List agents",
            "help" => "Show all commands",
            "status" => "Session status",
            "export" => "Export as Markdown",
            "usage" => "Token usage",
            "search" => "Search the web",
            "permission" => "Set tool permission mode",
            "plan" => "Enter/exit plan mode",
            "prompts" => "View system prompt",
            "recap" => "Generate a deep analysis recap report",
            "context" => "Show context window breakdown",
            "cross-session" => "Toggle cross-session behavior awareness",
            _ => "Command",
        }
        .to_string()
    }
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

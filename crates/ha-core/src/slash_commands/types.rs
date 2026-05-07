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
    /// Show an interactive project picker card. Desktop renders a clickable
    /// list; IM channels render inline buttons.
    ShowProjectPicker { projects: Vec<ProjectPickerItem> },
    /// Enter a project — frontend should create a new session inside the
    /// project (using its `default_agent_id` if set) and switch to it.
    EnterProject { project_id: String },
    /// Bind the current session to a project (used by `/project <id>` from
    /// inside an IM chat). Desktop falls back to `EnterProject`; IM updates
    /// `sessions.project_id` of the chat's current session in place.
    AssignProject { project_id: String },
    /// Show an interactive session picker card. Desktop renders a list;
    /// IM channels render inline buttons keyed by session id (one row each).
    ShowSessionPicker { sessions: Vec<SessionPickerItem> },
    /// Open the given session — desktop switches the active session, IM
    /// channels treat it as `AttachToSession`.
    EnterSession { session_id: String },
    /// Attach the current IM chat to `session_id` (used by `/session <id>`).
    /// Desktop has no analog; if reached on desktop, treat as `EnterSession`.
    AttachToSession { session_id: String },
    /// Detach the current IM chat from its session (used by `/session exit`).
    /// On desktop the action is a no-op (no chat-to-session binding to
    /// release).
    DetachFromSession,
    /// Hand the current session over to an IM chat — pushes a new attach row
    /// for (channel, account, chat, thread) and promotes it to primary.
    /// Used by GUI `/handover` and the GUI Handover dialog.
    HandoverToChannel {
        session_id: String,
        channel_id: String,
        account_id: String,
        chat_id: String,
        thread_id: Option<String>,
    },
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
            "thinking" | "think" => "Set thinking effort",
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
            "awareness" => "Toggle behavior awareness",
            "imreply" => "Set IM reply mode (split|final|preview)",
            "reason" => "Toggle whether the model's thinking is shown in IM messages",
            "project" => "Switch to or pick a project",
            "projects" => "List all projects",
            "sessions" => "Pick a session (optional search query)",
            "session" => "Show / attach / exit current chat's session",
            "handover" => "Hand the current session over to an IM chat",
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

/// A single project entry for the project picker card surfaced by `/project`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPickerItem {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub session_count: u32,
}

/// A single session entry for the session picker card surfaced by `/sessions`.
/// Filtered set: user-conversation sessions only (excluding incognito,
/// cron-driven, and subagent child sessions).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPickerItem {
    pub id: String,
    /// Display title (auto-generated from first message when not user-set).
    pub title: String,
    /// Agent that owns the session — surfaced so users can pick a session
    /// running with the agent they want.
    pub agent_id: String,
    /// Friendly agent display label (`AgentConfig.name` if set, else
    /// `agent_id`). Resolved by the handler so picker renderers don't have
    /// to load agent definitions themselves.
    pub agent_label: String,
    /// Project id when the session is assigned to one, else `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Project display label (emoji + name) when assigned. Resolved by the
    /// handler so picker renderers don't have to hit the project DB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_label: Option<String>,
    /// `Some(label)` when the session is currently surfaced from an IM chat
    /// (rendered as a small chip in the picker so the user knows it's
    /// shared with IM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_label: Option<String>,
    /// RFC3339 timestamp — most-recent activity, used by the picker for
    /// ordering / display. Matches `SessionMeta.updated_at` shape so the
    /// picker can be built without re-parsing.
    pub updated_at: String,
    /// `Some(text)` when the session was surfaced via message-content FTS
    /// match (i.e. `/sessions <query>` matched inside a message rather than
    /// the session metadata). Caller-formatted snippet, already stripped of
    /// the FTS5 mark sentinels — pickers render it on a second line so the
    /// user can see the match context without opening the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

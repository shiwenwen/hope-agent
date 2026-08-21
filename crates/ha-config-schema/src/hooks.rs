//! Hook configuration schema (`AppConfig.hooks`).
//!
//! Field-level aligned with the Claude Code `settings.json` `hooks` block
//! (design doc §4.2): event keys are PascalCase, handler `type` values are
//! snake_case, handler inner fields are camelCase. All five handler types
//! deserialize even though this phase only *executes* `command` — so a config
//! carrying `http`/`mcp_tool`/`prompt`/`agent` still round-trips cleanly.
//!
//! 仅数据定义；事件枚举（`HookEvent`）与 matcher / 分发 / scope 合并等行为
//! 逻辑留在 ha-core `hooks` 模块（`groups_for` / `is_empty` / `merge_from`
//! 以扩展 trait 形式承载于 ha-core）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Content-bound trust for one canonical workspace's project/local Hook files.
/// A missing file is represented by the stable `"missing"` sentinel; adding,
/// removing, moving, or changing either file invalidates the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookWorkspaceTrust {
    pub canonical_path: String,
    pub project_hash: String,
    pub local_hash: String,
}

/// Top-level `hooks` config. One ordered list of matcher groups per event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HooksConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_start: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_end: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_prompt_submit: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_prompt_expansion: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_tool_use: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_use: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_use_failure: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_batch: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_request: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_denied: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_failure: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_compact: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_compact: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notification: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_display: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_start: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagent_stop: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_created: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_completed: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teammate_idle: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_change: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cwd_changed: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_changed: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions_loaded: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elicitation: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elicitation_result: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worktree_create: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worktree_remove: Vec<HookMatcherGroup>,
}

/// One `{ matcher, hooks: [...] }` block. `matcher: None` means `"*"` (always
/// matches).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcherGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookHandlerConfig>,
}

/// A single hook handler. The `type` tag selects the variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookHandlerConfig {
    Command(CommandHookConfig),
    Http(HttpHookConfig),
    McpTool(McpToolHookConfig),
    Prompt(PromptHookConfig),
    Agent(AgentHookConfig),
}

impl HookHandlerConfig {
    /// The `if` rule for conditional execution (run only when the tool call
    /// matches), if set. Shared accessor so dispatch needn't match the variant.
    pub fn if_rule(&self) -> Option<&str> {
        match self {
            Self::Command(c) => c.if_rule.as_deref(),
            Self::Http(c) => c.if_rule.as_deref(),
            Self::McpTool(c) => c.if_rule.as_deref(),
            Self::Prompt(c) => c.if_rule.as_deref(),
            Self::Agent(c) => c.if_rule.as_deref(),
        }
    }

    /// Whether this handler runs at most once per session.
    pub fn once(&self) -> bool {
        match self {
            Self::Command(c) => c.once,
            Self::Http(c) => c.once,
            Self::McpTool(c) => c.once,
            Self::Prompt(c) => c.once,
            Self::Agent(c) => c.once,
        }
        .unwrap_or(false)
    }

    /// Static status message shown to the user while the handler runs.
    pub fn status_message(&self) -> Option<&str> {
        match self {
            Self::Command(c) => c.status_message.as_deref(),
            Self::Http(c) => c.status_message.as_deref(),
            Self::McpTool(c) => c.status_message.as_deref(),
            Self::Prompt(c) => c.status_message.as_deref(),
            Self::Agent(c) => c.status_message.as_deref(),
        }
    }

    /// (command + async only) Inject stderr into the next turn on exit 2.
    pub fn async_rewake(&self) -> bool {
        // Exhaustive match — same contract as the sibling accessors: adding a
        // new variant must fail to compile until each accessor is updated.
        match self {
            Self::Command(c) => c.async_rewake == Some(true),
            Self::Http(_) | Self::McpTool(_) | Self::Prompt(_) | Self::Agent(_) => false,
        }
    }
}

/// Which shell to run a `command` handler in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookShell {
    Bash,
    Powershell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHookConfig {
    pub command: String,
    /// Official exec-form argv. When present the command is spawned directly
    /// (`command` + these args, no shell) — `shell` is ignored. When absent the
    /// command runs through the shell (`<shell> -c command`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Process environment variables explicitly inherited from the Hope Agent
    /// process. Command Hooks otherwise run with a minimal, secret-free
    /// environment plus the synthetic HOPE_/CLAUDE_ variables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_env_vars: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<HookShell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "async")]
    pub async_run: Option<bool>,
    /// Static message shown to the user while this handler runs (`statusMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Run only when the tool call matches this rule, e.g. `exec(rm *)` (`if`).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    /// Run at most once per session for this handler identity (`once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// (async only) When the detached command exits 2, inject its escaped stderr
    /// as Hook data into the next turn (`asyncRewake`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_rewake: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpHookConfig {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_env_vars: Vec<String>,
    /// Static message shown to the user while this handler runs (`statusMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Run only when the tool call matches this rule, e.g. `exec(rm *)` (`if`).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    /// Run at most once per session for this handler identity (`once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolHookConfig {
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Static message shown to the user while this handler runs (`statusMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Run only when the tool call matches this rule, e.g. `exec(rm *)` (`if`).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    /// Run at most once per session for this handler identity (`once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptHookConfig {
    pub prompt: String,
    /// Deprecated — superseded by `modelOverride`. `provider:model` string.
    /// Kept for backward compatibility: still parsed when `modelOverride` is
    /// unset, but the GUI no longer writes this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Model chain override for this hook's side-query. `None` = fall through
    /// to the deprecated `model` (if still set) → `function_models.automation`
    /// → chat default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<crate::provider::ModelChain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Static message shown to the user while this handler runs (`statusMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Run only when the tool call matches this rule, e.g. `exec(rm *)` (`if`).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    /// Run at most once per session for this handler identity (`once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHookConfig {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "async")]
    pub async_run: Option<bool>,
    /// Static message shown to the user while this handler runs (`statusMessage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    /// Run only when the tool call matches this rule, e.g. `exec(rm *)` (`if`).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    /// Run at most once per session for this handler identity (`once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

//! Hook configuration schema (`AppConfig.hooks`).
//!
//! Field-level aligned with the Claude Code `settings.json` `hooks` block
//! (design doc §4.2): event keys are PascalCase, handler `type` values are
//! snake_case, handler inner fields are camelCase. All five handler types
//! deserialize even though this phase only *executes* `command` — so a config
//! carrying `http`/`mcp_tool`/`prompt`/`agent` still round-trips cleanly.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::types::HookEvent;

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

impl HooksConfig {
    /// Borrow the matcher groups configured for a given event.
    pub fn groups_for(&self, event: HookEvent) -> &[HookMatcherGroup] {
        match event {
            HookEvent::SessionStart => &self.session_start,
            HookEvent::SessionEnd => &self.session_end,
            HookEvent::Setup => &self.setup,
            HookEvent::UserPromptSubmit => &self.user_prompt_submit,
            HookEvent::UserPromptExpansion => &self.user_prompt_expansion,
            HookEvent::PreToolUse => &self.pre_tool_use,
            HookEvent::PostToolUse => &self.post_tool_use,
            HookEvent::PostToolUseFailure => &self.post_tool_use_failure,
            HookEvent::PostToolBatch => &self.post_tool_batch,
            HookEvent::PermissionRequest => &self.permission_request,
            HookEvent::PermissionDenied => &self.permission_denied,
            HookEvent::Stop => &self.stop,
            HookEvent::StopFailure => &self.stop_failure,
            HookEvent::PreCompact => &self.pre_compact,
            HookEvent::PostCompact => &self.post_compact,
            HookEvent::Notification => &self.notification,
            HookEvent::MessageDisplay => &self.message_display,
            HookEvent::SubagentStart => &self.subagent_start,
            HookEvent::SubagentStop => &self.subagent_stop,
            HookEvent::TaskCreated => &self.task_created,
            HookEvent::TaskCompleted => &self.task_completed,
            HookEvent::TeammateIdle => &self.teammate_idle,
            HookEvent::ConfigChange => &self.config_change,
            HookEvent::CwdChanged => &self.cwd_changed,
            HookEvent::FileChanged => &self.file_changed,
            HookEvent::InstructionsLoaded => &self.instructions_loaded,
            HookEvent::Elicitation => &self.elicitation,
            HookEvent::ElicitationResult => &self.elicitation_result,
            HookEvent::WorktreeCreate => &self.worktree_create,
            HookEvent::WorktreeRemove => &self.worktree_remove,
        }
    }

    /// True when no event has any configured matcher group. Lets the
    /// dispatcher short-circuit cheaply on the hot path.
    pub fn is_empty(&self) -> bool {
        HOOK_EVENTS_FOR_EMPTY_CHECK
            .iter()
            .all(|e| self.groups_for(*e).is_empty())
    }

    /// Merge another scope's matcher groups into `self` (union — every scope's
    /// hooks run; there is no override precedence, design §4). Used to layer
    /// managed / project / local hooks on top of the user scope. `other`'s
    /// groups are appended after `self`'s so config order is preserved.
    pub fn merge_from(&mut self, mut other: HooksConfig) {
        self.session_start.append(&mut other.session_start);
        self.session_end.append(&mut other.session_end);
        self.setup.append(&mut other.setup);
        self.user_prompt_submit
            .append(&mut other.user_prompt_submit);
        self.user_prompt_expansion
            .append(&mut other.user_prompt_expansion);
        self.pre_tool_use.append(&mut other.pre_tool_use);
        self.post_tool_use.append(&mut other.post_tool_use);
        self.post_tool_use_failure
            .append(&mut other.post_tool_use_failure);
        self.post_tool_batch.append(&mut other.post_tool_batch);
        self.permission_request
            .append(&mut other.permission_request);
        self.permission_denied.append(&mut other.permission_denied);
        self.stop.append(&mut other.stop);
        self.stop_failure.append(&mut other.stop_failure);
        self.pre_compact.append(&mut other.pre_compact);
        self.post_compact.append(&mut other.post_compact);
        self.notification.append(&mut other.notification);
        self.message_display.append(&mut other.message_display);
        self.subagent_start.append(&mut other.subagent_start);
        self.subagent_stop.append(&mut other.subagent_stop);
        self.task_created.append(&mut other.task_created);
        self.task_completed.append(&mut other.task_completed);
        self.teammate_idle.append(&mut other.teammate_idle);
        self.config_change.append(&mut other.config_change);
        self.cwd_changed.append(&mut other.cwd_changed);
        self.file_changed.append(&mut other.file_changed);
        self.instructions_loaded
            .append(&mut other.instructions_loaded);
        self.elicitation.append(&mut other.elicitation);
        self.elicitation_result
            .append(&mut other.elicitation_result);
        self.worktree_create.append(&mut other.worktree_create);
        self.worktree_remove.append(&mut other.worktree_remove);
    }
}

const HOOK_EVENTS_FOR_EMPTY_CHECK: [HookEvent; 30] = [
    HookEvent::SessionStart,
    HookEvent::SessionEnd,
    HookEvent::Setup,
    HookEvent::UserPromptSubmit,
    HookEvent::UserPromptExpansion,
    HookEvent::PreToolUse,
    HookEvent::PostToolUse,
    HookEvent::PostToolUseFailure,
    HookEvent::PostToolBatch,
    HookEvent::PermissionRequest,
    HookEvent::PermissionDenied,
    HookEvent::Stop,
    HookEvent::StopFailure,
    HookEvent::PreCompact,
    HookEvent::PostCompact,
    HookEvent::Notification,
    HookEvent::MessageDisplay,
    HookEvent::SubagentStart,
    HookEvent::SubagentStop,
    HookEvent::TaskCreated,
    HookEvent::TaskCompleted,
    HookEvent::TeammateIdle,
    HookEvent::ConfigChange,
    HookEvent::CwdChanged,
    HookEvent::FileChanged,
    HookEvent::InstructionsLoaded,
    HookEvent::Elicitation,
    HookEvent::ElicitationResult,
    HookEvent::WorktreeCreate,
    HookEvent::WorktreeRemove,
];

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
    /// (async only) When the detached command exits 2, inject its stderr as a
    /// system-reminder into the next turn (`asyncRewake`).
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

/// Combined hooks settings payload for the Settings → Hooks GUI: the
/// `disable_all_hooks` master switch (a top-level `AppConfig` field) plus the
/// user-scope `hooks` map. Read via `get_hooks_config`, written via
/// `save_hooks_config` (user scope only — project / local / managed are
/// file-based and never written from the GUI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksSettings {
    #[serde(rename = "disableAllHooks", default)]
    pub disable_all_hooks: bool,
    /// Mirrors `AppConfig::hooks_allow_project_scope`: opt-in for loading
    /// `<cwd>/.hope-agent/hooks.json` / `hooks.local.json` (off by default, a
    /// supply-chain guard so a repo's checked-in hooks don't auto-execute).
    #[serde(rename = "allowProjectScope", default)]
    pub allow_project_scope: bool,
    #[serde(default)]
    pub hooks: HooksConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_from_unions_groups_per_event() {
        let mut base: HooksConfig = serde_json::from_str(
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"a"}]}]}"#,
        )
        .unwrap();
        let overlay: HooksConfig = serde_json::from_str(
            r#"{"PreToolUse":[{"matcher":"Write","hooks":[{"type":"command","command":"b"}]}],
                "PostToolUse":[{"hooks":[{"type":"command","command":"c"}]}]}"#,
        )
        .unwrap();
        base.merge_from(overlay);
        // PreToolUse now has both groups (union, base first); PostToolUse gained one.
        assert_eq!(base.groups_for(HookEvent::PreToolUse).len(), 2);
        assert_eq!(base.groups_for(HookEvent::PostToolUse).len(), 1);
    }

    #[test]
    fn deserialize_settings_example() {
        // Trimmed-down version of design doc §4.3.
        let json = r#"{
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        { "type": "command", "command": "./block-rm.sh", "timeout": 10 }
                    ]
                },
                {
                    "matcher": "mcp__.*__write.*",
                    "hooks": [
                        { "type": "http", "url": "http://localhost:8080/h", "headers": {"Authorization": "Bearer $T"}, "allowedEnvVars": ["T"] }
                    ]
                }
            ],
            "PostToolUse": [
                {
                    "matcher": "Write|Edit",
                    "hooks": [
                        { "type": "command", "command": "./fmt.sh", "async": true }
                    ]
                }
            ],
            "SessionStart": [
                { "hooks": [ { "type": "command", "command": "~/setup.sh" } ] }
            ]
        }"#;
        let cfg: HooksConfig = serde_json::from_str(json).unwrap();

        assert_eq!(cfg.pre_tool_use.len(), 2);
        let g0 = &cfg.pre_tool_use[0];
        assert_eq!(g0.matcher.as_deref(), Some("Bash"));
        match &g0.hooks[0] {
            HookHandlerConfig::Command(c) => {
                assert_eq!(c.command, "./block-rm.sh");
                assert_eq!(c.timeout, Some(10));
            }
            _ => panic!("expected command"),
        }
        match &cfg.pre_tool_use[1].hooks[0] {
            HookHandlerConfig::Http(h) => {
                assert_eq!(h.url, "http://localhost:8080/h");
                assert_eq!(h.headers.get("Authorization").unwrap(), "Bearer $T");
                assert_eq!(h.allowed_env_vars, vec!["T".to_string()]);
            }
            _ => panic!("expected http"),
        }
        // async (Rust keyword) round-trips via rename.
        match &cfg.post_tool_use[0].hooks[0] {
            HookHandlerConfig::Command(c) => {
                assert_eq!(c.async_run, Some(true));
            }
            _ => panic!("expected command"),
        }
        // matcher: None means wildcard.
        assert!(cfg.session_start[0].matcher.is_none());
    }

    #[test]
    fn empty_object_is_default_and_serializes_clean() {
        let cfg: HooksConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.is_empty());
        // skip_serializing_if keeps empty events out → re-serializes to `{}`.
        assert_eq!(serde_json::to_string(&cfg).unwrap(), "{}");
    }

    #[test]
    fn unknown_event_keys_are_ignored() {
        // serde ignores unknown fields by default → no error.
        let cfg: HooksConfig = serde_json::from_str(r#"{"NotAnEvent": [{"hooks": []}]}"#).unwrap();
        assert!(cfg.is_empty());
    }

    #[test]
    fn mcp_tool_and_prompt_and_agent_deserialize() {
        let json = r#"{
            "PostToolUse": [
                { "hooks": [
                    { "type": "mcp_tool", "server": "sec", "tool": "scan", "input": {"f": "${tool_input.file_path}"} },
                    { "type": "prompt", "prompt": "judge this" },
                    { "type": "agent", "prompt": "investigate", "allowedTools": ["Read", "Grep"] }
                ] }
            ]
        }"#;
        let cfg: HooksConfig = serde_json::from_str(json).unwrap();
        let hooks = &cfg.post_tool_use[0].hooks;
        assert_eq!(hooks.len(), 3);
        assert!(matches!(hooks[0], HookHandlerConfig::McpTool(_)));
        assert!(matches!(hooks[1], HookHandlerConfig::Prompt(_)));
        match &hooks[2] {
            HookHandlerConfig::Agent(a) => {
                assert_eq!(
                    a.allowed_tools,
                    vec!["Read".to_string(), "Grep".to_string()]
                )
            }
            _ => panic!("expected agent"),
        }
    }

    #[test]
    fn groups_for_indexes_all_events() {
        let cfg = HooksConfig::default();
        // Sanity: every event maps to an (empty) slice without panicking.
        for e in HOOK_EVENTS_FOR_EMPTY_CHECK {
            assert!(cfg.groups_for(e).is_empty());
        }
    }

    #[test]
    fn new_event_keys_and_args_exec_form_deserialize() {
        let json = r#"{
            "Setup": [ { "hooks": [ { "type": "command", "command": "echo setup" } ] } ],
            "MessageDisplay": [ { "hooks": [ { "type": "command", "command": "echo msg" } ] } ],
            "PreToolUse": [ { "hooks": [
                { "type": "command", "command": "/usr/bin/lint", "args": ["--fix", "src"] }
            ] } ]
        }"#;
        let cfg: HooksConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.groups_for(HookEvent::Setup).len(), 1);
        assert_eq!(cfg.groups_for(HookEvent::MessageDisplay).len(), 1);
        match &cfg.pre_tool_use[0].hooks[0] {
            HookHandlerConfig::Command(c) => {
                assert_eq!(c.command, "/usr/bin/lint");
                assert_eq!(c.args, vec!["--fix".to_string(), "src".to_string()]);
            }
            _ => panic!("expected command"),
        }
    }
}

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
}

const HOOK_EVENTS_FOR_EMPTY_CHECK: [HookEvent; 28] = [
    HookEvent::SessionStart,
    HookEvent::SessionEnd,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<HookShell>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "async")]
    pub async_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_rewake: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptHookConfig {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_settings_example() {
        // Trimmed-down version of design doc §4.3.
        let json = r#"{
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        { "type": "command", "command": "./block-rm.sh", "if": "Bash(rm *)", "timeout": 10 }
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
                        { "type": "command", "command": "./fmt.sh", "async": true, "statusMessage": "Formatting..." }
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
                assert_eq!(c.if_rule.as_deref(), Some("Bash(rm *)"));
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
                assert_eq!(c.status_message.as_deref(), Some("Formatting..."));
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
}

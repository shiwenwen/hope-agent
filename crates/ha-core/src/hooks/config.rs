//! Hook configuration schema (`AppConfig.hooks`).
//!
//! 类型定义已下沉 [`ha_config_schema::hooks`]，此处原地再导出保持
//! `crate::hooks::config::*` 路径不变；依赖 [`HookEvent`]（未下沉的事件枚举）
//! 的行为方法以 [`HooksConfigExt`] 扩展 trait 留在本 crate，测试留在本 crate。

use super::types::HookEvent;

// 类型已下沉 ha-config-schema
pub use ha_config_schema::hooks::{
    AgentHookConfig, CommandHookConfig, HookHandlerConfig, HookMatcherGroup, HookShell,
    HookWorkspaceTrust, HooksConfig, HttpHookConfig, McpToolHookConfig, PromptHookConfig,
};

/// [`HooksConfig`] 的行为面：这些方法依赖 `hooks::types::HookEvent`（事件
/// 枚举属分发行为层，未下沉 schema），而类型下沉后无法在本 crate 保留
/// inherent impl，故以扩展 trait 承载——调用方 `use` 本 trait 后方法语法不变。
pub trait HooksConfigExt {
    /// Borrow the matcher groups configured for a given event.
    fn groups_for(&self, event: HookEvent) -> &[HookMatcherGroup];

    /// True when no event has any configured matcher group. Lets the
    /// dispatcher short-circuit cheaply on the hot path.
    fn is_empty(&self) -> bool;

    /// Merge another scope's matcher groups into `self` (union — every scope's
    /// hooks run; there is no override precedence, design §4). Used to layer
    /// managed / project / local hooks on top of the user scope. `other`'s
    /// groups are appended after `self`'s so config order is preserved.
    fn merge_from(&mut self, other: HooksConfig);
}

impl HooksConfigExt for HooksConfig {
    fn groups_for(&self, event: HookEvent) -> &[HookMatcherGroup] {
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

    fn is_empty(&self) -> bool {
        HOOK_EVENTS_FOR_EMPTY_CHECK
            .iter()
            .all(|e| self.groups_for(*e).is_empty())
    }

    fn merge_from(&mut self, other: HooksConfig) {
        // 穷举解构（不写 `..`）：HooksConfig 定义在 ha-config-schema，新增
        // 事件字段时这里立刻编译失败——跨 crate 后 merge_from 唯一的
        // 完整性保障就是这个编译期强制，勿改回逐字段 append 的开放写法。
        let HooksConfig {
            mut session_start,
            mut session_end,
            mut setup,
            mut user_prompt_submit,
            mut user_prompt_expansion,
            mut pre_tool_use,
            mut post_tool_use,
            mut post_tool_use_failure,
            mut post_tool_batch,
            mut permission_request,
            mut permission_denied,
            mut stop,
            mut stop_failure,
            mut pre_compact,
            mut post_compact,
            mut notification,
            mut message_display,
            mut subagent_start,
            mut subagent_stop,
            mut task_created,
            mut task_completed,
            mut teammate_idle,
            mut config_change,
            mut cwd_changed,
            mut file_changed,
            mut instructions_loaded,
            mut elicitation,
            mut elicitation_result,
            mut worktree_create,
            mut worktree_remove,
        } = other;
        self.session_start.append(&mut session_start);
        self.session_end.append(&mut session_end);
        self.setup.append(&mut setup);
        self.user_prompt_submit.append(&mut user_prompt_submit);
        self.user_prompt_expansion
            .append(&mut user_prompt_expansion);
        self.pre_tool_use.append(&mut pre_tool_use);
        self.post_tool_use.append(&mut post_tool_use);
        self.post_tool_use_failure
            .append(&mut post_tool_use_failure);
        self.post_tool_batch.append(&mut post_tool_batch);
        self.permission_request.append(&mut permission_request);
        self.permission_denied.append(&mut permission_denied);
        self.stop.append(&mut stop);
        self.stop_failure.append(&mut stop_failure);
        self.pre_compact.append(&mut pre_compact);
        self.post_compact.append(&mut post_compact);
        self.notification.append(&mut notification);
        self.message_display.append(&mut message_display);
        self.subagent_start.append(&mut subagent_start);
        self.subagent_stop.append(&mut subagent_stop);
        self.task_created.append(&mut task_created);
        self.task_completed.append(&mut task_completed);
        self.teammate_idle.append(&mut teammate_idle);
        self.config_change.append(&mut config_change);
        self.cwd_changed.append(&mut cwd_changed);
        self.file_changed.append(&mut file_changed);
        self.instructions_loaded.append(&mut instructions_loaded);
        self.elicitation.append(&mut elicitation);
        self.elicitation_result.append(&mut elicitation_result);
        self.worktree_create.append(&mut worktree_create);
        self.worktree_remove.append(&mut worktree_remove);
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

/// Combined hooks settings payload for the Settings → Hooks GUI: the
/// `disable_all_hooks` master switch (a top-level `AppConfig` field) plus the
/// user-scope `hooks` map. Read via `get_hooks_config`, written via
/// `save_hooks_config` (user scope only — project / local / managed are
/// file-based and never written from the GUI).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HooksSettings {
    #[serde(rename = "disableAllHooks", default)]
    pub disable_all_hooks: bool,
    /// Canonical workspace paths currently trusted for the exact Hook file
    /// contents recorded in AppConfig. Existing paths retain their server-side
    /// hashes; only a newly added or explicitly re-added path is hashed.
    #[serde(rename = "trustedProjectScopes", default)]
    pub trusted_project_scopes: Vec<String>,
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

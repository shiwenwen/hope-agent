//! `PreToolUse` continue:false aggregation:
//!
//! A safety hook can short-circuit a tool call without an explicit
//! `permissionDecision:"deny"` by returning `{"continue":false,"stopReason":…}`
//! and `exit 0`. The hook dispatcher must fold this into the aggregate outcome
//! and the `PreToolUse` callsite (`tools::execution::fire_pre_tool_use_hook`)
//! must map it to a hard block — covered by `pre_tool_gate_from_outcome` unit
//! tests in the lib. This integration test guards the dispatcher half: a real
//! command hook emitting that JSON must show up as `continue_execution:false`
//! on the dispatched outcome.
//!
//! Single `#[test]` per binary so the process-global OnceLocks don't race with
//! the other hooks integration tests.
//!
//! Unix-only: the hook shells out to `bash`.
#![cfg(unix)]

use std::path::PathBuf;

use ha_core::hooks::{
    self, CommonHookInput, HookDispatcher, HookEvent, HookInput, HooksConfig, PermissionMode,
};

fn pre_tool_use() -> HookInput {
    HookInput::PreToolUse {
        common: CommonHookInput {
            session_id: "cf-sess".into(),
            transcript_path: PathBuf::from("/tmp/cf.jsonl"),
            cwd: std::env::temp_dir(),
            permission_mode: PermissionMode::Default,
            hook_event_name: "PreToolUse".into(),
            agent_id: None,
            agent_type: None,
        },
        tool_name: "Bash".into(),
        tool_input: serde_json::json!({ "command": "ls" }),
        tool_use_id: "c1".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pretooluse_continue_false_aggregates_to_block() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // A safety hook that prints `{"continue":false,"stopReason":"halt by hook"}`
    // and exits 0 — i.e. opts out of the exit-code-2 shorthand but still wants
    // the loop terminated. This is the Claude Code script shape that silently
    // passed through the old `match outcome.decision` branch.
    let cfg: HooksConfig = serde_json::from_str(
        r#"{
            "PreToolUse": [
                { "hooks": [
                    { "type": "command", "shell": "bash",
                      "command": "printf '%s' '{\"continue\":false,\"stopReason\":\"halt by hook\"}'" }
                ]}
            ]
        }"#,
    )
    .expect("parse hooks config");

    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = cfg.clone();
        Ok(())
    })
    .expect("write hooks config");
    hooks::registry::reload_from_config();

    let out = HookDispatcher::dispatch(HookEvent::PreToolUse, pre_tool_use()).await;
    assert!(
        !out.continue_execution,
        "continue:false from any PreToolUse hook must terminate the loop, got outcome {:?}",
        out
    );
    assert_eq!(
        out.stop_reason.as_deref(),
        Some("halt by hook"),
        "stopReason must propagate so the user sees why the tool was blocked"
    );
}

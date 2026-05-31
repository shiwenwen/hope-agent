//! End-to-end hooks wiring: command hooks written through the live config path
//! are picked up by the global registry and injected through the public
//! `HookDispatcher::dispatch` / `fire_session_start_observation` paths.
//!
//! The in-module unit tests (`hooks::…`) only exercise `dispatch_with` against
//! an explicit registry; this file covers the real chain config → reload →
//! global dispatch, plus the once-per-session SessionStart helper and the
//! config-cleared no-op. Single `#[test]` per binary (per the `init_runtime.rs`
//! convention) so the process-global OnceLocks don't race.
//!
//! Unix-only: the hooks shell out to `bash`.
#![cfg(unix)]

use std::path::PathBuf;

use ha_core::hooks::{
    self, CommonHookInput, HookDispatcher, HookEvent, HookInput, HooksConfig, PermissionMode,
};

fn common(event: &str) -> CommonHookInput {
    CommonHookInput {
        session_id: "e2e-sess".into(),
        transcript_path: PathBuf::from("/tmp/e2e.jsonl"),
        cwd: std::env::temp_dir(),
        permission_mode: PermissionMode::Default,
        hook_event_name: event.into(),
        agent_id: None,
        agent_type: None,
    }
}

fn post_tool_use(tool: &str, call_id: &str) -> HookInput {
    HookInput::PostToolUse {
        common: common("PostToolUse"),
        tool_name: tool.into(),
        tool_input: serde_json::json!({ "path": "x" }),
        tool_response: serde_json::json!("ok"),
        tool_use_id: call_id.into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_driven_hooks_dispatch_end_to_end() {
    // Sandbox the data dir (HA_DATA_DIR is honored by paths::root_dir directly).
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // Configure three hooks at once: a Write-scoped PostToolUse injector, an
    // unscoped SessionStart injector, and a Notification hook that exits
    // non-zero (must be non-blocking for an observation event).
    let cfg: HooksConfig = serde_json::from_str(
        r#"{
            "PostToolUse": [
                { "matcher": "Write", "hooks": [
                    { "type": "command", "shell": "bash",
                      "command": "printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"WROTE_VIA_HOOK\"}}'" }
                ]}
            ],
            "SessionStart": [
                { "hooks": [
                    { "type": "command", "shell": "bash",
                      "command": "printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"SESSION_BOOT\"}}'" }
                ]}
            ],
            "Notification": [
                { "hooks": [
                    { "type": "command", "shell": "bash", "command": "echo noisy 1>&2; exit 1" }
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

    // PostToolUse(Write) → additionalContext injected via the GLOBAL dispatch.
    let out = HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("write", "c1")).await;
    assert_eq!(
        out.merged_additional_context().as_deref(),
        Some("WROTE_VIA_HOOK")
    );

    // A non-matching tool falls through to a no-op.
    let noop = HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("read", "c2")).await;
    assert!(noop.merged_additional_context().is_none());

    // SessionStart via the shared helper (engine + ACP both call this).
    let ctx = hooks::fire_session_start_observation("e2e-sess", "ha-main", "test-model").await;
    assert_eq!(ctx.as_deref(), Some("SESSION_BOOT"));
    // Once-per-session: a second fire for the same session returns None.
    let ctx2 = hooks::fire_session_start_observation("e2e-sess", "ha-main", "test-model").await;
    assert!(ctx2.is_none());

    // An observation Notification hook exiting non-zero is non-blocking: it
    // contributes no context and doesn't panic / change the decision.
    let nz = HookDispatcher::dispatch(
        HookEvent::Notification,
        HookInput::Notification {
            common: common("Notification"),
            notification_type: "idle_prompt".into(),
            message: "hi".into(),
            title: None,
        },
    )
    .await;
    assert!(nz.merged_additional_context().is_none());

    // Overflow: a hook emitting far more than the 10 000-char inject cap keeps a
    // head slice + a truncation pointer (not just a bare pointer), and the
    // injected text stays within the cap. Emits JSON additionalContext of
    // 15 000 'X's via PostToolUse (whose additionalContext is honored).
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = serde_json::from_str(
            r#"{"PostToolUse":[{"matcher":"Write","hooks":[
                {"type":"command","shell":"bash","command":"printf '{\"hookSpecificOutput\":{\"additionalContext\":\"%s\"}}' \"$(printf 'X%.0s' $(seq 1 15000))\""}
            ]}]}"#,
        )
        .unwrap();
        Ok(())
    })
    .expect("write overflow hook config");
    hooks::registry::reload_from_config();
    let big =
        HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("write", "c-big")).await;
    let injected = big
        .merged_additional_context()
        .expect("overflow hook injects context");
    assert!(injected.starts_with('X'), "head slice preserved");
    assert!(injected.contains("truncated"), "truncation pointer present");
    assert!(
        injected.chars().count() <= 10_000,
        "injected stays within the cap, got {}",
        injected.chars().count()
    );

    // Clearing hooks in config + reloading the registry → dispatch is a no-op
    // (hot-reload removes handlers, not just adds them).
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = HooksConfig::default();
        Ok(())
    })
    .expect("clear hooks config");
    hooks::registry::reload_from_config();
    let after_clear =
        HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("write", "c3")).await;
    assert!(after_clear.merged_additional_context().is_none());
}

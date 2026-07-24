//! End-to-end hooks wiring: command hooks written through the live config path
//! are picked up by the global registry and injected through the public
//! `HookDispatcher::dispatch` / `fire_session_start_observation` paths.
//!
//! The in-module unit tests (`hooks::…`) only exercise `dispatch_with` against
//! an explicit registry; this file covers the real chain config → reload →
//! global dispatch, plus the once-per-session SessionStart helper, the
//! fire-and-forget `fire_*` path, and the config-cleared no-op. Single `#[test]`
//! per binary (per the `init_runtime.rs` convention) so the process-global
//! OnceLocks don't race.
//!
//! Unix-only: the hooks shell out to `bash`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ha_core::hooks::{
    self, CommonHookInput, HookDispatcher, HookEvent, HookInput, HooksConfig, PermissionMode,
};

fn common(event: &str) -> CommonHookInput {
    CommonHookInput {
        session_id: "e2e-sess".into(),
        prompt_id: None,
        transcript_path: PathBuf::from("/tmp/e2e.jsonl"),
        cwd: std::env::temp_dir(),
        permission_mode: PermissionMode::Default,
        effort: None,
        hook_event_name: event.into(),
        agent_id: None,
        agent_type: None,
    }
}

/// Poll until `log` exists and is non-empty, or panic. The `fire_*` helpers are
/// fire-and-forget (they spawn), so their side effects can only be observed with
/// a deadline.
async fn wait_for_file(log: &Path, what: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(s) = std::fs::read_to_string(log) {
            if !s.trim().is_empty() {
                return s;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waited 30s for {what} to write {}, nothing appeared",
            log.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn post_tool_use(tool: &str, call_id: &str) -> HookInput {
    HookInput::PostToolUse {
        common: common("PostToolUse"),
        tool_name: tool.into(),
        tool_input: serde_json::json!({ "path": "x" }),
        tool_response: serde_json::json!("ok"),
        tool_use_id: call_id.into(),
        job_id: None,
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

    // PermissionRequest tool-name matcher over the REAL chain (config → global
    // registry → `hooks::dispatch_permission_request`). The in-module unit test
    // `permission_request_matches_on_tool_name_alias` covers `dispatch_with`
    // against a hand-built registry, but the regression it guards — the tool
    // name never reaching the hook input, so `matcher_target()` silently fell
    // back to the `command` string and a tool-name matcher never fired — lived
    // in this chain, not in the matcher.
    //
    // The PostToolUse(Write) injector rides along so the config-clear section
    // below still proves hot-reload REMOVES handlers rather than trivially
    // passing on a config that never had a PostToolUse hook.
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = serde_json::from_str(
            r#"{
                "PostToolUse": [
                    { "matcher": "Write", "hooks": [
                        { "type": "command", "shell": "bash",
                          "command": "printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"WROTE_VIA_HOOK\"}}'" }
                    ]}
                ],
                "PermissionRequest": [
                    { "matcher": "Bash", "hooks": [
                        { "type": "command", "shell": "bash",
                          "command": "echo denied-by-policy 1>&2; exit 2" }
                    ]}
                ]
            }"#,
        )
        .unwrap();
        Ok(())
    })
    .expect("write permission-request hook config");
    hooks::registry::reload_from_config();
    // Sanity: the PostToolUse handler the teardown section relies on is live.
    let wrote =
        HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("write", "c-perm")).await;
    assert_eq!(
        wrote.merged_additional_context().as_deref(),
        Some("WROTE_VIA_HOOK"),
        "PostToolUse(Write) injector must be live before the config-clear section"
    );

    // `matcher: "Bash"` is the Claude Code alias, normalized onto the internal
    // tool name `exec`, so a PermissionRequest carrying tool_name=Some("exec")
    // must match and its `exit 2` must surface as a veto. `tool_input` is
    // deliberately non-None: the payload now carries the tool args (the field
    // was hard-coded `None` until the same fix threaded them through the
    // approval path), so a hook can gate on `.tool_input.command`.
    let denied = hooks::dispatch_permission_request(
        Some("e2e-sess"),
        Some("exec"),
        Some(&serde_json::json!({ "command": "npm test" })),
        "npm test",
        Some("call-perm-1"),
    )
    .await;
    assert!(
        denied.block_reason().is_some(),
        "PermissionRequest matcher \"Bash\" must normalize onto the internal tool name `exec` \
         and veto via exit 2 through the config→registry→dispatch chain, got {:?}",
        denied.decision
    );

    // Without a tool name the matcher target falls back to the `command` string
    // ("npm test"), which "Bash"→`exec` must not match → no veto. This is the
    // half that passed even while the bug was live, so it pins the fallback.
    let allowed = hooks::dispatch_permission_request(
        Some("e2e-sess"),
        None,
        None,
        "npm test",
        Some("call-perm-2"),
    )
    .await;
    assert!(
        allowed.block_reason().is_none(),
        "PermissionRequest with tool_name=None must fall back to the command string \
         (\"npm test\"), which the \"Bash\" matcher must not match, got {:?}",
        allowed.decision
    );

    // The fire-and-forget `fire_*` path, which NO other test exercises.
    //
    // These 15 helpers build their hook input — which costs a
    // `sessions.working_dir` lookup — before they can reach the cwd-aware
    // `any_handlers_for` gate, so each one first runs the cwd-free
    // `scopes::definitely_no_handlers_for` pre-gate and returns early on a
    // "definitely nothing configured" answer. That pre-gate is pure
    // optimization, and an optimization that is too eager here fails SILENTLY:
    // every observation hook (SessionEnd / Notification / FileChanged /
    // Subagent* / Task* / PermissionDenied / …) would simply stop firing, with
    // no error anywhere. `fire_*` returns nothing, so the only way to observe it
    // is a side channel — the hook journals to a file.
    let fired = tmp.path().join("fire-and-forget.log");
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        // The PostToolUse(Write) injector rides along so the config-clear
        // section below still proves hot-reload REMOVES handlers, rather than
        // trivially passing on a config that never had a PostToolUse hook.
        c.hooks = serde_json::from_str(&format!(
            r#"{{
                "PostToolUse": [
                    {{ "matcher": "Write", "hooks": [
                        {{ "type": "command", "shell": "bash",
                          "command": "printf '%s' '{{\"hookSpecificOutput\":{{\"additionalContext\":\"WROTE_VIA_HOOK\"}}}}'" }}
                    ]}}
                ],
                "SessionEnd": [
                    {{ "hooks": [
                        {{ "type": "command", "shell": "bash",
                          "command": "cat >/dev/null; echo fired >> '{log}'" }}
                    ]}}
                ]
            }}"#,
            log = fired.display()
        ))
        .unwrap();
        Ok(())
    })
    .expect("write session-end hook config");
    hooks::registry::reload_from_config();
    let still_wired =
        HookDispatcher::dispatch(HookEvent::PostToolUse, post_tool_use("write", "c-ff")).await;
    assert_eq!(
        still_wired.merged_additional_context().as_deref(),
        Some("WROTE_VIA_HOOK"),
        "PostToolUse(Write) injector must be live before the config-clear section"
    );
    hooks::fire_session_end("e2e-sess", "logout");
    let journal = wait_for_file(&fired, "the SessionEnd fire_* hook").await;
    assert!(
        journal.contains("fired"),
        "fire_session_end must actually run a configured handler — an over-eager \
         cwd-free pre-gate would skip it with no error anywhere; journal={journal:?}"
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

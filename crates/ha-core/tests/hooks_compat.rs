//! Official Claude Code hook-script compatibility suite (design §17.4).
//!
//! Runs UNMODIFIED official/community-style hook scripts as fixtures against the
//! full dispatch chain and asserts the system serializes inputs and honors
//! outputs exactly as those scripts expect. This is the hard acceptance test
//! for goal G1 ("字段级对齐 Claude Code hooks 协议"): if the payload field names
//! or the exit-code / JSON output protocol drift, these scripts break.
//!
//! The fixtures live in `tests/fixtures/hooks/claude-code-compat/` and parse
//! stdin with `jq`, exactly as the official Bash-validator examples do. `jq` is
//! preinstalled on the GitHub-hosted CI runners; on a machine without it the
//! suite SKIPS (with a loud notice) rather than failing, so a `jq`-less local
//! `git push` isn't blocked.
//!
//! Single `#[test]` per binary so the process-global OnceLocks (SessionDB /
//! EventBus / config) don't race with the other hooks integration tests.
//!
//! Unix-only: the hooks shell out to `bash`.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

use ha_core::hooks::{
    self, CommonHookInput, HookDecision, HookDispatcher, HookEvent, HookInput, HooksConfig,
    PermissionMode,
};

/// Absolute path to a fixture script, resolved at compile time against the
/// crate manifest dir so the test is location-independent.
fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/hooks/claude-code-compat/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

/// Whether `jq` is callable. The official scripts depend on it; without it the
/// suite can't run authentically, so we skip instead of asserting.
fn jq_available() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Install a single-command hook on `event` pointing at fixture `script`, then
/// reload the registry so the next dispatch picks it up. `bash <path>` form so
/// the script runs even without the executable bit (and stdin still carries the
/// hook-input JSON the runner pipes in).
fn install_hook(event: &str, script: &str) {
    let cfg: HooksConfig = serde_json::from_str(&format!(
        r#"{{ "{event}": [ {{ "hooks": [ {{ "type": "command", "shell": "bash", "command": "bash {script}" }} ] }} ] }}"#
    ))
    .expect("parse hooks config");
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = cfg.clone();
        Ok(())
    })
    .expect("write hooks config");
    hooks::registry::reload_from_config();
}

fn pre_tool_use(cwd: PathBuf, command: &str) -> HookInput {
    HookInput::PreToolUse {
        common: CommonHookInput {
            session_id: "compat-sess".into(),
            prompt_id: None,
            transcript_path: cwd.join("transcript.jsonl"),
            cwd,
            permission_mode: PermissionMode::Default,
            effort: None,
            hook_event_name: "PreToolUse".into(),
            agent_id: None,
            agent_type: None,
        },
        // Internal tool name (not the Claude Code alias) — the fixtures gate on
        // `.tool_input.command`, which IS field-aligned, so the alias divergence
        // (documented as a red line in hooks.md) doesn't affect them.
        tool_name: "exec".into(),
        tool_input: serde_json::json!({ "command": command }),
        tool_use_id: "call-1".into(),
    }
}

fn user_prompt_submit(prompt: &str) -> HookInput {
    HookInput::UserPromptSubmit {
        common: CommonHookInput {
            session_id: "compat-sess".into(),
            prompt_id: None,
            transcript_path: PathBuf::from("/tmp/compat.jsonl"),
            cwd: std::env::temp_dir(),
            permission_mode: PermissionMode::Default,
            effort: None,
            hook_event_name: "UserPromptSubmit".into(),
            agent_id: None,
            agent_type: None,
        },
        prompt: prompt.into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_claude_code_scripts_run_unmodified() {
    if !jq_available() {
        eprintln!(
            "SKIP hooks_compat: `jq` not installed — the official-script compat \
             suite needs it. (CI runners ship jq; this skip only affects jq-less \
             local environments.)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // 1. block_rm.sh — PreToolUse, `jq .tool_input.command` + exit 2 → Block.
    //    Proves tool_input.command alignment and the exit-2 block protocol.
    install_hook("PreToolUse", &fixture("block_rm.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(tmp.path().to_path_buf(), "rm -rf /tmp/whatever"),
    )
    .await;
    assert!(
        matches!(out.decision, HookDecision::Block { .. }),
        "block_rm.sh must block an `rm -rf` command via exit 2, got {:?}",
        out.decision
    );
    // A benign command must pass (exit 0, no block) — guards against the script
    // matching everything.
    let allow = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(tmp.path().to_path_buf(), "ls -la"),
    )
    .await;
    assert!(
        matches!(allow.decision, HookDecision::Allow),
        "block_rm.sh must allow a benign command, got {:?}",
        allow.decision
    );

    // 2. pretooluse_deny.sh — PreToolUse, exit 0 + JSON permissionDecision:deny.
    //    Proves the official JSON decision schema parses into a hard Deny.
    install_hook("PreToolUse", &fixture("pretooluse_deny.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(tmp.path().to_path_buf(), "echo hi > /etc/motd"),
    )
    .await;
    assert!(
        matches!(out.decision, HookDecision::Deny { .. }),
        "pretooluse_deny.sh must deny a /etc write via permissionDecision JSON, got {:?}",
        out.decision
    );

    // 3. prompt_context.sh — UserPromptSubmit, `jq .prompt` + additionalContext.
    //    Proves the prompt field is delivered and additionalContext is injected.
    install_hook("UserPromptSubmit", &fixture("prompt_context.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::UserPromptSubmit,
        user_prompt_submit("refactor X"),
    )
    .await;
    let ctx = out
        .merged_additional_context()
        .expect("prompt_context.sh must inject additionalContext");
    assert!(
        ctx.contains("house-rules") && ctx.contains("prompt_len=10"),
        "additionalContext must reflect the .prompt field (len of \"refactor X\" = 10), got {ctx:?}"
    );

    // 4. projectdir_env.sh — PreToolUse, requires $CLAUDE_PROJECT_DIR == .cwd.
    //    Proves the dual env injection (G7) and that it equals the payload cwd.
    install_hook("PreToolUse", &fixture("projectdir_env.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(tmp.path().to_path_buf(), "noop"),
    )
    .await;
    let ctx = out.merged_additional_context().unwrap_or_default();
    assert!(
        ctx.contains("project_dir_ok:"),
        "projectdir_env.sh must confirm CLAUDE_PROJECT_DIR == payload cwd, got context {ctx:?}"
    );
}

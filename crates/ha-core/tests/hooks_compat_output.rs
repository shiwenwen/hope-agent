//! Official Claude Code hook **output**-protocol compatibility suite.
//!
//! Sibling of `hooks_compat.rs`, which pins the INPUT half (payload field
//! names). This binary pins the OUTPUT half: what an unmodified official-shaped
//! hook script prints on stdout, and which [`HookOutcome`] field the dispatch
//! chain lands it on.
//!
//! Motivation: every surface asserted here (`updatedToolOutput`, the structured
//! `decision.behavior` object, `retry`, `suppressOutput`, `permissionDecision:
//! "defer"`, and the explicit-vs-default allow distinction) is covered ONLY by
//! `src/hooks/parse.rs` unit tests that hand-feed a `RawHookResult` struct. No
//! real script had ever produced them, so a break anywhere between "bash writes
//! JSON to stdout" and "the outcome carries the value" — runner capture, JSON
//! trimming, aggregation, the observation-only downgrade — was invisible. These
//! fixtures close that gap end-to-end.
//!
//! Fixtures live in `tests/fixtures/hooks/claude-code-compat/` and parse stdin
//! with `jq`, exactly as the official Bash examples do. `jq` ships on the
//! GitHub-hosted CI runners; on a machine without it the suite SKIPS (loudly)
//! rather than failing, so a `jq`-less local `git push` isn't blocked.
//!
//! Single `#[test]` per binary so the process-global OnceLocks (SessionDB /
//! EventBus / config) and the `reload_from_config()` registry swap don't race
//! with the other hooks integration tests.
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

const SESSION: &str = "compat-out-sess";

fn common(event: &str, cwd: PathBuf) -> CommonHookInput {
    CommonHookInput {
        session_id: SESSION.into(),
        prompt_id: None,
        transcript_path: cwd.join("transcript.jsonl"),
        cwd,
        permission_mode: PermissionMode::Default,
        effort: None,
        hook_event_name: event.into(),
        agent_id: None,
        agent_type: None,
    }
}

fn pre_tool_use(cwd: PathBuf, command: &str) -> HookInput {
    HookInput::PreToolUse {
        common: common("PreToolUse", cwd),
        // Internal tool name (not the Claude Code alias) — the fixtures gate on
        // `.tool_input.command`, which IS field-aligned, so the alias divergence
        // (documented as a red line in hooks.md) doesn't affect them.
        tool_name: "exec".into(),
        tool_input: serde_json::json!({ "command": command }),
        tool_use_id: "call-1".into(),
    }
}

fn post_tool_use(cwd: PathBuf, tool_response: serde_json::Value) -> HookInput {
    HookInput::PostToolUse {
        common: common("PostToolUse", cwd),
        tool_name: "exec".into(),
        tool_input: serde_json::json!({ "command": "printenv API_KEY" }),
        tool_response,
        tool_use_id: "call-1".into(),
        job_id: None,
    }
}

fn permission_request(cwd: PathBuf) -> HookInput {
    HookInput::PermissionRequest {
        common: common("PermissionRequest", cwd),
        tool_name: Some("write".into()),
        tool_input: Some(serde_json::json!({ "path": "/prod/config.yaml" })),
        command: "tool: write /prod/config.yaml".into(),
        tool_use_id: Some("call-1".into()),
        job_id: None,
    }
}

fn permission_denied(cwd: PathBuf) -> HookInput {
    HookInput::PermissionDenied {
        common: common("PermissionDenied", cwd),
        tool_name: Some("write".into()),
        tool_input: Some(serde_json::json!({ "path": "/prod/config.yaml" })),
        command: "tool: write /prod/config.yaml".into(),
        reason: "user_declined".into(),
        tool_use_id: Some("call-1".into()),
        job_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_output_protocol_fields_are_honored() {
    if !jq_available() {
        eprintln!(
            "SKIP hooks_compat_output: `jq` not installed — the official-script \
             output-protocol compat suite needs it. (CI runners ship jq; this \
             skip only affects jq-less local environments.)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");
    let cwd = tmp.path().to_path_buf();

    // 1. posttooluse_redact.sh — PostToolUse, `jq .tool_response` + official
    //    `hookSpecificOutput.updatedToolOutput`. parse.rs maps that key onto
    //    `HookOutcome::updated_mcp_output` (deliberately NOT named
    //    `updated_tool_output`), so a rename on either side breaks here.
    install_hook("PostToolUse", &fixture("posttooluse_redact.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PostToolUse,
        post_tool_use(cwd.clone(), serde_json::json!("token sk-abc123")),
    )
    .await;
    let updated = out
        .updated_mcp_output
        .as_ref()
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    assert_eq!(
        updated, "token REDACTED",
        "posttooluse_redact.sh must land its `updatedToolOutput` on \
         HookOutcome::updated_mcp_output with the sk- token scrubbed, got {:?}",
        out.updated_mcp_output
    );

    // 2. permissionrequest_behavior_deny.sh — PermissionRequest, official
    //    structured `hookSpecificOutput.decision.behavior:"deny"` with the
    //    justification on the TOP-LEVEL `reason`. PermissionRequest is not
    //    observation-only, so the Deny survives the dispatch downgrade pass.
    install_hook(
        "PermissionRequest",
        &fixture("permissionrequest_behavior_deny.sh"),
    );
    let out = HookDispatcher::dispatch(
        HookEvent::PermissionRequest,
        permission_request(cwd.clone()),
    )
    .await;
    assert!(
        matches!(&out.decision, HookDecision::Deny { reason } if reason == "policy: no prod writes"),
        "permissionrequest_behavior_deny.sh must deny via \
         hookSpecificOutput.decision.behavior and carry the top-level `reason`, got {:?}",
        out.decision
    );
    assert_eq!(
        out.block_reason().as_deref(),
        Some("policy: no prod writes"),
        "the deny must read as a veto at the approval gate's accessor, got {:?}",
        out.block_reason()
    );

    // 3. permissionrequest_behavior_allow.sh — same object with `"allow"`.
    //    Parse-level truth: the decision stays `Allow` and the EXPLICIT-allow
    //    flag (`permission_allow`) is set — that flag is the only thing telling
    //    a deliberate allow apart from the default `Allow` a context-only hook
    //    yields. NOTE: an explicit hook allow is deliberately NOT an
    //    auto-approve at the approval gate — `tools/approval.rs` consumes
    //    PermissionRequest hooks DENY-ONLY (it only checks `block_reason()`) so
    //    a hook can never bypass the user prompt or strict mode. That policy
    //    lives there, not in this protocol layer; this section pins only what
    //    parse.rs produces.
    install_hook(
        "PermissionRequest",
        &fixture("permissionrequest_behavior_allow.sh"),
    );
    let out = HookDispatcher::dispatch(
        HookEvent::PermissionRequest,
        permission_request(cwd.clone()),
    )
    .await;
    assert_eq!(
        out.decision,
        HookDecision::Allow,
        "permissionrequest_behavior_allow.sh must not produce a blocking decision, got {:?}",
        out.decision
    );
    assert!(
        out.permission_allow,
        "permissionrequest_behavior_allow.sh must set the explicit-allow flag via \
         hookSpecificOutput.decision.behavior:\"allow\", got permission_allow={}",
        out.permission_allow
    );

    // 4. permissiondenied_retry.sh — PermissionDenied, official
    //    `hookSpecificOutput.retry:true`. `HookOutcome::retry` was a dead
    //    hard-coded `false` until recently; nothing but a unit test covered the
    //    parse. PermissionDenied IS observation-only, but the downgrade pass
    //    only neutralizes the decision — flags like `retry` ride through.
    install_hook("PermissionDenied", &fixture("permissiondenied_retry.sh"));
    let out =
        HookDispatcher::dispatch(HookEvent::PermissionDenied, permission_denied(cwd.clone())).await;
    assert!(
        out.retry,
        "permissiondenied_retry.sh must set HookOutcome::retry from \
         hookSpecificOutput.retry:true, got retry={}",
        out.retry
    );
    assert_eq!(
        out.decision,
        HookDecision::Allow,
        "PermissionDenied is observation-only — retry must not smuggle in a \
         blocking decision, got {:?}",
        out.decision
    );

    // 5. suppress_output.sh — the two TOP-LEVEL output keys (siblings of
    //    `hookSpecificOutput`): `suppressOutput` and `systemMessage`.
    //    PLAIN STATEMENT OF SCOPE: this pins the parse → aggregate plumbing
    //    ONLY. `HookOutcome::suppress_output` currently has NO production
    //    consumer — nothing in the transcript / UI path reads it — so this
    //    section must NOT be read as proving that anything is actually
    //    suppressed anywhere. It proves the field survives the trip.
    install_hook("PostToolUse", &fixture("suppress_output.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PostToolUse,
        post_tool_use(cwd.clone(), serde_json::json!("ok")),
    )
    .await;
    assert!(
        out.suppress_output,
        "suppress_output.sh must set HookOutcome::suppress_output from the \
         top-level `suppressOutput`, got suppress_output={}",
        out.suppress_output
    );
    assert_eq!(
        out.system_message.as_deref(),
        Some("hook ran quietly"),
        "suppress_output.sh must set HookOutcome::system_message from the \
         top-level `systemMessage`, got {:?}",
        out.system_message
    );

    // 6. pretooluse_defer.sh — official `permissionDecision:"defer"`.
    //    Recent behavior change: `defer` used to fall through to a silent
    //    `Allow`; it must now produce the distinct `Defer` decision so the
    //    normal permission flow actually runs. Unit-only until this fixture.
    install_hook("PreToolUse", &fixture("pretooluse_defer.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(cwd.clone(), "kubectl apply -f prod.yaml"),
    )
    .await;
    assert_eq!(
        out.decision,
        HookDecision::Defer,
        "pretooluse_defer.sh must produce Defer (not a silent Allow) from \
         permissionDecision:\"defer\", got {:?}",
        out.decision
    );
    assert!(
        !out.permission_allow,
        "a deferring hook must never read as an approval, got permission_allow={}",
        out.permission_allow
    );

    // 7. pretooluse_allow.sh — official `permissionDecision:"allow"` sets the
    //    explicit-allow flag (the deliberate auto-approve).
    install_hook("PreToolUse", &fixture("pretooluse_allow.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(cwd.clone(), "cargo check -p ha-core"),
    )
    .await;
    assert_eq!(
        out.decision,
        HookDecision::Allow,
        "pretooluse_allow.sh must produce Allow, got {:?}",
        out.decision
    );
    assert!(
        out.permission_allow,
        "pretooluse_allow.sh must set HookOutcome::permission_allow from \
         permissionDecision:\"allow\", got permission_allow={}",
        out.permission_allow
    );

    // 8. pretooluse_context_only.sh — the pair that makes §7 meaningful: an
    //    advisory hook returning ONLY `additionalContext` is also `Allow`, but
    //    the explicit-allow flag must stay FALSE. If it didn't, every advisory
    //    hook in the wild would silently auto-approve tool calls.
    install_hook("PreToolUse", &fixture("pretooluse_context_only.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        pre_tool_use(cwd.clone(), "cargo check -p ha-core"),
    )
    .await;
    let ctx = out
        .merged_additional_context()
        .expect("pretooluse_context_only.sh must inject additionalContext");
    assert!(
        ctx.contains("[advisory] observed command: cargo check -p ha-core"),
        "pretooluse_context_only.sh must echo back the .tool_input.command it \
         observed, got {ctx:?}"
    );
    assert!(
        !out.permission_allow,
        "a context-only hook must NOT set permission_allow (that would make every \
         advisory hook an auto-approve), got permission_allow={}",
        out.permission_allow
    );
    assert_eq!(
        out.decision,
        HookDecision::Allow,
        "pretooluse_context_only.sh must still resolve to Allow, got {:?}",
        out.decision
    );
}

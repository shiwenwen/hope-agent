//! Official-veto compat suite for the newly gate-capable hook events.
//!
//! Six events were promoted from observation-only to gate-capable: `Stop`,
//! `PostToolBatch`, `TaskCreated`, `TaskCompleted`, `UserPromptExpansion`,
//! `PermissionRequest`. Until now only their METADATA was pinned
//! (`blocking_events_are_not_observation_only` in `hooks/types.rs`) plus two
//! inline `exit 2` unit tests — no official-shaped script had ever driven them
//! end-to-end. If any of them were re-added to `HookEvent::is_observation_only`,
//! `downgrade_block_on_observation` would silently neutralize every official
//! veto script and nothing in the suite would fail.
//!
//! This binary runs UNMODIFIED official-shaped fixtures through the full
//! dispatch chain and asserts the veto survives, including `Stop`'s inverted
//! `block` = "keep going" semantics. Section 3 is the negative control: the same
//! veto script on a still-observation-only event (`PostToolUse`) must come back
//! `Allow`, proving sections 1–2 measure a real property rather than "the
//! downgrade never runs".
//!
//! Fixtures live in `tests/fixtures/hooks/claude-code-compat/`; the suite SKIPS
//! (loudly) without `jq` so a jq-less local `git push` isn't blocked, matching
//! `hooks_compat.rs`.
//!
//! Single `#[tokio::test]` per binary: `install_hook` writes process-global
//! config and `reload_from_config()` swaps a global registry, so a second test
//! fn here would race on parallel threads.
//!
//! Unix-only: the hooks shell out to `bash`.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

use ha_core::hooks::{
    self, CommonHookInput, HookDecision, HookDispatcher, HookEvent, HookInput, HooksConfig,
    PermissionMode, ToolCallSummary,
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

/// Common block for a veto dispatch. No matcher is installed, so every group
/// compiles to `MatcherKind::Wildcard` and the per-event matcher target is
/// irrelevant here.
fn common(event: &str) -> CommonHookInput {
    CommonHookInput {
        session_id: "veto-compat-sess".into(),
        prompt_id: None,
        transcript_path: PathBuf::from("/tmp/veto-compat.jsonl"),
        cwd: std::env::temp_dir(),
        permission_mode: PermissionMode::Default,
        effort: None,
        hook_event_name: event.into(),
        agent_id: None,
        agent_type: None,
    }
}

/// A minimal-but-valid input for each of the five forward-veto events (the
/// ones whose call site aborts the action on a blocking decision). `Stop` is
/// deliberately absent — its `block` is inverted and is covered by section 2.
fn forward_veto_input(event: HookEvent) -> HookInput {
    match event {
        HookEvent::TaskCreated => HookInput::TaskCreated {
            common: common("TaskCreated"),
            content: "ship the release".into(),
            active_form: None,
            batch_id: "batch-1".into(),
        },
        HookEvent::TaskCompleted => HookInput::TaskCompleted {
            common: common("TaskCompleted"),
            task_id: 1,
            content: "ship the release".into(),
        },
        HookEvent::UserPromptExpansion => HookInput::UserPromptExpansion {
            common: common("UserPromptExpansion"),
            command: "deploy".into(),
            command_text: "/deploy prod".into(),
        },
        HookEvent::PermissionRequest => HookInput::PermissionRequest {
            common: common("PermissionRequest"),
            tool_name: Some("exec".into()),
            tool_input: Some(serde_json::json!({ "command": "npm test" })),
            command: "tool: exec npm test".into(),
            tool_use_id: Some("call-veto".into()),
            job_id: None,
        },
        HookEvent::PostToolBatch => HookInput::PostToolBatch {
            common: common("PostToolBatch"),
            round: 0,
            tool_names: vec!["exec".into()],
            tool_calls: vec![ToolCallSummary {
                tool_name: "exec".into(),
                tool_input: serde_json::json!({ "command": "npm test" }),
                tool_response: serde_json::json!("ok"),
            }],
        },
        other => unreachable!("{other:?} is not a forward-veto event"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn newly_blocking_events_honor_official_veto() {
    if !jq_available() {
        eprintln!(
            "SKIP hooks_compat_blocking: `jq` not installed — the official-script \
             compat suites need it. (CI runners ship jq; this skip only affects \
             jq-less local environments.)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // 1. veto_exit2.sh — the canonical official veto shorthand (`exit 2`, stderr
    //    = reason) on each of the five forward-veto events. `parse::parse` maps
    //    `Some(2)` to `HookDecision::Block { reason: stderr.trim() }`, and
    //    `HookOutcome::block_reason` returns that reason verbatim — so the
    //    expected value is the TRIMMED stderr line (echo's trailing newline is
    //    stripped, nothing else is added or wrapped).
    for event in [
        HookEvent::TaskCreated,
        HookEvent::TaskCompleted,
        HookEvent::UserPromptExpansion,
        HookEvent::PermissionRequest,
        HookEvent::PostToolBatch,
    ] {
        install_hook(event.as_str(), &fixture("veto_exit2.sh"));
        let out = HookDispatcher::dispatch(event, forward_veto_input(event)).await;
        assert_eq!(
            out.block_reason().as_deref(),
            Some("vetoed by policy"),
            "veto_exit2.sh must veto {} via exit 2 (stderr is the reason); \
             a re-added is_observation_only entry would neutralize it. \
             decision={:?}, block_reason={:?}",
            event.as_str(),
            out.decision,
            out.block_reason()
        );
    }

    // 2. stop_block_continue.sh — Stop's INVERTED semantics in official JSON
    //    form. For every other event `decision:"block"` vetoes the action; for
    //    Stop the action IS stopping, so block means "do NOT stop, keep going"
    //    and `reason` is the feedback injected before the extra turn. Both
    //    accessors are asserted: `stop_wants_continue()` is the Stop-specific
    //    read (Deny/Block → Some(reason), `continue:false` deliberately
    //    excluded), while `block_reason()` sees the same Block and is Some —
    //    that is expected, NOT a bug to "fix": the Stop call site reads
    //    `stop_wants_continue`, never `block_reason`.
    install_hook("Stop", &fixture("stop_block_continue.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::Stop,
        HookInput::Stop {
            common: common("Stop"),
            status: "completed".into(),
            last_assistant_message: Some("all done".into()),
            stop_hook_active: false,
        },
    )
    .await;
    assert_eq!(
        out.stop_wants_continue().as_deref(),
        Some("tests still failing — keep going"),
        "stop_block_continue.sh must make Stop keep going (official block = \
         prevent stopping) and carry the feedback reason, got decision={:?}, \
         stop_wants_continue={:?}",
        out.decision,
        out.stop_wants_continue()
    );
    assert!(
        out.block_reason().is_some(),
        "the same Stop Block must also read as a block_reason (Stop is gate-capable, \
         so the decision is not downgraded), got {:?}",
        out.decision
    );

    // 3. NEGATIVE CONTROL — the very same veto script on `PostToolUse`, which is
    //    STILL observation-only, must come back `Allow`:
    //    `downgrade_block_on_observation` neutralizes it. Without this section a
    //    build where NOTHING is ever downgraded would pass section 1 vacuously.
    install_hook("PostToolUse", &fixture("veto_exit2.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PostToolUse,
        HookInput::PostToolUse {
            common: common("PostToolUse"),
            tool_name: "exec".into(),
            tool_input: serde_json::json!({ "command": "npm test" }),
            tool_response: serde_json::json!("ok"),
            tool_use_id: "call-observed".into(),
            job_id: None,
        },
    )
    .await;
    assert_eq!(
        out.decision,
        HookDecision::Allow,
        "veto_exit2.sh must be downgraded on PostToolUse (still observation-only) — \
         this is the control proving section 1 measures a real gate, got {:?}",
        out.decision
    );
    assert!(
        out.block_reason().is_none(),
        "a downgraded observation veto must expose no block_reason, got {:?}",
        out.block_reason()
    );
}

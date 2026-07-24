//! Official payload-field-name compatibility suite (design §17.4, goal G1).
//!
//! Sibling of `hooks_compat.rs`: that file pins the *protocol* (exit-2 block,
//! `permissionDecision` JSON, `additionalContext` injection, the
//! `$CLAUDE_PROJECT_DIR` env contract) but only ever reads three payload keys
//! (`.tool_input.command`, `.prompt`, `.cwd`). This file pins the **payload
//! field names** themselves: every key that was renamed to its official Claude
//! Code spelling (`SessionEnd.reason`, `Notification.type`,
//! `StopFailure.error_type`, `FileChanged.file_path`,
//! `UserPromptExpansion.command_name`/`raw_input`, `Task*.task_name`,
//! `WorktreeCreate.worktree_name`) or newly added (`tool_calls[]`,
//! `last_assistant_message`, `stop_hook_active`, `custom_instructions`,
//! `session_title`, the approval `tool_input`, `prompt_id`, `effort`) must
//! arrive under that exact key when an UNMODIFIED official-style `jq` script
//! reads it. Renames are invisible to Rust type-checking — only a script
//! parsing the wire JSON catches them.
//!
//! Each fixture proves delivery by echoing the value back as
//! `hookSpecificOutput.additionalContext`, which the assertions then read off
//! the [`HookOutcome`]. That trick is event-agnostic: observation-only events
//! neutralize a hook's *decision* (`downgrade_block_on_observation`) but always
//! preserve `additional_context`.
//!
//! The fixtures live in `tests/fixtures/hooks/claude-code-compat/` and parse
//! stdin with `jq`, exactly as the official Bash-validator examples do. `jq` is
//! preinstalled on the GitHub-hosted CI runners; on a machine without it the
//! suite SKIPS (with a loud notice) rather than failing, so a `jq`-less local
//! `git push` isn't blocked.
//!
//! Single `#[test]` per binary so the process-global OnceLocks (SessionDB /
//! EventBus / config) and the registry swap `install_hook` performs don't race
//! with the other hooks integration tests.
//!
//! Unix-only: the hooks shell out to `bash`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ha_core::hooks::{
    self, CommonHookInput, CompactTrigger, HookDispatcher, HookEffort, HookEvent, HookInput,
    HooksConfig, PermissionMode, SessionStartSource, ToolCallSummary,
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
/// hook-input JSON the runner pipes in). Replaces `c.hooks` wholesale, so each
/// numbered section below starts from a clean slate.
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

/// Same as [`install_hook`] but with a `matcher` on the group, so the hook only
/// runs when the event's matcher target matches. `matcher` is JSON-encoded
/// (rather than interpolated raw) so a regex containing backslashes survives
/// into the config verbatim.
fn install_hook_matched(event: &str, matcher: &str, script: &str) {
    let matcher_json = serde_json::to_string(matcher).expect("encode matcher");
    let cfg: HooksConfig = serde_json::from_str(&format!(
        r#"{{ "{event}": [ {{ "matcher": {matcher_json}, "hooks": [ {{ "type": "command", "shell": "bash", "command": "bash {script}" }} ] }} ] }}"#
    ))
    .expect("parse hooks config");
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks = cfg.clone();
        Ok(())
    })
    .expect("write hooks config");
    hooks::registry::reload_from_config();
}

/// The flattened common block every event carries. `CommonHookInput` has no
/// `Default`, so this spells out the full literal once.
fn common(session_id: &str, event: &str, cwd: &Path) -> CommonHookInput {
    CommonHookInput {
        session_id: session_id.into(),
        prompt_id: None,
        transcript_path: cwd.join("transcript.jsonl"),
        cwd: cwd.to_path_buf(),
        permission_mode: PermissionMode::Default,
        effort: None,
        hook_event_name: event.into(),
        agent_id: None,
        agent_type: None,
    }
}

/// The merged `additionalContext` a fixture echoed back, or a loud panic naming
/// the fixture that stayed silent (which is what a renamed/missing payload key
/// looks like: the script's `[ -n … ] || exit 1` guard fires, the runner treats
/// a non-2 exit as a non-blocking error, and nothing is injected).
fn context_of(outcome: &ha_core::hooks::HookOutcome, script: &str) -> String {
    outcome.merged_additional_context().unwrap_or_else(|| {
        panic!(
            "{script} injected no additionalContext — its `jq` read of the official \
             payload key came back empty (field renamed or dropped?)"
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_payload_field_names_match_upstream() {
    if !jq_available() {
        eprintln!(
            "SKIP hooks_compat_payload: `jq` not installed — the official-script \
             payload-field suite needs it. (CI runners ship jq; this skip only \
             affects jq-less local environments.)"
        );
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");
    let cwd: PathBuf = tmp.path().to_path_buf();
    let sid = "compat-payload-sess";

    // 1. session_end_reason.sh — SessionEnd, `jq .reason`. The variant field is
    //    internally `source`; only the serde rename puts it on the wire under
    //    the official `reason` key. The fixture also echoes a `stale_source=`
    //    marker if `.source` ever reappears, so an additive (rather than
    //    complete) rename fails here too.
    install_hook("SessionEnd", &fixture("session_end_reason.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::SessionEnd,
        HookInput::SessionEnd {
            common: common(sid, "SessionEnd", &cwd),
            source: "logout".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "session_end_reason.sh");
    assert!(
        ctx.contains("session_end_reason=logout"),
        "session_end_reason.sh must read the end cause from the official `.reason` \
         key (not `.source`), got {ctx:?}"
    );
    assert!(
        !ctx.contains("stale_source="),
        "SessionEnd must not ALSO emit the internal `source` key — the rename is \
         complete, not additive; got {ctx:?}"
    );

    // 2. notification_type.sh — Notification, `jq .type` + `.message`. The
    //    variant field is internally `notification_type`; official scripts
    //    branch on the bare `type` key.
    install_hook("Notification", &fixture("notification_type.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::Notification,
        HookInput::Notification {
            common: common(sid, "Notification", &cwd),
            notification_type: "idle_prompt".into(),
            message: "waiting for input".into(),
            title: Some("Hope Agent".into()),
        },
    )
    .await;
    let ctx = context_of(&out, "notification_type.sh");
    assert!(
        ctx.contains("notification_type=idle_prompt"),
        "notification_type.sh must read the kind from the official `.type` key \
         (not `.notification_type`), got {ctx:?}"
    );
    assert!(
        ctx.contains("notification_message=waiting for input"),
        "notification_type.sh must read the official `.message` key, got {ctx:?}"
    );

    // 3. stopfailure_error_type.sh — StopFailure, `jq .error_type`. Internally
    //    the variant field is `reason`; the fixture echoes a `reason=` fragment
    //    only if that legacy key is still on the wire, so its ABSENCE from the
    //    context is the assertion that the rename left nothing behind.
    install_hook("StopFailure", &fixture("stopfailure_error_type.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::StopFailure,
        HookInput::StopFailure {
            common: common(sid, "StopFailure", &cwd),
            reason: "provider_failed".into(),
            error: Some("upstream 503".into()),
        },
    )
    .await;
    let ctx = context_of(&out, "stopfailure_error_type.sh");
    assert!(
        ctx.contains("stop_error_type=provider_failed"),
        "stopfailure_error_type.sh must read the failure class from the official \
         `.error_type` key, got {ctx:?}"
    );
    assert!(
        !ctx.contains("reason="),
        "StopFailure must not ALSO emit the internal `reason` key alongside \
         `error_type` (a duplicate would let the stale spelling keep working), \
         got {ctx:?}"
    );

    // 4. filechanged_path.sh — FileChanged, `jq .file_path` + `.action`,
    //    installed behind the regex matcher `.*\.rs$`. Two claims in one: the
    //    payload key is the official `file_path` (internally `path`), AND
    //    matcher regexes are UNANCHORED against the matcher target — the `.txt`
    //    dispatch below must find no handler at all.
    install_hook_matched("FileChanged", r".*\.rs$", &fixture("filechanged_path.sh"));
    let rs_path = cwd.join("src").join("lib.rs").display().to_string();
    let out = HookDispatcher::dispatch(
        HookEvent::FileChanged,
        HookInput::FileChanged {
            common: common(sid, "FileChanged", &cwd),
            path: rs_path.clone(),
            action: "edit".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "filechanged_path.sh");
    assert!(
        ctx.contains(&format!("changed_file={rs_path}")),
        "filechanged_path.sh must read the path from the official `.file_path` key \
         (not `.path`) and the `.*\\.rs$` matcher must match it unanchored, got {ctx:?}"
    );
    assert!(
        ctx.contains("changed_action=edit"),
        "filechanged_path.sh must read the official `.action` key, got {ctx:?}"
    );
    let txt_path = cwd.join("notes.txt").display().to_string();
    let out = HookDispatcher::dispatch(
        HookEvent::FileChanged,
        HookInput::FileChanged {
            common: common(sid, "FileChanged", &cwd),
            path: txt_path.clone(),
            action: "edit".into(),
        },
    )
    .await;
    assert!(
        out.merged_additional_context().is_none(),
        "the `.*\\.rs$` matcher must NOT fire filechanged_path.sh for {txt_path} \
         (matcher target is the file path), got {:?}",
        out.merged_additional_context()
    );

    // 5. prompt_expansion_command.sh — UserPromptExpansion, `jq .command_name`
    //    + `.raw_input`. Internally `command` / `command_text`.
    install_hook(
        "UserPromptExpansion",
        &fixture("prompt_expansion_command.sh"),
    );
    let out = HookDispatcher::dispatch(
        HookEvent::UserPromptExpansion,
        HookInput::UserPromptExpansion {
            common: common(sid, "UserPromptExpansion", &cwd),
            command: "deploy".into(),
            command_text: "/deploy prod --dry-run".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "prompt_expansion_command.sh");
    assert!(
        ctx.contains("expansion_command=deploy"),
        "prompt_expansion_command.sh must read the slash command from the official \
         `.command_name` key (not `.command`), got {ctx:?}"
    );
    assert!(
        ctx.contains("expansion_raw=/deploy prod --dry-run"),
        "prompt_expansion_command.sh must read the full text from the official \
         `.raw_input` key (not `.command_text`), got {ctx:?}"
    );

    // 6. task_name.sh — TaskCreated AND TaskCompleted, `jq .task_name`. Both
    //    variants store the text in an internal `content` field, so both need
    //    the rename; one unmodified official script must work for both.
    install_hook("TaskCreated", &fixture("task_name.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::TaskCreated,
        HookInput::TaskCreated {
            common: common(sid, "TaskCreated", &cwd),
            content: "ship the payload suite".into(),
            active_form: Some("shipping the payload suite".into()),
            batch_id: "batch-1".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "task_name.sh");
    assert!(
        ctx.contains("task_event=TaskCreated") && ctx.contains("task_name=ship the payload suite"),
        "task_name.sh must read TaskCreated's text from the official `.task_name` \
         key (not `.content`), got {ctx:?}"
    );
    install_hook("TaskCompleted", &fixture("task_name.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::TaskCompleted,
        HookInput::TaskCompleted {
            common: common(sid, "TaskCompleted", &cwd),
            task_id: 42,
            content: "ship the payload suite".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "task_name.sh");
    assert!(
        ctx.contains("task_event=TaskCompleted")
            && ctx.contains("task_name=ship the payload suite"),
        "task_name.sh must read TaskCompleted's text from the official `.task_name` \
         key too (same rename, second variant), got {ctx:?}"
    );

    // 7. worktree_name.sh — WorktreeCreate, `jq .worktree_name` (internally
    //    `name`) IN and `hookSpecificOutput.worktreePath` OUT. Both halves of
    //    the official contract in one dispatch.
    install_hook("WorktreeCreate", &fixture("worktree_name.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::WorktreeCreate,
        HookInput::WorktreeCreate {
            common: common(sid, "WorktreeCreate", &cwd),
            name: "wt-alpha".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "worktree_name.sh");
    assert!(
        ctx.contains("worktree_name=wt-alpha"),
        "worktree_name.sh must read the worktree from the official `.worktree_name` \
         key (not `.name`), got {ctx:?}"
    );
    assert_eq!(
        out.worktree_path.as_deref(),
        Some("/tmp/worktrees/wt-alpha"),
        "worktree_name.sh's `hookSpecificOutput.worktreePath` must land on \
         HookOutcome::worktree_path, got {:?}",
        out.worktree_path
    );

    // 8. subagent_agent_type.sh — SubagentStart AND SubagentStop,
    //    `jq .agent_type`. The variant field and the flattened common field
    //    share the official key, so production keeps `common.agent_type = None`
    //    (that is what `observation_common` hard-codes) and exactly one
    //    `"agent_type"` key may hit the wire. `jq` would silently keep only the
    //    last of two duplicates, so the fixture also greps the raw stdin.
    install_hook("SubagentStart", &fixture("subagent_agent_type.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::SubagentStart,
        HookInput::SubagentStart {
            // `common.agent_type` stays None — the production shape.
            common: common(sid, "SubagentStart", &cwd),
            subagent_id: "sa-1".into(),
            agent_type: "code-reviewer".into(),
            run_id: "run-1".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "subagent_agent_type.sh");
    assert!(
        ctx.contains("subagent_event=SubagentStart")
            && ctx.contains("subagent_agent_type=code-reviewer"),
        "subagent_agent_type.sh must read the sub-agent type from the official \
         `.agent_type` key on SubagentStart, got {ctx:?}"
    );
    assert!(
        ctx.contains("keys=1"),
        "SubagentStart must emit exactly ONE raw \"agent_type\" key — a second \
         (from a populated common block) would be silently swallowed by jq; got {ctx:?}"
    );
    install_hook("SubagentStop", &fixture("subagent_agent_type.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::SubagentStop,
        HookInput::SubagentStop {
            common: common(sid, "SubagentStop", &cwd),
            subagent_id: "sa-1".into(),
            agent_type: "code-reviewer".into(),
            run_id: "run-1".into(),
            status: "completed".into(),
            last_assistant_message: Some("review done".into()),
        },
    )
    .await;
    let ctx = context_of(&out, "subagent_agent_type.sh");
    assert!(
        ctx.contains("subagent_event=SubagentStop")
            && ctx.contains("subagent_agent_type=code-reviewer"),
        "subagent_agent_type.sh must read `.agent_type` on SubagentStop too, got {ctx:?}"
    );
    assert!(
        ctx.contains("keys=1"),
        "SubagentStop must emit exactly ONE raw \"agent_type\" key, got {ctx:?}"
    );

    // 9. post_tool_batch_calls.sh — PostToolBatch, `jq .tool_calls | length` +
    //    `.tool_calls[0].tool_name` / `.tool_response`. The official array is
    //    new (Hope Agent's flat `tool_names[]` extension predates it), so a
    //    script reading only `tool_names` would never notice it regressing.
    install_hook("PostToolBatch", &fixture("post_tool_batch_calls.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PostToolBatch,
        HookInput::PostToolBatch {
            common: common(sid, "PostToolBatch", &cwd),
            round: 0,
            tool_names: vec!["exec".into(), "read".into()],
            tool_calls: vec![
                ToolCallSummary {
                    tool_name: "exec".into(),
                    tool_input: serde_json::json!({ "command": "npm test" }),
                    tool_response: serde_json::json!("batch-ok"),
                },
                ToolCallSummary {
                    tool_name: "read".into(),
                    tool_input: serde_json::json!({ "path": "/a" }),
                    // Null is the official shape for a call that failed.
                    tool_response: serde_json::Value::Null,
                },
            ],
        },
    )
    .await;
    let ctx = context_of(&out, "post_tool_batch_calls.sh");
    assert!(
        ctx.contains("batch_calls=2"),
        "post_tool_batch_calls.sh must see both entries under the official \
         `.tool_calls[]` array, got {ctx:?}"
    );
    assert!(
        ctx.contains("batch_first_tool=exec"),
        "post_tool_batch_calls.sh must read `.tool_calls[0].tool_name`, got {ctx:?}"
    );
    assert!(
        ctx.contains("batch_first_response=batch-ok"),
        "post_tool_batch_calls.sh must read `.tool_calls[0].tool_response`, got {ctx:?}"
    );

    // 10. stop_last_message.sh — Stop, `jq .last_assistant_message` +
    //     `.stop_hook_active`. The latter is the flag an official script checks
    //     to avoid re-driving a turn a previous Stop hook already re-drove.
    install_hook("Stop", &fixture("stop_last_message.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::Stop,
        HookInput::Stop {
            common: common(sid, "Stop", &cwd),
            status: "completed".into(),
            last_assistant_message: Some("all done".into()),
            stop_hook_active: true,
        },
    )
    .await;
    let ctx = context_of(&out, "stop_last_message.sh");
    assert!(
        ctx.contains("stop_last_message=all done"),
        "stop_last_message.sh must read the turn's final text from the official \
         `.last_assistant_message` key, got {ctx:?}"
    );
    assert!(
        ctx.contains("stop_hook_active=true"),
        "stop_last_message.sh must see the official `.stop_hook_active` flag as a \
         real JSON bool (an omitted key would leave a script unable to break its \
         own continue loop), got {ctx:?}"
    );

    // 11. session_start_fork.sh — SessionStart, `jq .source` + `.session_title`.
    //     `fork` is the newest official source token and `session_title` the
    //     newest SessionStart field. Its own session id: `SessionStart` is
    //     claimed once per session id process-globally.
    install_hook("SessionStart", &fixture("session_start_fork.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::SessionStart,
        HookInput::SessionStart {
            common: common("compat-payload-sess-start", "SessionStart", &cwd),
            source: SessionStartSource::Fork,
            model: "test-model".into(),
            session_title: Some("Forked Session".into()),
        },
    )
    .await;
    let ctx = context_of(&out, "session_start_fork.sh");
    assert!(
        ctx.contains("src=fork"),
        "session_start_fork.sh must see SessionStartSource::Fork serialized as the \
         official lowercase `fork` under `.source`, got {ctx:?}"
    );
    assert!(
        ctx.contains("session_title=Forked Session"),
        "session_start_fork.sh must read the official `.session_title` key, got {ctx:?}"
    );

    // 12. precompact_instructions.sh — PreCompact, `jq .custom_instructions` +
    //     `.trigger`. The user's compaction guidance is only reachable to a hook
    //     through this key.
    install_hook("PreCompact", &fixture("precompact_instructions.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PreCompact,
        HookInput::PreCompact {
            common: common(sid, "PreCompact", &cwd),
            trigger: CompactTrigger::Manual,
            usage_ratio: 0.82,
            custom_instructions: Some("keep the API notes".into()),
        },
    )
    .await;
    let ctx = context_of(&out, "precompact_instructions.sh");
    assert!(
        ctx.contains("compact_trigger=manual"),
        "precompact_instructions.sh must see CompactTrigger::Manual as the official \
         lowercase `manual` under `.trigger`, got {ctx:?}"
    );
    assert!(
        ctx.contains("compact_instructions=keep the API notes"),
        "precompact_instructions.sh must read the user's guidance from the official \
         `.custom_instructions` key, got {ctx:?}"
    );

    // 13. permission_tool_input.sh — PermissionRequest AND PermissionDenied,
    //     `jq .tool_name` + `.tool_input.command`. Freshly wired through the
    //     approval chain: one official policy script must be able to gate
    //     `.tool_input.command` on the approval events exactly as on PreToolUse.
    install_hook("PermissionRequest", &fixture("permission_tool_input.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PermissionRequest,
        HookInput::PermissionRequest {
            common: common(sid, "PermissionRequest", &cwd),
            tool_name: Some("exec".into()),
            tool_input: Some(serde_json::json!({ "command": "npm test" })),
            command: "tool: exec".into(),
            tool_use_id: Some("call-1".into()),
            job_id: None,
        },
    )
    .await;
    let ctx = context_of(&out, "permission_tool_input.sh");
    assert!(
        ctx.contains("perm_event=PermissionRequest") && ctx.contains("perm_tool=exec"),
        "permission_tool_input.sh must read the official `.tool_name` key on \
         PermissionRequest, got {ctx:?}"
    );
    assert!(
        ctx.contains("perm_command=npm test"),
        "permission_tool_input.sh must reach the args through the official \
         `.tool_input.command` path on PermissionRequest (not only the flat \
         `command` extension), got {ctx:?}"
    );
    install_hook("PermissionDenied", &fixture("permission_tool_input.sh"));
    let out = HookDispatcher::dispatch(
        HookEvent::PermissionDenied,
        HookInput::PermissionDenied {
            common: common(sid, "PermissionDenied", &cwd),
            tool_name: Some("exec".into()),
            tool_input: Some(serde_json::json!({ "command": "npm test" })),
            command: "tool: exec".into(),
            reason: "user_declined".into(),
            tool_use_id: Some("call-1".into()),
            job_id: None,
        },
    )
    .await;
    let ctx = context_of(&out, "permission_tool_input.sh");
    assert!(
        ctx.contains("perm_event=PermissionDenied") && ctx.contains("perm_tool=exec"),
        "permission_tool_input.sh must read the official `.tool_name` key on \
         PermissionDenied too, got {ctx:?}"
    );
    assert!(
        ctx.contains("perm_command=npm test"),
        "permission_tool_input.sh must reach `.tool_input.command` on \
         PermissionDenied too, got {ctx:?}"
    );

    // 14. prompt_id_effort.sh — PreToolUse carrying the COMMON block:
    //     `jq .prompt_id` + `.effort.level`, cross-checked against the
    //     `$CLAUDE_EFFORT` env var (modelled on projectdir_env.sh's
    //     `$CLAUDE_PROJECT_DIR` check). Pins the payload AND `hooks/env.rs` in
    //     one dispatch — the two must agree or a script that reads one and keys
    //     state off the other corrupts itself.
    install_hook("PreToolUse", &fixture("prompt_id_effort.sh"));
    let mut pre_common = common(sid, "PreToolUse", &cwd);
    pre_common.prompt_id = Some("p-1".into());
    pre_common.effort = Some(HookEffort {
        level: "high".into(),
    });
    let out = HookDispatcher::dispatch(
        HookEvent::PreToolUse,
        HookInput::PreToolUse {
            common: pre_common,
            tool_name: "exec".into(),
            tool_input: serde_json::json!({ "command": "noop" }),
            tool_use_id: "call-1".into(),
        },
    )
    .await;
    let ctx = context_of(&out, "prompt_id_effort.sh");
    assert!(
        ctx.contains("prompt_id=p-1"),
        "prompt_id_effort.sh must read the per-turn correlation id from the official \
         `.prompt_id` key, got {ctx:?}"
    );
    assert!(
        ctx.contains("effort=high"),
        "prompt_id_effort.sh must read the level from the official nested \
         `.effort.level` object (not a flat `effort` string), got {ctx:?}"
    );
    assert!(
        ctx.contains("env_ok"),
        "prompt_id_effort.sh must confirm $CLAUDE_EFFORT == .effort.level — \
         hooks/env.rs publishes the same level it puts in the payload; got {ctx:?}"
    );

    // 15. user_prompt_submit_id.sh — `prompt_id` on UserPromptSubmit, through
    //     the real `hooks::fire_user_prompt_submit` helper rather than a
    //     hand-built input.
    //
    //     Every other section here dispatches an input the test constructed, so
    //     it pins serialization only. This one pins the *plumbing*: the id is
    //     supplied by the entry point (`PreflightArgs.turn_id`) and OVERRIDES
    //     the `active_turn` registry lookup inside `observation_common`. This
    //     test process holds no active turn at all — the registry lookup would
    //     return `None` — so the id in the payload can only have come from the
    //     caller. Drop the override in `fire_user_prompt_submit` and the fixture
    //     exits 1 on its `[ -n "$prompt_id" ]` guard, injecting nothing, and
    //     `context_of` panics.
    install_hook("UserPromptSubmit", &fixture("user_prompt_submit_id.sh"));
    let out =
        hooks::fire_user_prompt_submit(sid, Some("ha-main"), "refactor X", "turn-abc-123").await;
    let ctx = context_of(&out, "user_prompt_submit_id.sh");
    assert!(
        ctx.contains("prompt_id=turn-abc-123"),
        "UserPromptSubmit must carry the caller-minted turn id as `.prompt_id` even with \
         no active turn registered (IM fires pre-acquire; ACP never acquires), got {ctx:?}"
    );
    assert!(
        ctx.contains("prompt=refactor X"),
        "the raw user text must still arrive under the official `.prompt` key, got {ctx:?}"
    );

    // An entry point with no turn to run passes `""`, which must leave the key
    // ABSENT rather than emit an empty string — `skip_serializing_if` keeps
    // "no turn" distinguishable from "a turn whose id is empty". The fixture's
    // guard turns that into a non-blocking error with no context injected.
    let bare = hooks::fire_user_prompt_submit(sid, Some("ha-main"), "no turn", "").await;
    assert!(
        bare.merged_additional_context().is_none(),
        "an empty turn_id must omit `.prompt_id` entirely (the fixture then fails its \
         guard and injects nothing), got {:?}",
        bare.merged_additional_context()
    );

    // 16. permission_tool_input.sh again — this time through the real
    //     `hooks::dispatch_permission_request` helper rather than a hand-built
    //     input, the same way section 15 does for UserPromptSubmit.
    //
    //     Section 13 pins SERIALIZATION (a `HookInput` with `tool_input` set
    //     reaches the script under `.tool_input`). This pins the PLUMBING: the
    //     approval layer passes the args in as a borrow and the helper is what
    //     clones them into the payload. Both were hard-coded `None` until the
    //     approval chain threaded them through, and a regression to `None` there
    //     is invisible to section 13.
    install_hook("PermissionRequest", &fixture("permission_tool_input.sh"));
    let out = hooks::dispatch_permission_request(
        Some(sid),
        Some("exec"),
        Some(&serde_json::json!({ "command": "npm test" })),
        "tool: exec npm test",
        Some("call-plumbed"),
    )
    .await;
    let ctx = context_of(&out, "permission_tool_input.sh");
    assert!(
        ctx.contains("perm_tool=exec") && ctx.contains("perm_command=npm test"),
        "dispatch_permission_request must carry the borrowed tool args into the payload \
         as the official `.tool_input` (and the tool name as `.tool_name`), got {ctx:?}"
    );

    // No args available at the call site → `.tool_input` is omitted entirely
    // (not `null`), so the fixture's own guard fires and nothing is injected.
    // That is the observable difference between "no args" and "empty args".
    let bare = hooks::dispatch_permission_request(
        Some(sid),
        Some("exec"),
        None,
        "tool: exec npm test",
        None,
    )
    .await;
    assert!(
        bare.merged_additional_context().is_none(),
        "a PermissionRequest with no args must omit `.tool_input` entirely, so \
         permission_tool_input.sh fails its guard and injects nothing, got {:?}",
        bare.merged_additional_context()
    );
}

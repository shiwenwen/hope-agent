//! Project-scope `PreToolUse` end-to-end: a repo's `.hope-agent/hooks.json`
//! must (a) be ignored entirely while `allowProjectScope` is off — the secure
//! default and supply-chain guard — and (b) gate tool calls once the user opts
//! in.
//!
//! Covers the adversarial-review fix where the `PreToolUse` fast-path consulted
//! only the global registry and silently skipped project-only hooks, and the
//! follow-up fix where the live transcript mirror was likewise gated on the
//! global registry alone — leaving project-only hooks reading stale/missing
//! history. Single `#[test]` per binary so the process-global OnceLocks don't
//! race with the other hooks integration tests.
//!
//! Unix-only: the hook shells out to `bash`.
#![cfg(unix)]

use std::path::PathBuf;

use ha_core::hooks::{
    self, CommonHookInput, HookDecision, HookDispatcher, HookEvent, HookInput, PermissionMode,
};

fn pre_tool_use(session_id: &str, cwd: PathBuf) -> HookInput {
    HookInput::PreToolUse {
        common: CommonHookInput {
            session_id: session_id.into(),
            prompt_id: None,
            transcript_path: PathBuf::from("/tmp/proj-scope.jsonl"),
            cwd,
            permission_mode: PermissionMode::Default,
            effort: None,
            hook_event_name: "PreToolUse".into(),
            agent_id: None,
            agent_type: None,
        },
        // Use the internal tool name — the dispatcher passes this verbatim,
        // and `matcher:"Bash"` is normalized to `exec` at compile time.
        tool_name: "exec".into(),
        tool_input: serde_json::json!({ "command": "ls" }),
        tool_use_id: "c1".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_scope_pretooluse_gated_by_opt_in() {
    // Sandbox the data dir so the session DB / config live under a temp root.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HA_DATA_DIR", tmp.path());
    ha_core::paths::ensure_dirs().expect("ensure_dirs");
    ha_core::init_runtime("test");

    // A separate "repository" dir with a checked-in PreToolUse deny hook
    // (`exit 2` == block). Canonicalize so the path matches what
    // `effective_session_working_dir` returns (macOS /var → /private/var).
    let repo = tempfile::tempdir().expect("repo dir");
    let canon = std::fs::canonicalize(repo.path()).expect("canonicalize repo");
    let proj = canon.join(".hope-agent");
    std::fs::create_dir_all(&proj).expect("mkdir .hope-agent");
    std::fs::write(
        proj.join("hooks.json"),
        r#"{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","shell":"bash","command":"exit 2"}]}]}"#,
    )
    .expect("write project hooks.json");

    // A session whose working dir points at that repo.
    let db = ha_core::globals::get_session_db().expect("session db");
    let meta = db.create_session("ha-main").expect("create session");
    let sid = meta.id.clone();
    db.update_session_working_dir(&sid, Some(canon.to_string_lossy().into_owned()))
        .expect("set working dir");

    // (a) Default: project scope is OFF → the repo's hook is ignored entirely,
    // so the call is not blocked.
    hooks::registry::reload_from_config();
    let off =
        HookDispatcher::dispatch(HookEvent::PreToolUse, pre_tool_use(&sid, canon.clone())).await;
    assert!(
        matches!(off.decision, HookDecision::Allow),
        "project scope off → repo hook ignored, got {:?}",
        off.decision
    );

    // (b) Opt in → the project `PreToolUse` hook now fires and blocks the call.
    ha_core::config::mutate_config(("hooks", "test"), |c| {
        c.hooks_allow_project_scope = true;
        Ok(())
    })
    .expect("enable project scope");
    hooks::registry::reload_from_config();
    let on =
        HookDispatcher::dispatch(HookEvent::PreToolUse, pre_tool_use(&sid, canon.clone())).await;
    assert!(
        matches!(
            on.decision,
            HookDecision::Deny { .. } | HookDecision::Block { .. }
        ),
        "project scope on → repo PreToolUse hook blocks, got {:?}",
        on.decision
    );

    // (c) Adversarial review: with ONLY project-scope hooks configured (the
    // global user/managed registry is empty in this test), the live transcript
    // mirror must STILL track new messages. A hook script that reads
    // `transcript.jsonl` would otherwise make security/audit decisions on
    // stale/missing history. Append a message and assert it lands in the
    // session transcript — the pre-fix gate (`registry::global().is_empty()`)
    // skipped the mirror entirely here.
    assert!(
        hooks::registry::global().is_empty(),
        "precondition: this test configures no global hooks"
    );
    db.append_message(
        &sid,
        &ha_core::session::NewMessage::user("hi from a project-only hook session"),
    )
    .expect("append message");
    let transcript = ha_core::paths::session_dir(&sid)
        .expect("session dir")
        .join("transcript.jsonl");
    let body = std::fs::read_to_string(&transcript).unwrap_or_default();
    assert!(
        body.contains("hi from a project-only hook session"),
        "project-only hooks must keep the transcript mirror current; transcript was {body:?}"
    );
}

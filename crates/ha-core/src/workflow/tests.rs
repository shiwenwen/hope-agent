use serde_json::json;
use std::process::Command;
use std::sync::Arc;

use crate::permission::SessionMode;
use crate::session::SessionDB;
use crate::subagent::{SubagentRun, SubagentStatus};

use super::{
    recover_pending_workflow_runs, run_workflow_script,
    runtime::{
        ask_user_tool_args, spawn_agent_tool_args, validation_exit_code, wait_all_tool_args,
    },
    CreateWorkflowRunInput, StartedOpRecoveryAction, UpsertWorkflowOpInput, WorkflowEffectClass,
    WorkflowOpState, WorkflowRunState,
};

fn temp_db() -> (tempfile::TempDir, SessionDB) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = SessionDB::open(&dir.path().join("sessions.db")).expect("open session db");
    (dir, db)
}

fn create_run(db: &SessionDB) -> (String, String) {
    let session = db.create_session("ha-main").expect("create session");
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.feature".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({ "max_runtime_secs": 300, "max_ops": 12 }),
        })
        .expect("create workflow run");
    (session.id, run.id)
}

fn create_run_with_script(db: &SessionDB, script_source: &str) -> (String, String) {
    let session = db.create_session("ha-main").expect("create session");
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: script_source.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 12 }),
        })
        .expect("create workflow run");
    (session.id, run.id)
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn workflow_run_survives_db_reopen_and_lists_by_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let (session_id, run_id) = {
        let db = SessionDB::open(&db_path).expect("open session db");
        create_run(&db)
    };

    let db = SessionDB::open(&db_path).expect("reopen session db");
    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.session_id, session_id);
    assert_eq!(run.state, WorkflowRunState::Draft);

    let listed = db
        .list_workflow_runs_for_session(&session_id, 10)
        .expect("list runs");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, run_id);
}

#[test]
fn workflow_run_rejects_incognito_sessions() {
    let (_dir, db) = temp_db();
    let session = db
        .create_session_with_project("ha-main", None, Some(true))
        .expect("create incognito session");
    assert!(session.incognito);

    let err = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id,
            kind: "coding.review".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({}),
        })
        .expect_err("incognito must be rejected");
    assert!(err.to_string().contains("incognito"));
}

#[test]
fn completed_op_replay_returns_recorded_output_without_regressing_state() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let input = UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#0(fileSearch)".to_string(),
        op_type: "fileSearch".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({ "query": "workflow" }),
        child_handle: None,
    };
    let started = db
        .upsert_workflow_op_started(input.clone())
        .expect("start op");
    assert_eq!(started.state, WorkflowOpState::Started);

    let completed = db
        .complete_workflow_op(&run_id, &input.op_key, json!({ "matches": ["a.rs"] }))
        .expect("complete op");
    assert_eq!(completed.state, WorkflowOpState::Completed);

    let replayed = db
        .upsert_workflow_op_started(input)
        .expect("replay completed op");
    assert_eq!(replayed.state, WorkflowOpState::Completed);
    assert_eq!(replayed.output, Some(json!({ "matches": ["a.rs"] })));
}

#[test]
fn op_input_hash_change_blocks_run_instead_of_overwriting_history() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let op_key = "main/op#0(read)".to_string();
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "read".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({ "path": "a.rs" }),
        child_handle: None,
    })
    .expect("start op");

    let err = db
        .upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run_id.clone(),
            op_key: op_key.clone(),
            op_type: "read".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({ "path": "b.rs" }),
            child_handle: None,
        })
        .expect_err("changed input must fail");
    assert!(err.to_string().contains("input hash changed"));

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("input_hash_mismatch:main/op#0(read)")
    );
}

#[test]
fn started_non_idempotent_recovery_action_blocks_run() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    let op_key = "main/op#1(validate)".to_string();

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "validate".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({ "commands": ["cargo check -p ha-core"] }),
        child_handle: Some("job_123".to_string()),
    })
    .expect("start op");

    let action = db
        .started_op_recovery_action(&run_id, &op_key)
        .expect("recovery action");
    assert_eq!(action, Some(StartedOpRecoveryAction::BlockNonIdempotent));

    let run = db
        .block_run_for_started_non_idempotent_op(&run_id, &op_key)
        .expect("block run");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("started_non_idempotent_op:main/op#1(validate)")
    );
}

#[test]
fn started_spawn_agent_with_child_handle_attaches_and_preserves_original_handle() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    let op_key = "main/op#1(spawnAgent)".to_string();
    let original_handle = uuid::Uuid::new_v4().to_string();
    let replay_handle = uuid::Uuid::new_v4().to_string();

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "Review",
                "agent_id": "ha-review",
                "label": "review"
            },
            "label": "review"
        }),
        child_handle: Some(original_handle.clone()),
    })
    .expect("start spawn op");

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "Review",
                "agent_id": "ha-review",
                "label": "review"
            },
            "label": "review"
        }),
        child_handle: Some(replay_handle),
    })
    .expect("replay spawn op");

    let op = db
        .get_workflow_op(&run_id, &op_key)
        .expect("get op")
        .expect("op exists");
    assert_eq!(op.child_handle.as_deref(), Some(original_handle.as_str()));

    let action = db
        .started_op_recovery_action(&run_id, &op_key)
        .expect("recovery action");
    assert_eq!(
        action,
        Some(StartedOpRecoveryAction::AttachChildHandle(original_handle))
    );
}

#[test]
fn started_spawn_agent_without_child_handle_still_blocks() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    let op_key = "main/op#1(spawnAgent)".to_string();

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "Review",
                "agent_id": "ha-review",
                "label": "review"
            },
            "label": "review"
        }),
        child_handle: None,
    })
    .expect("start spawn op");

    let action = db
        .started_op_recovery_action(&run_id, &op_key)
        .expect("recovery action");
    assert_eq!(action, Some(StartedOpRecoveryAction::BlockNonIdempotent));
}

#[test]
fn cancel_prevents_new_ops() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    db.cancel_workflow_run(&run_id).expect("cancel");

    let err = db
        .upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run_id.clone(),
            op_key: "main/op#0(fileSearch)".to_string(),
            op_type: "fileSearch".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({ "query": "x" }),
            child_handle: None,
        })
        .expect_err("cancelled run must reject op");
    assert!(err.to_string().contains("cancelled"));
}

#[test]
fn recovery_runner_claims_and_replays_completed_ops_without_duplicates() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Recover me" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "recovered" });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let existing_task = db
        .create_task(&session_id, "Recover me", None)
        .expect("create existing task");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#0(task.create)".to_string(),
        op_type: "task.create".to_string(),
        effect_class: WorkflowEffectClass::Idempotent,
        input: json!({ "title": "Recover me" }),
        child_handle: None,
    })
    .expect("start task op");
    db.complete_workflow_op(
        &run_id,
        "main/op#0(task.create)",
        json!({
            "id": existing_task.id,
            "sessionId": session_id,
            "title": existing_task.content,
            "status": existing_task.status,
            "label": null
        }),
    )
    .expect("complete task op");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let report = runtime
        .block_on(recover_pending_workflow_runs(db.clone(), "test-owner"))
        .expect("recover workflows");
    assert_eq!(report.attempted, 1);
    assert_eq!(report.recovered, 1);
    assert!(report.errors.is_empty());

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Completed);
    assert!(run.primary_owner.is_none());

    let tasks = db.list_tasks(&session_id).expect("list tasks");
    assert_eq!(tasks.len(), 1, "recovery replay must not duplicate task");
    assert_eq!(tasks[0].id, existing_task.id);
    assert_eq!(tasks[0].status, "completed");
    assert!(db
        .list_workflow_events(&run_id, 20)
        .expect("list events")
        .iter()
        .any(|event| event.event_type == "run_recovery_claimed"));
}

#[test]
fn recovery_runner_does_not_steal_already_claimed_runs() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    let claimed = db
        .claim_workflow_run_for_recovery(&run_id, "other-owner")
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed.state, WorkflowRunState::Recovering);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let report = runtime
        .block_on(recover_pending_workflow_runs(db.clone(), "test-owner"))
        .expect("recover workflows");
    assert_eq!(report.attempted, 0);
    assert_eq!(report.recovered, 0);

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Recovering);
    assert_eq!(run.primary_owner.as_deref(), Some("other-owner"));
}

#[test]
fn runtime_executes_script_host_apis_and_finishes_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("src")).expect("create workspace");
    std::fs::write(workspace.join("src/workflow_runtime.rs"), "runtime").expect("write file");

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");

    let script = r#"
export default async function main(workflow) {
  const observeTask = await workflow.task.create({
    title: "收集相关文件和约束",
    label: "observe"
  });
  const files = await workflow.fileSearch({
    query: "workflow runtime",
    limit: 5,
    label: "find-runtime"
  });
  await workflow.trace({
    label: "matches",
    payload: files.matches.map((file) => file.relPath)
  });
  await workflow.task.update({ task: observeTask, status: "completed" });
  await workflow.finish({
    summary: "done",
    matchCount: files.matches.length
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
        })
        .expect("create workflow run");
    assert_eq!(run.session_id, session.id);

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output.as_ref().and_then(|v| v.get("summary")),
        Some(&json!("done"))
    );

    let op_types: Vec<&str> = result
        .snapshot
        .ops
        .iter()
        .map(|op| op.op_type.as_str())
        .collect();
    assert_eq!(
        op_types,
        vec![
            "task.create",
            "fileSearch",
            "trace",
            "task.update",
            "finish"
        ]
    );
    assert!(result
        .snapshot
        .events
        .iter()
        .any(|event| event.event_type == "trace"));

    let tasks = db.list_tasks(&session.id).expect("list tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "completed");
}

#[test]
fn runtime_bridges_read_grep_and_generic_tool_through_tool_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("src")).expect("create workspace");
    std::fs::write(
        workspace.join("src/workflow_runtime.rs"),
        "fn main() {\n    println!(\"runtime bridge\");\n}\n",
    )
    .expect("write file");

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");

    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Read and grep" });
  const readOutput = await workflow.read({
    path: "src/workflow_runtime.rs",
    limit: 20
  });
  const grepOutput = await workflow.grep({
    pattern: "runtime bridge",
    path: "src",
    limit: 5
  });
  const rawToolOutput = await workflow.tool({
    name: "read",
    args: { path: "src/workflow_runtime.rs", limit: 20 },
    label: "read-via-tool"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    readOk: readOutput.includes("runtime bridge"),
    grepOk: grepOutput.includes("workflow_runtime.rs"),
    toolOk: rawToolOutput.includes("runtime bridge")
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
        })
        .expect("create workflow run");
    assert_eq!(run.session_id, session.id);

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({ "readOk": true, "grepOk": true, "toolOk": true }))
    );

    let op_types: Vec<&str> = result
        .snapshot
        .ops
        .iter()
        .map(|op| op.op_type.as_str())
        .collect();
    assert_eq!(
        op_types,
        vec![
            "task.create",
            "read",
            "grep",
            "tool:read",
            "task.update",
            "finish"
        ]
    );
}

#[test]
fn runtime_diff_returns_git_snapshot_for_session_workspace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("src")).expect("create workspace");
    git(&workspace, &["init"]);
    git(
        &workspace,
        &["config", "user.email", "hope-agent@example.invalid"],
    );
    git(&workspace, &["config", "user.name", "Hope Agent Test"]);
    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    1\n}\n",
    )
    .expect("write baseline");
    git(&workspace, &["add", "src/lib.rs"]);
    git(&workspace, &["commit", "-m", "initial"]);

    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .expect("modify tracked file");
    std::fs::write(workspace.join("README.md"), "# Hope\n").expect("write untracked file");

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");

    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Snapshot diff" });
  const diff = await workflow.diff({ label: "working-tree" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    kind: diff.kind,
    changes: diff.changes.map((change) => ({
      path: change.path,
      action: change.action,
      linesAdded: change.linesAdded,
      linesRemoved: change.linesRemoved,
      hasAfter: typeof change.after === "string" && change.after.length > 0
    }))
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
        })
        .expect("create workflow run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.expect("workflow output");
    assert_eq!(output.get("kind"), Some(&json!("file_changes")));
    let changes = output
        .get("changes")
        .and_then(|value| value.as_array())
        .expect("changes array");
    let tracked = changes
        .iter()
        .find(|change| {
            change
                .get("path")
                .and_then(|value| value.as_str())
                .is_some_and(|path| path.ends_with("src/lib.rs"))
        })
        .expect("tracked edit");
    assert_eq!(tracked.get("action"), Some(&json!("edit")));
    assert_eq!(tracked.get("hasAfter"), Some(&json!(true)));
    let created = changes
        .iter()
        .find(|change| {
            change
                .get("path")
                .and_then(|value| value.as_str())
                .is_some_and(|path| path.ends_with("README.md"))
        })
        .expect("untracked create");
    assert_eq!(created.get("action"), Some(&json!("create")));

    let op_types: Vec<&str> = result
        .snapshot
        .ops
        .iter()
        .map(|op| op.op_type.as_str())
        .collect();
    assert_eq!(
        op_types,
        vec!["task.create", "diff", "task.update", "finish"]
    );
}

#[test]
fn runtime_validate_runs_targeted_exec_and_returns_structured_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");
    db.update_session_permission_mode(&session.id, SessionMode::Yolo)
        .expect("set yolo mode");

    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Targeted validation" });
  const validation = await workflow.validate({
    commands: ["rustc --version"],
    reason: "workflow validate bridge smoke test"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    ok: validation.ok,
    resultCount: validation.results.length,
    exitCode: validation.results[0].exitCode,
    hasRustc: validation.results[0].output.includes("rustc")
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            loop_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
        })
        .expect("create workflow run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({
            "ok": true,
            "resultCount": 1,
            "exitCode": 0,
            "hasRustc": true
        }))
    );

    let op_types: Vec<&str> = result
        .snapshot
        .ops
        .iter()
        .map(|op| op.op_type.as_str())
        .collect();
    assert_eq!(
        op_types,
        vec!["task.create", "validate", "task.update", "finish"]
    );
}

#[test]
fn runtime_ask_user_fails_closed_on_unattended_surface() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Clarify" });
  await workflow.askUser({
    label: "clarify",
    question: "Continue without a visible user?",
    options: ["Continue", "Stop"]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "unreachable" });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let err = run_workflow_script(db.clone(), &run_id).expect_err("askUser must fail closed");
    assert!(err
        .to_string()
        .contains("workflow.askUser unattended surface"));

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Failed);
    let ops = db.list_workflow_ops(&run_id).expect("list ops");
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].op_type, "task.create");
    assert_eq!(ops[0].state, WorkflowOpState::Completed);
    assert_eq!(ops[1].op_type, "askUser");
    assert_eq!(ops[1].state, WorkflowOpState::Failed);
}

#[test]
fn validation_exit_code_parses_exec_output_markers() {
    assert_eq!(validation_exit_code("hello\n[exit code: 3]"), 3);
    assert_eq!(
        validation_exit_code("Command completed with exit code 7"),
        7
    );
    assert_eq!(validation_exit_code("rustc 1.90.0"), 0);
}

#[test]
fn workflow_subagent_host_args_normalize_agent_and_handles() {
    let spawn = spawn_agent_tool_args(&json!({
        "task": "Review the current diff",
        "agent": "ha-review",
        "label": "review",
        "model": "openai/gpt-5",
        "timeout": 30,
        "files": []
    }))
    .expect("normalize spawn args");
    assert_eq!(
        spawn,
        json!({
            "action": "spawn",
            "task": "Review the current diff",
            "agent_id": "ha-review",
            "label": "review",
            "model": "openai/gpt-5",
            "timeout_secs": 30,
            "files": []
        })
    );

    let wait = wait_all_tool_args(&json!({
        "handles": [
            { "runId": "sar_1" },
            { "run_id": "sar_2" }
        ],
        "waitTimeout": 5
    }))
    .expect("normalize wait args");
    assert_eq!(
        wait,
        json!({
            "action": "wait_all",
            "run_ids": ["sar_1", "sar_2"],
            "wait_timeout": 5
        })
    );
}

#[test]
fn workflow_ask_user_host_args_normalize_question_options() {
    let args = ask_user_tool_args(&json!({
        "label": "choose-path",
        "question": "How should the workflow continue?",
        "context": "Validation failed after the first repair.",
        "questionId": "next_step",
        "header": "Next",
        "options": [
            "Retry",
            {
                "value": "stop",
                "label": "Stop",
                "description": "Return the current findings.",
                "recommended": true,
                "previewKind": "markdown"
            }
        ],
        "defaultValues": ["stop"],
        "timeoutSecs": 60
    }))
    .expect("normalize askUser args");

    assert_eq!(
        args,
        json!({
            "questions": [{
                "question_id": "next_step",
                "text": "How should the workflow continue?",
                "header": "Next",
                "options": [
                    { "value": "Retry", "label": "Retry" },
                    {
                        "value": "stop",
                        "label": "Stop",
                        "description": "Return the current findings.",
                        "recommended": true,
                        "previewKind": "markdown"
                    }
                ],
                "timeout_secs": 60,
                "default_values": ["stop"]
            }],
            "context": "Validation failed after the first repair."
        })
    );
}

#[test]
fn runtime_replays_completed_spawn_agent_and_wait_all_without_rescheduling() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Delegate review" });
  const review = await workflow.spawnAgent({
    task: "Review the current diff",
    agent: "ha-review",
    label: "review"
  });
  const waited = await workflow.waitAll([review], { waitTimeout: 1 });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    runId: review.runId,
    allCompleted: waited.allCompleted,
    firstStatus: waited.runs[0].status
  });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let handle = json!({
        "kind": "subagent",
        "runId": "sar_replayed",
        "run_id": "sar_replayed",
        "status": "spawned",
        "label": "review",
        "task": "Review the current diff",
        "message": "pre-recorded"
    });
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(spawnAgent)".to_string(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "Review the current diff",
                "agent_id": "ha-review",
                "label": "review"
            },
            "label": "review"
        }),
        child_handle: None,
    })
    .expect("start preexisting spawn op");
    db.complete_workflow_op(&run_id, "main/op#1(spawnAgent)", handle.clone())
        .expect("complete preexisting spawn op");

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#2(waitAll)".to_string(),
        op_type: "waitAll".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({
            "handles": [handle],
            "waitTimeout": 1
        }),
        child_handle: None,
    })
    .expect("start preexisting wait op");
    db.complete_workflow_op(
        &run_id,
        "main/op#2(waitAll)",
        json!({
            "all_completed": true,
            "allCompleted": true,
            "runs": [{
                "run_id": "sar_replayed",
                "runId": "sar_replayed",
                "status": "completed",
                "result_preview": "ok"
            }]
        }),
    )
    .expect("complete preexisting wait op");

    let result = run_workflow_script(db.clone(), &run_id).expect("recover workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({
            "runId": "sar_replayed",
            "allCompleted": true,
            "firstStatus": "completed"
        }))
    );

    let mut op_types_by_key: Vec<(&str, &str)> = result
        .snapshot
        .ops
        .iter()
        .map(|op| (op.op_key.as_str(), op.op_type.as_str()))
        .collect();
    op_types_by_key.sort_by(|left, right| left.0.cmp(right.0));
    let op_types: Vec<&str> = op_types_by_key
        .iter()
        .map(|(_op_key, op_type)| *op_type)
        .collect();
    assert_eq!(
        op_types,
        vec![
            "task.create",
            "spawnAgent",
            "waitAll",
            "task.update",
            "finish"
        ]
    );
}

#[test]
fn runtime_attaches_started_spawn_agent_child_handle_without_rescheduling() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Delegate review" });
  const review = await workflow.spawnAgent({
    task: "Review",
    agent: "ha-review",
    label: "review"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    runId: review.runId,
    status: review.status,
    label: review.label
  });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    let child_handle = uuid::Uuid::new_v4().to_string();

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(spawnAgent)".to_string(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "Review",
                "agent_id": "ha-review",
                "label": "review"
            },
            "label": "review"
        }),
        child_handle: Some(child_handle.clone()),
    })
    .expect("start preexisting spawn op");
    db.insert_subagent_run(&SubagentRun {
        run_id: child_handle.clone(),
        parent_session_id: session_id.clone(),
        parent_agent_id: "ha-main".to_string(),
        child_agent_id: "ha-review".to_string(),
        child_session_id: "child-session".to_string(),
        task: "Review".to_string(),
        status: SubagentStatus::Running,
        result: None,
        error: None,
        depth: 1,
        model_used: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        duration_ms: None,
        label: Some("review".to_string()),
        attachment_count: 0,
        input_tokens: None,
        output_tokens: None,
    })
    .expect("insert subagent run");

    let result = run_workflow_script(db.clone(), &run_id).expect("recover workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({
            "runId": child_handle,
            "status": "running",
            "label": "review"
        }))
    );

    let spawn_op = result
        .snapshot
        .ops
        .iter()
        .find(|op| op.op_key == "main/op#1(spawnAgent)")
        .expect("spawn op");
    assert_eq!(spawn_op.state, WorkflowOpState::Completed);
}

#[test]
fn runtime_replays_completed_task_create_without_duplicate_task() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const observeTask = await workflow.task.create({ title: "Observe" });
  await workflow.task.update({ task: observeTask, status: "completed" });
  await workflow.finish({ summary: "recovered" });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let existing_task = db
        .create_task(&session_id, "Observe", None)
        .expect("create existing task");
    let op_key = "main/op#0(task.create)";
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.to_string(),
        op_type: "task.create".to_string(),
        effect_class: WorkflowEffectClass::Idempotent,
        input: json!({ "title": "Observe" }),
        child_handle: None,
    })
    .expect("start preexisting op");
    db.complete_workflow_op(
        &run_id,
        op_key,
        json!({
            "id": existing_task.id,
            "sessionId": session_id,
            "title": existing_task.content,
            "status": existing_task.status,
            "label": null
        }),
    )
    .expect("complete preexisting op");

    let result = run_workflow_script(db.clone(), &run_id).expect("recover workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);

    let tasks = db.list_tasks(&session_id).expect("list tasks");
    assert_eq!(tasks.len(), 1, "task.create replay must not duplicate task");
    assert_eq!(tasks[0].id, existing_task.id);
    assert_eq!(tasks[0].status, "completed");
}

#[test]
fn runtime_blocks_started_non_idempotent_tool_on_replay() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Observe" });
  await workflow.tool({ name: "exec", args: { cmd: "echo should_not_run" } });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "unreachable" });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let existing_task = db
        .create_task(&session_id, "Observe", None)
        .expect("create existing task");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#0(task.create)".to_string(),
        op_type: "task.create".to_string(),
        effect_class: WorkflowEffectClass::Idempotent,
        input: json!({ "title": "Observe" }),
        child_handle: None,
    })
    .expect("start task op");
    db.complete_workflow_op(
        &run_id,
        "main/op#0(task.create)",
        json!({
            "id": existing_task.id,
            "sessionId": session_id,
            "title": existing_task.content,
            "status": existing_task.status,
            "label": null
        }),
    )
    .expect("complete task op");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(tool:exec)".to_string(),
        op_type: "tool:exec".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "name": "exec",
            "args": { "cmd": "echo should_not_run" },
            "label": null
        }),
        child_handle: None,
    })
    .expect("start non-idempotent op");

    let err = run_workflow_script(db.clone(), &run_id).expect_err("started exec must block");
    assert!(err
        .to_string()
        .contains("previously-started non-idempotent op"));

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("started_non_idempotent_op:main/op#1(tool:exec)")
    );
}

#[test]
fn runtime_rejects_bad_script_before_starting_ops() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  await workflow.task.update({ label: "observe", status: "completed" });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let err = run_workflow_script(db.clone(), &run_id).expect_err("gate must reject script");
    assert!(err.to_string().contains("Workflow Script Gate"));
    assert!(err.to_string().contains("task_update_by_label"));

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Draft);
    assert!(db.list_workflow_ops(&run_id).expect("list ops").is_empty());
}

#[test]
fn runtime_determinism_guard_blocks_dynamic_random_access() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const observeTask = await workflow.task.create({ title: "Observe" });
  globalThis["Math"]["random"]();
  await workflow.task.update({ task: observeTask, status: "completed" });
  await workflow.finish({ summary: "unreachable" });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let err = run_workflow_script(db.clone(), &run_id).expect_err("runtime guard must reject");
    assert!(err.to_string().contains("Math.random() is disabled"));

    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Failed);

    let ops = db.list_workflow_ops(&run_id).expect("list ops");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "task.create");
    assert_eq!(ops[0].state, WorkflowOpState::Completed);
}

use serde_json::json;
use std::sync::Arc;

use crate::session::SessionDB;

use super::{
    run_workflow_script, CreateWorkflowRunInput, StartedOpRecoveryAction, UpsertWorkflowOpInput,
    WorkflowEffectClass, WorkflowOpState, WorkflowRunState,
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

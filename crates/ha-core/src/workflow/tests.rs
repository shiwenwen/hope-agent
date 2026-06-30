use serde_json::json;

use crate::session::SessionDB;

use super::{
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

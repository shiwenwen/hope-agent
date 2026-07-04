use rusqlite::params;
use serde_json::{json, Value};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::async_jobs::{BackgroundJob, JobKind, JobOrigin, JobStatus, JobsDB};
use crate::channel::ChannelDB;
use crate::goal::CreateGoalInput;
use crate::permission::SessionMode;
use crate::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};
use crate::session::SessionDB;
use crate::subagent::{SubagentRun, SubagentStatus};

use super::{
    cancel_workflow_run_with_children, ensure_workflow_script_can_create,
    preview_workflow_script_for_session, recover_pending_workflow_runs, run_workflow_script,
    run_workflow_script_async,
    runtime::{
        ask_user_tool_args, spawn_agent_tool_args, validation_exit_code, wait_all_tool_args,
    },
    spawn_workflow_run_if_primary, CreateWorkflowRunInput, StartedOpRecoveryAction,
    UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowOpState, WorkflowRunState,
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
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({ "max_runtime_secs": 300, "max_ops": 12 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
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
            execution_mode: "guarded".to_string(),
            script_source: script_source.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 12 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    (session.id, run.id)
}

fn ensure_async_jobs_db() {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("async jobs tempdir"));
    let db = JobsDB::open(&dir.path().join("background_jobs.db")).expect("open async jobs db");
    crate::async_jobs::set_async_jobs_db(Arc::new(db));
}

fn async_jobs_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn workflow_spawn_global_env() -> (&'static tempfile::TempDir, Arc<SessionDB>) {
    static ENV: OnceLock<(tempfile::TempDir, Arc<SessionDB>)> = OnceLock::new();
    let env = ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("workflow spawn data dir");
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            write_workflow_spawn_agent("ha-main", |cfg| {
                cfg.subagents.max_concurrent = 1;
            });
            write_workflow_spawn_agent("ha-review", |_| {});
        });

        let db = if let Some(existing) = crate::get_session_db() {
            existing.clone()
        } else {
            let db = Arc::new(
                SessionDB::open(&root.path().join("workflow-spawn-sessions.db"))
                    .expect("open workflow spawn session db"),
            );
            let _ = crate::SESSION_DB.set(db.clone());
            db
        };
        let _ =
            crate::SUBAGENT_CANCELS.set(Arc::new(crate::subagent::SubagentCancelRegistry::new()));
        ensure_async_jobs_db();
        (root, db)
    });
    (&env.0, env.1.clone())
}

fn write_workflow_spawn_agent(
    id: &str,
    configure: impl FnOnce(&mut crate::agent_config::AgentConfig),
) {
    let dir = crate::paths::agent_dir(id).expect("agent dir");
    std::fs::create_dir_all(&dir).expect("create agent dir");
    let mut cfg = crate::agent_config::AgentConfig::default();
    configure(&mut cfg);
    std::fs::write(
        dir.join("agent.json"),
        serde_json::to_string(&cfg).expect("serialize agent config"),
    )
    .expect("write agent config");
}

fn phase2_mock_model_config(id: &str) -> ModelConfig {
    ModelConfig {
        id: id.to_string(),
        name: id.to_string(),
        input_types: vec!["text".to_string()],
        context_window: 128_000,
        max_tokens: 8192,
        reasoning: false,
        thinking_style: None,
        cost_input: 0.0,
        cost_output: 0.0,
    }
}

fn phase2_openai_chat_sse(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({
            "choices": [{
                "delta": { "content": text }
            }]
        }),
        json!({
            "choices": [{
                "delta": {}
            }],
            "usage": {
                "prompt_tokens": 7,
                "completion_tokens": 5
            }
        })
    )
}

struct ConfigCacheRestore(crate::config::AppConfig);

impl Drop for ConfigCacheRestore {
    fn drop(&mut self) {
        crate::config::replace_cache_for_test(self.0.clone());
    }
}

fn replace_config_cache_for_test(config: crate::config::AppConfig) -> ConfigCacheRestore {
    let previous = (*crate::config::cached_config()).clone();
    crate::config::replace_cache_for_test(config);
    ConfigCacheRestore(previous)
}

fn insert_completed_async_job(job_id: &str, session_id: &str, output: &str) {
    ensure_async_jobs_db();
    let db = crate::async_jobs::get_async_jobs_db().expect("async jobs db initialized");
    insert_async_job_row(
        &db,
        job_id,
        session_id,
        JobStatus::Completed,
        Some(output.to_string()),
        None,
        true,
    );
}

fn insert_running_async_job(job_id: &str, session_id: &str) {
    ensure_async_jobs_db();
    let db = crate::async_jobs::get_async_jobs_db().expect("async jobs db initialized");
    insert_async_job_row(
        &db,
        job_id,
        session_id,
        JobStatus::Running,
        None,
        None,
        true,
    );
}

fn insert_async_job_row(
    db: &JobsDB,
    job_id: &str,
    session_id: &str,
    status: JobStatus,
    result_preview: Option<String>,
    error: Option<String>,
    injected: bool,
) {
    let now = chrono::Utc::now().timestamp();
    let job = BackgroundJob {
        job_id: job_id.to_string(),
        kind: JobKind::Tool,
        subagent_run_id: None,
        group_id: None,
        session_id: Some(session_id.to_string()),
        agent_id: Some("ha-main".to_string()),
        tool_name: crate::tools::TOOL_EXEC.to_string(),
        tool_call_id: None,
        args_json: "{}".to_string(),
        status,
        result_preview,
        result_path: None,
        error,
        created_at: now,
        completed_at: status.is_terminal().then_some(now),
        injected,
        origin: JobOrigin::Explicit.as_str().to_string(),
        approval_origin: None,
        incognito: false,
        pid: None,
        cancel_requested: false,
    };
    db.insert(&job).expect("insert completed async job");
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
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({}),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect_err("incognito must be rejected");
    assert!(err.to_string().contains("incognito"));
}

#[test]
fn create_workflow_run_links_empty_managed_worktree_reverse_binding() {
    let (dir, db) = temp_db();
    let session = db.create_session("ha-main").expect("create session");
    let worktree_id = "mwt_reverse_link";
    let now = chrono::Utc::now().to_rfc3339();
    let repo_root = dir.path().join("repo");
    let worktree_path = dir.path().join("workflow-worktree");
    std::fs::create_dir_all(&repo_root).expect("repo dir");
    std::fs::create_dir_all(&worktree_path).expect("worktree dir");
    let repo_root = repo_root.to_string_lossy().to_string();
    let worktree_path = worktree_path.to_string_lossy().to_string();

    {
        let conn = db.conn.lock().expect("lock session db");
        conn.execute(
            "INSERT INTO managed_worktrees (
                id, session_id, child_session_id, workflow_run_id, purpose, state, label,
                repo_root, source_working_dir, path, base_ref, base_branch, base_sha,
                git_branch, dirty_snapshot_json, created_at, updated_at,
                archived_at, restored_at, handed_off_at
             ) VALUES (
                ?1, ?2, NULL, NULL, 'workflow', 'active', 'Workflow worktree',
                ?3, ?3, ?4, 'HEAD', NULL, NULL,
                NULL, NULL, ?5, ?5,
                NULL, NULL, NULL
             )",
            params![worktree_id, session.id, repo_root, worktree_path, now,],
        )
        .expect("insert managed worktree");
    }

    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "general.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: "export default async function main(workflow) {}".to_string(),
            budget: json!({ "max_runtime_secs": 300, "max_ops": 12 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: Some(worktree_id.to_string()),
        })
        .expect("create workflow run");

    let worktree = db
        .get_managed_worktree(worktree_id)
        .expect("get managed worktree")
        .expect("worktree exists");
    assert_eq!(worktree.workflow_run_id.as_deref(), Some(run.id.as_str()));
}

#[test]
fn workflow_create_records_permission_preview_event() {
    let (_dir, db) = temp_db();
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 6 };
  const task = await workflow.task.create({ title: "Inspect" });
  const files = await workflow.fileSearch({ query: "workflow", limit: 3 });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ files, budget });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    let preview = events
        .iter()
        .find(|event| event.event_type == "script_permission_preview")
        .expect("preview event");
    assert_eq!(preview.payload["summary"]["total"], json!(3));
    assert_eq!(preview.payload["summary"]["ask"], json!(0));
}

#[test]
fn workflow_create_records_parent_repair_derivation() {
    let (_dir, db) = temp_db();
    let parent_script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 6 };
  const task = await workflow.task.create({ title: "Parent" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;
    let (session_id, parent_run_id) = create_run_with_script(&db, parent_script);
    let child_script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 6 };
  const task = await workflow.task.create({ title: "Repair" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;
    let child = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session_id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: child_script.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 12 }),
            parent_run_id: Some(parent_run_id.clone()),
            origin: Some("repair".to_string()),
            goal_id: None,
            worktree_id: None,
        })
        .expect("create child workflow run");

    assert_eq!(child.parent_run_id.as_deref(), Some(parent_run_id.as_str()));
    assert_eq!(child.origin.as_deref(), Some("repair"));

    let child_events = db
        .list_workflow_events(&child.id, 20)
        .expect("list child events");
    assert!(child_events.iter().any(|event| {
        event.event_type == "run_derived_from"
            && event.payload.get("parentRunId").and_then(Value::as_str)
                == Some(parent_run_id.as_str())
            && event.payload.get("origin").and_then(Value::as_str) == Some("repair")
    }));

    let parent_events = db
        .list_workflow_events(&parent_run_id, 20)
        .expect("list parent events");
    assert!(parent_events.iter().any(|event| {
        event.event_type == "run_derived_child_created"
            && event.payload.get("childRunId").and_then(Value::as_str) == Some(child.id.as_str())
            && event.payload.get("origin").and_then(Value::as_str) == Some("repair")
    }));
}

#[test]
fn workflow_create_preflight_rejects_gate_failure() {
    let (_dir, db) = temp_db();
    let session = db.create_session("ha-main").expect("create session");
    let script = r#"
export default async function main(workflow) {
  await workflow.task.update({ label: "observe", status: "completed" });
}
"#;

    let err = ensure_workflow_script_can_create(&db, &session.id, script, Some("guarded"))
        .expect_err("gate failure must block owner create");
    assert!(err.to_string().contains("Workflow Script Gate"));
    assert!(err.to_string().contains("task_update_by_label"));
}

#[test]
fn workflow_create_preflight_allows_approval_required_script() {
    let (_dir, db) = temp_db();
    let session = db.create_session("ha-main").expect("create session");
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 8 };
  const task = await workflow.task.create({ title: "Write" });
  const call = {
    name: "write",
    args: { path: "a.txt", content: "hello" },
    label: "write-file"
  };
  await workflow.tool(call);
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;

    let preview = ensure_workflow_script_can_create(&db, &session.id, script, Some("guarded"))
        .expect("approval-required scripts can be created");
    assert!(preview.can_create);
    assert!(preview.requires_approval);
    assert!(!preview.has_denials);
}

#[test]
fn workflow_create_preflight_denies_unattended_ask_user_by_default() {
    let (_dir, db) = temp_db();
    let session = db.create_session("ha-main").expect("create session");
    let mut config = crate::config::AppConfig::default();
    config.permission.unattended_approval_action =
        crate::permission::UnattendedApprovalAction::Deny;
    let _config_restore = replace_config_cache_for_test(config);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 8 };
  const task = await workflow.task.create({ title: "Clarify" });
  await workflow.askUser({
    label: "clarify",
    question: "Continue without a visible user?",
    options: ["Continue", "Stop"]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;

    let preview = preview_workflow_script_for_session(&db, &session.id, script, Some("guarded"));
    assert!(preview.gate_passed);
    assert!(preview.has_denials);
    assert!(!preview.can_create);
    assert_eq!(preview.permission.summary.deny, 1);
    let ask = preview
        .permission
        .calls
        .iter()
        .find(|call| call.api == "workflow.askUser")
        .expect("askUser preview call");
    assert_eq!(ask.decision, "deny");
    assert!(ask
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("unattendedApprovalAction=deny"));

    let err = ensure_workflow_script_can_create(&db, &session.id, script, Some("guarded"))
        .expect_err("unattended askUser deny must block owner create");
    assert!(err.to_string().contains("permission preview denied"));
}

#[test]
fn workflow_create_preflight_allows_unattended_ask_user_when_policy_proceeds() {
    let (_dir, db) = temp_db();
    let session = db.create_session("ha-main").expect("create session");
    let mut config = crate::config::AppConfig::default();
    config.permission.unattended_approval_action =
        crate::permission::UnattendedApprovalAction::Proceed;
    let _config_restore = replace_config_cache_for_test(config);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 8 };
  const task = await workflow.task.create({ title: "Clarify" });
  await workflow.askUser(workflow.trace({ label: "dynamic-question" }));
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;

    let preview = ensure_workflow_script_can_create(&db, &session.id, script, Some("guarded"))
        .expect("proceed policy allows unattended askUser create");
    assert!(preview.can_create);
    assert!(!preview.has_denials);
    let ask = preview
        .permission
        .calls
        .iter()
        .find(|call| call.api == "workflow.askUser")
        .expect("askUser preview call");
    assert_eq!(ask.decision, "allow");
    assert!(ask.args.is_none(), "dynamic askUser args are not static");
    assert!(ask
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("unattendedApprovalAction=proceed"));
}

#[test]
fn draft_workflow_requires_approval_before_dynamic_tool_call() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 8 };
  const task = await workflow.task.create({ title: "Write" });
  const call = {
    name: "write",
    args: { path: "a.txt", content: "hello" },
    label: "write-file"
  };
  await workflow.tool(call);
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let err = run_workflow_script(db.clone(), &run_id).expect_err("preview asks first");
    assert!(
        err.to_string().contains("requires user approval"),
        "{err:#}"
    );
    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::AwaitingApproval);

    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    assert!(events
        .iter()
        .any(|event| event.event_type == "script_permission_approval_required"));

    let approved = db.approve_workflow_run(&run_id).expect("approve workflow");
    assert_eq!(approved.state, WorkflowRunState::Running);
}

#[test]
fn phase2_eval_user_approval_pause_resume_cancel_flow() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 8 };
  const task = await workflow.task.create({ title: "Approval control flow" });
  const call = {
    name: "write",
    args: { path: "phase2.txt", content: "approval flow" },
    label: "write-file"
  };
  await workflow.tool(call);
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true, budget });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);

    let err = run_workflow_script(db.clone(), &run_id).expect_err("preview asks first");
    assert!(
        err.to_string().contains("requires user approval"),
        "{err:#}"
    );
    let awaiting = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(awaiting.state, WorkflowRunState::AwaitingApproval);

    let approved = db.approve_workflow_run(&run_id).expect("approve workflow");
    assert_eq!(approved.state, WorkflowRunState::Running);
    let paused = db.pause_workflow_run(&run_id).expect("pause workflow");
    assert_eq!(paused.state, WorkflowRunState::Paused);
    let resumed = db.resume_workflow_run(&run_id).expect("resume workflow");
    assert_eq!(resumed.state, WorkflowRunState::Running);
    let cancelled = db.cancel_workflow_run(&run_id).expect("cancel workflow");
    assert_eq!(cancelled.state, WorkflowRunState::Cancelled);

    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    for reason in [
        "permission_preview",
        "approval_granted",
        "pause_requested",
        "resume_requested",
        "cancel_requested",
    ] {
        assert!(
            events.iter().any(|event| {
                event.event_type == "run_state_changed"
                    && event.payload.get("reason").and_then(Value::as_str) == Some(reason)
            }),
            "missing state transition reason {reason}"
        );
    }
    for (action, state) in [
        ("approve", "running"),
        ("pause", "paused"),
        ("resume", "running"),
        ("cancel", "cancelled"),
    ] {
        assert!(
            events.iter().any(|event| {
                event.event_type == "run_control_action"
                    && event.payload.get("action").and_then(Value::as_str) == Some(action)
                    && event.payload.get("resultState").and_then(Value::as_str) == Some(state)
                    && event.payload.get("accepted").and_then(Value::as_bool) == Some(true)
            }),
            "missing control action {action}"
        );
    }
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
    let op_key = "main/op#1(customSideEffect)".to_string();

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: op_key.clone(),
        op_type: "customSideEffect".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({ "sideEffect": true }),
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
        Some("started_non_idempotent_op:main/op#1(customSideEffect)")
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
fn owner_cancel_cancels_workflow_child_async_jobs() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let (session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    insert_running_async_job(&job_id, &session_id);
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#0(tool:exec)".to_string(),
        op_type: "tool:exec".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "name": "exec",
            "args": {
                "command": "sleep 999",
                "run_in_background": true
            },
            "label": null
        }),
        child_handle: Some(job_id.clone()),
    })
    .expect("start async child op");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let run = runtime
        .block_on(cancel_workflow_run_with_children(db.clone(), &run_id))
        .expect("cancel workflow with children");
    assert_eq!(run.state, WorkflowRunState::Cancelled);

    let job = crate::async_jobs::JobManager::get(&job_id)
        .expect("get job")
        .expect("job exists");
    assert_eq!(job.status, JobStatus::Cancelled);
    assert!(
        job.injected,
        "cancelled workflow child jobs must not inject"
    );

    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    assert!(events.iter().any(|event| {
        event.event_type == "run_child_cancel_requested"
            && event
                .payload
                .get("children")
                .and_then(Value::as_array)
                .is_some_and(|children| {
                    children.iter().any(|child| {
                        child.get("id").and_then(Value::as_str) == Some(job_id.as_str())
                            && child.get("kind").and_then(Value::as_str) == Some("async_job")
                    })
                })
    }));
}

#[test]
fn pause_prevents_new_ops() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    db.pause_workflow_run(&run_id).expect("pause");

    let err = db
        .upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: run_id.clone(),
            op_key: "main/op#0(fileSearch)".to_string(),
            op_type: "fileSearch".to_string(),
            effect_class: WorkflowEffectClass::Pure,
            input: json!({ "query": "x" }),
            child_handle: None,
        })
        .expect_err("paused run must reject op");
    let message = err.to_string();
    assert!(message.contains("paused"), "{message}");
}

#[test]
fn pause_clears_owner_so_resume_can_be_reclaimed() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
    db.claim_workflow_run_for_recovery(&run_id, "old-owner")
        .expect("claim")
        .expect("claimed");
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("runtime_start"))
        .expect("running after claim");

    let paused = db.pause_workflow_run(&run_id).expect("pause");
    assert_eq!(paused.state, WorkflowRunState::Paused);
    assert!(paused.primary_owner.is_none());

    let resumed = db.resume_workflow_run(&run_id).expect("resume");
    assert_eq!(resumed.state, WorkflowRunState::Running);
    assert!(resumed.primary_owner.is_none());

    let claimed = db
        .claim_workflow_run_for_recovery(&run_id, "new-owner")
        .expect("reclaim after resume")
        .expect("resumed run should be claimable");
    assert_eq!(claimed.state, WorkflowRunState::Recovering);
    assert_eq!(claimed.primary_owner.as_deref(), Some("new-owner"));
}

#[test]
fn launch_claim_sets_draft_owner_and_blocks_duplicate_launch() {
    let (_dir, db) = temp_db();
    let (_session_id, run_id) = create_run(&db);
    let owner = format!("launch:pid:{}", std::process::id());
    let duplicate_owner = format!("duplicate:pid:{}", std::process::id());

    let claimed = db
        .claim_workflow_run_for_launch(&run_id, &owner)
        .expect("claim draft for launch")
        .expect("draft run should be launch-claimable");
    assert_eq!(claimed.state, WorkflowRunState::Draft);
    assert_eq!(claimed.primary_owner.as_deref(), Some(owner.as_str()));

    let duplicate = db
        .claim_workflow_run_for_launch(&run_id, &duplicate_owner)
        .expect("duplicate launch claim should be handled");
    assert!(
        duplicate.is_none(),
        "alive launch owner must prevent duplicate runtime launch"
    );

    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    assert!(events.iter().any(|event| {
        event.event_type == "run_launch_claimed"
            && event.payload.get("fromState").and_then(Value::as_str) == Some("draft")
            && event.payload.get("toState").and_then(Value::as_str) == Some("draft")
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_workflow_run_records_runtime_launch_request() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let (_session_id, run_id) = create_run_with_script(
        &db,
        "export default async function main(workflow) { await workflow.finish({ summary: 'done' }); }",
    );
    let accepted = spawn_workflow_run_if_primary(db.clone(), run_id.clone(), "test:launch");

    assert_eq!(accepted, crate::runtime_lock::is_primary());
    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    assert!(events.iter().any(|event| {
        event.event_type == "run_runtime_launch"
            && event.payload.get("owner").and_then(Value::as_str) == Some("test:launch")
            && event.payload.get("accepted").and_then(Value::as_bool) == Some(accepted)
            && event.payload.get("reason").and_then(Value::as_str)
                == Some(if accepted {
                    "primary_spawn_accepted"
                } else {
                    "not_primary"
                })
    }));
    if accepted {
        for _ in 0..20 {
            let events = db
                .list_workflow_events(&run_id, 20)
                .expect("list workflow events");
            if events
                .iter()
                .any(|event| event.event_type == "run_runtime_result")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
    let events = db
        .list_workflow_events(&run_id, 20)
        .expect("list workflow events");
    assert!(events.iter().any(|event| {
        event.event_type == "run_runtime_result"
            && event.payload.get("owner").and_then(Value::as_str) == Some("test:launch")
            && event.payload.get("accepted").and_then(Value::as_bool) == Some(accepted)
            && event.payload.get("status").and_then(Value::as_str)
                == Some(if accepted { "finished" } else { "rejected" })
    }));
}

#[test]
fn permission_preview_clears_launch_owner_before_approval_resume() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Write" });
  const call = {
    name: "write",
    args: { path: "a.txt", content: "hello" },
    label: "write-file"
  };
  await workflow.tool(call);
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ ok: true });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);
    let owner = format!("launch:pid:{}", std::process::id());
    db.claim_workflow_run_for_launch(&run_id, &owner)
        .expect("claim draft for launch")
        .expect("draft run should be launch-claimable");

    let err = run_workflow_script(db.clone(), &run_id).expect_err("preview asks first");
    assert!(
        err.to_string().contains("requires user approval"),
        "{err:#}"
    );
    let awaiting = db
        .get_workflow_run(&run_id)
        .expect("get awaiting run")
        .expect("run exists");
    assert_eq!(awaiting.state, WorkflowRunState::AwaitingApproval);
    assert!(
        awaiting.primary_owner.is_none(),
        "awaiting approval must release runtime owner so approval can resume"
    );

    let approved = db.approve_workflow_run(&run_id).expect("approve workflow");
    assert_eq!(approved.state, WorkflowRunState::Running);
    assert!(approved.primary_owner.is_none());
    let resume_owner = format!("resume:pid:{}", std::process::id());
    let claimed = db
        .claim_workflow_run_for_launch(&run_id, &resume_owner)
        .expect("claim approved run for launch")
        .expect("approved run should be launch-claimable");
    assert_eq!(claimed.state, WorkflowRunState::Recovering);
    assert_eq!(
        claimed.primary_owner.as_deref(),
        Some(resume_owner.as_str())
    );
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
fn startup_like_recovery_replays_workflow_after_async_jobs_mark_interrupted() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Recover validation" });
  const validation = await workflow.validate({ commands: ["echo never-finishes"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    ok: validation.ok,
    jobStatus: validation.results[0].jobStatus,
    output: validation.results[0].output
  });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let existing_task = db
        .create_task(&session_id, "Recover validation", None)
        .expect("create existing task");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#0(task.create)".to_string(),
        op_type: "task.create".to_string(),
        effect_class: WorkflowEffectClass::Idempotent,
        input: json!({ "title": "Recover validation" }),
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

    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    insert_running_async_job(&job_id, &session_id);
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(validate)".to_string(),
        op_type: "validate".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({ "commands": ["echo never-finishes"] }),
        child_handle: Some(
            json!({
                "kind": "validate",
                "jobs": [{
                    "jobId": job_id.clone(),
                    "command": "echo never-finishes",
                    "cwd": null,
                    "timeout": null
                }]
            })
            .to_string(),
        ),
    })
    .expect("start validate op");

    crate::async_jobs::JobManager::replay_pending();
    let job = crate::async_jobs::JobManager::get(&job_id)
        .expect("load async job")
        .expect("async job exists");
    assert_eq!(job.status, JobStatus::Interrupted);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let report = runtime
        .block_on(recover_pending_workflow_runs(
            db.clone(),
            "startup-test-owner",
        ))
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

    let output = db
        .get_workflow_op(&run_id, "main/op#3(finish)")
        .expect("get finish op")
        .expect("finish op exists")
        .output
        .expect("workflow output");
    assert_eq!(
        output,
        json!({
            "ok": false,
            "jobStatus": "interrupted",
            "output": "interrupted by application restart"
        })
    );

    let tasks = db.list_tasks(&session_id).expect("list tasks");
    assert_eq!(tasks.len(), 1, "startup replay must not duplicate task");
    assert_eq!(tasks[0].id, existing_task.id);
    assert_eq!(tasks[0].status, "completed");
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
fn recovery_runner_reclaims_stale_pid_owned_runs() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Recover stale owner" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "recovered stale owner" });
}
"#;
    let (_recovering_session_id, recovering_run_id) = create_run_with_script(&db, script);
    let (_running_session_id, running_run_id) = create_run_with_script(&db, script);
    let stale_owner = format!("startup:pid:{}", u32::MAX);

    db.transition_workflow_run(&recovering_run_id, WorkflowRunState::Running, Some("test"))
        .expect("recovering run enters running");
    db.claim_workflow_run_for_recovery(&recovering_run_id, &stale_owner)
        .expect("claim recovering run")
        .expect("claimed recovering run");

    db.transition_workflow_run(&running_run_id, WorkflowRunState::Running, Some("test"))
        .expect("running run enters running");
    db.claim_workflow_run_for_recovery(&running_run_id, &stale_owner)
        .expect("claim running run")
        .expect("claimed running run");
    db.transition_workflow_run(
        &running_run_id,
        WorkflowRunState::Running,
        Some("runtime_start"),
    )
    .expect("simulated crash after recovery transitioned back to running");

    let recoverable_ids: Vec<String> = db
        .list_recoverable_workflow_runs()
        .expect("list recoverable runs")
        .into_iter()
        .map(|run| run.id)
        .collect();
    assert!(recoverable_ids.contains(&recovering_run_id));
    assert!(recoverable_ids.contains(&running_run_id));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let report = runtime
        .block_on(recover_pending_workflow_runs(db.clone(), "new-owner"))
        .expect("recover stale-owner workflows");
    assert_eq!(report.attempted, 2);
    assert_eq!(report.recovered, 2);
    assert_eq!(report.blocked, 0);
    assert_eq!(report.failed, 0);
    assert!(report.errors.is_empty());

    for run_id in [recovering_run_id, running_run_id] {
        let run = db
            .get_workflow_run(&run_id)
            .expect("get recovered run")
            .expect("run exists");
        assert_eq!(run.state, WorkflowRunState::Completed);
        assert!(run.primary_owner.is_none());
    }
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
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
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
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
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
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
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
fn runtime_review_and_verify_create_durable_control_plane_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    ChannelDB::new(db.clone())
        .migrate()
        .expect("migrate channel db");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("crates/ha-eval/src")).expect("create workspace");
    git(&workspace, &["init"]);
    git(
        &workspace,
        &["config", "user.email", "hope-agent@example.invalid"],
    );
    git(&workspace, &["config", "user.name", "Hope Agent Test"]);
    git(&workspace, &["config", "commit.gpgsign", "false"]);
    std::fs::write(
        workspace.join("crates/ha-eval/Cargo.toml"),
        "[package]\nname = \"ha-eval\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write manifest");
    std::fs::write(
        workspace.join("crates/ha-eval/src/lib.rs"),
        "pub fn answer() -> i32 {\n    41\n}\n",
    )
    .expect("write baseline");
    git(&workspace, &["add", "."]);
    git(&workspace, &["commit", "-m", "initial"]);

    std::fs::write(
        workspace.join("crates/ha-eval/src/lib.rs"),
        "pub fn answer() -> i32 {\n    println!(\"debug answer\");\n    42\n}\n",
    )
    .expect("modify source");

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");
    let goal = db
        .create_goal(CreateGoalInput {
            session_id: session.id.clone(),
            objective: "Ship answer fix".to_string(),
            completion_criteria: "Review and verification plan are recorded.".to_string(),
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })
        .expect("create goal");

    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 300, max_ops: 12 };
  const task = await workflow.task.create({ title: "Review and verify" });
  const review = await workflow.review({
    focusPaths: ["crates/ha-eval/src/lib.rs"],
    label: "focused-review"
  });
  const verification = await workflow.verify({
    focusPaths: ["crates/ha-eval/src/lib.rs"],
    maxCommands: 2,
    label: "focused-verify"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    reviewOk: review.ok,
    findingCount: review.findingCount,
    commandCount: verification.commandCount,
    commands: verification.commands.map((command) => command.command),
    reviewRunId: review.runId,
    verificationRunId: verification.runId,
    budget
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 12 }),
            parent_run_id: None,
            origin: None,
            goal_id: Some(goal.goal.id.clone()),
            worktree_id: None,
        })
        .expect("create workflow run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.as_ref().expect("workflow output");
    assert_eq!(output.get("reviewOk"), Some(&json!(true)));
    assert_eq!(
        output
            .get("commands")
            .and_then(Value::as_array)
            .and_then(|commands| commands.first())
            .and_then(Value::as_str),
        Some("cargo check -p ha-eval --locked"),
        "workflow output: {}",
        output
    );
    assert!(output
        .get("findingCount")
        .and_then(Value::as_u64)
        .is_some_and(|count| count >= 2));

    let op_types: Vec<&str> = result
        .snapshot
        .ops
        .iter()
        .map(|op| op.op_type.as_str())
        .collect();
    assert_eq!(
        op_types,
        vec!["task.create", "review", "verify", "task.update", "finish"]
    );

    let review_run_id = output
        .get("reviewRunId")
        .and_then(Value::as_str)
        .expect("review run id");
    let findings = db
        .list_review_findings_for_run(review_run_id)
        .expect("list review findings");
    assert!(findings.iter().any(|finding| {
        finding.title == "Debug output added in production code"
            && finding.file.ends_with("crates/ha-eval/src/lib.rs")
    }));

    let verification_run_id = output
        .get("verificationRunId")
        .and_then(Value::as_str)
        .expect("verification run id");
    let verification = db
        .verification_run_snapshot(verification_run_id, 20)
        .expect("load verification snapshot")
        .expect("verification run exists");
    assert_eq!(verification.steps.len(), 1);
    assert_eq!(
        verification.steps[0].command,
        "cargo check -p ha-eval --locked"
    );

    let goal_snapshot = db
        .goal_snapshot(&goal.goal.id, 100)
        .expect("goal snapshot")
        .expect("goal exists");
    let relations = goal_snapshot
        .evidence
        .iter()
        .map(|item| item.relation.as_str())
        .collect::<Vec<_>>();
    assert!(relations.contains(&"review_passed"));
    assert!(relations.contains(&"validation_completed"));
    assert!(relations.contains(&"workflow_completed"));
}

#[test]
fn phase2_eval_feature_workflow_writes_diffs_validates_and_finishes() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
    std::fs::write(workspace.join("README.md"), "# Hope\n").expect("write baseline");
    git(&workspace, &["add", "README.md"]);
    git(&workspace, &["commit", "-m", "initial"]);

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");
    db.update_session_permission_mode(&session.id, SessionMode::Yolo)
        .expect("set yolo mode");

    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Implement feature file" });
  await workflow.tool({
    name: "write",
    args: { path: "src/feature.txt", content: "feature enabled\n" },
    label: "write-feature"
  });
  const diff = await workflow.diff({ label: "feature-diff" });
  const validation = await workflow.validate({
    commands: ["test -f src/feature.txt"],
    reason: "feature file exists"
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    ok: validation.ok,
    changed: diff.changes.map((change) => change.path),
    resultCount: validation.results.length
  });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.feature".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 8 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.as_ref().expect("workflow output");
    assert_eq!(output.get("ok"), Some(&json!(true)));
    assert_eq!(output.get("resultCount"), Some(&json!(1)));
    let changed = output
        .get("changed")
        .and_then(Value::as_array)
        .expect("changed paths");
    assert!(changed.iter().any(|path| {
        path.as_str()
            .is_some_and(|path| path.ends_with("src/feature.txt"))
    }));
    assert_eq!(
        std::fs::read_to_string(workspace.join("src/feature.txt")).expect("read feature"),
        "feature enabled\n"
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
            "tool:write",
            "diff",
            "validate",
            "task.update",
            "finish"
        ]
    );
    assert!(result
        .snapshot
        .events
        .iter()
        .any(|event| event.event_type == "guarded_repair_validation_passed"));
}

#[test]
fn runtime_repair_loop_completes_after_successful_attempt() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
  const budget = { max_runtime_secs: 300, max_ops: 20, max_repair_attempts: 2 };
  const outer = await workflow.task.create({ title: "Repair loop success" });
  const repair = await workflow.repairLoop({
    label: "repair-success",
    maxAttempts: 2,
    validationCommands: ["true"],
    review: false,
    verify: false
  }, async ({ attempt }) => {
    await workflow.trace({ label: "attempt-callback", payload: { attempt } });
    return { changed: false, attempt };
  });
  await workflow.task.update({ task: outer, status: "completed" });
  await workflow.finish({ repair, budget });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 20 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.as_ref().expect("workflow output");
    assert_eq!(output.pointer("/repair/kind"), Some(&json!("repair_loop")));
    assert_eq!(output.pointer("/repair/ok"), Some(&json!(true)));
    assert_eq!(
        output.pointer("/repair/attempts/0/validationOk"),
        Some(&json!(true))
    );
    assert!(result.snapshot.events.iter().any(|event| {
        event.event_type == "trace"
            && event.payload.get("label").and_then(Value::as_str)
                == Some("repair-success:completed")
    }));
}

#[test]
fn runtime_repair_loop_blocks_when_attempt_budget_exhausted() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
  const budget = { max_runtime_secs: 300, max_ops: 20, max_repair_attempts: 1 };
  const outer = await workflow.task.create({ title: "Repair loop exhaustion" });
  await workflow.repairLoop({
    label: "repair-exhausted",
    maxAttempts: 1,
    validationCommands: ["printf still failing; exit 1"],
    review: false,
    verify: false
  }, async ({ attempt }) => {
    await workflow.trace({ label: "attempt-callback", payload: { attempt } });
    return { changed: false, attempt };
  });
  await workflow.task.update({ task: outer, status: "completed" });
  await workflow.finish({ reached: true, budget });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10, "max_ops": 20 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let err = run_workflow_script(db.clone(), &run.id).expect_err("repair loop must block");
    assert!(err.to_string().contains("repair_loop_attempts_exhausted"));
    let run = db
        .get_workflow_run(&run.id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("repair_loop_attempts_exhausted")
    );
    let snapshot = db
        .workflow_run_snapshot(&run.id, 100)
        .expect("snapshot")
        .expect("run snapshot");
    assert!(snapshot.events.iter().any(|event| {
        event.event_type == "trace"
            && event.payload.get("label").and_then(Value::as_str)
                == Some("repair-exhausted:exhausted")
    }));
    assert!(snapshot
        .events
        .iter()
        .any(|event| event.event_type == "workflow_block_requested"));
    assert!(snapshot
        .ops
        .iter()
        .any(|op| op.op_type == "block" && op.state == WorkflowOpState::Failed));
}

#[test]
fn runtime_validate_runs_targeted_exec_and_returns_structured_result() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
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

    let jobs_db = crate::async_jobs::get_async_jobs_db().expect("async jobs db");
    let jobs = jobs_db
        .list_for_session(&session.id, 10)
        .expect("list validation jobs");
    let validation_job = jobs
        .iter()
        .find(|job| job.tool_name == crate::tools::TOOL_EXEC)
        .expect("validation exec job");
    assert_eq!(validation_job.status, JobStatus::Completed);
    assert!(
        validation_job.injected,
        "workflow-owned validation jobs are shown by Workflow UI, not chat injection"
    );
}

#[test]
fn runtime_guarded_repair_blocks_repeated_validation_failure() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
  const task = await workflow.task.create({ title: "Repeated validation failure" });
  await workflow.validate({ commands: ["printf repeated; exit 1"] });
  await workflow.validate({ commands: ["printf repeated; exit 1"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ reached: true });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let _err = run_workflow_script(db.clone(), &run.id).expect_err("guarded repair must stop");

    let run = db
        .get_workflow_run(&run.id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("guarded_repair_same_validation_fingerprint")
    );

    let events = db
        .list_workflow_events(&run.id, 20)
        .expect("list workflow events");
    let repair_events: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "guarded_repair_validation_failed")
        .collect();
    assert_eq!(repair_events.len(), 2);
    assert_eq!(
        repair_events[1]
            .payload
            .get("stopReason")
            .and_then(Value::as_str),
        Some("guarded_repair_same_validation_fingerprint")
    );
}

#[test]
fn runtime_guarded_repair_blocks_no_effective_diff_progress() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(SessionDB::open(&dir.path().join("sessions.db")).expect("open session db"));
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    git(&workspace, &["init"]);

    let session = db.create_session("ha-main").expect("create session");
    db.update_session_working_dir(&session.id, Some(workspace.to_string_lossy().to_string()))
        .expect("set working dir");
    db.update_session_permission_mode(&session.id, SessionMode::Yolo)
        .expect("set yolo mode");

    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "No diff progress" });
  await workflow.validate({ commands: ["printf alpha; exit 1"] });
  await workflow.validate({ commands: ["printf beta; exit 1"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ reached: true });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let _err = run_workflow_script(db.clone(), &run.id).expect_err("guarded repair must stop");

    let run = db
        .get_workflow_run(&run.id)
        .expect("get run")
        .expect("run exists");
    assert_eq!(run.state, WorkflowRunState::Blocked);
    assert_eq!(
        run.blocked_reason.as_deref(),
        Some("guarded_repair_no_effective_diff")
    );
}

#[test]
fn runtime_execution_mode_off_does_not_apply_repair_guard() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
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
  const task = await workflow.task.create({ title: "Loop off validation" });
  const first = await workflow.validate({ commands: ["printf repeated; exit 1"] });
  const second = await workflow.validate({ commands: ["printf repeated; exit 1"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ first: first.ok, second: second.ok });
}
"#;
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "off".to_string(),
            script_source: script.to_string(),
            budget: json!({ "max_script_secs": 10 }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({ "first": false, "second": false }))
    );

    let events = db
        .list_workflow_events(&run.id, 20)
        .expect("list workflow events");
    assert!(!events
        .iter()
        .any(|event| event.event_type.starts_with("guarded_repair_")));
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
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

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
fn runtime_map_materializes_items_and_replays_partial_children() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Fan out" });
  const results = await workflow.map("letters", ["a", "b"], async (item, index) => {
    const event = await workflow.trace({
      label: `item:${item}`,
      payload: { item, index }
    });
    return { item, index, eventSeq: event.eventSeq };
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ results });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(map)".to_string(),
        op_type: "map".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({
            "label": "letters",
            "items": ["a", "b"]
        }),
        child_handle: None,
    })
    .expect("start preexisting map op");
    db.complete_workflow_op(
        &run_id,
        "main/op#1(map)",
        json!({
            "label": "letters",
            "items": ["a", "b"],
            "opKey": "main/op#1(map)"
        }),
    )
    .expect("complete preexisting map op");

    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(map)/item#0/op#0(trace)".to_string(),
        op_type: "trace".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({
            "label": "item:a",
            "payload": { "item": "a", "index": 0 }
        }),
        child_handle: None,
    })
    .expect("start preexisting first child op");
    db.complete_workflow_op(
        &run_id,
        "main/op#1(map)/item#0/op#0(trace)",
        json!({ "eventSeq": 99 }),
    )
    .expect("complete preexisting first child op");

    let result = run_workflow_script(db.clone(), &run_id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.as_ref().expect("workflow output");
    let results = output
        .get("results")
        .and_then(Value::as_array)
        .expect("results array");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].get("item"), Some(&json!("a")));
    assert_eq!(results[0].get("index"), Some(&json!(0)));
    assert_eq!(results[0].get("eventSeq"), Some(&json!(99)));
    assert_eq!(results[1].get("item"), Some(&json!("b")));
    assert_eq!(results[1].get("index"), Some(&json!(1)));
    assert!(results[1]
        .get("eventSeq")
        .and_then(Value::as_i64)
        .is_some_and(|seq| seq > 0));

    let mut op_types: Vec<(String, String)> = result
        .snapshot
        .ops
        .iter()
        .map(|op| (op.op_key.clone(), op.op_type.clone()))
        .collect();
    op_types.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        op_types,
        vec![
            (
                "main/op#0(task.create)".to_string(),
                "task.create".to_string()
            ),
            ("main/op#1(map)".to_string(), "map".to_string()),
            (
                "main/op#1(map)/item#0/op#0(trace)".to_string(),
                "trace".to_string()
            ),
            (
                "main/op#1(map)/item#1/op#0(trace)".to_string(),
                "trace".to_string()
            ),
            (
                "main/op#2(task.update)".to_string(),
                "task.update".to_string()
            ),
            ("main/op#3(finish)".to_string(), "finish".to_string()),
        ]
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
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

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
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");
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
fn runtime_spawn_agent_dispatches_real_subagent_tool_with_preallocated_run_id() {
    let _guard = async_jobs_test_guard();
    let (root, db) = workflow_spawn_global_env();
    crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
        let parent = db.create_session("ha-main").expect("create parent session");
        let active_run_id = uuid::Uuid::new_v4().to_string();
        db.insert_subagent_run(&SubagentRun {
            run_id: active_run_id,
            parent_session_id: parent.id.clone(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "ha-review".to_string(),
            child_session_id: "child-active".to_string(),
            task: "Active run holding the only slot".to_string(),
            status: SubagentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            model_used: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms: None,
            label: Some("active".to_string()),
            attachment_count: 0,
            input_tokens: None,
            output_tokens: None,
        })
        .expect("insert active subagent run");

        let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Delegate review" });
  const review = await workflow.spawnAgent({
    task: "Review the generated patch",
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
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: parent.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: script.to_string(),
                budget: json!({ "max_script_secs": 10, "max_ops": 8 }),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                worktree_id: None,
            })
            .expect("create workflow run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
            .expect("start workflow run");

        let result = run_workflow_script(db.clone(), &run.id).expect("run workflow script");
        assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
        let output = result.output.as_ref().expect("workflow output");
        assert_eq!(output.get("status"), Some(&json!("spawned")));
        assert_eq!(output.get("label"), Some(&json!("review")));
        let child_run_id = output
            .get("runId")
            .and_then(Value::as_str)
            .expect("child run id");

        let child = db
            .get_subagent_run(child_run_id)
            .expect("read spawned subagent")
            .expect("spawned subagent exists");
        assert_eq!(child.parent_session_id, parent.id);
        assert_eq!(child.parent_agent_id, "ha-main");
        assert_eq!(child.child_agent_id, "ha-review");
        assert_eq!(child.task, "Review the generated patch");
        assert_eq!(child.status, SubagentStatus::Queued);
        assert_eq!(child.label.as_deref(), Some("review"));

        let spawn_op = result
            .snapshot
            .ops
            .iter()
            .find(|op| op.op_type == "spawnAgent")
            .expect("spawn op");
        assert_eq!(spawn_op.state, WorkflowOpState::Completed);
        assert_eq!(spawn_op.child_handle.as_deref(), Some(child_run_id));

        let jobs_db = crate::async_jobs::get_async_jobs_db().expect("async jobs db");
        let projection = jobs_db
            .get_subagent_projection(child_run_id)
            .expect("read subagent projection")
            .expect("subagent projection");
        assert_eq!(projection.kind, JobKind::Subagent);
        assert_eq!(projection.subagent_run_id.as_deref(), Some(child_run_id));
        assert_eq!(projection.session_id.as_deref(), Some(parent.id.as_str()));

        let _ = crate::subagent::queue::remove_for_run(child_run_id);
        if let Some(registry) = crate::get_subagent_cancels() {
            registry.remove(child_run_id);
        }
    });
}

#[tokio::test(flavor = "current_thread")]
async fn phase2_eval_parallel_spawn_agents_complete_with_mock_model_response() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let _guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let (root, db) = workflow_spawn_global_env();

    crate::test_support::with_env_vars_async(&[("HA_DATA_DIR", root.path())], || async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(phase2_openai_chat_sse("mock reviewer completed")),
            )
            .mount(&server)
            .await;

        let unique = uuid::Uuid::new_v4().simple().to_string();
        let parent_agent = format!("ha-phase2-main-{unique}");
        let child_agent = format!("ha-phase2-review-{unique}");
        let provider_id = format!("phase2-openai-chat-{unique}");
        let model_id = "phase2-mock-review";
        let model_ref = format!("{provider_id}::{model_id}");

        write_workflow_spawn_agent(&parent_agent, |cfg| {
            cfg.subagents.max_concurrent = 2;
            cfg.subagents.allowed_agents = vec![child_agent.clone()];
        });
        write_workflow_spawn_agent(&child_agent, |cfg| {
            cfg.model.primary = Some(model_ref.clone());
            cfg.subagents.denied_agents = vec![child_agent.clone()];
        });

        let mut provider = ProviderConfig::new(
            "Phase2 Mock OpenAI Chat".to_string(),
            ApiType::OpenaiChat,
            server.uri(),
            "test-key".to_string(),
        );
        provider.id = provider_id.clone();
        provider.models.push(phase2_mock_model_config(model_id));

        let mut config = crate::config::AppConfig::default();
        config.providers = vec![provider];
        config.active_model = Some(ActiveModel {
            provider_id,
            model_id: model_id.to_string(),
        });
        let _config_restore = replace_config_cache_for_test(config);

        let parent = db
            .create_session(&parent_agent)
            .expect("create parent workflow session");
        let _busy_parent_guard = crate::subagent::ChatSessionGuard::new(&parent.id);
        let script = format!(
            r#"
export default async function main(workflow) {{
  const task = await workflow.task.create({{ title: "Parallel mock review" }});
  const first = await workflow.spawnAgent({{
    task: "Review API surface A",
    agent: "{child_agent}",
    label: "review-a"
  }});
  const second = await workflow.spawnAgent({{
    task: "Review API surface B",
    agent: "{child_agent}",
    label: "review-b"
  }});
  const waited = await workflow.waitAll([first, second], {{ waitTimeout: 8 }});
  await workflow.task.update({{ task, status: "completed" }});
  await workflow.finish({{
    allCompleted: waited.allCompleted,
    statuses: waited.runs.map((run) => run.status),
    results: waited.runs.map((run) => run.result_preview),
    runIds: waited.runs.map((run) => run.runId)
  }});
}}
"#,
        );
        let run = db
            .create_workflow_run(CreateWorkflowRunInput {
                session_id: parent.id.clone(),
                kind: "coding.workflow".to_string(),
                execution_mode: "guarded".to_string(),
                script_source: script,
                budget: json!({ "max_script_secs": 15, "max_ops": 10 }),
                parent_run_id: None,
                origin: None,
                goal_id: None,
                worktree_id: None,
            })
            .expect("create workflow run");
        db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
            .expect("start workflow run");

        let result = run_workflow_script_async(db.clone(), &run.id)
            .await
            .expect("run workflow script");
        assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);

        let output = result.output.as_ref().expect("workflow output");
        assert_eq!(output.get("allCompleted"), Some(&json!(true)));
        assert_eq!(
            output.get("statuses"),
            Some(&json!(["completed", "completed"]))
        );
        assert_eq!(
            output.get("results"),
            Some(&json!([
                "mock reviewer completed",
                "mock reviewer completed"
            ]))
        );

        let run_ids = output
            .get("runIds")
            .and_then(Value::as_array)
            .expect("run ids");
        assert_eq!(run_ids.len(), 2);
        for run_id in run_ids {
            let run_id = run_id.as_str().expect("run id string");
            let child = db
                .get_subagent_run(run_id)
                .expect("read child run")
                .expect("child run exists");
            assert_eq!(child.status, SubagentStatus::Completed);
            assert_eq!(child.result.as_deref(), Some("mock reviewer completed"));
            assert_eq!(child.model_used.as_deref(), Some(model_ref.as_str()));
        }

        let spawn_ops = result
            .snapshot
            .ops
            .iter()
            .filter(|op| op.op_type == "spawnAgent")
            .count();
        assert_eq!(spawn_ops, 2);
        let wait_op = result
            .snapshot
            .ops
            .iter()
            .find(|op| op.op_type == "waitAll")
            .expect("waitAll op");
        assert_eq!(wait_op.state, WorkflowOpState::Completed);

        drop(_busy_parent_guard);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let requests = server
            .received_requests()
            .await
            .expect("received mock provider requests");
        assert_eq!(
            requests.len(),
            2,
            "only the two child model calls should reach the mock provider"
        );
    })
    .await;
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
fn runtime_blocks_new_spawn_agent_after_output_token_budget_is_spent() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 60, max_ops: 12, maxOutputTokens: 5 };
  const task = await workflow.task.create({ title: "Check budget" });
  await workflow.task.update({ task, status: "in_progress" });
  const first = await workflow.spawnAgent({ task: "First", agent_id: "ha-review", label: "first" });
  await workflow.waitAll([first], { waitTimeout: 1 });
  await workflow.spawnAgent({ task: "Second", agent_id: "ha-review", label: "second" });
  await workflow.validate({ commands: ["echo unreachable"], reason: "budget test" });
  await workflow.finish({ summary: "unreachable", budget });
}
"#;
    let session = db.create_session("ha-main").expect("create session");
    let run = db
        .create_workflow_run(CreateWorkflowRunInput {
            session_id: session.id.clone(),
            kind: "coding.workflow".to_string(),
            execution_mode: "guarded".to_string(),
            script_source: script.to_string(),
            budget: json!({
                "max_script_secs": 10,
                "max_ops": 12,
                "maxOutputTokens": 5,
            }),
            parent_run_id: None,
            origin: None,
            goal_id: None,
            worktree_id: None,
        })
        .expect("create workflow run");
    db.transition_workflow_run(&run.id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let first_run_id = uuid::Uuid::new_v4().to_string();
    let first_handle = json!({
        "kind": "subagent",
        "runId": first_run_id,
        "run_id": first_run_id,
        "status": "completed",
        "label": "first",
        "task": "First",
        "message": "attached to existing sub-agent run",
    });
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run.id.clone(),
        op_key: "main/op#2(spawnAgent)".to_string(),
        op_type: "spawnAgent".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "args": {
                "action": "spawn",
                "task": "First",
                "agent_id": "ha-review",
                "label": "first"
            },
            "label": "first"
        }),
        child_handle: Some(first_run_id.clone()),
    })
    .expect("start first spawn op");
    db.complete_workflow_op(&run.id, "main/op#2(spawnAgent)", first_handle.clone())
        .expect("complete first spawn op");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run.id.clone(),
        op_key: "main/op#3(waitAll)".to_string(),
        op_type: "waitAll".to_string(),
        effect_class: WorkflowEffectClass::Pure,
        input: json!({
            "handles": [first_handle],
            "waitTimeout": 1
        }),
        child_handle: None,
    })
    .expect("start waitAll op");
    db.complete_workflow_op(
        &run.id,
        "main/op#3(waitAll)",
        json!({
            "allCompleted": true,
            "all_completed": true,
            "runs": [{
                "runId": first_run_id,
                "run_id": first_run_id,
                "status": "completed",
                "result_preview": "done"
            }]
        }),
    )
    .expect("complete waitAll op");
    db.insert_subagent_run(&SubagentRun {
        run_id: first_run_id,
        parent_session_id: session.id.clone(),
        parent_agent_id: "ha-main".to_string(),
        child_agent_id: "ha-review".to_string(),
        child_session_id: "child-session".to_string(),
        task: "First".to_string(),
        status: SubagentStatus::Completed,
        result: Some("done".to_string()),
        error: None,
        depth: 1,
        model_used: Some("mock".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: Some(chrono::Utc::now().to_rfc3339()),
        duration_ms: Some(1),
        label: Some("first".to_string()),
        attachment_count: 0,
        input_tokens: Some(7),
        output_tokens: Some(6),
    })
    .expect("insert completed subagent run");

    let err =
        run_workflow_script(db.clone(), &run.id).expect_err("budget should block second spawn");
    assert!(
        err.to_string()
            .contains("workflow output token budget exhausted"),
        "{err:#}"
    );
    let snapshot = db
        .workflow_run_snapshot(&run.id, 100)
        .expect("snapshot")
        .expect("run exists");
    assert_eq!(snapshot.run.state, WorkflowRunState::Blocked);
    assert_eq!(
        snapshot.run.blocked_reason.as_deref(),
        Some("workflow_budget_output_tokens_exhausted")
    );
    let budget_events: Vec<_> = snapshot
        .events
        .iter()
        .filter(|event| event.event_type == "budget_usage")
        .collect();
    assert_eq!(
        budget_events.len(),
        1,
        "replaying completed waitAll must not duplicate budget usage events"
    );
    assert_eq!(
        budget_events[0]
            .payload
            .get("spentOutputTokens")
            .and_then(Value::as_u64),
        Some(6)
    );
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
fn runtime_attaches_started_validate_child_job_without_blocking() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Validate recovery" });
  const validation = await workflow.validate({ commands: ["echo recovered"] });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    ok: validation.ok,
    jobId: validation.results[0].jobId,
    output: validation.results[0].output
  });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    insert_completed_async_job(&job_id, &session_id, "recovered\n[exit code: 0]");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(validate)".to_string(),
        op_type: "validate".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({ "commands": ["echo recovered"] }),
        child_handle: Some(
            json!({
                "kind": "validate",
                "jobs": [{
                    "jobId": job_id.clone(),
                    "command": "echo recovered",
                    "cwd": null,
                    "timeout": null
                }]
            })
            .to_string(),
        ),
    })
    .expect("start validate op");

    let result = run_workflow_script(db.clone(), &run_id).expect("recover workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({
            "ok": true,
            "jobId": job_id,
            "output": "recovered\n[exit code: 0]"
        }))
    );

    let validate_op = db
        .get_workflow_op(&run_id, "main/op#1(validate)")
        .expect("get validate op")
        .expect("validate op exists");
    assert_eq!(validate_op.state, WorkflowOpState::Completed);
    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    assert_ne!(run.state, WorkflowRunState::Blocked);
}

#[test]
fn runtime_attaches_started_async_tool_child_job_without_blocking() {
    let _async_guard = async_jobs_test_guard();
    ensure_async_jobs_db();
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Async tool recovery" });
  const started = await workflow.tool({
    name: "exec",
    args: { command: "echo recovered", run_in_background: true }
  });
  const job = JSON.parse(started);
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ jobId: job.job_id, status: job.status });
}
"#;
    let (session_id, run_id) = create_run_with_script(&db, script);
    db.transition_workflow_run(&run_id, WorkflowRunState::Running, Some("test"))
        .expect("run");

    let job_id = format!("job_{}", uuid::Uuid::new_v4().simple());
    insert_completed_async_job(&job_id, &session_id, "recovered\n[exit code: 0]");
    db.upsert_workflow_op_started(UpsertWorkflowOpInput {
        run_id: run_id.clone(),
        op_key: "main/op#1(tool:exec)".to_string(),
        op_type: "tool:exec".to_string(),
        effect_class: WorkflowEffectClass::NonIdempotent,
        input: json!({
            "name": "exec",
            "args": {
                "command": "echo recovered",
                "run_in_background": true
            },
            "label": null
        }),
        child_handle: Some(job_id.clone()),
    })
    .expect("start async tool op");

    let result = run_workflow_script(db.clone(), &run_id).expect("recover workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    assert_eq!(
        result.output,
        Some(json!({
            "jobId": job_id,
            "status": "started"
        }))
    );

    let tool_op = db
        .get_workflow_op(&run_id, "main/op#1(tool:exec)")
        .expect("get async tool op")
        .expect("async tool op exists");
    assert_eq!(tool_op.state, WorkflowOpState::Completed);
    assert_eq!(tool_op.child_handle.as_deref(), Some(job_id.as_str()));
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

#[test]
fn runtime_deterministic_helpers_replace_time_and_random_sources() {
    let (_dir, db_raw) = temp_db();
    let db = Arc::new(db_raw);
    let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 300, max_ops: 4 };
  const first = workflow.random("stable-seed");
  const repeated = workflow.random("stable-seed");
  const other = workflow.random("other-seed");
  const task = await workflow.task.create({ title: "Use deterministic helpers" });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({
    now: workflow.now(),
    first,
    repeated,
    other,
    budget
  });
}
"#;
    let (_session_id, run_id) = create_run_with_script(&db, script);
    let run = db
        .get_workflow_run(&run_id)
        .expect("get run")
        .expect("run exists");
    let expected_now = chrono::DateTime::parse_from_rfc3339(&run.created_at)
        .expect("parse created_at")
        .timestamp_millis();

    let result = run_workflow_script(db.clone(), &run_id).expect("run workflow script");
    assert_eq!(result.snapshot.run.state, WorkflowRunState::Completed);
    let output = result.output.as_ref().expect("workflow output");
    assert_eq!(output.get("now"), Some(&json!(expected_now)));
    assert_eq!(output.get("first"), output.get("repeated"));
    assert_ne!(output.get("first"), output.get("other"));
    for key in ["first", "repeated", "other"] {
        let value = output
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("{key} should be a number: {output}"));
        assert!((0.0..1.0).contains(&value), "{key} out of range: {value}");
    }
    let ops = db.list_workflow_ops(&run_id).expect("list ops");
    assert_eq!(
        ops.iter().map(|op| op.op_type.as_str()).collect::<Vec<_>>(),
        vec!["task.create", "task.update", "finish"],
        "deterministic helpers should not create durable ops"
    );
}

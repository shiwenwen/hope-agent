//! 托管 `/loop` 的 workflow 执行策略 → 领域运营闸 / soak 报表的**跨子系统**
//! 集成测试。
//!
//! 阶段 5 第八刀自 `ha_core::loop_control` 的 `mod tests` 迁来：断言的主体
//! （两道闸）随机器上浮到本 crate，而 kernel 不能反向 use 特征 crate。放在
//! `tests/` 而非模块内单测，是因为它本来就横跨 loop_control + workflow +
//! domain_eval 三个子系统，只是过去恰好住在其中一个的私有测试模块里。

use ha_core::cron::CronDB;
use ha_core::domain_eval::{DomainOperationalGateInput, DomainSoakReportInput};
use ha_core::goal::CreateGoalInput;
use ha_core::loop_control::{
    CreateLoopScheduleInput, LoopExecutionStrategy, LoopRunDecision, LoopRunState, LoopTriggerKind,
};
use ha_core::session::SessionDB;
use ha_core::util::now_rfc3339;
use ha_core::workflow::WorkflowRunState;
use ha_improve::domain_eval::{evaluate_domain_operational_gate, generate_domain_soak_report};
use serde_json::json;

fn temp_dbs() -> (tempfile::TempDir, SessionDB, CronDB) {
    ha_workflow::wire();
    ha_improve::wire();
    let dir = tempfile::tempdir().expect("tempdir");
    let session_db =
        SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db")).expect("session db");
    session_db
        .with_conn_for_test(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL,
                    thread_id TEXT,
                    session_id TEXT NOT NULL,
                    sender_id TEXT,
                    sender_name TEXT,
                    chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    attached_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );",
            )?;
            Ok(())
        })
        .expect("channel conversations table");
    let cron_db = CronDB::open(&dir.path().join("cron.db")).expect("cron db");
    (dir, session_db, cron_db)
}

#[test]
fn workflow_strategy_feeds_operational_and_soak_gates() {
    let (_dir, session_db, cron_db) = temp_dbs();
    let session = session_db.create_session("ha-main").expect("session");
    let goal = session_db
        .create_goal(CreateGoalInput {
            session_id: session.id.clone(),
            objective: "Keep the weekly writing brief fresh".to_string(),
            completion_criteria: "A reviewed writing brief workflow completes".to_string(),
            domain: Some("writing".to_string()),
            workflow_template_id: Some("writing-brief".to_string()),
            workflow_template_version: None,
            workflow_task_type: Some("weekly_report".to_string()),
            budget_token_limit: None,
            budget_time_limit_secs: None,
            budget_turn_limit: None,
        })
        .expect("create goal");
    let schedule = session_db
        .create_loop_schedule(
            &cron_db,
            CreateLoopScheduleInput {
                session_id: session.id.clone(),
                goal_id: Some(goal.goal.id.clone()),
                goal_criterion_id: None,
                prompt: "Refresh the brief from the newest evidence".into(),
                trigger_kind: LoopTriggerKind::Interval,
                trigger_spec: json!({ "intervalSecs": 60 }),
                execution_strategy: LoopExecutionStrategy::Workflow,
                max_runs: None,
                max_runtime_secs: None,
                token_budget: None,
                cost_budget_micros: None,
                max_no_progress_runs: None,
                max_failures: None,
                backoff_secs: None,
                agent_id: None,
            },
        )
        .expect("create workflow loop");
    let started_at = now_rfc3339();
    let admission = match session_db
        .prepare_loop_cron_run(&schedule.cron_job_id, &session.id, &started_at)
        .expect("prepare loop")
    {
        LoopRunDecision::Admit(admission) => admission,
        other => panic!("expected admission, got {other:?}"),
    };
    let launch = session_db
        .create_loop_workflow_run(&admission)
        .expect("create loop workflow run");
    session_db
        .transition_workflow_run(&launch.run_id, WorkflowRunState::Running, Some("loop_tick"))
        .expect("start workflow");
    session_db
        .transition_workflow_run(
            &launch.run_id,
            WorkflowRunState::Completed,
            Some("loop_tick_completed"),
        )
        .expect("complete workflow");
    let finished_at = now_rfc3339();
    session_db
        .finish_loop_cron_run_with_trace(
            &schedule.cron_job_id,
            Some(&admission.run_id),
            None,
            LoopRunState::Succeeded,
            Some("workflow launched and drained"),
            None,
            &finished_at,
            Some(json!({
                "executionStrategy": "workflow",
                "workflowRunId": launch.run_id,
                "workflowKind": launch.workflow_kind,
                "templateId": launch.template_id,
                "templateVersion": launch.template_version,
            })),
        )
        .expect("finish loop run");

    let operational = evaluate_domain_operational_gate(
        &session_db,
        DomainOperationalGateInput {
            session_id: Some(session.id.clone()),
            domain: Some("writing".to_string()),
            window_days: Some(1),
            min_workflow_runs: Some(1),
            min_loop_runs: Some(1),
            ..Default::default()
        },
    )
    .expect("evaluate operational gate");
    assert_eq!(operational.status, "passed", "{operational:?}");
    assert_eq!(operational.summary.workflow_runs, 1);
    assert_eq!(operational.summary.completed_workflow_runs, 1);
    assert_eq!(operational.summary.loop_runs, 1);
    assert_eq!(operational.summary.succeeded_loop_runs, 1);
    assert_eq!(operational.summary.active_workflow_runs, 0);
    assert!(operational.blockers.is_empty());

    let soak = generate_domain_soak_report(
        &session_db,
        DomainSoakReportInput {
            session_id: Some(session.id.clone()),
            domain: Some("writing".to_string()),
            window_days: Some(1),
            max_items: Some(20),
            ..Default::default()
        },
    )
    .expect("generate soak report");
    assert_eq!(soak.status, "passed", "{soak:?}");
    assert_eq!(soak.summary.workflow_runs, 1);
    assert_eq!(soak.summary.completed_workflow_runs, 1);
    assert_eq!(soak.summary.loop_runs, 1);
    assert_eq!(soak.summary.succeeded_loop_runs, 1);
    assert_eq!(soak.summary.critical_incidents, 0);
    assert!(soak
        .timeline
        .iter()
        .any(|item| item.source == "workflow" && item.id == launch.run_id));
    assert!(soak
        .timeline
        .iter()
        .any(|item| item.source == "loop" && item.id == admission.run_id));
}

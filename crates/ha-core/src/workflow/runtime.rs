use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};
use rquickjs::function::Opt;
use rquickjs::prelude::{Func, MutFn};
use rquickjs::{
    CatchResultExt, Context, Ctx, Exception, Function, Object, Runtime, Value as JsValue,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::runtime::Handle as TokioHandle;

use crate::async_jobs::{BackgroundJob, JobManager, JobOrigin, JobStatus};
use crate::domain_workflow::{DomainEvidenceItem, RecordDomainEvidenceInput};
use crate::plan::check_workflow_script_draft;
use crate::review::{self, ReviewFindingStatus, RunReviewInput};
use crate::runtime_tasks::{cancel_runtime_task, RuntimeTaskKind};
use crate::session::{SessionDB, SessionIdeContext, Task, TaskStatus};
use crate::tools::{self, ToolExecContext};
use crate::verification::{self, PlanVerificationInput};

use super::types::{
    UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowOpState, WorkflowRun, WorkflowRunSnapshot,
    WorkflowRunState,
};

const DEFAULT_SCRIPT_TIMEOUT_SECS: u64 = 30;
const MAX_SCRIPT_TIMEOUT_SECS: u64 = 300;
const SCRIPT_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const SCRIPT_STACK_LIMIT_BYTES: usize = 1024 * 1024;
const REPAIR_VALIDATION_FAILED_EVENT: &str = "guarded_repair_validation_failed";
const REPAIR_VALIDATION_PASSED_EVENT: &str = "guarded_repair_validation_passed";
const REPAIR_SAME_VALIDATION_REASON: &str = "guarded_repair_same_validation_fingerprint";
const REPAIR_NO_EFFECTIVE_DIFF_REASON: &str = "guarded_repair_no_effective_diff";
const REPAIR_LOOP_EXHAUSTED_REASON: &str = "repair_loop_attempts_exhausted";
const BUDGET_USAGE_EVENT: &str = "budget_usage";
const BUDGET_EXHAUSTED_REASON: &str = "workflow_budget_output_tokens_exhausted";
const VALIDATION_FINGERPRINT_OUTPUT_BYTES: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeResult {
    pub snapshot: WorkflowRunSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecoveryReport {
    pub owner: String,
    pub attempted: usize,
    pub recovered: usize,
    pub blocked: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub async fn recover_pending_workflow_runs(
    db: Arc<SessionDB>,
    owner: impl Into<String>,
) -> Result<WorkflowRecoveryReport> {
    let owner = owner.into();
    let mut report = WorkflowRecoveryReport {
        owner: owner.clone(),
        ..Default::default()
    };
    let runs = db
        .list_recoverable_workflow_runs()
        .context("list recoverable workflow runs")?;

    for run in runs {
        let Some(claimed) = db
            .claim_workflow_run_for_recovery(&run.id, &owner)
            .with_context(|| format!("claim workflow run {} for recovery", run.id))?
        else {
            report.skipped += 1;
            continue;
        };
        report.attempted += 1;

        match run_workflow_script_async(db.clone(), &claimed.id).await {
            Ok(result) => match result.snapshot.run.state {
                WorkflowRunState::Completed => report.recovered += 1,
                WorkflowRunState::Blocked => report.blocked += 1,
                WorkflowRunState::Failed => report.failed += 1,
                _ => {}
            },
            Err(err) => {
                let state = db
                    .get_workflow_run(&claimed.id)
                    .ok()
                    .flatten()
                    .map(|run| run.state);
                match state {
                    Some(WorkflowRunState::Blocked) => report.blocked += 1,
                    Some(WorkflowRunState::Failed) => report.failed += 1,
                    _ => report.failed += 1,
                }
                report.errors.push(format!("{}: {err:#}", claimed.id));
            }
        }
    }

    Ok(report)
}

pub fn spawn_startup_recovery_if_primary() {
    if !crate::runtime_lock::is_primary() {
        return;
    }
    let Some(db) = crate::get_session_db() else {
        return;
    };
    spawn_pending_workflow_milestone_injection_recovery(db.clone());
    let owner = format!("startup:pid:{}", std::process::id());
    tokio::spawn(async move {
        match recover_pending_workflow_runs(db.clone(), owner).await {
            Ok(report) => {
                if report.attempted > 0 || report.skipped > 0 || !report.errors.is_empty() {
                    crate::app_info!(
                        "workflow",
                        "startup_recovery",
                        "owner={} attempted={} recovered={} blocked={} failed={} skipped={} errors={}",
                        report.owner,
                        report.attempted,
                        report.recovered,
                        report.blocked,
                        report.failed,
                        report.skipped,
                        report.errors.len()
                    );
                }
            }
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "startup_recovery",
                    "workflow startup recovery failed: {err:#}"
                );
            }
        }
    });
}

fn spawn_pending_workflow_milestone_injection_recovery(db: Arc<SessionDB>) {
    tokio::spawn(async move {
        match recover_pending_workflow_milestone_injections(db.clone()) {
            Ok(recovered) => {
                if recovered > 0 {
                    crate::app_info!(
                        "workflow",
                        "milestone_injection_recovery",
                        "recovered {} pending workflow milestone injections",
                        recovered
                    );
                }
            }
            Err(err) => crate::app_warn!(
                "workflow",
                "milestone_injection_recovery",
                "workflow milestone injection recovery failed: {err:#}"
            ),
        }
        match recover_terminal_workflow_agent_checkpoints(&db) {
            Ok(recovered) if recovered > 0 => crate::app_info!(
                "workflow",
                "agent_checkpoint_recovery",
                "reconciled {} terminal workflow-owned child agent(s)",
                recovered
            ),
            Ok(_) => {}
            Err(err) => crate::app_warn!(
                "workflow",
                "agent_checkpoint_recovery",
                "workflow child checkpoint recovery failed: {err:#}"
            ),
        }
    });
}

pub(crate) fn recover_terminal_workflow_agent_checkpoints(db: &SessionDB) -> Result<usize> {
    let children = db.list_terminal_children_for_active_workflows(1000)?;
    let recovered = children.len();
    for (child_run_id, status) in children {
        on_workflow_child_status_changed(db, &child_run_id, status);
    }
    Ok(recovered)
}

fn recover_pending_workflow_milestone_injections(db: Arc<SessionDB>) -> Result<usize> {
    let pending = db
        .list_pending_workflow_milestone_injections(100)
        .context("list pending workflow milestone injections")?;
    let mut recovered = 0;
    for item in pending {
        if db
            .workflow_milestone_injection_settled(
                &item.run_id,
                &item.source_event_type,
                item.source_event_seq,
            )
            .unwrap_or(false)
        {
            continue;
        }
        spawn_workflow_milestone_injection(
            db.clone(),
            &item.run_id,
            &item.source_event_type,
            item.source_event_seq,
            &item.source_event.payload,
            false,
        );
        recovered += 1;
    }
    Ok(recovered)
}

pub fn spawn_workflow_run_if_primary(
    db: Arc<SessionDB>,
    run_id: impl Into<String>,
    owner: impl Into<String>,
) -> bool {
    let run_id = run_id.into();
    let owner = owner.into();
    if !crate::runtime_lock::is_primary() {
        let _ = db.append_workflow_event(
            &run_id,
            "run_runtime_launch",
            json!({
                "accepted": false,
                "owner": owner.as_str(),
                "reason": "not_primary",
                "pid": std::process::id(),
            }),
        );
        append_runtime_result_event(
            &db,
            &run_id,
            &owner,
            json!({
                "status": "rejected",
                "accepted": false,
                "reason": "not_primary",
            }),
        );
        crate::app_warn!(
            "workflow",
            "spawn_run",
            "skip workflow launch because this process is not primary"
        );
        return false;
    }

    let _ = db.append_workflow_event(
        &run_id,
        "run_runtime_launch",
        json!({
            "accepted": true,
            "owner": owner.as_str(),
            "reason": "primary_spawn_accepted",
            "pid": std::process::id(),
        }),
    );
    tokio::spawn(async move {
        let state = match db.get_workflow_run(&run_id) {
            Ok(Some(run)) => run.state,
            Ok(None) => {
                crate::app_warn!(
                    "workflow",
                    "spawn_run",
                    "workflow run {} not found before launch",
                    run_id
                );
                return;
            }
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "spawn_run",
                    "failed to load workflow run {} before launch: {err:#}",
                    run_id
                );
                return;
            }
        };

        let result = match state {
            WorkflowRunState::Draft | WorkflowRunState::Running | WorkflowRunState::Recovering => {
                match db.claim_workflow_run_for_launch(&run_id, &owner) {
                    Ok(Some(claimed)) => run_workflow_script_async(db.clone(), &claimed.id).await,
                    Ok(None) => {
                        append_runtime_result_event(
                            &db,
                            &run_id,
                            &owner,
                            json!({
                                "status": "skipped",
                                "accepted": true,
                                "reason": "claim_unavailable",
                                "initialState": state.as_str(),
                            }),
                        );
                        crate::app_info!(
                            "workflow",
                            "spawn_run",
                            "workflow run {} is already claimed or no longer launchable",
                            run_id
                        );
                        return;
                    }
                    Err(err) => Err(err).context("claim workflow run before launch"),
                }
            }
            WorkflowRunState::AwaitingApproval
            | WorkflowRunState::AwaitingUser
            | WorkflowRunState::Paused
            | WorkflowRunState::Completed
            | WorkflowRunState::Failed
            | WorkflowRunState::Cancelled
            | WorkflowRunState::Blocked => {
                append_runtime_result_event(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "skipped",
                        "accepted": true,
                        "reason": "state_not_launchable",
                        "initialState": state.as_str(),
                    }),
                );
                crate::app_info!(
                    "workflow",
                    "spawn_run",
                    "skip workflow run {} launch while state={}",
                    run_id,
                    state.as_str()
                );
                return;
            }
        };

        match result {
            Ok(result) => {
                append_runtime_result_event(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "finished",
                        "accepted": true,
                        "reason": "runtime_returned",
                        "finalState": result.snapshot.run.state.as_str(),
                        "hasOutput": result.output.is_some(),
                    }),
                );
                crate::app_info!(
                    "workflow",
                    "spawn_run",
                    "workflow run {} finished launch with state={}",
                    run_id,
                    result.snapshot.run.state.as_str()
                );
                maybe_spawn_workflow_result_injection(
                    db.clone(),
                    &run_id,
                    owner.as_str(),
                    Some(&result),
                    None,
                );
            }
            Err(err) => {
                append_runtime_result_event(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "error",
                        "accepted": true,
                        "reason": "runtime_error",
                        "error": err.to_string(),
                    }),
                );
                crate::app_warn!(
                    "workflow",
                    "spawn_run",
                    "workflow run {} launch failed: {err:#}",
                    run_id
                );
                maybe_spawn_workflow_result_injection(
                    db.clone(),
                    &run_id,
                    owner.as_str(),
                    None,
                    Some(&err.to_string()),
                );
            }
        }
    });
    true
}

fn append_runtime_result_event(db: &SessionDB, run_id: &str, owner: &str, payload: Value) {
    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object.insert("owner".to_string(), json!(owner));
        object.insert("pid".to_string(), json!(std::process::id()));
    }
    let _ = db.append_workflow_event(run_id, "run_runtime_result", payload);
}

fn maybe_spawn_workflow_result_injection(
    db: Arc<SessionDB>,
    run_id: &str,
    owner: &str,
    result: Option<&WorkflowRuntimeResult>,
    runtime_error: Option<&str>,
) {
    let run = match db.get_workflow_run(run_id) {
        Ok(Some(run)) => run,
        Ok(None) => return,
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "completion_injection",
                "failed to load workflow run {} for completion injection: {err:#}",
                run_id
            );
            return;
        }
    };

    let launched_by_workflow_tool = owner.starts_with("tool:workflow");
    let agent_origin = matches!(
        run.origin.as_deref(),
        Some("agent:workflow") | Some("agent:workflow_run")
    );
    if !launched_by_workflow_tool && !agent_origin {
        return;
    }
    if run.parent_run_id.is_some() {
        return;
    }
    if !run.state.is_terminal()
        && !matches!(
            run.state,
            WorkflowRunState::AwaitingApproval | WorkflowRunState::AwaitingUser
        )
    {
        return;
    }

    let session = match db.get_session(&run.session_id) {
        Ok(Some(session)) => session,
        Ok(None) => return,
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "completion_injection",
                "failed to load session {} for workflow injection: {err:#}",
                run.session_id
            );
            return;
        }
    };
    if session.incognito {
        return;
    }

    let snapshot = db.workflow_run_snapshot(&run.id, 160).ok().flatten();
    let output = result.and_then(|r| r.output.as_ref());
    let push_message =
        build_workflow_result_push_message(snapshot.as_ref(), &run, output, runtime_error);
    let parent_session_id = run.session_id.clone();
    let parent_agent_id = session.agent_id.clone();
    let run_id = run.id.clone();
    let session_db = db.clone();

    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let _ = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    parent_session_id,
                    parent_agent_id,
                    crate::subagent::injection::WORKFLOW_CHILD_AGENT_ID.to_string(),
                    run_id,
                    push_message,
                    session_db,
                    None,
                ));
            }
            Err(err) => crate::app_error!(
                "workflow",
                "completion_injection",
                "failed to build runtime for workflow completion injection: {}",
                err
            ),
        }
    });
}

fn should_inject_workflow_milestone(event_type: &str, payload: &Value) -> bool {
    let policy = payload
        .get("injectPolicy")
        .or_else(|| payload.get("inject"))
        .and_then(Value::as_str)
        .unwrap_or("auto");
    match policy {
        "never" => return false,
        "now" => return true,
        _ => {}
    }

    match event_type {
        "workflow_checkpoint" => matches!(
            payload.get("importance").and_then(Value::as_str),
            Some("high") | Some("critical")
        ),
        "workflow_report" => payload
            .get("needsUser")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

fn maybe_spawn_workflow_milestone_injection(
    db: Arc<SessionDB>,
    run_id: &str,
    event_type: &str,
    event_seq: i64,
    payload: &Value,
) {
    spawn_workflow_milestone_injection(db, run_id, event_type, event_seq, payload, true);
}

pub(crate) fn on_workflow_child_status_changed(
    db: &SessionDB,
    child_run_id: &str,
    status: crate::subagent::SubagentStatus,
) {
    let owners = match db.list_workflow_ops_for_child(child_run_id) {
        Ok(owners) => owners,
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "agent_status",
                "failed to resolve workflow owner for child {}: {err:#}",
                child_run_id
            );
            return;
        }
    };
    if owners.is_empty() {
        return;
    }

    for op in owners {
        let child = db.get_subagent_run(child_run_id).ok().flatten();
        let inject_policy = op
            .input
            .get("injectPolicy")
            .and_then(Value::as_str)
            .unwrap_or("none");
        let result_mode = op
            .input
            .get("resultMode")
            .and_then(Value::as_str)
            .unwrap_or("summary");
        let display_label = child
            .as_ref()
            .and_then(|run| run.label.as_deref())
            .or_else(|| op.input.get("label").and_then(Value::as_str));
        let terminal_event_exists = status.is_terminal()
            && db
                .workflow_agent_terminal_event_exists(&op.run_id, child_run_id)
                .unwrap_or(false);
        let result_handled = status.is_terminal()
            && db
                .workflow_agent_result_handled(&op.run_id, child_run_id)
                .unwrap_or(false);
        let checkpoint_exists = status.is_terminal()
            && db
                .workflow_agent_checkpoint_injection_run_ids(&op.run_id, child_run_id)
                .is_ok_and(|ids| !ids.is_empty());
        if terminal_event_exists
            && (inject_policy != "checkpoint" || result_handled || checkpoint_exists)
        {
            continue;
        }
        let event_type = if status.is_terminal() {
            "workflow_agent_terminal"
        } else {
            "workflow_agent_status_changed"
        };
        if !terminal_event_exists {
            let _ = db.append_workflow_event(
                &op.run_id,
                event_type,
                json!({
                    "childRunId": child_run_id,
                    "status": status.as_str(),
                    "label": display_label,
                    "injectPolicy": inject_policy,
                    "resultMode": result_mode,
                    "resultAvailable": child.as_ref().is_some_and(|run| run.result.is_some()),
                }),
            );
        }

        if !status.is_terminal() || inject_policy != "checkpoint" {
            continue;
        }
        if result_handled {
            let _ = db.append_workflow_event(
                &op.run_id,
                "workflow_agent_result_suppressed",
                json!({
                    "childRunIds": [child_run_id],
                    "reason": "already_consumed_before_checkpoint_injection",
                }),
            );
            continue;
        }

        let summary = child
            .as_ref()
            .and_then(|run| run.result.as_deref().or(run.error.as_deref()))
            .map(|value| truncate_chars(value, 2000))
            .unwrap_or_default();
        let checkpoint_payload = json!({
            "title": display_label,
            "summary": summary,
            "importance": "high",
            "injectPolicy": "now",
            "childRunId": child_run_id,
            "agentLabel": display_label,
            "agentStatus": status.as_str(),
            "fullResultAvailable": child.as_ref().is_some_and(|run| run.result.is_some()),
            "nextActionCode": "inspect_or_adjust_agents",
        });
        let Ok(event) = db.append_workflow_event(
            &op.run_id,
            "workflow_checkpoint",
            checkpoint_payload.clone(),
        ) else {
            continue;
        };
        if let Some(global_db) = crate::get_session_db() {
            maybe_spawn_workflow_milestone_injection(
                global_db.clone(),
                &op.run_id,
                "workflow_checkpoint",
                event.seq,
                &checkpoint_payload,
            );
        }
    }
}

fn spawn_workflow_milestone_injection(
    db: Arc<SessionDB>,
    run_id: &str,
    event_type: &str,
    event_seq: i64,
    payload: &Value,
    record_requested: bool,
) {
    let run = match db.get_workflow_run(run_id) {
        Ok(Some(run)) => run,
        Ok(None) => return,
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "milestone_injection",
                "failed to load workflow run {} for milestone injection: {err:#}",
                run_id
            );
            return;
        }
    };
    let injection_run_id = format!("{}:workflow-event:{}", run.id, event_seq);
    if let Some(child_run_id) = payload.get("childRunId").and_then(Value::as_str) {
        if db
            .workflow_agent_result_handled(&run.id, child_run_id)
            .unwrap_or(false)
        {
            crate::subagent::mark_run_fetched(&injection_run_id);
            if !db
                .workflow_milestone_injection_settled(&run.id, event_type, event_seq)
                .unwrap_or(false)
            {
                let _ = db.append_workflow_event(
                    &run.id,
                    "workflow_milestone_injection_suppressed",
                    json!({
                        "sourceEventType": event_type,
                        "sourceEventSeq": event_seq,
                        "injectionRunId": injection_run_id,
                        "childRunId": child_run_id,
                        "reason": "agent_result_already_consumed",
                    }),
                );
            }
            return;
        }
    }

    let agent_origin = run
        .origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("agent:workflow"));
    if !agent_origin {
        return;
    }

    let session = match db.get_session(&run.session_id) {
        Ok(Some(session)) => session,
        Ok(None) => return,
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "milestone_injection",
                "failed to load session {} for workflow milestone injection: {err:#}",
                run.session_id
            );
            return;
        }
    };
    if session.incognito {
        return;
    }

    let push_message = build_workflow_milestone_push_message(&run, event_type, event_seq, payload);
    if record_requested {
        let _ = db.append_workflow_event(
            &run.id,
            "workflow_milestone_injection_requested",
            json!({
                "sourceEventType": event_type,
                "sourceEventSeq": event_seq,
                "injectionRunId": injection_run_id,
                "title": payload.get("title").and_then(Value::as_str),
                "summary": payload.get("summary").and_then(Value::as_str),
            }),
        );
    }
    let parent_session_id = run.session_id.clone();
    let parent_agent_id = session.agent_id.clone();
    let session_db = db.clone();
    let delivered_db = db.clone();
    let delivered_run_id = run.id.clone();
    let delivered_event_type = event_type.to_string();
    let delivered_injection_run_id = injection_run_id.clone();
    let delivered_child_run_id = payload
        .get("childRunId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let on_injected: crate::subagent::injection::OnInjected = Arc::new(move || {
        if let Some(child_run_id) = delivered_child_run_id.as_deref() {
            if delivered_db
                .workflow_agent_result_handled(&delivered_run_id, child_run_id)
                .unwrap_or(false)
            {
                if !delivered_db
                    .workflow_milestone_injection_settled(
                        &delivered_run_id,
                        &delivered_event_type,
                        event_seq,
                    )
                    .unwrap_or(false)
                {
                    let _ = delivered_db.append_workflow_event(
                        &delivered_run_id,
                        "workflow_milestone_injection_suppressed",
                        json!({
                            "sourceEventType": delivered_event_type,
                            "sourceEventSeq": event_seq,
                            "injectionRunId": delivered_injection_run_id,
                            "childRunId": child_run_id,
                            "reason": "agent_result_consumed_while_injection_pending",
                        }),
                    );
                }
                return;
            }
        }
        let _ = delivered_db.append_workflow_event(
            &delivered_run_id,
            "workflow_milestone_injection_delivered",
            json!({
                "sourceEventType": delivered_event_type,
                "sourceEventSeq": event_seq,
                "injectionRunId": delivered_injection_run_id,
            }),
        );
        if let Some(child_run_id) = delivered_child_run_id.as_deref() {
            let _ = delivered_db.append_workflow_event(
                &delivered_run_id,
                "workflow_agent_result_consumed",
                json!({
                    "api": "checkpoint_injection",
                    "childRunIds": [child_run_id],
                }),
            );
            crate::subagent::mark_run_fetched(child_run_id);
        }
    });

    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let _ = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    parent_session_id,
                    parent_agent_id,
                    crate::subagent::injection::WORKFLOW_CHILD_AGENT_ID.to_string(),
                    injection_run_id,
                    push_message,
                    session_db,
                    Some(on_injected),
                ));
            }
            Err(err) => crate::app_error!(
                "workflow",
                "milestone_injection",
                "failed to build runtime for workflow milestone injection: {}",
                err
            ),
        }
    });
}

fn build_workflow_milestone_push_message(
    run: &WorkflowRun,
    event_type: &str,
    event_seq: i64,
    payload: &Value,
) -> String {
    const PAYLOAD_LIMIT: usize = 8 * 1024;

    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(match event_type {
            "workflow_report" => "Workflow report",
            "workflow_checkpoint" => "Workflow checkpoint",
            _ => "Workflow milestone",
        });
    let summary = payload
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("Workflow produced a stage-level update.");
    let next_action = payload
        .get("nextAction")
        .or_else(|| payload.get("next"))
        .and_then(Value::as_str)
        .unwrap_or("Call workflow.status or workflow.trace if details are needed.");
    let importance = payload
        .get("importance")
        .and_then(Value::as_str)
        .unwrap_or("normal");
    let needs_user = payload
        .get("needsUser")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let payload_json =
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    let (payload_json, payload_truncated) = truncate_for_injection(&payload_json, PAYLOAD_LIMIT);

    format!(
        "<workflow-checkpoint>\n\
         <run-id>{}</run-id>\n\
         <event-seq>{}</event-seq>\n\
         <event-type>{}</event-type>\n\
         <state>{}</state>\n\
         <kind>{}</kind>\n\
         <importance>{}</importance>\n\
         <needs-user>{}</needs-user>\n\
         <title>{}</title>\n\
         <summary>{}</summary>\n\
         <next-action>{}</next-action>\n\
         <payload-json truncated=\"{}\">\n{}\n</payload-json>\n\
         <query-hint>Use the workflow tool with action=status or action=trace and this run id if you need more detail.</query-hint>\n\
         </workflow-checkpoint>",
        escape_xml_text(&run.id),
        event_seq,
        escape_xml_text(event_type),
        escape_xml_text(run.state.as_str()),
        escape_xml_text(&run.kind),
        escape_xml_text(importance),
        needs_user,
        escape_xml_text(title),
        escape_xml_text(summary),
        escape_xml_text(next_action),
        payload_truncated,
        escape_xml_text(&payload_json)
    )
}

fn build_workflow_result_push_message(
    snapshot: Option<&WorkflowRunSnapshot>,
    run: &WorkflowRun,
    output: Option<&Value>,
    runtime_error: Option<&str>,
) -> String {
    const OUTPUT_LIMIT: usize = 16 * 1024;

    let (ops_total, ops_completed, ops_failed, ops_pending, ops_started) = snapshot
        .map(|snapshot| {
            let mut completed = 0usize;
            let mut failed = 0usize;
            let mut pending = 0usize;
            let mut started = 0usize;
            for op in &snapshot.ops {
                match op.state {
                    WorkflowOpState::Completed => completed += 1,
                    WorkflowOpState::Failed => failed += 1,
                    WorkflowOpState::Pending => pending += 1,
                    WorkflowOpState::Started => started += 1,
                }
            }
            (snapshot.ops.len(), completed, failed, pending, started)
        })
        .unwrap_or((0, 0, 0, 0, 0));

    let output_json = output
        .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
        .unwrap_or_default();
    let (output_json, output_truncated) = truncate_for_injection(&output_json, OUTPUT_LIMIT);
    let output_block = if output_json.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<output-json truncated=\"{}\">\n{}\n</output-json>\n",
            output_truncated,
            escape_xml_text(&output_json)
        )
    };
    let error_block = runtime_error
        .filter(|err| !err.trim().is_empty())
        .map(|err| format!("<error>{}</error>\n", escape_xml_text(err.trim())))
        .unwrap_or_default();
    let blocked_reason = run
        .blocked_reason
        .as_deref()
        .filter(|reason| !reason.trim().is_empty())
        .map(|reason| {
            format!(
                "<blocked-reason>{}</blocked-reason>\n",
                escape_xml_text(reason)
            )
        })
        .unwrap_or_default();
    let summary = match run.state {
        WorkflowRunState::Completed => "Workflow run completed. Use the output to answer the user.",
        WorkflowRunState::Blocked => {
            "Workflow run is blocked. Explain the blocker and the next action."
        }
        WorkflowRunState::Failed => "Workflow run failed. Explain the failure and recovery option.",
        WorkflowRunState::Cancelled => {
            "Workflow run was cancelled. Report that no final result was produced."
        }
        WorkflowRunState::AwaitingApproval => {
            "Workflow run is waiting for user approval before it can continue."
        }
        WorkflowRunState::AwaitingUser => {
            "Workflow run is waiting for user input before it can continue."
        }
        _ => "Workflow run changed state. Report the current state clearly.",
    };

    format!(
        "<workflow-result>\n\
         <run-id>{}</run-id>\n\
         <state>{}</state>\n\
         <kind>{}</kind>\n\
         <execution-mode>{}</execution-mode>\n\
         <ops total=\"{}\" completed=\"{}\" failed=\"{}\" pending=\"{}\" started=\"{}\" />\n\
         {blocked_reason}\
         {error_block}\
         {output_block}\
         <summary>{}</summary>\n\
         </workflow-result>",
        escape_xml_text(&run.id),
        escape_xml_text(run.state.as_str()),
        escape_xml_text(&run.kind),
        escape_xml_text(&run.execution_mode),
        ops_total,
        ops_completed,
        ops_failed,
        ops_pending,
        ops_started,
        escape_xml_text(summary)
    )
}

fn truncate_for_injection(input: &str, limit: usize) -> (String, bool) {
    if input.len() <= limit {
        return (input.to_string(), false);
    }
    let mut end = limit;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    (
        format!(
            "{}\n[truncated: {} bytes omitted]",
            &input[..end],
            input.len().saturating_sub(end)
        ),
        true,
    )
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn ensure_workflow_launcher_primary() -> Result<()> {
    if crate::runtime_lock::is_primary() {
        return Ok(());
    }
    Err(anyhow!(
        "workflow runs can only be started by the primary runtime process"
    ))
}

pub async fn cancel_workflow_run_with_children(
    db: Arc<SessionDB>,
    run_id: &str,
) -> Result<WorkflowRun> {
    let run = db.cancel_workflow_run(run_id)?;
    let child_refs = workflow_child_task_refs(&db, run_id)?;
    let mut results = Vec::new();
    for (kind, id) in child_refs {
        let kind_label = kind.as_str();
        match cancel_runtime_task(kind, &id).await {
            Ok(result) => results.push(json!(result)),
            Err(err) => results.push(json!({
                "kind": kind_label,
                "id": id,
                "accepted": false,
                "status": "error",
                "message": err.to_string(),
            })),
        }
    }
    if !results.is_empty() {
        let _ = db.append_workflow_event(
            run_id,
            "run_child_cancel_requested",
            json!({
                "children": results,
            }),
        );
    }
    Ok(run)
}

fn workflow_child_task_refs(
    db: &SessionDB,
    run_id: &str,
) -> Result<Vec<(RuntimeTaskKind, String)>> {
    let mut refs = Vec::new();
    for (op_type, child_handle) in db.list_workflow_child_handles(run_id)? {
        if op_type == "validate" {
            if let Ok(child) = parse_validation_child_handle(&child_handle) {
                refs.extend(
                    child
                        .jobs
                        .into_iter()
                        .map(|job| (RuntimeTaskKind::AsyncJob, job.job_id)),
                );
            }
        } else if op_type.starts_with("tool:") {
            refs.push((RuntimeTaskKind::AsyncJob, child_handle));
        } else if op_type == "spawnAgent" {
            refs.push((RuntimeTaskKind::Subagent, child_handle));
        }
    }
    Ok(refs)
}

pub fn run_workflow_script(db: Arc<SessionDB>, run_id: &str) -> Result<WorkflowRuntimeResult> {
    if TokioHandle::try_current().is_ok() {
        return Err(anyhow!(
            "run_workflow_script was called from an async runtime; use run_workflow_script_async"
        ));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create workflow runtime executor")?;
    runtime.block_on(run_workflow_script_async(db, run_id))
}

pub async fn run_workflow_script_async(
    db: Arc<SessionDB>,
    run_id: &str,
) -> Result<WorkflowRuntimeResult> {
    let run = db
        .get_workflow_run(run_id)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;

    if run.state == WorkflowRunState::Completed {
        return Ok(WorkflowRuntimeResult {
            snapshot: db
                .workflow_run_snapshot(run_id, 500)?
                .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?,
            output: None,
        });
    }
    if matches!(
        run.state,
        WorkflowRunState::Failed | WorkflowRunState::Cancelled | WorkflowRunState::Blocked
    ) {
        return Err(anyhow!(
            "workflow run {} is terminal ({}); refusing to execute script",
            run_id,
            run.state.as_str()
        ));
    }
    if run.state == WorkflowRunState::AwaitingApproval {
        return Err(anyhow!(
            "workflow run {} is awaiting user approval; approve it before execution",
            run_id
        ));
    }
    if run.state == WorkflowRunState::Paused {
        return Err(anyhow!("workflow run {} is paused", run_id));
    }

    let gate = check_workflow_script_draft(
        &run.script_source,
        super::preview::script_gate_options_for_execution_mode(&run.execution_mode),
    );
    if !gate.passed() {
        return Err(anyhow!(gate.render_feedback("Workflow Script Gate")));
    }
    if run.execution_mode == "autonomous" && !has_required_autonomous_budget(&run) {
        let _ = db.append_workflow_event(
            run_id,
            "workflow_budget_required",
            json!({
                "reason": "autonomous_requires_explicit_runtime_and_output_token_budget",
                "budget": run.budget.clone(),
            }),
        );
        let _ = db.transition_workflow_run(
            run_id,
            WorkflowRunState::Blocked,
            Some("autonomous_budget_required"),
        );
        return Err(anyhow!(
            "workflow run {} blocked: autonomous mode requires explicit max runtime and max output token budget",
            run_id
        ));
    }

    if run.state == WorkflowRunState::Draft {
        let preview = super::preview::preview_workflow_run(&db, &run);
        if preview.has_denials() {
            let _ = db.append_workflow_event(
                run_id,
                "script_permission_preview_blocked",
                json!({ "summary": preview.summary, "reason": "permission_preview_denied" }),
            );
            let _ = db.transition_workflow_run(
                run_id,
                WorkflowRunState::Blocked,
                Some("permission_preview_denied"),
            );
            return Err(anyhow!(
                "workflow run {} blocked by permission preview; inspect workflow trace",
                run_id
            ));
        }
        if preview.requires_user_approval() {
            let _ = db.append_workflow_event(
                run_id,
                "script_permission_approval_required",
                json!({ "summary": preview.summary }),
            );
            let _ = db.transition_workflow_run(
                run_id,
                WorkflowRunState::AwaitingApproval,
                Some("permission_preview"),
            );
            return Err(anyhow!(
                "workflow run {} requires user approval after permission preview",
                run_id
            ));
        }
    }

    let session_context = match workflow_session_context_for_run(&db, &run) {
        Ok(context) => context,
        Err(err) => {
            let _ = db.transition_workflow_run(
                run_id,
                WorkflowRunState::Blocked,
                Some("worktree_unavailable"),
            );
            return Err(err.context("workflow worktree unavailable"));
        }
    };
    if run.state != WorkflowRunState::Running {
        db.transition_workflow_run(run_id, WorkflowRunState::Running, Some("runtime_start"))?;
    }

    let tokio_handle = TokioHandle::current();
    let db_for_script = db.clone();
    let run_for_script = run.clone();
    let output = match tokio::task::spawn_blocking(move || {
        execute_script(db_for_script, run_for_script, session_context, tokio_handle)
    })
    .await
    .context("workflow runtime worker panicked or was cancelled")?
    {
        Ok(output) => output,
        Err(err) => {
            let _ =
                db.transition_workflow_run(run_id, WorkflowRunState::Failed, Some("runtime_error"));
            return Err(err);
        }
    };

    let snapshot = db
        .workflow_run_snapshot(run_id, 500)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
    Ok(WorkflowRuntimeResult {
        snapshot,
        output: Some(output),
    })
}

fn execute_script(
    db: Arc<SessionDB>,
    run: super::types::WorkflowRun,
    session_context: WorkflowSessionContext,
    tokio_handle: TokioHandle,
) -> Result<Value> {
    let runtime = Runtime::new().context("create QuickJS runtime")?;
    runtime.set_memory_limit(SCRIPT_MEMORY_LIMIT_BYTES);
    runtime.set_max_stack_size(SCRIPT_STACK_LIMIT_BYTES);

    let timeout = script_timeout(&run);
    let started_at = Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || started_at.elapsed() >= timeout)));

    let ctx = Context::full(&runtime).context("create QuickJS context")?;
    ctx.with(|ctx| -> Result<Value> {
        let host = Rc::new(RefCell::new(WorkflowRuntimeHost::new(
            db.clone(),
            run.id.clone(),
            run.session_id.clone(),
            run.created_at.clone(),
            run.goal_id.clone(),
            run.execution_mode.clone(),
            session_context.clone(),
            tokio_handle.clone(),
        )));
        let workflow = build_workflow_object(ctx.clone(), host.clone())?;
        ctx.globals()
            .set("workflow", workflow.clone())
            .context("install workflow global")?;
        install_workflow_js_helpers(&ctx)?;
        install_runtime_guards(&ctx)?;

        let script = prepare_script_for_eval(&run.script_source);
        ctx.eval::<(), _>(script)
            .catch(&ctx)
            .map_err(|err| anyhow!("workflow script load failed: {}", err))?;

        let main: Function = ctx
            .globals()
            .get("__hopeWorkflowMain")
            .context("workflow script must export default function main(workflow)")?;
        let raw = main
            .call::<_, JsValue>((workflow,))
            .catch(&ctx)
            .map_err(|err| anyhow!("workflow script failed: {}", err))?;
        let _returned = finish_maybe_promise(ctx.clone(), raw)
            .map_err(|err| anyhow!("workflow script promise failed: {}", err))?;

        let finished = host
            .borrow()
            .finished_output
            .clone()
            .ok_or_else(|| anyhow!("workflow script exited without workflow.finish(result)"))?;
        Ok(finished)
    })
}

fn build_workflow_object<'js>(
    ctx: Ctx<'js>,
    host: Rc<RefCell<WorkflowRuntimeHost>>,
) -> rquickjs::Result<Object<'js>> {
    let workflow = Object::new(ctx.clone())?;
    let task = Object::new(ctx.clone())?;

    let create_host = host.clone();
    task.set(
        "create",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &create_host, args, WorkflowRuntimeHost::task_create)
            },
        )),
    )?;

    let update_host = host.clone();
    task.set(
        "update",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &update_host, args, WorkflowRuntimeHost::task_update)
            },
        )),
    )?;
    workflow.set("task", task)?;

    let evidence = Object::new(ctx.clone())?;
    let evidence_record_host = host.clone();
    evidence.set(
        "record",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &evidence_record_host,
                    args,
                    WorkflowRuntimeHost::evidence_record,
                )
            },
        )),
    )?;
    workflow.set("evidence", evidence)?;

    let file_search_host = host.clone();
    workflow.set(
        "fileSearch",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &file_search_host,
                    args,
                    WorkflowRuntimeHost::file_search,
                )
            },
        )),
    )?;

    let tool_host = host.clone();
    workflow.set(
        "tool",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &tool_host, args, WorkflowRuntimeHost::tool)
            },
        )),
    )?;

    let spawn_agent_host = host.clone();
    workflow.set(
        "spawnAgent",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &spawn_agent_host,
                    args,
                    WorkflowRuntimeHost::spawn_agent,
                )
            },
        )),
    )?;

    let wait_all_host = host.clone();
    workflow.set(
        "waitAll",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handles: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                wait_all_host_call(&ctx, &wait_all_host, handles, options)
            },
        )),
    )?;

    let wait_any_host = host.clone();
    workflow.set(
        "waitAny",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handles: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                agent_handles_host_call(
                    &ctx,
                    &wait_any_host,
                    handles,
                    options,
                    "workflow.waitAny",
                    WorkflowRuntimeHost::wait_any,
                )
            },
        )),
    )?;

    let agent_status_host = host.clone();
    workflow.set(
        "agentStatus",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handles: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                agent_handles_host_call(
                    &ctx,
                    &agent_status_host,
                    handles,
                    options,
                    "workflow.agentStatus",
                    WorkflowRuntimeHost::agent_status,
                )
            },
        )),
    )?;

    let agent_result_host = host.clone();
    workflow.set(
        "agentResult",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handle: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                agent_handles_host_call(
                    &ctx,
                    &agent_result_host,
                    handle,
                    options,
                    "workflow.agentResult",
                    WorkflowRuntimeHost::agent_result,
                )
            },
        )),
    )?;

    let agent_steer_host = host.clone();
    workflow.set(
        "agentSteer",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handle: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                agent_handles_host_call(
                    &ctx,
                    &agent_steer_host,
                    handle,
                    options,
                    "workflow.agentSteer",
                    WorkflowRuntimeHost::agent_steer,
                )
            },
        )),
    )?;

    let cancel_agent_host = host.clone();
    workflow.set(
        "cancelAgent",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>,
                  handles: JsValue<'js>,
                  options: Opt<JsValue<'js>>|
                  -> rquickjs::Result<JsValue<'js>> {
                agent_handles_host_call(
                    &ctx,
                    &cancel_agent_host,
                    handles,
                    options,
                    "workflow.cancelAgent",
                    WorkflowRuntimeHost::cancel_agent,
                )
            },
        )),
    )?;

    let materialize_map_host = host.clone();
    workflow.set(
        "__materializeMap",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &materialize_map_host,
                    args,
                    WorkflowRuntimeHost::materialize_map,
                )
            },
        )),
    )?;

    let enter_map_item_host = host.clone();
    workflow.set(
        "__enterMapItem",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &enter_map_item_host,
                    args,
                    WorkflowRuntimeHost::enter_map_item,
                )
            },
        )),
    )?;

    let exit_map_item_host = host.clone();
    workflow.set(
        "__exitMapItem",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &exit_map_item_host,
                    args,
                    WorkflowRuntimeHost::exit_map_item,
                )
            },
        )),
    )?;

    let read_host = host.clone();
    workflow.set(
        "read",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &read_host, args, WorkflowRuntimeHost::read)
            },
        )),
    )?;

    let grep_host = host.clone();
    workflow.set(
        "grep",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &grep_host, args, WorkflowRuntimeHost::grep)
            },
        )),
    )?;

    let validate_host = host.clone();
    workflow.set(
        "validate",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &validate_host, args, WorkflowRuntimeHost::validate)
            },
        )),
    )?;

    let review_host = host.clone();
    workflow.set(
        "review",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &review_host, args, WorkflowRuntimeHost::review)
            },
        )),
    )?;

    let verify_host = host.clone();
    workflow.set(
        "verify",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &verify_host, args, WorkflowRuntimeHost::verify)
            },
        )),
    )?;

    let ask_user_host = host.clone();
    workflow.set(
        "askUser",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &ask_user_host, args, WorkflowRuntimeHost::ask_user)
            },
        )),
    )?;

    let diff_host = host.clone();
    workflow.set(
        "diff",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &diff_host, args, WorkflowRuntimeHost::diff)
            },
        )),
    )?;

    let phase_start_host = host.clone();
    workflow.set(
        "__phaseStart",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &phase_start_host,
                    args,
                    WorkflowRuntimeHost::phase_start,
                )
            },
        )),
    )?;

    let phase_complete_host = host.clone();
    workflow.set(
        "__phaseComplete",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &phase_complete_host,
                    args,
                    WorkflowRuntimeHost::phase_complete,
                )
            },
        )),
    )?;

    let phase_fail_host = host.clone();
    workflow.set(
        "__phaseFail",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &phase_fail_host,
                    args,
                    WorkflowRuntimeHost::phase_fail,
                )
            },
        )),
    )?;

    let progress_host = host.clone();
    workflow.set(
        "progress",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &progress_host, args, WorkflowRuntimeHost::progress)
            },
        )),
    )?;

    let checkpoint_host = host.clone();
    workflow.set(
        "checkpoint",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &checkpoint_host,
                    args,
                    WorkflowRuntimeHost::checkpoint,
                )
            },
        )),
    )?;

    let report_host = host.clone();
    workflow.set(
        "report",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &report_host, args, WorkflowRuntimeHost::report)
            },
        )),
    )?;

    let trace_host = host.clone();
    workflow.set(
        "trace",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &trace_host, args, WorkflowRuntimeHost::trace)
            },
        )),
    )?;

    let block_host = host.clone();
    workflow.set(
        "block",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &block_host, args, WorkflowRuntimeHost::block)
            },
        )),
    )?;

    let now_host = host.clone();
    workflow.set(
        "__now",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &now_host,
                    args,
                    WorkflowRuntimeHost::deterministic_now,
                )
            },
        )),
    )?;

    let random_host = host.clone();
    workflow.set(
        "__random",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(
                    &ctx,
                    &random_host,
                    args,
                    WorkflowRuntimeHost::deterministic_random,
                )
            },
        )),
    )?;

    let finish_host = host.clone();
    workflow.set(
        "finish",
        Func::from(MutFn::from(
            move |ctx: Ctx<'js>, args: JsValue<'js>| -> rquickjs::Result<JsValue<'js>> {
                host_call(&ctx, &finish_host, args, WorkflowRuntimeHost::finish)
            },
        )),
    )?;

    Ok(workflow)
}

fn install_workflow_js_helpers(ctx: &Ctx<'_>) -> Result<()> {
    ctx.eval::<(), _>(
        r#"
        const __hopeMaterializeMap = workflow.__materializeMap;
        const __hopeEnterMapItem = workflow.__enterMapItem;
        const __hopeExitMapItem = workflow.__exitMapItem;
        const __hopeBlock = workflow.block;
        const __hopePhaseStart = workflow.__phaseStart;
        const __hopePhaseComplete = workflow.__phaseComplete;
        const __hopePhaseFail = workflow.__phaseFail;
        const __hopeNow = workflow.__now;
        const __hopeRandom = workflow.__random;
        function __hopeErrorMessage(error) {
          if (error && typeof error.message === "string") return error.message;
          return String(error);
        }
        function __hopeRepairLoopArray(value, name) {
          if (value == null) return [];
          if (typeof value === "string") return value.trim().length > 0 ? [value.trim()] : [];
          if (!Array.isArray(value)) {
            throw new Error(`workflow.repairLoop ${name} must be a string or array`);
          }
          return value
            .map((item) => {
              if (typeof item !== "string") {
                throw new Error(`workflow.repairLoop ${name} entries must be strings`);
              }
              return item.trim();
            })
            .filter((item) => item.length > 0);
        }
        function __hopeRepairLoopClampAttempts(value) {
          const parsed = Number.isFinite(value) ? Math.trunc(value) : 2;
          return Math.max(1, Math.min(parsed, 5));
        }
        Object.defineProperty(workflow, "map", {
          configurable: false,
          enumerable: true,
          writable: false,
          value: async function map(label, list, fn) {
            if (typeof label !== "string" || label.trim().length === 0) {
              throw new Error("workflow.map requires a non-empty label");
            }
            if (!Array.isArray(list)) {
              throw new Error("workflow.map requires list to be an array");
            }
            if (typeof fn !== "function") {
              throw new Error("workflow.map requires callback function");
            }
            const materialized = await __hopeMaterializeMap({ label, items: list });
            const items = Array.isArray(materialized.items) ? materialized.items : [];
            const mapOpKey = materialized.opKey;
            if (typeof mapOpKey !== "string" || mapOpKey.length === 0) {
              throw new Error("workflow.map materialization did not return opKey");
            }
            const results = [];
            for (let i = 0; i < items.length; i++) {
              await __hopeEnterMapItem({ mapOpKey, index: i });
              try {
                results.push(await fn(items[i], i));
              } finally {
                await __hopeExitMapItem({ mapOpKey, index: i });
              }
            }
            return results;
          }
        });
        Object.defineProperty(workflow, "repairLoop", {
          configurable: false,
          enumerable: true,
          writable: false,
          value: async function repairLoop(options, fn) {
            if (options == null || typeof options !== "object" || Array.isArray(options)) {
              throw new Error("workflow.repairLoop requires an options object");
            }
            if (typeof fn !== "function") {
              throw new Error("workflow.repairLoop requires callback function");
            }

            const loopLabel = typeof options.label === "string" && options.label.trim().length > 0
              ? options.label.trim()
              : "repair-loop";
            const maxAttempts = __hopeRepairLoopClampAttempts(options.maxAttempts ?? options.max_attempts);
            const focusPaths = __hopeRepairLoopArray(options.focusPaths ?? options.focus_paths ?? options.files, "focusPaths");
            const commands = __hopeRepairLoopArray(options.validationCommands ?? options.validation_commands ?? options.commands, "validationCommands");
            const reviewProfiles = __hopeRepairLoopArray(options.reviewProfiles ?? options.review_profiles ?? options.profiles, "reviewProfiles");
            const reviewEnabled = options.review !== false;
            const verifyEnabled = options.verify !== false;
            const attempts = [];
            let previous = null;

            await workflow.trace({
              label: `${loopLabel}:start`,
              payload: {
                kind: "repair_loop_started",
                label: loopLabel,
                maxAttempts,
                focusPaths,
                validationCommands: commands,
                reviewProfiles,
                review: reviewEnabled,
                verify: verifyEnabled,
              },
            });

            for (let attempt = 1; attempt <= maxAttempts; attempt++) {
              const task = await workflow.task.create({
                title: `${loopLabel} repair attempt ${attempt}/${maxAttempts}`,
                label: `${loopLabel}:attempt-${attempt}`,
              });
              let repairResult = null;
              try {
                repairResult = await fn({
                  attempt,
                  maxAttempts,
                  label: loopLabel,
                  focusPaths,
                  previous,
                });
                await workflow.task.update({ task, status: "completed" });
              } catch (error) {
                await workflow.task.update({ task, status: "in_progress" });
                throw error;
              }

              const validation = commands.length > 0
                ? await workflow.validate({
                    commands,
                    reason: options.validationReason ?? options.validation_reason ?? `${loopLabel} repair attempt ${attempt}`,
                    label: `${loopLabel}:validate-${attempt}`,
                  })
                : null;
              const review = reviewEnabled
                ? await workflow.review({
                    focusPaths,
                    profiles: reviewProfiles,
                    label: `${loopLabel}:review-${attempt}`,
                  })
                : null;
              const verification = verifyEnabled
                ? await workflow.verify({
                    focusPaths,
                    maxCommands: options.maxVerificationCommands ?? options.max_verification_commands,
                    label: `${loopLabel}:verify-${attempt}`,
                  })
                : null;

              const validationOk = !validation || validation.ok === true;
              const reviewOk = !review || review.ok === true;
              const verificationOk = !verification || verification.ok === true;
              const ok = validationOk && reviewOk && verificationOk;
              const attemptResult = {
                attempt,
                ok,
                validationOk,
                reviewOk,
                verificationOk,
                blockingFindings: review ? review.blockingFindings : 0,
                validationSummary: validation ? validation.summary : null,
                reviewSummary: review ? review.summary : null,
                verificationSummary: verification ? verification.summary : null,
                commandCount: verification ? verification.commandCount : 0,
                repairResult,
              };
              attempts.push(attemptResult);
              previous = {
                attempt,
                validation,
                review,
                verification,
                result: attemptResult,
              };

              await workflow.trace({
                label: `${loopLabel}:attempt-${attempt}`,
                payload: {
                  kind: "repair_loop_attempt",
                  label: loopLabel,
                  ...attemptResult,
                },
              });

              if (ok) {
                const completed = {
                  kind: "repair_loop",
                  ok: true,
                  label: loopLabel,
                  attempts,
                  summary: `Repair loop ${loopLabel} completed after ${attempt} attempt(s).`,
                };
                await workflow.trace({
                  label: `${loopLabel}:completed`,
                  payload: {
                    kind: "repair_loop_completed",
                    label: loopLabel,
                    attempts: attempt,
                  },
                });
                return completed;
              }
            }

            const exhausted = {
              kind: "repair_loop",
              ok: false,
              label: loopLabel,
              attempts,
              summary: `Repair loop ${loopLabel} exhausted ${maxAttempts} attempt(s).`,
            };
            await workflow.trace({
              label: `${loopLabel}:exhausted`,
              payload: {
                kind: "repair_loop_exhausted",
                label: loopLabel,
                maxAttempts,
                attempts,
              },
            });
            await __hopeBlock({
              reason: "repair_loop_attempts_exhausted",
              label: loopLabel,
              payload: exhausted,
            });
          }
        });
        Object.defineProperty(workflow, "phase", {
          configurable: false,
          enumerable: true,
          writable: false,
          value: async function phase(options, fn) {
            if (options == null || typeof options !== "object" || Array.isArray(options)) {
              throw new Error("workflow.phase requires an options object");
            }
            if (typeof fn !== "function") {
              throw new Error("workflow.phase requires callback function");
            }
            const phase = await __hopePhaseStart(options);
            const phaseKey = phase && phase.phaseKey;
            try {
              const result = await fn(phase);
              await __hopePhaseComplete({ phaseKey, resultSummary: phase && phase.label ? `${phase.label} completed` : "phase completed" });
              return result;
            } catch (error) {
              await __hopePhaseFail({ phaseKey, error: __hopeErrorMessage(error) });
              throw error;
            }
          }
        });
        Object.defineProperty(workflow, "now", {
          configurable: false,
          enumerable: true,
          writable: false,
          value: function now() {
            return __hopeNow({});
          }
        });
        Object.defineProperty(workflow, "random", {
          configurable: false,
          enumerable: true,
          writable: false,
          value: function random(seed) {
            if (typeof seed !== "string" && typeof seed !== "number" && typeof seed !== "boolean") {
              throw new Error("workflow.random(seed) requires a string, number, or boolean seed");
            }
            return __hopeRandom({ seed: String(seed) });
          }
        });
        delete workflow.__materializeMap;
        delete workflow.__enterMapItem;
        delete workflow.__exitMapItem;
        delete workflow.__phaseStart;
        delete workflow.__phaseComplete;
        delete workflow.__phaseFail;
        delete workflow.__now;
        delete workflow.__random;
        "#,
    )
    .catch(ctx)
    .map_err(|err| anyhow!("install workflow JS helpers failed: {}", err))
}

fn install_runtime_guards(ctx: &Ctx<'_>) -> Result<()> {
    ctx.eval::<(), _>(
        r#"
        const __HopeNativeDate = Date;
        function __hopeDeterminismError(name) {
          throw new Error(`${name} is disabled in workflow runtime; use a workflow host API deterministic source instead`);
        }
        function HopeWorkflowDate(...args) {
          if (args.length === 0) {
            __hopeDeterminismError("new Date()");
          }
          if (new.target) {
            return Reflect.construct(__HopeNativeDate, args, new.target);
          }
          return __HopeNativeDate(...args);
        }
        Object.setPrototypeOf(HopeWorkflowDate, __HopeNativeDate);
        HopeWorkflowDate.prototype = __HopeNativeDate.prototype;
        HopeWorkflowDate.now = () => __hopeDeterminismError("Date.now()");
        HopeWorkflowDate.parse = __HopeNativeDate.parse;
        HopeWorkflowDate.UTC = __HopeNativeDate.UTC;
        globalThis.Date = HopeWorkflowDate;
        Math.random = () => __hopeDeterminismError("Math.random()");
        "#,
    )
    .catch(ctx)
    .map_err(|err| anyhow!("install workflow runtime guards failed: {}", err))
}

fn host_call<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<RefCell<WorkflowRuntimeHost>>,
    args: JsValue<'js>,
    f: fn(&mut WorkflowRuntimeHost, Value) -> Result<Value>,
) -> rquickjs::Result<JsValue<'js>> {
    let args = js_to_json(ctx, args)?;
    let output = host
        .try_borrow_mut()
        .map_err(|_| Exception::throw_message(ctx, "workflow host API called recursively"))?
        .call(args, f)
        .map_err(|err| js_error(ctx, err))?;
    json_to_js(ctx.clone(), output)
}

fn wait_all_host_call<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<RefCell<WorkflowRuntimeHost>>,
    handles: JsValue<'js>,
    options: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    return agent_handles_host_call(
        ctx,
        host,
        handles,
        options,
        "workflow.waitAll",
        WorkflowRuntimeHost::wait_all,
    );
}

fn agent_handles_host_call<'js>(
    ctx: &Ctx<'js>,
    host: &Rc<RefCell<WorkflowRuntimeHost>>,
    handles: JsValue<'js>,
    options: Opt<JsValue<'js>>,
    api: &str,
    f: fn(&mut WorkflowRuntimeHost, Value) -> Result<Value>,
) -> rquickjs::Result<JsValue<'js>> {
    let handles = js_to_json(ctx, handles)?;
    let options = options
        .0
        .filter(|value| !value.is_undefined() && !value.is_null())
        .map(|value| js_to_json(ctx, value))
        .transpose()?;
    let args =
        agent_handles_args_from_values(api, handles, options).map_err(|err| js_error(ctx, err))?;
    let output = host
        .try_borrow_mut()
        .map_err(|_| Exception::throw_message(ctx, "workflow host API called recursively"))?
        .call(args, f)
        .map_err(|err| js_error(ctx, err))?;
    json_to_js(ctx.clone(), output)
}

fn js_to_json<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> rquickjs::Result<Value> {
    rquickjs_serde::from_value_strict(value)
        .map_err(|err| Exception::throw_message(ctx, &format!("invalid workflow host args: {err}")))
}

fn json_to_js<'js>(ctx: Ctx<'js>, value: Value) -> rquickjs::Result<JsValue<'js>> {
    rquickjs_serde::to_value(ctx.clone(), value)
        .map_err(|err| Exception::throw_message(&ctx, &format!("serialize workflow result: {err}")))
}

fn js_error<'js>(ctx: &Ctx<'js>, err: anyhow::Error) -> rquickjs::Error {
    Exception::throw_message(ctx, &format!("{err:#}"))
}

fn finish_maybe_promise<'js>(
    ctx: Ctx<'js>,
    value: JsValue<'js>,
) -> rquickjs::CaughtResult<'js, JsValue<'js>> {
    if value.is_promise() {
        let promise = value.into_promise().expect("checked promise");
        promise.finish::<JsValue>().catch(&ctx)
    } else {
        Ok(value)
    }
}

fn prepare_script_for_eval(script: &str) -> String {
    let trimmed = script.trim();
    let prepared = if trimmed.contains("export default") {
        trimmed.replacen("export default", "globalThis.__hopeWorkflowMain =", 1)
    } else {
        let mut s = trimmed.to_string();
        if !s.contains("__hopeWorkflowMain") && s.contains("function main") {
            s.push_str("\nglobalThis.__hopeWorkflowMain = main;");
        }
        s
    };
    format!("\"use strict\";\n{prepared}")
}

fn script_timeout(run: &super::types::WorkflowRun) -> Duration {
    let secs = run
        .budget
        .get("maxScriptSecs")
        .or_else(|| run.budget.get("max_script_secs"))
        .or_else(|| run.budget.get("maxRuntimeSecs"))
        .or_else(|| run.budget.get("max_runtime_secs"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_SCRIPT_TIMEOUT_SECS)
        .clamp(1, MAX_SCRIPT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

fn output_token_budget_limit_from_budget(budget: &Value) -> Option<u64> {
    optional_u64_any(budget, &["maxOutputTokens", "max_output_tokens"]).filter(|limit| *limit > 0)
}

fn has_runtime_budget(run: &super::types::WorkflowRun) -> bool {
    optional_u64_any(
        &run.budget,
        &[
            "maxScriptSecs",
            "max_script_secs",
            "maxRuntimeSecs",
            "max_runtime_secs",
        ],
    )
    .is_some_and(|secs| secs > 0)
}

fn has_required_autonomous_budget(run: &super::types::WorkflowRun) -> bool {
    has_runtime_budget(run) && output_token_budget_limit_from_budget(&run.budget).is_some()
}

struct WorkflowRuntimeHost {
    db: Arc<SessionDB>,
    run_id: String,
    session_id: String,
    run_created_at: String,
    goal_id: Option<String>,
    execution_mode: String,
    session_context: WorkflowSessionContext,
    tokio_handle: TokioHandle,
    op_scopes: Vec<WorkflowOpScope>,
    finished_output: Option<Value>,
}

struct WorkflowOpScope {
    prefix: String,
    next_op_index: usize,
}

struct ExecutedWorkflowOp {
    op_key: String,
    output: Value,
    replayed: bool,
}

impl WorkflowRuntimeHost {
    fn new(
        db: Arc<SessionDB>,
        run_id: String,
        session_id: String,
        run_created_at: String,
        goal_id: Option<String>,
        execution_mode: String,
        session_context: WorkflowSessionContext,
        tokio_handle: TokioHandle,
    ) -> Self {
        Self {
            db,
            run_id,
            session_id,
            run_created_at,
            goal_id,
            execution_mode,
            session_context,
            tokio_handle,
            op_scopes: vec![WorkflowOpScope {
                prefix: "main".to_string(),
                next_op_index: 0,
            }],
            finished_output: None,
        }
    }

    fn call(
        &mut self,
        args: Value,
        f: fn(&mut WorkflowRuntimeHost, Value) -> Result<Value>,
    ) -> Result<Value> {
        f(self, args)
    }

    fn task_create(&mut self, args: Value) -> Result<Value> {
        let title = required_string(&args, "title")?;
        let label = optional_string(&args, "label");
        let input = compact_input(args);
        self.execute_op(
            "task.create",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let task = host
                    .db
                    .create_task(&host.session_id, &title, None)
                    .context("create workflow task")?;
                let tasks = host.db.list_tasks(&host.session_id).unwrap_or_default();
                crate::session::emit_task_snapshot(&host.session_id, &tasks);
                crate::hooks::fire_task_created(&host.session_id, &title, None, "");
                Ok(task_handle(&task, label.as_deref()))
            },
        )
    }

    fn task_update(&mut self, args: Value) -> Result<Value> {
        let id = task_id_from_args(&args)?;
        let status = optional_string(&args, "status")
            .map(|value| {
                TaskStatus::from_str(&value)
                    .ok_or_else(|| anyhow!("invalid task status '{}'", value))
            })
            .transpose()?;
        let content = optional_string(&args, "title").or_else(|| optional_string(&args, "content"));
        let active_form = optional_string(&args, "activeForm");
        if status.is_none() && content.is_none() && active_form.is_none() {
            return Err(anyhow!(
                "workflow.task.update requires status, title/content, or activeForm"
            ));
        }

        let input = compact_input(args);
        self.execute_op(
            "task.update",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let current = host.db.list_tasks(&host.session_id)?;
                if !current.iter().any(|task| task.id == id) {
                    return Err(anyhow!(
                        "task {} does not belong to workflow session {}",
                        id,
                        host.session_id
                    ));
                }
                let updated =
                    host.db
                        .update_task(id, status, content.as_deref(), active_form.as_deref())?;
                let tasks = host.db.list_tasks(&host.session_id).unwrap_or_default();
                crate::session::emit_task_snapshot(&host.session_id, &tasks);
                if status == Some(TaskStatus::Completed) {
                    crate::hooks::fire_task_completed(&host.session_id, id, &updated.content);
                }
                Ok(task_handle(&updated, None))
            },
        )
    }

    fn evidence_record(&mut self, args: Value) -> Result<Value> {
        let input = compact_input(args.clone());
        self.execute_op_with_key(
            "evidence.record",
            WorkflowEffectClass::NonIdempotent,
            input,
            move |host, op_key| host.record_domain_evidence(args, op_key),
        )
    }

    fn record_domain_evidence(&self, args: Value, op_key: &str) -> Result<Value> {
        let mut input: RecordDomainEvidenceInput =
            serde_json::from_value(args).context("parse workflow.evidence.record arguments")?;
        self.validate_domain_evidence_scope(&input)?;
        input.session_id = Some(self.session_id.clone());
        input.goal_id = self.goal_id.clone();
        input.project_id = self.session_context.project_id.clone();
        input.source_metadata =
            workflow_domain_evidence_source(input.source_metadata, self, op_key);
        let item = self
            .db
            .record_domain_evidence(input)
            .context("record workflow domain evidence")?;
        Ok(domain_evidence_output(&item))
    }

    fn validate_domain_evidence_scope(&self, input: &RecordDomainEvidenceInput) -> Result<()> {
        if let Some(session_id) = input.session_id.as_deref().map(str::trim) {
            if !session_id.is_empty() && session_id != self.session_id {
                bail!(
                    "workflow.evidence.record cannot target session {} from workflow session {}",
                    session_id,
                    self.session_id
                );
            }
        }
        if let Some(goal_id) = input.goal_id.as_deref().map(str::trim) {
            if goal_id.is_empty() {
                return Ok(());
            }
            match self.goal_id.as_deref() {
                Some(bound_goal_id) if bound_goal_id == goal_id => {}
                Some(bound_goal_id) => {
                    bail!(
                        "workflow.evidence.record cannot target goal {} from workflow bound to {}",
                        goal_id,
                        bound_goal_id
                    );
                }
                None => {
                    bail!(
                        "workflow.evidence.record cannot target goal {} because this workflow run is not goal-bound",
                        goal_id
                    );
                }
            }
        }
        if let Some(project_id) = input.project_id.as_deref().map(str::trim) {
            if !project_id.is_empty()
                && self.session_context.project_id.as_deref() != Some(project_id)
            {
                bail!(
                    "workflow.evidence.record cannot target project {} from workflow project {:?}",
                    project_id,
                    self.session_context.project_id
                );
            }
        }
        Ok(())
    }

    fn file_search(&mut self, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let root = optional_string(&args, "root")
            .or_else(|| self.session_context.working_dir.clone())
            .ok_or_else(|| anyhow!("workflow.fileSearch requires a session working directory"))?;
        let input = json!({
            "query": query.clone(),
            "limit": limit,
            "root": root.clone(),
            "label": optional_string(&args, "label"),
        });
        self.execute_op("fileSearch", WorkflowEffectClass::Pure, input, |_host| {
            let response = crate::filesystem::search_files(&root, &query, limit)
                .context("workflow.fileSearch failed")?;
            serde_json::to_value(response).context("serialize fileSearch response")
        })
    }

    fn tool(&mut self, args: Value) -> Result<Value> {
        let name = required_string(&args, "name")?;
        let tool_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let label = optional_string(&args, "label");
        let effect_class = tool_effect_class(&name);
        let op_type = format!("tool:{name}");
        let input = json!({
            "name": name.clone(),
            "args": tool_args.clone(),
            "label": label,
        });
        if workflow_tool_uses_async_child(&name, &tool_args) {
            let child_handle = JobManager::new_job_id();
            let recover_name = name.clone();
            let run_name = name.clone();
            let run_tool_args = tool_args.clone();
            return self.execute_op_with_child_handle(
                &op_type,
                effect_class,
                input,
                child_handle,
                move |host, child_handle| {
                    host.recover_async_tool_child(&recover_name, child_handle)
                },
                move |host, child_handle| {
                    host.dispatch_async_tool_with_child(&run_name, &run_tool_args, child_handle)
                },
            );
        }
        self.execute_op(&op_type, effect_class, input, |host| {
            host.dispatch_tool(&name, &tool_args).map(Value::String)
        })
    }

    fn recover_async_tool_child(&self, name: &str, child_handle: &str) -> Result<Option<Value>> {
        if JobManager::get(child_handle)?.is_none() {
            return Ok(None);
        }
        Ok(Some(Value::String(async_tool_started_output(
            name,
            child_handle,
        ))))
    }

    fn dispatch_async_tool_with_child(
        &self,
        name: &str,
        args: &Value,
        child_handle: &str,
    ) -> Result<Value> {
        let mut ctx = self.tool_exec_context();
        ctx.async_job_id_override = Some(child_handle.to_string());
        let output = self.dispatch_tool_with_context(name, args, ctx)?;
        let parsed = parse_tool_json_output(&output, "workflow.tool async child")?;
        let job_id = parsed
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("workflow.tool async child output missing job_id"))?;
        if job_id != child_handle {
            return Err(anyhow!(
                "workflow.tool({name}) returned job_id {} but expected preallocated child handle {}",
                job_id,
                child_handle
            ));
        }
        Ok(Value::String(output))
    }

    fn spawn_agent(&mut self, args: Value) -> Result<Value> {
        let tool_args = spawn_agent_tool_args(&args)?;
        let label = optional_string(&args, "label");
        let task = optional_string(&args, "task");
        let requested_inject_policy = optional_string(&args, "injectPolicy")
            .or_else(|| optional_string(&args, "inject_policy"));
        let inject_policy = requested_inject_policy
            .clone()
            .unwrap_or_else(|| "none".to_string());
        if !matches!(inject_policy.as_str(), "none" | "checkpoint" | "final") {
            return Err(anyhow!(
                "workflow.spawnAgent injectPolicy must be none, checkpoint, or final"
            ));
        }
        let requested_result_mode =
            optional_string(&args, "resultMode").or_else(|| optional_string(&args, "result_mode"));
        let result_mode = requested_result_mode
            .clone()
            .unwrap_or_else(|| "summary".to_string());
        if !matches!(result_mode.as_str(), "summary" | "full") {
            return Err(anyhow!(
                "workflow.spawnAgent resultMode must be summary or full"
            ));
        }
        let mut input = json!({
            "args": tool_args.clone(),
            "label": label.clone(),
        });
        if let Value::Object(ref mut map) = input {
            if let Some(policy) = requested_inject_policy {
                map.insert("injectPolicy".to_string(), Value::String(policy));
            }
            if let Some(mode) = requested_result_mode {
                map.insert("resultMode".to_string(), Value::String(mode));
            }
        }
        let child_handle = uuid::Uuid::new_v4().to_string();
        self.execute_op_with_child_handle(
            "spawnAgent",
            WorkflowEffectClass::NonIdempotent,
            input,
            child_handle,
            |host, child_handle| {
                host.recover_spawn_agent_child(
                    child_handle,
                    label.as_deref(),
                    task.as_deref(),
                    &inject_policy,
                    &result_mode,
                )
            },
            |host, child_handle| {
                host.ensure_output_token_budget("spawnAgent")?;
                let mut dispatch_args = tool_args.clone();
                inject_workflow_preallocated_run_id(&mut dispatch_args, child_handle)?;
                let output = host.dispatch_tool(tools::TOOL_SUBAGENT, &dispatch_args)?;
                let parsed = parse_tool_json_output(&output, "workflow.spawnAgent")?;
                let run_id = parsed
                    .get("run_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("workflow.spawnAgent subagent output missing run_id"))?;
                if run_id != child_handle {
                    return Err(anyhow!(
                        "workflow.spawnAgent returned run_id {} but expected preallocated child handle {}",
                        run_id,
                        child_handle
                    ));
                }
                let run = host
                    .db
                    .get_subagent_run(&run_id)?
                    .ok_or_else(|| anyhow!("workflow.spawnAgent run {run_id} was not persisted"))?;
                Ok(subagent_run_handle(
                    &run,
                    label.as_deref(),
                    task.as_deref(),
                    &inject_policy,
                    &result_mode,
                ))
            },
        )
    }

    fn recover_spawn_agent_child(
        &self,
        child_handle: &str,
        label: Option<&str>,
        task: Option<&str>,
        inject_policy: &str,
        result_mode: &str,
    ) -> Result<Option<Value>> {
        let Some(run) = self.db.get_subagent_run(child_handle)? else {
            return Ok(None);
        };
        Ok(Some(subagent_run_handle(
            &run,
            label,
            task,
            inject_policy,
            result_mode,
        )))
    }

    fn wait_all(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.waitAll")?;
        ensure_workflow_owned_agent_run_ids(&self.db, &self.run_id, &run_ids, "workflow.waitAll")?;
        let tool_args = wait_all_tool_args(&args)?;
        let input = compact_input(args);
        let executed =
            self.execute_op_tracked("waitAll", WorkflowEffectClass::Pure, input, |host| {
                let output = host.dispatch_tool(tools::TOOL_SUBAGENT, &tool_args)?;
                let mut parsed = parse_tool_json_output(&output, "workflow.waitAll")?;
                normalize_wait_all_response(&mut parsed);
                Ok(parsed)
            })?;
        if !executed.replayed {
            self.record_output_token_budget_usage("waitAll")?;
        }
        self.record_consumed_agent_results("waitAll", &executed.output)?;
        Ok(executed.output)
    }

    fn wait_any(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.waitAny")?;
        ensure_workflow_owned_agent_run_ids(&self.db, &self.run_id, &run_ids, "workflow.waitAny")?;
        let min = optional_u64_any(&args, &["min"])
            .unwrap_or(1)
            .clamp(1, run_ids.len() as u64) as usize;
        let timeout_secs = optional_u64_any(&args, &["timeout", "waitTimeout", "wait_timeout"])
            .unwrap_or(120)
            .min(600);
        let input = compact_input(args);
        self.execute_op("waitAny", WorkflowEffectClass::Pure, input, |host| {
            let deadline = Instant::now() + Duration::from_secs(timeout_secs);
            loop {
                let mut snapshot = workflow_agent_status_snapshot(&host.db, &run_ids)?;
                let terminal = snapshot
                    .get("terminal")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if terminal >= min || timeout_secs == 0 || Instant::now() >= deadline {
                    if let Value::Object(ref mut map) = snapshot {
                        map.insert("min".to_string(), json!(min));
                        map.insert("timedOut".to_string(), json!(terminal < min));
                    }
                    return Ok(snapshot);
                }
                let state = host
                    .db
                    .get_workflow_run(&host.run_id)?
                    .map(|run| run.state)
                    .unwrap_or(WorkflowRunState::Cancelled);
                if matches!(
                    state,
                    WorkflowRunState::Cancelled
                        | WorkflowRunState::Paused
                        | WorkflowRunState::Blocked
                ) {
                    return Err(anyhow!(
                        "workflow.waitAny stopped because run state is {}",
                        state.as_str()
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
    }

    fn agent_status(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.agentStatus")?;
        ensure_workflow_owned_agent_run_ids(
            &self.db,
            &self.run_id,
            &run_ids,
            "workflow.agentStatus",
        )?;
        let input = compact_input(args);
        self.execute_op("agentStatus", WorkflowEffectClass::Pure, input, |host| {
            workflow_agent_status_snapshot(&host.db, &run_ids)
        })
    }

    fn agent_result(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.agentResult")?;
        if run_ids.len() != 1 {
            return Err(anyhow!(
                "workflow.agentResult requires exactly one child handle"
            ));
        }
        let run_id = run_ids[0].clone();
        ensure_workflow_owned_agent_run_ids(
            &self.db,
            &self.run_id,
            &run_ids,
            "workflow.agentResult",
        )?;
        let mode = optional_string(&args, "mode")
            .or_else(|| optional_string(&args, "resultMode"))
            .or_else(|| optional_string(&args, "result_mode"))
            .unwrap_or_else(|| "summary".to_string());
        if !matches!(mode.as_str(), "summary" | "full") {
            return Err(anyhow!("workflow.agentResult mode must be summary or full"));
        }
        let input = compact_input(args);
        let executed =
            self.execute_op_tracked("agentResult", WorkflowEffectClass::Pure, input, |host| {
                host.ensure_output_token_budget("agentResult")?;
                let run = host
                    .db
                    .get_subagent_run(&run_id)?
                    .ok_or_else(|| anyhow!("sub-agent run {run_id} not found"))?;
                let mut output = workflow_agent_run_status(&run);
                if let Value::Object(ref mut map) = output {
                    map.insert("mode".to_string(), Value::String(mode.clone()));
                    if let Some(result) = run.result.as_deref() {
                        let result = if mode == "full" {
                            result.to_string()
                        } else {
                            truncate_chars(result, 2000)
                        };
                        map.insert("result".to_string(), Value::String(result));
                    }
                }
                Ok(output)
            })?;
        if !executed.replayed {
            self.record_output_token_budget_usage("agentResult")?;
        }
        self.record_consumed_agent_results("agentResult", &executed.output)?;
        Ok(executed.output)
    }

    fn agent_steer(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.agentSteer")?;
        if run_ids.len() != 1 {
            return Err(anyhow!(
                "workflow.agentSteer requires exactly one child handle"
            ));
        }
        let message = required_string(&args, "message")?;
        let run_id = run_ids[0].clone();
        ensure_workflow_owned_agent_run_ids(
            &self.db,
            &self.run_id,
            &run_ids,
            "workflow.agentSteer",
        )?;
        let input = compact_input(args);
        self.execute_op(
            "agentSteer",
            WorkflowEffectClass::NonIdempotent,
            input,
            |host| {
                let output = host.dispatch_tool(
                    tools::TOOL_SUBAGENT,
                    &json!({
                        "action": "steer",
                        "run_id": run_id,
                        "message": message,
                    }),
                )?;
                parse_tool_json_output(&output, "workflow.agentSteer")
            },
        )
    }

    fn cancel_agent(&mut self, args: Value) -> Result<Value> {
        let run_ids = workflow_agent_run_ids(&args, "workflow.cancelAgent")?;
        ensure_workflow_owned_agent_run_ids(
            &self.db,
            &self.run_id,
            &run_ids,
            "workflow.cancelAgent",
        )?;
        let reason = optional_string(&args, "reason");
        let input = compact_input(args);
        self.execute_op(
            "cancelAgent",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let mut results = Vec::with_capacity(run_ids.len());
                for run_id in &run_ids {
                    let output = host.dispatch_tool(
                        tools::TOOL_SUBAGENT,
                        &json!({
                            "action": "kill",
                            "run_id": run_id,
                        }),
                    );
                    match output {
                        Ok(output) => results.push(
                            parse_tool_json_output(&output, "workflow.cancelAgent").unwrap_or_else(
                                |_| json!({ "runId": run_id, "status": "cancelled" }),
                            ),
                        ),
                        Err(err) => results.push(json!({
                            "runId": run_id,
                            "status": "error",
                            "error": err.to_string(),
                        })),
                    }
                }
                let _ = host.db.append_workflow_event(
                    &host.run_id,
                    "workflow_agent_cancel_requested",
                    json!({
                        "childRunIds": run_ids,
                        "reason": reason,
                        "results": results,
                    }),
                )?;
                Ok(json!({ "results": results }))
            },
        )
    }

    fn record_consumed_agent_results(&self, api: &str, output: &Value) -> Result<()> {
        if api == "waitAll" && !wait_all_output_consumes_results(output) {
            return Ok(());
        }
        let mut run_ids = consumed_agent_result_ids(output);
        if matches!(api, "waitAll" | "agentResult" | "finish") {
            for run_id in terminal_agent_result_ids(output) {
                if !run_ids.iter().any(|existing| existing == &run_id) {
                    run_ids.push(run_id);
                }
            }
        }
        let mut fresh_run_ids = Vec::with_capacity(run_ids.len());
        for run_id in run_ids {
            if !self
                .db
                .workflow_agent_result_handled(&self.run_id, &run_id)?
            {
                fresh_run_ids.push(run_id);
            }
        }
        let run_ids = fresh_run_ids;
        if run_ids.is_empty() {
            return Ok(());
        }
        let _ = self.db.append_workflow_event(
            &self.run_id,
            "workflow_agent_result_consumed",
            json!({
                "api": api,
                "childRunIds": run_ids,
            }),
        )?;
        for run_id in &run_ids {
            crate::subagent::mark_run_fetched(run_id);
            for injection_run_id in self
                .db
                .workflow_agent_checkpoint_injection_run_ids(&self.run_id, run_id)?
            {
                crate::subagent::mark_run_fetched(&injection_run_id);
                let source_event_seq = injection_run_id
                    .rsplit(':')
                    .next()
                    .and_then(|value| value.parse::<i64>().ok());
                let Some(source_event_seq) = source_event_seq else {
                    continue;
                };
                if self.db.workflow_milestone_injection_settled(
                    &self.run_id,
                    "workflow_checkpoint",
                    source_event_seq,
                )? {
                    continue;
                }
                let _ = self.db.append_workflow_event(
                    &self.run_id,
                    "workflow_milestone_injection_suppressed",
                    json!({
                        "sourceEventType": "workflow_checkpoint",
                        "sourceEventSeq": source_event_seq,
                        "injectionRunId": injection_run_id,
                        "childRunId": run_id,
                        "reason": "explicit_agent_result_consumption",
                    }),
                )?;
            }
        }
        Ok(())
    }

    fn ensure_output_token_budget(&self, api: &str) -> Result<()> {
        let Some(limit) = self.output_token_budget_limit()? else {
            return Ok(());
        };
        let spent = self.output_tokens_spent()?;
        if spent < limit {
            return Ok(());
        }

        let _ = self.db.append_workflow_event(
            &self.run_id,
            BUDGET_USAGE_EVENT,
            json!({
                "api": api,
                "spentOutputTokens": spent,
                "maxOutputTokens": limit,
                "exhausted": true,
                "reason": BUDGET_EXHAUSTED_REASON,
            }),
        )?;
        let _ = self.db.transition_workflow_run(
            &self.run_id,
            WorkflowRunState::Blocked,
            Some(BUDGET_EXHAUSTED_REASON),
        )?;
        Err(anyhow!(
            "workflow output token budget exhausted before {api}: spent {spent}, limit {limit}"
        ))
    }

    fn record_output_token_budget_usage(&self, api: &str) -> Result<()> {
        let Some(limit) = self.output_token_budget_limit()? else {
            return Ok(());
        };
        let spent = self.output_tokens_spent()?;
        let _ = self.db.append_workflow_event(
            &self.run_id,
            BUDGET_USAGE_EVENT,
            json!({
                "api": api,
                "spentOutputTokens": spent,
                "maxOutputTokens": limit,
                "exhausted": spent >= limit,
            }),
        )?;
        Ok(())
    }

    fn output_token_budget_limit(&self) -> Result<Option<u64>> {
        let Some(run) = self.db.get_workflow_run(&self.run_id)? else {
            return Ok(None);
        };
        Ok(output_token_budget_limit_from_budget(&run.budget))
    }

    fn output_tokens_spent(&self) -> Result<u64> {
        let mut spent = 0u64;
        for op in self.db.list_workflow_ops(&self.run_id)? {
            if op.op_type != "spawnAgent" {
                continue;
            }
            let Some(child_handle) = op.child_handle else {
                continue;
            };
            if let Some(run) = self.db.get_subagent_run(&child_handle)? {
                spent = spent.saturating_add(run.output_tokens.unwrap_or(0));
            }
        }
        Ok(spent)
    }

    fn materialize_map(&mut self, args: Value) -> Result<Value> {
        let label = required_string(&args, "label")?;
        let items = args
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow!("workflow.map requires items array"))?;
        let input = json!({
            "label": label,
            "items": items,
        });
        self.execute_op_with_key(
            "map",
            WorkflowEffectClass::Pure,
            input.clone(),
            |_host, op_key| {
                let mut output = input;
                if let Value::Object(map) = &mut output {
                    map.insert("opKey".to_string(), Value::String(op_key.to_string()));
                }
                Ok(output)
            },
        )
    }

    fn enter_map_item(&mut self, args: Value) -> Result<Value> {
        let map_op_key = required_string(&args, "mapOpKey")?;
        let index = args
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("workflow.map item scope requires index"))?;
        self.op_scopes.push(WorkflowOpScope {
            prefix: format!("{map_op_key}/item#{index}"),
            next_op_index: 0,
        });
        Ok(json!({ "ok": true }))
    }

    fn exit_map_item(&mut self, args: Value) -> Result<Value> {
        let map_op_key = required_string(&args, "mapOpKey")?;
        let index = args
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("workflow.map item scope requires index"))?;
        let expected = format!("{map_op_key}/item#{index}");
        let Some(scope) = self.op_scopes.pop() else {
            return Err(anyhow!("workflow.map item scope stack is empty"));
        };
        if self.op_scopes.is_empty() {
            self.op_scopes.push(scope);
            return Err(anyhow!("workflow.map cannot exit root op scope"));
        }
        if scope.prefix != expected {
            self.op_scopes.push(scope);
            return Err(anyhow!(
                "workflow.map item scope mismatch: expected {}, got {}",
                expected,
                self.op_scopes
                    .last()
                    .map(|scope| scope.prefix.as_str())
                    .unwrap_or("<empty>")
            ));
        }
        Ok(json!({ "ok": true }))
    }

    fn read(&mut self, args: Value) -> Result<Value> {
        let tool_args = args.clone();
        self.execute_op("read", WorkflowEffectClass::Pure, args, |host| {
            host.dispatch_tool(tools::TOOL_READ, &tool_args)
                .map(Value::String)
        })
    }

    fn grep(&mut self, args: Value) -> Result<Value> {
        let tool_args = args.clone();
        self.execute_op("grep", WorkflowEffectClass::Pure, args, |host| {
            host.dispatch_tool(tools::TOOL_GREP, &tool_args)
                .map(Value::String)
        })
    }

    fn validate(&mut self, args: Value) -> Result<Value> {
        let commands = validation_commands_from_args(&args)?;
        let reason = optional_string(&args, "reason");
        let input = compact_input(args);
        let child_handle = validation_child_handle_for_commands(&commands)?;
        let recover_reason = reason.clone();
        let run_reason = reason.clone();
        let executed = self.execute_op_with_child_handle_tracked(
            "validate",
            WorkflowEffectClass::NonIdempotent,
            input,
            child_handle,
            move |host, child_handle| {
                host.recover_validate_child(child_handle, recover_reason.as_deref())
            },
            move |host, child_handle| host.run_validate_child(child_handle, run_reason.as_deref()),
        )?;
        if !executed.replayed {
            self.record_guarded_repair_validation(&executed.op_key, &executed.output)?;
        }
        Ok(executed.output)
    }

    fn review(&mut self, args: Value) -> Result<Value> {
        let input = compact_input(args.clone());
        self.execute_op("review", WorkflowEffectClass::Idempotent, input, |host| {
            host.run_review(args)
        })
    }

    fn run_review(&self, args: Value) -> Result<Value> {
        let input = RunReviewInput {
            scope: Some(optional_string(&args, "scope").unwrap_or_else(|| "local".to_string())),
            base_ref: optional_string(&args, "baseRef")
                .or_else(|| optional_string(&args, "base_ref")),
            goal_id: workflow_goal_id_from_args(&args, self.goal_id.clone()),
            profiles: string_array_arg(&args, "profiles")?,
            focus_paths: focus_paths_from_args(&args)?,
            ide_context: ide_context_from_args(&args)?,
        };
        let snapshot = self
            .tokio_handle
            .block_on(review::run_review_for_session(
                self.db.clone(),
                self.session_id.clone(),
                input,
            ))
            .context("workflow.review failed")?;
        Ok(workflow_review_output(snapshot))
    }

    fn verify(&mut self, args: Value) -> Result<Value> {
        let input = compact_input(args.clone());
        self.execute_op("verify", WorkflowEffectClass::Idempotent, input, |host| {
            host.plan_verification(args)
        })
    }

    fn plan_verification(&self, args: Value) -> Result<Value> {
        let input = PlanVerificationInput {
            scope: Some(optional_string(&args, "scope").unwrap_or_else(|| "local".to_string())),
            goal_id: workflow_goal_id_from_args(&args, self.goal_id.clone()),
            max_commands: optional_u64_any(&args, &["maxCommands", "max_commands"])
                .map(|value| value as usize),
            focus_paths: focus_paths_from_args(&args)?,
        };
        let snapshot = self
            .tokio_handle
            .block_on(verification::plan_verification_for_session(
                self.db.clone(),
                self.session_id.clone(),
                input,
            ))
            .context("workflow.verify failed")?;
        Ok(workflow_verify_output(snapshot))
    }

    fn recover_validate_child(
        &self,
        child_handle: &str,
        reason: Option<&str>,
    ) -> Result<Option<Value>> {
        Ok(Some(self.run_validate_child(child_handle, reason)?))
    }

    fn run_validate_child(&self, child_handle: &str, reason: Option<&str>) -> Result<Value> {
        let child = parse_validation_child_handle(child_handle)?;
        let mut results = Vec::with_capacity(child.jobs.len());
        for job_ref in child.jobs {
            let job = match JobManager::get(&job_ref.job_id)? {
                Some(job) => job,
                None => {
                    self.spawn_validation_exec_job(&job_ref)?;
                    self.wait_for_validation_job(&job_ref.job_id)?
                }
            };
            let job = if job.status.is_terminal() {
                job
            } else {
                self.wait_for_validation_job(&job_ref.job_id)?
            };
            results.push(validation_result_from_job(job_ref, &job)?);
        }
        let failed = results
            .iter()
            .filter(|result| !result.get("ok").and_then(Value::as_bool).unwrap_or(false))
            .count();
        let ok = failed == 0;
        let summary = if ok {
            format!("{} validation command(s) passed", results.len())
        } else {
            format!("{failed}/{} validation command(s) failed", results.len())
        };
        Ok(json!({
            "ok": ok,
            "summary": summary,
            "reason": reason,
            "results": results,
        }))
    }

    fn spawn_validation_exec_job(&self, job_ref: &ValidationJobRef) -> Result<()> {
        let mut ctx = self.tool_exec_context();
        ctx.async_tool_policy = crate::agent_config::AsyncToolPolicy::NeverBackground;
        ctx.suppress_completion_injection = true;
        let exec_args = job_ref.exec_args();
        let session_id = self.session_id.clone();
        let default_path = ctx.default_cwd();
        JobManager::spawn_tool_with_id(
            tools::TOOL_EXEC,
            exec_args,
            ctx,
            JobOrigin::Explicit,
            job_ref.job_id.clone(),
        )
        .with_context(|| {
            format!(
                "workflow.validate failed to spawn async exec job {} (session={session_id}, cwd={default_path}, command={})",
                job_ref.job_id, job_ref.command
            )
        })?;
        Ok(())
    }

    fn wait_for_validation_job(&self, job_id: &str) -> Result<BackgroundJob> {
        let session_id = self.session_id.clone();
        self.tokio_handle
            .block_on(JobManager::wait_for_terminal(job_id, None))?
            .ok_or_else(|| {
                anyhow!(
                    "workflow.validate child job {} disappeared (session={})",
                    job_id,
                    session_id
                )
            })
    }

    fn ask_user(&mut self, args: Value) -> Result<Value> {
        let tool_args = ask_user_tool_args(&args)?;
        let input = compact_input(args);
        self.execute_op(
            "askUser",
            WorkflowEffectClass::NonIdempotent,
            input,
            |host| host.dispatch_ask_user(&tool_args),
        )
    }

    fn diff(&mut self, args: Value) -> Result<Value> {
        let input = compact_input(args);
        self.execute_op("diff", WorkflowEffectClass::Pure, input, |host| {
            let root = host
                .session_context
                .working_dir
                .as_deref()
                .ok_or_else(|| anyhow!("workflow.diff requires a session working directory"))?;
            let diff = crate::session::load_git_diff_for_root(std::path::Path::new(root))
                .context("workflow.diff failed")?;
            serde_json::to_value(diff).context("serialize workflow.diff response")
        })
    }

    fn record_guarded_repair_validation(&self, op_key: &str, output: &Value) -> Result<()> {
        if !self.repair_guard_enabled() || self.repair_event_exists_for_op(op_key)? {
            return Ok(());
        }

        let ok = output.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let summary = output
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let results = output
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let failed = results
            .iter()
            .filter(|result| !result.get("ok").and_then(Value::as_bool).unwrap_or(false))
            .count();

        if ok {
            let _ = self.db.append_workflow_event(
                &self.run_id,
                REPAIR_VALIDATION_PASSED_EVENT,
                json!({
                    "opKey": op_key,
                    "summary": summary,
                    "total": results.len(),
                }),
            )?;
            return Ok(());
        }

        let fingerprint = validation_failure_fingerprint(&results)?;
        let (diff_hash, diff_error) = self.current_diff_hash();
        let previous = self.previous_repair_validation_event(op_key)?;
        let previous_failed = previous.as_ref().and_then(|event| {
            (event.event_type == REPAIR_VALIDATION_FAILED_EVENT).then_some(event)
        });
        let same_validation = previous_failed.is_some_and(|event| {
            event.payload.get("fingerprint").and_then(Value::as_str) == Some(fingerprint.as_str())
        });
        let no_effective_diff = diff_hash.as_ref().is_some_and(|hash| {
            previous_failed.is_some_and(|event| {
                event.payload.get("diffHash").and_then(Value::as_str) == Some(hash.as_str())
            })
        });
        let stop_reason = if same_validation {
            Some(REPAIR_SAME_VALIDATION_REASON)
        } else if no_effective_diff {
            Some(REPAIR_NO_EFFECTIVE_DIFF_REASON)
        } else {
            None
        };

        let _ = self.db.append_workflow_event(
            &self.run_id,
            REPAIR_VALIDATION_FAILED_EVENT,
            json!({
                "opKey": op_key,
                "summary": summary,
                "failed": failed,
                "total": results.len(),
                "fingerprint": fingerprint,
                "diffHash": diff_hash,
                "diffError": diff_error,
                "stopReason": stop_reason,
            }),
        )?;

        if let Some(reason) = stop_reason {
            let _ = self.db.transition_workflow_run(
                &self.run_id,
                WorkflowRunState::Blocked,
                Some(reason),
            )?;
            return Err(anyhow!(
                "workflow guarded repair stopped after validation failure: {reason}"
            ));
        }

        Ok(())
    }

    fn repair_guard_enabled(&self) -> bool {
        !matches!(self.execution_mode.as_str(), "off")
    }

    fn repair_event_exists_for_op(&self, op_key: &str) -> Result<bool> {
        Ok(self
            .db
            .list_workflow_events(&self.run_id, 500)?
            .iter()
            .any(|event| {
                matches!(
                    event.event_type.as_str(),
                    REPAIR_VALIDATION_FAILED_EVENT | REPAIR_VALIDATION_PASSED_EVENT
                ) && event.payload.get("opKey").and_then(Value::as_str) == Some(op_key)
            }))
    }

    fn previous_repair_validation_event(
        &self,
        op_key: &str,
    ) -> Result<Option<super::types::WorkflowEvent>> {
        Ok(self
            .db
            .list_workflow_events(&self.run_id, 500)?
            .into_iter()
            .rev()
            .find(|event| {
                matches!(
                    event.event_type.as_str(),
                    REPAIR_VALIDATION_FAILED_EVENT | REPAIR_VALIDATION_PASSED_EVENT
                ) && event.payload.get("opKey").and_then(Value::as_str) != Some(op_key)
            }))
    }

    fn current_diff_hash(&self) -> (Option<String>, Option<String>) {
        let Some(root) = self.session_context.working_dir.as_deref() else {
            return (None, Some("session has no working directory".to_string()));
        };
        match crate::session::load_git_diff_for_root(std::path::Path::new(root))
            .and_then(|diff| stable_value_hash(&serde_json::to_value(diff)?))
        {
            Ok(hash) => (Some(hash), None),
            Err(err) => (None, Some(err.to_string())),
        }
    }

    fn phase_start(&mut self, args: Value) -> Result<Value> {
        let name = optional_string(&args, "name")
            .or_else(|| optional_string(&args, "label"))
            .unwrap_or_else(|| "phase".to_string());
        let label = optional_string(&args, "label").unwrap_or_else(|| name.clone());
        let expected = optional_string(&args, "expected");
        let criteria_ids = args
            .get("criteriaIds")
            .or_else(|| args.get("criteria_ids"))
            .cloned()
            .unwrap_or_else(|| json!([]));
        let inject_policy = optional_string(&args, "injectPolicy")
            .or_else(|| optional_string(&args, "inject_policy"))
            .unwrap_or_else(|| "auto".to_string());
        let input = compact_input(args);
        self.execute_op_with_key(
            "phase.start",
            WorkflowEffectClass::Idempotent,
            input,
            |host, op_key| {
                let event = host.db.append_workflow_event(
                    &host.run_id,
                    "workflow_phase_started",
                    json!({
                        "phaseKey": op_key,
                        "name": name,
                        "label": label,
                        "expected": expected,
                        "criteriaIds": criteria_ids,
                        "injectPolicy": inject_policy,
                    }),
                )?;
                Ok(json!({
                    "phaseKey": op_key,
                    "name": name,
                    "label": label,
                    "eventSeq": event.seq,
                }))
            },
        )
    }

    fn phase_complete(&mut self, args: Value) -> Result<Value> {
        let phase_key = required_string(&args, "phaseKey")?;
        let result_summary =
            optional_string(&args, "resultSummary").or_else(|| optional_string(&args, "summary"));
        let input = compact_input(args);
        self.execute_op(
            "phase.complete",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let event = host.db.append_workflow_event(
                    &host.run_id,
                    "workflow_phase_completed",
                    json!({
                        "phaseKey": phase_key,
                        "summary": result_summary,
                    }),
                )?;
                Ok(json!({ "phaseKey": phase_key, "eventSeq": event.seq }))
            },
        )
    }

    fn phase_fail(&mut self, args: Value) -> Result<Value> {
        let phase_key = required_string(&args, "phaseKey")?;
        let error = optional_string(&args, "error").unwrap_or_else(|| "phase failed".to_string());
        let input = compact_input(args);
        self.execute_op(
            "phase.fail",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let event = host.db.append_workflow_event(
                    &host.run_id,
                    "workflow_phase_failed",
                    json!({
                        "phaseKey": phase_key,
                        "error": error,
                    }),
                )?;
                Ok(json!({ "phaseKey": phase_key, "eventSeq": event.seq }))
            },
        )
    }

    fn progress(&mut self, args: Value) -> Result<Value> {
        let message = required_string(&args, "message")?;
        let phase_key =
            optional_string(&args, "phaseKey").or_else(|| optional_string(&args, "phase"));
        let percent = args
            .get("percent")
            .and_then(Value::as_f64)
            .map(|value| value.clamp(0.0, 100.0));
        let counters = args.get("counters").cloned().unwrap_or_else(|| json!({}));
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let importance = optional_string(&args, "importance").unwrap_or_else(|| "low".to_string());
        let input = compact_input(args);
        self.execute_op("progress", WorkflowEffectClass::Pure, input, |host| {
            let event = host.db.append_workflow_event(
                &host.run_id,
                "workflow_progress",
                json!({
                    "phaseKey": phase_key,
                    "message": message,
                    "percent": percent,
                    "counters": counters,
                    "payload": payload,
                    "importance": importance,
                }),
            )?;
            Ok(json!({ "eventSeq": event.seq }))
        })
    }

    fn checkpoint(&mut self, args: Value) -> Result<Value> {
        let title = required_string(&args, "title")?;
        let summary = required_string(&args, "summary")?;
        let phase_key =
            optional_string(&args, "phaseKey").or_else(|| optional_string(&args, "phase"));
        let importance =
            optional_string(&args, "importance").unwrap_or_else(|| "normal".to_string());
        let inject_policy = optional_string(&args, "inject")
            .or_else(|| optional_string(&args, "injectPolicy"))
            .or_else(|| optional_string(&args, "inject_policy"))
            .unwrap_or_else(|| "auto".to_string());
        let findings = args.get("findings").cloned().unwrap_or_else(|| json!([]));
        let evidence = args.get("evidence").cloned().unwrap_or_else(|| json!([]));
        let decisions = args.get("decisions").cloned().unwrap_or_else(|| json!([]));
        let next = args.get("next").cloned().unwrap_or(Value::Null);
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let input = compact_input(args);
        self.execute_op(
            "checkpoint",
            WorkflowEffectClass::Idempotent,
            input,
            |host| {
                let event_payload = json!({
                    "phaseKey": phase_key,
                    "title": title,
                    "summary": summary,
                    "importance": importance,
                    "injectPolicy": inject_policy,
                    "findings": findings,
                    "evidence": evidence,
                    "decisions": decisions,
                    "next": next,
                    "payload": payload,
                });
                let event = host.db.append_workflow_event(
                    &host.run_id,
                    "workflow_checkpoint",
                    event_payload.clone(),
                )?;
                if should_inject_workflow_milestone("workflow_checkpoint", &event_payload) {
                    maybe_spawn_workflow_milestone_injection(
                        host.db.clone(),
                        &host.run_id,
                        "workflow_checkpoint",
                        event.seq,
                        &event_payload,
                    );
                }
                Ok(json!({ "eventSeq": event.seq, "title": title }))
            },
        )
    }

    fn report(&mut self, args: Value) -> Result<Value> {
        let summary = required_string(&args, "summary")?;
        let title =
            optional_string(&args, "title").unwrap_or_else(|| "Workflow report".to_string());
        let next_action =
            optional_string(&args, "nextAction").or_else(|| optional_string(&args, "next_action"));
        let needs_user = args
            .get("needsUser")
            .or_else(|| args.get("needs_user"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let inject_policy = optional_string(&args, "inject")
            .or_else(|| optional_string(&args, "injectPolicy"))
            .or_else(|| optional_string(&args, "inject_policy"))
            .unwrap_or_else(|| {
                if needs_user {
                    "now".to_string()
                } else {
                    "auto".to_string()
                }
            });
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let input = compact_input(args);
        self.execute_op("report", WorkflowEffectClass::Idempotent, input, |host| {
            let event_payload = json!({
                "title": title,
                "summary": summary,
                "nextAction": next_action,
                "needsUser": needs_user,
                "injectPolicy": inject_policy,
                "payload": payload,
            });
            let event = host.db.append_workflow_event(
                &host.run_id,
                "workflow_report",
                event_payload.clone(),
            )?;
            if should_inject_workflow_milestone("workflow_report", &event_payload) {
                maybe_spawn_workflow_milestone_injection(
                    host.db.clone(),
                    &host.run_id,
                    "workflow_report",
                    event.seq,
                    &event_payload,
                );
            }
            Ok(json!({ "eventSeq": event.seq, "needsUser": needs_user }))
        })
    }

    fn trace(&mut self, args: Value) -> Result<Value> {
        let label = optional_string(&args, "label");
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let input = compact_input(args);
        self.execute_op("trace", WorkflowEffectClass::Pure, input, |host| {
            let event = host.db.append_workflow_event(
                &host.run_id,
                "trace",
                json!({
                    "label": label,
                    "payload": payload,
                }),
            )?;
            Ok(json!({ "eventSeq": event.seq }))
        })
    }

    fn block(&mut self, args: Value) -> Result<Value> {
        let reason = block_reason_from_args(&args);
        let label = optional_string(&args, "label");
        let payload = args.get("payload").cloned().unwrap_or(Value::Null);
        let input = compact_input(args);
        self.execute_op("block", WorkflowEffectClass::Idempotent, input, |host| {
            let _event = host.db.append_workflow_event(
                &host.run_id,
                "workflow_block_requested",
                json!({
                    "reason": reason,
                    "label": label,
                    "payload": payload,
                }),
            )?;
            host.db.transition_workflow_run(
                &host.run_id,
                WorkflowRunState::Blocked,
                Some(&reason),
            )?;
            Err(anyhow!("workflow blocked: {reason}"))
        })
    }

    fn deterministic_now(&mut self, _args: Value) -> Result<Value> {
        let created =
            chrono::DateTime::parse_from_rfc3339(&self.run_created_at).with_context(|| {
                format!("invalid workflow run created_at `{}`", self.run_created_at)
            })?;
        Ok(json!(created.timestamp_millis()))
    }

    fn deterministic_random(&mut self, args: Value) -> Result<Value> {
        let seed = required_string(&args, "seed")?;
        let input = format!(
            "{}\n{}\n{}\n{}",
            self.run_id,
            self.run_created_at,
            self.current_position_hint(),
            seed
        );
        let hash = blake3::hash(input.as_bytes());
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&hash.as_bytes()[..8]);
        let n = u64::from_le_bytes(bytes);
        let value = ((n >> 11) as f64) / ((1u64 << 53) as f64);
        Ok(json!(value))
    }

    fn finish(&mut self, args: Value) -> Result<Value> {
        let mut agent_usage = self.db.workflow_agent_usage_snapshot(&self.run_id)?;
        if agent_usage.running_agents > 0 {
            let _ = self.db.append_workflow_event(
                &self.run_id,
                "workflow_finish_waiting_children",
                json!({
                    "spawnedAgents": agent_usage.spawned_agents,
                    "runningAgents": agent_usage.running_agents,
                }),
            )?;
            let run = self
                .db
                .get_workflow_run(&self.run_id)?
                .ok_or_else(|| anyhow!("workflow run {} not found", self.run_id))?;
            let wait_secs = optional_u64_any(
                &run.budget,
                &[
                    "maxRuntimeSecs",
                    "max_runtime_secs",
                    "maxScriptSecs",
                    "max_script_secs",
                ],
            )
            .unwrap_or(600)
            .clamp(1, 1800);
            let deadline = Instant::now() + Duration::from_secs(wait_secs);
            while agent_usage.running_agents > 0 && Instant::now() < deadline {
                let current = self
                    .db
                    .get_workflow_run(&self.run_id)?
                    .map(|run| run.state)
                    .unwrap_or(WorkflowRunState::Cancelled);
                if matches!(
                    current,
                    WorkflowRunState::Cancelled
                        | WorkflowRunState::Paused
                        | WorkflowRunState::Blocked
                ) {
                    return Err(anyhow!(
                        "workflow.finish stopped while waiting for children because run state is {}",
                        current.as_str()
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
                agent_usage = self.db.workflow_agent_usage_snapshot(&self.run_id)?;
            }
            if agent_usage.running_agents > 0 {
                let reason = "workflow_children_wait_timeout";
                self.db.transition_workflow_run(
                    &self.run_id,
                    WorkflowRunState::Blocked,
                    Some(reason),
                )?;
                return Err(anyhow!(
                    "workflow.finish timed out after {wait_secs}s with {} child agent(s) still running",
                    agent_usage.running_agents
                ));
            }
        }

        let mut output_arg = args;
        agent_usage = self.db.workflow_agent_usage_snapshot(&self.run_id)?;
        let agent_results = if agent_usage.spawned_agents > 0 {
            let agent_results = workflow_agent_results_for_finish(&self.db, &self.run_id)?;
            if let Value::Object(ref mut map) = output_arg {
                map.entry("agentResults".to_string())
                    .or_insert_with(|| agent_results.clone());
            } else {
                output_arg = json!({
                    "result": output_arg,
                    "agentResults": agent_results.clone(),
                });
            }
            Some(agent_results)
        } else {
            None
        };
        let input = compact_input(output_arg.clone());
        let executed =
            self.execute_op_tracked("finish", WorkflowEffectClass::Pure, input, |_host| {
                Ok(output_arg.clone())
            })?;
        if let Some(agent_results) = agent_results.as_ref() {
            self.record_consumed_agent_results("finish", agent_results)?;
        }
        let output = executed.output;
        self.finished_output = Some(output.clone());
        self.db.transition_workflow_run(
            &self.run_id,
            WorkflowRunState::Completed,
            Some("workflow_finish"),
        )?;
        Ok(output)
    }

    fn execute_op<F>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        f: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut WorkflowRuntimeHost) -> Result<Value>,
    {
        self.execute_op_tracked(op_type, effect_class, input, f)
            .map(|executed| executed.output)
    }

    fn execute_op_tracked<F>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        f: F,
    ) -> Result<ExecutedWorkflowOp>
    where
        F: FnOnce(&mut WorkflowRuntimeHost) -> Result<Value>,
    {
        self.execute_op_with_key_tracked(op_type, effect_class, input, |host, _op_key| f(host))
    }

    fn execute_op_with_key<F>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        f: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Value>,
    {
        self.execute_op_with_key_tracked(op_type, effect_class, input, f)
            .map(|executed| executed.output)
    }

    fn execute_op_with_key_tracked<F>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        f: F,
    ) -> Result<ExecutedWorkflowOp>
    where
        F: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Value>,
    {
        let op_key = self.next_op_key(op_type);
        let existing = self.db.get_workflow_op(&self.run_id, &op_key)?;
        let op = self.db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: self.run_id.clone(),
            op_key: op_key.clone(),
            op_type: op_type.to_string(),
            effect_class,
            input,
            child_handle: None,
        })?;

        match op.state {
            WorkflowOpState::Completed => {
                return Ok(ExecutedWorkflowOp {
                    op_key,
                    output: op.output.unwrap_or(Value::Null),
                    replayed: true,
                })
            }
            WorkflowOpState::Failed => {
                return Err(anyhow!(
                    "workflow op {} previously failed: {}",
                    op_key,
                    op.error.unwrap_or(Value::Null)
                ));
            }
            WorkflowOpState::Pending | WorkflowOpState::Started => {}
        }
        if existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started)
        {
            match self.db.started_op_recovery_action(&self.run_id, &op_key)? {
                Some(super::types::StartedOpRecoveryAction::BlockNonIdempotent)
                | Some(super::types::StartedOpRecoveryAction::AttachChildHandle(_)) => {
                    let _ = self
                        .db
                        .block_run_for_started_non_idempotent_op(&self.run_id, &op_key);
                    return Err(anyhow!(
                        "workflow op {} is a previously-started non-idempotent op; run was blocked",
                        op_key
                    ));
                }
                Some(super::types::StartedOpRecoveryAction::RerunPure)
                | Some(super::types::StartedOpRecoveryAction::RecheckIdempotent)
                | None => {}
            }
        }

        let output = match f(self, &op_key) {
            Ok(output) => output,
            Err(err) => {
                let _ = self.db.fail_workflow_op(
                    &self.run_id,
                    &op_key,
                    json!({ "message": err.to_string() }),
                );
                return Err(err);
            }
        };
        self.db
            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
        Ok(ExecutedWorkflowOp {
            op_key,
            output,
            replayed: false,
        })
    }

    fn execute_op_with_child_handle<F, R>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        child_handle: String,
        recover_started_child: R,
        f: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Value>,
        R: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Option<Value>>,
    {
        self.execute_op_with_child_handle_tracked(
            op_type,
            effect_class,
            input,
            child_handle,
            recover_started_child,
            f,
        )
        .map(|executed| executed.output)
    }

    fn execute_op_with_child_handle_tracked<F, R>(
        &mut self,
        op_type: &str,
        effect_class: WorkflowEffectClass,
        input: Value,
        child_handle: String,
        recover_started_child: R,
        f: F,
    ) -> Result<ExecutedWorkflowOp>
    where
        F: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Value>,
        R: FnOnce(&mut WorkflowRuntimeHost, &str) -> Result<Option<Value>>,
    {
        let op_key = self.next_op_key(op_type);
        let existing = self.db.get_workflow_op(&self.run_id, &op_key)?;
        let existing_started_without_child = existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started && op.child_handle.is_none());
        let effective_child_handle = existing
            .as_ref()
            .and_then(|op| op.child_handle.clone())
            .unwrap_or(child_handle);
        let op = self.db.upsert_workflow_op_started(UpsertWorkflowOpInput {
            run_id: self.run_id.clone(),
            op_key: op_key.clone(),
            op_type: op_type.to_string(),
            effect_class,
            input,
            child_handle: if existing_started_without_child {
                None
            } else {
                Some(effective_child_handle.clone())
            },
        })?;

        match op.state {
            WorkflowOpState::Completed => {
                return Ok(ExecutedWorkflowOp {
                    op_key,
                    output: op.output.unwrap_or(Value::Null),
                    replayed: true,
                })
            }
            WorkflowOpState::Failed => {
                return Err(anyhow!(
                    "workflow op {} previously failed: {}",
                    op_key,
                    op.error.unwrap_or(Value::Null)
                ));
            }
            WorkflowOpState::Pending | WorkflowOpState::Started => {}
        }

        if existing
            .as_ref()
            .is_some_and(|op| op.state == WorkflowOpState::Started)
        {
            match self.db.started_op_recovery_action(&self.run_id, &op_key)? {
                Some(super::types::StartedOpRecoveryAction::AttachChildHandle(handle)) => {
                    if let Some(output) = recover_started_child(self, &handle)? {
                        self.db
                            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
                        return Ok(ExecutedWorkflowOp {
                            op_key,
                            output,
                            replayed: false,
                        });
                    }
                }
                Some(super::types::StartedOpRecoveryAction::BlockNonIdempotent) => {
                    let _ = self
                        .db
                        .block_run_for_started_non_idempotent_op(&self.run_id, &op_key);
                    return Err(anyhow!(
                        "workflow op {} is a previously-started non-idempotent op; run was blocked",
                        op_key
                    ));
                }
                Some(super::types::StartedOpRecoveryAction::RerunPure)
                | Some(super::types::StartedOpRecoveryAction::RecheckIdempotent)
                | None => {}
            }
        }

        let output = match f(self, &effective_child_handle) {
            Ok(output) => output,
            Err(err) => {
                let _ = self.db.fail_workflow_op(
                    &self.run_id,
                    &op_key,
                    json!({ "message": err.to_string() }),
                );
                return Err(err);
            }
        };
        self.db
            .complete_workflow_op(&self.run_id, &op_key, output.clone())?;
        Ok(ExecutedWorkflowOp {
            op_key,
            output,
            replayed: false,
        })
    }

    fn next_op_key(&mut self, op_type: &str) -> String {
        let scope = self
            .op_scopes
            .last_mut()
            .expect("workflow runtime always has a root op scope");
        let idx = scope.next_op_index;
        scope.next_op_index += 1;
        format!("{}/op#{idx}({op_type})", scope.prefix)
    }

    fn current_position_hint(&self) -> String {
        self.op_scopes
            .iter()
            .map(|scope| format!("{}#{}", scope.prefix, scope.next_op_index))
            .collect::<Vec<_>>()
            .join("/")
    }

    fn dispatch_tool(&self, name: &str, args: &Value) -> Result<String> {
        let ctx = self.tool_exec_context();
        self.dispatch_tool_with_context(name, args, ctx)
    }

    fn dispatch_tool_with_context(
        &self,
        name: &str,
        args: &Value,
        ctx: ToolExecContext,
    ) -> Result<String> {
        let default_path = ctx.default_path().to_string();
        let session_id = self.session_id.clone();
        self.tokio_handle
            .block_on(tools::execute_tool_with_context(name, args, &ctx))
            .with_context(|| {
                format!("workflow.tool({name}) failed (session={session_id}, cwd={default_path})")
            })
    }

    fn dispatch_ask_user(&self, args: &Value) -> Result<Value> {
        if let crate::permission::ApprovalSurface::Unattended(reason) =
            crate::permission::evaluate_approval_surface(Some(&self.session_id))
        {
            return self.resolve_unattended_ask_user(reason);
        }

        let raw = self
            .tokio_handle
            .block_on(tools::ask_user_question::execute(
                args,
                Some(&self.session_id),
            ));
        parse_ask_user_output(raw)
    }

    fn resolve_unattended_ask_user(
        &self,
        reason: crate::permission::UnattendedReason,
    ) -> Result<Value> {
        let action = crate::config::cached_config()
            .permission
            .unattended_approval_action;
        if let Some(bus) = crate::globals::get_event_bus() {
            bus.emit(
                "approval:unattended",
                json!({
                    "session_id": self.session_id,
                    "reason": reason.as_str(),
                    "action": match action {
                        crate::permission::UnattendedApprovalAction::Proceed => "proceed",
                        crate::permission::UnattendedApprovalAction::Deny => "deny",
                    },
                    "strict": false,
                    "effective": match action {
                        crate::permission::UnattendedApprovalAction::Proceed => "proceed",
                        crate::permission::UnattendedApprovalAction::Deny => "deny",
                    },
                    "command": "workflow.askUser",
                }),
            );
        }

        match action {
            crate::permission::UnattendedApprovalAction::Deny => Err(anyhow!(
                "workflow.askUser unattended surface ({}): {}",
                reason.as_str(),
                reason.explain()
            )),
            crate::permission::UnattendedApprovalAction::Proceed => {
                crate::app_warn!(
                    "workflow",
                    "ask_user",
                    "workflow.askUser auto-proceeded on unattended surface ({}) for session {}",
                    reason.as_str(),
                    self.session_id
                );
                Ok(json!({
                    "answers": [],
                    "unattended": true,
                    "proceeded": true,
                    "reason": reason.as_str(),
                    "message": "No human approval surface was available; continued because unattendedApprovalAction=proceed.",
                }))
            }
        }
    }

    fn tool_exec_context(&self) -> ToolExecContext {
        ToolExecContext {
            session_id: Some(self.session_id.clone()),
            session_db: Some(crate::tools::SessionDbHandle(self.db.clone())),
            session_working_dir: self.session_context.working_dir.clone(),
            agent_id: self.session_context.agent_id.clone(),
            session_mode: self.session_context.session_mode,
            project_id: self.session_context.project_id.clone(),
            incognito: self.session_context.incognito,
            ..Default::default()
        }
    }
}

fn task_handle(task: &Task, label: Option<&str>) -> Value {
    json!({
        "id": task.id,
        "sessionId": task.session_id,
        "title": task.content,
        "status": task.status,
        "label": label,
    })
}

pub(crate) fn spawn_agent_tool_args(args: &Value) -> Result<Value> {
    let task = required_string(args, "task")?;
    let mut map = serde_json::Map::new();
    map.insert("action".to_string(), Value::String("spawn".to_string()));
    map.insert("task".to_string(), Value::String(task));
    if let Some(agent_id) =
        optional_string(args, "agent_id").or_else(|| optional_string(args, "agent"))
    {
        map.insert("agent_id".to_string(), Value::String(agent_id));
    }
    if let Some(label) = optional_string(args, "label") {
        map.insert("label".to_string(), Value::String(label));
    }
    if let Some(model) = optional_string(args, "model") {
        map.insert("model".to_string(), Value::String(model));
    }
    if let Some(timeout_secs) = optional_u64_any(args, &["timeout_secs", "timeoutSecs", "timeout"])
    {
        map.insert(
            "timeout_secs".to_string(),
            Value::Number(timeout_secs.into()),
        );
    }
    if let Some(files) = args.get("files") {
        if !files.is_array() {
            return Err(anyhow!("workflow.spawnAgent files must be an array"));
        }
        map.insert("files".to_string(), files.clone());
    }
    Ok(Value::Object(map))
}

fn inject_workflow_preallocated_run_id(args: &mut Value, run_id: &str) -> Result<()> {
    let Value::Object(map) = args else {
        return Err(anyhow!(
            "workflow.spawnAgent internal args must be an object"
        ));
    };
    map.insert(
        tools::subagent::WORKFLOW_PREALLOCATED_RUN_ID_ARG.to_string(),
        Value::String(run_id.to_string()),
    );
    map.insert(
        tools::subagent::WORKFLOW_SKIP_PARENT_INJECTION_ARG.to_string(),
        Value::Bool(true),
    );
    Ok(())
}

fn subagent_run_handle(
    run: &crate::subagent::SubagentRun,
    label: Option<&str>,
    task: Option<&str>,
    inject_policy: &str,
    result_mode: &str,
) -> Value {
    json!({
        "kind": "subagent",
        "runId": run.run_id,
        "run_id": run.run_id,
        "sessionId": run.child_session_id,
        "session_id": run.child_session_id,
        "status": run.status.as_str(),
        "label": label.map(ToOwned::to_owned).or_else(|| run.label.clone()),
        "task": task.map(ToOwned::to_owned).unwrap_or_else(|| run.task.clone()),
        "injectPolicy": inject_policy,
        "resultMode": result_mode,
        "resultAvailable": run.result.is_some(),
        "message": "attached to existing sub-agent run",
    })
}

fn parse_tool_json_output(output: &str, context: &str) -> Result<Value> {
    serde_json::from_str(output).with_context(|| format!("{context} returned non-JSON output"))
}

fn agent_handles_args_from_values(
    api: &str,
    handles: Value,
    options: Option<Value>,
) -> Result<Value> {
    let mut map = match handles {
        Value::Object(map)
            if map.contains_key("handles")
                || map.contains_key("runIds")
                || map.contains_key("run_ids") =>
        {
            map
        }
        value => {
            let mut map = serde_json::Map::new();
            map.insert("handles".to_string(), value);
            map
        }
    };

    if let Some(options) = options {
        let Value::Object(options) = options else {
            return Err(anyhow!("{api} options must be an object"));
        };
        for (key, value) in options {
            map.insert(key, value);
        }
    }

    Ok(Value::Object(map))
}

pub(crate) fn wait_all_tool_args(args: &Value) -> Result<Value> {
    let handles = args
        .get("handles")
        .or_else(|| args.get("runIds"))
        .or_else(|| args.get("run_ids"))
        .ok_or_else(|| anyhow!("workflow.waitAll requires handles or runIds"))?;
    let run_ids = extract_subagent_run_ids(handles, "workflow.waitAll")?;
    if run_ids.is_empty() {
        return Err(anyhow!("workflow.waitAll requires at least one handle"));
    }

    let mut map = serde_json::Map::new();
    map.insert("action".to_string(), Value::String("wait_all".to_string()));
    map.insert("run_ids".to_string(), json!(run_ids));
    if let Some(wait_timeout) = optional_u64_any(args, &["wait_timeout", "waitTimeout", "timeout"])
    {
        map.insert(
            "wait_timeout".to_string(),
            Value::Number(wait_timeout.into()),
        );
    }
    if let Some(partial) = args.get("partial").and_then(Value::as_bool) {
        map.insert("partial".to_string(), Value::Bool(partial));
    }
    let result_mode = optional_string(args, "resultMode")
        .or_else(|| optional_string(args, "result_mode"))
        .unwrap_or_else(|| "full".to_string());
    if !matches!(
        result_mode.as_str(),
        "status" | "preview" | "summary" | "full"
    ) {
        return Err(anyhow!(
            "workflow.waitAll resultMode must be status, preview, summary, or full"
        ));
    }
    map.insert("result_mode".to_string(), Value::String(result_mode));
    Ok(Value::Object(map))
}

fn workflow_agent_run_ids(args: &Value, api: &str) -> Result<Vec<String>> {
    let handles = args
        .get("handles")
        .or_else(|| args.get("runIds"))
        .or_else(|| args.get("run_ids"))
        .ok_or_else(|| anyhow!("{api} requires one or more child handles"))?;
    let run_ids = extract_subagent_run_ids(handles, api)?;
    if run_ids.is_empty() {
        return Err(anyhow!("{api} requires at least one child handle"));
    }
    Ok(run_ids)
}

pub(crate) fn ensure_workflow_owned_agent_run_ids(
    db: &SessionDB,
    workflow_run_id: &str,
    run_ids: &[String],
    api: &str,
) -> Result<()> {
    let owned = db
        .list_workflow_child_handles(workflow_run_id)?
        .into_iter()
        .filter_map(|(op_type, child_handle)| (op_type == "spawnAgent").then_some(child_handle))
        .collect::<Vec<_>>();
    let foreign = run_ids
        .iter()
        .filter(|run_id| !owned.iter().any(|owned_id| owned_id == *run_id))
        .cloned()
        .collect::<Vec<_>>();
    if !foreign.is_empty() {
        return Err(anyhow!(
            "{api} only accepts child agents owned by workflow {workflow_run_id}; unknown handles: {}",
            foreign.join(", ")
        ));
    }
    Ok(())
}

fn workflow_agent_run_status(run: &crate::subagent::SubagentRun) -> Value {
    json!({
        "runId": run.run_id,
        "run_id": run.run_id,
        "sessionId": run.child_session_id,
        "status": run.status.as_str(),
        "terminal": run.status.is_terminal(),
        "resultAvailable": run.result.is_some(),
        "label": run.label,
        "task": run.task,
        "error": run.error,
        "durationMs": run.duration_ms,
        "inputTokens": run.input_tokens,
        "outputTokens": run.output_tokens,
    })
}

fn workflow_agent_status_snapshot(db: &SessionDB, run_ids: &[String]) -> Result<Value> {
    let mut runs = Vec::with_capacity(run_ids.len());
    let mut completed = 0usize;
    let mut running = 0usize;
    let mut failed = 0usize;
    let mut terminal = 0usize;
    for run_id in run_ids {
        match db.get_subagent_run(run_id)? {
            Some(run) => {
                if run.status.is_terminal() {
                    terminal += 1;
                    if run.status == crate::subagent::SubagentStatus::Completed {
                        completed += 1;
                    } else {
                        failed += 1;
                    }
                } else {
                    running += 1;
                }
                runs.push(workflow_agent_run_status(&run));
            }
            None => {
                terminal += 1;
                failed += 1;
                runs.push(json!({
                    "runId": run_id,
                    "run_id": run_id,
                    "status": "not_found",
                    "terminal": true,
                    "resultAvailable": false,
                }));
            }
        }
    }
    Ok(json!({
        "total": run_ids.len(),
        "terminal": terminal,
        "completed": completed,
        "running": running,
        "failed": failed,
        "allTerminal": terminal == run_ids.len(),
        "runs": runs,
    }))
}

fn workflow_agent_results_for_finish(db: &SessionDB, run_id: &str) -> Result<Value> {
    let ops = db.list_workflow_ops(run_id)?;
    let mut results = Vec::new();
    for op in ops.into_iter().filter(|op| op.op_type == "spawnAgent") {
        let Some(child_run_id) = op.child_handle.as_deref() else {
            continue;
        };
        let result_mode = op
            .input
            .get("resultMode")
            .and_then(Value::as_str)
            .unwrap_or("summary");
        let Some(child) = db.get_subagent_run(child_run_id)? else {
            results.push(json!({
                "runId": child_run_id,
                "run_id": child_run_id,
                "status": "not_found",
                "terminal": true,
                "resultAvailable": false,
                "mode": result_mode,
                "error": "workflow-owned child run not found",
            }));
            continue;
        };
        let mut value = workflow_agent_run_status(&child);
        if let Value::Object(ref mut map) = value {
            map.insert("mode".to_string(), json!(result_mode));
            if let Some(result) = child.result.as_deref() {
                map.insert(
                    "result".to_string(),
                    Value::String(if result_mode == "full" {
                        result.to_string()
                    } else {
                        truncate_chars(result, 2000)
                    }),
                );
            }
        }
        results.push(value);
    }
    Ok(Value::Array(results))
}

fn consumed_agent_result_ids(output: &Value) -> Vec<String> {
    fn collect(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            Value::Object(map) => {
                let has_result = [
                    "result",
                    "result_summary",
                    "resultSummary",
                    "result_preview",
                    "resultPreview",
                    "error",
                ]
                .iter()
                .any(|key| map.get(*key).is_some_and(|value| !value.is_null()));
                if has_result {
                    if let Some(run_id) = map
                        .get("runId")
                        .or_else(|| map.get("run_id"))
                        .and_then(Value::as_str)
                    {
                        if !out.iter().any(|existing| existing == run_id) {
                            out.push(run_id.to_string());
                        }
                    }
                }
                if let Some(runs) = map.get("runs") {
                    collect(runs, out);
                }
            }
            _ => {}
        }
    }

    let mut run_ids = Vec::new();
    collect(output, &mut run_ids);
    run_ids
}

pub(crate) fn wait_all_output_consumes_results(output: &Value) -> bool {
    output
        .get("resultMode")
        .or_else(|| output.get("result_mode"))
        .and_then(Value::as_str)
        != Some("status")
}

fn terminal_agent_result_ids(output: &Value) -> Vec<String> {
    fn collect(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, out);
                }
            }
            Value::Object(map) => {
                let terminal = map
                    .get("terminal")
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| {
                        matches!(
                            map.get("status").and_then(Value::as_str),
                            Some("completed" | "error" | "timeout" | "killed" | "not_found")
                        )
                    });
                if terminal {
                    if let Some(run_id) = map
                        .get("runId")
                        .or_else(|| map.get("run_id"))
                        .and_then(Value::as_str)
                    {
                        if !out.iter().any(|existing| existing == run_id) {
                            out.push(run_id.to_string());
                        }
                    }
                }
                if let Some(runs) = map.get("runs") {
                    collect(runs, out);
                }
            }
            _ => {}
        }
    }

    let mut run_ids = Vec::new();
    collect(output, &mut run_ids);
    run_ids
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let prefix = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    format!("{prefix}...")
}

fn extract_subagent_run_ids(value: &Value, api: &str) -> Result<Vec<String>> {
    match value {
        Value::String(run_id) => Ok(vec![run_id.clone()]),
        Value::Array(items) => {
            let mut run_ids = Vec::with_capacity(items.len());
            for item in items {
                run_ids.extend(extract_subagent_run_ids(item, api)?);
            }
            Ok(run_ids)
        }
        Value::Object(map) => {
            if let Some(run_id) = map
                .get("runId")
                .or_else(|| map.get("run_id"))
                .and_then(Value::as_str)
            {
                return Ok(vec![run_id.to_string()]);
            }
            if let Some(nested) = map
                .get("handles")
                .or_else(|| map.get("runIds"))
                .or_else(|| map.get("run_ids"))
            {
                return extract_subagent_run_ids(nested, api);
            }
            Err(anyhow!("{api} handle object must include runId"))
        }
        _ => Err(anyhow!("{api} handles must be run IDs or subagent handles")),
    }
}

fn normalize_wait_all_response(value: &mut Value) {
    if let Value::Object(map) = value {
        if let Some(all_completed) = map.get("all_completed").cloned() {
            map.entry("allCompleted".to_string())
                .or_insert(all_completed);
        }
        for (snake, camel) in [
            ("timed_out", "timedOut"),
            ("accepted_partial", "acceptedPartial"),
            ("result_mode", "resultMode"),
        ] {
            if let Some(value) = map.get(snake).cloned() {
                map.entry(camel.to_string()).or_insert(value);
            }
        }
        if let Some(Value::Array(runs)) = map.get_mut("runs") {
            for run in runs {
                if let Value::Object(run) = run {
                    if let Some(run_id) = run.get("run_id").cloned() {
                        run.entry("runId".to_string()).or_insert(run_id);
                    }
                    if let Some(summary) = run.get("result_summary").cloned() {
                        run.entry("resultSummary".to_string()).or_insert(summary);
                    }
                    if let Some(preview) = run.get("result_preview").cloned() {
                        run.entry("resultPreview".to_string()).or_insert(preview);
                    }
                }
            }
        }
    }
}

pub(crate) fn ask_user_tool_args(args: &Value) -> Result<Value> {
    let questions = if let Some(questions) = args.get("questions") {
        let Value::Array(questions) = questions else {
            return Err(anyhow!("workflow.askUser questions must be an array"));
        };
        questions.clone()
    } else {
        vec![ask_user_question_from_args(args)?]
    };

    if questions.is_empty() {
        return Err(anyhow!("workflow.askUser requires at least one question"));
    }
    if questions.len() > 4 {
        return Err(anyhow!(
            "workflow.askUser supports at most 4 questions per call"
        ));
    }

    let mut map = serde_json::Map::new();
    map.insert("questions".to_string(), Value::Array(questions));
    if let Some(context) = args.get("context").cloned() {
        map.insert("context".to_string(), context);
    }
    Ok(Value::Object(map))
}

fn ask_user_question_from_args(args: &Value) -> Result<Value> {
    let question = required_string(args, "question")?;
    let mut map = serde_json::Map::new();
    map.insert(
        "question_id".to_string(),
        Value::String(
            optional_string(args, "question_id")
                .or_else(|| optional_string(args, "questionId"))
                .unwrap_or_else(|| "q_0".to_string()),
        ),
    );
    map.insert("text".to_string(), Value::String(question));

    if let Some(header) = args.get("header").cloned() {
        map.insert("header".to_string(), header);
    }
    if let Some(options) = args.get("options") {
        map.insert("options".to_string(), normalize_ask_user_options(options)?);
    } else {
        map.insert("options".to_string(), Value::Array(Vec::new()));
    }
    if let Some(allow_custom) = args.get("allow_custom").or_else(|| args.get("allowCustom")) {
        map.insert("allow_custom".to_string(), allow_custom.clone());
    }
    if let Some(multi_select) = args.get("multi_select").or_else(|| args.get("multiSelect")) {
        map.insert("multi_select".to_string(), multi_select.clone());
    }
    if let Some(template) = args.get("template") {
        map.insert("template".to_string(), template.clone());
    }
    if let Some(timeout) = args.get("timeout_secs").or_else(|| args.get("timeoutSecs")) {
        map.insert("timeout_secs".to_string(), timeout.clone());
    }
    if let Some(defaults) = args
        .get("default_values")
        .or_else(|| args.get("defaultValues"))
    {
        map.insert("default_values".to_string(), defaults.clone());
    }

    Ok(Value::Object(map))
}

fn normalize_ask_user_options(value: &Value) -> Result<Value> {
    let Value::Array(options) = value else {
        return Err(anyhow!("workflow.askUser options must be an array"));
    };
    if options.len() > 8 {
        return Err(anyhow!(
            "workflow.askUser supports at most 8 options per question"
        ));
    }

    let mut normalized = Vec::with_capacity(options.len());
    for option in options {
        match option {
            Value::String(label) => {
                normalized.push(json!({
                    "value": label,
                    "label": label,
                }));
            }
            Value::Object(option) => {
                let label = option
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| option.get("value").and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("workflow.askUser option requires label or value"))?;
                let value = option
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(label);
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), Value::String(value.to_string()));
                map.insert("label".to_string(), Value::String(label.to_string()));
                for key in ["description", "recommended", "preview"] {
                    if let Some(field) = option.get(key).cloned() {
                        map.insert(key.to_string(), field);
                    }
                }
                if let Some(preview_kind) = option
                    .get("previewKind")
                    .or_else(|| option.get("preview_kind"))
                {
                    map.insert("previewKind".to_string(), preview_kind.clone());
                }
                normalized.push(Value::Object(map));
            }
            _ => {
                return Err(anyhow!(
                    "workflow.askUser options must be strings or objects"
                ));
            }
        }
    }

    Ok(Value::Array(normalized))
}

fn workflow_tool_uses_async_child(name: &str, args: &Value) -> bool {
    tools::is_async_capable(name)
        && crate::config::cached_config().async_tools.enabled
        && args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn async_tool_started_output(name: &str, job_id: &str) -> String {
    crate::async_jobs::synthetic_started_result(job_id, name, JobOrigin::Explicit)
}

fn parse_ask_user_output(raw: String) -> Result<Value> {
    if raw.starts_with("Error:") {
        return Err(anyhow!("workflow.askUser failed: {raw}"));
    }
    if let Ok(value) = serde_json::from_str::<Value>(&raw) {
        return Ok(value);
    }

    let status = if raw.to_ascii_lowercase().contains("timed out") {
        "timed_out"
    } else if raw.to_ascii_lowercase().contains("cancelled") {
        "cancelled"
    } else {
        "message"
    };
    Ok(json!({
        "status": status,
        "message": raw,
    }))
}

fn workflow_goal_id_from_args(args: &Value, default_goal_id: Option<String>) -> Option<String> {
    optional_string(args, "goalId")
        .or_else(|| optional_string(args, "goal_id"))
        .or(default_goal_id)
}

fn block_reason_from_args(args: &Value) -> String {
    optional_string(args, "reason")
        .map(|value| value.trim().chars().take(160).collect::<String>())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| REPAIR_LOOP_EXHAUSTED_REASON.to_string())
}

fn ide_context_from_args(args: &Value) -> Result<Option<SessionIdeContext>> {
    let Some(value) = args
        .get("ideContext")
        .or_else(|| args.get("ide_context"))
        .cloned()
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let context = serde_json::from_value::<SessionIdeContext>(value)
        .context("workflow ideContext must match SessionIdeContext shape")?;
    Ok(Some(context.sanitized()))
}

fn focus_paths_from_args(args: &Value) -> Result<Vec<String>> {
    let Some(raw) = args
        .get("focusPaths")
        .or_else(|| args.get("focus_paths"))
        .or_else(|| args.get("files"))
    else {
        return Ok(Vec::new());
    };
    string_list_from_value(raw, "focusPaths")
}

fn string_array_arg(args: &Value, key: &str) -> Result<Vec<String>> {
    let Some(raw) = args.get(key) else {
        return Ok(Vec::new());
    };
    string_list_from_value(raw, key)
}

fn string_list_from_value(value: &Value, label: &str) -> Result<Vec<String>> {
    let raw = match value {
        Value::String(item) => vec![item.clone()],
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| anyhow!("workflow {label} entries must be strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => {
            return Err(anyhow!(
                "workflow {label} must be a string or array of strings"
            ))
        }
    };
    Ok(raw
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect())
}

fn workflow_review_output(snapshot: review::ReviewRunSnapshot) -> Value {
    let blocking = snapshot
        .findings
        .iter()
        .filter(|finding| {
            finding.status == ReviewFindingStatus::Open && finding.severity.is_blocking()
        })
        .count();
    json!({
        "kind": "review",
        "ok": blocking == 0,
        "runId": snapshot.run.id,
        "state": snapshot.run.state,
        "summary": snapshot.run.summary,
        "findingCount": snapshot.findings.len(),
        "blockingFindings": blocking,
        "stats": snapshot.run.stats,
        "findings": snapshot.findings.iter().map(|finding| {
            json!({
                "id": &finding.id,
                "file": &finding.file,
                "startLine": finding.start_line,
                "endLine": finding.end_line,
                "title": &finding.title,
                "category": &finding.category,
                "severity": finding.severity,
                "verdict": finding.verdict,
                "status": finding.status,
            })
        }).collect::<Vec<_>>(),
    })
}

fn workflow_verify_output(snapshot: verification::VerificationRunSnapshot) -> Value {
    json!({
        "kind": "verification_plan",
        "ok": snapshot.run.state == verification::VerificationRunState::Planned,
        "runId": snapshot.run.id,
        "state": snapshot.run.state,
        "summary": snapshot.run.summary,
        "commandCount": snapshot.steps.len(),
        "stats": snapshot.run.stats,
        "commands": snapshot.steps.iter().map(|step| {
            json!({
                "id": &step.id,
                "command": &step.command,
                "cwd": &step.cwd,
                "title": &step.title,
                "reason": &step.reason,
                "category": &step.category,
                "risk": step.risk,
                "autoRun": step.auto_run,
                "state": step.state,
            })
        }).collect::<Vec<_>>(),
    })
}

fn workflow_domain_evidence_source(
    source_metadata: Value,
    host: &WorkflowRuntimeHost,
    op_key: &str,
) -> Value {
    let mut map = match source_metadata {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("value".to_string(), other);
            map
        }
    };
    map.insert(
        "workflow".to_string(),
        json!({
            "runId": &host.run_id,
            "opKey": op_key,
            "sessionId": &host.session_id,
            "goalId": &host.goal_id,
            "executionMode": &host.execution_mode,
        }),
    );
    Value::Object(map)
}

fn domain_evidence_output(item: &DomainEvidenceItem) -> Value {
    json!({
        "kind": "domain_evidence",
        "id": &item.id,
        "goalId": &item.goal_id,
        "sessionId": &item.session_id,
        "projectId": &item.project_id,
        "domain": &item.domain,
        "evidenceType": &item.evidence_type,
        "title": &item.title,
        "summary": &item.summary,
        "sourceMetadata": &item.source_metadata,
        "confidence": item.confidence,
        "accessScope": &item.access_scope,
        "redactionStatus": &item.redaction_status,
        "createdAt": &item.created_at,
        "updatedAt": &item.updated_at,
    })
}

#[derive(Debug, Clone)]
struct ValidationCommand {
    command: String,
    cwd: Option<String>,
    timeout: Option<u64>,
}

impl ValidationCommand {
    fn exec_args(&self) -> Value {
        let mut args = serde_json::Map::new();
        args.insert("command".to_string(), Value::String(self.command.clone()));
        if let Some(cwd) = self.cwd.clone() {
            args.insert("cwd".to_string(), Value::String(cwd));
        }
        if let Some(timeout) = self.timeout {
            args.insert("timeout".to_string(), Value::Number(timeout.into()));
        }
        Value::Object(args)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationChildHandle {
    kind: String,
    jobs: Vec<ValidationJobRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationJobRef {
    job_id: String,
    command: String,
    cwd: Option<String>,
    timeout: Option<u64>,
}

impl ValidationJobRef {
    fn from_command(command: &ValidationCommand) -> Self {
        Self {
            job_id: JobManager::new_job_id(),
            command: command.command.clone(),
            cwd: command.cwd.clone(),
            timeout: command.timeout,
        }
    }

    fn exec_args(&self) -> Value {
        let command = ValidationCommand {
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            timeout: self.timeout,
        };
        let mut args = command.exec_args();
        if let Value::Object(map) = &mut args {
            if let Some(timeout) = self.timeout {
                map.insert(
                    tools::ASYNC_JOB_TIMEOUT_ARG.to_string(),
                    Value::Number(timeout.into()),
                );
            }
        }
        args
    }
}

fn validation_child_handle_for_commands(commands: &[ValidationCommand]) -> Result<String> {
    serde_json::to_string(&ValidationChildHandle {
        kind: "validate".to_string(),
        jobs: commands
            .iter()
            .map(ValidationJobRef::from_command)
            .collect(),
    })
    .context("serialize workflow.validate child handle")
}

fn parse_validation_child_handle(child_handle: &str) -> Result<ValidationChildHandle> {
    let child: ValidationChildHandle =
        serde_json::from_str(child_handle).context("parse workflow.validate child handle")?;
    if child.kind != "validate" {
        return Err(anyhow!(
            "workflow.validate child handle kind mismatch: {}",
            child.kind
        ));
    }
    if child.jobs.is_empty() {
        return Err(anyhow!("workflow.validate child handle contains no jobs"));
    }
    Ok(child)
}

fn validation_result_from_job(job_ref: ValidationJobRef, job: &BackgroundJob) -> Result<Value> {
    let (ok, exit_code, output) = match job.status {
        JobStatus::Completed => {
            let output = validation_job_output(job)?;
            let exit_code = validation_exit_code(&output);
            (exit_code == 0, exit_code, output)
        }
        JobStatus::Failed | JobStatus::Interrupted | JobStatus::TimedOut | JobStatus::Cancelled => {
            let output = job
                .error
                .clone()
                .unwrap_or_else(|| format!("workflow.validate job {}", job.status.as_str()));
            (false, -1, output)
        }
        JobStatus::Queued
        | JobStatus::Running
        | JobStatus::Cancelling
        | JobStatus::AwaitingApproval => {
            return Err(anyhow!(
                "workflow.validate child job {} is still {} after wait",
                job.job_id,
                job.status.as_str()
            ));
        }
    };
    Ok(json!({
        "command": job_ref.command,
        "cwd": job_ref.cwd,
        "timeout": job_ref.timeout,
        "jobId": job.job_id,
        "jobStatus": job.status.as_str(),
        "ok": ok,
        "exitCode": exit_code,
        "output": output,
    }))
}

fn validation_failure_fingerprint(results: &[Value]) -> Result<String> {
    let failed: Vec<Value> = results
        .iter()
        .filter(|result| !result.get("ok").and_then(Value::as_bool).unwrap_or(false))
        .map(|result| {
            json!({
                "command": result.get("command").cloned().unwrap_or(Value::Null),
                "cwd": result.get("cwd").cloned().unwrap_or(Value::Null),
                "timeout": result.get("timeout").cloned().unwrap_or(Value::Null),
                "jobStatus": result.get("jobStatus").cloned().unwrap_or(Value::Null),
                "exitCode": result.get("exitCode").cloned().unwrap_or(Value::Null),
                "output": normalized_validation_output(result),
            })
        })
        .collect();
    stable_value_hash(&Value::Array(failed))
}

fn normalized_validation_output(result: &Value) -> String {
    let raw = result
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    crate::truncate_utf8(normalized.trim(), VALIDATION_FINGERPRINT_OUTPUT_BYTES).to_string()
}

fn stable_value_hash(value: &Value) -> Result<String> {
    let serialized = serde_json::to_string(value)?;
    Ok(blake3::hash(serialized.as_bytes()).to_hex().to_string())
}

fn validation_job_output(job: &BackgroundJob) -> Result<String> {
    if let Some(path) = &job.result_path {
        return std::fs::read_to_string(path)
            .with_context(|| format!("read workflow.validate job result {}", path));
    }
    Ok(job.result_preview.clone().unwrap_or_default())
}

fn validation_commands_from_args(args: &Value) -> Result<Vec<ValidationCommand>> {
    let default_cwd = optional_string(args, "cwd");
    let default_timeout = args.get("timeout").and_then(Value::as_u64);
    let raw_commands = args
        .get("commands")
        .or_else(|| args.get("command"))
        .ok_or_else(|| anyhow!("workflow.validate requires commands"))?;
    let mut commands = Vec::new();
    match raw_commands {
        Value::String(command) => {
            commands.push(ValidationCommand {
                command: normalize_command(command)?,
                cwd: default_cwd,
                timeout: default_timeout,
            });
        }
        Value::Array(items) => {
            for item in items {
                commands.push(validation_command_from_value(
                    item,
                    default_cwd.clone(),
                    default_timeout,
                )?);
            }
        }
        _ => {
            return Err(anyhow!(
                "workflow.validate commands must be a string or array of strings/objects"
            ));
        }
    }
    if commands.is_empty() {
        return Err(anyhow!("workflow.validate requires at least one command"));
    }
    if commands.len() > 8 {
        return Err(anyhow!(
            "workflow.validate supports at most 8 commands per op"
        ));
    }
    Ok(commands)
}

fn validation_command_from_value(
    value: &Value,
    default_cwd: Option<String>,
    default_timeout: Option<u64>,
) -> Result<ValidationCommand> {
    match value {
        Value::String(command) => Ok(ValidationCommand {
            command: normalize_command(command)?,
            cwd: default_cwd,
            timeout: default_timeout,
        }),
        Value::Object(map) => {
            let command = map
                .get("command")
                .or_else(|| map.get("cmd"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("workflow.validate command object requires command"))?;
            Ok(ValidationCommand {
                command: normalize_command(command)?,
                cwd: map
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .or(default_cwd),
                timeout: map
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .or(default_timeout),
            })
        }
        _ => Err(anyhow!(
            "workflow.validate command entries must be strings or objects"
        )),
    }
}

fn normalize_command(command: &str) -> Result<String> {
    let command = command.trim();
    if command.is_empty() {
        return Err(anyhow!("workflow.validate command must not be empty"));
    }
    if command.len() > 4096 {
        return Err(anyhow!("workflow.validate command is too long"));
    }
    Ok(command.to_string())
}

pub(crate) fn validation_exit_code(output: &str) -> i64 {
    let trimmed = output.trim();
    if let Some(code) = trimmed
        .strip_prefix("Command completed with exit code ")
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return code;
    }
    if let Some(start) = trimmed.rfind("[exit code: ") {
        let after = &trimmed[start + "[exit code: ".len()..];
        if let Some(end) = after.find(']') {
            if let Ok(code) = after[..end].trim().parse::<i64>() {
                return code;
            }
        }
    }
    0
}

fn tool_effect_class(name: &str) -> WorkflowEffectClass {
    match name {
        tools::TOOL_READ
        | "read_file"
        | tools::TOOL_GREP
        | tools::TOOL_FIND
        | tools::TOOL_LS
        | "list_dir"
        | tools::TOOL_TOOL_SEARCH
        | tools::TOOL_GET_SETTINGS
        | tools::TOOL_AGENTS_LIST
        | tools::TOOL_RECALL_MEMORY
        | tools::TOOL_MEMORY_GET
        | tools::TOOL_JOB_STATUS
        | tools::TOOL_SESSIONS_LIST
        | tools::TOOL_SESSION_STATUS
        | tools::TOOL_SESSIONS_SEARCH
        | tools::TOOL_SESSIONS_HISTORY
        | tools::TOOL_PEEK_SESSIONS => WorkflowEffectClass::Pure,
        _ => WorkflowEffectClass::NonIdempotent,
    }
}

fn task_id_from_args(args: &Value) -> Result<i64> {
    let task = args
        .get("task")
        .ok_or_else(|| anyhow!("workflow.task.update requires task handle from task.create"))?;
    if let Some(id) = task.as_i64() {
        return Ok(id);
    }
    if let Some(id) = task.as_str().and_then(|s| s.parse::<i64>().ok()) {
        return Ok(id);
    }
    task.get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("workflow.task.update task handle must include id"))
}

fn required_string(args: &Value, key: &str) -> Result<String> {
    optional_string(args, key).ok_or_else(|| anyhow!("missing required string field '{}'", key))
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_u64_any(args: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(Value::as_u64))
}

fn compact_input(value: Value) -> Value {
    value
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowSessionContext {
    pub(crate) session_id: String,
    pub(crate) working_dir: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) session_mode: crate::permission::SessionMode,
    pub(crate) project_id: Option<String>,
    pub(crate) incognito: bool,
}

pub(crate) fn workflow_session_context(db: &SessionDB, session_id: &str) -> WorkflowSessionContext {
    let row = {
        let conn = match db.conn.lock() {
            Ok(conn) => conn,
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "resolve_root",
                    "session {} lookup lock failed while resolving workflow root: {}",
                    session_id,
                    err
                );
                return WorkflowSessionContext {
                    session_id: session_id.to_string(),
                    working_dir: current_dir_string(),
                    ..Default::default()
                };
            }
        };
        conn.query_row(
            "SELECT working_dir, project_id, agent_id, permission_mode, incognito FROM sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()
    };

    match row {
        Ok(Some((working_dir, project_id, agent_id, permission_mode, incognito))) => {
            let resolved_working_dir = working_dir
                .filter(|s| !s.trim().is_empty())
                .or_else(|| project_id.as_deref().and_then(workflow_root_for_project))
                .or_else(current_dir_string);
            WorkflowSessionContext {
                session_id: session_id.to_string(),
                working_dir: resolved_working_dir,
                agent_id: agent_id.filter(|s| !s.trim().is_empty()),
                session_mode: permission_mode
                    .as_deref()
                    .map(crate::permission::SessionMode::parse_or_default)
                    .unwrap_or_default(),
                project_id,
                incognito: incognito.unwrap_or(0) != 0,
            }
        }
        Ok(None) => WorkflowSessionContext {
            session_id: session_id.to_string(),
            working_dir: current_dir_string(),
            ..Default::default()
        },
        Err(err) => {
            crate::app_warn!(
                "workflow",
                "resolve_root",
                "session {} lookup failed while resolving workflow root: {}",
                session_id,
                err
            );
            WorkflowSessionContext {
                session_id: session_id.to_string(),
                working_dir: current_dir_string(),
                ..Default::default()
            }
        }
    }
}

fn workflow_session_context_for_run(
    db: &SessionDB,
    run: &super::types::WorkflowRun,
) -> Result<WorkflowSessionContext> {
    let mut context = workflow_session_context(db, &run.session_id);
    let Some(worktree_id) = run.worktree_id.as_deref() else {
        return Ok(context);
    };
    let worktree = db
        .get_managed_worktree(worktree_id)?
        .ok_or_else(|| anyhow!("managed worktree not found: {worktree_id}"))?;
    if worktree.session_id != run.session_id {
        bail!(
            "managed worktree {} belongs to session {}; expected {}",
            worktree_id,
            worktree.session_id,
            run.session_id
        );
    }
    let worktree = if worktree.state == crate::worktree::ManagedWorktreeState::Archived
        || !worktree.path_exists
    {
        db.restore_managed_worktree(worktree_id)?
    } else {
        worktree
    };
    context.working_dir = Some(worktree.path.clone());
    let _ = db.append_workflow_event(
        &run.id,
        "run_worktree_attached",
        json!({
            "worktreeId": worktree.id,
            "path": worktree.path,
            "state": worktree.state,
        }),
    );
    Ok(context)
}

fn workflow_root_for_project(project_id: &str) -> Option<String> {
    if let Some(db) = crate::get_project_db() {
        match db.get(project_id) {
            Ok(Some(project)) => {
                if let Some(wd) = project.working_dir.filter(|s| !s.trim().is_empty()) {
                    return Some(wd);
                }
            }
            Ok(None) => {}
            Err(err) => {
                crate::app_warn!(
                    "workflow",
                    "resolve_root",
                    "project {} lookup failed while resolving workflow root: {}",
                    project_id,
                    err
                );
            }
        }
    }
    let ws = crate::paths::project_workspace_dir(project_id).ok()?;
    crate::util::ensure_dir_canonical(&ws).ok()
}

fn current_dir_string() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

#![cfg_attr(test, allow(clippy::needless_return))]

use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::runtime::Handle as TokioHandle;

use crate::plan::check_workflow_script_draft;
use crate::runtime_tasks::{cancel_runtime_task, RuntimeTaskKind};
use crate::session::SessionDB;

use super::types::{WorkflowOpState, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState};

#[derive(Clone, Copy)]
pub struct WorkflowTypedResultRuntime {
    pub output_schema: fn(&Value) -> Result<Option<Value>>,
    pub extract_result: fn(&str) -> Result<Value>,
    pub validate_value: fn(&Value, &Value) -> Vec<String>,
}

static WORKFLOW_TYPED_RESULT_RUNTIME: OnceLock<WorkflowTypedResultRuntime> = OnceLock::new();

pub fn register_workflow_typed_result_runtime(
    runtime: WorkflowTypedResultRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    WORKFLOW_TYPED_RESULT_RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("workflow typed-result runtime"))
}

#[cfg(test)]
#[path = "../../../ha-workflow/src/typed_result.rs"]
mod test_typed_result;

#[derive(Default)]
struct WorkflowRuntimeRegistry {
    active: std::collections::HashMap<String, ActiveWorkflowRuntime>,
    resume_after_pause: std::collections::HashMap<String, Arc<SessionDB>>,
}

struct ActiveWorkflowRuntime {
    flag: Arc<std::sync::atomic::AtomicBool>,
    session_id: String,
    admitted_pause_epoch: u64,
}

static WORKFLOW_RUNTIME_REGISTRY: std::sync::LazyLock<std::sync::Mutex<WorkflowRuntimeRegistry>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(WorkflowRuntimeRegistry::default()));

struct WorkflowRuntimeCancelGuard {
    run_id: String,
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl WorkflowRuntimeCancelGuard {
    fn register(run_id: &str, session_id: &str, admitted_pause_epoch: u64) -> Self {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        WORKFLOW_RUNTIME_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .insert(
                run_id.to_string(),
                ActiveWorkflowRuntime {
                    flag: flag.clone(),
                    session_id: session_id.to_string(),
                    admitted_pause_epoch,
                },
            );
        Self {
            run_id: run_id.to_string(),
            flag,
        }
    }
}

impl Drop for WorkflowRuntimeCancelGuard {
    fn drop(&mut self) {
        let resume_db = {
            let mut registry = WORKFLOW_RUNTIME_REGISTRY
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let removed = if registry
                .active
                .get(&self.run_id)
                .is_some_and(|current| Arc::ptr_eq(&current.flag, &self.flag))
            {
                registry.active.remove(&self.run_id);
                true
            } else {
                false
            };
            removed
                .then(|| registry.resume_after_pause.remove(&self.run_id))
                .flatten()
        };
        if let Some(db) = resume_db {
            if TokioHandle::try_current().is_ok() {
                spawn_workflow_run_if_primary(db, self.run_id.clone(), "session_continue_handoff");
            } else {
                crate::app_warn!(
                    "workflow",
                    "resume_handoff",
                    "Workflow runtime {} stopped outside a Tokio context; durable Running state remains recoverable",
                    self.run_id
                );
            }
        }
    }
}

/// Interrupt the live QuickJS worker for a durable session pause. The DB state
/// remains the restart truth; this flag only shortens convergence for a worker
/// that was already executing when Stop landed.
pub(crate) fn request_workflow_runtime_pause(run_id: &str) -> bool {
    let flag = WORKFLOW_RUNTIME_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .active
        .get(run_id)
        .map(|runtime| runtime.flag.clone());
    if let Some(flag) = flag {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub(crate) fn active_workflow_runtime_generations() -> Vec<(String, String, u64)> {
    WORKFLOW_RUNTIME_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .active
        .iter()
        .map(|(run_id, runtime)| {
            (
                run_id.clone(),
                runtime.session_id.clone(),
                runtime.admitted_pause_epoch,
            )
        })
        .collect()
}

#[cfg(test)]
mod autonomy_generation_tests {
    use super::*;

    #[test]
    fn active_runtime_exposes_its_admitted_stop_generation() {
        let run_id = format!("workflow-stop-generation-{}", uuid::Uuid::new_v4());
        let session_id = format!("session-stop-generation-{}", uuid::Uuid::new_v4());
        let guard = WorkflowRuntimeCancelGuard::register(&run_id, &session_id, 7);

        assert!(active_workflow_runtime_generations().contains(&(
            run_id.clone(),
            session_id.clone(),
            7,
        )));
        assert!(request_workflow_runtime_pause(&run_id));
        assert!(guard.flag.load(std::sync::atomic::Ordering::SeqCst));

        drop(guard);
        assert!(!active_workflow_runtime_generations()
            .into_iter()
            .any(|(active_run_id, _, _)| active_run_id == run_id));
    }
}

/// Launch a continued run only after its pre-Stop QuickJS worker has dropped
/// the active generation. The registry check and pending handoff share one
/// mutex, closing the drop-vs-Continue race without polling.
pub(crate) fn schedule_workflow_resume_after_pause(db: Arc<SessionDB>, run_id: String) {
    let deferred = {
        let mut registry = WORKFLOW_RUNTIME_REGISTRY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if registry.active.contains_key(&run_id) {
            registry
                .resume_after_pause
                .insert(run_id.clone(), db.clone());
            true
        } else {
            false
        }
    };
    if !deferred {
        spawn_workflow_run_if_primary(db, run_id, "session_continue");
    }
}

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
        .run(move |db| db.list_recoverable_workflow_runs())
        .await
        .context("list recoverable workflow runs")?;

    for run in runs {
        let claimed = {
            let run_id = run.id.clone();
            let owner = owner.clone();
            db.run(move |db| db.claim_workflow_run_for_recovery(&run_id, &owner))
                .await
                .with_context(|| format!("claim workflow run {} for recovery", run.id))?
        };
        let Some(claimed) = claimed else {
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
                let state = {
                    let claimed_id = claimed.id.clone();
                    db.run(move |db| db.get_workflow_run(&claimed_id)).await
                }
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
        let (injections, checkpoints) = crate::blocking::run_blocking(move || {
            let injections = recover_pending_workflow_milestone_injections(db.clone());
            let checkpoints = recover_terminal_workflow_agent_checkpoints(&db);
            (injections, checkpoints)
        })
        .await;
        match injections {
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
        match checkpoints {
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
        let loaded = {
            let run_id = run_id.clone();
            db.run(move |db| {
                let run = db.get_workflow_run(&run_id)?;
                run.map(|run| {
                    let paused = db.is_session_or_ancestor_autonomy_paused(&run.session_id)?;
                    Ok::<_, anyhow::Error>((run.state, paused))
                })
                .transpose()
            })
            .await
        };
        let state = match loaded {
            Ok(Some((_, true))) => {
                append_runtime_result_event_off_worker(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "skipped",
                        "accepted": true,
                        "reason": "session_paused",
                    }),
                )
                .await;
                crate::app_info!(
                    "workflow",
                    "spawn_run",
                    "skip workflow run {} launch while its session is paused",
                    run_id
                );
                return;
            }
            Ok(Some((state, false))) => state,
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
                let claimed = {
                    let run_id = run_id.clone();
                    let owner = owner.clone();
                    db.run(move |db| db.claim_workflow_run_for_launch(&run_id, &owner))
                        .await
                };
                match claimed {
                    Ok(Some(claimed)) => run_workflow_script_async(db.clone(), &claimed.id).await,
                    Ok(None) => {
                        append_runtime_result_event_off_worker(
                            &db,
                            &run_id,
                            &owner,
                            json!({
                                "status": "skipped",
                                "accepted": true,
                                "reason": "claim_unavailable",
                                "initialState": state.as_str(),
                            }),
                        )
                        .await;
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
                append_runtime_result_event_off_worker(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "skipped",
                        "accepted": true,
                        "reason": "state_not_launchable",
                        "initialState": state.as_str(),
                    }),
                )
                .await;
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
                append_runtime_result_event_off_worker(
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
                )
                .await;
                crate::app_info!(
                    "workflow",
                    "spawn_run",
                    "workflow run {} finished launch with state={}",
                    run_id,
                    result.snapshot.run.state.as_str()
                );
                {
                    let db_for_injection = db.clone();
                    let run_id = run_id.clone();
                    let owner = owner.clone();
                    db.run(move |_| {
                        maybe_spawn_workflow_result_injection(
                            db_for_injection,
                            &run_id,
                            &owner,
                            Some(&result),
                            None,
                        )
                    })
                    .await;
                }
            }
            Err(err) => {
                append_runtime_result_event_off_worker(
                    &db,
                    &run_id,
                    &owner,
                    json!({
                        "status": "error",
                        "accepted": true,
                        "reason": "runtime_error",
                        "error": err.to_string(),
                    }),
                )
                .await;
                crate::app_warn!(
                    "workflow",
                    "spawn_run",
                    "workflow run {} launch failed: {err:#}",
                    run_id
                );
                {
                    let db_for_injection = db.clone();
                    let run_id = run_id.clone();
                    let owner = owner.clone();
                    let error = err.to_string();
                    db.run(move |_| {
                        maybe_spawn_workflow_result_injection(
                            db_for_injection,
                            &run_id,
                            &owner,
                            None,
                            Some(&error),
                        )
                    })
                    .await;
                }
            }
        }
    });
    true
}

/// Off-worker twin of [`append_runtime_result_event`] for async contexts.
async fn append_runtime_result_event_off_worker(
    db: &Arc<SessionDB>,
    run_id: &str,
    owner: &str,
    payload: Value,
) {
    let run_id = run_id.to_string();
    let owner = owner.to_string();
    db.run(move |db| append_runtime_result_event(db, &run_id, &owner, payload))
        .await;
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

pub fn should_inject_workflow_milestone(event_type: &str, payload: &Value) -> bool {
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

pub fn maybe_spawn_workflow_milestone_injection(
    db: Arc<SessionDB>,
    run_id: &str,
    event_type: &str,
    event_seq: i64,
    payload: &Value,
) {
    spawn_workflow_milestone_injection(db, run_id, event_type, event_seq, payload, true);
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
    let armed_db = db.clone();
    let armed_run_id = run.id.clone();
    let armed_event_type = event_type.to_string();
    let armed_injection_run_id = injection_run_id.clone();
    let armed_child_run_id = payload
        .get("childRunId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let delivered_db = db.clone();
    let delivered_run_id = run.id.clone();
    let delivered_event_type = event_type.to_string();
    let delivered_injection_run_id = injection_run_id.clone();
    let delivered_child_run_id = payload
        .get("childRunId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let on_injected = crate::subagent::injection::OnInjected::new(
        move || {
            if armed_db.claim_workflow_milestone_injection_no_replay(
                &armed_run_id,
                &armed_event_type,
                event_seq,
                &armed_injection_run_id,
                armed_child_run_id.as_deref(),
            )? {
                Ok(())
            } else {
                bail!("workflow milestone already has a durable delivery owner")
            }
        },
        move || {
            if let Some(child_run_id) = delivered_child_run_id.as_deref() {
                if delivered_db.workflow_agent_result_handled(&delivered_run_id, child_run_id)? {
                    if !delivered_db.workflow_milestone_injection_settled(
                        &delivered_run_id,
                        &delivered_event_type,
                        event_seq,
                    )? {
                        delivered_db.append_workflow_event(
                            &delivered_run_id,
                            "workflow_milestone_injection_suppressed",
                            json!({
                                "sourceEventType": &delivered_event_type,
                                "sourceEventSeq": event_seq,
                                "injectionRunId": &delivered_injection_run_id,
                                "childRunId": child_run_id,
                                "reason": "agent_result_consumed_while_injection_pending",
                            }),
                        )?;
                    }
                    return Ok(());
                }
            }
            delivered_db.append_workflow_event(
                &delivered_run_id,
                "workflow_milestone_injection_delivered",
                json!({
                    "sourceEventType": &delivered_event_type,
                    "sourceEventSeq": event_seq,
                    "injectionRunId": &delivered_injection_run_id,
                }),
            )?;
            if let Some(child_run_id) = delivered_child_run_id.as_deref() {
                delivered_db.append_workflow_event(
                    &delivered_run_id,
                    "workflow_agent_result_consumed",
                    json!({
                        "api": "checkpoint_injection",
                        "childRunIds": [child_run_id],
                    }),
                )?;
                crate::subagent::mark_run_fetched(child_run_id);
            }
            Ok(())
        },
    );

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
    let (run, child_refs) = {
        let db = db.clone();
        let run_id = run_id.to_string();
        db.run(
            move |db| -> Result<(WorkflowRun, Vec<(RuntimeTaskKind, String)>)> {
                let run = db.cancel_workflow_run(&run_id)?;
                let child_refs = workflow_child_task_refs(db, &run_id)?;
                Ok((run, child_refs))
            },
        )
        .await?
    };
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
        let db = db.clone();
        let run_id = run_id.to_string();
        let _ = db
            .run(move |db| {
                db.append_workflow_event(
                    &run_id,
                    "run_child_cancel_requested",
                    json!({
                        "children": results,
                    }),
                )
            })
            .await;
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
            if let Ok(job_ids) = validation_child_job_ids(&child_handle) {
                refs.extend(
                    job_ids
                        .into_iter()
                        .map(|job_id| (RuntimeTaskKind::AsyncJob, job_id)),
                );
            }
        } else if op_type.starts_with("tool:") {
            refs.push((RuntimeTaskKind::AsyncJob, child_handle));
        } else if matches!(op_type.as_str(), "spawnAgent" | "resumeAgent") {
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

/// Outcome of the synchronous pre-launch validation chain, computed on the
/// blocking pool as one hop (`run_workflow_script_async` prologue).
enum ScriptLaunchPrep {
    AlreadyCompleted(Box<super::types::WorkflowRunSnapshot>),
    Ready {
        run: Box<super::types::WorkflowRun>,
        session_context: WorkflowSessionContext,
    },
}

fn prepare_workflow_script_launch(db: &SessionDB, run_id: &str) -> Result<ScriptLaunchPrep> {
    let run = db
        .get_workflow_run(run_id)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;

    if run.state == WorkflowRunState::Completed {
        return Ok(ScriptLaunchPrep::AlreadyCompleted(Box::new(
            db.workflow_run_snapshot(run_id, 500)?
                .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?,
        )));
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
        let preview = super::preview::preview_workflow_run(db, &run);
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

    let session_context = match workflow_session_context_for_run(db, &run) {
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
    Ok(ScriptLaunchPrep::Ready {
        run: Box::new(run),
        session_context,
    })
}

pub async fn run_workflow_script_async(
    db: Arc<SessionDB>,
    run_id: &str,
) -> Result<WorkflowRuntimeResult> {
    let prep = {
        let run_id = run_id.to_string();
        db.run(move |db| prepare_workflow_script_launch(db, &run_id))
            .await?
    };
    let (run, session_context) = match prep {
        ScriptLaunchPrep::AlreadyCompleted(snapshot) => {
            return Ok(WorkflowRuntimeResult {
                snapshot: *snapshot,
                output: None,
            });
        }
        ScriptLaunchPrep::Ready {
            run,
            session_context,
        } => (*run, session_context),
    };

    let tokio_handle = TokioHandle::current();
    let admitted_pause_epoch = {
        let session_id = run.session_id.clone();
        db.clone()
            .run(move |db| db.session_autonomy_lineage_pause_epoch(&session_id))
            .await?
    };
    let runtime_cancel =
        WorkflowRuntimeCancelGuard::register(run_id, &run.session_id, admitted_pause_epoch);
    let run_id_for_pause_check = run_id.to_string();
    let state_after_registration = db
        .clone()
        .run(move |db| {
            db.get_workflow_run(&run_id_for_pause_check)?
                .map(|run| {
                    let epoch = db.session_autonomy_lineage_pause_epoch(&run.session_id)?;
                    Ok::<_, anyhow::Error>((run.state, epoch))
                })
                .transpose()
        })
        .await?;
    if !matches!(
        state_after_registration,
        Some((WorkflowRunState::Running, epoch)) if epoch == admitted_pause_epoch
    ) || runtime_cancel
        .flag
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        runtime_cancel
            .flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        return Err(anyhow!(
            "workflow run {} changed state before runtime start",
            run_id
        ));
    }
    let db_for_script = db.clone();
    let run_for_script = run.clone();
    let runtime_cancel_flag = runtime_cancel.flag.clone();
    let output = match tokio::task::spawn_blocking(move || {
        execute_script(
            db_for_script,
            run_for_script,
            session_context,
            tokio_handle,
            runtime_cancel_flag,
        )
    })
    .await
    .context("workflow runtime worker panicked or was cancelled")?
    {
        Ok(output) => output,
        Err(err) => {
            if runtime_cancel
                .flag
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let run_id = run_id.to_string();
                let _ = db
                    .run(move |db| {
                        db.append_workflow_event(
                            &run_id,
                            "workflow_runtime_paused",
                            json!({ "reason": "session_stop" }),
                        )
                    })
                    .await;
            } else {
                let run_id = run_id.to_string();
                let _ = db
                    .run(move |db| {
                        db.transition_workflow_run(
                            &run_id,
                            WorkflowRunState::Failed,
                            Some("runtime_error"),
                        )
                    })
                    .await;
            }
            return Err(err);
        }
    };

    let snapshot = {
        let run_id = run_id.to_string();
        db.run(move |db| db.workflow_run_snapshot(&run_id, 500))
            .await?
    }
    .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
    Ok(WorkflowRuntimeResult {
        snapshot,
        output: Some(output),
    })
}

#[derive(Clone, Copy)]
pub struct WorkflowMachineRuntime {
    pub execute_script: fn(
        Arc<SessionDB>,
        WorkflowRun,
        WorkflowSessionContext,
        TokioHandle,
        Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Value>,
    pub has_required_autonomous_budget: fn(&WorkflowRun) -> bool,
    pub spawn_agent_tool_args: fn(&Value) -> Result<Value>,
    pub wait_all_tool_args: fn(&Value) -> Result<Value>,
    pub ensure_visible_agent_runs: fn(&SessionDB, &str, &[String], &str) -> Result<()>,
    pub wait_all_output_consumes_results: fn(&Value) -> bool,
    pub ask_user_tool_args: fn(&Value) -> Result<Value>,
    pub validation_exit_code: fn(&str) -> i64,
    pub validation_child_job_ids: fn(&str) -> Result<Vec<String>>,
}

fn has_required_autonomous_budget(run: &WorkflowRun) -> bool {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.has_required_autonomous_budget)(run);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::has_required_autonomous_budget(run);
    }
    #[cfg(not(test))]
    false
}

static WORKFLOW_MACHINE_RUNTIME: OnceLock<WorkflowMachineRuntime> = OnceLock::new();

pub fn register_workflow_machine_runtime(
    runtime: WorkflowMachineRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    WORKFLOW_MACHINE_RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("workflow machine runtime"))
}

#[cfg(test)]
#[path = "../../../ha-workflow/src/runtime_machine.rs"]
mod test_runtime_machine;

fn execute_script(
    db: Arc<SessionDB>,
    run: WorkflowRun,
    session_context: WorkflowSessionContext,
    tokio_handle: TokioHandle,
    runtime_cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<Value> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.execute_script)(db, run, session_context, tokio_handle, runtime_cancel);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::execute_script(
            db,
            run,
            session_context,
            tokio_handle,
            runtime_cancel,
        );
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}

pub fn workflow_output_schema(args: &Value) -> Result<Option<Value>> {
    if let Some(runtime) = WORKFLOW_TYPED_RESULT_RUNTIME.get() {
        return (runtime.output_schema)(args);
    }
    #[cfg(test)]
    {
        return test_typed_result::workflow_output_schema(args);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow typed-result runtime is not wired"))
}

pub fn extract_workflow_typed_result(raw: &str) -> Result<Value> {
    if let Some(runtime) = WORKFLOW_TYPED_RESULT_RUNTIME.get() {
        return (runtime.extract_result)(raw);
    }
    #[cfg(test)]
    {
        return test_typed_result::extract_workflow_typed_result(raw);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow typed-result runtime is not wired"))
}

pub fn validate_workflow_typed_value(schema: &Value, value: &Value) -> Vec<String> {
    if let Some(runtime) = WORKFLOW_TYPED_RESULT_RUNTIME.get() {
        return (runtime.validate_value)(schema, value);
    }
    #[cfg(test)]
    {
        return test_typed_result::validate_workflow_typed_value(schema, value);
    }
    #[cfg(not(test))]
    vec!["Workflow typed-result runtime is not wired".to_string()]
}

pub fn spawn_agent_tool_args(args: &Value) -> Result<Value> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.spawn_agent_tool_args)(args);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::spawn_agent_tool_args(args);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}

#[cfg(test)]
pub(crate) fn wait_all_tool_args(args: &Value) -> Result<Value> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.wait_all_tool_args)(args);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::wait_all_tool_args(args);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}

#[cfg(test)]
fn ensure_workflow_visible_agent_run_ids(
    db: &SessionDB,
    workflow_run_id: &str,
    run_ids: &[String],
    api: &str,
) -> Result<()> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.ensure_visible_agent_runs)(db, workflow_run_id, run_ids, api);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::ensure_workflow_visible_agent_run_ids(
            db,
            workflow_run_id,
            run_ids,
            api,
        );
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}

#[cfg(test)]
pub(crate) fn ensure_workflow_owned_agent_run_ids(
    db: &SessionDB,
    workflow_run_id: &str,
    run_ids: &[String],
    api: &str,
) -> Result<()> {
    ensure_workflow_visible_agent_run_ids(db, workflow_run_id, run_ids, api)
}

#[cfg(test)]
pub(crate) fn wait_all_output_consumes_results(output: &Value) -> bool {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.wait_all_output_consumes_results)(output);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::wait_all_output_consumes_results(output);
    }
    #[cfg(not(test))]
    false
}

#[cfg(test)]
pub(crate) fn ask_user_tool_args(args: &Value) -> Result<Value> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.ask_user_tool_args)(args);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::ask_user_tool_args(args);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}

#[cfg(test)]
pub(crate) fn validation_exit_code(output: &str) -> i64 {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.validation_exit_code)(output);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::validation_exit_code(output);
    }
    #[cfg(not(test))]
    0
}

fn validation_child_job_ids(child_handle: &str) -> Result<Vec<String>> {
    if let Some(runtime) = WORKFLOW_MACHINE_RUNTIME.get() {
        return (runtime.validation_child_job_ids)(child_handle);
    }
    #[cfg(test)]
    {
        return test_runtime_machine::validation_child_job_ids(child_handle);
    }
    #[cfg(not(test))]
    Err(anyhow!("Workflow QuickJS runtime is not wired"))
}
#[derive(Debug, Clone, Default)]
pub struct WorkflowSessionContext {
    pub session_id: String,
    pub working_dir: Option<String>,
    pub agent_id: Option<String>,
    pub session_mode: crate::permission::SessionMode,
    pub project_id: Option<String>,
    pub incognito: bool,
}

pub fn workflow_session_context(db: &SessionDB, session_id: &str) -> WorkflowSessionContext {
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
    if !worktree.purpose.is_owner_transport_safe() {
        bail!(
            "managed worktree {} cannot be restored by workflow runtime through the generic owner API",
            worktree_id
        );
    }
    if worktree.owner_session_id.as_deref() != Some(run.session_id.as_str()) {
        bail!(
            "managed worktree {} belongs to session {}; expected {}",
            worktree_id,
            worktree.owner_session_id.as_deref().unwrap_or("<none>"),
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

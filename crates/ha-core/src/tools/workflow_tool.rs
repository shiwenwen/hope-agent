use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::ToolExecContext;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowToolArgs {
    action: WorkflowToolAction,
    #[serde(default)]
    script: Option<String>,
    #[serde(default, alias = "script_source", alias = "scriptSource")]
    script_source: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "execution_mode", alias = "executionMode")]
    execution_mode: Option<String>,
    #[serde(default)]
    budget: Option<Value>,
    #[serde(default, alias = "api_version", alias = "apiVersion")]
    api_version: Option<i64>,
    #[serde(default)]
    meta: Option<Value>,
    #[serde(default)]
    args: Option<Value>,
    #[serde(default, alias = "resume_from_run_id", alias = "resumeFromRunId")]
    resume_from_run_id: Option<String>,
    #[serde(default, alias = "size_guideline", alias = "workflowSize")]
    size_guideline: Option<String>,
    #[serde(default, alias = "run_immediately", alias = "runImmediately")]
    run_immediately: Option<bool>,
    #[serde(default, alias = "parent_run_id", alias = "parentRunId")]
    parent_run_id: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default, alias = "goal_id", alias = "goalId")]
    goal_id: Option<String>,
    #[serde(default, alias = "goal_criterion_id", alias = "goalCriterionId")]
    goal_criterion_id: Option<String>,
    #[serde(default, alias = "worktree_id", alias = "worktreeId")]
    worktree_id: Option<String>,
    #[serde(default, alias = "run_id", alias = "runId")]
    run_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default, alias = "since_seq", alias = "sinceSeq")]
    since_seq: Option<i64>,
    #[serde(default, alias = "include_payload", alias = "includePayload")]
    include_payload: Option<bool>,
    #[serde(default)]
    command: Option<WorkflowControlCommand>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default, alias = "inherit_goal", alias = "inheritGoal")]
    inherit_goal: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WorkflowToolAction {
    Guide,
    Create,
    List,
    Status,
    Trace,
    Control,
    Followup,
}

impl WorkflowToolAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Guide => "guide",
            Self::Create => "create",
            Self::List => "list",
            Self::Status => "status",
            Self::Trace => "trace",
            Self::Control => "control",
            Self::Followup => "followup",
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WorkflowControlCommand {
    Pause,
    Resume,
    Cancel,
}

impl WorkflowControlCommand {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Cancel => "cancel",
        }
    }
}

pub async fn tool_workflow(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let input: WorkflowToolArgs = serde_json::from_value(args.clone())
        .map_err(|e| anyhow!("Invalid workflow arguments: {e}"))?;
    let (session_id, db) = resolve_workflow_ctx(ctx)?;

    let workflow_mode = {
        let sid = session_id.clone();
        db.run(move |db| db.get_session_workflow_mode(&sid))
            .await?
            .unwrap_or_default()
    };
    if !workflow_mode.enabled() {
        return Err(anyhow!(
            "Workflow Mode is off for this session. Use `/workflow on` or the GUI Workflow Mode toggle before using the workflow tool."
        ));
    }

    let output = match input.action {
        // Async arm: spawns / awaits child cancellation. Its own synchronous DB
        // segments are routed through `SessionDB::run` inside the fn.
        WorkflowToolAction::Control => {
            control_workflow(&input, &db, &session_id, workflow_mode).await?
        }
        // Every other arm is fully synchronous (DB reads/writes + JSON shaping);
        // run the whole dispatch on the blocking pool in one hop so the shared
        // write lock never pins the async worker.
        _ => {
            let dispatch_db = db.clone();
            let dispatch_session_id = session_id.clone();
            crate::blocking::run_blocking(move || -> Result<Value> {
                match input.action {
                    WorkflowToolAction::Guide => Ok(workflow_authoring_guide(workflow_mode)),
                    WorkflowToolAction::Create => {
                        create_workflow(&input, &dispatch_db, &dispatch_session_id, workflow_mode)
                    }
                    WorkflowToolAction::Followup => create_followup_workflow(
                        &input,
                        &dispatch_db,
                        &dispatch_session_id,
                        workflow_mode,
                    ),
                    WorkflowToolAction::List => {
                        list_workflows(&input, &dispatch_db, &dispatch_session_id, workflow_mode)
                    }
                    WorkflowToolAction::Status => {
                        workflow_status(&input, &dispatch_db, &dispatch_session_id, workflow_mode)
                    }
                    WorkflowToolAction::Trace => {
                        workflow_trace(&input, &dispatch_db, &dispatch_session_id, workflow_mode)
                    }
                    WorkflowToolAction::Control => unreachable!("handled above"),
                }
            })
            .await?
        }
    };

    Ok(serde_json::to_string_pretty(&output)?)
}

fn workflow_authoring_guide(workflow_mode: crate::workflow_mode::WorkflowMode) -> Value {
    json!({
        "apiVersion": 5,
        "workflowMode": workflow_mode.as_str(),
        "contract": [
            "Export default async function main(workflow, args). Use immutable workflow.meta/workflow.args for durable inputs.",
            "Use options objects. Labels are display-only; retain returned task and agent handles.",
            "Use shared_read_only only for analysis that cannot mutate; keep worktree isolation for editing children.",
            "Before finish, consume or cancel every required child. Runtime blocks rather than claiming false completion.",
            "After waitAny/waitAll, inspect failed status and terminalReason. Use resumeAgent only when continuity is required; use spawnAgent for independent verification.",
            "Every continuation policy must have a hard attempt bound. resumeRecommended is diagnostic only; never auto-resume user-stopped, approval-denied, or cancelled attempts.",
            "Permission, approval, Goal scope, incognito, connector and browser guards remain authoritative."
        ],
        "scriptShape": "export default async function main(workflow, args) { const task = await workflow.task.create({ title: '...' }); /* work */ await workflow.task.update({ task, status: 'completed' }); await workflow.finish({ summary, verification, residualRisk }); }",
        "runInputs": {
            "create": ["apiVersion", "meta", "args", "resumeFromRunId", "sizeGuideline", "budget"],
            "resume": "Only a terminal same-session run. Only the longest matching completed explicit shared_read_only agent prefix is reusable; side effects and worktrees never are."
        },
        "orchestration": [
            "workflow.parallel(label, items, spawnFn, { timeout?, partial?, resultMode?, reserveOutputTokens? })",
            "workflow.pipeline(label, items, spawnFn, consumeFn, { concurrency?, timeout?, resultMode?, reserveOutputTokens? })",
            "workflow.map(label, items, fn)",
            "workflow.waitAny(handles, { min?, timeout?, label? })",
            "workflow.waitAll(handles, { timeout?, partial?, resultMode?, label? })",
            "workflow.budgetStatus()"
        ],
        "children": [
            "workflow.spawnAgent({ task, label?, agent_id?, timeout_secs?, files?, injectPolicy?, resultMode?, isolation?, outputSchema?, schemaRetries?, reserveOutputTokens? })",
            "workflow.resumeAgent(handle, { task, label?, timeout?, model?, files?, injectPolicy?, resultMode? })",
            "workflow.acceptAgentFailure(handle, { reason })",
            "workflow.agentStatus(handles, { label? })",
            "workflow.agentResult(handle, { mode?, label? })",
            "workflow.agentSteer(handle, { message, label? })",
            "workflow.cancelAgent(handles, { reason?, label? })"
        ],
        "host": [
            "workflow.phase({ name, label?, expected?, criteriaIds?, injectPolicy? }, fn)",
            "workflow.progress({ message, phase?, percent?, counters?, payload?, importance? })",
            "workflow.checkpoint({ title, summary, phase?, importance?, inject?, findings?, evidence?, decisions?, next?, payload? })",
            "workflow.report({ title?, summary, nextAction?, needsUser?, inject?, payload? })",
            "workflow.fileSearch({ query, limit?, label? }) / read / grep / tool",
            "workflow.validate / review / verify / diff / askUser",
            "workflow.trace / block / repairLoop / now / random / finish; V5 finish blocks unresolved child failures unless an explicit allow_partial policy includes a reason"
        ],
        "typedResults": "For machine-consumed child output, provide a bounded outputSchema and schemaRetries. agentResult validates JSON, applies bounded read-only schema repair, and returns original/resolved run provenance.",
        "timing": "Use checkpoint injection for stage awareness, explicit status/result for coordinator-controlled consumption, waitAny or pipeline for early results, and waitAll only for a deliberate barrier."
    })
}

/// Cheap resolution (no DB IO) of the session id + `SessionDB` handle for the
/// workflow tool. The workflow-mode gate is read separately via
/// `SessionDB::run` so the synchronous SQLite lookup never pins the async
/// worker (see `crate::blocking`).
fn resolve_workflow_ctx(
    ctx: &ToolExecContext,
) -> Result<(String, std::sync::Arc<crate::session::SessionDB>)> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("workflow requires an active session"))?;
    if ctx.incognito {
        return Err(anyhow!(
            "workflow is disabled for incognito sessions because workflow runs are durable"
        ));
    }
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| anyhow!("Session DB not initialized"))?;
    Ok((session_id.to_string(), db))
}

fn create_workflow(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let script_source = input
        .script
        .clone()
        .or_else(|| input.script_source.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("workflow action=create requires `script` or `scriptSource`"))?;

    create_workflow_run_from_script(
        input,
        db,
        session_id,
        workflow_mode,
        script_source,
        input.parent_run_id.clone(),
        input
            .origin
            .clone()
            .or_else(|| Some("agent:workflow".to_string())),
        input.goal_id.clone(),
        input.goal_criterion_id.clone(),
        WorkflowToolAction::Create,
        None,
    )
}

fn create_followup_workflow(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let parent_run_id = input
        .parent_run_id
        .clone()
        .or_else(|| input.run_id.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("workflow action=followup requires `runId` or `parentRunId`"))?;
    let parent = visible_workflow_run(db, session_id, &parent_run_id)?;
    let script_source = input
        .script
        .clone()
        .or_else(|| input.script_source.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("workflow action=followup requires `script` or `scriptSource`"))?;
    let inherit_goal = input.inherit_goal.unwrap_or(true);
    let goal_id = input
        .goal_id
        .clone()
        .or_else(|| inherit_goal.then(|| parent.goal_id.clone()).flatten());
    let goal_criterion_id = input.goal_criterion_id.clone().or_else(|| {
        inherit_goal
            .then(|| parent.goal_criterion_id.clone())
            .flatten()
    });
    let origin = input.origin.clone().or_else(|| {
        Some(format!(
            "agent:workflow_followup:{}",
            parent_run_id.chars().take(48).collect::<String>()
        ))
    });

    create_workflow_run_from_script(
        input,
        db,
        session_id,
        workflow_mode,
        script_source,
        Some(parent_run_id),
        origin,
        goal_id,
        goal_criterion_id,
        WorkflowToolAction::Followup,
        workflow_size_guideline_from_budget(&parent.budget).map(str::to_string),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_workflow_run_from_script(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
    script_source: String,
    parent_run_id: Option<String>,
    origin: Option<String>,
    goal_id: Option<String>,
    goal_criterion_id: Option<String>,
    action: WorkflowToolAction,
    default_size_guideline: Option<String>,
) -> Result<Value> {
    let execution_mode = resolve_execution_mode(&db, session_id, input.execution_mode.as_deref())?;
    let size_guideline = resolve_workflow_size_guideline(
        input.size_guideline.as_deref(),
        default_size_guideline.as_deref(),
        workflow_mode,
    )?;
    let budget = budget_with_size_guideline(
        input.budget.clone().unwrap_or_else(|| json!({})),
        size_guideline,
    );
    let start_now = input.run_immediately.unwrap_or(true);
    if start_now {
        crate::workflow::ensure_workflow_launcher_primary().map_err(|e| {
            anyhow!(
                "workflow cannot start immediately: {e}. Retry from the primary runtime or set runImmediately=false to create a draft."
            )
        })?;
    }
    if matches!(
        execution_mode,
        crate::execution_mode::ExecutionMode::Autonomous
    ) && !has_required_autonomous_budget(&budget)
    {
        return Err(anyhow!(
            "workflow action={} with executionMode `autonomous` requires budget.maxScriptSecs or budget.maxRuntimeSecs plus budget.maxOutputTokens",
            action.as_str()
        ));
    }

    let preview = crate::workflow::ensure_workflow_script_can_create(
        &db,
        session_id,
        &script_source,
        Some(execution_mode.as_str()),
    )?;
    let run = db.create_workflow_run_with_control(
        crate::workflow::CreateWorkflowRunInput {
            session_id: session_id.to_string(),
            kind: input
                .kind
                .clone()
                .unwrap_or_else(|| "general.workflow".to_string()),
            execution_mode: execution_mode.as_str().to_string(),
            script_source,
            budget,
            parent_run_id,
            origin,
            goal_id,
            goal_criterion_id,
            worktree_id: input.worktree_id.clone(),
        },
        crate::workflow::WorkflowRunControlInput {
            api_version: input.api_version.unwrap_or(5),
            meta: input.meta.clone().unwrap_or_else(|| json!({})),
            args: input.args.clone().unwrap_or_else(|| json!({})),
            resume_from_run_id: input.resume_from_run_id.clone(),
        },
    )?;

    let launch_accepted = if start_now {
        crate::workflow::spawn_workflow_run_if_primary(
            db.clone(),
            run.id.clone(),
            format!("tool:workflow:pid:{}", std::process::id()),
        )
    } else {
        false
    };
    if start_now {
        debug_assert!(
            launch_accepted,
            "primary preflight should make launch accepted"
        );
    }
    let expected_next_state = if start_now {
        if preview.requires_approval {
            "awaiting_approval"
        } else {
            "running"
        }
    } else {
        "draft"
    };
    let message = if start_now {
        if preview.requires_approval {
            "Workflow run created and launch accepted. It is expected to stop at awaiting_approval until the user approves the permission preview."
        } else {
            "Workflow run created and launch accepted. Inspect, pause, resume, or cancel it in the Workflow control center."
        }
    } else {
        "Workflow run draft created. The user can start it from the Workflow control center."
    };

    Ok(json!({
        "ok": true,
        "action": action.as_str(),
        "runId": run.id,
        "state": run.state.as_str(),
        "initialState": run.state.as_str(),
        "expectedNextState": expected_next_state,
        "kind": run.kind,
        "executionMode": run.execution_mode,
        "sizeGuideline": workflow_size_guideline_from_budget(&run.budget).unwrap_or(size_guideline),
        "workflowMode": workflow_mode.as_str(),
        "goalId": run.goal_id,
        "goalCriterionId": run.goal_criterion_id,
        "startRequested": start_now,
        "launchAccepted": launch_accepted,
        "apiVersion": input.api_version.unwrap_or(5),
        "resumeFromRunId": input.resume_from_run_id,
        "requiresApproval": preview.requires_approval,
        "permissionSummary": preview.permission.summary,
        "message": message,
        "modelNextAction": if start_now {
            if preview.requires_approval { "wait_for_user_approval" } else { "continue_or_check_status" }
        } else {
            "tell_user_draft_created"
        }
    }))
}

fn resolve_workflow_size_guideline<'a>(
    requested: Option<&'a str>,
    inherited: Option<&'a str>,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<&'static str> {
    if let Some(value) = requested.and_then(normalized_size_guideline) {
        return Ok(value);
    }
    if requested.is_some() {
        return Err(anyhow!(
            "workflow sizeGuideline must be one of unrestricted, small, medium, or large"
        ));
    }
    if let Some(value) = inherited.and_then(normalized_size_guideline) {
        return Ok(value);
    }
    Ok(match workflow_mode {
        crate::workflow_mode::WorkflowMode::Ultracode => "large",
        crate::workflow_mode::WorkflowMode::Off | crate::workflow_mode::WorkflowMode::On => {
            "medium"
        }
    })
}

fn normalized_size_guideline(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "unrestricted" | "unbounded" | "open" => Some("unrestricted"),
        "small" | "s" => Some("small"),
        "medium" | "med" | "m" => Some("medium"),
        "large" | "l" => Some("large"),
        _ => None,
    }
}

fn budget_with_size_guideline(mut budget: Value, size_guideline: &str) -> Value {
    if !budget.is_object() {
        budget = json!({});
    }
    if let Some(object) = budget.as_object_mut() {
        object.insert(
            "sizeGuideline".to_string(),
            Value::String(size_guideline.to_string()),
        );
    }
    budget
}

fn workflow_size_guideline_from_budget(budget: &Value) -> Option<&str> {
    budget
        .get("sizeGuideline")
        .and_then(Value::as_str)
        .and_then(normalized_size_guideline)
}

fn list_workflows(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let requested_limit = input.limit.unwrap_or(20).clamp(1, 50);
    let scope = input.scope.as_deref().unwrap_or("active");
    let mut runs = db.list_workflow_runs_for_session(session_id, 100)?;
    runs.retain(|run| match scope {
        "active" => is_visible_active_state(run.state),
        "recent" | "session" => true,
        "goal" => match input.goal_id.as_deref() {
            Some(goal_id) => run.goal_id.as_deref() == Some(goal_id),
            None => run.goal_id.is_some(),
        },
        other => {
            crate::app_warn!(
                "workflow",
                "tool",
                "unknown workflow list scope `{}`; returning active runs",
                other
            );
            is_visible_active_state(run.state)
        }
    });
    runs.truncate(requested_limit);
    let active_count = runs
        .iter()
        .filter(|run| is_visible_active_state(run.state))
        .count();
    Ok(json!({
        "ok": true,
        "action": "list",
        "workflowMode": workflow_mode.as_str(),
        "scope": scope,
        "count": runs.len(),
        "activeCount": active_count,
        "runs": runs.iter().map(run_summary_json).collect::<Vec<_>>(),
        "modelNextAction": if runs.is_empty() {
            "create_workflow_if_task_warrants"
        } else if active_count > 0 {
            "call_workflow_status_for_active_run"
        } else {
            "inspect_recent_run_or_create_followup"
        }
    }))
}

fn workflow_status(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let run = if let Some(run_id) = normalized(input.run_id.as_deref()) {
        visible_workflow_run(db, session_id, run_id)?
    } else {
        select_relevant_workflow_run(db, session_id)?.ok_or_else(|| {
            anyhow!("workflow action=status found no workflow runs for this session")
        })?
    };
    let event_limit = input.limit.unwrap_or(80).clamp(1, 200);
    let snapshot = db
        .workflow_run_snapshot(&run.id, event_limit)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run.id))?;
    let ops_summary = ops_summary_json(&snapshot.ops);
    let latest_event = snapshot.events.last().map(event_summary_json);
    let latest_checkpoint = latest_checkpoint_json(&snapshot.events);
    Ok(json!({
        "ok": true,
        "action": "status",
        "workflowMode": workflow_mode.as_str(),
        "run": run_summary_json(&snapshot.run),
        "ops": ops_summary,
        "pendingActions": pending_actions_json(&snapshot),
        "latestEvent": latest_event,
        "latestCheckpoint": latest_checkpoint,
        "traceAvailable": snapshot.events.len(),
        "modelNextAction": model_next_action_for_run(&snapshot.run, latest_checkpoint.is_some()),
    }))
}

fn workflow_trace(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let run_id = normalized(input.run_id.as_deref())
        .ok_or_else(|| anyhow!("workflow action=trace requires `runId`"))?;
    let run = visible_workflow_run(db, session_id, run_id)?;
    let limit = input.limit.unwrap_or(80).clamp(1, 200);
    let include_payload = input.include_payload.unwrap_or(true);
    let since_seq = input.since_seq.unwrap_or(0);
    let events = db
        .list_workflow_events(&run.id, limit)?
        .into_iter()
        .filter(|event| event.seq > since_seq)
        .map(|event| {
            if include_payload {
                json!({
                    "seq": event.seq,
                    "type": event.event_type,
                    "createdAt": event.created_at,
                    "payload": event.payload,
                })
            } else {
                json!({
                    "seq": event.seq,
                    "type": event.event_type,
                    "createdAt": event.created_at,
                    "payloadSummary": payload_summary(&event.payload),
                })
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "ok": true,
        "action": "trace",
        "workflowMode": workflow_mode.as_str(),
        "run": run_summary_json(&run),
        "sinceSeq": since_seq,
        "count": events.len(),
        "events": events,
        "modelNextAction": model_next_action_for_run(&run, true),
    }))
}

async fn control_workflow(
    input: &WorkflowToolArgs,
    db: &std::sync::Arc<crate::session::SessionDB>,
    session_id: &str,
    workflow_mode: crate::workflow_mode::WorkflowMode,
) -> Result<Value> {
    let run_id = normalized(input.run_id.as_deref())
        .ok_or_else(|| anyhow!("workflow action=control requires `runId`"))?
        .to_string();
    let command = input
        .command
        .ok_or_else(|| anyhow!("workflow action=control requires `command`"))?;
    let run = match command {
        WorkflowControlCommand::Pause => {
            let sid = session_id.to_string();
            let rid = run_id.clone();
            db.run(move |db| {
                visible_workflow_run(db, &sid, &rid)?;
                db.pause_workflow_run(&rid)
            })
            .await?
        }
        WorkflowControlCommand::Resume => {
            let sid = session_id.to_string();
            let rid = run_id.clone();
            let run = db
                .run(move |db| {
                    visible_workflow_run(db, &sid, &rid)?;
                    db.resume_workflow_run(&rid)
                })
                .await?;
            let _ = crate::workflow::spawn_workflow_run_if_primary(
                db.clone(),
                run.id.clone(),
                format!("tool:workflow:control:pid:{}", std::process::id()),
            );
            run
        }
        WorkflowControlCommand::Cancel => {
            {
                let sid = session_id.to_string();
                let rid = run_id.clone();
                db.run(move |db| visible_workflow_run(db, &sid, &rid).map(|_| ()))
                    .await?;
            }
            crate::workflow::cancel_workflow_run_with_children(db.clone(), &run_id).await?
        }
    };
    let reason = normalized(input.reason.as_deref())
        .unwrap_or("model_control_requested")
        .to_string();
    {
        let run_id_for_event = run.id.clone();
        let result_state = run.state.as_str().to_string();
        let command_str = command.as_str().to_string();
        let _ = db
            .run(move |db| {
                db.append_workflow_event(
                    &run_id_for_event,
                    "run_model_control_action",
                    json!({
                        "action": command_str,
                        "reason": reason,
                        "resultState": result_state,
                        "accepted": true,
                        "surface": "model_control",
                    }),
                )
            })
            .await;
    }
    Ok(json!({
        "ok": true,
        "action": "control",
        "workflowMode": workflow_mode.as_str(),
        "command": command.as_str(),
        "run": run_summary_json(&run),
        "message": match command {
            WorkflowControlCommand::Pause => "Workflow run paused.",
            WorkflowControlCommand::Resume => "Workflow run resumed and launch was requested from the primary runtime.",
            WorkflowControlCommand::Cancel => "Workflow run cancelled; child tasks were asked to stop when possible.",
        },
        "modelNextAction": match command {
            WorkflowControlCommand::Pause => "explain_pause_to_user",
            WorkflowControlCommand::Resume => "monitor_or_continue",
            WorkflowControlCommand::Cancel => "explain_cancel_to_user",
        },
    }))
}

fn resolve_execution_mode(
    db: &crate::session::SessionDB,
    session_id: &str,
    requested: Option<&str>,
) -> Result<crate::execution_mode::ExecutionMode> {
    if let Some(raw) = requested {
        return crate::execution_mode::ExecutionMode::from_str(raw)
            .ok_or_else(|| anyhow!("Invalid workflow executionMode `{raw}`"));
    }
    let session_mode = db
        .get_session_execution_mode(session_id)?
        .unwrap_or_default();
    Ok(match session_mode {
        crate::execution_mode::ExecutionMode::Off => crate::execution_mode::ExecutionMode::Guarded,
        mode => mode,
    })
}

fn has_required_autonomous_budget(budget: &Value) -> bool {
    let runtime = optional_positive_u64(
        budget,
        &[
            "maxScriptSecs",
            "max_script_secs",
            "maxRuntimeSecs",
            "max_runtime_secs",
        ],
    )
    .is_some();
    let output = optional_positive_u64(budget, &["maxOutputTokens", "max_output_tokens"]).is_some();
    runtime && output
}

fn optional_positive_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .filter(|n| *n > 0)
}

fn normalized(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn visible_workflow_run(
    db: &crate::session::SessionDB,
    session_id: &str,
    run_id: &str,
) -> Result<crate::workflow::WorkflowRun> {
    let run = db
        .get_workflow_run(run_id)?
        .ok_or_else(|| anyhow!("workflow run {} not found", run_id))?;
    if run.session_id != session_id {
        return Err(anyhow!(
            "workflow run {} belongs to a different session and is not visible to this model",
            run_id
        ));
    }
    Ok(run)
}

fn select_relevant_workflow_run(
    db: &crate::session::SessionDB,
    session_id: &str,
) -> Result<Option<crate::workflow::WorkflowRun>> {
    let runs = db.list_workflow_runs_for_session(session_id, 100)?;
    Ok(runs
        .iter()
        .find(|run| is_visible_active_state(run.state))
        .cloned()
        .or_else(|| runs.into_iter().next()))
}

fn is_visible_active_state(state: crate::workflow::WorkflowRunState) -> bool {
    matches!(
        state,
        crate::workflow::WorkflowRunState::Draft
            | crate::workflow::WorkflowRunState::AwaitingApproval
            | crate::workflow::WorkflowRunState::Running
            | crate::workflow::WorkflowRunState::AwaitingUser
            | crate::workflow::WorkflowRunState::Paused
            | crate::workflow::WorkflowRunState::Recovering
    )
}

fn run_summary_json(run: &crate::workflow::WorkflowRun) -> Value {
    json!({
        "runId": run.id,
        "sessionId": run.session_id,
        "kind": run.kind,
        "state": run.state.as_str(),
        "executionMode": run.execution_mode,
        "sizeGuideline": workflow_size_guideline_from_budget(&run.budget),
        "runtimeCaps": workflow_runtime_caps_json(&run.budget),
        "origin": run.origin,
        "parentRunId": run.parent_run_id,
        "goalId": run.goal_id,
        "goalCriterionId": run.goal_criterion_id,
        "goalCriterionText": run.goal_criterion_text,
        "goalCriterionKind": run.goal_criterion_kind,
        "goalRevision": run.goal_revision,
        "worktreeId": run.worktree_id,
        "blockedReason": run.blocked_reason,
        "cursorSeq": run.cursor_seq,
        "createdAt": run.created_at,
        "updatedAt": run.updated_at,
        "completedAt": run.completed_at,
    })
}

fn workflow_runtime_caps_json(budget: &Value) -> Value {
    json!({
        "maxScriptSecs": budget.get("maxScriptSecs").or_else(|| budget.get("max_script_secs")),
        "maxRuntimeSecs": budget.get("maxRuntimeSecs").or_else(|| budget.get("max_runtime_secs")),
        "maxOps": budget.get("maxOps").or_else(|| budget.get("max_ops")),
        "maxOutputTokens": budget.get("maxOutputTokens").or_else(|| budget.get("max_output_tokens")),
    })
}

fn ops_summary_json(ops: &[crate::workflow::WorkflowOp]) -> Value {
    let mut pending = 0usize;
    let mut started = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    for op in ops {
        match op.state {
            crate::workflow::WorkflowOpState::Pending => pending += 1,
            crate::workflow::WorkflowOpState::Started => started += 1,
            crate::workflow::WorkflowOpState::Completed => completed += 1,
            crate::workflow::WorkflowOpState::Failed => failed += 1,
        }
    }
    json!({
        "total": ops.len(),
        "pending": pending,
        "started": started,
        "completed": completed,
        "failed": failed,
        "recent": ops.iter().rev().take(8).map(|op| {
            json!({
                "opKey": op.op_key,
                "opType": op.op_type,
                "state": op.state.as_str(),
                "startedAt": op.started_at,
                "completedAt": op.completed_at,
                "hasOutput": op.output.is_some(),
                "hasError": op.error.is_some(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn pending_actions_json(snapshot: &crate::workflow::WorkflowRunSnapshot) -> Vec<Value> {
    let mut actions = Vec::new();
    match snapshot.run.state {
        crate::workflow::WorkflowRunState::AwaitingApproval => actions.push(json!({
            "kind": "user_approval",
            "severity": "blocking",
            "message": "Workflow is waiting for explicit user approval. The model cannot approve it."
        })),
        crate::workflow::WorkflowRunState::AwaitingUser => actions.push(json!({
            "kind": "user_input",
            "severity": "blocking",
            "message": "Workflow is waiting for user input."
        })),
        crate::workflow::WorkflowRunState::Paused => actions.push(json!({
            "kind": "paused",
            "severity": "waiting",
            "message": "Workflow is paused and can be resumed or cancelled."
        })),
        crate::workflow::WorkflowRunState::Blocked => actions.push(json!({
            "kind": "blocked",
            "severity": "blocking",
            "message": snapshot.run.blocked_reason.clone().unwrap_or_else(|| "Workflow is blocked.".to_string())
        })),
        crate::workflow::WorkflowRunState::Failed => actions.push(json!({
            "kind": "failed",
            "severity": "blocking",
            "message": "Workflow failed; inspect trace before creating a follow-up."
        })),
        _ => {}
    }
    let failed_ops = snapshot
        .ops
        .iter()
        .filter(|op| op.state == crate::workflow::WorkflowOpState::Failed)
        .take(5)
        .map(|op| {
            json!({
                "kind": "failed_op",
                "severity": "warning",
                "opKey": op.op_key,
                "opType": op.op_type,
            })
        });
    actions.extend(failed_ops);
    actions
}

fn event_summary_json(event: &crate::workflow::WorkflowEvent) -> Value {
    json!({
        "seq": event.seq,
        "type": event.event_type,
        "createdAt": event.created_at,
        "payloadSummary": payload_summary(&event.payload),
    })
}

fn latest_checkpoint_json(events: &[crate::workflow::WorkflowEvent]) -> Option<Value> {
    events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event.event_type.as_str(),
                "trace"
                    | "workflow_phase_started"
                    | "workflow_phase_completed"
                    | "workflow_phase_failed"
                    | "workflow_checkpoint"
                    | "workflow_report"
                    | "workflow_block_requested"
                    | "workflow_finish"
                    | "run_runtime_result"
                    | "run_state_changed"
            )
        })
        .map(|event| {
            json!({
                "seq": event.seq,
                "type": event.event_type,
                "createdAt": event.created_at,
                "payload": event.payload,
            })
        })
}

fn payload_summary(payload: &Value) -> String {
    match payload {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.chars().take(160).collect(),
        Value::Array(items) => format!("array[{}]", items.len()),
        Value::Object(map) => {
            let keys = map.keys().take(8).cloned().collect::<Vec<_>>().join(", ");
            format!("object{{{keys}}}")
        }
    }
}

fn model_next_action_for_run(
    run: &crate::workflow::WorkflowRun,
    has_checkpoint: bool,
) -> &'static str {
    match run.state {
        crate::workflow::WorkflowRunState::Completed => "summarize_workflow_result_to_user",
        crate::workflow::WorkflowRunState::Failed => {
            "inspect_trace_then_explain_failure_or_create_followup"
        }
        crate::workflow::WorkflowRunState::Cancelled => "tell_user_workflow_was_cancelled",
        crate::workflow::WorkflowRunState::Blocked => "explain_blocker_and_recovery_options",
        crate::workflow::WorkflowRunState::AwaitingApproval => {
            "ask_user_to_review_workflow_approval"
        }
        crate::workflow::WorkflowRunState::AwaitingUser => "ask_user_for_required_input",
        crate::workflow::WorkflowRunState::Paused => "resume_or_cancel_when_user_confirms",
        crate::workflow::WorkflowRunState::Running
        | crate::workflow::WorkflowRunState::Recovering => {
            if has_checkpoint {
                "use_checkpoint_or_continue_monitoring"
            } else {
                "continue_monitoring"
            }
        }
        crate::workflow::WorkflowRunState::Draft => "start_or_explain_draft_to_user",
    }
}

#[cfg(test)]
mod tests {
    use super::workflow_authoring_guide;
    use crate::workflow_mode::WorkflowMode;

    #[test]
    fn authoring_guide_exposes_v5_contract_on_demand() {
        let guide = workflow_authoring_guide(WorkflowMode::On);
        assert_eq!(guide["apiVersion"], 5);
        assert!(guide["orchestration"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item.as_str()
                    .is_some_and(|item| item.contains("workflow.pipeline"))
            })));
        assert!(guide["children"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| {
                item.as_str()
                    .is_some_and(|item| item.contains("outputSchema"))
            })));
        assert!(guide["typedResults"]
            .as_str()
            .is_some_and(|value| value.contains("schema repair")));
    }
}

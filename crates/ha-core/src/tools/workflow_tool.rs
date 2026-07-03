use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::ToolExecContext;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunToolArgs {
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
    #[serde(default, alias = "run_immediately", alias = "runImmediately")]
    run_immediately: Option<bool>,
    #[serde(default, alias = "parent_run_id", alias = "parentRunId")]
    parent_run_id: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default, alias = "goal_id", alias = "goalId")]
    goal_id: Option<String>,
    #[serde(default, alias = "worktree_id", alias = "worktreeId")]
    worktree_id: Option<String>,
}

pub async fn tool_workflow_run(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let input: WorkflowRunToolArgs = serde_json::from_value(args.clone())
        .map_err(|e| anyhow!("Invalid workflow_run arguments: {e}"))?;
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow!("workflow_run requires an active session"))?;
    if ctx.incognito {
        return Err(anyhow!(
            "workflow_run is disabled for incognito sessions because workflow runs are durable"
        ));
    }
    let db = ctx
        .session_db
        .as_ref()
        .map(|handle| handle.0.clone())
        .or_else(|| crate::get_session_db().cloned())
        .ok_or_else(|| anyhow!("Session DB not initialized"))?;
    let workflow_mode = db
        .get_session_workflow_mode(session_id)?
        .unwrap_or_default();
    if !workflow_mode.enabled() {
        return Err(anyhow!(
            "Workflow Mode is off for this session. Use `/workflow on` or the GUI Workflow Mode toggle before creating workflow runs."
        ));
    }

    let script_source = input
        .script
        .or(input.script_source)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("workflow_run requires `script` or `scriptSource`"))?;

    let execution_mode = resolve_execution_mode(&db, session_id, input.execution_mode.as_deref())?;
    let budget = input.budget.unwrap_or_else(|| json!({}));
    let start_now = input.run_immediately.unwrap_or(true);
    if start_now {
        crate::workflow::ensure_workflow_launcher_primary().map_err(|e| {
            anyhow!(
                "workflow_run cannot start immediately: {e}. Retry from the primary runtime or set runImmediately=false to create a draft."
            )
        })?;
    }
    if matches!(
        execution_mode,
        crate::execution_mode::ExecutionMode::Autonomous
    ) && !has_required_autonomous_budget(&budget)
    {
        return Err(anyhow!(
            "workflow_run with executionMode `autonomous` requires budget.maxScriptSecs or budget.maxRuntimeSecs plus budget.maxOutputTokens"
        ));
    }

    let preview = crate::workflow::ensure_workflow_script_can_create(
        &db,
        session_id,
        &script_source,
        Some(execution_mode.as_str()),
    )?;
    let run = db.create_workflow_run(crate::workflow::CreateWorkflowRunInput {
        session_id: session_id.to_string(),
        kind: input.kind.unwrap_or_else(|| "general.workflow".to_string()),
        execution_mode: execution_mode.as_str().to_string(),
        script_source,
        budget,
        parent_run_id: input.parent_run_id,
        origin: input
            .origin
            .or_else(|| Some("agent:workflow_run".to_string())),
        goal_id: input.goal_id,
        worktree_id: input.worktree_id,
    })?;

    let launch_accepted = if start_now {
        crate::workflow::spawn_workflow_run_if_primary(
            db.clone(),
            run.id.clone(),
            format!("tool:workflow_run:pid:{}", std::process::id()),
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

    Ok(serde_json::to_string_pretty(&json!({
        "runId": run.id,
        "state": run.state.as_str(),
        "initialState": run.state.as_str(),
        "expectedNextState": expected_next_state,
        "kind": run.kind,
        "executionMode": run.execution_mode,
        "workflowMode": workflow_mode.as_str(),
        "startRequested": start_now,
        "launchAccepted": launch_accepted,
        "requiresApproval": preview.requires_approval,
        "permissionSummary": preview.permission.summary,
        "message": message
    }))?)
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

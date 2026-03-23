use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

use super::ToolExecContext;
use crate::subagent::{self, SpawnParams, SubagentStatus};

/// Tool handler for the `subagent` tool.
/// Actions: spawn, check, list, result, kill, kill_all
pub(crate) async fn tool_subagent(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match action {
        "spawn" => action_spawn(args, ctx).await,
        "check" => action_check(args).await,
        "list" => action_list(ctx).await,
        "result" => action_result(args).await,
        "kill" => action_kill(args).await,
        "kill_all" => action_kill_all(ctx).await,
        _ => Err(anyhow::anyhow!(
            "Unknown subagent action '{}'. Valid actions: spawn, check, list, result, kill, kill_all",
            action
        )),
    }
}

async fn action_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let task = args.get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'task' is required for spawn action"))?;

    let agent_id = args.get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let timeout_secs = args.get("timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|t| t.min(1800)); // Cap at 30 minutes

    let model_override = args.get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let parent_session_id = ctx.session_id.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context — cannot spawn sub-agent outside a chat session"))?;

    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or("default");

    // Check agent-level permission
    let agent_def = crate::agent_loader::load_agent(parent_agent_id).ok();
    if let Some(ref def) = agent_def {
        if !def.config.subagents.enabled {
            return Err(anyhow::anyhow!("Sub-agent delegation is disabled for this agent"));
        }
        if !def.config.subagents.is_agent_allowed(&agent_id) {
            return Err(anyhow::anyhow!("Agent '{}' is not in the allowed delegation list", agent_id));
        }
    }

    let session_db = get_session_db()?;
    let cancel_registry = get_cancel_registry()?;

    let params = SpawnParams {
        task: task.to_string(),
        agent_id,
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: parent_agent_id.to_string(),
        depth: ctx.subagent_depth + 1,
        timeout_secs,
        model_override,
    };

    let run_id = subagent::spawn_subagent(params, session_db, cancel_registry).await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "spawned",
        "run_id": run_id,
        "message": "Sub-agent spawned. Use subagent(action='check', run_id='...') to poll for completion."
    }))?)
}

async fn action_check(args: &Value) -> Result<String> {
    let run_id = args.get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for check action"))?;

    // wait=true: poll until completion (default timeout 60s, max 300s)
    let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);
    let wait_timeout = args.get("wait_timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(300);

    let session_db = get_session_db()?;

    let run = if wait {
        // Poll DB every 2s until terminal or timeout
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_timeout);
        loop {
            let r = session_db.get_subagent_run(run_id)?
                .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;
            if r.status.is_terminal() {
                break r;
            }
            if std::time::Instant::now() >= deadline {
                break r; // Return current (non-terminal) status on timeout
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    } else {
        session_db.get_subagent_run(run_id)?
            .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?
    };

    let mut response = serde_json::json!({
        "run_id": run.run_id,
        "status": run.status.as_str(),
        "child_agent_id": run.child_agent_id,
        "task": truncate(&run.task, 100),
        "depth": run.depth,
    });

    if run.status.is_terminal() {
        if let Some(ref result) = run.result {
            response["result"] = serde_json::Value::String(result.clone());
        }
        if let Some(ref error) = run.error {
            response["error"] = serde_json::Value::String(error.clone());
        }
        if let Some(ms) = run.duration_ms {
            response["duration_ms"] = serde_json::Value::Number(ms.into());
        }
        if let Some(ref model) = run.model_used {
            response["model_used"] = serde_json::Value::String(model.clone());
        }
        // Mark as fetched so auto-injection is skipped
        crate::subagent::mark_run_fetched(run_id);
    }

    Ok(serde_json::to_string_pretty(&response)?)
}

async fn action_list(ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx.session_id.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let session_db = get_session_db()?;
    let runs = session_db.list_subagent_runs(parent_session_id)?;

    let items: Vec<serde_json::Value> = runs.iter().map(|r| {
        serde_json::json!({
            "run_id": r.run_id,
            "child_agent_id": r.child_agent_id,
            "task": truncate(&r.task, 80),
            "status": r.status.as_str(),
            "depth": r.depth,
            "started_at": r.started_at,
            "duration_ms": r.duration_ms,
        })
    }).collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "total": items.len(),
        "runs": items,
    }))?)
}

async fn action_result(args: &Value) -> Result<String> {
    let run_id = args.get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for result action"))?;

    let session_db = get_session_db()?;
    let run = session_db.get_subagent_run(run_id)?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

    if !run.status.is_terminal() {
        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run.run_id,
            "status": run.status.as_str(),
            "message": "Sub-agent is still running. Use check to poll status."
        }))?);
    }

    // Mark as fetched so auto-injection is skipped
    crate::subagent::mark_run_fetched(run_id);

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "run_id": run.run_id,
        "status": run.status.as_str(),
        "result": run.result,
        "error": run.error,
        "model_used": run.model_used,
        "duration_ms": run.duration_ms,
    }))?)
}

async fn action_kill(args: &Value) -> Result<String> {
    let run_id = args.get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for kill action"))?;

    let cancel_registry = get_cancel_registry()?;
    let session_db = get_session_db()?;

    // Verify the run exists and is active
    let run = session_db.get_subagent_run(run_id)?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

    if run.status.is_terminal() {
        return Ok(format!("Sub-agent run '{}' already in terminal state: {}", run_id, run.status.as_str()));
    }

    let cancelled = cancel_registry.cancel(run_id);
    if cancelled {
        Ok(format!("Kill signal sent to sub-agent run '{}'", run_id))
    } else {
        // Update DB directly if no cancel flag found (already cleaned up)
        let _ = session_db.update_subagent_status(
            run_id, SubagentStatus::Killed,
            None, Some("Killed by parent agent"), None, None,
        );
        Ok(format!("Sub-agent run '{}' marked as killed", run_id))
    }
}

async fn action_kill_all(ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx.session_id.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let cancel_registry = get_cancel_registry()?;
    let session_db = get_session_db()?;
    let count = cancel_registry.cancel_all_for_session(parent_session_id, &session_db);

    Ok(format!("Kill signal sent to {} active sub-agent(s)", count))
}

// ── Helpers ─────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{}...", cut)
    }
}

fn get_session_db() -> Result<Arc<crate::session::SessionDB>> {
    crate::get_session_db()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Session DB not initialized"))
}

fn get_cancel_registry() -> Result<Arc<subagent::SubagentCancelRegistry>> {
    crate::get_subagent_cancels()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Sub-agent cancel registry not initialized"))
}

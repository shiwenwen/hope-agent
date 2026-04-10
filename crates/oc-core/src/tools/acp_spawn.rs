//! Tool handler for the `acp_spawn` tool.
//! Actions: spawn, check, list, result, kill, kill_all, steer, backends

use anyhow::Result;
use serde_json::Value;

use super::ToolExecContext;
use crate::acp_control::types::AcpCreateParams;

pub(crate) async fn tool_acp_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "spawn" => action_spawn(args, ctx).await,
        "check" => action_check(args).await,
        "list" => action_list(ctx).await,
        "result" => action_result(args).await,
        "kill" => action_kill(args).await,
        "kill_all" => action_kill_all(ctx).await,
        "steer" => action_steer(args).await,
        "backends" => action_backends().await,
        _ => Err(anyhow::anyhow!(
            "Unknown acp_spawn action '{}'. Valid actions: spawn, check, list, result, kill, kill_all, steer, backends",
            action
        )),
    }
}

async fn action_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let backend = args
        .get("backend")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'backend' is required for spawn action"))?;

    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'task' is required for spawn action"))?;

    let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("No session context — cannot spawn ACP agent outside a chat session")
    })?;

    // Check agent-level ACP permission
    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or("default");
    if let Ok(def) = crate::agent_loader::load_agent(parent_agent_id) {
        if !def.config.acp.enabled {
            return Err(anyhow::anyhow!(
                "ACP external agent delegation is disabled for this agent"
            ));
        }
        if !def.config.acp.is_backend_allowed(backend) {
            return Err(anyhow::anyhow!(
                "ACP backend '{}' is not allowed for this agent",
                backend
            ));
        }
    }

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    // Check global config
    let store = crate::config::cached_config();
    if !store.acp_control.enabled {
        return Err(anyhow::anyhow!(
            "ACP control plane is disabled. Enable it in Settings → ACP."
        ));
    }

    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|t| t.min(3600));
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let params = AcpCreateParams {
        cwd,
        system_prompt: None,
        model,
        timeout_secs,
        resume_session_id: None,
    };

    let run_id = manager
        .spawn_run(backend, task, params, parent_session_id, label)
        .await?;

    Ok(serde_json::json!({
        "status": "accepted",
        "run_id": run_id,
        "backend": backend,
        "note": "ACP agent spawned. Results will be pushed when complete. Use check(run_id) to poll status."
    })
    .to_string())
}

async fn action_check(args: &Value) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for check action"))?;

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);

    if wait {
        // Poll until terminal state
        for _ in 0..600 {
            if let Some(run) = manager.check_run(run_id).await {
                if run.status.is_terminal() {
                    return Ok(serde_json::to_string(&run)?);
                }
            } else {
                // Try DB fallback
                if let Some(db) = crate::get_session_db() {
                    if let Ok(Some(run)) = db.get_acp_run(run_id) {
                        return Ok(serde_json::to_string_pretty(&run)?);
                    }
                }
                return Err(anyhow::anyhow!("Run not found: {}", run_id));
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        return Err(anyhow::anyhow!(
            "Timed out waiting for run {} to complete",
            run_id
        ));
    }

    if let Some(run) = manager.check_run(run_id).await {
        Ok(serde_json::to_string(&run)?)
    } else {
        // Fallback to DB for finished runs
        if let Some(db) = crate::get_session_db() {
            if let Ok(Some(run)) = db.get_acp_run(run_id) {
                return Ok(serde_json::to_string_pretty(&run)?);
            }
        }
        Err(anyhow::anyhow!("Run not found: {}", run_id))
    }
}

async fn action_list(ctx: &ToolExecContext) -> Result<String> {
    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    let runs = manager.list_runs(ctx.session_id.as_deref()).await;

    Ok(serde_json::to_string(&runs)?)
}

async fn action_result(args: &Value) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for result action"))?;

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    manager.get_result(run_id).await
}

async fn action_kill(args: &Value) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for kill action"))?;

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    manager.kill_run(run_id).await?;
    Ok(format!("ACP run {} killed.", run_id))
}

async fn action_kill_all(ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    let count = manager.kill_all(parent_session_id).await?;
    Ok(format!("Killed {} ACP run(s).", count))
}

async fn action_steer(args: &Value) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for steer action"))?;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'message' is required for steer action"))?;

    let manager = crate::get_acp_manager()
        .ok_or_else(|| anyhow::anyhow!("ACP control plane not initialized"))?;

    manager.steer_run(run_id, message).await?;
    Ok(format!("Sent steer message to ACP run {}.", run_id))
}

async fn action_backends() -> Result<String> {
    let store = crate::config::cached_config();
    if !store.acp_control.enabled {
        return Ok(serde_json::json!({
            "enabled": false,
            "note": "ACP control plane is disabled. Enable it in Settings → ACP."
        })
        .to_string());
    }

    // Use registry if available, otherwise build from config
    if let Some(manager) = crate::get_acp_manager() {
        // Registry is initialized — list from it
        // For now, build info from config + health checks
        let config = &store.acp_control;
        let mut backends = Vec::new();
        for b in &config.backends {
            if !b.enabled {
                continue;
            }
            let path = if std::path::Path::new(&b.binary).is_absolute() {
                Some(b.binary.clone())
            } else {
                crate::acp_control::registry::resolve_binary(&b.binary)
            };
            backends.push(serde_json::json!({
                "id": b.id,
                "name": b.name,
                "enabled": b.enabled,
                "binary": b.binary,
                "binaryPath": path,
                "available": path.is_some(),
            }));
        }
        let _ = manager; // suppress unused warning
        Ok(serde_json::json!({
            "enabled": true,
            "backends": backends,
        })
        .to_string())
    } else {
        Ok(serde_json::json!({
            "enabled": true,
            "note": "ACP control plane not yet initialized.",
            "backends": []
        })
        .to_string())
    }
}

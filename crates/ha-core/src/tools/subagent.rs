use anyhow::{Context as _, Result};
use serde_json::Value;
use std::sync::Arc;

use super::ToolExecContext;
use crate::agent_config::AgentConfig;
use crate::agent_loader::DEFAULT_AGENT_ID;
use crate::subagent::{self, SpawnParams, SubagentStatus};

pub(crate) const WORKFLOW_PREALLOCATED_RUN_ID_ARG: &str = "__hope_workflow_preallocated_run_id";
pub(crate) const WORKFLOW_SKIP_PARENT_INJECTION_ARG: &str = "__hope_workflow_skip_parent_injection";
pub(crate) const WORKFLOW_ISOLATION_ARG: &str = "__hope_workflow_isolation";

fn workflow_shared_read_only_mode() -> crate::agent::PlanAgentMode {
    crate::agent::PlanAgentMode::PlanAgent {
        allowed_tools: [
            "read",
            "ls",
            "grep",
            "find",
            "lsp",
            "glob",
            "web_search",
            "web_fetch",
            "ask_user_question",
            "recall_memory",
            "memory_get",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        ask_tools: Vec::new(),
    }
}

/// Look up the dispatcher's verdict on the `subagent` Tier 3 tool for the
/// given agent. Used by the runtime spawn gate (`tools::subagent`) and the
/// system-prompt guidance section so both reach the same conclusion.
pub(crate) fn subagent_capability_enabled(agent_id: &str, agent_config: &AgentConfig) -> bool {
    let app_config = crate::config::cached_config();
    let ctx = super::dispatch::DispatchContext {
        agent_id,
        incognito: false,
        mcp_enabled: agent_config.capabilities.mcp_enabled,
        memory_enabled: agent_config.memory.enabled,
        tools_filter: &agent_config.capabilities.tools,
        app_config: &app_config,
    };
    let def = super::dispatch::all_dispatchable_tools()
        .iter()
        .find(|t| t.name == super::TOOL_SUBAGENT);
    match def {
        Some(d) => !matches!(
            super::dispatch::resolve_tool_fate(d, &ctx),
            super::dispatch::ToolFate::Hidden
        ),
        None => false,
    }
}

/// Enforce the parent agent's sub-agent delegation gates before spawning
/// `child_agent_id`: the Tier 3 capability toggle (`subagent_capability_enabled`)
/// and the allowed/denied delegation list (`subagents.is_agent_allowed`). Shared
/// by `do_spawn` AND `action_batch_spawn` so the model can't bypass the gate via
/// `batch_spawn` (which historically skipped it entirely).
///
/// **Fail-closed**: if the parent agent definition can't be loaded we DENY rather
/// than silently allow — the gate is a security boundary (AGENTS.md「执行层兜底」),
/// and a model-writable delegation allowlist that fails open is a privilege
/// escalation. The parent agent is the one currently running, so a load failure
/// here is an anomaly (corrupt/half-written `agent.json`, racing delete), not a
/// normal path.
fn check_subagent_delegation_allowed(parent_agent_id: &str, child_agent_id: &str) -> Result<()> {
    let def = crate::agent_loader::load_agent(parent_agent_id).map_err(|e| {
        anyhow::anyhow!(
            "Cannot verify sub-agent delegation permission (failed to load agent '{}': {}); \
             delegation denied",
            parent_agent_id,
            e
        )
    })?;
    if !subagent_capability_enabled(parent_agent_id, &def.config) {
        return Err(anyhow::anyhow!(
            "Sub-agent delegation is disabled for this agent"
        ));
    }
    if !def.config.subagents.is_agent_allowed(child_agent_id) {
        return Err(anyhow::anyhow!(
            "Agent '{}' is not in the allowed delegation list",
            child_agent_id
        ));
    }
    Ok(())
}

/// Tool handler for the `subagent` tool.
/// Actions: spawn, check, list, result, kill, kill_all, steer
pub(crate) async fn tool_subagent(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "spawn" => action_spawn(args, ctx).await,
        "check" => action_check(args).await,
        "list" => action_list(ctx).await,
        "result" => action_result(args).await,
        "kill" => action_kill(args).await,
        "kill_all" => action_kill_all(ctx).await,
        "steer" => action_steer(args).await,
        "batch_spawn" => action_batch_spawn(args, ctx).await,
        "wait_all" => action_wait_all(args).await,
        "spawn_and_wait" => action_spawn_and_wait(args, ctx).await,
        _ => Err(anyhow::anyhow!(
            "Unknown subagent action '{}'. Valid actions: spawn, check, list, result, kill, kill_all, steer, batch_spawn, wait_all, spawn_and_wait",
            action
        )),
    }
}

async fn resolve_subagent_timeout_secs(
    requested: Option<u64>,
    ctx: &ToolExecContext,
    parent_agent_id: &str,
    parameter: &str,
) -> Option<u64> {
    let requested_secs = requested?;
    let effective_secs = requested_secs.min(1800);
    let user_limit_secs = subagent::default_timeout_for_agent(parent_agent_id);

    if user_limit_secs > 0 && (requested_secs == 0 || effective_secs > user_limit_secs) {
        super::audit_model_runtime_timeout_override(
            Some(ctx),
            super::TOOL_SUBAGENT,
            parameter,
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "model supplied sub-agent timeout would relax parent agent timeout",
        );
        super::emit_model_runtime_timeout_metadata(
            ctx,
            super::TOOL_SUBAGENT,
            parameter,
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "model supplied sub-agent timeout would relax parent agent timeout",
        )
        .await;
        return None;
    }

    if requested_secs > 0
        && super::should_ignore_model_runtime_timeout_when_user_unlimited(user_limit_secs)
    {
        super::audit_model_runtime_timeout_override(
            Some(ctx),
            super::TOOL_SUBAGENT,
            parameter,
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "parent agent sub-agent timeout is unlimited",
        );
        super::emit_model_runtime_timeout_metadata(
            ctx,
            super::TOOL_SUBAGENT,
            parameter,
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "parent agent sub-agent timeout is unlimited",
        )
        .await;
        return None;
    }

    super::audit_model_runtime_timeout_override(
        Some(ctx),
        super::TOOL_SUBAGENT,
        parameter,
        requested_secs,
        effective_secs,
        Some(user_limit_secs),
        false,
        "model supplied sub-agent timeout",
    );
    super::emit_model_runtime_timeout_metadata(
        ctx,
        super::TOOL_SUBAGENT,
        parameter,
        requested_secs,
        effective_secs,
        Some(user_limit_secs),
        false,
        "model supplied sub-agent timeout",
    )
    .await;
    Some(effective_secs)
}

fn parse_subagent_files(args: &Value) -> Result<Vec<crate::agent::Attachment>> {
    let Some(files) = args.get("files") else {
        return Ok(Vec::new());
    };
    let files = files
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'files' must be an array"))?;
    let mut attachments = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let name = file
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("files[{index}].name is required"))?;
        let content = file
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("files[{index}].content is required"))?;
        let mime_type = file
            .get("mime_type")
            .or_else(|| file.get("mimeType"))
            .and_then(Value::as_str)
            .unwrap_or("text/plain");
        let encoding = file
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("utf8");

        let attachment = match encoding {
            "base64" => crate::agent::Attachment {
                name: name.to_string(),
                mime_type: mime_type.to_string(),
                source: None,
                data: Some(content.to_string()),
                file_path: None,
                quote_lines: None,
            },
            "utf8" => {
                let tmp_dir = std::env::temp_dir().join("hope-agent_subagent_files");
                std::fs::create_dir_all(&tmp_dir).with_context(|| {
                    format!(
                        "create sub-agent attachment directory {}",
                        tmp_dir.display()
                    )
                })?;
                let safe_name = name.replace(['/', '\\', ':'], "_");
                let tmp_path = tmp_dir.join(format!("{}_{}", uuid::Uuid::new_v4(), safe_name));
                std::fs::write(&tmp_path, content).with_context(|| {
                    format!("write sub-agent attachment {}", tmp_path.display())
                })?;
                crate::agent::Attachment {
                    name: name.to_string(),
                    mime_type: mime_type.to_string(),
                    source: None,
                    data: None,
                    file_path: Some(tmp_path.to_string_lossy().to_string()),
                    quote_lines: None,
                }
            }
            other => {
                return Err(anyhow::anyhow!(
                    "files[{index}].encoding must be 'utf8' or 'base64', got '{other}'"
                ));
            }
        };
        attachments.push(attachment);
    }
    Ok(attachments)
}

/// Core spawn logic shared by action_spawn and action_spawn_and_wait.
/// Returns the run_id on success.
async fn do_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'task' is required for spawn action"))?;

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_AGENT_ID)
        .to_string();

    let model_override = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("No session context — cannot spawn sub-agent outside a chat session")
    })?;

    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or(DEFAULT_AGENT_ID);
    let timeout_secs = resolve_subagent_timeout_secs(
        args.get("timeout_secs").and_then(|v| v.as_u64()),
        ctx,
        parent_agent_id,
        "timeout_secs",
    )
    .await;

    // Enforce the parent's delegation gates (Tier 3 capability toggle + allowed
    // delegation list). Fail-closed — see `check_subagent_delegation_allowed`.
    check_subagent_delegation_allowed(parent_agent_id, &agent_id)?;

    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let workflow_preallocated_run_id = args
        .get(WORKFLOW_PREALLOCATED_RUN_ID_ARG)
        .and_then(|v| v.as_str())
        .map(|raw| {
            uuid::Uuid::parse_str(raw)
                .map(|id| id.to_string())
                .map_err(|_| anyhow::anyhow!("workflow preallocated run id must be a UUID"))
        })
        .transpose()?;
    let skip_parent_injection = args
        .get(WORKFLOW_SKIP_PARENT_INJECTION_ARG)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let workflow_isolation = workflow_preallocated_run_id
        .as_ref()
        .and_then(|_| args.get(WORKFLOW_ISOLATION_ARG).and_then(Value::as_str));
    let shared_read_only = workflow_isolation == Some("shared_read_only");
    let (plan_agent_mode, plan_mode_allow_paths, lock_plan_agent_mode) = if shared_read_only {
        (Some(workflow_shared_read_only_mode()), Vec::new(), true)
    } else {
        (None, Vec::new(), false)
    };

    let attachments = {
        let args = args.clone();
        crate::blocking::run_blocking(move || parse_subagent_files(&args)).await?
    };

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
        label,
        isolate_worktree: !shared_read_only,
        attachments,
        plan_agent_mode,
        plan_mode_allow_paths,
        lock_plan_agent_mode,
        skip_parent_injection,
        extra_system_context: shared_read_only.then(|| {
            "## Workflow Read-only Shared Workspace\nThis child shares the parent workspace for inspection only. Do not write, edit, patch, create, delete, rename, or run commands that mutate workspace or external state. Return findings to the owning Workflow; request a worktree-isolated child when mutation is required.".to_string()
        }),
        skill_allowed_tools: Vec::new(),
        reasoning_effort: None,
        skill_name: None,
        origin_source: ctx.origin_chat_source.or(ctx.chat_source),
        // WS8: carry the parent turn's IM origin identity so an IM-origin
        // subagent's KB opt-in is judged against the origin account/chat.
        origin_channel_kb_context: ctx.channel_kb_context.clone(),
        // A standalone spawn is not part of a Group (R5) — it injects its own
        // result individually. Only `batch_spawn` sets a group id.
        group_id: None,
    };

    let run_id = if let Some(run_id) = workflow_preallocated_run_id {
        subagent::spawn_subagent_with_run_id(params, session_db, cancel_registry, run_id).await?
    } else {
        subagent::spawn_subagent(params, session_db, cancel_registry).await?
    };
    Ok(run_id)
}

async fn action_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let run_id = do_spawn(args, ctx).await?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "spawned",
        "run_id": run_id,
        "message": "Sub-agent spawned. Use subagent(action='check', run_id='...') to poll for completion."
    }))?)
}

async fn action_check(args: &Value) -> Result<String> {
    if args.get("run_id").and_then(Value::as_str).is_none() && args.get("run_ids").is_some() {
        return Err(anyhow::anyhow!(
            "check accepts one 'run_id'; use action='wait_all' for 'run_ids'"
        ));
    }
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for check action"))?;

    // wait=true: poll until completion (default timeout 60s, max 300s)
    let wait = args.get("wait").and_then(|v| v.as_bool()).unwrap_or(false);
    let wait_timeout = args
        .get("wait_timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(300);

    let session_db = get_session_db()?;

    let run = if wait {
        // Poll DB every 2s until terminal or timeout
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_timeout);
        loop {
            let r = session_db
                .get_subagent_run(run_id)?
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
        session_db
            .get_subagent_run(run_id)?
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
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let session_db = get_session_db()?;
    let runs = session_db.list_subagent_runs(parent_session_id)?;

    let items: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            let mut item = serde_json::json!({
                "run_id": r.run_id,
                "child_agent_id": r.child_agent_id,
                "task": truncate(&r.task, 80),
                "status": r.status.as_str(),
                "depth": r.depth,
                "started_at": r.started_at,
                "duration_ms": r.duration_ms,
            });
            if let Some(ref label) = r.label {
                item["label"] = serde_json::Value::String(label.clone());
            }
            item
        })
        .collect();

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "total": items.len(),
        "runs": items,
    }))?)
}

async fn action_result(args: &Value) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for result action"))?;

    let session_db = get_session_db()?;
    let run = session_db
        .get_subagent_run(run_id)?
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
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for kill action"))?;

    let cancel_registry = get_cancel_registry()?;
    let session_db = get_session_db()?;

    // Verify the run exists and is active
    let run = session_db
        .get_subagent_run(run_id)?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

    if run.status.is_terminal() {
        return Ok(format!(
            "Sub-agent run '{}' already in terminal state: {}",
            run_id,
            run.status.as_str()
        ));
    }

    let cancelled = cancel_registry.cancel(run_id);
    if cancelled {
        Ok(format!("Kill signal sent to sub-agent run '{}'", run_id))
    } else {
        // Update DB directly if no cancel flag found (already cleaned up)
        let _ = session_db.update_subagent_status(
            run_id,
            SubagentStatus::Killed,
            None,
            Some("Killed by parent agent"),
            None,
            None,
        );
        Ok(format!("Sub-agent run '{}' marked as killed", run_id))
    }
}

async fn action_kill_all(ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let cancel_registry = get_cancel_registry()?;
    let session_db = get_session_db()?;
    let count = cancel_registry.cancel_all_for_session(parent_session_id, &session_db);

    // R7.2: `cancel_all_for_session` only signals ACTIVE (spawning/running)
    // runs — it reads `list_active_subagent_runs`, which excludes `Queued`. A
    // parked spawn holds no slot, so without this it would survive kill_all and
    // then be PROMOTED by the scheduler (killing the active runs just freed a
    // slot) — running AFTER the user asked to kill everything. Purge the parked
    // entries and stamp each terminal (mirrors the session-delete cascade).
    let parked = subagent::queue::purge_for_session(parent_session_id);
    let parked_count = parked.len();
    for run_id in parked {
        subagent::request_cancel_run(&run_id);
    }

    let queued_note = if parked_count > 0 {
        format!(" and cancelled {} queued sub-agent(s)", parked_count)
    } else {
        String::new()
    };
    Ok(format!(
        "Kill signal sent to {} active sub-agent(s){}",
        count, queued_note
    ))
}

async fn action_batch_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let tasks = args
        .get("tasks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("'tasks' array is required for batch_spawn action"))?;

    if tasks.is_empty() {
        return Err(anyhow::anyhow!("'tasks' array cannot be empty"));
    }

    let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("No session context — cannot spawn sub-agents outside a chat session")
    })?;
    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or(DEFAULT_AGENT_ID);

    let max_batch = subagent::max_batch_size_for_agent(parent_agent_id);
    if tasks.len() > max_batch {
        return Err(anyhow::anyhow!(
            "batch_spawn supports at most {} tasks at once (current agent config)",
            max_batch
        ));
    }

    let session_db = get_session_db()?;
    let cancel_registry = get_cancel_registry()?;

    // R5: validate EVERY task object up front, BEFORE creating the Group or
    // spawning anything. A malformed task (missing `task` field) must fail the
    // whole call cleanly. If we validated lazily inside the spawn loop instead,
    // an error on task k>0 would `?`-return AFTER the group + children `0..k`
    // were already created — and those grouped children would be stranded
    // forever (their individual injection is suppressed, but the group is never
    // sealed, so the merged injection never fires). No `?` may run between the
    // group's creation and `seal_group` below.
    struct BatchTask {
        task: String,
        agent_id: String,
        label: Option<String>,
        timeout_secs: Option<u64>,
        model_override: Option<String>,
        attachments: Vec<crate::agent::Attachment>,
    }
    let shared_attachments = {
        let args = args.clone();
        crate::blocking::run_blocking(move || parse_subagent_files(&args)).await?
    };
    let mut parsed: Vec<BatchTask> = Vec::with_capacity(tasks.len());
    for task_def in tasks {
        let task = task_def
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Each task in batch_spawn must have a 'task' field"))?;
        let child_agent_id = task_def
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_AGENT_ID)
            .to_string();
        // Enforce the delegation gates per child, up front (same as `do_spawn`)
        // — `batch_spawn` must NOT be a bypass of the Tier 3 capability toggle /
        // allowed-agent list. Validated here in the pre-flight loop (before the
        // Group is created) so a denied agent fails the whole call cleanly; no
        // `?` may run after the group's creation (see the comment above).
        check_subagent_delegation_allowed(parent_agent_id, &child_agent_id)?;
        let timeout_secs = resolve_subagent_timeout_secs(
            task_def.get("timeout_secs").and_then(|v| v.as_u64()),
            ctx,
            parent_agent_id,
            "tasks[].timeout_secs",
        )
        .await;
        let mut attachments = shared_attachments.clone();
        let task_files = {
            let task_def = task_def.clone();
            crate::blocking::run_blocking(move || parse_subagent_files(&task_def)).await?
        };
        attachments.extend(task_files);
        parsed.push(BatchTask {
            task: task.to_string(),
            agent_id: child_agent_id,
            label: task_def
                .get("label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            timeout_secs,
            model_override: task_def
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            attachments,
        });
    }

    // R5: fan these children out as a single Group so all results arrive as ONE
    // merged injection when the batch finishes, instead of N separate billed
    // turns. Skipped for incognito (no projection survives close-and-burn) and
    // when the jobs DB is uninitialized — those children fall back to per-child
    // injection (the pre-R5 behavior). The group is created BEFORE spawning so
    // each child can carry its `group_id`, then SEALED after the loop so the
    // join coordinator may complete it once every child settles.
    let group_id = if crate::session::is_session_incognito(Some(parent_session_id)) {
        None
    } else {
        crate::async_jobs::JobManager::spawn_group(parent_session_id, parent_agent_id)
    };

    let mut results = Vec::new();
    for bt in parsed {
        let params = SpawnParams {
            task: bt.task,
            agent_id: bt.agent_id,
            parent_session_id: parent_session_id.to_string(),
            parent_agent_id: parent_agent_id.to_string(),
            depth: ctx.subagent_depth + 1,
            timeout_secs: bt.timeout_secs,
            model_override: bt.model_override,
            label: bt.label,
            isolate_worktree: true,
            attachments: bt.attachments,
            plan_agent_mode: None,
            plan_mode_allow_paths: Vec::new(),
            lock_plan_agent_mode: false,
            skip_parent_injection: false,
            extra_system_context: None,
            skill_allowed_tools: Vec::new(),
            reasoning_effort: None,
            skill_name: None,
            origin_source: ctx.origin_chat_source.or(ctx.chat_source),
            // WS8: forward the parent turn's IM origin identity (see above).
            origin_channel_kb_context: ctx.channel_kb_context.clone(),
            // R5: tag each child with the Group so its result joins the merged
            // injection instead of injecting on its own.
            group_id: group_id.clone(),
        };

        match subagent::spawn_subagent(params, session_db.clone(), cancel_registry.clone()).await {
            Ok(run_id) => results.push(serde_json::json!({"status": "spawned", "run_id": run_id})),
            Err(e) => results.push(serde_json::json!({"status": "error", "error": e.to_string()})),
        }
    }

    // R5: seal the group now that every child has been spawned — the join
    // coordinator may complete it (and fire the one merged injection) once all
    // children settle. The seal also runs an immediate completion check for the
    // case where fast children already finished during the spawn loop.
    if let Some(ref gid) = group_id {
        crate::async_jobs::JobManager::seal_group(gid);
    }

    let mut response = serde_json::json!({
        "status": "batch_spawned",
        "total": results.len(),
        "runs": results,
    });
    // Surface the group id so the model can `job_status(action='status',
    // job_id=...)` the batch as a whole (N-of-M) and knows results will arrive
    // as one merged notification when the batch finishes.
    if let Some(gid) = group_id {
        response["group_id"] = serde_json::Value::String(gid);
        response["delivery"] = serde_json::Value::String(
            "All results will be injected together as one notification when the batch finishes. \
             You can end your turn; no need to poll."
                .to_string(),
        );
    }

    Ok(serde_json::to_string_pretty(&response)?)
}

async fn action_wait_all(args: &Value) -> Result<String> {
    let run_ids = args
        .get("run_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("'run_ids' array is required for wait_all action"))?;

    let wait_timeout = args
        .get("wait_timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(120)
        .min(600);
    let partial = args
        .get("partial")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result_mode = args
        .get("result_mode")
        .or_else(|| args.get("resultMode"))
        .and_then(Value::as_str)
        .unwrap_or("preview");
    if !matches!(result_mode, "status" | "preview" | "summary" | "full") {
        return Err(anyhow::anyhow!(
            "'result_mode' must be status, preview, summary, or full"
        ));
    }

    let session_db = get_session_db()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_timeout);

    let ids: Vec<String> = run_ids
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    if ids.len() != run_ids.len() || ids.is_empty() {
        return Err(anyhow::anyhow!(
            "'run_ids' must contain at least one non-empty string run id"
        ));
    }

    // Poll until all terminal or timeout
    loop {
        let mut all_terminal = true;
        let mut results = Vec::new();
        for id in &ids {
            if let Ok(Some(run)) = session_db.get_subagent_run(id) {
                if !run.status.is_terminal() {
                    all_terminal = false;
                }
                let mut item = serde_json::json!({
                    "run_id": run.run_id,
                    "status": run.status.as_str(),
                });
                if run.status.is_terminal() {
                    if let Some(ref result) = run.result {
                        match result_mode {
                            "full" => {
                                item["result"] = serde_json::Value::String(result.clone());
                                item["result_preview"] =
                                    serde_json::Value::String(truncate(result, 200));
                            }
                            "summary" => {
                                item["result_summary"] =
                                    serde_json::Value::String(truncate(result, 2000));
                                item["result_preview"] =
                                    serde_json::Value::String(truncate(result, 200));
                            }
                            "preview" => {
                                item["result_preview"] =
                                    serde_json::Value::String(truncate(result, 200));
                            }
                            _ => {}
                        }
                    }
                    if let Some(ref error) = run.error {
                        item["error"] = serde_json::Value::String(error.clone());
                    }
                    if let Some(ms) = run.duration_ms {
                        item["duration_ms"] = serde_json::Value::Number(ms.into());
                    }
                    if result_mode != "status" {
                        crate::subagent::mark_run_fetched(id);
                    }
                }
                results.push(item);
            } else {
                results.push(serde_json::json!({"run_id": id, "status": "not_found"}));
            }
        }

        let timed_out = !all_terminal && std::time::Instant::now() >= deadline;
        if all_terminal || timed_out {
            let completed = results
                .iter()
                .filter(|run| run.get("status").and_then(Value::as_str) == Some("completed"))
                .count();
            let failed = results
                .iter()
                .filter(|run| {
                    matches!(
                        run.get("status").and_then(Value::as_str),
                        Some("error" | "timeout" | "killed" | "not_found")
                    )
                })
                .count();
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "all_completed": all_terminal,
                "timed_out": timed_out,
                "partial": partial,
                "accepted_partial": partial && timed_out,
                "completed": completed,
                "failed": failed,
                "total": results.len(),
                "result_mode": result_mode,
                "runs": results,
            }))?);
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
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

    // Verify the run exists and is still active
    let session_db = get_session_db()?;
    let run = session_db
        .get_subagent_run(run_id)?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

    if run.status.is_terminal() {
        return Err(anyhow::anyhow!(
            "Cannot steer sub-agent run '{}': already in terminal state '{}'",
            run_id,
            run.status.as_str()
        ));
    }

    // Push the steer message to the mailbox
    let delivered = crate::subagent::SUBAGENT_MAILBOX.push(run_id, message.to_string());
    if !delivered {
        return Err(anyhow::anyhow!(
            "Sub-agent run '{}' mailbox not found (may have just completed)",
            run_id
        ));
    }

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "steered",
        "run_id": run_id,
        "message": "Steer message delivered. The sub-agent will process it in the next tool loop round."
    }))?)
}

/// Spawn a sub-agent and wait for completion with auto-backgrounding.
///
/// If the sub-agent completes within `foreground_timeout` seconds, its result
/// is returned inline (like a synchronous call). If it exceeds the timeout,
/// it's automatically converted to a background task — the spawn continues
/// running and the result will be injected via the existing injection system.
async fn action_spawn_and_wait(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let fg_timeout = args
        .get("foreground_timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .min(120);

    let run_id = do_spawn(args, ctx).await?;

    // Poll for completion within foreground timeout
    let session_db = get_session_db()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(fg_timeout);

    loop {
        let run = session_db
            .get_subagent_run(&run_id)?
            .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

        if run.status.is_terminal() {
            // Completed within foreground timeout — return inline
            crate::subagent::mark_run_fetched(&run_id);
            let mut response = serde_json::json!({
                "status": run.status.as_str(),
                "mode": "foreground",
                "run_id": run_id,
            });
            if let Some(ref result) = run.result {
                response["result"] = serde_json::Value::String(result.clone());
            }
            if let Some(ref error) = run.error {
                response["error"] = serde_json::Value::String(error.clone());
            }
            if let Some(ms) = run.duration_ms {
                response["duration_ms"] = serde_json::Value::Number(ms.into());
            }
            return Ok(serde_json::to_string_pretty(&response)?);
        }

        if std::time::Instant::now() >= deadline {
            // D6 (DEADLOCK-5): distinguish "still working" from "paused waiting on
            // a tool approval". A pending child approval only persists where it can
            // actually be answered (unattended surfaces fail-close instead) — so if
            // one exists, tell the parent the child is blocked on the user, instead
            // of implying it's making background progress. (Checks the direct child
            // session; a deeper nested descendant's approval isn't probed here.)
            let awaiting_approval =
                crate::tools::approval::session_has_pending_approval(&run.child_session_id).await;
            let (status, message) = if awaiting_approval {
                (
                    "awaiting_approval",
                    format!(
                        "Sub-agent is paused waiting for a tool approval and did not finish within \
                         {}s. It will stay blocked until the approval is answered (or it times out / \
                         is denied). Approve it to let it continue; its result is injected when it \
                         completes.",
                        fg_timeout
                    ),
                )
            } else {
                (
                    "backgrounded",
                    format!(
                        "Sub-agent did not complete within {}s. Automatically backgrounded. \
                         Result will be injected into the conversation when complete.",
                        fg_timeout
                    ),
                )
            };
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": status,
                "mode": "background",
                "run_id": run_id,
                "message": message,
            }))?);
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
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
    crate::require_session_db().map(Arc::clone)
}

fn get_cancel_registry() -> Result<Arc<subagent::SubagentCancelRegistry>> {
    crate::require_subagent_cancels().map(Arc::clone)
}

#[cfg(test)]
mod delegation_gate_tests {
    use super::*;

    #[test]
    fn workflow_shared_read_only_mode_has_no_mutation_or_delegation_tools() {
        let crate::agent::PlanAgentMode::PlanAgent {
            allowed_tools,
            ask_tools,
        } = workflow_shared_read_only_mode()
        else {
            panic!("workflow shared read-only isolation must use the hard plan gate");
        };

        for denied in [
            "write",
            "edit",
            "apply_patch",
            "canvas",
            "exec",
            "process",
            "browser",
            "subagent",
            "team",
        ] {
            assert!(
                !allowed_tools.iter().any(|tool| tool == denied),
                "shared read-only isolation unexpectedly allows {denied}"
            );
        }
        assert!(allowed_tools.iter().any(|tool| tool == "read"));
        assert!(allowed_tools.iter().any(|tool| tool == "grep"));
        assert!(ask_tools.is_empty());
    }

    #[test]
    fn subagent_file_parser_supports_shared_and_task_specific_payloads() {
        let shared = parse_subagent_files(&serde_json::json!({
            "files": [{
                "name": "shared.txt",
                "content": "c2hhcmVk",
                "encoding": "base64"
            }]
        }))
        .expect("parse shared files");
        let private = parse_subagent_files(&serde_json::json!({
            "files": [{
                "name": "private.txt",
                "content": "cHJpdmF0ZQ==",
                "encoding": "base64"
            }]
        }))
        .expect("parse task files");
        let mut combined = shared;
        combined.extend(private);

        assert_eq!(combined.len(), 2);
        assert_eq!(combined[0].name, "shared.txt");
        assert_eq!(combined[1].name, "private.txt");
    }

    #[test]
    fn subagent_file_parser_rejects_malformed_entries() {
        let err = parse_subagent_files(&serde_json::json!({
            "files": [{"name": "missing-content.txt"}]
        }))
        .expect_err("missing content must fail");
        assert!(err.to_string().contains("files[0].content"));
    }

    #[test]
    fn delegation_fails_closed_when_parent_agent_cant_load() {
        // B1: if the parent agent definition can't be loaded, delegation must be
        // DENIED, not silently allowed — a model-writable allowlist that fails
        // open is a privilege escalation. (`do_spawn` and `action_batch_spawn`
        // both route through this gate so `batch_spawn` can't bypass it.)
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let err = check_subagent_delegation_allowed("__nonexistent_parent__", "helper")
                .expect_err("a missing parent agent definition must deny delegation");
            assert!(
                err.to_string().contains("delegation denied"),
                "expected fail-closed denial, got: {err}"
            );
        });
    }
}

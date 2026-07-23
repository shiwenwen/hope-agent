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
pub(crate) const WORKFLOW_RUN_ID_ARG: &str = "__hope_workflow_run_id";
pub(crate) const WORKFLOW_DISPATCH_ID_ARG: &str = "__hope_workflow_dispatch_id";

/// Authenticate internal Workflow-only arguments against execution context.
/// JSON schema omission is not a security boundary: a model can still emit
/// unknown fields, so ownership must never be inferred from args alone.
fn authenticated_workflow_owner<'a>(
    args: &Value,
    ctx: &'a ToolExecContext,
) -> Result<Option<&'a str>> {
    let declared = args
        .get(WORKFLOW_RUN_ID_ARG)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let has_internal_fields = [
        WORKFLOW_PREALLOCATED_RUN_ID_ARG,
        WORKFLOW_SKIP_PARENT_INJECTION_ARG,
        WORKFLOW_ISOLATION_ARG,
        WORKFLOW_RUN_ID_ARG,
        WORKFLOW_DISPATCH_ID_ARG,
    ]
    .iter()
    .any(|key| args.get(*key).is_some());

    match (declared, ctx.workflow_run_id.as_deref()) {
        (None, None) if !has_internal_fields => Ok(None),
        (Some(declared), Some(context)) if declared == context => Ok(Some(context)),
        _ => Err(anyhow::anyhow!(
            "Workflow-internal sub-agent arguments are not authorized in this execution context"
        )),
    }
}

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
        use_memories: true,
        contribute_to_memories: true,
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
/// Actions: spawn, send, check, list, result, kill, kill_all plus compatibility
/// aliases `resume` and `steer`.
pub(crate) async fn tool_subagent(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if ctx.workflow_run_id.is_some()
        && !matches!(action, "spawn" | "send" | "resume" | "steer" | "kill")
    {
        return Err(anyhow::anyhow!(
            "Workflow scripts must use spawnAgent/resumeAgent/agentSteer/cancelAgent instead of subagent action '{}'",
            action
        ));
    }

    match action {
        "spawn" => action_spawn(args, ctx).await,
        "send" => action_send(args, ctx).await,
        "resume" => action_resume(args, ctx).await,
        "check" => action_check(args, ctx).await,
        "list" => action_list(ctx).await,
        "result" => action_result(args, ctx).await,
        "kill" => action_kill(args, ctx).await,
        "kill_all" => action_kill_all(ctx).await,
        "steer" => action_steer(args, ctx).await,
        "batch_spawn" => action_batch_spawn(args, ctx).await,
        "wait_all" => action_wait_all(args, ctx).await,
        "spawn_and_wait" => action_spawn_and_wait(args, ctx).await,
        _ => Err(anyhow::anyhow!(
            "Unknown subagent action '{}'. Valid actions: spawn, send, resume, check, list, result, kill, kill_all, steer, batch_spawn, wait_all, spawn_and_wait",
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
                upload_id: None,
                quote_lines: None,
                quote_role: None,
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
                    upload_id: None,
                    quote_lines: None,
                    quote_role: None,
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

async fn load_subagent_run(
    session_db: &Arc<crate::session::SessionDB>,
    run_id: &str,
) -> Result<Option<crate::subagent::SubagentRun>> {
    let db = session_db.clone();
    let run_id = run_id.to_string();
    db.run(move |db| db.get_subagent_run(&run_id)).await
}

/// Suppress parent auto-delivery durably, then trip the process-local fast
/// cancellation signal. Keeping this async prevents a slow SQLite lock or
/// filesystem stall from consuming a Tokio runtime worker.
async fn consume_subagent_result(
    session_db: &Arc<crate::session::SessionDB>,
    run_id: &str,
) -> Result<()> {
    let db = session_db.clone();
    let run_id_owned = run_id.to_string();
    db.run(move |db| db.suppress_subagent_result_delivery(&run_id_owned, "explicitly_consumed"))
        .await?;
    crate::subagent::mark_run_fetched_in_memory(run_id);
    Ok(())
}

fn ensure_ordinary_run_owner(
    run: &crate::subagent::SubagentRun,
    ctx: &ToolExecContext,
    action: &str,
) -> Result<()> {
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;
    if run.parent_session_id != parent_session_id
        || run.owner_kind != crate::subagent::SubagentOwnerKind::ParentSession
        || run.owner_id != parent_session_id
    {
        return Err(anyhow::anyhow!(
            "Cannot {} sub-agent run '{}': it is not owned by this parent session",
            action,
            run.run_id
        ));
    }
    Ok(())
}

/// Return the native durable-work handle for a dispatched sub-agent run.
///
/// Keep the legacy snake_case fields and caller-selected `status` for existing
/// clients, while exposing a uniform contract that distinguishes this handle
/// from a generic `async_jobs` job. The run store remains authoritative.
fn subagent_dispatch_handle(run: &crate::subagent::SubagentRun, dispatch_status: &str) -> Value {
    let result_delivery = match run.delivery_kind {
        crate::subagent::SubagentDeliveryKind::Parent => "durable_parent_push",
        crate::subagent::SubagentDeliveryKind::Group => "durable_group_push",
        crate::subagent::SubagentDeliveryKind::Workflow => "workflow_controlled",
        crate::subagent::SubagentDeliveryKind::None => "none",
    };
    serde_json::json!({
        "kind": "subagent",
        "workKind": "subagent_run",
        "backgroundPolicy": "self_managed",
        "waitRequired": false,
        "status": dispatch_status,
        "runStatus": run.status.as_str(),
        "threadId": run.thread_id,
        "thread_id": run.thread_id,
        "runId": run.run_id,
        "run_id": run.run_id,
        "childAgentId": run.child_agent_id,
        "child_agent_id": run.child_agent_id,
        "childSessionId": run.child_session_id,
        "child_session_id": run.child_session_id,
        "deliveryMode": run.delivery_kind.as_str(),
        "resultDelivery": result_delivery,
    })
}

/// Core spawn logic shared by action_spawn and action_spawn_and_wait.
/// Returns the run_id on success.
async fn do_spawn(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let workflow_owner = authenticated_workflow_owner(args, ctx)?;
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
    let workflow_run_id = workflow_preallocated_run_id
        .as_ref()
        .and_then(|_| workflow_owner);
    if workflow_preallocated_run_id.is_some() && workflow_run_id.is_none() {
        return Err(anyhow::anyhow!(
            "Workflow-owned sub-agent spawn is missing its durable owner id"
        ));
    }
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
        owner_kind: if workflow_run_id.is_some() {
            crate::subagent::SubagentOwnerKind::Workflow
        } else {
            crate::subagent::SubagentOwnerKind::ParentSession
        },
        owner_id: workflow_run_id
            .unwrap_or(parent_session_id)
            .to_string(),
        delivery_kind: if workflow_run_id.is_some() {
            crate::subagent::SubagentDeliveryKind::Workflow
        } else {
            crate::subagent::SubagentDeliveryKind::Parent
        },
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
    let session_db = get_session_db()?;
    let run = load_subagent_run(&session_db, &run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' was not persisted", run_id))?;
    let mut response = subagent_dispatch_handle(&run, "spawned");
    response["message"] = Value::String(
        "Sub-agent dispatched asynchronously. Its durable result will be delivered when complete; polling is not required."
            .to_string(),
    );
    Ok(serde_json::to_string_pretty(&response)?)
}

async fn action_resume(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let source_run_id = args
        .get("run_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for resume action"))?;
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .ok_or_else(|| anyhow::anyhow!("'task' is required for resume action"))?;
    let parent_session_id = ctx.session_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("No session context — cannot resume a sub-agent outside a chat session")
    })?;
    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or(DEFAULT_AGENT_ID);
    let workflow_owner = authenticated_workflow_owner(args, ctx)?;
    let expected_owner_kind = if workflow_owner.is_some() {
        crate::subagent::SubagentOwnerKind::Workflow
    } else {
        crate::subagent::SubagentOwnerKind::ParentSession
    };
    let expected_owner_id = workflow_owner.unwrap_or(parent_session_id);
    let session_db = get_session_db()?;
    let source = {
        let db = session_db.clone();
        let source_run_id = source_run_id.to_string();
        db.run(move |db| db.get_subagent_run(&source_run_id))
            .await?
    }
    .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", source_run_id))?;

    // A run id is a capability-like opaque handle, but it must never be usable
    // to cross session boundaries. Resume is more powerful than read/check
    // because it starts a new model/tool turn, so enforce ownership here and
    // again in the core resume path.
    if source.parent_session_id != parent_session_id {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': it belongs to a different parent session",
            source_run_id
        ));
    }
    if source.owner_kind != expected_owner_kind || source.owner_id != expected_owner_id {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': its thread is owned by '{}' rather than this parent session",
            source_run_id,
            source.owner_kind.as_str()
        ));
    }
    if !source.status.is_terminal() {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': it is still '{}' (use steer instead)",
            source_run_id,
            source.status.as_str()
        ));
    }
    if matches!(source.status, SubagentStatus::Killed)
        || matches!(
            source.terminal_reason,
            Some(
                crate::subagent::SubagentTerminalReason::UserKilled
                    | crate::subagent::SubagentTerminalReason::ApprovalDenied
                    | crate::subagent::SubagentTerminalReason::ParentCancelled
                    | crate::subagent::SubagentTerminalReason::WorkflowCancelled
            )
        )
    {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': terminal reason '{}' requires an explicit user restart",
            source_run_id,
            source
                .terminal_reason
                .map(|reason| reason.as_str())
                .unwrap_or("user_killed")
        ));
    }
    check_subagent_delegation_allowed(parent_agent_id, &source.child_agent_id)?;

    let timeout_secs = resolve_subagent_timeout_secs(
        args.get("timeout_secs").and_then(Value::as_u64),
        ctx,
        parent_agent_id,
        "timeout_secs",
    )
    .await;
    let attachments = {
        let args = args.clone();
        crate::blocking::run_blocking(move || parse_subagent_files(&args)).await?
    };
    let model_override = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| source.label.clone());
    let workflow_isolation =
        workflow_owner.and_then(|_| args.get(WORKFLOW_ISOLATION_ARG).and_then(Value::as_str));
    let shared_read_only = workflow_isolation == Some("shared_read_only");
    let params = SpawnParams {
        task: task.to_string(),
        agent_id: source.child_agent_id.clone(),
        parent_session_id: parent_session_id.to_string(),
        parent_agent_id: parent_agent_id.to_string(),
        // A continuation is another turn at the same nesting level, not a
        // newly nested child.
        depth: source.depth,
        timeout_secs,
        model_override,
        label,
        // Reuse the child session's existing managed worktree / cwd. Creating a
        // second worktree would break continuity with the work already done.
        isolate_worktree: false,
        attachments,
        plan_agent_mode: shared_read_only.then(workflow_shared_read_only_mode),
        plan_mode_allow_paths: Vec::new(),
        lock_plan_agent_mode: shared_read_only,
        skip_parent_injection: workflow_owner.is_some(),
        extra_system_context: Some(if shared_read_only {
            format!(
                "## Continuation\nThis turn continues terminal sub-agent run `{}` in the same read-only shared workspace. Reuse prior findings and conversation, remain strictly read-only, and do not repeat completed work unnecessarily.",
                source_run_id
            )
        } else {
            format!(
                "## Continuation\nThis turn continues terminal sub-agent run `{}` in the same isolated child session. Reuse the prior conversation, findings, and working directory; address the new task below without repeating completed work unnecessarily.",
                source_run_id
            )
        }),
        skill_allowed_tools: Vec::new(),
        reasoning_effort: None,
        skill_name: None,
        origin_source: ctx.origin_chat_source.or(ctx.chat_source),
        origin_channel_kb_context: ctx.channel_kb_context.clone(),
        group_id: None,
        owner_kind: expected_owner_kind,
        owner_id: expected_owner_id.to_string(),
        delivery_kind: if workflow_owner.is_some() {
            crate::subagent::SubagentDeliveryKind::Workflow
        } else {
            crate::subagent::SubagentDeliveryKind::Parent
        },
    };
    let cancel_registry = get_cancel_registry()?;
    let dispatch_id = args
        .get(WORKFLOW_DISPATCH_ID_ARG)
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let preallocated_run_id = args
        .get(WORKFLOW_PREALLOCATED_RUN_ID_ARG)
        .and_then(Value::as_str)
        .map(str::to_string);
    let run_id = subagent::resume_subagent(
        source_run_id,
        params,
        session_db.clone(),
        cancel_registry,
        Some(dispatch_id.clone()),
        preallocated_run_id,
    )
    .await?;
    let run = {
        let db = session_db.clone();
        let new_run_id = run_id.clone();
        db.run(move |db| db.get_subagent_run(&new_run_id)).await?
    }
    .ok_or_else(|| anyhow::anyhow!("Resumed sub-agent run '{}' was not persisted", run_id))?;

    let run_status = run.status.as_str().to_string();
    let mut response = subagent_dispatch_handle(&run, &run_status);
    response["previous_run_id"] = Value::String(source_run_id.to_string());
    response["resumed_from_run_id"] = Value::String(source_run_id.to_string());
    response["dispatch_id"] = Value::String(dispatch_id);
    response["disposition"] = Value::String("resumed".to_string());
    response["message"] = Value::String(
        "Sub-agent resumed asynchronously in the same child session. The new run keeps the prior conversation and working directory; its durable result will be delivered when complete."
            .to_string(),
    );
    Ok(serde_json::to_string_pretty(&response)?)
}

/// Canonical thread-aware follow-up. Active attempts are steered; terminal
/// attempts get a fresh immutable continuation. `mode` lets deterministic
/// callers refuse the other branch rather than making a state-dependent choice.
async fn action_send(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context — cannot send to a sub-agent thread"))?;
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("auto");
    if !matches!(mode, "auto" | "steer_only" | "resume_only") {
        return Err(anyhow::anyhow!(
            "'mode' must be one of auto, steer_only, resume_only"
        ));
    }
    let message = args
        .get("message")
        .or_else(|| args.get("task"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .ok_or_else(|| anyhow::anyhow!("'message' is required for send action"))?;
    let session_db = get_session_db()?;

    let requested_run = if let Some(run_id) = args.get("run_id").and_then(Value::as_str) {
        Some(
            load_subagent_run(&session_db, run_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?,
        )
    } else {
        None
    };
    let thread_id = args
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| requested_run.as_ref().map(|run| run.thread_id.clone()))
        .ok_or_else(|| anyhow::anyhow!("'thread_id' (or compatibility 'run_id') is required"))?;
    if requested_run
        .as_ref()
        .is_some_and(|run| run.thread_id != thread_id)
    {
        return Err(anyhow::anyhow!(
            "run_id and thread_id identify different sub-agent threads"
        ));
    }
    let (thread, current) = {
        let db = session_db.clone();
        let lookup_thread_id = thread_id.clone();
        db.run(move |db| {
            let thread = db.get_subagent_thread(&lookup_thread_id)?.ok_or_else(|| {
                anyhow::anyhow!("Sub-agent thread '{}' not found", lookup_thread_id)
            })?;
            let current = db
                .get_current_subagent_run(&lookup_thread_id)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Sub-agent thread '{}' has no current attempt",
                        lookup_thread_id
                    )
                })?;
            Result::<_, anyhow::Error>::Ok((thread, current))
        })
        .await?
    };
    let workflow_owner = authenticated_workflow_owner(args, ctx)?;
    let expected_owner_kind = if workflow_owner.is_some() {
        crate::subagent::SubagentOwnerKind::Workflow
    } else {
        crate::subagent::SubagentOwnerKind::ParentSession
    };
    let expected_owner_id = workflow_owner.unwrap_or(parent_session_id);
    if thread.owner_kind != expected_owner_kind
        || thread.owner_id != expected_owner_id
        || thread.parent_session_id != parent_session_id
    {
        return Err(anyhow::anyhow!(
            "Cannot send to sub-agent thread '{}': it is not owned by this parent session",
            thread_id
        ));
    }
    if thread.lifecycle_state != crate::subagent::SubagentThreadState::Open {
        return Err(anyhow::anyhow!(
            "Cannot send to sub-agent thread '{}': lifecycle state is '{}'",
            thread_id,
            thread.lifecycle_state.as_str()
        ));
    }
    if requested_run
        .as_ref()
        .is_some_and(|run| run.run_id != current.run_id)
    {
        return Err(anyhow::anyhow!(
            "Run '{}' is not the current attempt for thread '{}'",
            requested_run
                .as_ref()
                .map(|run| run.run_id.as_str())
                .unwrap_or(""),
            thread_id
        ));
    }
    let parent_agent_id = ctx.agent_id.as_deref().unwrap_or(DEFAULT_AGENT_ID);
    check_subagent_delegation_allowed(parent_agent_id, &current.child_agent_id)?;

    if !current.status.is_terminal() {
        if mode == "resume_only" {
            return Err(anyhow::anyhow!(
                "Sub-agent thread '{}' is active; use steer_only or auto",
                thread_id
            ));
        }
        let dispatch_id = uuid::Uuid::new_v4().to_string();
        {
            let db = session_db.clone();
            let dispatch_id = dispatch_id.clone();
            let thread_id = thread_id.clone();
            let current_run_id = current.run_id.clone();
            let expected_owner_id = expected_owner_id.to_string();
            let message = message.to_string();
            db.run(move |db| {
                db.insert_subagent_steer_dispatch(
                    &dispatch_id,
                    &thread_id,
                    &current_run_id,
                    expected_owner_kind,
                    &expected_owner_id,
                    &message,
                )
            })
            .await?;
        }
        let delivered =
            crate::subagent::SUBAGENT_MAILBOX.push(&current.run_id, message.to_string());
        if delivered {
            let db = session_db.clone();
            let delivered_dispatch_id = dispatch_id.clone();
            db.run(move |db| db.mark_subagent_dispatch_delivered(&delivered_dispatch_id))
                .await?;
        } else if matches!(current.status, SubagentStatus::Running) {
            // A Running row without a mailbox normally means it crossed the
            // terminal boundary between the transaction and the push. Refuse
            // this dispatch rather than claiming delivery.
            let db = session_db.clone();
            let refused_dispatch_id = dispatch_id.clone();
            db.run(move |db| db.mark_subagent_dispatch_refused(&refused_dispatch_id))
                .await?;
            return Err(anyhow::anyhow!(
                "Sub-agent run '{}' stopped before the steer message could be delivered; retry send",
                current.run_id
            ));
        }
        let current_status = current.status.as_str().to_string();
        let mut response = subagent_dispatch_handle(&current, &current_status);
        response["previous_run_id"] = Value::String(current.run_id.clone());
        response["dispatch_id"] = Value::String(dispatch_id);
        response["disposition"] = Value::String("steered".to_string());
        response["delivery"] =
            Value::String(if delivered { "delivered" } else { "accepted" }.to_string());
        return Ok(serde_json::to_string_pretty(&response)?);
    }

    if mode == "steer_only" {
        return Err(anyhow::anyhow!(
            "Sub-agent thread '{}' is terminal; use resume_only or auto",
            thread_id
        ));
    }
    if workflow_owner.is_some() {
        return Err(anyhow::anyhow!(
            "Workflow-owned thread '{}' is terminal; use workflow.resumeAgent in Workflow API V5",
            thread_id
        ));
    }
    let mut resume_args = args.clone();
    let map = resume_args
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("subagent.send args must be an object"))?;
    map.insert("run_id".to_string(), Value::String(current.run_id));
    map.insert("task".to_string(), Value::String(message.to_string()));
    action_resume(&resume_args, ctx).await
}

async fn action_check(args: &Value, ctx: &ToolExecContext) -> Result<String> {
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
            let r = load_subagent_run(&session_db, run_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;
            ensure_ordinary_run_owner(&r, ctx, "check")?;
            if r.status.is_terminal() {
                break r;
            }
            if std::time::Instant::now() >= deadline {
                break r; // Return current (non-terminal) status on timeout
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    } else {
        let run = load_subagent_run(&session_db, run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;
        ensure_ordinary_run_owner(&run, ctx, "check")?;
        run
    };

    let mut response = serde_json::json!({
        "thread_id": run.thread_id,
        "run_id": run.run_id,
        "attempt": run.lease_epoch,
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
        if let Some(reason) = run.terminal_reason {
            response["terminal_reason"] = serde_json::Value::String(reason.as_str().to_string());
        }
        // Mark as fetched so auto-injection is skipped
        consume_subagent_result(&session_db, run_id).await?;
    }

    Ok(serde_json::to_string_pretty(&response)?)
}

async fn action_list(ctx: &ToolExecContext) -> Result<String> {
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;

    let session_db = get_session_db()?;
    let runs = {
        let db = session_db.clone();
        let parent_session_id = parent_session_id.to_string();
        db.run(move |db| db.list_subagent_runs(&parent_session_id))
            .await?
    };
    let runs: Vec<_> = runs
        .into_iter()
        .filter(|run| {
            run.owner_kind == crate::subagent::SubagentOwnerKind::ParentSession
                && run.owner_id == parent_session_id
        })
        .collect();

    let items: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            let mut item = serde_json::json!({
                "thread_id": r.thread_id,
                "run_id": r.run_id,
                "attempt": r.lease_epoch,
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

async fn action_result(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for result action"))?;

    let session_db = get_session_db()?;
    let run = load_subagent_run(&session_db, run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;
    ensure_ordinary_run_owner(&run, ctx, "read result for")?;

    if !run.status.is_terminal() {
        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "run_id": run.run_id,
            "status": run.status.as_str(),
            "message": "Sub-agent is still running. Use check to poll status."
        }))?);
    }

    // Mark as fetched so auto-injection is skipped
    consume_subagent_result(&session_db, run_id).await?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "thread_id": run.thread_id,
        "run_id": run.run_id,
        "attempt": run.lease_epoch,
        "status": run.status.as_str(),
        "terminal_reason": run.terminal_reason.map(|reason| reason.as_str()),
        "result": run.result,
        "error": run.error,
        "model_used": run.model_used,
        "duration_ms": run.duration_ms,
    }))?)
}

async fn action_kill(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'run_id' is required for kill action"))?;

    let cancel_registry = get_cancel_registry()?;
    let session_db = get_session_db()?;

    // Verify the run exists and is active
    let run = load_subagent_run(&session_db, run_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;
    let parent_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No session context"))?;
    let workflow_owner = authenticated_workflow_owner(args, ctx)?;
    let expected_owner_kind = if workflow_owner.is_some() {
        crate::subagent::SubagentOwnerKind::Workflow
    } else {
        crate::subagent::SubagentOwnerKind::ParentSession
    };
    let expected_owner_id = workflow_owner.unwrap_or(parent_session_id);
    if run.parent_session_id != parent_session_id
        || run.owner_kind != expected_owner_kind
        || run.owner_id != expected_owner_id
    {
        return Err(anyhow::anyhow!(
            "Cannot kill sub-agent run '{}': it is not controlled by this caller",
            run_id
        ));
    }

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
        let terminal_reason = if workflow_owner.is_some() {
            crate::subagent::SubagentTerminalReason::WorkflowCancelled
        } else {
            crate::subagent::SubagentTerminalReason::UserKilled
        };
        let db = session_db.clone();
        let run_id_owned = run_id.to_string();
        db.run(move |db| {
            db.update_subagent_status_with_reason(
                &run_id_owned,
                SubagentStatus::Killed,
                Some(terminal_reason),
                None,
                Some("Killed by parent agent"),
                None,
                None,
            )
        })
        .await?;
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
    let ordinary_active = {
        let db = session_db.clone();
        let parent_session_id = parent_session_id.to_string();
        db.run(move |db| db.list_active_subagent_runs(&parent_session_id))
            .await?
    };
    let ordinary_active = ordinary_active
        .into_iter()
        .filter(|run| {
            run.owner_kind == crate::subagent::SubagentOwnerKind::ParentSession
                && run.owner_id == parent_session_id
        })
        .collect::<Vec<_>>();
    let mut count = 0usize;
    for run in ordinary_active {
        if cancel_registry.cancel(&run.run_id) {
            count += 1;
            continue;
        }
        let db = session_db.clone();
        let run_id = run.run_id;
        db.run(move |db| {
            db.update_subagent_status_with_reason(
                &run_id,
                SubagentStatus::Killed,
                Some(crate::subagent::SubagentTerminalReason::UserKilled),
                None,
                Some("Killed by parent agent"),
                None,
                None,
            )
        })
        .await?;
        count += 1;
    }

    // R7.2: active lookup excludes `Queued`. A
    // parked spawn holds no slot, so without this it would survive kill_all and
    // then be PROMOTED by the scheduler (killing the active runs just freed a
    // slot) — running AFTER the parent asked to kill everything. Purge only
    // this ordinary owner, then explicitly stamp each removed row terminal.
    let parked = subagent::queue::purge_for_owner(
        parent_session_id,
        crate::subagent::SubagentOwnerKind::ParentSession,
        parent_session_id,
    );
    let parked_count = parked.len();
    for run_id in parked {
        cancel_registry.cancel(&run_id);
        cancel_registry.remove(&run_id);
        let db = session_db.clone();
        db.run(move |db| {
            db.update_subagent_status_with_reason(
                &run_id,
                SubagentStatus::Killed,
                Some(crate::subagent::SubagentTerminalReason::UserKilled),
                None,
                Some("Killed while queued by parent agent"),
                None,
                None,
            )
        })
        .await?;
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
            owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
            owner_id: parent_session_id.to_string(),
            delivery_kind: if group_id.is_some() {
                crate::subagent::SubagentDeliveryKind::Group
            } else {
                crate::subagent::SubagentDeliveryKind::Parent
            },
        };

        match subagent::spawn_subagent(params, session_db.clone(), cancel_registry.clone()).await {
            Ok(run_id) => {
                let persisted = load_subagent_run(&session_db, &run_id).await.ok().flatten();
                if let Some(run) = persisted.as_ref() {
                    results.push(subagent_dispatch_handle(run, "spawned"));
                } else {
                    results.push(serde_json::json!({
                        "status": "error",
                        "run_id": run_id,
                        "error": "Sub-agent run was not persisted",
                    }));
                }
            }
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
        "kind": "subagent_batch",
        "workKind": "subagent_run",
        "backgroundPolicy": "self_managed",
        "waitRequired": false,
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

async fn action_wait_all(args: &Value, ctx: &ToolExecContext) -> Result<String> {
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
        let mut snapshots = {
            let db = session_db.clone();
            let lookup_ids = ids.clone();
            db.run(move |db| db.get_subagent_runs_batch(&lookup_ids))
                .await?
        };
        let mut consumed_run_ids = Vec::new();
        for id in &ids {
            if let Some(run) = snapshots.remove(id) {
                ensure_ordinary_run_owner(&run, ctx, "wait for")?;
                if !run.status.is_terminal() {
                    all_terminal = false;
                }
                let mut item = serde_json::json!({
                    "thread_id": run.thread_id,
                    "run_id": run.run_id,
                    "attempt": run.lease_epoch,
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
                    if let Some(reason) = run.terminal_reason {
                        item["terminal_reason"] =
                            serde_json::Value::String(reason.as_str().to_string());
                    }
                    if result_mode != "status" {
                        consumed_run_ids.push(id.clone());
                    }
                }
                results.push(item);
            } else {
                results.push(serde_json::json!({"run_id": id, "status": "not_found"}));
            }
        }
        if !consumed_run_ids.is_empty() {
            let db = session_db.clone();
            let durable_ids = consumed_run_ids.clone();
            db.run(move |db| {
                for run_id in &durable_ids {
                    db.suppress_subagent_result_delivery(run_id, "explicitly_consumed")?;
                }
                Result::<_, anyhow::Error>::Ok(())
            })
            .await?;
            for run_id in consumed_run_ids {
                crate::subagent::mark_run_fetched_in_memory(&run_id);
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
                        Some("error" | "timeout" | "killed" | "interrupted" | "not_found")
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

async fn action_steer(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let mut send_args = args.clone();
    let map = send_args
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("subagent.steer args must be an object"))?;
    map.insert("mode".to_string(), Value::String("steer_only".to_string()));
    action_send(&send_args, ctx).await
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
        let run = load_subagent_run(&session_db, &run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", run_id))?;

        if run.status.is_terminal() {
            // Completed within foreground timeout — return inline
            consume_subagent_result(&session_db, &run_id).await?;
            let mut response = subagent_dispatch_handle(&run, run.status.as_str());
            response["mode"] = Value::String("foreground".to_string());
            response["resultDelivery"] = Value::String("inline_consumed".to_string());
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
            let mut response = subagent_dispatch_handle(&run, status);
            response["mode"] = Value::String("background".to_string());
            response["message"] = Value::String(message);
            return Ok(serde_json::to_string_pretty(&response)?);
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
    fn dispatch_handle_identifies_native_async_lifecycle() {
        let run = crate::subagent::SubagentRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            child_agent_id: "researcher".to_string(),
            child_session_id: "thread-1".to_string(),
            status: SubagentStatus::Queued,
            delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
            ..Default::default()
        };

        let handle = subagent_dispatch_handle(&run, "spawned");
        assert_eq!(handle["kind"], "subagent");
        assert_eq!(handle["workKind"], "subagent_run");
        assert_eq!(handle["backgroundPolicy"], "self_managed");
        assert_eq!(handle["waitRequired"], false);
        assert_eq!(handle["status"], "spawned");
        assert_eq!(handle["runStatus"], "queued");
        assert_eq!(handle["runId"], "run-1");
        assert_eq!(handle["threadId"], "thread-1");
        assert_eq!(handle["resultDelivery"], "durable_parent_push");
    }

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
    fn workflow_internal_owner_cannot_be_forged_by_model_args() {
        let args = serde_json::json!({
            WORKFLOW_RUN_ID_ARG: "workflow-run",
            WORKFLOW_PREALLOCATED_RUN_ID_ARG: uuid::Uuid::new_v4().to_string(),
        });
        let err = authenticated_workflow_owner(&args, &ToolExecContext::default())
            .expect_err("model-only hidden args must not authenticate Workflow ownership");
        assert!(err.to_string().contains("not authorized"));
    }

    #[test]
    fn workflow_internal_owner_requires_matching_execution_context() {
        let args = serde_json::json!({ WORKFLOW_RUN_ID_ARG: "workflow-run" });
        let matching = ToolExecContext {
            workflow_run_id: Some("workflow-run".to_string()),
            ..Default::default()
        };
        assert_eq!(
            authenticated_workflow_owner(&args, &matching).expect("matching owner"),
            Some("workflow-run")
        );

        let mismatched = ToolExecContext {
            workflow_run_id: Some("other-workflow".to_string()),
            ..Default::default()
        };
        assert!(authenticated_workflow_owner(&args, &mismatched).is_err());
    }

    #[tokio::test]
    async fn workflow_context_rejects_generic_subagent_fanout_escape() {
        let ctx = ToolExecContext {
            workflow_run_id: Some("workflow-run".to_string()),
            ..Default::default()
        };
        let error = tool_subagent(
            &serde_json::json!({ "action": "batch_spawn", "tasks": [] }),
            &ctx,
        )
        .await
        .expect_err("Workflow must use its owner-aware Agent host APIs");
        assert!(error.to_string().contains("must use spawnAgent"));
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

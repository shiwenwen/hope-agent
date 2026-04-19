use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::session::SessionDB;

use super::cancel::SubagentCancelRegistry;
use super::helpers::{emit_subagent_event, truncate_str};
use super::injection::{build_subagent_push_message, inject_and_run_parent};
use super::mailbox::SUBAGENT_MAILBOX;
use super::types::{SpawnParams, SubagentEvent, SubagentRun, SubagentStatus};
use super::{
    max_depth_for_agent, DEFAULT_TIMEOUT_SECS, MAX_CONCURRENT_PER_SESSION, MAX_RESULT_CHARS,
};

// ── Spawn Logic ─────────────────────────────────────────────────

/// Spawn a sub-agent asynchronously. Returns the run_id immediately.
pub async fn spawn_subagent(
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) -> Result<String> {
    // 1. Validate depth (use parent agent's configured max)
    let effective_max_depth = max_depth_for_agent(&params.parent_agent_id);
    if params.depth > effective_max_depth {
        return Err(anyhow::anyhow!(
            "Sub-agent depth limit reached ({}/{}). Cannot spawn further sub-agents.",
            params.depth,
            effective_max_depth
        ));
    }

    // 2. Check concurrent limit
    let active_count = session_db.count_active_subagent_runs(&params.parent_session_id)?;
    if active_count >= MAX_CONCURRENT_PER_SESSION {
        return Err(anyhow::anyhow!(
            "Max concurrent sub-agents reached ({}/{}). Wait for some to complete or kill them.",
            active_count,
            MAX_CONCURRENT_PER_SESSION
        ));
    }

    // 3. Validate agent exists
    let _agent_def = crate::agent_loader::load_agent(&params.agent_id)
        .map_err(|e| anyhow::anyhow!("Agent '{}' not found: {}", params.agent_id, e))?;

    // 4. Generate run_id and create isolated session (linked to parent)
    let run_id = uuid::Uuid::new_v4().to_string();
    let child_session =
        session_db.create_session_with_parent(&params.agent_id, Some(&params.parent_session_id))?;
    let child_session_id = child_session.id.clone();

    // Set a descriptive title for the sub-agent session
    let task_preview = truncate_str(&params.task, 50);
    let _ = session_db.update_session_title(&child_session_id, &task_preview);

    // 5. Insert run record
    let now = chrono::Utc::now().to_rfc3339();
    let attachment_count = params.attachments.len() as u32;
    let run = SubagentRun {
        run_id: run_id.clone(),
        parent_session_id: params.parent_session_id.clone(),
        parent_agent_id: params.parent_agent_id.clone(),
        child_agent_id: params.agent_id.clone(),
        child_session_id: child_session_id.clone(),
        task: params.task.clone(),
        status: SubagentStatus::Spawning,
        result: None,
        error: None,
        depth: params.depth,
        model_used: None,
        started_at: now,
        finished_at: None,
        duration_ms: None,
        label: params.label.clone(),
        attachment_count,
        input_tokens: None,
        output_tokens: None,
    };
    session_db.insert_subagent_run(&run)?;

    // 6. Register cancel flag and steer mailbox slot
    let cancel_flag = cancel_registry.register(&run_id);
    SUBAGENT_MAILBOX.register(&run_id);

    // 7. Emit spawned event
    emit_subagent_event(&SubagentEvent {
        event_type: "spawned".into(),
        run_id: run_id.clone(),
        parent_session_id: params.parent_session_id.clone(),
        child_agent_id: params.agent_id.clone(),
        child_session_id: child_session_id.clone(),
        task_preview: task_preview.clone(),
        status: SubagentStatus::Spawning,
        result_preview: None,
        error: None,
        duration_ms: None,
        label: params.label.clone(),
        input_tokens: None,
        output_tokens: None,
        result_full: None,
        skill_name: params.skill_name.clone(),
    });

    // 8. Spawn async task
    let run_id_clone = run_id.clone();
    let db = session_db.clone();
    let registry = cancel_registry.clone();
    let agent_id = params.agent_id.clone();
    let task = params.task.clone();
    let depth = params.depth;
    let timeout_secs = params.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let model_override = params.model_override.clone();
    let parent_session_id = params.parent_session_id.clone();
    let parent_agent_id = params.parent_agent_id.clone();
    let child_session_id_clone = child_session_id.clone();
    let label = params.label.clone();
    let attachments = params.attachments.clone();
    let plan_agent_mode = params.plan_agent_mode.clone();
    let plan_mode_allow_paths = params.plan_mode_allow_paths.clone();
    let skip_parent_injection = params.skip_parent_injection;
    let extra_system_context = params.extra_system_context.clone();
    let skill_allowed_tools = params.skill_allowed_tools.clone();
    let reasoning_effort = params.reasoning_effort.clone();
    let skill_name_for_events = params.skill_name.clone();

    tokio::spawn(async move {
        let start = std::time::Instant::now();

        // Update status to Running
        let _ = db.update_subagent_status(
            &run_id_clone,
            SubagentStatus::Running,
            None,
            None,
            None,
            None,
        );

        // Execute sub-agent with timeout, catch_unwind to guarantee completion event
        let agent_id_exec = agent_id.clone();
        let task_exec = task.clone();
        let model_override_exec = model_override.clone();
        let cancel_exec = cancel_flag.clone();

        let run_id_exec = run_id_clone.clone();
        let attachments_exec = attachments.clone();
        let plan_agent_mode_exec = plan_agent_mode.clone();
        let plan_mode_allow_paths_exec = plan_mode_allow_paths.clone();
        let extra_system_context_exec = extra_system_context.clone();
        let skill_allowed_tools_exec = skill_allowed_tools.clone();
        let reasoning_effort_exec = reasoning_effort.clone();
        let exec_result = std::panic::AssertUnwindSafe(tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            execute_subagent(
                agent_id_exec,
                task_exec,
                depth,
                model_override_exec,
                cancel_exec,
                run_id_exec,
                attachments_exec,
                parent_session_id.clone(),
                plan_agent_mode_exec,
                plan_mode_allow_paths_exec,
                extra_system_context_exec,
                skill_allowed_tools_exec,
                reasoning_effort_exec,
            ),
        ));
        let result = futures_util::FutureExt::catch_unwind(exec_result).await;

        let duration_ms = start.elapsed().as_millis() as u64;
        let finished_at = chrono::Utc::now().to_rfc3339();

        // Determine outcome — handles Ok, Err, Timeout, Cancel, and Panic
        let (status, result_text, error_text, model_used) = match result {
            Ok(Ok(Ok((response, model)))) => {
                let truncated = truncate_str(&response, MAX_RESULT_CHARS);
                (SubagentStatus::Completed, Some(truncated), None, model)
            }
            Ok(Ok(Err(e))) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    (
                        SubagentStatus::Killed,
                        None,
                        Some("Killed by parent".into()),
                        None,
                    )
                } else {
                    (SubagentStatus::Error, None, Some(e.to_string()), None)
                }
            }
            Ok(Err(_)) => {
                // Timeout
                (
                    SubagentStatus::Timeout,
                    None,
                    Some(format!("Timed out after {}s", timeout_secs)),
                    None,
                )
            }
            Err(_panic) => {
                // Panic caught — still deliver the event
                (
                    SubagentStatus::Error,
                    None,
                    Some("Sub-agent panicked unexpectedly".into()),
                    None,
                )
            }
        };

        // Save messages to child session so they're visible when clicking into it
        let _ = db.append_message(&child_session_id, &crate::session::NewMessage::user(&task));
        let reply_text = result_text
            .as_deref()
            .or(error_text.as_deref())
            .unwrap_or("(no response)");
        let _ = db.append_message(
            &child_session_id,
            &crate::session::NewMessage::assistant(reply_text),
        );

        // Update DB — guaranteed to run even after panic
        let _ = db.update_subagent_status(
            &run_id_clone,
            status.clone(),
            result_text.as_deref(),
            error_text.as_deref(),
            model_used.as_deref(),
            Some(duration_ms),
        );
        let _ = db.set_subagent_finished_at(&run_id_clone, &finished_at);

        // Emit completion event — guaranteed to fire
        let result_preview = result_text.as_ref().map(|r| truncate_str(r, 200));
        // Clone values needed after the move into SubagentEvent
        let status_for_inject = status.clone();
        let agent_id_for_inject = agent_id.clone();
        let result_text_for_inject = result_text.clone();
        let error_text_for_inject = error_text.clone();
        let parent_session_id_for_inject = parent_session_id.clone();
        let child_session_id_for_cleanup = child_session_id_clone.clone();
        emit_subagent_event(&SubagentEvent {
            event_type: status.as_str().to_string(),
            run_id: run_id_clone.clone(),
            parent_session_id,
            child_agent_id: agent_id,
            child_session_id: child_session_id_clone,
            task_preview: truncate_str(&task, 50),
            status,
            result_preview,
            error: error_text.clone(),
            duration_ms: Some(duration_ms),
            label: label.clone(),
            input_tokens: None, // TODO: extract from agent usage when available
            output_tokens: None,
            result_full: result_text,
            skill_name: skill_name_for_events.clone(),
        });

        // Cleanup cancel flag and steer mailbox
        registry.remove(&run_id_clone);
        SUBAGENT_MAILBOX.remove(&run_id_clone);

        app_info!(
            "subagent",
            "spawn",
            "Sub-agent run {} finished in {}ms",
            run_id_clone,
            duration_ms
        );

        // Cleanup plan subagent registration if applicable
        crate::plan::try_unregister_plan_subagent_sync(&child_session_id_for_cleanup);

        // Backend-driven result injection: push result to parent agent without relying on frontend.
        // Uses a dedicated OS thread + runtime to avoid the Send cycle:
        // inject_and_run_parent → agent.chat() → action_spawn → spawn_subagent → tokio::spawn
        if !skip_parent_injection
            && matches!(
                status_for_inject,
                SubagentStatus::Completed | SubagentStatus::Error | SubagentStatus::Timeout
            )
        {
            let push_msg = build_subagent_push_message(
                &run_id_clone,
                &agent_id_for_inject,
                &task,
                &status_for_inject,
                duration_ms,
                result_text_for_inject.as_deref(),
                error_text_for_inject.as_deref(),
            );
            let db2 = db.clone();
            let parent_sid2 = parent_session_id_for_inject;
            let parent_agent_id2 = parent_agent_id.clone();
            let child_agent_id2 = agent_id_for_inject.clone();
            let run_id2 = run_id_clone.clone();
            // Spawn on a separate OS thread so the future doesn't need to be Send
            std::thread::spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(inject_and_run_parent(
                        parent_sid2,
                        parent_agent_id2,
                        child_agent_id2,
                        run_id2,
                        push_msg,
                        db2,
                    )),
                    Err(e) => app_error!(
                        "subagent",
                        "inject",
                        "Failed to build runtime for injection: {}",
                        e
                    ),
                }
            });
        }
    });

    Ok(run_id)
}

/// Execute the sub-agent (runs within the spawned tokio task).
/// Returns (response_text, model_used).
fn execute_subagent(
    agent_id: String,
    task: String,
    depth: u32,
    model_override: Option<String>,
    cancel: Arc<AtomicBool>,
    run_id: String,
    attachments: Vec<crate::agent::Attachment>,
    parent_session_id: String,
    plan_agent_mode: Option<crate::agent::PlanAgentMode>,
    plan_mode_allow_paths: Vec<String>,
    extra_system_context_override: Option<String>,
    skill_allowed_tools: Vec<String>,
    reasoning_effort: Option<String>,
) -> impl std::future::Future<Output = Result<(String, Option<String>)>> + Send {
    async move {
        use crate::agent::AssistantAgent;
        use crate::failover;
        use crate::provider;

        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10_000;

        let store = crate::config::cached_config();

        // Load agent config for model resolution
        let agent_def = crate::agent_loader::load_agent(&agent_id)?;
        let agent_model_config = if let Some(ref override_str) = model_override {
            let mut cfg = agent_def.config.model.clone();
            cfg.primary = Some(override_str.clone());
            cfg
        } else {
            // Check if the agent's subagent config specifies a model override
            let subagent_model = agent_def.config.subagents.model.clone();
            if let Some(ref m) = subagent_model {
                let mut cfg = agent_def.config.model.clone();
                cfg.primary = Some(m.clone());
                cfg
            } else {
                agent_def.config.model.clone()
            }
        };

        let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

        let mut model_chain = Vec::new();
        if let Some(p) = primary {
            model_chain.push(p);
        }
        for fb in fallbacks {
            if !model_chain
                .iter()
                .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
            {
                model_chain.push(fb);
            }
        }

        if model_chain.is_empty() {
            return Err(anyhow::anyhow!(
                "No model configured for sub-agent execution"
            ));
        }

        // Build extra system context for sub-agent
        let effective_max = super::max_depth_for_agent(&agent_id);
        let depth_info = if depth >= effective_max {
            format!(
                "- You are at maximum nesting depth ({}/{}) and CANNOT spawn further sub-agents.",
                depth, effective_max
            )
        } else {
            format!(
                "- Current nesting depth: {}/{}. You can delegate to sub-agents if needed.",
                depth, effective_max
            )
        };

        let extra_context = format!(
        "## Execution Context\n\
         You are running as a **sub-agent** spawned by another agent.\n\
         - Task: {}\n\
         - {}\n\
         - Complete the task directly and concisely. Your full response will be returned to the parent agent.\n\
         - You do NOT have access to the parent's conversation history.\n\
         - This is an isolated session.",
        &task, depth_info
    );

        let mut last_error = String::new();
        for (_idx, model_ref) in model_chain.iter().enumerate() {
            let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
                Some(p) => p,
                None => continue,
            };

            let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);
            let mut retry_count: u32 = 0;

            loop {
                // Check cancellation before each attempt
                if cancel.load(Ordering::SeqCst) {
                    return Err(anyhow::anyhow!("Sub-agent cancelled"));
                }

                let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id)
                    .with_failover_context(prov);
                agent.set_agent_id(&agent_id);
                // Use custom system context if provided (e.g., PLAN_MODE_SYSTEM_PROMPT), otherwise use default
                if let Some(ref ctx) = extra_system_context_override {
                    agent.set_extra_system_context(format!("{}\n\n{}", ctx, extra_context));
                } else {
                    agent.set_extra_system_context(extra_context.clone());
                }
                agent.set_subagent_depth(depth);
                agent.set_steer_run_id(run_id.clone());
                // Apply plan agent mode if configured (for plan creation sub-agents)
                if let Some(ref mode) = plan_agent_mode {
                    agent.set_plan_agent_mode(mode.clone());
                    agent.set_plan_mode_allow_paths(plan_mode_allow_paths.clone());
                }
                // Apply skill-level tool restriction (for fork-mode skills)
                if !skill_allowed_tools.is_empty() {
                    agent.set_skill_allowed_tools(skill_allowed_tools.clone());
                }
                // Apply denied_tools from parent agent's subagent config
                let mut denied = Vec::new();
                if let Ok(parent_def) = crate::agent_loader::load_agent(&agent_id) {
                    if !parent_def.config.subagents.denied_tools.is_empty() {
                        denied.extend(parent_def.config.subagents.denied_tools.clone());
                    }
                }
                // Inherit plan mode tool restrictions from parent session
                // (prevents subagents from bypassing plan mode safety)
                // Skip if this sub-agent has its own plan_agent_mode (it IS the plan agent)
                if plan_agent_mode.is_none() {
                    let parent_plan_state = crate::plan::get_plan_state(&parent_session_id).await;
                    if matches!(
                        parent_plan_state,
                        crate::plan::PlanModeState::Planning | crate::plan::PlanModeState::Review
                    ) {
                        for tool in crate::plan::PLAN_MODE_DENIED_TOOLS {
                            let t = tool.to_string();
                            if !denied.contains(&t) {
                                denied.push(t);
                            }
                        }
                    }
                }
                if !denied.is_empty() {
                    agent.set_denied_tools(denied);
                }

                let cancel_clone = cancel.clone();
                let effort_ref = reasoning_effort.as_deref();
                match agent
                    .chat(&task, &attachments, effort_ref, cancel_clone, |_delta| {})
                    .await
                {
                    Ok((response, _thinking)) => {
                        return Ok((response, Some(model_label)));
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        let reason = failover::classify_error(&last_error);

                        if reason.is_terminal() {
                            return Err(anyhow::anyhow!("{}", last_error));
                        }

                        if reason.is_retryable() && retry_count < MAX_RETRIES {
                            retry_count += 1;
                            let delay = std::cmp::min(
                                RETRY_BASE_MS * 2u64.pow(retry_count - 1),
                                RETRY_MAX_MS,
                            );
                            app_warn!(
                                "subagent",
                                "retry",
                                "Model {} failed ({:?}), retry {}/{} in {}ms: {}",
                                model_label,
                                reason,
                                retry_count,
                                MAX_RETRIES,
                                delay,
                                last_error
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            continue;
                        }

                        // Non-retryable or exhausted retries — try next model
                        app_warn!(
                            "subagent",
                            "failover",
                            "Model {} failed ({:?}), moving to next model: {}",
                            model_label,
                            reason,
                            last_error
                        );
                        break;
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "All models failed for sub-agent: {}",
            last_error
        ))
    } // async move
}

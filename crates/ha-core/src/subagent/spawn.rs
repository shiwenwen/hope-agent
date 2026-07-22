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
    default_timeout_for_agent, max_concurrent_for_agent, max_depth_for_agent, queue,
    MAX_RESULT_CHARS,
};

fn usage_tokens(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok())
}

// ── Spawn Logic ─────────────────────────────────────────────────

/// `SpawnParams.label` value used by the `agent` hook handler. Subagents
/// spawned with this label are children OF a hook, so they MUST NOT fire
/// `SubagentStart` / `SubagentStop` observation hooks: a `SubagentStart`
/// agent handler would otherwise spawn another labelled child on every fire,
/// cascading without bound (the matcher target is the spawned `subagent_id`,
/// so each new spawn re-matches and re-spawns).
///
/// Kept here as a single source of truth so the spawn site
/// ([`crate::hooks::runner::agent::AgentHandler::run`]) and the gate inside
/// `spawn_subagent` agree about the marker string.
pub const HOOK_SPAWN_LABEL: &str = "hook";

/// Whether this spawn came from an `agent` hook handler — the cascade guard.
fn is_hook_spawn(label: Option<&str>) -> bool {
    label == Some(HOOK_SPAWN_LABEL)
}

fn append_extra_system_context(existing: Option<String>, addition: String) -> Option<String> {
    Some(match existing {
        Some(current) if !current.trim().is_empty() => format!("{current}\n\n{addition}"),
        _ => addition,
    })
}

/// Spawn a sub-agent asynchronously. Returns the run_id immediately.
pub async fn spawn_subagent(
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) -> Result<String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    spawn_subagent_with_run_id(params, session_db, cancel_registry, run_id).await
}

/// Spawn a sub-agent using a caller-preallocated run id.
///
/// This is used by durable workflow replay: the workflow op stores the run id as
/// `child_handle` before the side effect is launched, so recovery can reattach to
/// or safely retry the same child instead of creating an untracked duplicate.
pub(crate) async fn spawn_subagent_with_run_id(
    mut params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    run_id: String,
) -> Result<String> {
    let run_id = uuid::Uuid::parse_str(&run_id)
        .map(|id| id.to_string())
        .map_err(|_| anyhow::anyhow!("preallocated sub-agent run id must be a UUID"))?;

    // ── Structural limits: hard-reject (a breach can't become legal by waiting;
    // guarded by `structural_limit_tests`). ──
    // 1. Depth (use parent agent's configured max).
    let effective_max_depth = max_depth_for_agent(&params.parent_agent_id);
    if params.depth > effective_max_depth {
        return Err(anyhow::anyhow!(
            "Sub-agent depth limit reached ({}/{}). Cannot spawn further sub-agents.",
            params.depth,
            effective_max_depth
        ));
    }
    // 2. Agent exists.
    let _agent_run_admission = crate::agent_lifecycle::begin_agent_run(&params.agent_id)
        .map_err(|e| anyhow::anyhow!("Agent '{}' is unavailable: {}", params.agent_id, e))?;

    // ── Resource limit (R7.2): at the per-session concurrency limit, PARK the
    // spawn as `Queued` instead of rejecting — the subagent scheduler
    // ([`super::queue`]) promotes it when a running child settles. A full queue
    // is the only hard reject here (the queue pins live `SpawnParams` in RAM, so
    // it must stay bounded). `Queued` is excluded from `count_active_subagent_runs`
    // so a parked run can't inflate the count and deadlock its own promotion. ──
    let max_concurrent = max_concurrent_for_agent(&params.parent_agent_id);
    let active_count = session_db.count_active_subagent_runs(&params.parent_session_id)?;
    let should_queue = active_count >= max_concurrent;
    if should_queue && queue::is_full() {
        return Err(anyhow::anyhow!(
            "Sub-agent queue is full. Wait for some to complete or kill them."
        ));
    }
    let initial_status = if should_queue {
        SubagentStatus::Queued
    } else {
        SubagentStatus::Spawning
    };

    // 4. Create isolated session (linked to parent)
    let child_session = {
        let db = session_db.clone();
        let agent_id = params.agent_id.clone();
        let parent_session_id = params.parent_session_id.clone();
        db.run(move |db| db.create_session_with_parent(&agent_id, Some(&parent_session_id)))
            .await?
    };
    let child_session_id = child_session.id.clone();
    let eval_child_guard = match crate::eval_context::context_for_session(&params.parent_session_id)
    {
        Some(context) => Some(crate::eval_context::register_child_session_from_parent(
            &params.parent_session_id,
            &child_session_id,
            context,
        )?),
        None => None,
    };

    // Set a descriptive title for the sub-agent session
    let task_preview = truncate_str(&params.task, 50);
    {
        let db = session_db.clone();
        let sid = child_session_id.clone();
        let title = task_preview.clone();
        let _ = db
            .run(move |db| db.update_session_title(&sid, &title))
            .await;
    }

    let mut assigned_child_working_dir = false;
    if params.isolate_worktree {
        match session_db
            .create_managed_worktree(crate::worktree::CreateManagedWorktreeInput {
                session_id: params.parent_session_id.clone(),
                source_working_dir: None,
                label: params.label.clone().or_else(|| Some(task_preview.clone())),
                purpose: crate::worktree::ManagedWorktreePurpose::Subagent,
                workflow_run_id: None,
                child_session_id: Some(child_session_id.clone()),
                base_ref: None,
                include_local_changes: false,
                bootstrap_request_id: None,
                bind_session_working_dir: false,
            })
            .await
        {
            Ok(worktree) => {
                let update_result = {
                    let db = session_db.clone();
                    let sid = child_session_id.clone();
                    let path = worktree.path.clone();
                    db.run(move |db| db.update_session_working_dir(&sid, Some(path)))
                        .await
                };
                match update_result {
                    Ok(_) => {
                        assigned_child_working_dir = true;
                        params.extra_system_context = append_extra_system_context(
                            params.extra_system_context.take(),
                            format!(
                                "## Managed Worktree\nThis sub-agent has an isolated managed git worktree at `{}`. Treat this as the default workspace for file reads, edits, commands, and evidence gathering. The parent session tracks it as `{}` for handoff, restore, and cleanup.",
                                worktree.path, worktree.id
                            ),
                        );
                    }
                    Err(e) => {
                        crate::app_warn!(
                            "subagent",
                            "worktree",
                            "created worktree {} but failed to assign child session cwd: {}",
                            worktree.id,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                crate::app_warn!(
                    "subagent",
                    "worktree",
                    "failed to create isolated worktree for run {}: {}",
                    run_id,
                    e
                );
            }
        }
    }
    if !assigned_child_working_dir {
        if let Some(parent_cwd) =
            crate::session::effective_session_working_dir(Some(&params.parent_session_id))
        {
            let inherit_result = {
                let db = session_db.clone();
                let sid = child_session_id.clone();
                db.run(move |db| db.update_session_working_dir(&sid, Some(parent_cwd)))
                    .await
            };
            if let Err(e) = inherit_result {
                crate::app_warn!(
                    "subagent",
                    "worktree",
                    "failed to inherit parent working dir for child session {}: {}",
                    child_session_id,
                    e
                );
            }
        }
    }

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
        status: initial_status.clone(),
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

    // R6: project user-delegated background subagent runs into the unified
    // `background_jobs` surface (one-way; `subagent_runs` stays the truth
    // source) so they appear in `job_status` list/cancel + the future panel.
    // Gate: only runs that auto-inject (`!skip_parent_injection` — excludes
    // internal plan / team / hook spawns) and non-incognito parents (close-and-
    // burn leaves no persisted trace). The incognito check uses the canonical
    // `is_session_incognito` helper, which fail-closes a missing/burned parent
    // row to incognito (do NOT project on `Ok(None)`). Best-effort: a projection
    // failure must never block the spawn.
    //
    // R5: a `batch_spawn` child carries its owning Group's id in
    // `params.group_id`. `effective_group_id` is `Some` ONLY when the child is
    // grouped AND its projection was created — a projection-insert failure
    // ungroups the child so it falls back to its own per-child injection
    // (below) rather than stranding its result with no delivery path (the Group
    // join only tracks children it can see as projections).
    let mut effective_group_id: Option<String> = None;
    if !params.skip_parent_injection
        && !crate::session::is_session_incognito(Some(&params.parent_session_id))
    {
        match crate::async_jobs::JobManager::project_subagent_spawn(
            &run_id,
            &params.parent_session_id,
            &params.parent_agent_id,
            &params.agent_id,
            initial_status.clone(),
            params.group_id.as_deref(),
        ) {
            Ok(()) => effective_group_id = params.group_id.clone(),
            Err(e) => crate::app_warn!(
                "subagent",
                "spawn",
                "Failed to project subagent run {} into background_jobs: {}",
                run_id,
                e
            ),
        }
    }

    // R7.2: over the concurrency limit → PARK as `Queued`; the subagent
    // scheduler promotes it when a running child settles. Otherwise launch now.
    if should_queue {
        // Register the cancel flag NOW, at park time, so `request_cancel_run`
        // can trip a flag that the promoted run REUSES (see
        // `SubagentCancelRegistry::register`, which is get-or-create). Without
        // this, a cancel arriving in the window between the scheduler's dequeue
        // and the promoted run registering its own flag would create a fresh
        // (untripped) flag — letting a killed run execute to completion and
        // inject its result.
        cancel_registry.register(&run_id);
        if !queue::enqueue(queue::PendingSubagentSpawn {
            params,
            run_id: run_id.clone(),
            child_session_id,
            effective_group_id,
            enqueued_at: std::time::Instant::now(),
            eval_guard: eval_child_guard,
        }) {
            // Lost the cap race after the earlier check — settle the row and
            // drop the just-registered flag so we never leave a dangling
            // `Queued` run with no queue entry.
            cancel_registry.remove(&run_id);
            let _ = session_db.update_subagent_status(
                &run_id,
                SubagentStatus::Killed,
                None,
                Some("Sub-agent queue full"),
                None,
                None,
            );
            return Err(anyhow::anyhow!(
                "Sub-agent queue is full. Wait for some to complete or kill them."
            ));
        }
        return Ok(run_id);
    }

    launch_subagent_run(
        params,
        run_id.clone(),
        child_session_id,
        effective_group_id,
        eval_child_guard,
        0,
        session_db,
        cancel_registry,
    );
    Ok(run_id)
}

/// Launch a sub-agent run: register the cancel flag + steer mailbox, emit the
/// `spawned` event, fire `SubagentStart`, and spawn the execution task. The run
/// row + projection already exist (status `Spawning`). Called directly by
/// [`spawn_subagent`] for an under-limit spawn, and by the subagent scheduler
/// ([`super::queue`]) when promoting a previously `Queued` run.
pub(crate) fn launch_subagent_run(
    params: SpawnParams,
    run_id: String,
    child_session_id: String,
    effective_group_id: Option<String>,
    eval_child_guard: Option<crate::eval_context::EvalSessionGuard>,
    queue_wait_ms: u64,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) {
    let task_preview = truncate_str(&params.task, 50);
    crate::eval_context::record_queue_wait(
        Some(&child_session_id),
        "subagent",
        &run_id,
        queue_wait_ms,
    );
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

    // SubagentStart observation hook (sub-agent spawned). Parent session is the
    // hook session; the spawned agent id is the matcher target.
    //
    // Skip the fire when this spawn itself originated from an `agent` hook
    // handler ([`HOOK_SPAWN_LABEL`]) — otherwise a SubagentStart agent handler
    // re-spawns a labelled child on every fire, cascading without bound.
    if !is_hook_spawn(params.label.as_deref()) {
        crate::hooks::fire_subagent_start(&params.parent_session_id, &params.agent_id, &run_id);
    }

    // 8. Spawn async task
    let run_id_clone = run_id.clone();
    let db = session_db.clone();
    let registry = cancel_registry.clone();
    let agent_id = params.agent_id.clone();
    let task = params.task.clone();
    let depth = params.depth;
    let parent_agent_id = params.parent_agent_id.clone();
    let timeout_secs = params
        .timeout_secs
        .unwrap_or_else(|| default_timeout_for_agent(&parent_agent_id));
    let model_override = params.model_override.clone();
    let parent_session_id = params.parent_session_id.clone();
    let child_session_id_clone = child_session_id.clone();
    let label = params.label.clone();
    let attachments = params.attachments.clone();
    let plan_agent_mode = params.plan_agent_mode.clone();
    let plan_mode_allow_paths = params.plan_mode_allow_paths.clone();
    let lock_plan_agent_mode = params.lock_plan_agent_mode;
    let skip_parent_injection = params.skip_parent_injection;
    // R5: a grouped child suppresses its individual completion injection — the
    // Group fires ONE merged injection when every child settles (see below).
    let grouped = effective_group_id.is_some();
    let extra_system_context = params.extra_system_context.clone();
    let skill_allowed_tools = params.skill_allowed_tools.clone();
    let reasoning_effort = params.reasoning_effort.clone();
    let skill_name_for_events = params.skill_name.clone();
    // Parent turn's KB-access origin (D10) — forwarded to the child engine so an
    // IM-origin chain can't reacquire KB access via the neutral Subagent source.
    let origin_source = params.origin_source;
    // Parent turn's IM origin identity (WS8) — forwarded so the child's KB opt-in
    // is judged against the account/chat that started the chain.
    let origin_channel_kb_context = params.origin_channel_kb_context.clone();

    tokio::spawn(async move {
        // Keeps campaign attribution alive across the child task's complete
        // lifecycle, including timeout/cancel/final injection cleanup.
        let _eval_child_guard = eval_child_guard;
        if let Some(fault) = crate::eval_context::scheduler_fault_action(
            Some(&child_session_id_clone),
            &run_id_clone,
        ) {
            if fault.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(fault.delay_ms)).await;
            }
        }
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
        let lock_plan_agent_mode_exec = lock_plan_agent_mode;
        let extra_system_context_exec = extra_system_context.clone();
        let skill_allowed_tools_exec = skill_allowed_tools.clone();
        let reasoning_effort_exec = reasoning_effort.clone();
        let child_session_id_exec = child_session_id_clone.clone();

        let _ = db.append_message(
            &child_session_id_exec,
            &crate::session::NewMessage::user(&task)
                .with_source(crate::chat_engine::ChatSource::Subagent),
        );

        enum ExecutionResult {
            Finished(Result<(String, Option<String>, crate::chat_engine::CapturedUsage)>),
            Timeout,
        }

        let execution = execute_subagent(
            agent_id_exec,
            task_exec,
            depth,
            model_override_exec,
            cancel_exec,
            run_id_exec,
            child_session_id_exec,
            db.clone(),
            attachments_exec,
            parent_session_id.clone(),
            plan_agent_mode_exec,
            plan_mode_allow_paths_exec,
            lock_plan_agent_mode_exec,
            extra_system_context_exec,
            skill_allowed_tools_exec,
            reasoning_effort_exec,
            origin_source,
            origin_channel_kb_context,
        );

        let exec_result = std::panic::AssertUnwindSafe(async move {
            if timeout_secs == 0 {
                ExecutionResult::Finished(execution.await)
            } else {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), execution)
                    .await
                {
                    Ok(result) => ExecutionResult::Finished(result),
                    Err(_) => ExecutionResult::Timeout,
                }
            }
        });
        let result = futures_util::FutureExt::catch_unwind(exec_result).await;

        let duration_ms = start.elapsed().as_millis() as u64;
        let finished_at = chrono::Utc::now().to_rfc3339();

        // Determine outcome — handles Ok, Err, Timeout, Cancel, and Panic
        let (status, result_text, error_text, model_used, usage) = match result {
            Ok(ExecutionResult::Finished(Ok((response, model, usage)))) => {
                let truncated = truncate_str(&response, MAX_RESULT_CHARS);
                (
                    SubagentStatus::Completed,
                    Some(truncated),
                    None,
                    model,
                    usage,
                )
            }
            Ok(ExecutionResult::Finished(Err(e))) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    (
                        SubagentStatus::Killed,
                        None,
                        Some("Killed by parent".into()),
                        None,
                        Default::default(),
                    )
                } else {
                    (
                        SubagentStatus::Error,
                        None,
                        Some(e.to_string()),
                        None,
                        Default::default(),
                    )
                }
            }
            Ok(ExecutionResult::Timeout) => {
                // Timeout
                (
                    SubagentStatus::Timeout,
                    None,
                    Some(format!("Timed out after {}s", timeout_secs)),
                    None,
                    Default::default(),
                )
            }
            Err(_panic) => {
                // Panic caught — still deliver the event
                (
                    SubagentStatus::Error,
                    None,
                    Some("Sub-agent panicked unexpectedly".into()),
                    None,
                    Default::default(),
                )
            }
        };
        let input_tokens = usage_tokens(usage.input_tokens);
        let output_tokens = usage_tokens(usage.output_tokens);

        if !matches!(status, SubagentStatus::Completed) {
            let reply_text = error_text
                .as_deref()
                .or(result_text.as_deref())
                .unwrap_or("(no response)");
            let _ = db.append_message(
                &child_session_id,
                &crate::session::NewMessage::error_event(reply_text)
                    .with_source(crate::chat_engine::ChatSource::Subagent),
            );
        }

        // Update DB — guaranteed to run even after panic
        let _ = db.update_subagent_status(
            &run_id_clone,
            status.clone(),
            result_text.as_deref(),
            error_text.as_deref(),
            model_used.as_deref(),
            Some(duration_ms),
        );
        let _ = db.set_subagent_usage(&run_id_clone, input_tokens, output_tokens);
        let _ = db.set_subagent_finished_at(&run_id_clone, &finished_at);

        // Emit completion event — guaranteed to fire
        let result_preview = result_text.as_ref().map(|r| truncate_str(r, 200));
        // Clone values needed after the move into SubagentEvent
        // SubagentStop observation hook (terminal state) — fired before the
        // values are moved into the completion event below. Skipped for
        // hook-originated spawns (mirrors the SubagentStart gate above so a
        // SubagentStop agent handler can't recurse).
        if !is_hook_spawn(label.as_deref()) {
            crate::hooks::fire_subagent_stop(
                &parent_session_id,
                &agent_id,
                &run_id_clone,
                status.as_str(),
                result_preview.as_deref(),
            );
        }

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
            input_tokens,
            output_tokens,
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
        // R5: `grouped` children never inject individually — their Group joins
        // all child results into ONE merged injection (covering every terminal
        // status, including Killed, which the per-child path below skips).
        if !skip_parent_injection
            && !grouped
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
                    Ok(rt) => {
                        // Subagent runs track completion via FETCHED_RUN_IDS /
                        // subagent_runs status, not an async-job row — no
                        // on_injected callback, and the outcome is ignored.
                        let _ = rt.block_on(inject_and_run_parent(
                            parent_sid2,
                            parent_agent_id2,
                            child_agent_id2,
                            run_id2,
                            push_msg,
                            db2,
                            None,
                        ));
                    }
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
}

/// Execute the sub-agent (runs within the spawned tokio task).
/// Returns (response_text, model_used, captured_usage).
///
/// `+ Send` is declared explicitly so the spawner's `tokio::spawn` bounds
/// stay self-documenting. Collapsing to `async fn` would infer the bound
/// from captures, which is less explicit about the Send contract.
#[allow(clippy::manual_async_fn)]
fn execute_subagent(
    agent_id: String,
    task: String,
    depth: u32,
    model_override: Option<String>,
    cancel: Arc<AtomicBool>,
    run_id: String,
    child_session_id: String,
    session_db: Arc<SessionDB>,
    attachments: Vec<crate::agent::Attachment>,
    parent_session_id: String,
    plan_agent_mode: Option<crate::agent::PlanAgentMode>,
    plan_mode_allow_paths: Vec<String>,
    lock_plan_agent_mode: bool,
    extra_system_context_override: Option<String>,
    skill_allowed_tools: Vec<String>,
    reasoning_effort: Option<String>,
    origin_source: Option<crate::knowledge::KbAccessSource>,
    origin_channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
) -> impl std::future::Future<
    Output = Result<(String, Option<String>, crate::chat_engine::CapturedUsage)>,
> + Send {
    async move {
        use crate::provider;

        let store = crate::config::cached_config();

        // Load agent config for model resolution
        let agent_def = crate::agent_loader::load_agent(&agent_id)?;
        let effective_reasoning_effort =
            reasoning_effort.or_else(|| agent_def.config.model.reasoning_effort.clone());
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

        let mut denied = agent_def.config.subagents.denied_tools.clone();
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

        let extra_system_context = if let Some(ctx) = extra_system_context_override {
            Some(format!("{}\n\n{}", ctx, extra_context))
        } else {
            Some(extra_context)
        };

        // Spawn-supplied PlanAgent (e.g. spawn_plan_subagent): translate the
        // explicit mode + paths into a PlanResolvedContext override so the
        // chat engine bypasses backend probe (the child session's
        // `plan_mode = Off` and would otherwise clobber PlanAgent). Generic
        // sub-agents leave the override `None` so chat_engine reads the
        // child session's own backend state.
        //
        // `extra_system_context` (already-merged spawn-generic + caller
        // extras above) flows through ChatEngineParams.extra_system_context
        // unchanged — chat_engine's `merge_extra_system_context` will fold
        // it together with whatever the override / backend resolution
        // contributed (currently `None` from this path; spawn callers put
        // any plan-prompt text into the caller's extra_system_context).
        let plan_context_override = if lock_plan_agent_mode {
            plan_agent_mode.map(|mode| crate::chat_engine::PlanResolvedContext {
                // Spawn-supplied PlanAgent always means "child should run
                // as if it were in Planning" — the locked flag freezes
                // this against the mid-turn probe regardless.
                state: crate::plan::PlanModeState::Planning,
                mode,
                allow_paths: plan_mode_allow_paths,
                extra_system_context: None,
            })
        } else {
            None
        };

        let result = crate::chat_engine::run_chat_engine(crate::chat_engine::ChatEngineParams {
            session_id: child_session_id,
            agent_id: agent_id.clone(),
            turn_id: None,
            message: task,
            display_text: None,
            attachments,
            session_db,
            model_chain,
            providers: store.providers.clone(),
            codex_token: None,
            resolved_temperature: agent_def.config.model.temperature.or(store.temperature),
            compact_config: store.compact.clone(),
            extra_system_context,
            reasoning_effort: effective_reasoning_effort,
            cancel,
            plan_context_override,
            skill_allowed_tools,
            denied_tools: denied,
            tool_scope: None,
            subagent_depth: depth,
            steer_run_id: Some(run_id),
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: true,
            persist_final_error_event: false,
            source: crate::chat_engine::stream_seq::ChatSource::Subagent,
            origin_source,
            channel_kb_context: origin_channel_kb_context,
            event_sink: Arc::new(crate::chat_engine::NoopEventSink),
        })
        .await
        .map_err(|e| anyhow::anyhow!("All models failed for sub-agent: {}", e))?;

        let model_used = result.model_used.as_ref().map(ToString::to_string);
        Ok((result.response, model_used, result.usage))
    } // async move
}

#[cfg(test)]
mod hook_label_tests {
    use super::*;

    #[test]
    fn hook_label_const_recognized() {
        // The single source of truth for the marker string. Both
        // `crate::hooks::runner::agent::AgentHandler::run` and the gates in
        // `spawn_subagent` read this — drifting them apart re-opens the
        // SubagentStart/SubagentStop cascade.
        assert_eq!(HOOK_SPAWN_LABEL, "hook");
        assert!(is_hook_spawn(Some(HOOK_SPAWN_LABEL)));
    }

    #[test]
    fn non_hook_labels_dispatch_normally() {
        // Unlabelled spawns (the model's `subagent` tool, agent team picks)
        // and any other label must still fire the observation events.
        assert!(!is_hook_spawn(None));
        assert!(!is_hook_spawn(Some("")));
        assert!(!is_hook_spawn(Some("agent-team")));
        assert!(!is_hook_spawn(Some("subagent-tool")));
    }
}

/// R7.0/R7.4 acceptance: structural limits (`depth` / `batch` / `turn`) must
/// REJECT when hit — never queue. Per R7.0's three-way taxonomy, a structural
/// breach can't become legal by waiting (unlike a resource/cost limit, which
/// the R7.1 background-job queue defers), so it fails fast with an error and is
/// NOT routed through any [`crate::async_jobs::slots`] queue. This guards
/// against R7.1's "reject → queue" change ever leaking into the structural path.
#[cfg(test)]
mod structural_limit_tests {
    use super::*;
    use std::sync::Arc;

    fn params_at_depth(depth: u32) -> SpawnParams {
        SpawnParams {
            task: "t".into(),
            // Nonexistent agent → `max_depth_for_agent` falls back to the
            // hardcoded DEFAULT_MAX_DEPTH (3), independent of on-disk config.
            agent_id: "__nonexistent_for_test__".into(),
            parent_session_id: "s".into(),
            parent_agent_id: "__nonexistent_for_test__".into(),
            depth,
            timeout_secs: None,
            model_override: None,
            label: None,
            isolate_worktree: false,
            attachments: Vec::new(),
            plan_agent_mode: None,
            plan_mode_allow_paths: Vec::new(),
            lock_plan_agent_mode: false,
            skip_parent_injection: false,
            extra_system_context: None,
            skill_allowed_tools: Vec::new(),
            reasoning_effort: None,
            skill_name: None,
            origin_source: None,
            origin_channel_kb_context: None,
            group_id: None,
        }
    }

    #[tokio::test]
    async fn subagent_depth_overflow_rejects_not_queues() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let registry = Arc::new(SubagentCancelRegistry::new());
        // Default cap is 3 (DEFAULT_MAX_DEPTH); depth 99 is structurally illegal.
        let err = spawn_subagent(params_at_depth(99), db, registry)
            .await
            .expect_err("a depth past the structural cap must reject, not queue");
        assert!(
            err.to_string().contains("depth limit"),
            "expected a depth-limit rejection, got: {err}"
        );
    }

    #[test]
    fn batch_size_cap_is_a_fixed_structural_limit() {
        // The `batch_spawn` fan-out cap is a fixed structural limit (default 10),
        // enforced by a hard reject in `action_batch_spawn` (`tasks.len() >
        // max_batch → Err`), NOT a resizable resource quota. Pin the default so a
        // future edit can't silently turn it into a tunable queue depth.
        assert_eq!(
            super::super::max_batch_size_for_agent("__nonexistent_for_test__"),
            10
        );
    }

    fn active_run(run_id: &str, parent_session: &str, agent: &str) -> SubagentRun {
        SubagentRun {
            run_id: run_id.into(),
            parent_session_id: parent_session.into(),
            parent_agent_id: agent.into(),
            child_agent_id: agent.into(),
            child_session_id: format!("child-{run_id}"),
            task: "t".into(),
            status: SubagentStatus::Running,
            result: None,
            error: None,
            depth: 1,
            model_used: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: None,
            duration_ms: None,
            label: None,
            attachment_count: 0,
            input_tokens: None,
            output_tokens: None,
        }
    }

    #[test]
    fn concurrency_over_limit_queues_instead_of_rejecting() {
        // R7.2: at the per-session concurrency limit, an extra spawn must PARK as
        // `Queued` (Ok) — NOT reject (the pre-R7.2 behavior). Uses a real on-disk
        // agent (the agent-exists check precedes the concurrency decision) with
        // `max_concurrent = 1`, and one pre-inserted active run so the next spawn
        // is over-limit. The env lock in `with_env_vars` serializes HA_DATA_DIR.
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let agent_id = "test-queue-agent";
            let dir = crate::paths::agent_dir(agent_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let mut cfg = crate::agent_config::AgentConfig::default();
            cfg.subagents.max_concurrent = 1;
            std::fs::write(dir.join("agent.json"), serde_json::to_string(&cfg).unwrap()).unwrap();

            let db = Arc::new(SessionDB::open(&root.path().join("s.db")).unwrap());
            let registry = Arc::new(SubagentCancelRegistry::new());
            let parent = db.create_session(agent_id).unwrap();

            // One active run → active_count (1) == max_concurrent (1): at limit.
            db.insert_subagent_run(&active_run("active-1", &parent.id, agent_id))
                .unwrap();

            let params = SpawnParams {
                task: "queued task".into(),
                agent_id: agent_id.into(),
                parent_session_id: parent.id.clone(),
                parent_agent_id: agent_id.into(),
                depth: 1,
                timeout_secs: None,
                model_override: None,
                label: None,
                isolate_worktree: false,
                attachments: Vec::new(),
                plan_agent_mode: None,
                plan_mode_allow_paths: Vec::new(),
                lock_plan_agent_mode: false,
                skip_parent_injection: false,
                extra_system_context: None,
                skill_allowed_tools: Vec::new(),
                reasoning_effort: None,
                skill_name: None,
                origin_source: None,
                origin_channel_kb_context: None,
                group_id: None,
            };

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let run_id = rt
                .block_on(spawn_subagent(params, db.clone(), registry))
                .expect("over-limit spawn must QUEUE (Ok), not reject");

            // Parked, not launched: row is Queued and the entry is in the queue.
            let run = db.get_subagent_run(&run_id).unwrap().unwrap();
            assert_eq!(
                run.status,
                SubagentStatus::Queued,
                "over-limit spawn must park as Queued"
            );

            // The parked spawn is dequeuable — the queue half of the cancel path
            // (the terminal stamp goes through the global SessionDB, exercised in
            // production wiring, not reachable with this test's local db).
            assert!(
                super::queue::remove_for_run(&run_id).is_some(),
                "the parked spawn must be in the in-memory queue and dequeuable"
            );

            // Leave the process-global queue clean for sibling tests.
            super::queue::purge_for_session(&parent.id);
        });
    }
}

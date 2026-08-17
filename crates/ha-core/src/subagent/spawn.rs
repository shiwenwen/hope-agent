use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::session::SessionDB;

use super::cancel::SubagentCancelRegistry;
use super::helpers::{emit_subagent_event, truncate_str};
use super::injection::{build_subagent_push_message, inject_and_run_parent_with_ui_guard};
use super::mailbox::SUBAGENT_MAILBOX;
use super::types::{
    SpawnParams, SubagentEvent, SubagentRun, SubagentStatus, SubagentTerminalReason,
};
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

fn append_run_instruction_context(existing: Option<String>, addition: String) -> Option<String> {
    Some(match existing {
        Some(current) if !current.trim().is_empty() => format!("{current}\n\n{addition}"),
        _ => addition,
    })
}

/// Durable control-plane fence checked by the final launch CAS.
///
/// Team members are prepared before they are attached to the roster. Carrying
/// this identity into both the immediate and queued launch paths makes
/// `Team=Active + member=Working + exact run/session` part of the same atomic
/// write that claims execution. A pause/dissolve that commits first therefore
/// prevents hooks and model/tool execution; a launch claim that commits first
/// is necessarily visible to the lifecycle snapshot.
#[derive(Debug, Clone)]
pub(crate) struct TeamMemberLaunchFence {
    pub team_id: String,
    pub member_id: String,
}

/// A sub-agent attempt whose child session and run row are durable, but whose
/// execution has not been scheduled and whose observation hooks have not fired.
///
/// The value is intentionally non-Clone: exactly one caller must either attach
/// and launch it, or settle it through [`discard_prepared_subagent`].
pub(crate) struct PreparedSubagentSpawn {
    params: SpawnParams,
    run_id: String,
    child_session_id: String,
    initial_status: SubagentStatus,
    should_queue: bool,
    effective_group_id: Option<String>,
    eval_child_guard: Option<crate::eval_context::EvalSessionGuard>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    // Keep Agent deletion/disable fenced throughout the prepare -> attach gap.
    _agent_run_admission: crate::agent_lifecycle::AgentRunGuard,
}

impl PreparedSubagentSpawn {
    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(crate) fn child_session_id(&self) -> &str {
        &self.child_session_id
    }
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
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    run_id: String,
) -> Result<String> {
    let prepared =
        prepare_subagent_with_run_id(params, session_db, cancel_registry, run_id).await?;
    launch_prepared_subagent(prepared, None).await
}

/// Materialize a fresh attempt without making it executable yet.
pub(crate) async fn prepare_subagent(
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) -> Result<PreparedSubagentSpawn> {
    let run_id = uuid::Uuid::new_v4().to_string();
    prepare_subagent_with_run_id(params, session_db, cancel_registry, run_id).await
}

async fn prepare_subagent_with_run_id(
    mut params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    run_id: String,
) -> Result<PreparedSubagentSpawn> {
    let run_id = uuid::Uuid::parse_str(&run_id)
        .map(|id| id.to_string())
        .map_err(|_| anyhow::anyhow!("preallocated sub-agent run id must be a UUID"))?;

    let parent_paused = {
        let db = session_db.clone();
        let parent_session_id = params.parent_session_id.clone();
        db.run(move |db| db.is_session_or_ancestor_autonomy_paused(&parent_session_id))
            .await?
    };
    if parent_paused {
        return Err(anyhow::anyhow!(
            "Parent session is paused by Stop; use Continue before spawning sub-agents"
        ));
    }

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
    let agent_run_admission = crate::agent_lifecycle::begin_agent_run(&params.agent_id)
        .map_err(|e| anyhow::anyhow!("Agent '{}' is unavailable: {}", params.agent_id, e))?;

    // ── Resource limit (R7.2): at the per-session concurrency limit, PARK the
    // spawn as `Queued` instead of rejecting — the subagent scheduler
    // ([`super::queue`]) promotes it when a running child settles. A full queue
    // is the only hard reject here (the queue pins live `SpawnParams` in RAM, so
    // it must stay bounded). `Queued` is excluded from `count_active_subagent_runs`
    // so a parked run can't inflate the count and deadlock its own promotion. ──
    let max_concurrent = max_concurrent_for_agent(&params.parent_agent_id);
    let active_count = {
        let db = session_db.clone();
        let parent_session_id = params.parent_session_id.clone();
        db.run(move |db| db.count_active_subagent_runs(&parent_session_id))
            .await?
    };
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
    let reattachable_ui_guard = crate::permission::register_reattachable_ui_child_session(
        &params.parent_session_id,
        &child_session_id,
    );

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
                        params.run_instruction_context = append_run_instruction_context(
                            params.run_instruction_context.take(),
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

    materialize_prepared_run(
        params,
        run_id,
        child_session_id,
        initial_status,
        should_queue,
        None,
        None,
        eval_child_guard,
        reattachable_ui_guard,
        session_db,
        cancel_registry,
        agent_run_admission,
    )
    .await
}

/// Continue a terminal sub-agent in its existing child session.
///
/// The source run stays immutable and terminal. A fresh run id is created for
/// the continuation, while the child session (conversation context, working
/// directory, and nested-session ancestry) is reused. This mirrors a follow-up
/// turn rather than resurrecting an old lifecycle record.
pub async fn resume_subagent(
    source_run_id: &str,
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    dispatch_id: Option<String>,
    preallocated_run_id: Option<String>,
) -> Result<String> {
    let parent_paused = {
        let db = session_db.clone();
        let parent_session_id = params.parent_session_id.clone();
        db.run(move |db| db.is_session_or_ancestor_autonomy_paused(&parent_session_id))
            .await?
    };
    if parent_paused {
        return Err(anyhow::anyhow!(
            "Parent session is paused by Stop; use Continue before resuming sub-agents"
        ));
    }
    let source = {
        let db = session_db.clone();
        let source_run_id = source_run_id.to_string();
        db.run(move |db| db.get_subagent_run(&source_run_id))
            .await?
    }
    .ok_or_else(|| anyhow::anyhow!("Sub-agent run '{}' not found", source_run_id))?;
    if !source.status.is_terminal() {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': it is still '{}' (use steer instead)",
            source_run_id,
            source.status.as_str()
        ));
    }
    if params.parent_session_id != source.parent_session_id {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': it belongs to a different parent session",
            source_run_id
        ));
    }
    if params.agent_id != source.child_agent_id || params.depth != source.depth {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': child identity does not match",
            source_run_id
        ));
    }
    if params.owner_kind != source.owner_kind || params.owner_id != source.owner_id {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': control-plane owner does not match",
            source_run_id
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
            "Cannot resume sub-agent run '{}': its terminal reason requires freshly spawned replacement work after an explicit user request",
            source_run_id
        ));
    }
    if params.group_id.is_some() {
        return Err(anyhow::anyhow!(
            "A resumed sub-agent run cannot join a batch group"
        ));
    }

    let child_session = {
        let db = session_db.clone();
        let child_session_id = source.child_session_id.clone();
        db.run(move |db| db.get_session(&child_session_id)).await?
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': child session no longer exists",
            source_run_id
        )
    })?;
    if child_session.parent_session_id.as_deref() != Some(source.parent_session_id.as_str())
        || child_session.agent_id != source.child_agent_id
    {
        return Err(anyhow::anyhow!(
            "Cannot resume sub-agent run '{}': child session identity is invalid",
            source_run_id
        ));
    }
    if let Some(working_dir) = child_session.working_dir.as_deref() {
        match tokio::fs::metadata(working_dir).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(anyhow::anyhow!(
                    "Cannot resume sub-agent run '{}': continuation workspace '{}' is not a directory",
                    source_run_id,
                    working_dir
                ));
            }
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Cannot resume sub-agent run '{}': continuation workspace '{}' is unavailable: {}",
                    source_run_id,
                    working_dir,
                    error
                ));
            }
        }
    }

    // Re-check Agent lifecycle admission for every continuation. The child may
    // have been disabled or removed since the source run completed.
    let agent_run_admission = crate::agent_lifecycle::begin_agent_run(&params.agent_id)
        .map_err(|e| anyhow::anyhow!("Agent '{}' is unavailable: {}", params.agent_id, e))?;

    let max_concurrent = max_concurrent_for_agent(&params.parent_agent_id);
    let active_count = {
        let db = session_db.clone();
        let parent_session_id = params.parent_session_id.clone();
        db.run(move |db| db.count_active_subagent_runs(&parent_session_id))
            .await?
    };
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
    let run_id = match preallocated_run_id {
        Some(run_id) => uuid::Uuid::parse_str(&run_id)
            .map(|id| id.to_string())
            .map_err(|_| anyhow::anyhow!("preallocated continuation run id must be a UUID"))?,
        None => uuid::Uuid::new_v4().to_string(),
    };
    let eval_child_guard = match crate::eval_context::context_for_session(&params.parent_session_id)
    {
        Some(context) => Some(crate::eval_context::register_child_session_from_parent(
            &params.parent_session_id,
            &source.child_session_id,
            context,
        )?),
        None => None,
    };
    let reattachable_ui_guard = crate::permission::register_reattachable_ui_child_session(
        &params.parent_session_id,
        &source.child_session_id,
    );

    let prepared = materialize_prepared_run(
        params,
        run_id,
        source.child_session_id,
        initial_status,
        should_queue,
        Some(source_run_id),
        dispatch_id.as_deref(),
        eval_child_guard,
        reattachable_ui_guard,
        session_db,
        cancel_registry,
        agent_run_admission,
    )
    .await?;
    let run_id = launch_prepared_subagent(prepared, None).await?;
    // Reading a terminal run in order to continue it also consumes that result;
    // suppress a late duplicate auto-injection from the source run.
    if source.delivery_kind == crate::subagent::SubagentDeliveryKind::Parent {
        // `insert_resumed_subagent_run` atomically suppressed a still-pending
        // durable delivery (and refused to race an active one) in the same
        // transaction that created this continuation. Only the process-local
        // fast path remains here, chiefly for incognito delivery.
        super::mark_run_fetched_in_memory(source_run_id);
    }
    Ok(run_id)
}

#[allow(clippy::too_many_arguments)]
async fn materialize_prepared_run(
    mut params: SpawnParams,
    run_id: String,
    child_session_id: String,
    initial_status: SubagentStatus,
    should_queue: bool,
    resumed_from_run_id: Option<&str>,
    resume_dispatch_id: Option<&str>,
    eval_child_guard: Option<crate::eval_context::EvalSessionGuard>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
    agent_run_admission: crate::agent_lifecycle::AgentRunGuard,
) -> Result<PreparedSubagentSpawn> {
    // Insert a fresh immutable run record. Continuations use the transactional
    // insert variant so two resumes cannot overlap on one child conversation.
    let now = chrono::Utc::now().to_rfc3339();
    let attachment_count = params.attachments.len() as u32;
    let attachment_refs: Vec<_> = params
        .attachments
        .iter()
        .map(|attachment| {
            serde_json::json!({
                "name": &attachment.name,
                "mimeType": &attachment.mime_type,
                "filePath": &attachment.file_path,
                "uploadId": &attachment.upload_id,
            })
        })
        .collect();
    let plan_agent_mode = params.plan_agent_mode.as_ref().map(|mode| match mode {
        crate::agent::PlanAgentMode::Off => "off",
        crate::agent::PlanAgentMode::PlanAgent { .. } => "plan_agent",
        crate::agent::PlanAgentMode::ExecutingAgent => "executing_agent",
    });
    let launch_spec_json = serde_json::json!({
        "task": &params.task,
        "timeoutSecs": params.timeout_secs,
        "requestedModel": &params.model_override,
        "isolateWorktree": params.isolate_worktree,
        "attachments": attachment_refs,
        "planAgentMode": plan_agent_mode,
        "reasoningEffort": &params.reasoning_effort,
    })
    .to_string();
    let trigger_kind = match (resumed_from_run_id.is_some(), params.owner_kind) {
        (false, _) => "spawn",
        (true, crate::subagent::SubagentOwnerKind::Workflow) => "workflow_resume",
        (true, crate::subagent::SubagentOwnerKind::ParentSession) => "parent_followup",
        (true, _) => "internal",
    };
    let run = SubagentRun {
        run_id: run_id.clone(),
        thread_id: child_session_id.clone(),
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
        started_at: now.clone(),
        finished_at: None,
        duration_ms: None,
        label: params.label.clone(),
        attachment_count,
        input_tokens: None,
        output_tokens: None,
        continuation_of_run_id: resumed_from_run_id.map(str::to_string),
        trigger_kind: trigger_kind.to_string(),
        terminal_reason: None,
        runner_owner: Some(super::runtime_owner_token().to_string()),
        lease_epoch: 1,
        last_heartbeat_at: Some(now.clone()),
        delivery_kind: params.delivery_kind,
        launch_spec_json: Some(launch_spec_json),
        owner_kind: params.owner_kind,
        owner_id: params.owner_id.clone(),
    };
    if let Some(source_run_id) = resumed_from_run_id {
        let db = session_db.clone();
        let source_run_id = source_run_id.to_string();
        let run = run.clone();
        let dispatch_id = resume_dispatch_id.map(str::to_string);
        let dispatch_message = run.task.clone();
        db.run(move |db| {
            db.insert_resumed_subagent_run(
                &source_run_id,
                &run,
                dispatch_id.as_deref(),
                dispatch_id.as_ref().map(|_| dispatch_message.as_str()),
            )
        })
        .await?;
    } else {
        let db = session_db.clone();
        let run = run.clone();
        db.run(move |db| db.insert_subagent_run(&run)).await?;
    }

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
    if params.group_id.is_some() {
        let actual_delivery = if effective_group_id.is_some() {
            crate::subagent::SubagentDeliveryKind::Group
        } else {
            crate::subagent::SubagentDeliveryKind::Parent
        };
        if params.delivery_kind != actual_delivery {
            let delivery_db = session_db.clone();
            let delivery_run_id = run_id.clone();
            delivery_db
                .run(move |db| db.set_subagent_delivery_kind(&delivery_run_id, actual_delivery))
                .await?;
            params.delivery_kind = actual_delivery;
        }
    }

    // Register the cancel flag while the attempt is still non-executable. Team
    // attach happens only after this returns, so pause/dissolve can never see an
    // attached run without also having a token capable of fencing its launch.
    cancel_registry.register(&run_id);

    Ok(PreparedSubagentSpawn {
        params,
        run_id,
        child_session_id,
        initial_status,
        should_queue,
        effective_group_id,
        eval_child_guard,
        reattachable_ui_guard,
        session_db,
        cancel_registry,
        _agent_run_admission: agent_run_admission,
    })
}

/// Launch an already-materialized attempt. For Team members, `team_fence` is
/// persisted into a queued entry and checked by the final status CAS, not by a
/// fallible read-before-write pre-check.
pub(crate) async fn launch_prepared_subagent(
    prepared: PreparedSubagentSpawn,
    team_fence: Option<TeamMemberLaunchFence>,
) -> Result<String> {
    let PreparedSubagentSpawn {
        params,
        run_id,
        child_session_id,
        initial_status,
        should_queue,
        effective_group_id,
        eval_child_guard,
        reattachable_ui_guard,
        session_db,
        cancel_registry,
        _agent_run_admission,
    } = prepared;

    if should_queue {
        let cancel_flag = cancel_registry.register(&run_id);
        if !queue::enqueue(queue::PendingSubagentSpawn {
            params,
            run_id: run_id.clone(),
            child_session_id,
            effective_group_id,
            enqueued_at: std::time::Instant::now(),
            eval_guard: eval_child_guard,
            reattachable_ui_guard,
            team_fence,
        }) {
            settle_unlaunched_run(
                &session_db,
                &cancel_registry,
                &run_id,
                "Sub-agent queue full before launch",
            )
            .await?;
            return Err(anyhow::anyhow!(
                "Sub-agent queue is full. Wait for some to complete or kill them."
            ));
        }
        // Close prepare -> enqueue: pause may have observed the attached run and
        // tripped its pre-registered token before an entry existed for the
        // canonical cancel path to claim. Recheck after enqueue. If we win the
        // queue mutex, settle now; if the scheduler already dequeued it, its
        // launch precheck observes the same token and settles instead.
        if cancel_flag.load(Ordering::SeqCst) && queue::remove_for_run(&run_id).is_some() {
            settle_unlaunched_run(
                &session_db,
                &cancel_registry,
                &run_id,
                "Sub-agent launch cancelled before queue attachment",
            )
            .await?;
            anyhow::bail!("Sub-agent launch was revoked before queueing");
        }
        return Ok(run_id);
    }

    let launched = launch_subagent_run(
        params,
        run_id.clone(),
        child_session_id,
        effective_group_id,
        eval_child_guard,
        reattachable_ui_guard,
        initial_status,
        team_fence,
        0,
        session_db,
        cancel_registry,
    )
    .await?;
    if !launched {
        anyhow::bail!("Sub-agent launch was revoked before execution");
    }
    Ok(run_id)
}

/// Settle a prepared attempt that could not attach. No hook, mailbox consumer,
/// queue entry, or executor is allowed to survive this path.
pub(crate) async fn discard_prepared_subagent(
    prepared: PreparedSubagentSpawn,
    reason: &str,
) -> Result<()> {
    settle_unlaunched_run(
        &prepared.session_db,
        &prepared.cancel_registry,
        &prepared.run_id,
        reason,
    )
    .await
}

async fn settle_unlaunched_run(
    session_db: &Arc<SessionDB>,
    cancel_registry: &Arc<SubagentCancelRegistry>,
    run_id: &str,
    reason: &str,
) -> Result<()> {
    let _ = queue::remove_for_run(run_id);
    let session_paused = matches!(
        cancel_registry.reason(run_id),
        Some(super::cancel::SubagentCancelReason::SessionPaused)
    );
    if !session_paused {
        cancel_registry.cancel(run_id);
    }
    SUBAGENT_MAILBOX.remove(run_id);
    let db = session_db.clone();
    let run_id_owned = run_id.to_string();
    let reason = reason.to_string();
    let (status, terminal_reason) = if session_paused {
        (
            SubagentStatus::Interrupted,
            SubagentTerminalReason::SessionPaused,
        )
    } else {
        (
            SubagentStatus::Killed,
            SubagentTerminalReason::ParentCancelled,
        )
    };
    db.run(move |db| {
        db.update_subagent_status_with_reason(
            &run_id_owned,
            status,
            Some(terminal_reason),
            None,
            Some(&reason),
            None,
            Some(0),
        )
    })
    .await?;
    cancel_registry.remove(run_id);
    Ok(())
}

/// Claim and launch a prepared sub-agent run. The guarded status transition is
/// the execution linearization point: no mailbox, event, hook, or task exists
/// before it succeeds. Team-owned attempts additionally require their exact
/// attached roster capability to remain live in the same SQL statement.
pub(crate) async fn launch_subagent_run(
    params: SpawnParams,
    run_id: String,
    child_session_id: String,
    _effective_group_id: Option<String>,
    eval_child_guard: Option<crate::eval_context::EvalSessionGuard>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    expected_status: SubagentStatus,
    team_fence: Option<TeamMemberLaunchFence>,
    queue_wait_ms: u64,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) -> Result<bool> {
    // The cancel token was registered during prepare, before a Team attempt can
    // attach. Observe it before claiming execution, and again immediately after
    // the claim, so a pause/dissolve in either window cannot reach hooks/LLM.
    let cancel_flag = cancel_registry.register(&run_id);
    if cancel_flag.load(Ordering::SeqCst) {
        settle_unlaunched_run(
            &session_db,
            &cancel_registry,
            &run_id,
            "Sub-agent launch cancelled before execution claim",
        )
        .await?;
        return Ok(false);
    }
    let claim_result = {
        let db = session_db.clone();
        let claim_run_id = run_id.clone();
        let claim_expected = expected_status.clone();
        let claim_child_session_id = child_session_id.clone();
        let claim_team_fence = team_fence.clone();
        db.run(move |db| match claim_team_fence {
            Some(fence) => db.claim_team_member_attempt_launch(
                &fence.team_id,
                &fence.member_id,
                &claim_run_id,
                &claim_child_session_id,
                &claim_expected,
            ),
            None => db.try_transition_subagent_status(
                &claim_run_id,
                claim_expected,
                SubagentStatus::Running,
            ),
        })
        .await
    };
    let claimed = match claim_result {
        Ok(claimed) => claimed,
        Err(claim_error) => {
            // The run row (and, for Team, its roster attachment) already exists,
            // but no executor has been created yet. A transient SQLite/I/O error
            // here must not leave a durable Spawning run holding a cancel token
            // and a concurrency slot. Preserve the claim error for the caller;
            // Team restoration happens one level up after this local settlement.
            if let Err(settle_error) = settle_unlaunched_run(
                &session_db,
                &cancel_registry,
                &run_id,
                "Sub-agent execution claim failed before launch",
            )
            .await
            {
                crate::app_warn!(
                    "subagent",
                    "launch_claim_cleanup",
                    "failed to settle unlaunched run {} after execution claim error: claim_error={} settle_error={}",
                    run_id,
                    crate::logging::redact_sensitive(&claim_error.to_string()),
                    crate::logging::redact_sensitive(&settle_error.to_string())
                );
            }
            return Err(claim_error);
        }
    };
    if !claimed {
        settle_unlaunched_run(
            &session_db,
            &cancel_registry,
            &run_id,
            "Sub-agent launch capability was revoked before execution",
        )
        .await?;
        return Ok(false);
    }
    if cancel_flag.load(Ordering::SeqCst) {
        settle_unlaunched_run(
            &session_db,
            &cancel_registry,
            &run_id,
            "Sub-agent launch cancelled after execution claim",
        )
        .await?;
        return Ok(false);
    }

    let task_preview = truncate_str(&params.task, 50);
    crate::eval_context::record_queue_wait(
        Some(&child_session_id),
        "subagent",
        &run_id,
        queue_wait_ms,
    );
    // 6. Register the steer mailbox only after execution is claimed.
    SUBAGENT_MAILBOX.register(&run_id);
    let accepted_dispatches = {
        let dispatch_db = session_db.clone();
        let dispatch_run_id = run_id.clone();
        dispatch_db
            .run(move |db| db.list_accepted_subagent_dispatches(&dispatch_run_id))
            .await
    };
    match accepted_dispatches {
        Ok(dispatches) => {
            for (dispatch_id, message) in dispatches {
                if !SUBAGENT_MAILBOX.push_dispatch(&run_id, dispatch_id.clone(), message) {
                    crate::app_warn!(
                        "subagent",
                        "dispatch",
                        "failed to restore accepted steer dispatch {} into run {} mailbox",
                        dispatch_id,
                        run_id
                    );
                }
            }
        }
        Err(error) => crate::app_warn!(
            "subagent",
            "dispatch",
            "failed to restore accepted steer dispatches for run {}: {}",
            run_id,
            error
        ),
    }

    // 7. Emit spawned event
    emit_subagent_event(&SubagentEvent {
        event_type: "spawned".into(),
        run_id: run_id.clone(),
        parent_session_id: params.parent_session_id.clone(),
        child_agent_id: params.agent_id.clone(),
        child_session_id: child_session_id.clone(),
        task_preview: task_preview.clone(),
        status: SubagentStatus::Running,
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
    let run_instruction_context = params.run_instruction_context.clone();
    let run_data_context = params.run_data_context.clone();
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
        // A detached UI parent may finish before this background child asks for
        // approval. Keep its verified reattachment capability for the child's
        // complete queue/run lifecycle instead of tying it to the parent turn.
        let reattachable_ui_guard = reattachable_ui_guard;
        if let Some(fault) = crate::eval_context::scheduler_fault_action(
            Some(&child_session_id_clone),
            &run_id_clone,
        ) {
            if fault.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(fault.delay_ms)).await;
            }
        }
        let start = std::time::Instant::now();

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
        let run_instruction_context_exec = run_instruction_context.clone();
        let run_data_context_exec = run_data_context.clone();
        let skill_allowed_tools_exec = skill_allowed_tools.clone();
        let reasoning_effort_exec = reasoning_effort.clone();
        let child_session_id_exec = child_session_id_clone.clone();

        // A cancellation can race the final claim and task scheduling. Keep the
        // child conversation pristine when it wins, then let execute_subagent's
        // first instruction return through the normal Killed finalizer.
        if !cancel_flag.load(Ordering::SeqCst) {
            let message_db = db.clone();
            let message_session_id = child_session_id_exec.clone();
            let message_task = task.clone();
            let _ = message_db
                .run(move |db| {
                    db.append_message(
                        &message_session_id,
                        &crate::session::NewMessage::user(&message_task)
                            .with_source(crate::chat_engine::ChatSource::Subagent),
                    )
                })
                .await;
        }

        enum ExecutionResult {
            Finished(
                std::result::Result<
                    (String, Option<String>, crate::chat_engine::CapturedUsage),
                    SubagentExecutionFailure,
                >,
            ),
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
            parent_agent_id.clone(),
            plan_agent_mode_exec,
            plan_mode_allow_paths_exec,
            lock_plan_agent_mode_exec,
            run_instruction_context_exec,
            run_data_context_exec,
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

        // Determine outcome — handles Ok, Err, Timeout, Cancel, and Panic
        let raw_outcome = match result {
            Ok(ExecutionResult::Finished(Ok((response, model, usage)))) => {
                let truncated = truncate_str(&response, MAX_RESULT_CHARS);
                (
                    SubagentStatus::Completed,
                    crate::subagent::SubagentTerminalReason::Success,
                    Some(truncated),
                    None,
                    model,
                    usage,
                )
            }
            Ok(ExecutionResult::Finished(Err(e))) => (
                SubagentStatus::Error,
                e.terminal_reason,
                None,
                Some(e.to_string()),
                None,
                Default::default(),
            ),
            Ok(ExecutionResult::Timeout) => {
                // Timeout
                (
                    SubagentStatus::Timeout,
                    crate::subagent::SubagentTerminalReason::DeadlineExceeded,
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
                    crate::subagent::SubagentTerminalReason::RunnerPanic,
                    None,
                    Some("Sub-agent panicked unexpectedly".into()),
                    None,
                    Default::default(),
                )
            }
        };
        let (status, terminal_reason, result_text, error_text, model_used, usage) =
            if cancel_flag.load(Ordering::SeqCst) {
                let (_, _, result_text, _, model_used, usage) = raw_outcome;
                // Read the reason only after observing the flag. Requests
                // publish the reason before the flag while holding the
                // registry lock, so this cannot classify from a stale None.
                match registry.reason(&run_id_clone) {
                    Some(super::cancel::SubagentCancelReason::SessionPaused) => (
                        SubagentStatus::Interrupted,
                        crate::subagent::SubagentTerminalReason::SessionPaused,
                        result_text,
                        Some(
                            "Paused by session Stop; inspect any preserved result before continuing"
                                .into(),
                        ),
                        model_used,
                        usage,
                    ),
                    _ => (
                        SubagentStatus::Killed,
                        crate::subagent::SubagentTerminalReason::UserKilled,
                        None,
                        Some("Killed by parent".into()),
                        None,
                        Default::default(),
                    ),
                }
            } else {
                raw_outcome
            };
        let input_tokens = usage_tokens(usage.input_tokens);
        let output_tokens = usage_tokens(usage.output_tokens);

        if !matches!(status, SubagentStatus::Completed) {
            let reply_text = error_text
                .as_deref()
                .or(result_text.as_deref())
                .unwrap_or("(no response)");
            let message_db = db.clone();
            let message_session_id = child_session_id.clone();
            let reply_text = reply_text.to_string();
            let _ = message_db
                .run(move |db| {
                    db.append_message(
                        &message_session_id,
                        &crate::session::NewMessage::error_event(&reply_text)
                            .with_source(crate::chat_engine::ChatSource::Subagent),
                    )
                })
                .await;
        }

        // Update DB — guaranteed to run even after panic
        {
            let finalize_db = db.clone();
            let finalize_run_id = run_id_clone.clone();
            let finalize_status = status.clone();
            let finalize_result = result_text.clone();
            let finalize_error = error_text.clone();
            let finalize_model = model_used.clone();
            let _ = finalize_db
                .run(move |db| {
                    db.update_subagent_status_with_reason(
                        &finalize_run_id,
                        finalize_status,
                        Some(terminal_reason),
                        finalize_result.as_deref(),
                        finalize_error.as_deref(),
                        finalize_model.as_deref(),
                        Some(duration_ms),
                    )?;
                    db.set_subagent_usage(&finalize_run_id, input_tokens, output_tokens)
                })
                .await;
        }

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
                // The hook gets the FULL final answer (`last_assistant_message`),
                // not the 200-char UI preview — a SubagentStop hook inspecting the
                // child's answer must not be truncated.
                result_text.as_deref(),
            );
        }

        let child_session_id_for_cleanup = child_session_id_clone.clone();
        emit_subagent_event(&SubagentEvent {
            event_type: status.as_str().to_string(),
            run_id: run_id_clone.clone(),
            parent_session_id: parent_session_id.clone(),
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

        // The DB row chooses parent/group/workflow/none delivery and provides
        // restart replay. This CAS is a no-op for non-parent or consumed runs.
        // Delivery claiming performs synchronous SQLite work. The dispatcher
        // moves the complete claim + injection lifecycle to one dedicated OS
        // thread rather than pinning a runtime worker.
        let delivery_run_id = run_id_clone.clone();
        let delivery_db = db.clone();
        let parent_delivery_ui_guard = reattachable_ui_guard
            .as_ref()
            .map(|_| crate::permission::register_reattachable_ui_session(&parent_session_id));
        dispatch_parent_result_delivery_with_ui_guard(
            &delivery_run_id,
            delivery_db,
            parent_delivery_ui_guard,
        );
    });
    Ok(true)
}

/// Claim and dispatch one durable ordinary-parent result delivery. Safe to call
/// both on the live completion path and during startup replay; the database CAS
/// admits exactly one in-flight injector.
pub(crate) fn dispatch_parent_result_delivery(run_id: &str, db: Arc<SessionDB>) {
    dispatch_parent_result_delivery_with_ui_guard(run_id, db, None);
}

fn dispatch_parent_result_delivery_with_ui_guard(
    run_id: &str,
    db: Arc<SessionDB>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
) {
    let run_id = run_id.to_string();
    std::thread::spawn(move || {
        dispatch_parent_result_delivery_blocking(&run_id, db, reattachable_ui_guard);
    });
}

fn dispatch_parent_result_delivery_blocking(
    run_id: &str,
    db: Arc<SessionDB>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
) -> bool {
    let run = match db.get_subagent_run(run_id) {
        Ok(Some(run)) => run,
        Ok(None) => return false,
        Err(error) => {
            crate::app_warn!(
                "subagent",
                "delivery",
                "failed to load run {} for parent delivery: {}",
                run_id,
                error
            );
            return false;
        }
    };
    if run.delivery_kind != crate::subagent::SubagentDeliveryKind::Parent
        || run.owner_kind != crate::subagent::SubagentOwnerKind::ParentSession
        || !run.status.is_terminal()
        || matches!(run.status, SubagentStatus::Killed)
    {
        return false;
    }
    if db
        .is_session_or_ancestor_autonomy_paused(&run.parent_session_id)
        .unwrap_or(true)
    {
        crate::app_info!(
            "subagent",
            "delivery",
            "Deferring parent delivery for run {} while session {} is paused",
            run_id,
            run.parent_session_id
        );
        return false;
    }
    // Incognito deliveries are intentionally process-local: they may notify
    // the still-open parent now, but must never create a durable row that a
    // later Primary could replay before close-and-burn cleanup runs.
    let incognito = matches!(
        db.get_session(&run.parent_session_id),
        Ok(Some(ref session)) if session.incognito
    );
    if !incognito {
        match db.claim_subagent_result_delivery(run_id) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(error) => {
                crate::app_warn!(
                    "subagent",
                    "delivery",
                    "failed to claim parent delivery for run {}: {}",
                    run_id,
                    error
                );
                return false;
            }
        }
    }

    let push_message = build_subagent_push_message(
        &run.thread_id,
        &run.run_id,
        &run.child_agent_id,
        &run.task,
        &run.status,
        run.duration_ms.unwrap_or(0),
        run.result.as_deref(),
        run.error.as_deref(),
        run.terminal_reason,
    );
    let arm_db = db.clone();
    let arm_run_id = run.run_id.clone();
    let delivery_db = db.clone();
    let delivery_run_id = run.run_id.clone();
    let release_db = db.clone();
    let release_run_id = run.run_id.clone();
    let release_callback_run_id = run.run_id.clone();
    let on_injected: Option<super::injection::OnInjected> = (!incognito).then(|| {
        super::injection::OnInjected::new(
            move || arm_db.arm_subagent_result_delivery_no_replay(&arm_run_id),
            move || delivery_db.mark_subagent_result_delivered(&delivery_run_id),
        )
        .with_primary_handoff()
        .with_release_unarmed(move || {
            release_db.release_subagent_result_delivery_claim(
                &release_callback_run_id,
                "Parent injection abandoned before no-replay arm; waiting for a later sweep",
            )
        })
    });
    let release_receipt = on_injected.clone();
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => {
            let outcome = runtime.block_on(inject_and_run_parent_with_ui_guard(
                run.parent_session_id,
                run.parent_agent_id,
                run.child_agent_id,
                run.run_id,
                push_message,
                db,
                on_injected,
                reattachable_ui_guard,
                None,
            ));
            if outcome == super::injection::InjectionOutcome::Abandoned {
                // Pre-provider failures must not strand the durable source
                // in `injecting`. The state predicate preserves an armed
                // no-replay fence if one exists.
                super::injection::release_unarmed_injection_source(
                    release_receipt.as_ref(),
                    &release_run_id,
                );
            }
        }
        Err(error) => {
            crate::app_error!(
                "subagent",
                "delivery",
                "failed to build runtime for result delivery: {}",
                error
            );
            super::injection::release_unarmed_injection_source(
                release_receipt.as_ref(),
                &release_run_id,
            );
        }
    }
    true
}

#[derive(Debug)]
struct SubagentExecutionFailure {
    terminal_reason: SubagentTerminalReason,
    message: String,
}

impl SubagentExecutionFailure {
    fn new(terminal_reason: SubagentTerminalReason, message: impl Into<String>) -> Self {
        Self {
            terminal_reason,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for SubagentExecutionFailure {
    fn from(error: anyhow::Error) -> Self {
        Self::new(SubagentTerminalReason::ModelError, error.to_string())
    }
}

impl std::fmt::Display for SubagentExecutionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn subagent_terminal_reason_for_chat_failure(
    kind: crate::chat_engine::ChatEngineFailureKind,
    reason: Option<crate::failover::FailoverReason>,
) -> SubagentTerminalReason {
    match (kind, reason) {
        (
            crate::chat_engine::ChatEngineFailureKind::Terminal,
            Some(crate::failover::FailoverReason::CurrentToolGroupOverflow),
        ) => SubagentTerminalReason::CurrentToolGroupOverflow,
        (
            crate::chat_engine::ChatEngineFailureKind::Terminal,
            Some(crate::failover::FailoverReason::DispatchUnknown),
        ) => SubagentTerminalReason::DispatchUnknown,
        (crate::chat_engine::ChatEngineFailureKind::ProviderExhausted, _) => {
            SubagentTerminalReason::ProviderExhausted
        }
        (crate::chat_engine::ChatEngineFailureKind::Cancelled, _) => {
            SubagentTerminalReason::ParentCancelled
        }
        (
            crate::chat_engine::ChatEngineFailureKind::Terminal
            | crate::chat_engine::ChatEngineFailureKind::Infrastructure,
            _,
        ) => SubagentTerminalReason::ModelError,
    }
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
    parent_agent_id: String,
    plan_agent_mode: Option<crate::agent::PlanAgentMode>,
    plan_mode_allow_paths: Vec<String>,
    lock_plan_agent_mode: bool,
    run_instruction_context_override: Option<String>,
    run_data_context: Option<String>,
    skill_allowed_tools: Vec<String>,
    reasoning_effort: Option<String>,
    origin_source: Option<crate::knowledge::KbAccessSource>,
    origin_channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
) -> impl std::future::Future<
    Output = std::result::Result<
        (String, Option<String>, crate::chat_engine::CapturedUsage),
        SubagentExecutionFailure,
    >,
> + Send {
    async move {
        use crate::provider;

        // This must precede config resolution, title scheduling, provider
        // selection, and chat-engine construction. It is the last task-local
        // fence for a pause/dissolve racing tokio scheduling.
        if cancel.load(Ordering::SeqCst) {
            return Err(SubagentExecutionFailure::new(
                SubagentTerminalReason::ParentCancelled,
                "Sub-agent launch was cancelled before execution",
            ));
        }

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
            return Err(anyhow::anyhow!("No model configured for sub-agent execution").into());
        }

        // Build the trusted, platform-owned sub-agent run frame.
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
         - {}\n\
         - Complete the task directly and concisely. Your full response will be returned to the parent agent.\n\
         - You do NOT have access to the parent's conversation history.\n\
         - This is an isolated session.",
        depth_info
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

        let run_instruction_context = if let Some(ctx) = run_instruction_context_override {
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
        // The fixed child frame and any caller-owned platform contract travel
        // through the typed run-instruction lane. The task remains solely in
        // the user message; Plan document data has its own data lane.
        let plan_context_override = if lock_plan_agent_mode {
            plan_agent_mode.map(|mode| crate::chat_engine::PlanResolvedContext {
                // Spawn-supplied PlanAgent always means "child should run
                // as if it were in Planning" — the locked flag freezes
                // this against the mid-turn probe regardless.
                state: crate::plan::PlanModeState::Planning,
                mode,
                allow_paths: plan_mode_allow_paths,
                run_instruction: None,
                plan_data: None,
            })
        } else {
            None
        };

        let run_context = match (run_instruction_context, run_data_context) {
            (Some(instruction), data) => Some(
                crate::prompt_context::RunInstructionContext::new(
                    crate::prompt_context::RunInstructionSource::Subagent,
                    instruction,
                )
                .map(|context| match data {
                    Some(data) => context.with_untrusted_data(data),
                    None => context,
                }),
            ),
            (None, Some(data)) => Some(crate::prompt_context::RunInstructionContext::data_only(
                crate::prompt_context::RunInstructionSource::Subagent,
                data,
            )),
            (None, None) => None,
        }
        .transpose()
        .map_err(|error| {
            SubagentExecutionFailure::new(SubagentTerminalReason::ModelError, error.to_string())
        })?;

        let parent_subagent_config = crate::agent_loader::load_agent(&parent_agent_id)
            .map(|definition| definition.config.subagents)
            .unwrap_or_default();
        let max_provider_retries = parent_subagent_config.provider_retry_attempts.min(10);
        let retry_base_secs = parent_subagent_config
            .provider_retry_backoff_secs
            .clamp(1, 60);
        let mut retry_attempt = 0u32;
        let mut attempt_message = task.clone();
        let mut attempt_attachments = attachments;

        let result = loop {
            let attempt = crate::chat_engine::run_chat_engine_classified(
                crate::chat_engine::ChatEngineParams {
                    session_id: child_session_id.clone(),
                    agent_id: agent_id.clone(),
                    turn_id: None,
                    message: attempt_message,
                    incoming_turn: None,
                    display_text: None,
                    attachments: attempt_attachments,
                    session_db: session_db.clone(),
                    model_chain: model_chain.clone(),
                    providers: store.providers.clone(),
                    codex_token: None,
                    resolved_temperature: agent_def.config.model.temperature.or(store.temperature),
                    compact_config: store.compact.clone(),
                    run_context: run_context.clone(),
                    reasoning_effort: effective_reasoning_effort.clone(),
                    cancel: cancel.clone(),
                    foreground_stop_admission: None,
                    plan_context_override: plan_context_override.clone(),
                    skill_allowed_tools: skill_allowed_tools.clone(),
                    denied_tools: denied.clone(),
                    tool_scope: None,
                    subagent_depth: depth,
                    steer_run_id: Some(run_id.clone()),
                    auto_approve_tools: false,
                    follow_global_reasoning_effort: false,
                    post_turn_effects: false,
                    abort_on_cancel: true,
                    persist_final_error_event: false,
                    source: crate::chat_engine::stream_seq::ChatSource::Subagent,
                    ui_surface: None,
                    origin_source,
                    channel_kb_context: origin_channel_kb_context.clone(),
                    event_sink: Arc::new(crate::chat_engine::NoopEventSink),
                },
            )
            .await;

            match attempt {
                Ok(result) => break Ok(result),
                Err(error)
                    if error.kind()
                        == crate::chat_engine::ChatEngineFailureKind::ProviderExhausted
                        && retry_attempt < max_provider_retries
                        && !cancel.load(Ordering::SeqCst) =>
                {
                    retry_attempt += 1;
                    let exponent = retry_attempt.saturating_sub(1).min(6);
                    let delay_secs = retry_base_secs.saturating_mul(1u64 << exponent).min(300);
                    let next_attempt_at =
                        chrono::Utc::now() + chrono::Duration::seconds(delay_secs as i64);
                    let error_message = format!("Sub-agent provider chain exhausted: {error}");
                    let recovery_db = session_db.clone();
                    let recovery_run_id = run_id.clone();
                    let recovery_next = next_attempt_at.to_rfc3339();
                    let recovery_error = error_message.clone();
                    if let Err(record_error) = recovery_db
                        .run(move |db| {
                            db.record_subagent_provider_recovery(
                                &recovery_run_id,
                                retry_attempt,
                                max_provider_retries,
                                &recovery_next,
                                &recovery_error,
                            )
                        })
                        .await
                    {
                        crate::app_warn!(
                            "subagent",
                            "provider_retry",
                            "Failed to persist provider retry state for run {}: {}",
                            run_id,
                            record_error
                        );
                    }
                    emit_subagent_event(&SubagentEvent {
                        event_type: "retrying".into(),
                        run_id: run_id.clone(),
                        parent_session_id: parent_session_id.clone(),
                        child_agent_id: agent_id.clone(),
                        child_session_id: child_session_id.clone(),
                        task_preview: truncate_str(&task, 50),
                        status: SubagentStatus::Running,
                        result_preview: None,
                        error: Some(format!(
                            "Provider gateway unavailable; retry {}/{} in {}s",
                            retry_attempt, max_provider_retries, delay_secs
                        )),
                        duration_ms: None,
                        label: None,
                        input_tokens: None,
                        output_tokens: None,
                        result_full: None,
                        skill_name: None,
                    });
                    crate::app_warn!(
                        "subagent",
                        "provider_retry",
                        "Provider chain exhausted for run {}; retry {}/{} in {}s",
                        run_id,
                        retry_attempt,
                        max_provider_retries,
                        delay_secs
                    );

                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(delay_secs);
                    while tokio::time::Instant::now() < deadline {
                        if cancel.load(Ordering::SeqCst) {
                            break;
                        }
                        let remaining =
                            deadline.saturating_duration_since(tokio::time::Instant::now());
                        tokio::time::sleep(remaining.min(std::time::Duration::from_millis(250)))
                            .await;
                    }
                    if cancel.load(Ordering::SeqCst) {
                        break Err(crate::chat_engine::ChatEngineFailure::cancelled(
                            "Sub-agent provider recovery cancelled",
                        ));
                    }
                    attempt_message = format!(
                        "<provider-recovery>\nThe previous sub-agent turn exhausted every configured provider/profile/model fallback. This is recovery attempt {}/{}. Continue from the durable child-session history. Inspect prior tool results before acting, do not repeat completed side effects, and finish the original task.\n</provider-recovery>",
                        retry_attempt, max_provider_retries
                    );
                    attempt_attachments = Vec::new();
                }
                other => break other,
            }
        };

        let clear_db = session_db.clone();
        let clear_run_id = run_id.clone();
        let _ = clear_db
            .run(move |db| db.clear_subagent_provider_recovery(&clear_run_id))
            .await;

        let result = result.map_err(|error| {
            let message = format!("Sub-agent chat execution failed: {error}");
            let terminal_reason =
                subagent_terminal_reason_for_chat_failure(error.kind(), error.reason());
            SubagentExecutionFailure::new(terminal_reason, message)
        })?;

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

    #[test]
    fn execution_failure_keeps_provider_exhaustion_distinct_from_setup_errors() {
        let provider = SubagentExecutionFailure::new(
            subagent_terminal_reason_for_chat_failure(
                crate::chat_engine::ChatEngineFailureKind::ProviderExhausted,
                Some(crate::failover::FailoverReason::Timeout),
            ),
            "providers unavailable",
        );
        assert_eq!(
            provider.terminal_reason,
            SubagentTerminalReason::ProviderExhausted
        );

        let setup = SubagentExecutionFailure::from(anyhow::anyhow!("agent config invalid"));
        assert_eq!(setup.terminal_reason, SubagentTerminalReason::ModelError);
    }

    #[test]
    fn current_tool_group_terminal_never_becomes_provider_recovery() {
        let terminal_reason = subagent_terminal_reason_for_chat_failure(
            crate::chat_engine::ChatEngineFailureKind::Terminal,
            Some(crate::failover::FailoverReason::CurrentToolGroupOverflow),
        );
        assert_eq!(
            terminal_reason,
            SubagentTerminalReason::CurrentToolGroupOverflow
        );
        assert_ne!(terminal_reason, SubagentTerminalReason::ProviderExhausted);
        assert!(!terminal_reason.resume_recommended());
    }

    #[test]
    fn dispatch_unknown_never_becomes_provider_recovery() {
        let terminal_reason = subagent_terminal_reason_for_chat_failure(
            crate::chat_engine::ChatEngineFailureKind::Terminal,
            Some(crate::failover::FailoverReason::DispatchUnknown),
        );
        assert_eq!(terminal_reason, SubagentTerminalReason::DispatchUnknown);
        assert_ne!(terminal_reason, SubagentTerminalReason::ProviderExhausted);
        assert!(!terminal_reason.resume_recommended());
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
            run_instruction_context: None,
            run_data_context: None,
            skill_allowed_tools: Vec::new(),
            reasoning_effort: None,
            skill_name: None,
            origin_source: None,
            origin_channel_kb_context: None,
            group_id: None,
            owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
            owner_id: "s".into(),
            delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
        }
    }

    #[tokio::test]
    async fn subagent_depth_overflow_rejects_not_queues() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(SessionDB::open_ephemeral_for_test(&tmp.path().join("s.db")).unwrap());
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
            thread_id: format!("child-{run_id}"),
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
            continuation_of_run_id: None,
            trigger_kind: "spawn".into(),
            terminal_reason: None,
            runner_owner: None,
            lease_epoch: 1,
            last_heartbeat_at: None,
            delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
            launch_spec_json: None,
            owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
            owner_id: parent_session.into(),
        }
    }

    #[tokio::test]
    async fn unlaunched_settlement_preserves_session_pause_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(SessionDB::open_ephemeral_for_test(&tmp.path().join("s.db")).unwrap());
        let registry = Arc::new(SubagentCancelRegistry::new());
        let parent = db.create_session("ha-main").unwrap();
        let run_id = "pause-during-launch";
        db.insert_subagent_run(&active_run(run_id, &parent.id, "ha-main"))
            .unwrap();

        registry.register(run_id);
        assert!(registry.pause(run_id));
        settle_unlaunched_run(
            &db,
            &registry,
            run_id,
            "Sub-agent launch cancelled before execution claim",
        )
        .await
        .unwrap();

        let settled = db.get_subagent_run(run_id).unwrap().unwrap();
        assert_eq!(settled.status, SubagentStatus::Interrupted);
        assert_eq!(
            settled.terminal_reason,
            Some(SubagentTerminalReason::SessionPaused)
        );
        assert!(!registry.pause(run_id), "settled token must be removed");
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

            let db =
                Arc::new(SessionDB::open_ephemeral_for_test(&root.path().join("s.db")).unwrap());
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
                run_instruction_context: None,
                run_data_context: None,
                skill_allowed_tools: Vec::new(),
                reasoning_effort: None,
                skill_name: None,
                origin_source: None,
                origin_channel_kb_context: None,
                group_id: None,
                owner_kind: crate::subagent::SubagentOwnerKind::ParentSession,
                owner_id: parent.id.clone(),
                delivery_kind: crate::subagent::SubagentDeliveryKind::Parent,
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

    #[test]
    fn cancellation_between_prepare_and_enqueue_settles_without_a_free_slot() {
        // Deterministic version of the Team pause-before-enqueue race: the
        // parent is already at its concurrency cap, prepare registers the token
        // but deliberately does not enqueue, then cancellation wins. The
        // enqueue postcheck must claim and terminalize the entry immediately;
        // waiting for a slot would leave resume blocked on a phantom Queued run.
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let agent_id = "test-prepare-enqueue-cancel-agent";
            let dir = crate::paths::agent_dir(agent_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let mut cfg = crate::agent_config::AgentConfig::default();
            cfg.subagents.max_concurrent = 1;
            std::fs::write(dir.join("agent.json"), serde_json::to_string(&cfg).unwrap()).unwrap();

            let db =
                Arc::new(SessionDB::open_ephemeral_for_test(&root.path().join("s.db")).unwrap());
            let registry = Arc::new(SubagentCancelRegistry::new());
            let parent = db.create_session(agent_id).unwrap();
            db.insert_subagent_run(&active_run("active-blocker", &parent.id, agent_id))
                .unwrap();
            let params = SpawnParams {
                task: "must never execute".into(),
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
                skip_parent_injection: true,
                run_instruction_context: None,
                run_data_context: None,
                skill_allowed_tools: Vec::new(),
                reasoning_effort: None,
                skill_name: None,
                origin_source: None,
                origin_channel_kb_context: None,
                group_id: None,
                owner_kind: crate::subagent::SubagentOwnerKind::Internal,
                owner_id: "prepare-enqueue-race".into(),
                delivery_kind: crate::subagent::SubagentDeliveryKind::None,
            };
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let prepared = rt
                .block_on(prepare_subagent(params, db.clone(), registry.clone()))
                .expect("prepare queued run");
            let run_id = prepared.run_id().to_string();
            assert_eq!(
                db.get_subagent_run(&run_id).unwrap().unwrap().status,
                SubagentStatus::Queued
            );
            assert!(registry.cancel(&run_id), "cancel trips prepared token");

            let error = rt
                .block_on(launch_prepared_subagent(prepared, None))
                .expect_err("cancelled prepared run must not remain queued");
            assert!(error.to_string().contains("revoked"));
            let settled = db.get_subagent_run(&run_id).unwrap().unwrap();
            assert_eq!(settled.status, SubagentStatus::Killed);
            assert_eq!(
                settled.terminal_reason,
                Some(SubagentTerminalReason::ParentCancelled)
            );
            assert!(super::queue::remove_for_run(&run_id).is_none());
            assert!(!registry.cancel(&run_id), "cancel flag must be cleaned");
            assert!(
                !SUBAGENT_MAILBOX.push(&run_id, "must not have a mailbox".into()),
                "no mailbox is registered before a successful launch claim"
            );
        });
    }

    #[test]
    fn launch_claim_error_settles_persisted_run_and_cancel_token() {
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let agent_id = "test-launch-claim-error-agent";
            let dir = crate::paths::agent_dir(agent_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("agent.json"),
                serde_json::to_string(&crate::agent_config::AgentConfig::default()).unwrap(),
            )
            .unwrap();

            let db =
                Arc::new(SessionDB::open_ephemeral_for_test(&root.path().join("s.db")).unwrap());
            let registry = Arc::new(SubagentCancelRegistry::new());
            let parent = db.create_session(agent_id).unwrap();
            let mut params = params_at_depth(1);
            params.agent_id = agent_id.into();
            params.parent_agent_id = agent_id.into();
            params.parent_session_id = parent.id.clone();
            params.owner_kind = crate::subagent::SubagentOwnerKind::Internal;
            params.owner_id = "launch-claim-error".into();
            params.delivery_kind = crate::subagent::SubagentDeliveryKind::None;
            params.skip_parent_injection = true;

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let prepared = rt
                .block_on(prepare_subagent(params, db.clone(), registry.clone()))
                .expect("prepare fresh run");
            let run_id = prepared.run_id().to_string();
            assert_eq!(
                db.get_subagent_run(&run_id).unwrap().unwrap().status,
                SubagentStatus::Spawning
            );
            assert_eq!(db.count_active_subagent_runs(&parent.id).unwrap(), 1);

            db.with_conn_for_test(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER fail_subagent_running_claim
                     BEFORE UPDATE OF status ON subagent_runs
                     WHEN NEW.status = 'running'
                     BEGIN
                       SELECT RAISE(ABORT, 'injected launch claim failure');
                     END;",
                )?;
                Ok(())
            })
            .unwrap();

            let error = rt
                .block_on(launch_prepared_subagent(prepared, None))
                .expect_err("claim error must propagate after local settlement");
            assert!(
                error.to_string().contains("injected launch claim failure"),
                "original claim error must be preserved: {error}"
            );
            assert_eq!(
                db.get_subagent_run(&run_id).unwrap().unwrap().status,
                SubagentStatus::Killed
            );
            assert_eq!(db.count_active_subagent_runs(&parent.id).unwrap(), 0);
            assert!(super::queue::remove_for_run(&run_id).is_none());
            assert!(!registry.cancel(&run_id), "cancel flag must be cleaned");
            assert!(
                !SUBAGENT_MAILBOX.push(&run_id, "must not have a mailbox".into()),
                "mailbox must not exist before a successful launch claim"
            );
        });
    }
}

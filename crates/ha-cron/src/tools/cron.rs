use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use ha_core::cron::{
    self, CronDeliveryTarget, CronPayload, CronSchedule, CronWorkspaceCleanup, CronWorkspaceMode,
    CronWorkspacePolicy, NewCronJob,
};

use crate::cron::{
    evaluate_cron_preflight, start_cron_run_now, CronPreflightRequest, CronRunNowResult,
};

/// Tool: manage_cron — create, list, get, update, delete, and trigger scheduled tasks,
/// and discover IM channel delivery targets.
///
/// Returns `Pin<Box<dyn Future + Send>>` instead of an opaque `async fn` future
/// to break the type-level recursion: tool_manage_cron → execute_job → agent.chat
/// → execute_tool_with_context → tool_manage_cron. Without the boxing, the compiler
/// cannot compute the infinite recursive future type to verify `Send`.
pub(crate) fn tool_manage_cron<'a>(
    args: &'a Value,
    ctx: &'a ha_core::tools::ToolExecContext,
) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
    // Own the session_id so the returned future doesn't borrow from the caller;
    // `ctx` itself is only needed by the `delete` approval gate below.
    let session_id = ctx.session_id.clone();
    Box::pin(async move {
        let cron_db = ha_core::get_cron_db()
            .ok_or_else(|| anyhow::anyhow!("Cron service not initialized"))?;

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "create" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

                let prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;

                let schedule = parse_schedule(args)?;

                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let (delivery_targets, inferred) =
                    resolve_delivery_targets_for_create(args, session_id.as_deref())?;
                let conversation_target = parse_conversation_target(args)?;
                let (payload, project_id, workspace_policy) = match conversation_target {
                    CronConversationTarget::New => {
                        anyhow::ensure!(
                            args.get("target_session_id").is_none(),
                            "target_session_id is only valid with conversation_target=existing_session"
                        );
                        let agent_id = args
                            .get("agent_id")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        (
                            CronPayload::AgentTurn {
                                prompt: prompt.to_string(),
                                agent_id,
                            },
                            resolve_project_id_for_create(args, session_id.as_deref())?,
                            resolve_workspace_policy(args, None)?,
                        )
                    }
                    CronConversationTarget::Current => {
                        reject_existing_session_context_overrides(args)?;
                        anyhow::ensure!(
                            args.get("target_session_id").is_none(),
                            "target_session_id is only valid with conversation_target=existing_session"
                        );
                        let target_session_id = session_id.clone().ok_or_else(|| {
                            anyhow::anyhow!(
                                "conversation_target=current_session requires a current chat"
                            )
                        })?;
                        (
                            CronPayload::SessionTurn {
                                session_id: target_session_id,
                                prompt: prompt.to_string(),
                            },
                            None,
                            CronWorkspacePolicy::default(),
                        )
                    }
                    CronConversationTarget::Existing => {
                        reject_existing_session_context_overrides(args)?;
                        let target_session_id = args
                            .get("target_session_id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "conversation_target=existing_session requires target_session_id from sessions_list"
                                )
                            })?;
                        (
                            CronPayload::SessionTurn {
                                session_id: target_session_id.to_string(),
                                prompt: prompt.to_string(),
                            },
                            None,
                            CronWorkspacePolicy::default(),
                        )
                    }
                };

                let job_timeout_secs = match resolve_cron_job_timeout_secs_arg(args, ctx).await {
                    CronTimeoutArg::Set(value) => value,
                    CronTimeoutArg::Absent | CronTimeoutArg::Ignored => None,
                };

                let input = NewCronJob {
                    name: name.to_string(),
                    description,
                    project_id,
                    schedule,
                    payload,
                    max_failures: args
                        .get("max_failures")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    notify_on_complete: args.get("notify_on_complete").and_then(|v| v.as_bool()),
                    delivery_targets: Some(delivery_targets),
                    prefix_delivery_with_name: args
                        .get("prefix_delivery_with_name")
                        .and_then(|v| v.as_bool()),
                    job_timeout_secs,
                    // Permission / sandbox overrides are owner-plane only (set via
                    // the GUI cron form / Tauri / HTTP). The model-facing tool must
                    // NOT set them — otherwise it could schedule a `yolo` task to
                    // self-escalate unattended, or lower its own sandbox. Always
                    // None here = follow the agent default.
                    permission_mode_override: None,
                    sandbox_mode_override: None,
                    workspace_policy,
                };

                require_cron_preflight(
                    cron_db.clone(),
                    CronPreflightRequest::Create { job: input.clone() },
                )
                .await?;
                let db = cron_db.clone();
                let job = ha_core::blocking::run_blocking(move || db.add_job(&input)).await?;
                ctx.emit_metadata(json!({
                    "kind": "schedule_entity",
                    "entityType": "cronTask",
                    "entityId": job.id.clone(),
                    "sessionId": session_id.clone(),
                    "title": job.name.clone(),
                    "schedule": job.schedule.clone(),
                    "state": job.status.as_str(),
                    "nextRunAt": job.next_run_at.clone(),
                    "projectId": job.project_id.clone(),
                    "workspaceMode": job.workspace_policy.mode,
                    "workspaceBaseRef": job.workspace_policy.base_ref.clone(),
                }))
                .await;
                let mut out = format!(
                    "Created scheduled task '{}' (id: {}). Next run: {}",
                    job.name,
                    job.id,
                    job.next_run_at.as_deref().unwrap_or("none")
                );
                if !job.delivery_targets.is_empty() {
                    out.push_str(&format!(
                        "\nDelivery targets: {}",
                        format_targets_inline(&job.delivery_targets)
                    ));
                    if inferred {
                        out.push_str(
                            " (inferred from the current IM channel conversation — \
                             pass delivery_targets=[] to opt out)",
                        );
                    }
                }
                if let Some(project_id) = job.project_id.as_deref() {
                    out.push_str(&format!("\nProject: {}", project_label(project_id)));
                }
                out.push_str(&format!(
                    "\nWorkspace: {}{}{}",
                    workspace_mode_label(job.workspace_policy.mode),
                    job.workspace_policy
                        .base_ref
                        .as_deref()
                        .map(|base_ref| format!(" (base_ref={base_ref})"))
                        .unwrap_or_default(),
                    if job.workspace_policy.mode == CronWorkspaceMode::Fresh {
                        format!(
                            " (cleanup={})",
                            workspace_cleanup_label(job.workspace_policy.cleanup)
                        )
                    } else {
                        String::new()
                    }
                ));
                if let CronPayload::SessionTurn {
                    session_id: target_session_id,
                    ..
                } = &job.payload
                {
                    let target = if session_id.as_deref() == Some(target_session_id.as_str()) {
                        "current session".to_string()
                    } else {
                        format!("existing session {target_session_id}")
                    };
                    out.push_str(&format!(
                        "\nConversation target: {target} (shared history and live context)"
                    ));
                }
                Ok(out)
            }

            "update" => {
                if args.get("conversation_target").is_some()
                    || args.get("target_session_id").is_some()
                {
                    anyhow::bail!(
                        "conversation_target and target_session_id are create-only and cannot change an existing task's target"
                    );
                }
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let mut job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

                // A job carrying owner-set permission/sandbox overrides is
                // read-only to the model. These overrides can only be set on the
                // owner plane (GUI / Tauri / HTTP); letting the model edit such a
                // job (e.g. rewrite its prompt) would let an injected model
                // *repurpose* an owner-authorized yolo / unconfined task to run
                // arbitrary instructions unattended with that standing privilege.
                // The model never has a legitimate reason to edit an owner-
                // configured privileged task — direct it back to the user.
                if job.permission_mode_override.is_some() || job.sandbox_mode_override.is_some() {
                    anyhow::bail!(
                        "Scheduled task '{}' has owner-configured permission/sandbox settings and \
                         can only be edited by the user in the app — the assistant cannot modify it.",
                        id
                    );
                }

                if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                    job.name = name.to_string();
                }
                if let Some(desc) = args.get("description") {
                    job.description = desc.as_str().map(String::from);
                }
                if args.get("schedule_type").is_some() {
                    job.schedule = parse_schedule(args)?;
                }
                match job.payload {
                    CronPayload::AgentTurn {
                        ref mut prompt,
                        ref mut agent_id,
                    } => {
                        if let Some(p) = args.get("prompt").and_then(|v| v.as_str()) {
                            *prompt = p.to_string();
                        }
                        if let Some(v) = args.get("agent_id") {
                            *agent_id = v.as_str().map(String::from);
                        }
                    }
                    CronPayload::SessionTurn { ref mut prompt, .. } => {
                        reject_existing_session_context_overrides(args)?;
                        if let Some(p) = args.get("prompt").and_then(|v| v.as_str()) {
                            *prompt = p.to_string();
                        }
                    }
                    CronPayload::SessionLoop { .. } => {
                        anyhow::bail!(
                            "Scheduled task '{}' is owner-managed and can only be edited from its app control plane.",
                            id
                        );
                    }
                }
                if let Some(n) = args.get("max_failures").and_then(|v| v.as_u64()) {
                    job.max_failures = n as u32;
                }
                if let Some(b) = args.get("notify_on_complete").and_then(|v| v.as_bool()) {
                    job.notify_on_complete = b;
                }
                if let Some(b) = args
                    .get("prefix_delivery_with_name")
                    .and_then(|v| v.as_bool())
                {
                    job.prefix_delivery_with_name = b;
                }
                // C19 per-job timeout: a number sets the override, explicit null
                // clears it (back to the global default); absent leaves it as-is.
                match resolve_cron_job_timeout_secs_arg(args, ctx).await {
                    CronTimeoutArg::Set(value) => {
                        job.job_timeout_secs = value;
                    }
                    CronTimeoutArg::Absent | CronTimeoutArg::Ignored => {}
                }
                if let Some(v) = args.get("project_id") {
                    job.project_id = parse_project_id_value(v)?;
                    validate_project_id(job.project_id.as_deref())?;
                }
                if args.get("workspace_mode").is_some() || args.get("workspace_base_ref").is_some()
                {
                    job.workspace_policy =
                        resolve_workspace_policy(args, Some(&job.workspace_policy))?;
                }
                // delivery_targets tri-state on update (no inference — never silently
                // clobber what the user set in the GUI).
                if let Some(v) = args.get("delivery_targets") {
                    if !v.is_null() {
                        let parsed: Vec<CronDeliveryTarget> = serde_json::from_value(v.clone())
                            .map_err(|e| anyhow::anyhow!("Invalid 'delivery_targets': {}", e))?;
                        job.delivery_targets = parsed;
                    }
                }

                require_cron_preflight(
                    cron_db.clone(),
                    CronPreflightRequest::Update {
                        job: job.clone(),
                        expected_revision: job.revision,
                    },
                )
                .await?;
                let db = cron_db.clone();
                let draft = job.clone();
                ha_core::blocking::run_blocking(move || db.update_job(&draft)).await?;
                Ok(format!(
                    "Updated scheduled task '{}' (id: {}). Next run: {} | Project: {} | Workspace: {}{} | Targets: {}",
                    job.name,
                    job.id,
                    job.next_run_at.as_deref().unwrap_or("none"),
                    job.project_id
                        .as_deref()
                        .map(project_label)
                        .unwrap_or_else(|| "none".to_string()),
                    workspace_mode_label(job.workspace_policy.mode),
                    job.workspace_policy
                        .base_ref
                        .as_deref()
                        .map(|base_ref| format!(" ({base_ref})"))
                        .unwrap_or_default(),
                    if job.delivery_targets.is_empty() {
                        "none".to_string()
                    } else {
                        format_targets_inline(&job.delivery_targets)
                    }
                ))
            }

            "list" => {
                let jobs = cron_db.list_jobs()?;
                if jobs.is_empty() {
                    return Ok("No scheduled tasks.".to_string());
                }
                // One query for every task's last outcome: a failing task should
                // be visible in the list itself, not only after a follow-up call.
                let last_runs = cron_db.latest_run_summaries().unwrap_or_default();
                let mut lines = Vec::new();
                lines.push(format!("{} scheduled task(s):", jobs.len()));
                for job in &jobs {
                    let next = job.next_run_at.as_deref().unwrap_or("none");
                    let last = last_runs
                        .get(&job.id)
                        .map(|run| {
                            let detail = run
                                .error
                                .as_deref()
                                .map(|error| {
                                    format!(": {}", ha_core::truncate_utf8(error, LIST_ERROR_LIMIT))
                                })
                                .unwrap_or_default();
                            format!(
                                " | Last: {} at {}{}",
                                run.status,
                                run.finished_at.as_deref().unwrap_or(&run.started_at),
                                detail
                            )
                        })
                        .unwrap_or_default();
                    let targets = if job.delivery_targets.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " | Targets: {}",
                            format_targets_inline(&job.delivery_targets)
                        )
                    };
                    let project = job
                        .project_id
                        .as_deref()
                        .map(|pid| format!(" | Project: {}", project_label(pid)))
                        .unwrap_or_default();
                    let workspace = format!(
                        " | Workspace: {}{}",
                        workspace_mode_label(job.workspace_policy.mode),
                        job.workspace_policy
                            .base_ref
                            .as_deref()
                            .map(|base_ref| format!(" ({base_ref})"))
                            .unwrap_or_default()
                    );
                    lines.push(format!(
                        "  - [{}] {} ({}) | Next: {} | Status: {}{}{}{}{}",
                        ha_core::truncate_utf8(&job.id, 8),
                        job.name,
                        schedule_summary(&job.schedule),
                        next,
                        job.status.as_str(),
                        last,
                        project,
                        workspace,
                        targets,
                    ));
                }
                Ok(lines.join("\n"))
            }

            "get" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                match cron_db.get_job(id)? {
                    // Carry the last occurrence's outcome so "why didn't it work"
                    // is answerable without a second call.
                    Some(job) => {
                        let last_run = cron_db
                            .latest_run_summaries()
                            .ok()
                            .and_then(|mut summaries| summaries.remove(&job.id));
                        Ok(serde_json::to_string_pretty(&json!({
                            "job": job,
                            "lastRun": last_run,
                        }))?)
                    }
                    None => Ok(format!("Job '{}' not found.", id)),
                }
            }

            "runs" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(RUNS_DEFAULT_LIMIT)
                    .clamp(1, RUNS_MAX_LIMIT) as usize;
                let session_db = cron_session_db().await?;
                let db = cron_db.clone();
                let job_id = id.to_string();
                let logs = ha_core::blocking::run_blocking(move || {
                    crate::cron::visible_cron_run_logs(&db, &session_db, &job_id, limit, 0)
                })
                .await?;
                if logs.is_empty() {
                    return Ok(format!("No recorded runs for scheduled task '{}'.", id));
                }
                let mut lines = vec![format!("Runs for '{}' (newest first):", id)];
                for log in &logs {
                    let mut line = format!(
                        "  - [{}] {} | started: {}",
                        log.id, log.status, log.started_at
                    );
                    if let Some(duration) = log.duration_ms {
                        line.push_str(&format!(" | {}ms", duration));
                    }
                    if let Some(delivery) = log.delivery_status.as_deref() {
                        line.push_str(&format!(" | delivery: {}", delivery));
                    }
                    if let Some(error) = log.error.as_deref() {
                        line.push_str(&format!(
                            "\n      error: {}",
                            ha_core::truncate_utf8(error, RUN_TEXT_LIMIT)
                        ));
                    } else if let Some(preview) = log.result_preview.as_deref() {
                        line.push_str(&format!(
                            "\n      result: {}",
                            ha_core::truncate_utf8(preview, RUN_TEXT_LIMIT)
                        ));
                    }
                    lines.push(line);
                }
                Ok(lines.join("\n"))
            }

            "cancel_run" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;
                reject_loop_managed_job(&job, id, "cancelled")?;
                // An explicit run log id wins: the task's live claim can advance
                // between two calls, and a stale "cancel whatever is running"
                // must never terminate the next occurrence instead.
                let run_log_id = match args.get("run_log_id").and_then(Value::as_i64) {
                    Some(explicit) => explicit,
                    None => match cron_db.find_open_run_log_for_job(id)? {
                        Some(found) => found,
                        None => {
                            return Ok(format!(
                                "Scheduled task '{}' has no run in flight. Cancelling is only for a live occurrence; use action='pause' to stop future runs.",
                                job.name
                            ))
                        }
                    },
                };
                match crate::cron::cancel_run(run_log_id).await? {
                    Some(result) if result.cancel_requested => Ok(format!(
                        "Requested cancellation of run {} for '{}' (status: {}).",
                        result.run_log_id, job.name, result.status
                    )),
                    Some(result) => Ok(format!(
                        "Run {} for '{}' was not cancelled (status: {}{}). Future runs are unaffected — use action='pause' to stop them.",
                        result.run_log_id,
                        job.name,
                        result.status,
                        result
                            .code
                            .as_deref()
                            .map(|code| format!(", {code}"))
                            .unwrap_or_default()
                    )),
                    None => Ok(format!("Run {} not found.", run_log_id)),
                }
            }

            "workspace_status" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;
                reject_loop_managed_job(&job, id, "inspected")?;
                let session_db = cron_session_db().await?;
                let db = cron_db.clone();
                let session_db_for_query = session_db.clone();
                let id_for_query = id.to_string();
                let resources = ha_core::blocking::run_blocking(move || {
                    crate::cron::workspace_resources(
                        &db,
                        &session_db_for_query,
                        Some(&id_for_query),
                    )
                })
                .await?;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "jobId": job.id,
                    "workspacePolicy": job.workspace_policy,
                    "lifecycle": workspace_lifecycle(job.workspace_policy.mode),
                    "resources": resources,
                }))?)
            }

            "delete" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db.get_job(id)?;
                if let Some(job) = job.as_ref() {
                    reject_loop_managed_job(job, id, "deleted")?;
                }
                // Best-effort human-readable name for the approval dialog before
                // we touch anything.
                let desc = match job.as_ref() {
                    Some(job) => format!("Delete scheduled task '{}' (id={})", job.name, id),
                    None => format!("Delete scheduled task (id={})", id),
                };
                // OQ6: delete is the one consequential `manage_cron` action, so it
                // takes an explicit trip through the unified permission engine
                // (every other action stays internal-exempt). `?` aborts with an
                // already-rendered rejection on deny / unattended / timeout.
                gate_cron_delete(args, ctx, desc).await?;
                match ha_core::get_session_db() {
                    Some(session_db) => {
                        crate::cron::delete_job_and_legacy_sessions(cron_db, session_db, id).await?
                    }
                    // SessionDB should always be initialized when tools run; fall
                    // back to a job-only delete so the user's delete still lands.
                    None => cron_db.delete_job(id)?,
                }
                app_info!(
                    "cron",
                    "manage",
                    "manage_cron deleted scheduled task '{}' (approved)",
                    id
                );
                Ok(format!(
                    "Deleted scheduled task '{}'; its run history and conversations were retained.",
                    id
                ))
            }

            "pause" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;
                reject_loop_managed_job(&job, id, "paused")?;
                cron_db.toggle_job(id, false)?;
                Ok(format!("Paused scheduled task '{}'.", id))
            }

            "resume" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;
                reject_loop_managed_job(&job, id, "resumed")?;
                cron_db.toggle_job(id, true)?;
                Ok(format!("Resumed scheduled task '{}'.", id))
            }

            "run_now" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                // Cron only runs on the Primary instance — `execute_job_public`
                // no-ops on a Secondary (C10). This is the third run-now entry
                // point (alongside the Tauri command + HTTP route); without this
                // preflight, spawning on a Secondary would return "Triggered
                // immediate execution" to the model while nothing claims, runs,
                // logs, or delivers. Fail loudly instead of reporting a fake success.
                if !ha_core::runtime_lock::is_primary() {
                    anyhow::bail!(
                        "run_now is unavailable on this instance: scheduled jobs only run on the primary"
                    );
                }
                let job = {
                    let cron_db = cron_db.clone();
                    let id = id.to_string();
                    ha_core::blocking::run_blocking(move || cron_db.get_job(&id)).await?
                }
                .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

                // Prefer the global SessionDB (Tauri app); fall back to opening a fresh
                // connection (ACP mode where SESSION_DB OnceLock is never populated).
                let session_db = cron_session_db().await?;

                match start_cron_run_now(
                    cron_db.clone(),
                    session_db,
                    id.to_string(),
                    job.revision,
                    crate::cron::CronManagedWorkspaceWritePolicy::Allowed,
                )
                .await?
                {
                    CronRunNowResult::Started { revision, .. } => Ok(format!(
                        "Started immediate execution of '{}' at revision {}.",
                        id, revision
                    )),
                    CronRunNowResult::Rejected { report } => anyhow::bail!(
                        "run_now preflight rejected '{}': {}",
                        id,
                        report
                            .issues
                            .iter()
                            .map(|issue| format!("{:?}", issue.code))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }

            "list_channel_targets" => Ok(list_channel_targets_text()),

            "list_projects" => Ok(list_projects_text(
                args.get("include_archived")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )),

            _ => Err(anyhow::anyhow!(
                "Unknown action: '{}'. Valid actions: create, update, list, get, runs, delete, \
                 pause, resume, run_now, cancel_run, workspace_status, list_channel_targets, \
                 list_projects",
                action
            )),
        }
    })
}

async fn cron_session_db() -> Result<Arc<ha_core::session::SessionDB>> {
    if let Some(db) = ha_core::get_session_db() {
        return Ok(db.clone());
    }
    ha_core::blocking::run_blocking(move || {
        let path = ha_core::session::db_path()?;
        Ok(Arc::new(ha_core::session::SessionDB::open(&path)?))
    })
    .await
}

async fn require_cron_preflight(
    cron_db: Arc<cron::CronDB>,
    request: CronPreflightRequest,
) -> Result<()> {
    let report = evaluate_cron_preflight(
        cron_db,
        cron_session_db().await?,
        request,
        crate::cron::CronManagedWorkspaceWritePolicy::Allowed,
    )
    .await?;
    anyhow::ensure!(
        report.can_proceed,
        "cron preflight rejected: {:?}",
        report.issues
    );
    Ok(())
}

fn reject_loop_managed_job(job: &cron::CronJob, id: &str, action: &str) -> Result<()> {
    if matches!(job.payload, CronPayload::SessionLoop { .. }) {
        anyhow::bail!(
            "Scheduled task '{}' is managed by /loop and can only be {} from the loop control plane.",
            id,
            action
        );
    }
    Ok(())
}

/// Gate a `manage_cron action=delete` through the unified permission engine
/// (OQ6). `manage_cron` stays `internal:true`, so every other action runs
/// approval-free; delete alone re-enters the engine with `is_internal=false`,
/// where the engine's `check_cron_delete` rule emits the non-strict
/// `AskReason::CronDelete`. That makes the outcome session-mode-aware for free:
/// Default prompts, Smart lets the judge model decide, YOLO / global-YOLO /
/// AllowAlways bypass, and an unattended surface (a cron run with no human to
/// answer) fail-closes per `unattended_approval_action`. The shared
/// `run_tool_approval` keeps the strict-timeout / unattended / AllowAlways
/// handling in one place. Returns `Ok(())` to proceed or an already-rendered
/// `ToolRejection` to abort the delete.
async fn gate_cron_delete(
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
    desc: String,
) -> Result<()> {
    let decision =
        ha_core::tools::resolve_tool_permission(ha_core::tools::TOOL_MANAGE_CRON, args, ctx, false)
            .await;
    match decision {
        ha_core::permission::Decision::Allow => Ok(()),
        ha_core::permission::Decision::Deny { reason } => {
            Err(ha_core::tools::ToolRejection::denied_by_policy(
                ha_core::tools::TOOL_MANAGE_CRON,
                reason,
            ))
        }
        ha_core::permission::Decision::Ask { reason } => {
            // Force `allow_always_forbidden=true`: a one-off "Allow Always" on a
            // cron delete must NOT persist a standing grant. The allowlist matcher
            // for `manage_cron` keys on `action` only (not the job `id`), so a
            // persisted rule would silently authorize deleting *any* scheduled task
            // forever — and `allows_tool_call` is consulted before this gate, so it
            // would bypass the prompt on every future delete. CronDelete stays
            // non-strict for the timeout / unattended axis (this flag only governs
            // AllowAlways persistence); the frontend likewise disables the button.
            ha_core::tools::run_tool_approval(
                ha_core::tools::TOOL_MANAGE_CRON,
                args,
                ctx,
                Some(ha_core::tools::ApprovalReasonPayload::from(&reason)),
                true,
                Some(desc),
            )
            .await
            .map(|_origin| ())
        }
    }
}

enum CronTimeoutArg {
    Absent,
    Set(Option<u64>),
    Ignored,
}

/// Run-history read bounds: enough to diagnose a failure, not enough to dump a
/// task's whole output history into the context window.
const RUNS_DEFAULT_LIMIT: u64 = 5;
const RUNS_MAX_LIMIT: u64 = 20;
const RUN_TEXT_LIMIT: usize = 400;
/// The list is a survey, so its per-row error is a pointer to `action='runs'`.
const LIST_ERROR_LIMIT: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CronConversationTarget {
    New,
    Current,
    Existing,
}

fn parse_conversation_target(args: &Value) -> Result<CronConversationTarget> {
    let target = match args.get("conversation_target") {
        None => "new_session",
        Some(Value::String(target)) => target.as_str(),
        Some(_) => anyhow::bail!("conversation_target must be a string"),
    };
    match target {
        "new_session" => Ok(CronConversationTarget::New),
        "current_session" => Ok(CronConversationTarget::Current),
        "existing_session" => Ok(CronConversationTarget::Existing),
        other => anyhow::bail!(
            "Invalid conversation_target: '{}'. Use 'new_session', 'current_session', or 'existing_session'",
            other
        ),
    }
}

fn reject_existing_session_context_overrides(args: &Value) -> Result<()> {
    let supplied = [
        "agent_id",
        "project_id",
        "workspace_mode",
        "workspace_base_ref",
    ]
    .into_iter()
    .filter(|key| args.get(*key).is_some())
    .collect::<Vec<_>>();
    anyhow::ensure!(
        supplied.is_empty(),
        "existing-session tasks use the chat's live context; omit {}",
        supplied.join(", ")
    );
    Ok(())
}

async fn resolve_cron_job_timeout_secs_arg(
    args: &Value,
    ctx: &ha_core::tools::ToolExecContext,
) -> CronTimeoutArg {
    let Some(value) = args.get("job_timeout_secs") else {
        return CronTimeoutArg::Absent;
    };
    if value.is_null() {
        return CronTimeoutArg::Set(None);
    }
    let Some(requested_secs) = value.as_u64() else {
        return CronTimeoutArg::Set(None);
    };

    let effective_secs = ha_core::config::clamp_cron_job_timeout_secs(requested_secs);
    let user_limit_secs = ha_core::config::cached_config()
        .cron
        .effective_job_timeout_secs();

    if user_limit_secs > 0 && (requested_secs == 0 || effective_secs > user_limit_secs) {
        ha_core::tools::audit_model_runtime_timeout_override(
            Some(ctx),
            ha_core::tools::TOOL_MANAGE_CRON,
            "job_timeout_secs",
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "model supplied cron per-job timeout would relax global cron timeout",
        );
        ha_core::tools::emit_model_runtime_timeout_metadata(
            ctx,
            ha_core::tools::TOOL_MANAGE_CRON,
            "job_timeout_secs",
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "model supplied cron per-job timeout would relax global cron timeout",
        )
        .await;
        return CronTimeoutArg::Ignored;
    }

    if requested_secs > 0
        && ha_core::tools::should_ignore_model_runtime_timeout_when_user_unlimited(user_limit_secs)
    {
        ha_core::tools::audit_model_runtime_timeout_override(
            Some(ctx),
            ha_core::tools::TOOL_MANAGE_CRON,
            "job_timeout_secs",
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "global cron job timeout is unlimited",
        );
        ha_core::tools::emit_model_runtime_timeout_metadata(
            ctx,
            ha_core::tools::TOOL_MANAGE_CRON,
            "job_timeout_secs",
            requested_secs,
            user_limit_secs,
            Some(user_limit_secs),
            true,
            "global cron job timeout is unlimited",
        )
        .await;
        return CronTimeoutArg::Ignored;
    }

    ha_core::tools::audit_model_runtime_timeout_override(
        Some(ctx),
        ha_core::tools::TOOL_MANAGE_CRON,
        "job_timeout_secs",
        requested_secs,
        effective_secs,
        Some(user_limit_secs),
        false,
        "model supplied cron per-job timeout",
    );
    ha_core::tools::emit_model_runtime_timeout_metadata(
        ctx,
        ha_core::tools::TOOL_MANAGE_CRON,
        "job_timeout_secs",
        requested_secs,
        effective_secs,
        Some(user_limit_secs),
        false,
        "model supplied cron per-job timeout",
    )
    .await;
    CronTimeoutArg::Set(Some(requested_secs))
}

fn resolve_project_id_for_create(args: &Value, session_id: Option<&str>) -> Result<Option<String>> {
    if let Some(v) = args.get("project_id") {
        let project_id = parse_project_id_value(v)?;
        validate_project_id(project_id.as_deref())?;
        return Ok(project_id);
    }

    let project_id = session_id
        .and_then(|sid| ha_core::session::lookup_session_meta(Some(sid)))
        .and_then(|meta| meta.project_id);
    validate_project_id(project_id.as_deref())?;
    Ok(project_id)
}

fn parse_project_id_value(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let Some(raw) = value.as_str() else {
        return Err(anyhow::anyhow!(
            "Invalid 'project_id': expected string, null, or omitted"
        ));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn validate_project_id(project_id: Option<&str>) -> Result<()> {
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let project_db = ha_core::require_project_db()?;
    if project_db.get(project_id)?.is_none() {
        anyhow::bail!("Project '{}' not found", project_id);
    }
    Ok(())
}

fn project_label(project_id: &str) -> String {
    ha_core::get_project_db()
        .and_then(|db| db.get(project_id).ok().flatten())
        .map(|project| format!("{} ({})", project.display_label(), project.id))
        .unwrap_or_else(|| format!("{} (missing)", project_id))
}

fn resolve_workspace_policy(
    args: &Value,
    current: Option<&CronWorkspacePolicy>,
) -> Result<CronWorkspacePolicy> {
    let mut policy = current.cloned().unwrap_or_default().normalized();

    if let Some(value) = args.get("workspace_mode") {
        let raw = value.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid 'workspace_mode': expected project, fresh_worktree, or persistent_worktree"
            )
        })?;
        policy.mode = match raw.trim() {
            "project" => CronWorkspaceMode::Project,
            "fresh_worktree" | "freshWorktree" | "fresh" => CronWorkspaceMode::Fresh,
            "persistent_worktree" | "persistentWorktree" | "persistent" => {
                CronWorkspaceMode::Persistent
            }
            other => {
                anyhow::bail!(
                    "Invalid workspace_mode: '{}'. Use 'project', 'fresh_worktree', or 'persistent_worktree'",
                    other
                )
            }
        };
    }

    let supplied_cleanup = args
        .get("workspace_cleanup")
        .map(parse_workspace_cleanup)
        .transpose()?;
    if let Some(cleanup) = supplied_cleanup {
        policy.cleanup = cleanup;
    }

    let supplied_base_ref = args
        .get("workspace_base_ref")
        .map(parse_workspace_base_ref)
        .transpose()?;
    if let Some(base_ref) = supplied_base_ref.clone() {
        policy.base_ref = base_ref;
    }

    if policy.mode == CronWorkspaceMode::Project {
        if matches!(supplied_base_ref, Some(Some(_))) {
            anyhow::bail!(
                "workspace_base_ref is only valid for fresh_worktree or persistent_worktree"
            );
        }
        policy.base_ref = None;
    }
    // Reject rather than normalize away: a task asked to clean up after each run
    // must not come back silently as "retain everything".
    if policy.mode != CronWorkspaceMode::Fresh && policy.cleanup != CronWorkspaceCleanup::Retain {
        anyhow::bail!(
            "workspace_cleanup is only valid for workspace_mode=fresh_worktree; persistent reuses one Worktree and project creates none"
        );
    }

    Ok(policy.normalized())
}

fn parse_workspace_cleanup(value: &Value) -> Result<CronWorkspaceCleanup> {
    let raw = value.as_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid 'workspace_cleanup': expected retain, discard_if_clean, or always")
    })?;
    match raw.trim() {
        "retain" | "keep" => Ok(CronWorkspaceCleanup::Retain),
        "discard_if_clean" | "discardIfClean" => Ok(CronWorkspaceCleanup::DiscardIfClean),
        "always" | "always_discard" => Ok(CronWorkspaceCleanup::Always),
        other => anyhow::bail!(
            "Invalid workspace_cleanup: '{}'. Use 'retain', 'discard_if_clean', or 'always'",
            other
        ),
    }
}

fn workspace_cleanup_label(cleanup: CronWorkspaceCleanup) -> &'static str {
    match cleanup {
        CronWorkspaceCleanup::Retain => "retain",
        CronWorkspaceCleanup::DiscardIfClean => "discard_if_clean",
        CronWorkspaceCleanup::Always => "always",
    }
}

fn parse_workspace_base_ref(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let raw = value.as_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid 'workspace_base_ref': expected string, null, or omitted")
    })?;
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

fn workspace_mode_label(mode: CronWorkspaceMode) -> &'static str {
    match mode {
        CronWorkspaceMode::Project => "project",
        CronWorkspaceMode::Fresh => "fresh_worktree",
        CronWorkspaceMode::Persistent => "persistent_worktree",
    }
}

fn workspace_lifecycle(mode: CronWorkspaceMode) -> Value {
    match mode {
        CronWorkspaceMode::Project => serde_json::json!({
            "afterRun": "project_directory_unchanged",
            "automaticCleanup": false,
            "meaning": "No managed Worktree is created in Project mode."
        }),
        CronWorkspaceMode::Fresh => serde_json::json!({
            "afterRun": "retained",
            "automaticCleanup": false,
            "meaning": "Each run creates a separate Worktree that remains available until the user explicitly archives or discards it."
        }),
        CronWorkspaceMode::Persistent => serde_json::json!({
            "afterRun": "ready_for_next_run",
            "automaticCleanup": false,
            "meaning": "The task-owned Worktree is retained and reused by later runs until the user explicitly discards it."
        }),
    }
}

/// Parse schedule from tool arguments.
fn parse_schedule(args: &Value) -> Result<CronSchedule> {
    let schedule_type = args
        .get("schedule_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'schedule_type' parameter (at, every, or cron)"))?;

    // Each arm extracts + normalizes the JSON fields (presence errors are
    // field-specific here), then delegates *value* validation to the single
    // source of truth `cron::validate_schedule` — the same check the persistence
    // chokepoint (`add_job`/`update_job`) and the owner-plane paths run, so the
    // three entry points can never diverge on what a legal schedule is.
    let schedule = match schedule_type {
        "at" => {
            let timestamp = args
                .get("timestamp")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'timestamp' for 'at' schedule"))?;
            CronSchedule::At {
                timestamp: timestamp.to_string(),
            }
        }
        "every" => {
            let interval_ms = args
                .get("interval_ms")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'interval_ms' for 'every' schedule"))?;
            let start_at = args
                .get("start_at")
                .and_then(|v| v.as_str())
                .map(String::from);
            CronSchedule::Every {
                interval_ms,
                start_at,
            }
        }
        "cron" => {
            let expression = args
                .get("cron_expression")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'cron_expression' for 'cron' schedule"))?;
            // Trim the timezone here (normalization); validity is checked by
            // validate_schedule below. An empty value normalizes to `None` (UTC).
            let timezone = match args.get("timezone").and_then(|v| v.as_str()) {
                Some(raw) if !raw.trim().is_empty() => Some(raw.trim().to_string()),
                _ => None,
            };
            CronSchedule::Cron {
                expression: expression.to_string(),
                timezone,
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Invalid schedule_type: '{}'. Use 'at', 'every', or 'cron'",
                schedule_type
            ))
        }
    };
    cron::validate_schedule(&schedule)?;
    Ok(schedule)
}

/// Human-readable schedule summary.
fn schedule_summary(schedule: &CronSchedule) -> String {
    match schedule {
        CronSchedule::At { timestamp } => format!("once at {}", timestamp),
        CronSchedule::Every { interval_ms, .. } => {
            let secs = interval_ms / 1000;
            if secs < 60 {
                format!("every {}s", secs)
            } else if secs < 3600 {
                format!("every {}m", secs / 60)
            } else if secs < 86400 {
                format!("every {}h", secs / 3600)
            } else {
                format!("every {}d", secs / 86400)
            }
        }
        CronSchedule::Cron {
            expression,
            timezone,
        } => match timezone.as_deref() {
            Some(tz) => format!("cron: {} ({})", expression, tz),
            None => format!("cron: {} (UTC)", expression),
        },
    }
}

/// Resolve the delivery_targets for a `create` call.
///
/// - args key missing / explicit null → try to infer from the current session's
///   IM channel conversation (if the caller is chatting via an IM channel). Returns
///   `(inferred_targets, true)` if inference kicked in, otherwise `(vec![], false)`.
/// - `delivery_targets=[]` → explicit opt-out, returns `(vec![], false)`.
/// - `delivery_targets=[...]` → parsed verbatim, returns `(parsed, false)`.
fn resolve_delivery_targets_for_create(
    args: &Value,
    session_id: Option<&str>,
) -> Result<(Vec<CronDeliveryTarget>, bool)> {
    match args.get("delivery_targets") {
        None | Some(Value::Null) => {
            // Try to infer from current channel session.
            if let (Some(sid), Some(db)) = (session_id, ha_core::get_channel_db()) {
                if let Ok(Some(conv)) = db.get_conversation_by_session(sid) {
                    let label = conv
                        .sender_name
                        .clone()
                        .filter(|s| !s.is_empty())
                        .map(|name| format!("{} / {}", conv.channel_id, name))
                        .or_else(|| Some(format!("{} / {}", conv.channel_id, conv.chat_id)));
                    let target = CronDeliveryTarget {
                        channel_id: conv.channel_id,
                        account_id: conv.account_id,
                        chat_id: conv.chat_id,
                        thread_id: conv.thread_id,
                        label,
                        stale: false,
                    };
                    return Ok((vec![target], true));
                }
            }
            Ok((Vec::new(), false))
        }
        Some(v) => {
            let parsed: Vec<CronDeliveryTarget> = serde_json::from_value(v.clone())
                .map_err(|e| anyhow::anyhow!("Invalid 'delivery_targets': {}", e))?;
            Ok((parsed, false))
        }
    }
}

/// Create/update half of the delivery whitelist (OQ5). Every target the model
/// supplies explicitly must point at a conversation already recorded in
/// `channel_conversations` — the same set `action='list_channel_targets'`
/// surfaces. Rejecting unknown targets at create/update time stops a
/// prompt-injected model from *persisting* a cron job that fans output out to an
/// attacker-controlled chat (a periodic, account-authenticated exfil channel),
/// and is the create-time complement to the runtime skip-and-warn guard in
/// `cron::delivery`. Inferred targets (derived from the caller's own IM
/// conversation row) skip this — they are recorded by construction.
/// Compact single-line summary of delivery targets for status messages.
fn format_targets_inline(targets: &[CronDeliveryTarget]) -> String {
    targets
        .iter()
        .map(|t| {
            let base = format!("{}:{}", t.channel_id, t.chat_id);
            match &t.thread_id {
                Some(tid) => format!("{} (thread {})", base, tid),
                None => base,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// List every enabled IM channel account and its recorded conversations as
/// candidate delivery targets for cron jobs. Output is both human-readable and
/// copy-pasteable — the model can read the `channel_id=... account_id=... chat_id=...`
/// fields straight into a subsequent `create` / `update` call.
fn list_channel_targets_text() -> String {
    let store = ha_core::config::cached_config();
    let channel_db = ha_core::get_channel_db();

    let enabled: Vec<_> = store
        .channels
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .collect();
    if enabled.is_empty() {
        return "No enabled IM channel accounts are configured. \
                Open Settings → Channels to set one up first."
            .to_string();
    }

    let mut blocks = Vec::new();
    let mut total = 0usize;

    for account in &enabled {
        let channel_slug = account.channel_id.to_string();
        let conversations = match channel_db.as_ref() {
            Some(db) => db
                .list_conversations(&channel_slug, &account.id)
                .unwrap_or_default(),
            None => Vec::new(),
        };

        if conversations.is_empty() {
            blocks.push(format!(
                "[{channel_slug} · \"{label}\" (account_id={account_id})]\n    no recorded conversations yet — send the bot a message first to register a chat",
                channel_slug = channel_slug,
                label = account.label,
                account_id = account.id,
            ));
            continue;
        }

        for conv in &conversations {
            total += 1;
            let display = conv
                .sender_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .unwrap_or_else(|| conv.chat_id.clone());
            let thread_part = conv
                .thread_id
                .as_deref()
                .map(|t| format!("  thread_id=\"{}\"", t))
                .unwrap_or_default();
            blocks.push(format!(
                "[{idx}] {channel_slug} · \"{display}\" ({chat_type})\n    \
                 channel_id=\"{channel_slug}\"  account_id=\"{account_id}\"  chat_id=\"{chat_id}\"{thread_part}",
                idx = total,
                channel_slug = channel_slug,
                display = display,
                chat_type = conv.chat_type,
                account_id = account.id,
                chat_id = conv.chat_id,
                thread_part = thread_part,
            ));
        }
    }

    format!(
        "Found {} channel target(s):\n\n{}\n\nPass the ids above into `delivery_targets` \
         on `action=create` or `action=update`.",
        total,
        blocks.join("\n\n"),
    )
}

fn list_projects_text(include_archived: bool) -> String {
    let project_db = match ha_core::require_project_db() {
        Ok(db) => db,
        Err(e) => return format!("Project service not initialized: {}", e),
    };

    let projects = match project_db.list(include_archived, None) {
        Ok(projects) => projects,
        Err(e) => return format!("Failed to list projects: {}", e),
    };

    if projects.is_empty() {
        return "No projects found.".to_string();
    }

    let mut lines = vec![format!("{} project(s):", projects.len())];
    for meta in projects {
        let project = meta.project;
        let archived = if project.archived { " | archived" } else { "" };
        let default_agent = project
            .default_agent_id
            .as_deref()
            .map(|id| format!(" | default_agent={}", id))
            .unwrap_or_default();
        let description = project
            .description
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!(" — {}", ha_core::truncate_utf8(s, 120)))
            .unwrap_or_default();
        lines.push(format!(
            "  - {} | project_id=\"{}\"{}{}{}",
            project.display_label(),
            project.id,
            default_agent,
            archived,
            description
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        parse_conversation_target, parse_project_id_value,
        reject_existing_session_context_overrides, resolve_workspace_policy, workspace_lifecycle,
        CronConversationTarget,
    };
    use ha_core::cron::{CronWorkspaceCleanup, CronWorkspaceMode, CronWorkspacePolicy};
    use serde_json::{json, Value};

    #[test]
    fn conversation_target_defaults_to_new_session_and_accepts_current() {
        assert_eq!(
            parse_conversation_target(&json!({})).unwrap(),
            CronConversationTarget::New
        );
        assert_eq!(
            parse_conversation_target(&json!({"conversation_target": "current_session"})).unwrap(),
            CronConversationTarget::Current
        );
        assert_eq!(
            parse_conversation_target(&json!({"conversation_target": "existing_session"})).unwrap(),
            CronConversationTarget::Existing
        );
        assert!(parse_conversation_target(&json!({"conversation_target": null})).is_err());
        assert!(
            parse_conversation_target(&json!({"conversation_target": "another_session"})).is_err()
        );
    }

    #[test]
    fn existing_session_target_rejects_task_context_copies() {
        reject_existing_session_context_overrides(&json!({})).unwrap();
        for key in [
            "agent_id",
            "project_id",
            "workspace_mode",
            "workspace_base_ref",
        ] {
            let mut args = serde_json::Map::new();
            args.insert(key.to_string(), Value::Null);
            assert!(reject_existing_session_context_overrides(&Value::Object(args)).is_err());
        }
    }

    #[test]
    fn parse_project_id_value_clears_null_and_empty_string() {
        assert_eq!(parse_project_id_value(&json!(null)).unwrap(), None);
        assert_eq!(parse_project_id_value(&json!("")).unwrap(), None);
        assert_eq!(parse_project_id_value(&json!("  ")).unwrap(), None);
    }

    #[test]
    fn parse_project_id_value_trims_project_id() {
        assert_eq!(
            parse_project_id_value(&json!(" project-1 "))
                .unwrap()
                .as_deref(),
            Some("project-1")
        );
    }

    #[test]
    fn parse_project_id_value_rejects_non_string() {
        assert!(parse_project_id_value(&json!(123)).is_err());
    }

    #[test]
    fn workspace_policy_defaults_to_project() {
        let policy = resolve_workspace_policy(&json!({}), None).unwrap();
        assert_eq!(policy, CronWorkspacePolicy::default());
    }

    #[test]
    fn workspace_policy_parses_worktree_mode_and_trimmed_base_ref() {
        let policy = resolve_workspace_policy(
            &json!({
                "workspace_mode": "fresh_worktree",
                "workspace_base_ref": " main "
            }),
            None,
        )
        .unwrap();
        assert_eq!(policy.mode, CronWorkspaceMode::Fresh);
        assert_eq!(policy.base_ref.as_deref(), Some("main"));
    }

    #[test]
    fn workspace_policy_update_can_switch_to_project_and_clear_base_ref() {
        let current = CronWorkspacePolicy {
            mode: CronWorkspaceMode::Persistent,
            base_ref: Some("release".to_string()),
            cleanup: CronWorkspaceCleanup::Retain,
        };
        let policy =
            resolve_workspace_policy(&json!({ "workspace_mode": "project" }), Some(&current))
                .unwrap();
        assert_eq!(policy, CronWorkspacePolicy::default());
    }

    #[test]
    fn workspace_policy_update_can_clear_worktree_base_ref() {
        let current = CronWorkspacePolicy {
            mode: CronWorkspaceMode::Fresh,
            base_ref: Some("release".to_string()),
            cleanup: CronWorkspaceCleanup::DiscardIfClean,
        };
        let policy =
            resolve_workspace_policy(&json!({ "workspace_base_ref": null }), Some(&current))
                .unwrap();
        assert_eq!(policy.mode, CronWorkspaceMode::Fresh);
        assert_eq!(policy.base_ref, None);
        assert_eq!(
            policy.cleanup,
            CronWorkspaceCleanup::DiscardIfClean,
            "clearing the base ref must not reset an unrelated cleanup policy",
        );
    }

    #[test]
    fn workspace_cleanup_is_fresh_only_and_survives_a_round_trip() {
        let policy = resolve_workspace_policy(
            &json!({ "workspace_mode": "fresh_worktree", "workspace_cleanup": "discard_if_clean" }),
            None,
        )
        .unwrap();
        assert_eq!(policy.cleanup, CronWorkspaceCleanup::DiscardIfClean);

        // Switching a cleaning task to a mode that has nothing to clean up is a
        // contradiction, not something to silently normalize away.
        let error = resolve_workspace_policy(
            &json!({ "workspace_mode": "persistent_worktree", "workspace_cleanup": "always" }),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("fresh_worktree"));

        let error =
            resolve_workspace_policy(&json!({ "workspace_cleanup": "burn" }), None).unwrap_err();
        assert!(error.to_string().contains("Invalid workspace_cleanup"));
    }

    #[test]
    fn workspace_policy_rejects_base_ref_in_project_mode() {
        let error = resolve_workspace_policy(
            &json!({ "workspace_base_ref": "main" }),
            Some(&CronWorkspacePolicy::default()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("only valid"));
    }

    #[test]
    fn fresh_workspace_lifecycle_never_claims_automatic_cleanup() {
        let lifecycle = workspace_lifecycle(CronWorkspaceMode::Fresh);
        assert_eq!(lifecycle["afterRun"], "retained");
        assert_eq!(lifecycle["automaticCleanup"], false);
        assert!(lifecycle["meaning"]
            .as_str()
            .is_some_and(|value| value.contains("explicitly archives or discards")));
    }
}

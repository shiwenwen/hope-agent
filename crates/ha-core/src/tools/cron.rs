use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

use crate::cron::{self, CronDeliveryTarget, CronPayload, CronSchedule, NewCronJob};

/// Tool: manage_cron — create, list, get, update, delete, and trigger scheduled tasks,
/// and discover IM channel delivery targets.
///
/// Returns `Pin<Box<dyn Future + Send>>` instead of an opaque `async fn` future
/// to break the type-level recursion: tool_manage_cron → execute_job → agent.chat
/// → execute_tool_with_context → tool_manage_cron. Without the boxing, the compiler
/// cannot compute the infinite recursive future type to verify `Send`.
pub(crate) fn tool_manage_cron<'a>(
    args: &'a Value,
    ctx: &'a super::ToolExecContext,
) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
    // Own the session_id so the returned future doesn't borrow from the caller;
    // `ctx` itself is only needed by the `delete` approval gate below.
    let session_id = ctx.session_id.clone();
    Box::pin(async move {
        let cron_db =
            crate::get_cron_db().ok_or_else(|| anyhow::anyhow!("Cron service not initialized"))?;

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

                let agent_id = args
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let (delivery_targets, inferred) =
                    resolve_delivery_targets_for_create(args, session_id.as_deref())?;
                let project_id = resolve_project_id_for_create(args, session_id.as_deref())?;

                let input = NewCronJob {
                    name: name.to_string(),
                    description,
                    project_id,
                    schedule,
                    payload: CronPayload::AgentTurn {
                        prompt: prompt.to_string(),
                        agent_id,
                    },
                    max_failures: args
                        .get("max_failures")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    notify_on_complete: args.get("notify_on_complete").and_then(|v| v.as_bool()),
                    delivery_targets: Some(delivery_targets),
                    prefix_delivery_with_name: args
                        .get("prefix_delivery_with_name")
                        .and_then(|v| v.as_bool()),
                    job_timeout_secs: args.get("job_timeout_secs").and_then(|v| v.as_u64()),
                };

                let job = cron_db.add_job(&input)?;
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
                Ok(out)
            }

            "update" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let mut job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

                if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
                    job.name = name.to_string();
                }
                if let Some(desc) = args.get("description") {
                    job.description = desc.as_str().map(String::from);
                }
                if args.get("schedule_type").is_some() {
                    job.schedule = parse_schedule(args)?;
                }
                let CronPayload::AgentTurn {
                    ref mut prompt,
                    ref mut agent_id,
                } = job.payload;
                if let Some(p) = args.get("prompt").and_then(|v| v.as_str()) {
                    *prompt = p.to_string();
                }
                if let Some(v) = args.get("agent_id") {
                    *agent_id = v.as_str().map(String::from);
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
                if let Some(v) = args.get("job_timeout_secs") {
                    job.job_timeout_secs = v.as_u64();
                }
                if let Some(v) = args.get("project_id") {
                    job.project_id = parse_project_id_value(v)?;
                    validate_project_id(job.project_id.as_deref())?;
                }
                // delivery_targets tri-state on update (no inference — never silently
                // clobber what the user set in the GUI).
                if let Some(v) = args.get("delivery_targets") {
                    if !v.is_null() {
                        let parsed: Vec<CronDeliveryTarget> = serde_json::from_value(v.clone())
                            .map_err(|e| anyhow::anyhow!("Invalid 'delivery_targets': {}", e))?;
                        validate_delivery_targets(&parsed)?;
                        job.delivery_targets = parsed;
                    }
                }

                cron_db.update_job(&job)?;
                Ok(format!(
                    "Updated scheduled task '{}' (id: {}). Next run: {} | Project: {} | Targets: {}",
                    job.name,
                    job.id,
                    job.next_run_at.as_deref().unwrap_or("none"),
                    job.project_id
                        .as_deref()
                        .map(project_label)
                        .unwrap_or_else(|| "none".to_string()),
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
                let mut lines = Vec::new();
                lines.push(format!("{} scheduled task(s):", jobs.len()));
                for job in &jobs {
                    let next = job.next_run_at.as_deref().unwrap_or("none");
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
                    lines.push(format!(
                        "  - [{}] {} ({}) | Next: {} | Status: {}{}{}",
                        crate::truncate_utf8(&job.id, 8),
                        job.name,
                        schedule_summary(&job.schedule),
                        next,
                        job.status.as_str(),
                        project,
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
                    Some(job) => Ok(serde_json::to_string_pretty(&job)?),
                    None => Ok(format!("Job '{}' not found.", id)),
                }
            }

            "delete" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                // Best-effort human-readable name for the approval dialog before
                // we touch anything.
                let desc = match cron_db.get_job(id) {
                    Ok(Some(job)) => format!("Delete scheduled task '{}' (id={})", job.name, id),
                    _ => format!("Delete scheduled task (id={})", id),
                };
                // OQ6: delete is the one consequential `manage_cron` action, so it
                // takes an explicit trip through the unified permission engine
                // (every other action stays internal-exempt). `?` aborts with an
                // already-rendered rejection on deny / unattended / timeout.
                gate_cron_delete(args, ctx, desc).await?;
                cron_db.delete_job(id)?;
                app_info!(
                    "cron",
                    "manage",
                    "manage_cron deleted scheduled task '{}' (approved)",
                    id
                );
                Ok(format!("Deleted scheduled task '{}'.", id))
            }

            "pause" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                cron_db.toggle_job(id, false)?;
                Ok(format!("Paused scheduled task '{}'.", id))
            }

            "resume" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                cron_db.toggle_job(id, true)?;
                Ok(format!("Resumed scheduled task '{}'.", id))
            }

            "run_now" => {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
                let job = cron_db
                    .get_job(id)?
                    .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

                // Fire and forget — run in background via tokio::spawn.
                // The type-level recursion is broken by this function returning
                // Pin<Box<dyn Future + Send>> instead of an opaque async fn future.
                let db = cron_db.clone();
                let job_clone = job;
                // Prefer the global SessionDB (Tauri app); fall back to opening a fresh
                // connection (ACP mode where SESSION_DB OnceLock is never populated).
                let session_db = match crate::get_session_db() {
                    Some(db) => db.clone(),
                    None => {
                        let path = crate::session::db_path()?;
                        std::sync::Arc::new(crate::session::SessionDB::open(&path)?)
                    }
                };

                tokio::spawn(async move {
                    cron::execute_job_public(&db, &session_db, &job_clone).await;
                });
                Ok(format!("Triggered immediate execution of '{}'.", id))
            }

            "list_channel_targets" => Ok(list_channel_targets_text()),

            "list_projects" => Ok(list_projects_text(
                args.get("include_archived")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )),

            _ => Err(anyhow::anyhow!(
                "Unknown action: '{}'. Valid actions: create, update, list, get, delete, \
                 pause, resume, run_now, list_channel_targets, list_projects",
                action
            )),
        }
    })
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
async fn gate_cron_delete(args: &Value, ctx: &super::ToolExecContext, desc: String) -> Result<()> {
    let decision =
        super::execution::resolve_tool_permission(super::TOOL_MANAGE_CRON, args, ctx, false).await;
    match decision {
        crate::permission::Decision::Allow => Ok(()),
        crate::permission::Decision::Deny { reason } => Err(
            super::rejection::ToolRejection::denied_by_policy(super::TOOL_MANAGE_CRON, reason),
        ),
        crate::permission::Decision::Ask { reason } => {
            // Force `allow_always_forbidden=true`: a one-off "Allow Always" on a
            // cron delete must NOT persist a standing grant. The allowlist matcher
            // for `manage_cron` keys on `action` only (not the job `id`), so a
            // persisted rule would silently authorize deleting *any* scheduled task
            // forever — and `allows_tool_call` is consulted before this gate, so it
            // would bypass the prompt on every future delete. CronDelete stays
            // non-strict for the timeout / unattended axis (this flag only governs
            // AllowAlways persistence); the frontend likewise disables the button.
            super::execution::run_tool_approval(
                super::TOOL_MANAGE_CRON,
                args,
                ctx,
                Some(super::approval::ApprovalReasonPayload::from(&reason)),
                true,
                Some(desc),
            )
            .await
            .map(|_origin| ())
        }
    }
}

fn resolve_project_id_for_create(args: &Value, session_id: Option<&str>) -> Result<Option<String>> {
    if let Some(v) = args.get("project_id") {
        let project_id = parse_project_id_value(v)?;
        validate_project_id(project_id.as_deref())?;
        return Ok(project_id);
    }

    let project_id = session_id
        .and_then(|sid| crate::session::lookup_session_meta(Some(sid)))
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
    let project_db = crate::require_project_db()?;
    if project_db.get(project_id)?.is_none() {
        anyhow::bail!("Project '{}' not found", project_id);
    }
    Ok(())
}

fn project_label(project_id: &str) -> String {
    crate::get_project_db()
        .and_then(|db| db.get(project_id).ok().flatten())
        .map(|project| format!("{} ({})", project.display_label(), project.id))
        .unwrap_or_else(|| format!("{} (missing)", project_id))
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
            if let (Some(sid), Some(db)) = (session_id, crate::get_channel_db()) {
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
            validate_delivery_targets(&parsed)?;
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
fn validate_delivery_targets(targets: &[CronDeliveryTarget]) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let Some(db) = crate::get_channel_db() else {
        anyhow::bail!(
            "Cannot validate delivery_targets: the IM channel subsystem is not available. \
             Configure an IM channel account first."
        );
    };
    for t in targets {
        let known = db
            .conversation_exists(
                &t.channel_id,
                &t.account_id,
                &t.chat_id,
                t.thread_id.as_deref(),
            )
            .unwrap_or(false);
        if !known {
            anyhow::bail!(
                "delivery_target {}:{} (account '{}') is not a recorded conversation. \
                 Call action='list_channel_targets' to discover valid \
                 channel_id/account_id/chat_id triples before setting delivery_targets.",
                t.channel_id,
                t.chat_id,
                t.account_id
            );
        }
    }
    Ok(())
}

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
    let store = crate::config::cached_config();
    let channel_db = crate::get_channel_db();

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
    let project_db = match crate::require_project_db() {
        Ok(db) => db,
        Err(e) => return format!("Project service not initialized: {}", e),
    };

    let projects = match project_db.list(include_archived) {
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
            .map(|s| format!(" — {}", crate::truncate_utf8(s, 120)))
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
    use super::parse_project_id_value;
    use serde_json::json;

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
}

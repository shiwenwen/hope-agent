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
    session_id: Option<&'a str>,
) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
    // Own the session_id so the returned future doesn't borrow from the caller.
    let session_id = session_id.map(String::from);
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

                let input = NewCronJob {
                    name: name.to_string(),
                    description,
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
                // delivery_targets tri-state on update (no inference — never silently
                // clobber what the user set in the GUI).
                if let Some(v) = args.get("delivery_targets") {
                    if !v.is_null() {
                        let parsed: Vec<CronDeliveryTarget> = serde_json::from_value(v.clone())
                            .map_err(|e| {
                                anyhow::anyhow!("Invalid 'delivery_targets': {}", e)
                            })?;
                        job.delivery_targets = parsed;
                    }
                }

                cron_db.update_job(&job)?;
                Ok(format!(
                    "Updated scheduled task '{}' (id: {}). Next run: {} | Targets: {}",
                    job.name,
                    job.id,
                    job.next_run_at.as_deref().unwrap_or("none"),
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
                        format!(" | Targets: {}", format_targets_inline(&job.delivery_targets))
                    };
                    lines.push(format!(
                        "  - [{}] {} ({}) | Next: {} | Status: {}{}",
                        &job.id[..8],
                        job.name,
                        schedule_summary(&job.schedule),
                        next,
                        job.status.as_str(),
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
                cron_db.delete_job(id)?;
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

            _ => Err(anyhow::anyhow!(
                "Unknown action: '{}'. Valid actions: create, update, list, get, delete, \
                 pause, resume, run_now, list_channel_targets",
                action
            )),
        }
    })
}

/// Parse schedule from tool arguments.
fn parse_schedule(args: &Value) -> Result<CronSchedule> {
    let schedule_type = args
        .get("schedule_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'schedule_type' parameter (at, every, or cron)"))?;

    match schedule_type {
        "at" => {
            let timestamp = args
                .get("timestamp")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'timestamp' for 'at' schedule"))?;
            // Validate ISO8601
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
            Ok(CronSchedule::At {
                timestamp: timestamp.to_string(),
            })
        }
        "every" => {
            let interval_ms = args
                .get("interval_ms")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'interval_ms' for 'every' schedule"))?;
            if interval_ms < 60_000 {
                return Err(anyhow::anyhow!(
                    "Interval must be at least 60000ms (1 minute)"
                ));
            }
            Ok(CronSchedule::Every { interval_ms })
        }
        "cron" => {
            let expression = args
                .get("cron_expression")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'cron_expression' for 'cron' schedule"))?;
            cron::validate_cron_expression(expression)?;
            let timezone = args
                .get("timezone")
                .and_then(|v| v.as_str())
                .map(String::from);
            Ok(CronSchedule::Cron {
                expression: expression.to_string(),
                timezone,
            })
        }
        _ => Err(anyhow::anyhow!(
            "Invalid schedule_type: '{}'. Use 'at', 'every', or 'cron'",
            schedule_type
        )),
    }
}

/// Human-readable schedule summary.
fn schedule_summary(schedule: &CronSchedule) -> String {
    match schedule {
        CronSchedule::At { timestamp } => format!("once at {}", timestamp),
        CronSchedule::Every { interval_ms } => {
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
        CronSchedule::Cron { expression, .. } => format!("cron: {}", expression),
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
                        .or_else(|| {
                            Some(format!("{} / {}", conv.channel_id, conv.chat_id))
                        });
                    let target = CronDeliveryTarget {
                        channel_id: conv.channel_id,
                        account_id: conv.account_id,
                        chat_id: conv.chat_id,
                        thread_id: conv.thread_id,
                        label,
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

    let enabled: Vec<_> = store.channels.accounts.iter().filter(|a| a.enabled).collect();
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

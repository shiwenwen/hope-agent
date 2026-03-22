use anyhow::Result;
use serde_json::Value;

use crate::cron::{self, CronPayload, CronSchedule, NewCronJob};

/// Tool: manage_cron — create, list, update, delete, and trigger scheduled tasks.
pub(crate) async fn tool_manage_cron(args: &Value) -> Result<String> {
    let cron_db = crate::get_cron_db()
        .ok_or_else(|| anyhow::anyhow!("Cron service not initialized"))?;

    let action = args.get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "create" => {
            let name = args.get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

            let prompt = args.get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' parameter"))?;

            let schedule = parse_schedule(args)?;

            let agent_id = args.get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);

            let description = args.get("description")
                .and_then(|v| v.as_str())
                .map(String::from);

            let input = NewCronJob {
                name: name.to_string(),
                description,
                schedule,
                payload: CronPayload::AgentTurn {
                    prompt: prompt.to_string(),
                    agent_id,
                },
                max_failures: args.get("max_failures").and_then(|v| v.as_u64()).map(|v| v as u32),
                notify_on_complete: args.get("notify_on_complete").and_then(|v| v.as_bool()),
            };

            let job = cron_db.add_job(&input)?;
            Ok(format!("Created scheduled task '{}' (id: {}). Next run: {}",
                job.name, job.id,
                job.next_run_at.as_deref().unwrap_or("none")
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
                lines.push(format!("  - [{}] {} ({}) | Next: {} | Status: {}",
                    &job.id[..8], job.name,
                    schedule_summary(&job.schedule),
                    next, job.status.as_str()
                ));
            }
            Ok(lines.join("\n"))
        }

        "get" => {
            let id = args.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
            match cron_db.get_job(id)? {
                Some(job) => Ok(serde_json::to_string_pretty(&job)?),
                None => Ok(format!("Job '{}' not found.", id)),
            }
        }

        "delete" => {
            let id = args.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
            cron_db.delete_job(id)?;
            Ok(format!("Deleted scheduled task '{}'.", id))
        }

        "pause" => {
            let id = args.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
            cron_db.toggle_job(id, false)?;
            Ok(format!("Paused scheduled task '{}'.", id))
        }

        "resume" => {
            let id = args.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
            cron_db.toggle_job(id, true)?;
            Ok(format!("Resumed scheduled task '{}'.", id))
        }

        "run_now" => {
            let id = args.get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter"))?;
            let job = cron_db.get_job(id)?
                .ok_or_else(|| anyhow::anyhow!("Job '{}' not found", id))?;

            // Fire and forget — run in background
            // We need Send-safe clones before spawning
            let db = cron_db.clone();
            let job_clone = job;
            let session_db_path = crate::session::db_path()?;

            tokio::task::spawn_blocking(move || {
                let session_db = std::sync::Arc::new(
                    crate::session::SessionDB::open(&session_db_path).unwrap()
                );
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    cron::execute_job_public(&db, &session_db, &job_clone).await;
                });
            });
            Ok(format!("Triggered immediate execution of '{}'.", id))
        }

        _ => Err(anyhow::anyhow!("Unknown action: '{}'. Valid actions: create, list, get, delete, pause, resume, run_now", action)),
    }
}

/// Parse schedule from tool arguments.
fn parse_schedule(args: &Value) -> Result<CronSchedule> {
    let schedule_type = args.get("schedule_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'schedule_type' parameter (at, every, or cron)"))?;

    match schedule_type {
        "at" => {
            let timestamp = args.get("timestamp")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'timestamp' for 'at' schedule"))?;
            // Validate ISO8601
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .map_err(|e| anyhow::anyhow!("Invalid timestamp: {}", e))?;
            Ok(CronSchedule::At { timestamp: timestamp.to_string() })
        }
        "every" => {
            let interval_ms = args.get("interval_ms")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| anyhow::anyhow!("Missing 'interval_ms' for 'every' schedule"))?;
            if interval_ms < 60_000 {
                return Err(anyhow::anyhow!("Interval must be at least 60000ms (1 minute)"));
            }
            Ok(CronSchedule::Every { interval_ms })
        }
        "cron" => {
            let expression = args.get("cron_expression")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'cron_expression' for 'cron' schedule"))?;
            cron::validate_cron_expression(expression)?;
            let timezone = args.get("timezone")
                .and_then(|v| v.as_str())
                .map(String::from);
            Ok(CronSchedule::Cron {
                expression: expression.to_string(),
                timezone,
            })
        }
        _ => Err(anyhow::anyhow!("Invalid schedule_type: '{}'. Use 'at', 'every', or 'cron'", schedule_type)),
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

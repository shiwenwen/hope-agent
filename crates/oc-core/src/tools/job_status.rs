//! `job_status` tool — query or actively wait on an async tool job.
//!
//! Always available (deferred / discoverable via `tool_search`). The model
//! uses this when it wants to block on a backgrounded tool call instead of
//! waiting for the auto-injection.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::async_jobs::{self, AsyncJobStatus};

const POLL_INTERVAL_MS: u64 = 200;
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

pub async fn tool_job_status(args: &Value) -> Result<String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("job_status: missing required `job_id` parameter"))?;

    let block = args
        .get("block")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);

    let db = async_jobs::get_async_jobs_db()
        .ok_or_else(|| anyhow!("Async jobs DB not initialized"))?;

    let initial = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Unknown job_id: {}", job_id))?;

    if !block || initial.status.is_terminal() {
        return Ok(format_job_response(&initial));
    }

    // Blocking mode: subscribe to event bus + fall back to short polling.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut rx = match crate::get_event_bus() {
        Some(bus) => Some(bus.subscribe()),
        None => None,
    };

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            // Final check, then return whatever we have.
            let job = db
                .load(job_id)?
                .ok_or_else(|| anyhow!("Job {} disappeared during wait", job_id))?;
            return Ok(format_job_response(&job));
        }

        let poll_window =
            std::cmp::min(remaining, std::time::Duration::from_millis(POLL_INTERVAL_MS));
        if let Some(rx_ref) = rx.as_mut() {
            tokio::select! {
                event = rx_ref.recv() => {
                    if let Ok(ev) = event {
                        if ev.name == "async_tool_job:completed" {
                            let evt_id = ev.payload.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
                            if evt_id == job_id {
                                let job = db
                                    .load(job_id)?
                                    .ok_or_else(|| anyhow!("Job {} disappeared after completion", job_id))?;
                                return Ok(format_job_response(&job));
                            }
                        }
                    }
                }
                _ = tokio::time::sleep(poll_window) => {}
            }
        } else {
            tokio::time::sleep(poll_window).await;
        }

        let job = db
            .load(job_id)?
            .ok_or_else(|| anyhow!("Job {} disappeared during wait", job_id))?;
        if job.status.is_terminal() {
            return Ok(format_job_response(&job));
        }
    }
}

fn format_job_response(job: &crate::async_jobs::AsyncJob) -> String {
    let mut payload = json!({
        "job_id": job.job_id,
        "tool": job.tool_name,
        "status": job.status.as_str(),
        "origin": job.origin,
        "created_at": job.created_at,
        "completed_at": job.completed_at,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(d) = job.completed_at {
            map.insert(
                "duration_secs".to_string(),
                json!(d.saturating_sub(job.created_at)),
            );
        }
        match job.status {
            AsyncJobStatus::Completed => {
                if let Some(preview) = &job.result_preview {
                    map.insert("result_preview".to_string(), json!(preview));
                }
                if let Some(path) = &job.result_path {
                    map.insert("result_path".to_string(), json!(path));
                    map.insert(
                        "hint".to_string(),
                        json!("Full result spooled to disk; use the read tool with result_path to load."),
                    );
                }
            }
            AsyncJobStatus::Failed | AsyncJobStatus::TimedOut | AsyncJobStatus::Interrupted => {
                if let Some(err) = &job.error {
                    map.insert("error".to_string(), json!(err));
                }
            }
            AsyncJobStatus::Running => {
                map.insert(
                    "hint".to_string(),
                    json!("Job is still running. Re-call with block=true to wait."),
                );
            }
        }
    }
    payload.to_string()
}

/// Tool definition for `job_status` — registered as deferred so it doesn't
/// pollute the always-loaded tool catalog. Discoverable via `tool_search`.
pub fn get_job_status_tool() -> super::definitions::ToolDefinition {
    super::definitions::ToolDefinition {
        name: super::TOOL_JOB_STATUS.into(),
        description: "Inspect or wait on an async tool job created by `run_in_background: true` \
            or auto-backgrounded by the runtime. Use after the model received a synthetic \
            `{job_id, status: \"started\"}` response from another tool. Set `block: true` to \
            actively wait until the job reaches a terminal state (with `timeout_ms` cap). \
            Without `block`, returns the current snapshot immediately."
            .into(),
        internal: true,
        deferred: false,
        always_load: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job id returned in the synthetic tool response (e.g. 'job_<uuid>')."
                },
                "block": {
                    "type": "boolean",
                    "description": "When true, wait until the job completes (or until timeout_ms). Default false (snapshot)."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Max milliseconds to wait when block=true. Default 60000, max 600000.",
                    "minimum": 0,
                    "maximum": 600000
                }
            },
            "required": ["job_id"],
            "additionalProperties": false
        }),
    }
}

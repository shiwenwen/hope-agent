use anyhow::Result;
use serde_json::Value;

use crate::process_registry::{
    derive_session_name, format_duration_compact, get_registry, now_ms, ProcessStatus,
};

pub(crate) async fn tool_process(args: &Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

    match action {
        "list" => tool_process_list().await,
        "poll" => {
            let session_id = require_session_id(args)?;
            let timeout_ms = args
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(120_000);
            tool_process_poll(&session_id, timeout_ms).await
        }
        "log" => {
            let session_id = require_session_id(args)?;
            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            tool_process_log(&session_id, offset, limit).await
        }
        "write" => {
            let session_id = require_session_id(args)?;
            let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
            tool_process_write(&session_id, data).await
        }
        "kill" => {
            let session_id = require_session_id(args)?;
            tool_process_kill(&session_id).await
        }
        "clear" | "remove" => {
            let session_id = require_session_id(args)?;
            tool_process_remove(&session_id).await
        }
        _ => Err(anyhow::anyhow!("Unknown process action: {}", action)),
    }
}

fn require_session_id(args: &Value) -> Result<String> {
    args.get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("session_id is required for this action"))
}

async fn tool_process_list() -> Result<String> {
    let registry = get_registry().lock().await;
    let mut sessions: Vec<_> = registry.list_all().into_iter().cloned().collect();
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if sessions.is_empty() {
        return Ok("No running or recent sessions.".to_string());
    }

    let now = now_ms();
    let lines: Vec<String> = sessions
        .iter()
        .map(|s| {
            let runtime = now.saturating_sub(s.started_at);
            let name = derive_session_name(&s.command);
            format!(
                "{} {:>9} {:>8} :: {}",
                s.id,
                s.status.to_string(),
                format_duration_compact(runtime),
                name
            )
        })
        .collect();

    Ok(lines.join("\n"))
}

async fn tool_process_poll(session_id: &str, timeout_ms: u64) -> Result<String> {
    // Wait for new output or timeout
    if timeout_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let registry = get_registry().lock().await;
                if let Some(session) = registry.get_session(session_id) {
                    if session.exited
                        || !session.pending_stdout.is_empty()
                        || !session.pending_stderr.is_empty()
                    {
                        break;
                    }
                } else {
                    return Err(anyhow::anyhow!("No session found for {}", session_id));
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    let mut registry = get_registry().lock().await;
    let (stdout, stderr) = registry.drain_output(session_id);

    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    let mut output = String::new();
    if !stdout.is_empty() {
        output.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&stderr);
    }

    if session.exited {
        let exit_info = if let Some(signal) = &session.exit_signal {
            format!("signal {}", signal)
        } else {
            format!("code {}", session.exit_code.unwrap_or(0))
        };

        if output.is_empty() {
            output = format!("(no new output)\n\nProcess exited with {}.", exit_info);
        } else {
            output.push_str(&format!("\n\nProcess exited with {}.", exit_info));
        }
    } else if output.is_empty() {
        output = "(no new output)\n\nProcess still running.".to_string();
    } else {
        output.push_str("\n\nProcess still running.");
    }

    Ok(output)
}

async fn tool_process_log(
    session_id: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String> {
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    let log_text = &session.aggregated_output;
    if log_text.is_empty() {
        return Ok("(no output recorded)".to_string());
    }

    let lines: Vec<&str> = log_text.lines().collect();
    let total = lines.len();
    let default_tail = 200;

    let start = offset.unwrap_or_else(|| total.saturating_sub(limit.unwrap_or(default_tail)));
    let end = limit.map(|l| (start + l).min(total)).unwrap_or(total);

    let slice: String = lines[start..end].join("\n");

    let mut result = if slice.is_empty() {
        "(no output in range)".to_string()
    } else {
        slice
    };

    if offset.is_none() && limit.is_none() && total > default_tail {
        result.push_str(&format!(
            "\n\n[showing last {} of {} lines; pass offset/limit to page]",
            default_tail, total
        ));
    }

    Ok(result)
}

async fn tool_process_write(session_id: &str, _data: &str) -> Result<String> {
    // TODO: Phase 3 will implement stdin writing via PTY/process supervisor
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    if session.exited {
        return Err(anyhow::anyhow!("Session {} has already exited", session_id));
    }

    Ok(format!(
        "Write to stdin is not yet supported in this version. Session {} is still running. Use kill to terminate.",
        session_id
    ))
}

async fn tool_process_kill(session_id: &str) -> Result<String> {
    let mut registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!("No session found for {}", session_id))?;

    if session.exited {
        return Ok(format!("Session {} has already exited.", session_id));
    }

    if let Some(pid) = session.pid {
        // Kill the process and its children (Unix: SIGKILL to pgid;
        // Windows: taskkill /F /T).
        crate::platform::terminate_process_tree(pid);
    }

    registry.mark_exited(
        session_id,
        None,
        Some("SIGKILL".to_string()),
        ProcessStatus::Failed,
    );
    Ok(format!("Terminated session {}.", session_id))
}

async fn tool_process_remove(session_id: &str) -> Result<String> {
    let mut registry = get_registry().lock().await;
    if registry.remove_session(session_id).is_some() {
        Ok(format!("Removed session {}.", session_id))
    } else {
        Err(anyhow::anyhow!("No session found for {}", session_id))
    }
}

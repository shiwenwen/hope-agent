use anyhow::Result;
use serde_json::Value;

use super::ToolExecContext;
use crate::process_registry::{
    derive_session_name, format_duration_compact, get_registry, now_ms, ProcessSession,
};

const NOT_CONTROLLED_MESSAGE: &str =
    "Process session was not found or is not controlled by the current session";

pub(crate) async fn tool_process(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;
    let owner_session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))?;

    match action {
        "list" => tool_process_list(owner_session_id).await,
        "poll" => {
            let session_id = require_session_id(args)?;
            let timeout_ms = args
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(120_000);
            tool_process_poll(&session_id, owner_session_id, timeout_ms).await
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
            tool_process_log(&session_id, owner_session_id, offset, limit).await
        }
        "write" => {
            let session_id = require_session_id(args)?;
            let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
            tool_process_write(&session_id, owner_session_id, data).await
        }
        "kill" => {
            let session_id = require_session_id(args)?;
            tool_process_kill(&session_id, owner_session_id).await
        }
        "clear" | "remove" => {
            let session_id = require_session_id(args)?;
            tool_process_remove(&session_id, owner_session_id).await
        }
        _ => Err(anyhow::anyhow!("Unknown process action: {}", action)),
    }
}

fn process_is_owned_by(session: &ProcessSession, owner_session_id: &str) -> bool {
    session.parent_session_id.as_deref() == Some(owner_session_id)
}

fn ensure_process_owner(session: Option<&ProcessSession>, owner_session_id: &str) -> Result<()> {
    if session.is_some_and(|session| process_is_owned_by(session, owner_session_id)) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))
    }
}

fn require_session_id(args: &Value) -> Result<String> {
    args.get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("session_id is required for this action"))
}

async fn tool_process_list(owner_session_id: &str) -> Result<String> {
    let registry = get_registry().lock().await;
    let mut sessions: Vec<_> = registry
        .list_all()
        .into_iter()
        .filter(|session| process_is_owned_by(session, owner_session_id))
        .cloned()
        .collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));

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

async fn tool_process_poll(
    session_id: &str,
    owner_session_id: &str,
    timeout_ms: u64,
) -> Result<String> {
    // Wait for new output or timeout
    if timeout_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let registry = get_registry().lock().await;
                let session = registry.get_session(session_id);
                ensure_process_owner(session, owner_session_id)?;
                let session = session.expect("owner check requires a session");
                if session.exited
                    || !session.pending_stdout.is_empty()
                    || !session.pending_stderr.is_empty()
                {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    let mut registry = get_registry().lock().await;
    ensure_process_owner(registry.get_session(session_id), owner_session_id)?;
    let (stdout, stderr) = registry.drain_output(session_id);

    let session = registry
        .get_session(session_id)
        .ok_or_else(|| anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))?;

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
        crate::process_notification::mark_observed(session_id);
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
    owner_session_id: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String> {
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .filter(|session| process_is_owned_by(session, owner_session_id))
        .ok_or_else(|| anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))?;

    let log_text = &session.aggregated_output;
    if session.exited {
        crate::process_notification::mark_observed(session_id);
    }
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

async fn tool_process_write(
    session_id: &str,
    owner_session_id: &str,
    _data: &str,
) -> Result<String> {
    // TODO: Phase 3 will implement stdin writing via PTY/process supervisor
    let registry = get_registry().lock().await;
    let session = registry
        .get_session(session_id)
        .filter(|session| process_is_owned_by(session, owner_session_id))
        .ok_or_else(|| anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))?;

    if session.exited {
        return Err(anyhow::anyhow!("Session {} has already exited", session_id));
    }

    Ok(format!(
        "Write to stdin is not yet supported in this version. Session {} is still running. Use kill to terminate.",
        session_id
    ))
}

async fn tool_process_kill(session_id: &str, owner_session_id: &str) -> Result<String> {
    let session = {
        let registry = get_registry().lock().await;
        registry
            .get_session(session_id)
            .filter(|session| process_is_owned_by(session, owner_session_id))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))?
    };

    if session.exited {
        crate::process_notification::mark_observed(session_id);
        return Ok(format!("Session {} has already exited.", session_id));
    }

    let pid = process_pid_for_termination(session_id, &session)?;
    // Request termination of the process and its children (Unix: SIGKILL to
    // pgid; Windows: taskkill /F /T). The platform primitive is void and
    // best-effort, so it is never proof that the target exited.
    crate::blocking::run_blocking(move || crate::platform::terminate_process_tree(pid)).await;

    // Only the exec/PTY/sandbox waiter writes terminal registry truth. A failed
    // or delayed signal therefore remains Running here instead of being hidden
    // behind a fabricated Failed state.
    let observed = {
        let registry = get_registry().lock().await;
        registry
            .get_session(session_id)
            .filter(|session| process_is_owned_by(session, owner_session_id))
            .cloned()
    };
    if observed.as_ref().is_some_and(|session| session.exited) {
        crate::process_notification::mark_observed(session_id);
    }
    Ok(process_kill_response_after_request(
        session_id,
        observed.as_ref(),
    ))
}

fn process_pid_for_termination(session_id: &str, session: &ProcessSession) -> Result<u32> {
    session.pid.ok_or_else(|| {
        anyhow::anyhow!(
            "Termination unavailable for session {}: no process id is available",
            session_id
        )
    })
}

fn process_kill_response_after_request(
    session_id: &str,
    observed: Option<&ProcessSession>,
) -> String {
    match observed {
        Some(session) if session.exited => format!(
            "Termination was requested for session {}; the process waiter now records terminal status {}.",
            session_id, session.status
        ),
        Some(_) => format!(
            "Termination requested for session {}. Exit is not yet confirmed; use process(action=\"poll\", session_id=\"{}\") to observe the real terminal state.",
            session_id, session_id
        ),
        None => format!(
            "Termination was requested for session {}, but the registry row is no longer available; terminal state is unconfirmed.",
            session_id
        ),
    }
}

async fn tool_process_remove(session_id: &str, owner_session_id: &str) -> Result<String> {
    let mut registry = get_registry().lock().await;
    ensure_process_owner(registry.get_session(session_id), owner_session_id)?;
    crate::process_notification::mark_observed(session_id);
    if registry.remove_session(session_id).is_some() {
        Ok(format!("Removed session {}.", session_id))
    } else {
        Err(anyhow::anyhow!(NOT_CONTROLLED_MESSAGE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(owner: Option<&str>) -> ProcessSession {
        ProcessSession {
            id: "process-1".into(),
            parent_session_id: owner.map(str::to_string),
            command: "echo test".into(),
            pid: None,
            cwd: ".".into(),
            started_at: 0,
            exited: false,
            exit_code: None,
            exit_signal: None,
            status: crate::process_registry::ProcessStatus::Running,
            backgrounded: true,
            aggregated_output: String::new(),
            tail: String::new(),
            truncated: false,
            max_output_chars: 1024,
            pending_stdout: String::new(),
            pending_stderr: String::new(),
        }
    }

    #[test]
    fn process_owner_check_is_exact_and_orphans_fail_closed() {
        let owned = session(Some("session-1"));
        let orphan = session(None);
        assert!(process_is_owned_by(&owned, "session-1"));
        assert!(!process_is_owned_by(&owned, "session-2"));
        assert!(!process_is_owned_by(&orphan, "session-1"));
        assert!(ensure_process_owner(Some(&owned), "session-2")
            .unwrap_err()
            .to_string()
            .contains("not found or is not controlled"));
    }

    #[test]
    fn process_kill_pidless_process_cannot_claim_a_termination_request() {
        let process = session(Some("session-1"));
        let error = process_pid_for_termination("process-1", &process)
            .expect_err("no pid means no signal can be sent");
        assert!(error.to_string().contains("Termination unavailable"));
    }

    #[test]
    fn process_kill_response_waits_for_waiter_terminal_truth() {
        let running = session(Some("session-1"));
        let pending = process_kill_response_after_request("process-1", Some(&running));
        assert!(pending.contains("Termination requested"));
        assert!(pending.contains("Exit is not yet confirmed"));
        assert!(!pending.contains("Terminated session"));

        let mut exited = running;
        exited.exited = true;
        exited.status = crate::process_registry::ProcessStatus::Failed;
        let terminal = process_kill_response_after_request("process-1", Some(&exited));
        assert!(terminal.contains("process waiter now records terminal status failed"));
    }
}

//! Process-session live events and completion injection.
//!
//! `process` remains an exec-owned session surface, but background/yielded
//! sessions should still feel alive: stdout/stderr chunks are pushed to the UI
//! via EventBus and terminal exits are injected back into the owning chat as a
//! `<process-notification>` message.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;

use crate::process_registry::{ProcessSession, ProcessStatus};

fn observed_set() -> &'static Mutex<std::collections::HashSet<String>> {
    static OBSERVED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    OBSERVED.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Mark a process result as explicitly observed through the `process` tool.
/// Future notification attempts can skip it.
pub(crate) fn mark_observed(process_id: &str) {
    mark_observed_local(process_id);
    crate::subagent::mark_run_fetched(&process_run_id(process_id));
}

fn mark_observed_local(process_id: &str) {
    let mut guard = observed_set().lock().unwrap_or_else(|p| p.into_inner());
    guard.insert(process_id.to_string());
}

fn was_observed(process_id: &str) -> bool {
    observed_set()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(process_id)
}

pub(crate) fn emit_output(session: &ProcessSession, stream: &str, data: &str) {
    if !session.backgrounded {
        return;
    }
    let Some(bus) = crate::get_event_bus() else {
        return;
    };
    bus.emit(
        "process:output",
        json!({
            "process_id": &session.id,
            "parent_session_id": &session.parent_session_id,
            "stream": stream,
            "chunk": event_chunk(data),
            "truncated": data.chars().count() > EVENT_CHUNK_CHARS,
            "status": session.status.to_string(),
        }),
    );
}

pub(crate) fn on_process_exited(session: ProcessSession) {
    if !session.backgrounded {
        return;
    }
    emit_completed(&session);

    if was_observed(&session.id) {
        app_debug!(
            "process",
            "notification",
            "Process {} already observed; skipping completion injection",
            &session.id
        );
        return;
    }
    let Some(parent_session_id) = session.parent_session_id.clone() else {
        return;
    };
    if crate::session::is_session_incognito(Some(&parent_session_id)) {
        return;
    }

    let Some(session_db) = crate::get_session_db() else {
        app_warn!(
            "process",
            "notification",
            "Session DB not initialized; cannot inject process {} completion",
            &session.id
        );
        return;
    };
    let session_lookup = session_db.get_session(&parent_session_id);
    let parent_agent_id = match session_lookup {
        Ok(Some(row)) => row.agent_id,
        Ok(None) => {
            app_info!(
                "process",
                "notification",
                "Parent session {} gone; skipping process {} notification",
                &parent_session_id,
                &session.id
            );
            return;
        }
        Err(e) => {
            app_warn!(
                "process",
                "notification",
                "Parent session {} lookup failed ({}); proceeding with process {} injection",
                &parent_session_id,
                e,
                &session.id
            );
            crate::agent_loader::DEFAULT_AGENT_ID.to_string()
        }
    };

    let process_id = session.id.clone();
    let run_id = process_run_id(&process_id);
    let push_message = build_process_push_message(&session);
    let child_agent_id =
        crate::subagent::injection::PROCESS_NOTIFICATION_CHILD_AGENT_ID.to_string();
    let db = session_db.clone();

    std::thread::spawn(move || {
        if was_observed(&process_id) {
            return;
        }
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let outcome = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    parent_session_id,
                    parent_agent_id,
                    child_agent_id,
                    run_id,
                    push_message,
                    db,
                    Some(Arc::new(move || mark_observed_local(&process_id))),
                ));
                if matches!(
                    outcome,
                    crate::subagent::injection::InjectionOutcome::Abandoned
                ) {
                    app_warn!(
                        "process",
                        "notification",
                        "Process completion injection abandoned; process result remains available via process(log)"
                    );
                }
            }
            Err(e) => app_error!(
                "process",
                "notification",
                "Failed to build runtime for process injection: {}",
                e
            ),
        }
    });
}

fn process_run_id(process_id: &str) -> String {
    format!("process:{}", process_id)
}

fn emit_completed(session: &ProcessSession) {
    let Some(bus) = crate::get_event_bus() else {
        return;
    };
    bus.emit(
        "process:completed",
        json!({
            "process_id": &session.id,
            "parent_session_id": &session.parent_session_id,
            "status": session.status.to_string(),
            "exit_code": session.exit_code,
            "exit_signal": &session.exit_signal,
            "tail": event_chunk(&session.tail),
            "truncated": session.truncated,
        }),
    );
}

fn build_process_push_message(session: &ProcessSession) -> String {
    let status = session.status.to_string();
    let exit_detail = if let Some(signal) = &session.exit_signal {
        format!("signal {}", signal)
    } else {
        format!("code {}", session.exit_code.unwrap_or_default())
    };
    let summary = match session.status {
        ProcessStatus::Completed => format!("Exec process completed with {exit_detail}."),
        ProcessStatus::Failed => {
            format!("Exec process failed or was terminated with {exit_detail}.")
        }
        ProcessStatus::Running => "Exec process is still running.".to_string(),
    };
    let output_tail = if session.tail.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<output-tail>\n{}\n</output-tail>\n",
            escape_xml_text(&session.tail)
        )
    };
    let exit_code = session
        .exit_code
        .map(|code| format!("<exit-code>{}</exit-code>\n", code))
        .unwrap_or_default();
    let exit_signal = session
        .exit_signal
        .as_ref()
        .map(|signal| format!("<exit-signal>{}</exit-signal>\n", escape_xml_text(signal)))
        .unwrap_or_default();
    format!(
        "<process-notification>\n\
         <process-id>{}</process-id>\n\
         <command>{}</command>\n\
         <status>{}</status>\n\
         {exit_code}\
         {exit_signal}\
         <truncated>{}</truncated>\n\
         {output_tail}\
         <summary>{}</summary>\n\
         </process-notification>",
        escape_xml_text(&session.id),
        escape_xml_text(&session.command),
        escape_xml_text(&status),
        session.truncated,
        escape_xml_text(&summary)
    )
}

const EVENT_CHUNK_CHARS: usize = 16 * 1024;

fn event_chunk(input: &str) -> String {
    if input.chars().count() <= EVENT_CHUNK_CHARS {
        return input.to_string();
    }
    let idx = input
        .char_indices()
        .nth(EVENT_CHUNK_CHARS)
        .map(|(idx, _)| idx)
        .unwrap_or(input.len());
    input[..idx].to_string()
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_push_message_contains_terminal_fields() {
        let session = ProcessSession {
            id: "abc123".into(),
            parent_session_id: Some("parent".into()),
            command: "printf '<ok>'".into(),
            pid: None,
            cwd: "/tmp".into(),
            started_at: 0,
            exited: true,
            exit_code: Some(0),
            exit_signal: None,
            status: ProcessStatus::Completed,
            backgrounded: true,
            aggregated_output: "<ok>".into(),
            tail: "<ok>".into(),
            truncated: false,
            max_output_chars: 1000,
            pending_stdout: String::new(),
            pending_stderr: String::new(),
        };

        let msg = build_process_push_message(&session);
        assert!(msg.starts_with("<process-notification>"));
        assert!(msg.contains("<process-id>abc123</process-id>"));
        assert!(msg.contains("<status>completed</status>"));
        assert!(msg.contains("<exit-code>0</exit-code>"));
        assert!(msg.contains("&lt;ok&gt;"));
    }
}

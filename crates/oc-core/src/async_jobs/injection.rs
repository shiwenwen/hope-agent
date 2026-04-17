//! Bridge from finished async tool jobs back into the parent chat session.
//!
//! Reuses the subagent injection pipeline (`subagent::injection::inject_and_run_parent`)
//! by formatting the tool job result as a push message and passing the job id
//! as the `run_id` parameter — this lets us share the idle-wait, cancellation,
//! and retry machinery with no duplication.

use super::types::AsyncJobStatus;

/// Dispatch a tool-job completion injection in the background.
///
/// Falls back to a no-op (logs an error) if the SessionDB is missing.
pub fn dispatch_injection(
    session_id: String,
    parent_agent_id: Option<String>,
    job_id: String,
    tool_name: String,
    status: AsyncJobStatus,
    result_preview: Option<String>,
    error: Option<String>,
) {
    let session_db = match crate::get_session_db() {
        Some(db) => db.clone(),
        None => {
            app_warn!(
                "async_jobs",
                "injection",
                "Session DB not initialized; cannot inject job {}",
                &job_id
            );
            return;
        }
    };

    // Resolve the parent agent id from the session row when not supplied.
    let parent_agent_id = match parent_agent_id {
        Some(id) => id,
        None => session_db
            .get_session(&session_id)
            .ok()
            .flatten()
            .map(|s| s.agent_id)
            .unwrap_or_else(|| "default".to_string()),
    };

    let push_message = build_tool_job_push_message(
        &job_id,
        &tool_name,
        status,
        result_preview.as_deref(),
        error.as_deref(),
    );
    // The subagent injection pipeline expects a `child_agent_id` label — we
    // tag tool jobs with `tool_job:<name>` so frontends can distinguish them
    // from real subagent runs.
    let child_agent_id = format!("tool_job:{}", tool_name);
    let job_id_for_db = job_id.clone();
    let db_clone = session_db.clone();

    std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    session_id,
                    parent_agent_id,
                    child_agent_id,
                    job_id,
                    push_message,
                    db_clone,
                ));
                mark_injected_with_retry(&job_id_for_db);
            }
            Err(e) => app_error!(
                "async_jobs",
                "injection",
                "Failed to build runtime for injection: {}",
                e
            ),
        }
    });
}

/// Retry `mark_injected` with exponential backoff. If all retries fail,
/// log an error and emit an EventBus alarm — the row will be replayed on
/// the next `replay_pending_jobs()` sweep, creating a duplicate
/// `<tool-job-result>` injection, so surfacing the failure matters.
fn mark_injected_with_retry(job_id: &str) {
    const BACKOFFS_MS: &[u64] = &[0, 100, 500, 2_000];
    let Some(jdb) = crate::async_jobs::get_async_jobs_db() else {
        app_error!(
            "async_jobs",
            "injection",
            "Cannot mark job {} injected: async_jobs DB is not initialized",
            job_id
        );
        return;
    };
    let mut last_err: Option<String> = None;
    for (attempt, delay_ms) in BACKOFFS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
        }
        match jdb.mark_injected(job_id) {
            Ok(()) => return,
            Err(e) => {
                last_err = Some(e.to_string());
                app_warn!(
                    "async_jobs",
                    "injection",
                    "mark_injected({}) attempt {} failed: {}",
                    job_id,
                    attempt + 1,
                    e
                );
            }
        }
    }
    let err = last_err.unwrap_or_else(|| "unknown".to_string());
    app_error!(
        "async_jobs",
        "injection",
        "mark_injected({}) failed after {} attempts: {} — job may be re-injected on restart",
        job_id,
        BACKOFFS_MS.len(),
        &err
    );
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "async_tool_job:mark_injected_failed",
            serde_json::json!({
                "job_id": job_id,
                "error": err,
            }),
        );
    }
}

/// Format the user-visible message that gets injected back into the parent
/// session when a tool job completes. The LLM correlates this with the
/// original synthetic response via the `job-id` attribute.
pub fn build_tool_job_push_message(
    job_id: &str,
    tool_name: &str,
    status: AsyncJobStatus,
    result_preview: Option<&str>,
    error: Option<&str>,
) -> String {
    let body = match status {
        AsyncJobStatus::Completed => {
            let output = result_preview.unwrap_or("(empty result)");
            format!(
                "<tool-job-result job-id=\"{job_id}\" tool=\"{tool_name}\" status=\"completed\">\n\
                 <output>\n{output}\n</output>\n\
                 </tool-job-result>"
            )
        }
        AsyncJobStatus::Failed => {
            let err = error.unwrap_or("(unknown error)");
            format!(
                "<tool-job-result job-id=\"{job_id}\" tool=\"{tool_name}\" status=\"failed\">\n\
                 <error>{err}</error>\n\
                 </tool-job-result>"
            )
        }
        AsyncJobStatus::TimedOut => {
            let err = error.unwrap_or("exceeded max_job_secs");
            format!(
                "<tool-job-result job-id=\"{job_id}\" tool=\"{tool_name}\" status=\"timed_out\">\n\
                 <error>{err}</error>\n\
                 </tool-job-result>"
            )
        }
        AsyncJobStatus::Interrupted => {
            format!(
                "<tool-job-result job-id=\"{job_id}\" tool=\"{tool_name}\" status=\"interrupted\">\n\
                 <error>Job was running when the application restarted; result is unrecoverable.</error>\n\
                 </tool-job-result>"
            )
        }
        AsyncJobStatus::Running => {
            format!(
                "<tool-job-result job-id=\"{job_id}\" tool=\"{tool_name}\" status=\"running\">\n\
                 <note>Still running — call job_status to check.</note>\n\
                 </tool-job-result>"
            )
        }
    };
    format!("[Tool Job Completion — auto-delivered]\n{body}")
}

//! Bridge from finished async tool jobs back into the parent chat session.
//!
//! Reuses the subagent injection pipeline (`subagent::injection::inject_and_run_parent`)
//! by formatting the tool job notification as a push message and passing the job id
//! as the `run_id` parameter — this lets us share the idle-wait, cancellation,
//! and retry machinery with no duplication.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use super::types::JobStatus;

/// In-flight dispatch set. A job_id present here means another task in this
/// process has already called `dispatch_injection` for it and is either still
/// running injection or waiting for `mark_injected` to commit. Entries are
/// removed unconditionally once the dispatch thread exits, so a crashed or
/// cancelled dispatch won't pin the job forever — it'll be retried on the
/// next `replay_pending_jobs()` sweep.
fn dispatching_set() -> &'static Mutex<HashSet<String>> {
    static DISPATCHING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DISPATCHING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Try to claim a dispatch slot for `job_id`. Returns `false` if another
/// dispatch is already in flight for this job in the current process,
/// preventing `list_pending_injection()` + event-driven retries from racing
/// into a double-injection. Cross-process races (desktop + server hitting
/// the same `background_jobs.db`) still require a DB-level claim; that's tracked
/// separately.
fn try_claim_dispatch(job_id: &str) -> bool {
    let mut guard = dispatching_set().lock().unwrap_or_else(|p| p.into_inner());
    guard.insert(job_id.to_string())
}

fn release_dispatch(job_id: &str) {
    let mut guard = dispatching_set().lock().unwrap_or_else(|p| p.into_inner());
    guard.remove(job_id);
}

// ── Completion merge window (R4) ───────────────────────────────────────────
//
// When several background jobs in the SAME session finish close together (the
// common "fire 5 `run_in_background` at once" case), injecting each separately
// would burn N billed turns. Instead we buffer terminal completions per session
// for a short window (`async_tools.completion_merge_window_secs`, default 3s)
// and fire ONE merged injection listing every task. The first completion opens
// the window (one timer thread); everything that settles before it elapses joins
// the batch; the flush atomically drains the buffer so a later completion starts
// a fresh window. This is a pure in-memory live-path optimization: if the
// process dies mid-window the rows are terminal-but-uninjected and
// `replay_pending_jobs()` re-dispatches each on the next start (no merge, no
// loss). A `Group` (R5) is the pre-merged special case — it bypasses this
// entirely via its own single injection.

/// A terminal tool-job completion waiting in the merge window. Carries exactly
/// the fields [`dispatch_injection`] needs (session id is the buffer key).
struct PendingJobInjection {
    parent_agent_id: Option<String>,
    job_id: String,
    tool_name: String,
    tool_call_id: Option<String>,
    status: JobStatus,
    result_preview: Option<String>,
    result_path: Option<String>,
    error: Option<String>,
}

/// Per-session merge buffer. A session id present here has an open window with a
/// timer thread that will `flush_merge_buffer` it.
fn merge_buffers() -> &'static Mutex<HashMap<String, Vec<PendingJobInjection>>> {
    static M: OnceLock<Mutex<HashMap<String, Vec<PendingJobInjection>>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Live-path completion entry point (R4): buffer this job's injection for the
/// merge window, or — when merging is disabled (`window == 0`) — inject it
/// immediately. The first job in an empty buffer opens the window + starts the
/// timer; subsequent jobs just join. Replaces the direct `dispatch_injection`
/// call in `finalize_job`. Startup replay still calls `dispatch_injection`
/// directly (each un-injected row is independent — no live window to join).
#[allow(clippy::too_many_arguments)]
pub fn enqueue_injection(
    session_id: String,
    parent_agent_id: Option<String>,
    job_id: String,
    tool_name: String,
    tool_call_id: Option<String>,
    status: JobStatus,
    result_preview: Option<String>,
    result_path: Option<String>,
    error: Option<String>,
) {
    let window_secs = crate::config::cached_config()
        .async_tools
        .completion_merge_window_secs;
    if window_secs == 0 {
        // Merging disabled — preserve the legacy immediate-injection behavior.
        dispatch_injection(
            session_id,
            parent_agent_id,
            job_id,
            tool_name,
            tool_call_id,
            status,
            result_preview,
            result_path,
            error,
        );
        return;
    }

    let pending = PendingJobInjection {
        parent_agent_id,
        job_id,
        tool_name,
        tool_call_id,
        status,
        result_preview,
        result_path,
        error,
    };
    let start_timer = {
        let mut buffers = merge_buffers().lock().unwrap_or_else(|p| p.into_inner());
        let entry = buffers.entry(session_id.clone()).or_default();
        let was_empty = entry.is_empty();
        entry.push(pending);
        was_empty
    };
    if start_timer {
        let sid = session_id;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(window_secs));
            flush_merge_buffer(&sid);
        });
    }
}

/// Drain a session's merge buffer and inject: a single job goes through the
/// normal single-`<task-notification>` path; multiple go through one merged
/// `<task-notification-batch>` injection. Atomically removes the buffer so a
/// completion arriving after the drain opens a fresh window.
fn flush_merge_buffer(session_id: &str) {
    let jobs = {
        let mut buffers = merge_buffers().lock().unwrap_or_else(|p| p.into_inner());
        match buffers.remove(session_id) {
            Some(jobs) if !jobs.is_empty() => jobs,
            _ => return,
        }
    };
    if jobs.len() == 1 {
        let j = jobs.into_iter().next().expect("len == 1 checked");
        dispatch_injection(
            session_id.to_string(),
            j.parent_agent_id,
            j.job_id,
            j.tool_name,
            j.tool_call_id,
            j.status,
            j.result_preview,
            j.result_path,
            j.error,
        );
        return;
    }
    dispatch_merged_injection(session_id.to_string(), jobs);
}

/// Fire ONE merged injection for several jobs that finished in the same window.
/// Mirrors [`dispatch_injection`]'s ghost-turn gate + per-process dedup, but
/// over a batch: claims each job's dispatch slot, builds one
/// `<task-notification-batch>`, and marks every claimed row injected on the
/// single terminal landing (each callback fires exactly once, per gotcha I7).
fn dispatch_merged_injection(session_id: String, jobs: Vec<PendingJobInjection>) {
    let session_db = match crate::get_session_db() {
        Some(db) => db.clone(),
        None => {
            app_warn!(
                "async_jobs",
                "injection",
                "Session DB not initialized; cannot inject merged batch of {} jobs for session {}",
                jobs.len(),
                &session_id
            );
            return;
        }
    };

    let session_lookup = session_db.get_session(&session_id);
    let parent_agent_id = jobs
        .iter()
        .find_map(|j| j.parent_agent_id.clone())
        .or_else(|| {
            session_lookup
                .as_ref()
                .ok()
                .and_then(|row| row.as_ref())
                .map(|s| s.agent_id.clone())
        })
        .unwrap_or_else(|| crate::agent_loader::DEFAULT_AGENT_ID.to_string());

    // Ghost-turn gate (mirrors dispatch_injection): a deleted/burned parent
    // would resurrect a billed turn. Mark every row injected so replay stops
    // retrying a dead session, then skip.
    match session_lookup {
        Ok(Some(_)) => {}
        Ok(None) => {
            app_info!(
                "async_jobs",
                "injection",
                "Parent session {} gone; marking {} merged jobs injected and skipping ghost turn",
                &session_id,
                jobs.len()
            );
            for j in &jobs {
                mark_injected_with_retry(&j.job_id);
            }
            return;
        }
        Err(e) => {
            app_warn!(
                "async_jobs",
                "injection",
                "Parent session {} lookup failed ({}); proceeding with merged inject — backstop will re-check",
                &session_id,
                e
            );
        }
    }

    // Per-process dedup: claim each job. A job already in-flight (a racing
    // startup replay) is dropped from this batch, not double-injected.
    let mut claimed: Vec<PendingJobInjection> = Vec::with_capacity(jobs.len());
    for j in jobs {
        if try_claim_dispatch(&j.job_id) {
            claimed.push(j);
        } else {
            app_debug!(
                "async_jobs",
                "injection",
                "Job {} already has an in-flight dispatch; dropping from merged batch",
                &j.job_id
            );
        }
    }
    if claimed.is_empty() {
        return;
    }
    // A single survivor degrades to the normal single-job message (no batch
    // envelope for one task). Release its claim first so dispatch_injection can
    // re-claim it through its own path.
    if claimed.len() == 1 {
        let j = claimed.into_iter().next().expect("len == 1 checked");
        release_dispatch(&j.job_id);
        dispatch_injection(
            session_id,
            j.parent_agent_id,
            j.job_id,
            j.tool_name,
            j.tool_call_id,
            j.status,
            j.result_preview,
            j.result_path,
            j.error,
        );
        return;
    }

    let push_message = build_merged_push_message(&claimed);
    let claimed_ids: Vec<String> = claimed.iter().map(|j| j.job_id.clone()).collect();
    // Synthetic batch run id for the injection pipeline. Tool job ids are never
    // in FETCHED_RUN_IDS (only subagents are marked fetched), so this never
    // collides with the fetch-skip path.
    let run_id = format!("batch:{}", claimed_ids.first().cloned().unwrap_or_default());
    let child_agent_id = "tool_job:batch".to_string();
    let claimed_count = claimed_ids.len();
    let ids_for_release = claimed_ids.clone();
    let ids_for_injected = claimed_ids;

    std::thread::spawn(move || {
        struct DispatchGuard(Vec<String>);
        impl Drop for DispatchGuard {
            fn drop(&mut self) {
                for id in &self.0 {
                    release_dispatch(id);
                }
            }
        }
        let _guard = DispatchGuard(ids_for_release);

        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let on_injected: crate::subagent::injection::OnInjected = {
                    let ids = ids_for_injected;
                    Arc::new(move || {
                        for id in &ids {
                            mark_injected_with_retry(id);
                        }
                    })
                };
                let outcome = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    session_id,
                    parent_agent_id,
                    child_agent_id,
                    run_id,
                    push_message,
                    session_db,
                    Some(on_injected),
                ));
                if matches!(
                    outcome,
                    crate::subagent::injection::InjectionOutcome::Abandoned
                ) {
                    app_warn!(
                        "async_jobs",
                        "injection",
                        "Merged injection abandoned (parent never went idle); {} jobs left pending for restart replay",
                        claimed_count
                    );
                }
            }
            Err(e) => app_error!(
                "async_jobs",
                "injection",
                "Failed to build runtime for merged injection: {}",
                e
            ),
        }
    });
}

/// Dispatch a tool-job completion injection in the background.
///
/// Falls back to a no-op (logs an error) if the SessionDB is missing.
pub fn dispatch_injection(
    session_id: String,
    parent_agent_id: Option<String>,
    job_id: String,
    tool_name: String,
    tool_call_id: Option<String>,
    status: JobStatus,
    result_preview: Option<String>,
    result_path: Option<String>,
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

    // Resolve the session row once — it backs both the agent-id fallback and
    // the ghost-turn gate below, so one lookup serves both.
    let session_lookup = session_db.get_session(&session_id);

    // Resolve the parent agent id from the session row when not supplied.
    let parent_agent_id = match parent_agent_id {
        Some(id) => id,
        None => session_lookup
            .as_ref()
            .ok()
            .and_then(|row| row.as_ref())
            .map(|s| s.agent_id.clone())
            .unwrap_or_else(|| crate::agent_loader::DEFAULT_AGENT_ID.to_string()),
    };

    // E2 / DELETE-3 / INCOG-3: the parent session can be deleted or burned
    // (incognito close) after the job started. Injecting into a gone session
    // would resurrect a *ghost turn* — append a user row and run a billed LLM
    // turn against a session that no longer exists. Gate it before spawning the
    // injection thread.
    match session_lookup {
        // Alive — proceed to inject as normal.
        Ok(Some(_)) => {}
        // Row genuinely gone: mark the job injected so `replay_pending_jobs()`
        // won't keep retrying a dead session forever, then skip.
        Ok(None) => {
            app_info!(
                "async_jobs",
                "injection",
                "Parent session {} gone (deleted/burned); marking job {} injected and skipping ghost turn",
                &session_id,
                &job_id
            );
            mark_injected_with_retry(&job_id);
            return;
        }
        // Transient lookup failure: don't drop a real job on a momentary glitch.
        // Proceed — `inject_and_run_parent` re-checks existence as a backstop,
        // and an idle timeout there leaves the row un-injected for restart replay.
        Err(e) => {
            app_warn!(
                "async_jobs",
                "injection",
                "Parent session {} lookup failed ({}); proceeding — inject backstop will re-check",
                &session_id,
                e
            );
        }
    }

    // Deduplicate in-flight dispatches inside this process. Replay on startup
    // + a late EventBus retry for the same terminal job could otherwise fire
    // two threads racing the same injection.
    if !try_claim_dispatch(&job_id) {
        app_debug!(
            "async_jobs",
            "injection",
            "Job {} already has an in-flight dispatch; skipping duplicate",
            &job_id
        );
        return;
    }

    let push_message = build_tool_job_push_message(
        &job_id,
        &tool_name,
        tool_call_id.as_deref(),
        status,
        result_preview.as_deref(),
        result_path.as_deref(),
        error.as_deref(),
    );
    // The subagent injection pipeline expects a `child_agent_id` label — we
    // tag tool jobs with `tool_job:<name>` so frontends can distinguish them
    // from real subagent runs.
    let child_agent_id = format!("tool_job:{}", tool_name);
    let job_id_for_db = job_id.clone();
    let job_id_for_release = job_id.clone();
    let db_clone = session_db.clone();

    std::thread::spawn(move || {
        // Ensure the dispatch slot is released no matter how we exit
        // (success, panic-free error, or runtime build failure).
        struct DispatchGuard(String);
        impl Drop for DispatchGuard {
            fn drop(&mut self) {
                release_dispatch(&self.0);
            }
        }
        let _guard = DispatchGuard(job_id_for_release);

        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                // I7: hand the mark-injected step to the injection pipeline as a
                // callback so it fires only at the real terminal landing — even
                // when the attempt is deferred and re-queued (the callback rides
                // the PendingInjection through flush). An idle-timeout returns
                // `Abandoned` WITHOUT firing it, so the row stays un-injected and
                // `replay_pending_jobs()` retries it on the next restart
                // (MISC-15: an abandoned injection must not look delivered).
                let on_injected: crate::subagent::injection::OnInjected = {
                    let jid = job_id_for_db.clone();
                    Arc::new(move || mark_injected_with_retry(&jid))
                };
                let outcome = rt.block_on(crate::subagent::injection::inject_and_run_parent(
                    session_id,
                    parent_agent_id,
                    child_agent_id,
                    job_id,
                    push_message,
                    db_clone,
                    Some(on_injected),
                ));
                if matches!(
                    outcome,
                    crate::subagent::injection::InjectionOutcome::Abandoned
                ) {
                    app_warn!(
                        "async_jobs",
                        "injection",
                        "Injection for job {} abandoned (parent never went idle); left pending for restart replay",
                        &job_id_for_db
                    );
                }
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
/// `<task-notification>` injection, so surfacing the failure matters.
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
    super::events::emit_mark_injected_failed(job_id, &err);
}

/// Format the user-visible message that gets injected back into the parent
/// session when a tool job completes. The LLM correlates this with the
/// original synthetic response via `task-id`, and reads `output-file` when it
/// needs the detailed output.
pub fn build_tool_job_push_message(
    job_id: &str,
    tool_name: &str,
    tool_call_id: Option<&str>,
    status: JobStatus,
    result_preview: Option<&str>,
    result_path: Option<&str>,
    error: Option<&str>,
) -> String {
    let output_file = result_path
        .map(|path| format!("<output-file>{}</output-file>\n", escape_xml_text(path)))
        .unwrap_or_default();
    let tool_use_id = tool_call_id
        .filter(|id| !id.trim().is_empty())
        .map(|id| format!("<tool-use-id>{}</tool-use-id>\n", escape_xml_text(id)))
        .unwrap_or_default();
    let (clean_preview, media_items) = result_preview
        .map(crate::agent::extract_media_items)
        .unwrap_or_else(|| (String::new(), Vec::new()));
    let media_block = if media_items.is_empty() {
        String::new()
    } else {
        let json = serde_json::to_string(&media_items).unwrap_or_else(|_| "[]".to_string());
        format!(
            "<media-items-json>{}</media-items-json>\n",
            escape_xml_text(&json)
        )
    };
    let error_block = error
        .map(|err| format!("<error>{}</error>\n", escape_xml_text(err)))
        .unwrap_or_default();
    let preview_block =
        if status == JobStatus::Completed && result_path.is_none() && !clean_preview.is_empty() {
            format!(
                "<output-preview>\n{}\n</output-preview>\n",
                escape_xml_text(&clean_preview)
            )
        } else {
            String::new()
        };
    let summary = match status {
        JobStatus::Completed => {
            if result_path.is_some() {
                format!(
                    "Async tool \"{tool_name}\" completed; full output is saved in output-file."
                )
            } else {
                format!("Async tool \"{tool_name}\" completed; output file is unavailable. See output-preview.")
            }
        }
        JobStatus::Failed => {
            let err = error.unwrap_or("(unknown error)");
            format!("Async tool \"{tool_name}\" failed: {err}")
        }
        JobStatus::TimedOut => {
            let err = error.unwrap_or("exceeded max_job_secs");
            format!("Async tool \"{tool_name}\" timed out: {err}")
        }
        JobStatus::Cancelled => {
            let err = error.unwrap_or("Job was cancelled.");
            format!("Async tool \"{tool_name}\" was cancelled: {err}")
        }
        JobStatus::Interrupted => {
            format!("Async tool \"{tool_name}\" was interrupted by application restart.")
        }
        JobStatus::Running => {
            format!("Async tool \"{tool_name}\" is still running; wait for the terminal notification, or use job_status only for an occasional status snapshot.")
        }
        JobStatus::Cancelling => {
            format!("Async tool \"{tool_name}\" is cancelling; wait for terminal notification.")
        }
        JobStatus::AwaitingApproval => {
            // Non-terminal: never finalized, so it shouldn't reach the
            // injection path. Defensive arm to keep the match exhaustive.
            format!("Async tool \"{tool_name}\" is awaiting a human approval decision.")
        }
        JobStatus::Queued => {
            // Non-terminal: a queued job is never finalized, so it shouldn't
            // reach the injection path. Defensive arm to keep the match exhaustive.
            format!("Async tool \"{tool_name}\" is queued, waiting for a free concurrency slot.")
        }
    };
    format!(
        "<task-notification>\n\
         <task-id>{}</task-id>\n\
         {tool_use_id}\
         <tool>{}</tool>\n\
         <status>{}</status>\n\
         {output_file}\
         {media_block}\
         {preview_block}\
         {error_block}\
         <summary>{}</summary>\n\
         </task-notification>",
        escape_xml_text(job_id),
        escape_xml_text(tool_name),
        escape_xml_text(status.as_str()),
        escape_xml_text(&summary)
    )
}

/// Merge several completed jobs' notifications into ONE injected message (R4).
/// Wraps each job's standard `<task-notification>` block in a
/// `<task-notification-batch>` envelope carrying aggregate counts, so the LLM
/// sees every task-id in one turn and the frontend can render a "N tasks" pill.
fn build_merged_push_message(jobs: &[PendingJobInjection]) -> String {
    let count = jobs.len();
    let completed = jobs
        .iter()
        .filter(|j| j.status == JobStatus::Completed)
        .count();
    let failed = count.saturating_sub(completed);
    let blocks: Vec<String> = jobs
        .iter()
        .map(|j| {
            build_tool_job_push_message(
                &j.job_id,
                &j.tool_name,
                j.tool_call_id.as_deref(),
                j.status,
                j.result_preview.as_deref(),
                j.result_path.as_deref(),
                j.error.as_deref(),
            )
        })
        .collect();
    format!(
        "<task-notification-batch count=\"{count}\" completed=\"{completed}\" failed=\"{failed}\">\n\
         {}\n\
         </task-notification-batch>",
        blocks.join("\n")
    )
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

    fn pending(job_id: &str, status: JobStatus) -> PendingJobInjection {
        PendingJobInjection {
            parent_agent_id: None,
            job_id: job_id.to_string(),
            tool_name: "exec".to_string(),
            tool_call_id: None,
            status,
            result_preview: Some("ok".to_string()),
            result_path: None,
            error: if status == JobStatus::Completed {
                None
            } else {
                Some("boom".to_string())
            },
        }
    }

    #[test]
    fn merged_message_wraps_every_task_with_aggregate_counts() {
        let jobs = vec![
            pending("job-a", JobStatus::Completed),
            pending("job-b", JobStatus::Failed),
            pending("job-c", JobStatus::Completed),
        ];
        let msg = build_merged_push_message(&jobs);
        assert!(msg.starts_with("<task-notification-batch count=\"3\" completed=\"2\" failed=\"1\">"));
        assert!(msg.trim_end().ends_with("</task-notification-batch>"));
        // Every task-id is present so the LLM can correlate each background job.
        for id in ["job-a", "job-b", "job-c"] {
            assert!(
                msg.contains(&format!("<task-id>{id}</task-id>")),
                "merged message missing {id}"
            );
        }
        // Three inner notifications, one per job.
        assert_eq!(msg.matches("<task-notification>").count(), 3);
        // The failure carries its error through into its block.
        assert!(msg.contains("<error>boom</error>"));
    }
}

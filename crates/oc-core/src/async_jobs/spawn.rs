use anyhow::Result;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use super::db::AsyncJobsDB;
use super::injection;
use super::types::{AsyncJob, AsyncJobStatus, JobOrigin};
use crate::tools::ToolExecContext;

const DEFAULT_PREVIEW_BYTES: usize = 4096;

/// Generate a stable, short-prefix job id.
pub fn new_job_id() -> String {
    format!("job_{}", uuid::Uuid::new_v4().simple())
}

/// Persist a freshly spawned job row in `running` state. Returns the job id.
pub fn record_running_job(
    db: &AsyncJobsDB,
    job_id: &str,
    ctx: &ToolExecContext,
    tool_name: &str,
    args: &Value,
    origin: JobOrigin,
) -> Result<()> {
    let job = AsyncJob {
        job_id: job_id.to_string(),
        session_id: ctx.session_id.clone(),
        agent_id: ctx.agent_id.clone(),
        tool_name: tool_name.to_string(),
        tool_call_id: None,
        args_json: serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
        status: AsyncJobStatus::Running,
        result_preview: None,
        result_path: None,
        error: None,
        created_at: now_secs(),
        completed_at: None,
        injected: false,
        origin: origin.as_str().to_string(),
    };
    db.insert(&job)
}

/// Build the synthetic tool result string returned to the LLM when a tool
/// call is detached into the background. The model receives a job id it can
/// later poll via `job_status` (or wait for auto-injection).
pub fn synthetic_started_result(job_id: &str, tool_name: &str, origin: JobOrigin) -> String {
    let hint = match origin {
        JobOrigin::Explicit | JobOrigin::PolicyForced => {
            "The tool is running in the background. Continue with other work; the result will \
             be auto-injected as a `<tool-job-result>` user message when ready. To actively \
             wait, call `job_status` with `block: true`."
        }
        JobOrigin::AutoBackgrounded => {
            "The tool exceeded the synchronous time budget and was auto-backgrounded. The \
             result will be auto-injected when ready, or you can call `job_status` to wait \
             for it explicitly."
        }
    };
    json!({
        "job_id": job_id,
        "status": "started",
        "tool": tool_name,
        "origin": origin.as_str(),
        "hint": hint,
    })
    .to_string()
}

/// Public API: spawn a background tool job.
///
/// Used by the explicit `run_in_background: true` and policy `always-background`
/// paths. The actual tool dispatch runs on a separate OS thread + current-thread
/// runtime to avoid the `Send` requirement on the tool's future, mirroring the
/// approach used by `subagent::injection::inject_and_run_parent`.
pub fn spawn_explicit_job(
    tool_name: &str,
    args: Value,
    mut ctx: ToolExecContext,
    origin: JobOrigin,
) -> Result<String> {
    let db = match super::get_async_jobs_db() {
        Some(db) => db.clone(),
        None => {
            return Err(anyhow::anyhow!(
                "Async jobs DB not initialized; cannot background tool '{}'",
                tool_name
            ));
        }
    };

    let job_id = new_job_id();
    record_running_job(&db, &job_id, &ctx, tool_name, &args, origin)?;

    let synthetic = synthetic_started_result(&job_id, tool_name, origin);

    // Strip `run_in_background` from args AND set bypass on the ctx so the
    // recursive `execute_tool_with_context` call inside the OS thread runtime
    // goes straight to the sync dispatch path. Without bypass the
    // `AlwaysBackground` policy would re-enter `spawn_explicit_job` forever.
    let mut clean_args = args;
    if let Some(obj) = clean_args.as_object_mut() {
        obj.remove("run_in_background");
    }
    ctx.bypass_async_dispatch = true;
    // The outer call already passed the approval gate for this exact arg set;
    // the recursive inner call must not re-prompt (the user has no surface to
    // answer it from inside a background runtime). The visibility / plan-mode
    // checks still re-run as belt-and-suspenders.
    ctx.auto_approve_tools = true;
    let max_secs = crate::config::cached_config().async_tools.max_job_secs;
    let preview_bytes = preview_byte_budget();
    let tool_name_owned = tool_name.to_string();
    let job_id_owned = job_id.clone();

    // Run on a dedicated OS thread so we don't constrain the dispatch future
    // to be `Send`. This mirrors `subagent::injection::inject_and_run_parent`.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                app_error!(
                    "async_jobs",
                    "spawn",
                    "Failed to build runtime for job {}: {}",
                    &job_id_owned,
                    e
                );
                let _ = db.update_terminal(
                    &job_id_owned,
                    AsyncJobStatus::Failed,
                    None,
                    None,
                    Some(&format!("runtime build failed: {}", e)),
                    now_secs(),
                );
                super::wait::notify_completion(&job_id_owned);
                emit_completion_event(&job_id_owned, &tool_name_owned, "failed");
                return;
            }
        };
        rt.block_on(async move {
            run_job_to_completion(
                db,
                job_id_owned,
                tool_name_owned,
                clean_args,
                ctx,
                max_secs,
                preview_bytes,
            )
            .await;
        });
    });

    Ok(synthetic)
}

/// Run an async-capable tool synchronously, but transfer it to a background
/// job if it exceeds `auto_bg_secs`. This is the third decision-tier
/// described in `agile-stirring-fountain.md`: when the model didn't request
/// `run_in_background`, we still race the dispatch against a budget so the
/// chat doesn't stall on accidentally-long tool calls.
///
/// The dispatch always runs on a dedicated OS thread so we don't need to
/// constrain the underlying tool future to be `Send`. Coordination with the
/// main thread uses an explicit phase machine to avoid the race window
/// between "OS thread finished" and "main thread already gave up."
pub async fn dispatch_with_auto_background(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    auto_bg_secs: u64,
) -> Result<String> {
    let phase = Arc::new(Mutex::new(Phase::Pending));
    let notify = Arc::new(tokio::sync::Notify::new());

    // Pre-allocate a job id so that, if we end up detaching, the OS thread
    // can later finalize it through the same path used by explicit jobs.
    let job_id = new_job_id();

    let phase_w = phase.clone();
    let notify_w = notify.clone();
    let job_id_w = job_id.clone();
    let name_w = name.to_string();
    let args_w = args.clone();
    let ctx_w = ctx.clone();
    let preview_bytes = preview_byte_budget();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let mut p = phase_w.lock().unwrap_or_else(|p| p.into_inner());
                *p = Phase::ResultReady(Err(format!("runtime build failed: {}", e)));
                notify_w.notify_one();
                return;
            }
        };
        rt.block_on(async move {
            let result: Result<String, String> =
                Box::pin(crate::tools::execute_tool_with_context(&name_w, &args_w, &ctx_w))
                    .await
                    .map_err(|e| e.to_string());

            let mut p = phase_w.lock().unwrap_or_else(|p| p.into_inner());
            let next = match std::mem::replace(&mut *p, Phase::Pending) {
                Phase::Pending => {
                    *p = Phase::ResultReady(result);
                    notify_w.notify_one();
                    None
                }
                Phase::DetachedRunning => {
                    *p = Phase::DetachedDone;
                    Some(result)
                }
                other => {
                    // Already terminal — should not happen, but stay safe.
                    *p = other;
                    None
                }
            };
            drop(p);

            // If we transitioned to DetachedDone, finalize the job now.
            if let Some(r) = next {
                let db = match super::get_async_jobs_db() {
                    Some(db) => db.clone(),
                    None => return,
                };
                let session_id = ctx_w.session_id.clone();
                let agent_id = ctx_w.agent_id.clone();
                finalize_job(
                    &db,
                    &job_id_w,
                    &name_w,
                    session_id.as_deref(),
                    agent_id.as_deref(),
                    r,
                    preview_bytes,
                )
                .await;
            }
        });
    });

    let timer = tokio::time::sleep(std::time::Duration::from_secs(auto_bg_secs));
    tokio::pin!(timer);

    loop {
        // Cheap fast-path: if the worker already published a result, take it.
        {
            let mut p = phase.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(*p, Phase::ResultReady(_)) {
                if let Phase::ResultReady(r) =
                    std::mem::replace(&mut *p, Phase::Consumed)
                {
                    return r.map_err(|e| anyhow::anyhow!(e));
                }
            }
        }

        tokio::select! {
            _ = notify.notified() => {
                // Loop and re-check the phase.
                continue;
            }
            _ = &mut timer => {
                // Budget exceeded — atomically transition to DetachedRunning
                // unless the worker already finished in the meantime.
                let mut p = phase.lock().unwrap_or_else(|p| p.into_inner());
                match std::mem::replace(&mut *p, Phase::Pending) {
                    Phase::ResultReady(r) => {
                        *p = Phase::Consumed;
                        return r.map_err(|e| anyhow::anyhow!(e));
                    }
                    Phase::Pending => {
                        *p = Phase::DetachedRunning;
                        drop(p);

                        // Persist the job row so `job_status` can find it.
                        if let Some(db) = super::get_async_jobs_db() {
                            if let Err(e) = record_running_job(
                                db,
                                &job_id,
                                ctx,
                                name,
                                args,
                                JobOrigin::AutoBackgrounded,
                            ) {
                                app_warn!(
                                    "async_jobs",
                                    "auto_bg",
                                    "Failed to insert auto-background job row: {}",
                                    e
                                );
                            }
                        }
                        app_info!(
                            "async_jobs",
                            "auto_bg",
                            "Tool '{}' exceeded {}s sync budget — backgrounded as job {}",
                            name,
                            auto_bg_secs,
                            &job_id
                        );
                        return Ok(synthetic_started_result(
                            &job_id,
                            name,
                            JobOrigin::AutoBackgrounded,
                        ));
                    }
                    other => {
                        *p = other;
                        // Loop again — should be transient.
                        continue;
                    }
                }
            }
        }
    }
}

/// Phase machine for the auto-background race between OS-thread dispatch
/// and main-thread budget timer. Transitions are guarded by a `Mutex` so
/// the worker and the awaiter agree on who finalizes the job.
#[derive(Debug)]
enum Phase {
    Pending,
    ResultReady(Result<String, String>),
    /// Main thread gave up; OS thread will finalize when done.
    DetachedRunning,
    /// OS thread finished after detach; main thread already returned synthetic.
    DetachedDone,
    /// Main thread consumed an inline result.
    Consumed,
}


async fn run_job_to_completion(
    db: Arc<AsyncJobsDB>,
    job_id: String,
    tool_name: String,
    args: Value,
    ctx: ToolExecContext,
    max_secs: u64,
    preview_bytes: usize,
) {
    let session_id = ctx.session_id.clone();
    let agent_id = ctx.agent_id.clone();

    let dispatch = Box::pin(crate::tools::execute_tool_with_context(&tool_name, &args, &ctx));
    let result: Result<String, String> = if max_secs == 0 {
        dispatch.await.map_err(|e| e.to_string())
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(max_secs), dispatch).await {
            Ok(inner) => inner.map_err(|e| e.to_string()),
            Err(_elapsed) => Err(format!(
                "Async tool job '{}' exceeded max_job_secs ({}s) and was cancelled",
                job_id, max_secs
            )),
        }
    };

    finalize_job(
        &db,
        &job_id,
        &tool_name,
        session_id.as_deref(),
        agent_id.as_deref(),
        result,
        preview_bytes,
    )
    .await;
}

async fn finalize_job(
    db: &AsyncJobsDB,
    job_id: &str,
    tool_name: &str,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    result: Result<String, String>,
    preview_bytes: usize,
) {
    let (status, preview, path, error_text) = match result {
        Ok(output) => {
            let (preview, path) = persist_result(job_id, &output, preview_bytes);
            (AsyncJobStatus::Completed, Some(preview), path, None)
        }
        Err(e) => {
            let is_timeout = e.contains("exceeded max_job_secs");
            let st = if is_timeout {
                AsyncJobStatus::TimedOut
            } else {
                AsyncJobStatus::Failed
            };
            (st, None, None, Some(e))
        }
    };

    if let Err(e) = db.update_terminal(
        job_id,
        status,
        preview.as_deref(),
        path.as_deref(),
        error_text.as_deref(),
        now_secs(),
    ) {
        app_error!(
            "async_jobs",
            "finalize",
            "Failed to update terminal status for job {}: {}",
            job_id,
            e
        );
    }

    // Wake per-job `job_status(block=true)` waiters; the EventBus emit below
    // is retained for frontend subscribers only.
    super::wait::notify_completion(job_id);
    emit_completion_event(job_id, tool_name, status.as_str());

    // Schedule injection back into the parent session.
    if let Some(sid) = session_id {
        injection::dispatch_injection(
            sid.to_string(),
            agent_id.map(|s| s.to_string()),
            job_id.to_string(),
            tool_name.to_string(),
            status,
            preview,
            error_text,
        );
    } else {
        // No parent session — mark as injected so it isn't replayed forever.
        let _ = db.mark_injected(job_id);
    }
}

/// Spool the full result to disk if it exceeds the inline budget, returning
/// (preview, optional_disk_path).
fn persist_result(job_id: &str, output: &str, max_bytes: usize) -> (String, Option<String>) {
    if output.len() <= max_bytes {
        return (output.to_string(), None);
    }
    let path = match crate::paths::async_job_result_path(job_id) {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "async_jobs",
                "persist",
                "Failed to resolve job result path for {}: {}",
                job_id,
                e
            );
            return (truncate_preview(output, max_bytes), None);
        }
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            app_warn!(
                "async_jobs",
                "persist",
                "Failed to create result dir for {}: {}",
                job_id,
                e
            );
        }
    }
    if let Err(e) = std::fs::write(&path, output) {
        app_warn!(
            "async_jobs",
            "persist",
            "Failed to write result file for {}: {}",
            job_id,
            e
        );
        return (truncate_preview(output, max_bytes), None);
    }
    let preview = truncate_preview(output, max_bytes);
    (preview, Some(path.to_string_lossy().to_string()))
}

fn truncate_preview(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }
    let head_budget = max_bytes.saturating_mul(2) / 3;
    let tail_budget = max_bytes.saturating_sub(head_budget);
    let head = crate::truncate_utf8(output, head_budget);
    let tail = crate::truncate_utf8_tail(output, tail_budget);
    let omitted = output.len().saturating_sub(head.len() + tail.len());
    format!("{head}\n\n[...{omitted} bytes omitted...]\n\n{tail}")
}

fn emit_completion_event(job_id: &str, tool_name: &str, status: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "async_tool_job:completed",
            json!({
                "job_id": job_id,
                "tool": tool_name,
                "status": status,
            }),
        );
    }
}

fn preview_byte_budget() -> usize {
    let n = crate::config::cached_config()
        .async_tools
        .inline_result_bytes;
    if n == 0 {
        DEFAULT_PREVIEW_BYTES
    } else {
        n
    }
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

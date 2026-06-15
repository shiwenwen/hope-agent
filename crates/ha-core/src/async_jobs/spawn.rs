use anyhow::Result;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use super::db::AsyncJobsDB;
use super::error::JobError;
use super::injection;
use super::types::{AsyncJob, AsyncJobStatus, JobOrigin};
use crate::tools::{ToolExecContext, ASYNC_JOB_TIMEOUT_ARG};

const DEFAULT_PREVIEW_BYTES: usize = 4096;
const CANCEL_CLEANUP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

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
    // E4 (INCOG-2): an incognito job must not persist its raw tool args (which
    // can carry sensitive commands / prompts) in plaintext on disk. Store a
    // redaction placeholder — the live in-memory dispatch still receives the
    // real args; only the durable `async_jobs.db` row is scrubbed.
    let args_json = if ctx.incognito {
        "{\"_incognito_redacted\":true}".to_string()
    } else {
        serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
    };
    let job = AsyncJob {
        job_id: job_id.to_string(),
        session_id: ctx.session_id.clone(),
        agent_id: ctx.agent_id.clone(),
        tool_name: tool_name.to_string(),
        tool_call_id: ctx.tool_call_id.clone(),
        args_json,
        status: AsyncJobStatus::Running,
        result_preview: None,
        result_path: None,
        error: None,
        created_at: now_secs(),
        completed_at: None,
        injected: false,
        origin: origin.as_str().to_string(),
        // B4: how the backgrounded call was authorized (audit, TIMEOUT-2). Set
        // by the exec async approval-reorder; `None` for tools that skipped the
        // gate.
        approval_origin: ctx.approval_origin.map(|o| o.as_str().to_string()),
        incognito: ctx.incognito, // E4: gates spool persistence at finalize.
        pid: None,                // I3 (orphan cleanup)
        cancel_requested: false,  // I4 (cross-process cancel)
    };
    db.insert(&job)
}

/// Build the synthetic tool result string returned to the LLM when a tool
/// call is detached into the background. The model receives a job id it can
/// later snapshot via `job_status`; completion primarily arrives through
/// auto-injection.
pub fn synthetic_started_result(job_id: &str, tool_name: &str, origin: JobOrigin) -> String {
    let hint = match origin {
        JobOrigin::Explicit | JobOrigin::PolicyForced => {
            "The tool is running in the background. Continue with other work if possible; \
             otherwise stop the turn and wait for the auto-injected `<task-notification>`. \
             Do not immediately call `job_status` just to wait. Use `job_status` only for a \
             quick non-blocking snapshot after meaningful elapsed time or when the user asks. \
             Detailed output is saved to the notification's `output-file` when available."
        }
        JobOrigin::AutoBackgrounded => {
            "The tool exceeded the synchronous time budget and was auto-backgrounded. The \
             result will be auto-injected as a `<task-notification>` when ready. Continue \
             independent work if possible; otherwise stop the turn. Do not repeatedly poll \
             with `job_status`; use it only for a quick non-blocking snapshot after meaningful \
             elapsed time or when the user asks. \
             Detailed output is saved to the notification's `output-file` when available."
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

/// Strip async-job control parameters before recursively dispatching the
/// actual tool implementation. The outer async layer consumes these knobs;
/// individual tools should only see their own schema parameters.
fn strip_async_control_args(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        obj.remove("run_in_background");
        obj.remove(ASYNC_JOB_TIMEOUT_ARG);
    }
    args
}

fn requested_job_timeout_secs(args: &Value) -> Option<u64> {
    args.get(ASYNC_JOB_TIMEOUT_ARG)
        .and_then(|v| v.as_u64())
        .filter(|secs| *secs > 0)
}

fn clamp_job_timeout_secs(configured_max_secs: u64, requested_secs: Option<u64>) -> u64 {
    match (configured_max_secs, requested_secs) {
        (0, Some(requested)) => requested,
        (0, None) => 0,
        (configured, Some(requested)) => configured.min(requested),
        (configured, None) => configured,
    }
}

fn effective_max_job_secs(args: &Value) -> u64 {
    clamp_job_timeout_secs(
        crate::config::cached_config().async_tools.max_job_secs,
        requested_job_timeout_secs(args),
    )
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

    // I2 (MISC-5): reserve a background-job concurrency slot before committing
    // any state. The guard is moved into the runner thread below and released
    // when that thread exits, so the slot frees exactly when the job ends. At
    // the cap, surface an actionable error result the model can act on (wait /
    // poll job_status / run synchronously) instead of stacking another OS
    // thread. Held across all early returns below — dropped (released) on each.
    let slot = match super::slots::try_acquire_job_slot() {
        Some(slot) => slot,
        None => {
            let max = crate::config::cached_config()
                .async_tools
                .max_concurrent_jobs;
            return Err(anyhow::anyhow!(
                "Background job limit reached ({max} concurrent). Too many tools are already \
                 running in the background — wait for some to finish (check `job_status`) before \
                 backgrounding more, or re-run this one synchronously (without `run_in_background`)."
            ));
        }
    };

    let job_id = new_job_id();
    // review#1: register the cancel token BEFORE the row becomes queryable, so a
    // concurrent cancel (e.g. cancel_jobs_for_session on session delete) that
    // sees the fresh `running` row also finds a live token — instead of taking
    // the "no active runner" branch and marking it Cancelled while this worker
    // keeps running unsignalled. Roll the registration back if the insert fails.
    let cancel_token = super::cancel::register_job(&job_id);
    if let Err(e) = record_running_job(&db, &job_id, &ctx, tool_name, &args, origin) {
        super::cancel::remove_job(&job_id);
        return Err(e);
    }

    let synthetic = synthetic_started_result(&job_id, tool_name, origin);

    // Strip async-job controls from args AND set bypass on the ctx so the
    // recursive `execute_tool_with_context` call inside the OS thread runtime
    // goes straight to the sync dispatch path. Without bypass the
    // `AlwaysBackground` policy would re-enter `spawn_explicit_job` forever.
    let max_secs = effective_max_job_secs(&args);
    let clean_args = strip_async_control_args(args);
    ctx.bypass_async_dispatch = true;
    // Engine gate already ran (or was deliberately skipped for `exec`) at
    // the outer dispatch; the recursive inner call must not re-prompt (the
    // user has no surface to answer it from inside a background runtime).
    // `external_pre_approved` silences the engine-level prompt **without**
    // bypassing `exec`'s command-level dangerous/edit audit — flipping
    // `auto_approve_tools` here would let any shell command run silently
    // whenever `run_in_background: true` is set. Visibility / plan-mode
    // checks still re-run as belt-and-suspenders.
    ctx.external_pre_approved = true;
    ctx.suppress_global_tool_timeout = true;
    ctx.suppress_result_disk_persistence = true;
    ctx.cancellation_token = Some(cancel_token.clone());
    // I3: let the re-dispatched tool record its spawned child pid into this
    // job's row, so a crash/restart can terminate the orphaned process tree
    // (the row already exists — recorded above). Cheap guarded UPDATE per spawn.
    {
        let pid_db = db.clone();
        let pid_job_id = job_id.clone();
        ctx.pid_sink = Some(crate::tools::PidSink(std::sync::Arc::new(
            move |pid: u32| {
                let _ = pid_db.set_pid(&pid_job_id, pid as i64);
            },
        )));
    }
    let preview_bytes = preview_byte_budget();
    let tool_name_owned = tool_name.to_string();
    let job_id_owned = job_id.clone();

    // Run on a dedicated OS thread so we don't constrain the dispatch future
    // to be `Send`. This mirrors `subagent::injection::inject_and_run_parent`.
    std::thread::spawn(move || {
        // Hold the concurrency slot for the job's whole lifetime; it releases
        // when this thread exits via any path (success, failure, or the
        // runtime-build-failure early return below).
        let _slot = slot;
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let err_msg = format!("runtime build failed: {}", e);
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
                    Some(&err_msg),
                    now_secs(),
                );
                super::wait::notify_completion(&job_id_owned);
                emit_completion_event(&job_id_owned, &tool_name_owned, "failed");
                super::cancel::remove_job(&job_id_owned);
                // I6: don't silently lose the job. `finalize_job` never runs on
                // this build-failure path, so fire the terminal hook (H4 parity)
                // and inject the failure back into the parent session — the model
                // already saw a synthetic "started" and would otherwise wait
                // forever. `dispatch_injection` spawns its own thread + runtime,
                // so it works even though THIS runtime failed to build.
                crate::hooks::fire_async_job_terminal(
                    ctx.session_id.as_deref(),
                    ctx.agent_id.as_deref(),
                    &tool_name_owned,
                    ctx.tool_call_id.as_deref(),
                    &job_id_owned,
                    true,
                    false,
                    &err_msg,
                );
                if let Some(sid) = ctx.session_id.clone() {
                    injection::dispatch_injection(
                        sid,
                        ctx.agent_id.clone(),
                        job_id_owned.clone(),
                        tool_name_owned.clone(),
                        ctx.tool_call_id.clone(),
                        AsyncJobStatus::Failed,
                        None,
                        None,
                        Some(err_msg),
                    );
                } else {
                    let _ = db.mark_injected(&job_id_owned);
                }
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
                cancel_token,
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
    let cancel_token = ctx
        .cancellation_token
        .as_ref()
        .map(CancellationToken::child_token)
        .unwrap_or_default();

    let phase_w = phase.clone();
    let notify_w = notify.clone();
    let job_id_w = job_id.clone();
    let name_w = name.to_string();
    let args_w = strip_async_control_args(args.clone());
    let cancel_w = cancel_token.clone();
    let mut ctx_w = ctx.clone();
    ctx_w.cancellation_token = Some(cancel_w.clone());
    ctx_w.suppress_result_disk_persistence = true;
    let preview_bytes = preview_byte_budget();
    let max_secs = effective_max_job_secs(args);

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let mut p = phase_w.lock().unwrap_or_else(|p| p.into_inner());
                *p = Phase::ResultReady(Err(JobError::Failed {
                    message: format!("runtime build failed: {}", e),
                }));
                notify_w.notify_one();
                return;
            }
        };
        rt.block_on(async move {
            // I4 parity with the explicit path: observe a cross-process cancel
            // (the DB `cancel_requested` flag set by another process) and trip the
            // local token. Before detach the row doesn't exist yet so the poll
            // just reads `false`; once detached it becomes effective. Aborted the
            // moment the job settles, below.
            let poll_handle = super::get_async_jobs_db().map(|db| {
                spawn_cross_process_cancel_watcher(db.clone(), job_id_w.clone(), cancel_w.clone())
            });
            let mut dispatch = Box::pin(crate::tools::execute_tool_with_context(
                &name_w, &args_w, &ctx_w,
            ));
            let result: Result<String, JobError> = if max_secs == 0 {
                tokio::select! {
                    inner = &mut dispatch => inner.map_err(JobError::from_dispatch_error),
                    _ = cancel_w.cancelled() => {
                        let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                        Err(JobError::Cancelled)
                    },
                }
            } else {
                let timer = tokio::time::sleep(std::time::Duration::from_secs(max_secs));
                tokio::pin!(timer);
                tokio::select! {
                    inner = &mut dispatch => inner.map_err(JobError::from_dispatch_error),
                    _ = &mut timer => {
                        cancel_w.cancel();
                        let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                        Err(JobError::TimedOut { max_secs })
                    },
                    _ = cancel_w.cancelled() => {
                        let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                        Err(JobError::Cancelled)
                    },
                }
            };

            // Job settled — stop the cross-process cancel watcher.
            if let Some(h) = poll_handle {
                h.abort();
            }

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
                let tool_call_id = ctx_w.tool_call_id.clone();
                // E4 (INCOG-2) parity with `run_job_to_completion`: re-check
                // incognito at settle, not only at spawn. A session burned/deleted
                // mid-flight would otherwise keep a stale `false` and spool its
                // result to disk. `is_session_incognito` is fail-closed on a gone
                // row, so a now-removed session also skips the spool.
                let incognito =
                    ctx_w.incognito || crate::session::is_session_incognito(session_id.as_deref());
                finalize_job(
                    &db,
                    &job_id_w,
                    &name_w,
                    session_id.as_deref(),
                    agent_id.as_deref(),
                    tool_call_id,
                    r,
                    preview_bytes,
                    incognito,
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
                if let Phase::ResultReady(r) = std::mem::replace(&mut *p, Phase::Consumed) {
                    return r.map_err(JobError::into_inline_error);
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
                        return r.map_err(JobError::into_inline_error);
                    }
                    Phase::Pending => {
                        // Persist the job row before returning a synthetic id.
                        // If this claim fails, the model would receive an
                        // unpollable job_id and the worker would be unable to
                        // inject its result, so fail the current tool call
                        // instead and cancel the detached worker best-effort.
                        // Keep the phase lock until the row exists: otherwise
                        // the worker can observe DetachedRunning, finish first,
                        // and lose its result because update_terminal sees no row.
                        let db = match super::get_async_jobs_db() {
                            Some(db) => db,
                            None => {
                                *p = Phase::Consumed;
                                drop(p);
                                cancel_token.cancel();
                                return Err(anyhow::anyhow!(
                                    "Async jobs DB not initialized; cannot auto-background tool '{}'",
                                    name
                                ));
                            }
                        };
                        // review#1: register the cancel token BEFORE the row is
                        // queryable, so a concurrent session-delete cancel finds
                        // a live token instead of taking the "no active runner"
                        // branch and marking it Cancelled while this detached
                        // worker keeps running unsignalled.
                        super::cancel::register_job_token(&job_id, cancel_token.clone());
                        if let Err(e) =
                            record_running_job(db, &job_id, ctx, name, args, JobOrigin::AutoBackgrounded)
                        {
                            *p = Phase::Consumed;
                            drop(p);
                            super::cancel::remove_job(&job_id);
                            cancel_token.cancel();
                            // I6 (MISC-9): the row failed to persist, so the
                            // pre-allocated `job_id` is a ghost the model could
                            // never poll — keep it out of the error and surface
                            // the tool instead.
                            return Err(anyhow::anyhow!(
                                "Failed to background tool '{}': {}",
                                name,
                                e
                            ));
                        }
                        *p = Phase::DetachedRunning;
                        drop(p);
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
    ResultReady(Result<String, JobError>),
    /// Main thread gave up; OS thread will finalize when done.
    DetachedRunning,
    /// OS thread finished after detach; main thread already returned synthetic.
    DetachedDone,
    /// Main thread consumed an inline result.
    Consumed,
}

/// Spawn the I4 cross-process cancel watcher shared by both background paths.
///
/// A cancel issued from another process (desktop + headless `server` share
/// `async_jobs.db`) can't reach this runtime's in-memory [`CancellationToken`],
/// so [`super::cancel_job`] persists a `cancel_requested` flag instead. This
/// polls that flag (single-row PK read, 5s) and trips `token` on a hit. It stops
/// itself the moment the token is cancelled by any source; the caller must
/// `.abort()` the returned handle once the job settles so it can't outlive the
/// work. Used by `run_job_to_completion` (explicit) and the
/// `dispatch_with_auto_background` worker (auto) so both honor a cross-process
/// cancel identically.
fn spawn_cross_process_cancel_watcher(
    db: Arc<AsyncJobsDB>,
    job_id: String,
    token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    if db.is_cancel_requested(&job_id).unwrap_or(false) {
                        token.cancel();
                        return;
                    }
                }
            }
        }
    })
}

async fn run_job_to_completion(
    db: Arc<AsyncJobsDB>,
    job_id: String,
    tool_name: String,
    args: Value,
    ctx: ToolExecContext,
    max_secs: u64,
    preview_bytes: usize,
    cancel_token: CancellationToken,
) {
    let session_id = ctx.session_id.clone();
    let agent_id = ctx.agent_id.clone();
    let tool_call_id = ctx.tool_call_id.clone();
    let incognito = ctx.incognito;

    // I4: cross-process cancel watcher (shared with the auto-background worker).
    // Aborted once the job settles below.
    let poll_handle =
        spawn_cross_process_cancel_watcher(db.clone(), job_id.clone(), cancel_token.clone());

    let mut dispatch = Box::pin(crate::tools::execute_tool_with_context(
        &tool_name, &args, &ctx,
    ));
    let result: Result<String, JobError> = if max_secs == 0 {
        tokio::select! {
            inner = &mut dispatch => inner.map_err(JobError::from_dispatch_error),
            _ = cancel_token.cancelled() => {
                let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                Err(JobError::Cancelled)
            },
        }
    } else {
        let timer = tokio::time::sleep(std::time::Duration::from_secs(max_secs));
        tokio::pin!(timer);
        tokio::select! {
            inner = &mut dispatch => inner.map_err(JobError::from_dispatch_error),
            _ = &mut timer => {
                cancel_token.cancel();
                let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                Err(JobError::TimedOut { max_secs })
            },
            _ = cancel_token.cancelled() => {
                let _ = tokio::time::timeout(CANCEL_CLEANUP_GRACE, &mut dispatch).await;
                Err(JobError::Cancelled)
            },
        }
    };

    // The job has settled — stop the cross-process cancel watcher (no-op if it
    // already exited because the token was cancelled).
    poll_handle.abort();

    // E4 (INCOG-2) hardening: re-evaluate incognito at settle time, not only at
    // spawn. `ctx.incognito` was captured when the job started; a long-running
    // job whose session was burned/deleted meanwhile would otherwise keep a
    // stale `false` and spool its large result to disk for a session that no
    // longer exists. `is_session_incognito` is fail-closed (row-absent => true),
    // so a now-gone session also skips the spool.
    let incognito = incognito || crate::session::is_session_incognito(session_id.as_deref());

    finalize_job(
        &db,
        &job_id,
        &tool_name,
        session_id.as_deref(),
        agent_id.as_deref(),
        tool_call_id,
        result,
        preview_bytes,
        incognito,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn finalize_job(
    db: &AsyncJobsDB,
    job_id: &str,
    tool_name: &str,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    tool_call_id: Option<String>,
    result: Result<String, JobError>,
    preview_bytes: usize,
    incognito: bool,
) {
    let (status, preview, path, error_text) = match result {
        Ok(output) => {
            let (preview, path) = persist_result(job_id, &output, preview_bytes, incognito);
            (AsyncJobStatus::Completed, Some(preview), path, None)
        }
        Err(job_err) => {
            // Typed terminal status — no more re-parsing the error message
            // (MISC-7). `DeniedByUser` folds into `Failed` with STOP-preserving
            // text via `display_for_injection`.
            let status = job_err.to_status();
            let error_text = job_err.display_for_injection();
            (status, None, None, Some(error_text))
        }
    };

    let updated = match db.update_terminal(
        job_id,
        status,
        preview.as_deref(),
        path.as_deref(),
        error_text.as_deref(),
        now_secs(),
    ) {
        Ok(updated) => updated,
        Err(e) => {
            app_error!(
                "async_jobs",
                "finalize",
                "Failed to update terminal status for job {}: {}",
                job_id,
                e
            );
            false
        }
    };
    super::cancel::remove_job(job_id);
    if !updated {
        return;
    }

    // Wake per-job `job_status(block=true)` waiters; the EventBus emit below
    // is retained for frontend subscribers only.
    super::wait::notify_completion(job_id);
    emit_completion_event(job_id, tool_name, status.as_str());

    // H4: fire the terminal PostToolUse / PostToolUseFailure hook so a
    // backgrounded job is visible to hooks (HOOKS-1) — including cancellation
    // (HOOKS-4, is_interrupt=true). Borrow (as_deref) before the owned fields
    // are moved into `dispatch_injection` below. Fire-and-forget on the
    // process-lived runtime, so it survives this OS thread exiting.
    {
        let (is_error, is_interrupt) = status.terminal_hook_flags();
        let detail = if is_error {
            error_text.as_deref().unwrap_or("")
        } else {
            preview.as_deref().unwrap_or("")
        };
        crate::hooks::fire_async_job_terminal(
            session_id,
            agent_id,
            tool_name,
            tool_call_id.as_deref(),
            job_id,
            is_error,
            is_interrupt,
            detail,
        );
    }

    // Schedule injection back into the parent session.
    if status == AsyncJobStatus::Cancelled {
        let _ = db.mark_injected(job_id);
    } else if let Some(sid) = session_id {
        injection::dispatch_injection(
            sid.to_string(),
            agent_id.map(|s| s.to_string()),
            job_id.to_string(),
            tool_name.to_string(),
            tool_call_id,
            status,
            preview,
            path,
            error_text,
        );
    } else {
        // No parent session — mark as injected so it isn't replayed forever.
        let _ = db.mark_injected(job_id);
    }
}

/// Spool the full result to disk and keep a bounded inline preview in SQLite.
/// Returning a stable output file lets the parent agent decide when to spend a
/// `read` call on detailed output instead of embedding arbitrary tool text in
/// the notification envelope.
fn persist_result(
    job_id: &str,
    output: &str,
    max_bytes: usize,
    incognito: bool,
) -> (String, Option<String>) {
    let preview = if output.len() <= max_bytes {
        output.to_string()
    } else {
        truncate_preview(output, max_bytes)
    };
    // E4 (INCOG-2): incognito jobs keep only the bounded inline preview — never
    // spool the full output to disk, so burn-on-close leaves no spool file.
    if incognito {
        return (preview, None);
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
            return (preview, None);
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
        return (preview, None);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_job_id_uses_job_prefix_and_uuid_suffix() {
        let id = new_job_id();
        assert!(id.starts_with("job_"), "unexpected prefix in {}", id);
        // Simple uuid v4 is 32 hex chars; whole id is "job_" + 32 = 36.
        assert_eq!(id.len(), 36);
        assert!(id[4..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn new_job_id_is_unique() {
        let a = new_job_id();
        let b = new_job_id();
        assert_ne!(a, b);
    }

    #[test]
    fn persist_result_incognito_skips_spool() {
        // E4 (INCOG-2): even a large output must never spool to disk for an
        // incognito job — only the bounded inline preview is kept, with no path.
        let big = "x".repeat(10_000);
        let (preview, path) = persist_result("job_incognito", &big, 100, true);
        assert!(
            path.is_none(),
            "incognito job must not produce a spool path"
        );
        assert!(
            preview.len() < big.len(),
            "preview should be truncated to the inline budget"
        );
    }

    #[test]
    fn synthetic_started_result_explicit_shape() {
        let body = synthetic_started_result("job_xyz", "exec", JobOrigin::Explicit);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["job_id"], "job_xyz");
        assert_eq!(v["status"], "started");
        assert_eq!(v["tool"], "exec");
        assert_eq!(v["origin"], "explicit");
        let hint = v["hint"].as_str().unwrap();
        assert!(hint.contains("background"));
        assert!(hint.contains("job_status"));
    }

    #[test]
    fn synthetic_started_result_auto_backgrounded_has_distinct_hint() {
        let explicit = synthetic_started_result("j1", "t", JobOrigin::Explicit);
        let auto = synthetic_started_result("j1", "t", JobOrigin::AutoBackgrounded);
        assert_ne!(explicit, auto);
        let v: Value = serde_json::from_str(&auto).unwrap();
        assert_eq!(v["origin"], "auto_backgrounded");
        assert!(v["hint"]
            .as_str()
            .unwrap()
            .contains("exceeded the synchronous time budget"));
    }

    #[test]
    fn per_call_job_timeout_only_tightens_configured_cap() {
        assert_eq!(clamp_job_timeout_secs(1800, None), 1800);
        assert_eq!(clamp_job_timeout_secs(1800, Some(600)), 600);
        assert_eq!(clamp_job_timeout_secs(1800, Some(3600)), 1800);
        assert_eq!(clamp_job_timeout_secs(0, None), 0);
        assert_eq!(clamp_job_timeout_secs(0, Some(600)), 600);
    }

    #[test]
    fn strip_async_control_args_removes_outer_async_knobs() {
        let cleaned = strip_async_control_args(json!({
            "command": "echo hi",
            "run_in_background": true,
            "job_timeout_secs": 60
        }));

        assert_eq!(cleaned, json!({ "command": "echo hi" }));
    }

    #[test]
    fn truncate_preview_returns_input_unchanged_when_within_budget() {
        let s = "short output";
        assert_eq!(truncate_preview(s, 100), s);
    }

    #[test]
    fn truncate_preview_includes_head_tail_and_omitted_marker() {
        // 120-byte ASCII input forces truncation at `max_bytes`; the exact head
        // and tail sizes are computed from the same formula the production
        // code uses so that retuning the split ratio doesn't break this test.
        let max_bytes = 30usize;
        let head_budget = max_bytes.saturating_mul(2) / 3;
        let tail_budget = max_bytes.saturating_sub(head_budget);
        let body: String = (b'a'..=b'z')
            .chain(b'0'..=b'9')
            .cycle()
            .take(120)
            .map(|b| b as char)
            .collect();
        let preview = truncate_preview(&body, max_bytes);
        assert!(preview.contains("[..."), "expected marker in: {preview}");
        assert!(preview.contains("bytes omitted"));
        assert!(preview.starts_with(&body[..head_budget]));
        assert!(preview.ends_with(&body[body.len() - tail_budget..]));
    }

    #[test]
    fn truncate_preview_does_not_split_utf8_multibyte() {
        // Each "中" is 3 bytes. 40 × "中" = 120 bytes. A 20-byte budget gets
        // floored to char boundaries in both head and tail — the output must
        // start with whole "中" glyphs, not with a stray continuation byte.
        let body: String = "中".repeat(40);
        let preview = truncate_preview(&body, 20);
        assert!(preview.starts_with("中"));
        assert!(preview.ends_with("中"));
        assert!(preview.contains("bytes omitted"));
    }
}

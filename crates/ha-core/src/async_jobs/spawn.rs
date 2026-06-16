use anyhow::Result;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use super::db::JobsDB;
use super::error::JobError;
use super::injection;
use super::types::{BackgroundJob, JobKind, JobOrigin, JobStatus};
use crate::tools::{ToolExecContext, ASYNC_JOB_TIMEOUT_ARG};

const DEFAULT_PREVIEW_BYTES: usize = 4096;
const CANCEL_CLEANUP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Generate a stable, short-prefix job id.
pub fn new_job_id() -> String {
    format!("job_{}", uuid::Uuid::new_v4().simple())
}

/// Persist a freshly spawned job row in the given state — `running` for the
/// immediate path, `queued` when the cap is full and it must wait for a slot.
pub fn record_running_job(
    db: &JobsDB,
    job_id: &str,
    ctx: &ToolExecContext,
    tool_name: &str,
    args: &Value,
    origin: JobOrigin,
    status: JobStatus,
) -> Result<()> {
    // E4 (INCOG-2): an incognito job must not persist its raw tool args (which
    // can carry sensitive commands / prompts) in plaintext on disk. Store a
    // redaction placeholder — the live in-memory dispatch still receives the
    // real args; only the durable `background_jobs.db` row is scrubbed.
    let args_json = if ctx.incognito {
        "{\"_incognito_redacted\":true}".to_string()
    } else {
        serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
    };
    let job = BackgroundJob {
        job_id: job_id.to_string(),
        // R1: this path is the Tool executor — the only wired `JobKind` today.
        kind: super::types::JobKind::Tool,
        // R6: tool jobs never reference a subagent run.
        subagent_run_id: None,
        // R5: tool jobs are standalone — never part of a Group fan-out.
        group_id: None,
        session_id: ctx.session_id.clone(),
        agent_id: ctx.agent_id.clone(),
        tool_name: tool_name.to_string(),
        tool_call_id: ctx.tool_call_id.clone(),
        args_json,
        status,
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
pub(crate) fn spawn_explicit_job(
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
    // review#1: register the cancel token BEFORE the row becomes queryable, so a
    // concurrent cancel (cancel_jobs_for_session on session delete, or a cancel
    // of a still-QUEUED job) finds a live token. Roll back on any early failure.
    let cancel_token = super::cancel::register_job(&job_id);

    // Prepare the ctx for the (possibly deferred) inner re-dispatch BEFORE the
    // slot decision, so a queued job carries a ready-to-run ctx in the queue.
    let max_secs = effective_max_job_secs(&args);
    ctx.bypass_async_dispatch = true;
    // Engine gate already ran (or was deliberately skipped for `exec`) at the
    // outer dispatch; the recursive inner call must not re-prompt (the user has
    // no surface to answer it from inside a background runtime).
    // `external_pre_approved` silences the engine-level prompt **without**
    // bypassing `exec`'s command-level dangerous/edit audit — flipping
    // `auto_approve_tools` here would let any shell command run silently whenever
    // `run_in_background: true` is set. Visibility / plan-mode checks still
    // re-run as belt-and-suspenders.
    ctx.external_pre_approved = true;
    ctx.suppress_global_tool_timeout = true;
    ctx.suppress_result_disk_persistence = true;
    ctx.cancellation_token = Some(cancel_token.clone());
    // I3: let the re-dispatched tool record its spawned child pid into this job's
    // row so a crash/restart can terminate the orphaned process tree.
    {
        let pid_db = db.clone();
        let pid_job_id = job_id.clone();
        ctx.pid_sink = Some(crate::tools::PidSink(std::sync::Arc::new(
            move |pid: u32| {
                let _ = pid_db.set_pid(&pid_job_id, pid as i64);
            },
        )));
    }
    // R3 ①: tee a backgrounded `exec`'s running stdout/stderr into a bounded
    // tail buffer so `job_status(action:status)` can show a *running* job's
    // latest output (BashOutput parity). `exec` is the only async tool that
    // streams; incognito jobs leave no tail (close-and-burn, like the spool).
    if tool_name == crate::tools::TOOL_EXEC && !ctx.incognito {
        super::output_tail::register(&job_id);
        ctx.output_tail_job_id = Some(job_id.clone());
    }

    let synthetic = synthetic_started_result(&job_id, tool_name, origin);
    let session_key = ctx.session_id.clone().unwrap_or_default();

    // R7.1: try to reserve a concurrency slot. If the cap is full the job QUEUES
    // (status `Queued`) and the Primary scheduler promotes it per-session
    // round-robin when a slot frees — instead of hard-rejecting.
    let reservation = super::slots::try_reserve(&session_key);
    let status = if reservation.is_some() {
        JobStatus::Running
    } else {
        JobStatus::Queued
    };
    if let Err(e) = record_running_job(&db, &job_id, &ctx, tool_name, &args, origin, status) {
        super::cancel::remove_job(&job_id);
        // R3: this early-return bypasses finalize_job — drop the tail ring
        // registered above (no-op if none was). Paired with every cancel::remove_job.
        super::output_tail::remove(&job_id);
        return Err(e);
    }
    // R3: announce the new job (running or queued) on the unified `job:*` bus.
    super::events::emit_created(
        &job_id,
        super::types::JobKind::Tool,
        tool_name,
        status.as_str(),
        ctx.session_id.as_deref(),
    );

    let clean_args = strip_async_control_args(args);
    let prepared = super::slots::PreparedJob {
        job_id: job_id.clone(),
        tool_name: tool_name.to_string(),
        args: clean_args,
        ctx,
        max_secs,
        preview_bytes: preview_byte_budget(),
        cancel_token,
    };

    match reservation {
        Some(reservation) => {
            start_runner(db, prepared, reservation);
            Ok(synthetic)
        }
        None => {
            if super::slots::enqueue(prepared) {
                // Wake the scheduler in case a slot is already free (nothing
                // finished to notify it otherwise).
                super::slots::wake_scheduler();
                Ok(synthetic)
            } else {
                // The wait queue itself is full — roll back the row + token and
                // hard-reject so the model can wait or run synchronously.
                super::cancel::remove_job(&job_id);
                super::output_tail::remove(&job_id); // R3: bypasses finalize_job
                let _ = db.delete(&job_id);
                Err(anyhow::anyhow!(
                    "Background job queue is full — too many tools are already running or waiting. \
                     Wait for some to finish (check `job_status`) before backgrounding more, or \
                     re-run this one synchronously (without `run_in_background`)."
                ))
            }
        }
    }
}

/// Spawn the dedicated OS thread + current-thread runtime that runs a
/// backgrounded job to completion. Used by both the immediate path
/// ([`spawn_explicit_job`]) and the scheduler promote path ([`run_scheduler`]).
/// The [`super::slots::SlotReservation`] is held for the thread's whole
/// lifetime, so the slot releases (and the scheduler wakes) exactly when the job
/// ends — on every exit path including the runtime-build failure below.
fn start_runner(
    db: Arc<JobsDB>,
    prepared: super::slots::PreparedJob,
    reservation: super::slots::SlotReservation,
) {
    let super::slots::PreparedJob {
        job_id: job_id_owned,
        tool_name: tool_name_owned,
        args: clean_args,
        ctx,
        max_secs,
        preview_bytes,
        cancel_token,
    } = prepared;

    // Run on a dedicated OS thread so we don't constrain the dispatch future to
    // be `Send`. This mirrors `subagent::injection::inject_and_run_parent`.
    std::thread::spawn(move || {
        // Hold the slot reservation for the job's whole lifetime; on drop it
        // decrements the running count and wakes the scheduler, so the freed
        // slot immediately promotes the next queued job. Released on every exit
        // path (success, failure, or the runtime-build-failure early return).
        let _reservation = reservation;
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
                    JobStatus::Failed,
                    None,
                    None,
                    Some(&err_msg),
                    now_secs(),
                );
                super::wait::notify_completion(&job_id_owned);
                emit_completion_event(
                    &job_id_owned,
                    &tool_name_owned,
                    "failed",
                    ctx.session_id.as_deref(),
                );
                super::cancel::remove_job(&job_id_owned);
                // R3: this build-failure path also bypasses finalize_job — drop
                // the tail ring (no-op if none).
                super::output_tail::remove(&job_id_owned);
                // I6: don't silently lose the job. `finalize_job` never runs on
                // this build-failure path, so fire the terminal hook (H4 parity)
                // and inject the failure back into the parent session.
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
                    // R4: same merge window as the normal finalize path.
                    injection::enqueue_injection(
                        sid,
                        ctx.agent_id.clone(),
                        job_id_owned.clone(),
                        tool_name_owned.clone(),
                        ctx.tool_call_id.clone(),
                        JobStatus::Failed,
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
}

/// Background-job scheduler task (R7.1): parks until a slot frees / a job is
/// enqueued (with a periodic fallback tick), then promotes queued jobs into free
/// slots per-session round-robin until no slot is free or the queue is empty.
///
/// **Tier-agnostic** — the wait queue is process-local (it pins each job's live
/// ctx, which can't be persisted), so EVERY process that can background tools
/// runs its own scheduler over its OWN queue and never touches another process's
/// queued jobs (unlike `replay_pending_jobs`, which sweeps shared DB rows and is
/// therefore Primary-only). Idempotent: at most one loop per process even if
/// spawned from multiple init paths.
pub(crate) async fn run_scheduler() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return; // a scheduler loop is already running in this process
    }
    loop {
        // Wake on slot-free / enqueue, with a periodic fallback so queued jobs
        // still promote after an event that didn't notify us — e.g. the user
        // RAISES `max_concurrent_jobs` OR `max_concurrent_jobs_per_session` while
        // the queue is full (config changes fire no completion/Drop to wake us, so
        // a newly-eligible job waits up to one tick), or any theoretically-missed
        // notify. `try_take_next` re-reads both caps live, so the next tick honors
        // the raised value.
        tokio::select! {
            _ = super::slots::scheduler_notified() => {}
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        }
        let Some(db) = super::get_async_jobs_db() else {
            continue;
        };
        loop {
            let Some((prepared, reservation)) = super::slots::try_take_next() else {
                break;
            };
            // Promote the row `queued` -> `running`. If it is no longer queued (a
            // concurrent cancel already settled it terminal + removed it from the
            // queue), drop the reservation and skip — the cancel path handled it.
            match db.mark_running(&prepared.job_id) {
                Ok(true) => start_runner(db.clone(), prepared, reservation),
                Ok(false) => drop(reservation),
                Err(e) => {
                    app_error!(
                        "async_jobs",
                        "scheduler",
                        "mark_running failed for {}: {}",
                        prepared.job_id,
                        e
                    );
                    drop(reservation);
                }
            }
        }
    }
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
pub(crate) async fn dispatch_with_auto_background(
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
    // R3 ①: auto-backgrounded `exec` tees its running stdout/stderr into a tail
    // ring too (parity with explicit `run_in_background`), so `job_status` can
    // show a long auto-detached command's latest output. The ring is registered
    // INSIDE the worker (below), right before the tool streams — not here —
    // because this path runs the tool from the start without yet knowing it will
    // detach. Cleanup is uniform: the worker removes the ring on every
    // non-detached terminal outcome (`next.is_none()` — sync completion consumed
    // inline, or the awaiter already bailed), and `finalize_job` removes it on
    // the detached path. So no exit can leak the ring.
    let preview_bytes = preview_byte_budget();
    let max_secs = effective_max_job_secs(args);

    // R7.1: a job that auto-detaches must count against the concurrency pool for
    // the rest of its life (so later `try_reserve` sees the slot occupied). The
    // job is already running on the worker thread by the time we know it
    // detached, so we can only *count* it (forced reservation), not queue it. The
    // reservation lives in this slot, shared with the worker; the worker holds a
    // clone of the Arc, so it releases exactly when the worker thread ends — on
    // every terminal path. Stays `None` (harmless) unless we actually detach.
    let autobg_slot: Arc<Mutex<Option<super::slots::SlotReservation>>> = Arc::new(Mutex::new(None));
    let autobg_slot_w = autobg_slot.clone();

    std::thread::spawn(move || {
        // Hold the worker's Arc clone for the whole thread lifetime; dropping it
        // here (after `block_on` returns) releases any forced reservation set by
        // the detach path below.
        let _autobg_slot = autobg_slot_w;
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                // Ring not registered yet (we register after the runtime is up),
                // so nothing to clean here.
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
            // R3 ①: register the tail ring now that the runtime is up, before the
            // tool streams (exec only; incognito leaves no tail, like the spool).
            if name_w == crate::tools::TOOL_EXEC && !ctx_w.incognito {
                super::output_tail::register(&job_id_w);
                ctx_w.output_tail_job_id = Some(job_id_w.clone());
            }
            // R7.4: same retry path as the explicit runner (Failed-only, eligible
            // tools, cancellable backoff). A retrying auto-bg tool just takes
            // longer, which may trip the main thread's detach budget — correct
            // (it becomes a real background job and keeps retrying there).
            let result: Result<String, JobError> =
                run_tool_with_retry(&name_w, &args_w, &ctx_w, max_secs, &cancel_w).await;

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

            // R3 ①: `finalize_job` removes the tail ring on the detached path
            // (`next.is_some()`); every other terminal outcome — sync completion
            // consumed inline (`Pending → ResultReady`) or the awaiter already
            // bailed (`Consumed`/other) — bypasses `finalize_job`, so drop the
            // ring here. No-op when none was registered (non-exec / incognito).
            if next.is_none() {
                super::output_tail::remove(&job_id_w);
            }

            // If we transitioned to DetachedDone, finalize the job now.
            if let Some(r) = next {
                let db = match super::get_async_jobs_db() {
                    Some(db) => db.clone(),
                    None => {
                        // finalize_job (which removes the ring) is unreachable
                        // here — drop the ring so this early return can't leak it
                        // (symmetry with the explicit path's hoisted removals).
                        super::output_tail::remove(&job_id_w);
                        return;
                    }
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
                            record_running_job(db, &job_id, ctx, name, args, JobOrigin::AutoBackgrounded, JobStatus::Running)
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
                        // R7.1: now a real background job still running on its
                        // worker thread — count it against the pool (forced; it's
                        // already live, so it can't be queued/refused, only
                        // accounted). Set under the phase lock so the worker can't
                        // settle + drop its Arc before this is stored. Released
                        // when the worker thread ends (holds the other Arc clone).
                        // NOTE (pre-existing orphan window, orthogonal to R7.1): a
                        // session deleted in the sub-ms gap before the row insert
                        // above can be missed by `cancel_for_session` (it finds
                        // jobs via the DB row, not the token), so this worker may
                        // run orphaned to natural completion. The forced
                        // reservation tracks that live thread correctly and
                        // releases when it ends — no leak, just an uncancelled
                        // orphan (a cancellation-subsystem gap, not a quota one).
                        {
                            let session_key = ctx.session_id.clone().unwrap_or_default();
                            *autobg_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(super::slots::reserve_forced(&session_key));
                        }
                        drop(p);
                        // R3: it is now a real background job — announce it.
                        super::events::emit_created(
                            &job_id,
                            JobKind::Tool,
                            name,
                            JobStatus::Running.as_str(),
                            ctx.session_id.as_deref(),
                        );
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
/// `background_jobs.db`) can't reach this runtime's in-memory [`CancellationToken`],
/// so [`super::cancel_job`] persists a `cancel_requested` flag instead. This
/// polls that flag (single-row PK read, 5s) and trips `token` on a hit. It stops
/// itself the moment the token is cancelled by any source; the caller must
/// `.abort()` the returned handle once the job settles so it can't outlive the
/// work. Used by `run_job_to_completion` (explicit) and the
/// `dispatch_with_auto_background` worker (auto) so both honor a cross-process
/// cancel identically.
fn spawn_cross_process_cancel_watcher(
    db: Arc<JobsDB>,
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

/// Run a backgrounded tool's dispatch exactly once, racing it against the job's
/// cancel token (and the runtime budget when `max_secs > 0`). Shared by the
/// explicit-background runner ([`run_job_to_completion`]) and the auto-background
/// worker so both honor identical cancel/timeout semantics. A timeout cancels
/// the job token (terminal — the job settles `TimedOut`); a normal dispatch
/// error returns `Failed` WITHOUT touching the token, so a retry can re-run.
async fn run_tool_once(
    tool_name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    max_secs: u64,
    cancel_token: &CancellationToken,
) -> Result<String, JobError> {
    let mut dispatch = Box::pin(crate::tools::execute_tool_with_context(
        tool_name, args, ctx,
    ));
    if max_secs == 0 {
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
    }
}

/// Run a backgrounded tool with R7.4 retry: re-attempt a retry-eligible tool on
/// a transient `Failed` with exponential backoff (see [`super::retry`]). The
/// backoff is cancellable — a job-level cancel (incl. the cross-process watcher
/// tripping the token) during the wait stops retrying and settles `Cancelled`.
/// Non-eligible tools and non-`Failed` terminals settle on the first attempt.
async fn run_tool_with_retry(
    tool_name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    max_secs: u64,
    cancel_token: &CancellationToken,
) -> Result<String, JobError> {
    // Retry-eligible tools must never be tail-streaming tools: the `output_tail`
    // ring is registered ONCE before this loop, so a retried tail tool would
    // re-stream into a stale ring. `exec` (the only tail tool) is not eligible;
    // this guards the invariant if the allowlist ever drifts.
    debug_assert!(
        !(super::retry::is_retry_eligible(tool_name) && ctx.output_tail_job_id.is_some()),
        "retry-eligible tool '{tool_name}' must not register an output_tail ring",
    );
    let retry_cfg = super::retry::RetryConfig::current();
    let mut attempt: u32 = 1;
    loop {
        let err = match run_tool_once(tool_name, args, ctx, max_secs, cancel_token).await {
            Ok(out) => return Ok(out),
            Err(e) => e,
        };
        // Cancel wins any tie: if the token is already cancelled (e.g. the
        // dispatch arm of run_tool_once's select happened to win over a
        // simultaneously-ready cancel), settle Cancelled now — don't log a
        // spurious "retrying" line for a job that's actually being cancelled.
        if cancel_token.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        let backoff_ms = match super::retry::decide(tool_name, attempt, &err, &retry_cfg) {
            super::retry::RetryDecision::Stop => {
                // On a retried-then-exhausted failure, annotate the terminal
                // error so the injected `<task-notification>` + persisted row
                // reflect that it was retried (audit parity with the rest of the
                // subsystem). `attempt` is the count of attempts actually made.
                return Err(annotate_retry_attempts(err, attempt));
            }
            super::retry::RetryDecision::Retry { backoff_ms } => backoff_ms,
        };
        // Cancellable backoff. Log AFTER the wait so the "retrying" line is
        // emitted only when we actually proceed to re-dispatch (a cancel during
        // the wait settles Cancelled without a misleading retry log).
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
            _ = cancel_token.cancelled() => return Err(JobError::Cancelled),
        }
        app_warn!(
            "async_jobs",
            "retry",
            "Background tool '{}' attempt {} failed transiently; retrying (attempt {})",
            tool_name,
            attempt,
            attempt + 1
        );
        attempt += 1;
    }
}

/// Append a "(failed after N attempts)" marker to a retried job's terminal error
/// so a retried-then-failed job is distinguishable from a single failure in the
/// injected notification and the persisted `error` column. No-op for `attempt
/// <= 1` (never retried) and for non-`Failed` terminals (cancels/timeouts/denials
/// are returned verbatim — they're never reached with `attempt > 1`).
fn annotate_retry_attempts(err: JobError, attempt: u32) -> JobError {
    match err {
        JobError::Failed { message } if attempt > 1 => JobError::Failed {
            message: format!("{message} (failed after {attempt} attempts)"),
        },
        other => other,
    }
}

async fn run_job_to_completion(
    db: Arc<JobsDB>,
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

    // R8: install the approval bridge for this job-runner thread so an *attended*
    // approval gate hit during the dispatch (the explicit-background `exec`
    // command gate, now deferred to here) parks the row at AwaitingApproval for
    // the duration of the wait and reverts to Running on resolve. No-op unless
    // the dispatch actually blocks on a human. Held across the whole dispatch;
    // the RAII scope clears the thread-local at function end.
    let _approval_scope = super::approval_bridge::install(
        db.clone(),
        job_id.clone(),
        tool_name.clone(),
        session_id.clone(),
    );

    // I4: cross-process cancel watcher (shared with the auto-background worker).
    // Aborted once the job settles below.
    let poll_handle =
        spawn_cross_process_cancel_watcher(db.clone(), job_id.clone(), cancel_token.clone());

    // R7.4: dispatch with retry (re-attempts a retry-eligible tool on a transient
    // `Failed`; a single attempt for everything else). The cross-process cancel
    // watcher above stays live across attempts + backoffs.
    let result = run_tool_with_retry(&tool_name, &args, &ctx, max_secs, &cancel_token).await;

    // The job has settled — stop the cross-process cancel watcher (no-op if it
    // already exited because the token was cancelled).
    poll_handle.abort();
    // R8: terminal cleanup of any parked-approval record (idempotent — `on_resume`
    // already removed it on the normal path; this covers any stray).
    super::approval_bridge::forget(&job_id);

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
    db: &JobsDB,
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
            (JobStatus::Completed, Some(preview), path, None)
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
    // R3: drop the running-output tail ring; the full result is now on disk
    // (`result_path`) / in the preview. No-op for jobs that never had one.
    super::output_tail::remove(job_id);
    if !updated {
        return;
    }

    // Wake per-job `job_status(block=true)` waiters; the EventBus emit below
    // is retained for frontend subscribers only.
    super::wait::notify_completion(job_id);
    emit_completion_event(job_id, tool_name, status.as_str(), session_id);

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
    if status == JobStatus::Cancelled {
        let _ = db.mark_injected(job_id);
    } else if let Some(sid) = session_id {
        // R4: buffer through the per-session completion merge window so several
        // jobs finishing close together inject as ONE billed turn. Falls back to
        // immediate single injection when the window is 0 (disabled).
        injection::enqueue_injection(
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
    let path = match crate::paths::background_job_result_path(job_id) {
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

fn emit_completion_event(job_id: &str, tool_name: &str, status: &str, session_id: Option<&str>) {
    // R3: unified `job:*` namespace. finalize_job is the Tool executor's terminal
    // path, so `kind = Tool`.
    super::events::emit_completed(
        job_id,
        super::types::JobKind::Tool,
        tool_name,
        status,
        session_id,
    );
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

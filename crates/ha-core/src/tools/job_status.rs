//! `job_status` tool — snapshot an async tool job.
//!
//! Always available (deferred / discoverable via `tool_search`). Used by the
//! model for quick status checks while the real completion is delivered by
//! `<task-notification>` auto-injection. The implementation still accepts a
//! hidden `block=true` escape hatch for existing callers, but clamps it to a
//! short wait so a status poll cannot hold the chat UI hostage.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

use crate::async_jobs::{self, wait, JobKind, JobStatus};

const DEFAULT_WAIT_SECS: u64 = 5;
const MAX_BLOCK_WAIT_SECS: u64 = 10;
const STATUS_SNAPSHOT_COOLDOWN_SECS: i64 = 30;
const INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const MAX_BACKOFF: Duration = Duration::from_secs(2);

/// Scope guard that cleans the wait registry on every return path,
/// including `?` early-returns from DB reads. The `Arc<Notify>` must stay
/// alive until cleanup so the strong-count check in
/// [`wait::cleanup_if_last_waiter`] reflects this waiter's ownership.
struct WaiterGuard {
    job_id: String,
    notify: Arc<Notify>,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        wait::cleanup_if_last_waiter(&self.job_id, &self.notify);
    }
}

/// Clamp on the number of jobs `wait`/`list` will enumerate, so a runaway
/// session can't make one tool call fan out unboundedly.
const MAX_WAIT_TARGETS: usize = 32;

pub async fn tool_job_status(args: &Value, session_id: Option<&str>) -> Result<String> {
    // R5: `job_status` is the multi-job surface. `action` selects the verb;
    // omitting it preserves the original single-job `status` behavior so
    // existing callers (and the synthetic `{job_id}` responses) keep working.
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");
    match action {
        // `result` is an explicit alias for `status` — a terminal job's payload
        // already carries result_preview / result_path.
        "status" | "result" => action_status(args).await,
        "list" => action_list(session_id),
        "cancel" => action_cancel(args, session_id).await,
        "wait" => action_wait(args, session_id).await,
        other => Err(anyhow!(
            "job_status: unknown action '{}' (expected status | list | wait | cancel | result)",
            other
        )),
    }
}

async fn action_status(args: &Value) -> Result<String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("job_status: missing required `job_id` parameter"))?;

    let block = args.get("block").and_then(|v| v.as_bool()).unwrap_or(false);
    let requested_timeout_ms = args.get("timeout_ms").and_then(|v| v.as_u64());

    let db =
        async_jobs::get_async_jobs_db().ok_or_else(|| anyhow!("Async jobs DB not initialized"))?;

    let initial = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Unknown job_id: {}", job_id))?;

    if !block || initial.status.is_terminal() {
        return Ok(format_job_response(&initial));
    }

    let _guard = WaiterGuard {
        job_id: job_id.to_string(),
        notify: wait::register_waiter(job_id),
    };

    // Post-register recheck closes two race windows:
    //   (a) `finalize_job` committed between the initial load and register
    //       (we would otherwise miss the notify_waiters fire entirely).
    //   (b) Restart-replay: the DB row is already terminal but the in-memory
    //       registry was freshly initialized with no producer to wake us.
    let recheck = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Job {} disappeared during wait setup", job_id))?;
    if recheck.status.is_terminal() {
        return Ok(format_job_response(&recheck));
    }

    let effective_timeout = compute_effective_timeout(requested_timeout_ms);
    let deadline = std::time::Instant::now() + effective_timeout;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        let sleep_dur = std::cmp::min(backoff, remaining);
        tokio::select! {
            _ = _guard.notify.notified() => {}
            _ = tokio::time::sleep(sleep_dur) => {
                backoff = std::cmp::min(
                    backoff.saturating_mul(3) / 2,
                    MAX_BACKOFF,
                );
            }
        }

        let job = db
            .load(job_id)?
            .ok_or_else(|| anyhow!("Job {} disappeared during wait", job_id))?;
        if job.status.is_terminal() {
            return Ok(format_job_response(&job));
        }
    }

    let final_job = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Job {} disappeared during wait", job_id))?;
    Ok(format_job_response(&final_job))
}

fn compute_effective_timeout(requested_ms: Option<u64>) -> Duration {
    let configured_ceiling = crate::config::cached_config()
        .async_tools
        .job_status_ceiling_secs();
    let ceiling = configured_ceiling.min(MAX_BLOCK_WAIT_SECS).max(1);
    let requested_secs = match requested_ms {
        Some(ms) => ms.saturating_add(999).saturating_div(1000).max(1),
        None => DEFAULT_WAIT_SECS.min(ceiling),
    };
    Duration::from_secs(requested_secs.min(ceiling))
}

fn format_job_response(job: &crate::async_jobs::BackgroundJob) -> String {
    // Single-job status DOES include the running-output tail (that's its point).
    job_response_value(job, true).to_string()
}

/// Build a job's JSON snapshot. `include_output_tail` gates the (potentially
/// ~8KB) running-output tail: `true` for the single-job `status` view, but
/// `false` for `list` (a compact id roster — N×8KB tails would balloon it) and
/// `wait` (its `settled` entries are terminal, so they have no tail anyway).
fn job_response_value(job: &crate::async_jobs::BackgroundJob, include_output_tail: bool) -> Value {
    let mut payload = json!({
        "job_id": job.job_id,
        "kind": job.kind.as_str(),
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
            JobStatus::Completed => {
                if let Some(preview) = &job.result_preview {
                    map.insert("result_preview".to_string(), json!(preview));
                }
                if let Some(path) = &job.result_path {
                    map.insert("result_path".to_string(), json!(path));
                    map.insert(
                        "hint".to_string(),
                        json!("Full result is saved on disk; use the read tool with result_path when you need the details."),
                    );
                }
            }
            JobStatus::Failed
            | JobStatus::TimedOut
            | JobStatus::Interrupted
            | JobStatus::Cancelled => {
                if let Some(err) = &job.error {
                    map.insert("error".to_string(), json!(err));
                }
            }
            JobStatus::Running => {
                insert_running_poll_guidance(map, job);
                if include_output_tail {
                    insert_output_tail(map, job);
                }
                map.insert(
                    "hint".to_string(),
                    json!("Job is still running. Do not wait or repeatedly call job_status in this chat turn; continue independent work if possible, otherwise stop and rely on the auto-injected task-notification. If `output_tail` is present, it shows the most recent output so far — use it to judge progress vs stuck."),
                );
            }
            JobStatus::Cancelling => {
                insert_running_poll_guidance(map, job);
                if include_output_tail {
                    insert_output_tail(map, job);
                }
                map.insert(
                    "hint".to_string(),
                    json!("Cancellation has been requested; the job is shutting down. Do not repeatedly poll in this chat turn; wait for the terminal task-notification."),
                );
            }
            JobStatus::AwaitingApproval => {
                map.insert(
                    "hint".to_string(),
                    json!("This job is NOT executing yet — it is blocked waiting for a human to approve the tool call. Do not claim it is running or report progress on it. Do not repeatedly poll in this chat turn; it resolves once the user answers the approval (you will get a terminal task-notification)."),
                );
            }
            JobStatus::Queued => {
                map.insert(
                    "hint".to_string(),
                    json!("This job is NOT executing yet — it is queued, waiting for a free background slot (the concurrency cap is full). It starts automatically when a slot frees, and you will get the auto-injected task-notification on completion. Do not claim it is running, and do not repeatedly poll in this chat turn."),
                );
            }
        }

        // R5: a Group is a fan-out join coordinator — surface N-of-M child
        // progress and steer the agent toward the single merged injection
        // (overriding the generic running/terminal hint above). Child results
        // live in the subagent records, not this row.
        if job.kind == JobKind::Group {
            if let Some((total, terminal, completed, failed)) =
                async_jobs::JobManager::group_progress(&job.job_id)
            {
                map.insert("child_count".to_string(), json!(total));
                map.insert("children_terminal".to_string(), json!(terminal));
                map.insert("children_completed".to_string(), json!(completed));
                map.insert("children_failed".to_string(), json!(failed));
                let hint = if job.status.is_terminal() {
                    format!(
                        "Background batch finished ({completed} completed, {failed} failed). The \
                         merged results are delivered as ONE task-notification — read that, don't poll."
                    )
                } else {
                    format!(
                        "Background batch in progress ({terminal}/{total} sub-agents finished). All \
                         results arrive together as ONE task-notification when the batch completes — \
                         do not poll; end your turn and continue when it arrives."
                    )
                };
                map.insert("hint".to_string(), json!(hint));
            }
        }

        // R6: a subagent projection carries NO run content (result/error live in
        // the subagent record). Surface the run id and, when terminal, point the
        // agent at the subagent tool to read the actual result.
        if job.kind == JobKind::Subagent {
            if let Some(run_id) = &job.subagent_run_id {
                map.insert("subagent_run_id".to_string(), json!(run_id));
                if job.status.is_terminal() {
                    map.insert(
                        "hint".to_string(),
                        json!(format!(
                            "This is a background subagent run; its result/error lives in the subagent \
                             record, not in this job row. Fetch it with subagent(action='result', run_id='{run_id}')."
                        )),
                    );
                }
            }
        }
    }
    payload
}

/// Attach the running-output tail (R3 ①) for a still-running job, if one was
/// captured (backgrounded, non-incognito `exec`). Lets the agent peek the latest
/// output of a long job without waiting — judging "progressing" vs "stuck".
fn insert_output_tail(
    map: &mut serde_json::Map<String, Value>,
    job: &crate::async_jobs::BackgroundJob,
) {
    if let Some(tail) = crate::async_jobs::output_tail::read(&job.job_id) {
        if !tail.is_empty() {
            map.insert("output_tail".to_string(), json!(tail));
        }
    }
}

fn insert_running_poll_guidance(
    map: &mut serde_json::Map<String, Value>,
    job: &crate::async_jobs::BackgroundJob,
) {
    let age_secs = chrono::Utc::now()
        .timestamp()
        .saturating_sub(job.created_at)
        .max(0);
    let next_check_after_secs = STATUS_SNAPSHOT_COOLDOWN_SECS
        .saturating_sub(age_secs)
        .max(0);
    map.insert("age_secs".to_string(), json!(age_secs));
    map.insert(
        "polling_guidance".to_string(),
        json!({
            "should_poll_again_this_turn": false,
            "next_check_after_secs": next_check_after_secs,
            "completion_channel": "task-notification",
            "instruction": "Do not call job_status again in this chat turn just to wait. Continue independent work, answer that the job is still running, or wait for the auto-injected task-notification."
        }),
    );
}

/// R5 `list`: enumerate the session's in-flight background jobs (active =
/// non-terminal). Lets the model recover job ids it lost track of and see what
/// it has running, even after the synthetic `{job_id}` scrolled out of context.
fn action_list(session_id: Option<&str>) -> Result<String> {
    let session_id = session_id
        .ok_or_else(|| anyhow!("job_status list: no active session to enumerate jobs for"))?;
    let db =
        async_jobs::get_async_jobs_db().ok_or_else(|| anyhow!("Async jobs DB not initialized"))?;
    let mut jobs = db.list_active_by_session(session_id)?;
    jobs.sort_by_key(|j| j.created_at);
    let truncated = jobs.len() > MAX_WAIT_TARGETS;
    let items: Vec<Value> = jobs
        .iter()
        .take(MAX_WAIT_TARGETS)
        // No output_tail: list is an id roster, not a bulk-output dump.
        .map(|j| job_response_value(j, false))
        .collect();
    Ok(json!({
        "action": "list",
        "count": items.len(),
        "truncated": truncated,
        "jobs": items,
    })
    .to_string())
}

/// R5 `cancel`: best-effort cancel a specific job by id (reuses the cross-process
/// cancel path). Returns the updated snapshot.
///
/// Session-scoped: a job owned by another session cannot be cancelled from here.
/// The async_jobs DB is shared across desktop / HTTP / IM / cron (and across
/// processes), so without this check a model in session A could terminate
/// background work belonging to session B (another user's IM/cron turn).
async fn action_cancel(args: &Value, session_id: Option<&str>) -> Result<String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("job_status cancel: missing required `job_id`"))?;
    let db =
        async_jobs::get_async_jobs_db().ok_or_else(|| anyhow!("Async jobs DB not initialized"))?;
    let job = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Unknown job_id: {}", job_id))?;
    // A job that belongs to a session may only be cancelled from that same
    // session. Jobs with no session (system/orphan) are cancellable by anyone.
    if let Some(owner) = job.session_id.as_deref() {
        if session_id != Some(owner) {
            return Err(anyhow!(
                "job {} belongs to a different session and cannot be cancelled from here",
                job_id
            ));
        }
    }
    match async_jobs::JobManager::cancel(job_id)? {
        Some(job) => {
            Ok(json!({ "action": "cancel", "job": job_response_value(&job, false) }).to_string())
        }
        None => Err(anyhow!("Unknown job_id: {}", job_id)),
    }
}

/// R5 `wait`: a SHORT convenience sync for fast jobs — clamped to
/// `MAX_BLOCK_WAIT_SECS`, never a long block (turn=1 single-flight would
/// otherwise lock the session). Targets are an explicit `ids` array, or every
/// active job in the session when omitted. `mode` ∈ {all (default), any}. On
/// clamp expiry it returns the still-running ids and steers the model to the
/// auto-injection path rather than pretending the work is done.
async fn action_wait(args: &Value, session_id: Option<&str>) -> Result<String> {
    let db =
        async_jobs::get_async_jobs_db().ok_or_else(|| anyhow!("Async jobs DB not initialized"))?;

    let ids: Vec<String> = match args.get("ids").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        None => {
            let session_id = session_id
                .ok_or_else(|| anyhow!("job_status wait: provide `ids` or run within a session"))?;
            db.list_active_by_session(session_id)?
                .into_iter()
                .map(|j| j.job_id)
                .collect()
        }
    };
    if ids.is_empty() {
        return Ok(json!({
            "action": "wait",
            "settled": [],
            "still_running": [],
            "note": "No matching in-flight jobs to wait for."
        })
        .to_string());
    }
    // Cap the fan-out, but SURFACE what we dropped — silently truncating would
    // let the model read "absent from still_running" as "finished" for ids it
    // explicitly asked about. (action_list reports `truncated` the same way.)
    let dropped: Vec<String> = if ids.len() > MAX_WAIT_TARGETS {
        ids[MAX_WAIT_TARGETS..].to_vec()
    } else {
        Vec::new()
    };
    let ids: Vec<String> = ids.into_iter().take(MAX_WAIT_TARGETS).collect();
    let wait_any = args.get("mode").and_then(|v| v.as_str()) == Some("any");

    // Register a waiter per id so producers wake us; cleaned up on every return
    // (WaiterGuard::drop). Kept alive for the whole wait so the notifies stay
    // registered.
    let guards: Vec<WaiterGuard> = ids
        .iter()
        .map(|id| WaiterGuard {
            job_id: id.clone(),
            notify: wait::register_waiter(id),
        })
        .collect();

    let effective_timeout =
        compute_effective_timeout(args.get("timeout_ms").and_then(|v| v.as_u64()));
    let deadline = std::time::Instant::now() + effective_timeout;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        // Snapshot current statuses.
        let mut settled: Vec<Value> = Vec::new();
        let mut still_running: Vec<String> = Vec::new();
        // Track real terminal settlements separately from unknown ids: an
        // unknown/typo'd id must NOT count as "any settled" (it would let
        // mode=any return immediately while real targets are still running).
        let mut saw_real_terminal = false;
        for id in &ids {
            match db.load(id)? {
                Some(job) if job.status.is_terminal() => {
                    saw_real_terminal = true;
                    settled.push(job_response_value(&job, false));
                }
                Some(_) => still_running.push(id.clone()),
                // Unknown id: report it as settled-unknown so the model isn't
                // left waiting forever on a typo'd / purged id (but it doesn't
                // count toward mode=any).
                None => settled.push(json!({ "job_id": id, "status": "unknown" })),
            }
        }
        // Done when nothing is left to wait for, or (mode=any) a real job settled.
        let done = still_running.is_empty() || (wait_any && saw_real_terminal);
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if done || remaining.is_zero() {
            let note = match (!dropped.is_empty(), still_running.is_empty()) {
                // Too many ids — surface BOTH the dropped overflow AND that the
                // 32 we did wait on may still be running.
                (true, _) => {
                    "Too many ids: only the first 32 were waited on; the rest are in `dropped` \
                     and were NOT checked. Any ids still in `still_running` haven't finished \
                     either. End your turn and rely on the auto-injected task-notification."
                }
                (false, true) => "All target jobs reached a terminal state.",
                (false, false) => {
                    "Returned before all jobs finished (wait is capped at a few seconds). \
                     Do not keep calling wait — end your turn and rely on the auto-injected \
                     task-notification for the remaining jobs."
                }
            };
            return Ok(json!({
                "action": "wait",
                "mode": if wait_any { "any" } else { "all" },
                "settled": settled,
                "still_running": still_running,
                "dropped": dropped,
                "truncated": !dropped.is_empty(),
                "note": note,
            })
            .to_string());
        }

        let sleep_dur = std::cmp::min(backoff, remaining);
        // Wake as soon as ANY waited id's producer fires (not just the first),
        // or on the backoff timer. The notifies are re-armed each iteration; the
        // poll re-checks the DB regardless, so a missed wake only costs latency.
        let notifies = guards
            .iter()
            .map(|g| Box::pin(g.notify.notified()))
            .collect::<Vec<_>>();
        tokio::select! {
            _ = futures_util::future::select_all(notifies) => {}
            _ = tokio::time::sleep(sleep_dur) => {
                backoff = std::cmp::min(backoff.saturating_mul(3) / 2, MAX_BACKOFF);
            }
        }
    }
}

/// Tool definition for `job_status` — feature-gated core meta tool.
pub fn get_job_status_tool() -> super::definitions::ToolDefinition {
    super::definitions::ToolDefinition {
        name: super::TOOL_JOB_STATUS.into(),
        description: "Inspect and manage async tool jobs created by `run_in_background: true` \
            or auto-backgrounded by the runtime. Actions: `status` (default) — snapshot one job by \
            `job_id`; `list` — enumerate this session's in-flight jobs (recover ids you lost track \
            of); `wait` — a SHORT (capped at a few seconds) convenience sync for fast jobs, over \
            an explicit `ids` array or all of the session's active jobs, with `mode` all|any; \
            `cancel` — cancel a job by `job_id`; `result` — alias for status. \
            For a still-running backgrounded `exec`, `status` includes `output_tail` (the most \
            recent ~8KB of output) so you can judge progressing-vs-stuck WITHOUT waiting. \
            This is NOT how you collect long fan-out: do not poll `status` or hammer `wait` in a \
            loop — end your turn and rely on the auto-injected `<task-notification>` for completion. \
            Read `result_path` only when you need the full output."
            .into(),
        tier: super::definitions::ToolTier::Core {
            subclass: super::definitions::CoreSubclass::Meta,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "list", "wait", "cancel", "result"],
                    "description": "What to do. Defaults to `status`. `status`/`cancel`/`result` need `job_id`; `list` needs none; `wait` takes optional `ids`/`mode`/`timeout_ms`."
                },
                "job_id": {
                    "type": "string",
                    "description": "The job id from the synthetic tool response (e.g. 'job_<uuid>'). Required for status / cancel / result."
                },
                "ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "For action=wait: the specific job ids to wait on. Omit to wait on every in-flight job in this session."
                },
                "mode": {
                    "type": "string",
                    "enum": ["all", "any"],
                    "description": "For action=wait: return when ALL targets settle (default) or as soon as ANY one does."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "For action=wait/status(block): upper bound on the wait, clamped to a few seconds — wait never long-blocks."
                }
            },
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_jobs::{
        self,
        db::JobsDB,
        types::{BackgroundJob, JobKind, JobOrigin, JobStatus},
    };
    use crate::runtime_tasks::{cancel_runtime_task, RuntimeTaskKind};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
    use std::time::Instant;
    use tokio::time::timeout;

    // The async_jobs DB is a process-global `OnceLock`, so all tests in
    // this module share one fixture and serialize through `TEST_LOCK`.
    static FIXTURE: OnceLock<TestFixturePath> = OnceLock::new();
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn test_lock() -> MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct TestFixturePath {
        // Held only so the module keeps ownership; cleanup on process exit
        // is acceptable for a unit-test temp SQLite file.
        _path: PathBuf,
    }

    fn ensure_fixture() -> &'static TestFixturePath {
        FIXTURE.get_or_init(|| {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "ha-core-job-status-tests-{}.db",
                uuid::Uuid::new_v4().simple()
            ));
            let db = JobsDB::open(&path).expect("open db");
            async_jobs::set_async_jobs_db(Arc::new(db));
            TestFixturePath { _path: path }
        })
    }

    fn fresh_id() -> String {
        format!("itjob_{}", uuid::Uuid::new_v4().simple())
    }

    fn insert_running(job_id: &str) {
        let db = async_jobs::get_async_jobs_db().expect("db");
        let job = BackgroundJob {
            job_id: job_id.to_string(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: None,
            agent_id: None,
            tool_name: "test_tool".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            injected: false,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        db.insert(&job).expect("insert");
    }

    fn finalize_ok(job_id: &str, preview: &str) {
        let db = async_jobs::get_async_jobs_db().expect("db");
        db.update_terminal(
            job_id,
            JobStatus::Completed,
            Some(preview),
            None,
            None,
            chrono::Utc::now().timestamp(),
        )
        .expect("update_terminal");
        wait::notify_completion(job_id);
    }

    #[tokio::test]
    async fn block_wakes_on_completion_via_notify() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        let start = Instant::now();
        let args = json!({ "job_id": job_id, "block": true, "timeout_ms": 5000 });

        let task_id = job_id.clone();
        let finisher = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            finalize_ok(&task_id, "ok");
        });

        let out = tool_job_status(&args, None).await.expect("tool ok");
        finisher.await.expect("finisher ok");
        let elapsed = start.elapsed();
        let max_notify_elapsed = if cfg!(windows) {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(400)
        };

        assert!(out.contains("\"status\":\"completed\""), "got {out}");
        assert!(
            elapsed < max_notify_elapsed,
            "should return promptly via notify path, took {elapsed:?}"
        );
        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn block_terminal_on_restart_replay() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        // Mark terminal directly, without touching the wait registry —
        // simulates the state after `replay_pending_jobs` on startup.
        let db = async_jobs::get_async_jobs_db().expect("db");
        db.update_terminal(
            &job_id,
            JobStatus::Interrupted,
            None,
            None,
            Some("interrupted"),
            chrono::Utc::now().timestamp(),
        )
        .expect("update_terminal");

        let start = Instant::now();
        let args = json!({ "job_id": job_id, "block": true, "timeout_ms": 5000 });
        let out = tool_job_status(&args, None).await.expect("tool ok");
        let elapsed = start.elapsed();

        assert!(out.contains("\"status\":\"interrupted\""), "got {out}");
        assert!(
            elapsed < Duration::from_millis(50),
            "must return without parking, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cleanup_after_completion_leaves_empty_registry() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        let task_id = job_id.clone();
        let finisher = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            finalize_ok(&task_id, "ok");
        });

        let args = json!({ "job_id": job_id, "block": true, "timeout_ms": 2000 });
        tool_job_status(&args, None).await.expect("tool ok");
        finisher.await.expect("finisher ok");

        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn snapshot_mode_returns_immediately() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        let args = json!({ "job_id": job_id, "block": false });
        let out = tool_job_status(&args, None).await.expect("tool ok");
        assert!(out.contains("\"status\":\"running\""), "got {out}");
        // No waiter should have been registered.
        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn cancel_running_job_wakes_waiter_and_finishes_cancelled() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        let token = crate::async_jobs::cancel::register_job(&job_id);

        let wait_args = json!({ "job_id": job_id, "block": true, "timeout_ms": 5000 });
        let waiter = tokio::spawn(async move { tool_job_status(&wait_args, None).await });

        tokio::time::sleep(Duration::from_millis(20)).await;
        let result = cancel_runtime_task(RuntimeTaskKind::AsyncJob, &job_id)
            .await
            .expect("cancel");
        assert!(result.accepted);
        assert_eq!(result.status, "cancelling");
        assert!(token.is_cancelled());

        let db = async_jobs::get_async_jobs_db().expect("db");
        db.update_terminal(
            &job_id,
            JobStatus::Cancelled,
            None,
            None,
            Some("cancelled"),
            chrono::Utc::now().timestamp(),
        )
        .expect("update_terminal");
        db.mark_injected(&job_id).expect("mark injected");
        crate::async_jobs::cancel::remove_job(&job_id);
        wait::notify_completion(&job_id);

        let out = timeout(Duration::from_millis(500), waiter)
            .await
            .expect("waiter wakes")
            .expect("waiter join")
            .expect("tool ok");
        assert!(out.contains("\"status\":\"cancelled\""), "got {out}");
        let job = db.load(&job_id).expect("load").expect("job exists");
        assert!(job.injected, "cancelled jobs must not be replay-injected");
        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn cancelling_completed_job_is_noop() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        finalize_ok(&job_id, "done");

        let result = cancel_runtime_task(RuntimeTaskKind::AsyncJob, &job_id)
            .await
            .expect("cancel");
        assert!(!result.accepted);
        assert_eq!(result.status, "completed");
        let db = async_jobs::get_async_jobs_db().expect("db");
        let job = db.load(&job_id).expect("load").expect("job exists");
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.result_preview.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn unknown_job_id_errors() {
        let _guard = test_lock();
        ensure_fixture();
        let args = json!({ "job_id": "nonexistent", "block": false });
        let err = tool_job_status(&args, None).await.unwrap_err();
        assert!(err.to_string().contains("Unknown job_id"));
    }

    // ── R5: multi-job actions (list / cancel / wait) ──────────────

    fn insert_running_in_session(job_id: &str, session_id: &str) {
        let db = async_jobs::get_async_jobs_db().expect("db");
        let job = BackgroundJob {
            job_id: job_id.to_string(),
            kind: JobKind::Tool,
            subagent_run_id: None,
            group_id: None,
            session_id: Some(session_id.to_string()),
            agent_id: None,
            tool_name: "test_tool".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            injected: false,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        db.insert(&job).expect("insert");
    }

    #[tokio::test]
    async fn action_list_enumerates_only_this_sessions_active_jobs() {
        let _guard = test_lock();
        ensure_fixture();
        let sid = format!("sess-{}", uuid::Uuid::new_v4().simple());
        let other = format!("sess-{}", uuid::Uuid::new_v4().simple());
        let a = fresh_id();
        let b = fresh_id();
        let c = fresh_id();
        insert_running_in_session(&a, &sid);
        insert_running_in_session(&b, &sid);
        insert_running_in_session(&c, &other);

        let out = tool_job_status(&json!({ "action": "list" }), Some(&sid))
            .await
            .expect("list ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["count"], 2);
        let ids: Vec<&str> = v["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j["job_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&a.as_str()) && ids.contains(&b.as_str()));
        assert!(!ids.contains(&c.as_str()));
    }

    #[tokio::test]
    async fn action_list_without_session_errors() {
        let _guard = test_lock();
        ensure_fixture();
        let err = tool_job_status(&json!({ "action": "list" }), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no active session"));
    }

    #[tokio::test]
    async fn action_wait_reports_still_running_on_timeout_then_settles() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        // Tight timeout → returns still_running (never long-blocks).
        let out = tool_job_status(
            &json!({ "action": "wait", "ids": [job_id], "timeout_ms": 100 }),
            None,
        )
        .await
        .expect("wait ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["still_running"].as_array().unwrap().len(), 1);
        assert!(v["settled"].as_array().unwrap().is_empty());

        // Once terminal, wait returns it settled with no still_running.
        finalize_ok(&job_id, "done");
        let out = tool_job_status(
            &json!({ "action": "wait", "ids": [job_id], "timeout_ms": 100 }),
            None,
        )
        .await
        .expect("wait ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["still_running"].as_array().unwrap().is_empty());
        assert_eq!(v["settled"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn action_wait_unknown_id_settles_as_unknown_not_forever() {
        let _guard = test_lock();
        ensure_fixture();
        let out = tool_job_status(
            &json!({ "action": "wait", "ids": ["no-such-id"], "timeout_ms": 5000 }),
            None,
        )
        .await
        .expect("wait ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        // mode=all is satisfied immediately because the unknown id counts as settled.
        let settled = v["settled"].as_array().unwrap();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0]["status"], "unknown");
    }

    #[tokio::test]
    async fn action_cancel_unknown_errors() {
        let _guard = test_lock();
        ensure_fixture();
        let err = tool_job_status(&json!({ "action": "cancel", "job_id": "nope" }), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Unknown job_id"));
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let _guard = test_lock();
        ensure_fixture();
        let err = tool_job_status(&json!({ "action": "frobnicate" }), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[tokio::test]
    async fn status_surfaces_output_tail_while_running() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        async_jobs::output_tail::register(&job_id, 8192);
        async_jobs::output_tail::append(&job_id, b"compiling...\nlinking...\n");

        let out = tool_job_status(&json!({ "job_id": job_id }), None)
            .await
            .expect("ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "running");
        assert!(v["output_tail"].as_str().unwrap().contains("linking..."));
        async_jobs::output_tail::remove(&job_id);
    }

    #[tokio::test]
    async fn action_cancel_rejects_cross_session() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running_in_session(&job_id, "owner-sess");

        // A different session must NOT be able to cancel another session's job.
        let err = tool_job_status(
            &json!({ "action": "cancel", "job_id": job_id }),
            Some("other-sess"),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("different session"), "got {err}");

        // The owning session can.
        let out = tool_job_status(
            &json!({ "action": "cancel", "job_id": job_id }),
            Some("owner-sess"),
        )
        .await
        .expect("owner cancel ok");
        assert!(out.contains("\"action\":\"cancel\""));
    }

    #[tokio::test]
    async fn action_list_omits_output_tail() {
        let _guard = test_lock();
        ensure_fixture();
        let sid = format!("sess-{}", uuid::Uuid::new_v4().simple());
        let job_id = fresh_id();
        insert_running_in_session(&job_id, &sid);
        // Even with a populated tail, `list` must not embed it (id roster only).
        async_jobs::output_tail::register(&job_id, 8192);
        async_jobs::output_tail::append(&job_id, b"lots of build output\n");

        let out = tool_job_status(&json!({ "action": "list" }), Some(&sid))
            .await
            .expect("list ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        let job0 = &v["jobs"][0];
        assert_eq!(job0["status"], "running");
        assert!(
            job0.get("output_tail").is_none(),
            "list must omit output_tail"
        );
        async_jobs::output_tail::remove(&job_id);
    }

    #[tokio::test]
    async fn status_omits_output_tail_when_none_registered() {
        let _guard = test_lock();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        let out = tool_job_status(&json!({ "job_id": job_id }), None)
            .await
            .expect("ok");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], "running");
        assert!(v.get("output_tail").is_none());
    }

    /// R6: a subagent projection's status response advertises kind=subagent +
    /// the FK run id, and (when terminal) directs the agent to fetch the actual
    /// result via the subagent tool — the projection itself carries no content.
    #[test]
    fn subagent_projection_response_surfaces_run_id_and_fetch_hint() {
        let job = BackgroundJob {
            job_id: "job_sa".into(),
            kind: JobKind::Subagent,
            subagent_run_id: Some("run_xyz".into()),
            group_id: None,
            session_id: Some("s1".into()),
            agent_id: Some("ha-main".into()),
            tool_name: "subagent:researcher".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: JobStatus::Completed,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: 0,
            completed_at: Some(5),
            injected: true,
            origin: JobOrigin::Explicit.as_str().to_string(),
            approval_origin: None,
            incognito: false,
            pid: None,
            cancel_requested: false,
        };
        let v = job_response_value(&job, true);
        assert_eq!(v["kind"], "subagent");
        assert_eq!(v["subagent_run_id"], "run_xyz");
        assert!(
            v["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("subagent(action='result'"),
            "terminal subagent projection must point at the subagent result tool"
        );
        // The projection never carries run content.
        assert!(v.get("result_preview").is_none());
        assert!(v.get("output_tail").is_none());
    }
}

//! `job_status` tool — snapshot or actively wait on an async tool job.
//!
//! Always available (deferred / discoverable via `tool_search`). Used by the
//! model to block on a backgrounded tool call instead of waiting for the
//! auto-injection. See [`crate::async_jobs::wait`] for the wait registry's
//! invariants and [`crate::config::AsyncToolsConfig::job_status_ceiling_secs`]
//! for the timeout cap rule.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

use crate::async_jobs::{self, wait, AsyncJobStatus};

const DEFAULT_WAIT_SECS: u64 = 1800;
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

pub async fn tool_job_status(args: &Value) -> Result<String> {
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
    let ceiling = crate::config::cached_config()
        .async_tools
        .job_status_ceiling_secs();
    let requested_secs = match requested_ms {
        Some(ms) => (ms / 1000).max(1),
        None => DEFAULT_WAIT_SECS.min(ceiling),
    };
    Duration::from_secs(requested_secs.min(ceiling))
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
            AsyncJobStatus::Failed
            | AsyncJobStatus::TimedOut
            | AsyncJobStatus::Interrupted
            | AsyncJobStatus::Cancelled => {
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
            AsyncJobStatus::Cancelling => {
                map.insert(
                    "hint".to_string(),
                    json!("Cancellation has been requested; the job is shutting down."),
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
                    "description": "Max milliseconds to wait when block=true. Defaults to min(max_job_secs, 1800) seconds. Hard cap = max_job_secs (or asyncTools.jobStatusMaxWaitSecs when max_job_secs=0). Values above the cap are clamped.",
                    "minimum": 1
                }
            },
            "required": ["job_id"],
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_jobs::{
        self,
        db::AsyncJobsDB,
        types::{AsyncJob, AsyncJobStatus, JobOrigin},
    };
    use crate::runtime_tasks::{cancel_runtime_task, RuntimeTaskKind};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Instant;
    use tokio::time::timeout;

    // The async_jobs DB is a process-global `OnceLock`, so all tests in
    // this module share one fixture and serialize through `TEST_LOCK`.
    static FIXTURE: OnceLock<TestFixturePath> = OnceLock::new();
    static TEST_LOCK: Mutex<()> = Mutex::new(());

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
            let db = AsyncJobsDB::open(&path).expect("open db");
            async_jobs::set_async_jobs_db(Arc::new(db));
            TestFixturePath { _path: path }
        })
    }

    fn fresh_id() -> String {
        format!("itjob_{}", uuid::Uuid::new_v4().simple())
    }

    fn insert_running(job_id: &str) {
        let db = async_jobs::get_async_jobs_db().expect("db");
        let job = AsyncJob {
            job_id: job_id.to_string(),
            session_id: None,
            agent_id: None,
            tool_name: "test_tool".into(),
            tool_call_id: None,
            args_json: "{}".into(),
            status: AsyncJobStatus::Running,
            result_preview: None,
            result_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            injected: false,
            origin: JobOrigin::Explicit.as_str().to_string(),
        };
        db.insert(&job).expect("insert");
    }

    fn finalize_ok(job_id: &str, preview: &str) {
        let db = async_jobs::get_async_jobs_db().expect("db");
        db.update_terminal(
            job_id,
            AsyncJobStatus::Completed,
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
        let _guard = TEST_LOCK.lock().unwrap();
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

        let out = tool_job_status(&args).await.expect("tool ok");
        finisher.await.expect("finisher ok");
        let elapsed = start.elapsed();

        assert!(out.contains("\"status\":\"completed\""), "got {out}");
        assert!(
            elapsed < Duration::from_millis(400),
            "should return promptly via notify path, took {elapsed:?}"
        );
        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn block_terminal_on_restart_replay() {
        let _guard = TEST_LOCK.lock().unwrap();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        // Mark terminal directly, without touching the wait registry —
        // simulates the state after `replay_pending_jobs` on startup.
        let db = async_jobs::get_async_jobs_db().expect("db");
        db.update_terminal(
            &job_id,
            AsyncJobStatus::Interrupted,
            None,
            None,
            Some("interrupted"),
            chrono::Utc::now().timestamp(),
        )
        .expect("update_terminal");

        let start = Instant::now();
        let args = json!({ "job_id": job_id, "block": true, "timeout_ms": 5000 });
        let out = tool_job_status(&args).await.expect("tool ok");
        let elapsed = start.elapsed();

        assert!(out.contains("\"status\":\"interrupted\""), "got {out}");
        assert!(
            elapsed < Duration::from_millis(50),
            "must return without parking, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn cleanup_after_completion_leaves_empty_registry() {
        let _guard = TEST_LOCK.lock().unwrap();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        let task_id = job_id.clone();
        let finisher = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            finalize_ok(&task_id, "ok");
        });

        let args = json!({ "job_id": job_id, "block": true, "timeout_ms": 2000 });
        tool_job_status(&args).await.expect("tool ok");
        finisher.await.expect("finisher ok");

        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn snapshot_mode_returns_immediately() {
        let _guard = TEST_LOCK.lock().unwrap();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);

        let args = json!({ "job_id": job_id, "block": false });
        let out = tool_job_status(&args).await.expect("tool ok");
        assert!(out.contains("\"status\":\"running\""), "got {out}");
        // No waiter should have been registered.
        assert_eq!(wait::waiter_count(&job_id), 0);
    }

    #[tokio::test]
    async fn cancel_running_job_wakes_waiter_and_finishes_cancelled() {
        let _guard = TEST_LOCK.lock().unwrap();
        ensure_fixture();
        let job_id = fresh_id();
        insert_running(&job_id);
        let token = crate::async_jobs::cancel::register_job(&job_id);

        let wait_args = json!({ "job_id": job_id, "block": true, "timeout_ms": 5000 });
        let waiter = tokio::spawn(async move { tool_job_status(&wait_args).await });

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
            AsyncJobStatus::Cancelled,
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
        let _guard = TEST_LOCK.lock().unwrap();
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
        assert_eq!(job.status, AsyncJobStatus::Completed);
        assert_eq!(job.result_preview.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn unknown_job_id_errors() {
        let _guard = TEST_LOCK.lock().unwrap();
        ensure_fixture();
        let args = json!({ "job_id": "nonexistent", "block": false });
        let err = tool_job_status(&args).await.unwrap_err();
        assert!(err.to_string().contains("Unknown job_id"));
    }
}

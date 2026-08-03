use serde::{Deserialize, Serialize};

/// Runtime work units that can be cancelled best-effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskKind {
    AsyncJob,
    Subagent,
    Process,
    Cron,
}

impl RuntimeTaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AsyncJob => "async_job",
            Self::Subagent => "subagent",
            Self::Process => "process",
            Self::Cron => "cron",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelRuntimeTaskResult {
    pub kind: RuntimeTaskKind,
    pub id: String,
    pub accepted: bool,
    pub status: String,
    pub message: String,
}

/// Exact runtime identities captured while a stopped session is still gated.
/// Cancellation may finish later, but it must never re-enumerate the session
/// and accidentally pick up work from a replacement turn.
#[derive(Debug, Clone, Default)]
pub struct RuntimeTaskSnapshot {
    tasks: Vec<(RuntimeTaskKind, String)>,
}

impl CancelRuntimeTaskResult {
    fn new(
        kind: RuntimeTaskKind,
        id: &str,
        accepted: bool,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            id: id.to_string(),
            accepted,
            status: status.into(),
            message: message.into(),
        }
    }
}

pub async fn cancel_runtime_task(
    kind: RuntimeTaskKind,
    id: &str,
) -> anyhow::Result<CancelRuntimeTaskResult> {
    match kind {
        RuntimeTaskKind::AsyncJob => {
            let id = id.to_string();
            crate::blocking::run_blocking(move || cancel_async_job(&id)).await
        }
        RuntimeTaskKind::Subagent => {
            let id = id.to_string();
            crate::blocking::run_blocking(move || cancel_subagent(&id)).await
        }
        RuntimeTaskKind::Process => cancel_process(id).await,
        RuntimeTaskKind::Cron => {
            let id = id.to_string();
            crate::blocking::run_blocking(move || cancel_cron(&id)).await
        }
    }
}

/// Cancel active runtime work associated with a chat session. `None` is a
/// process-wide emergency stop and intentionally skips cron jobs, which are
/// scheduled runtime work rather than work owned by the visible chat turn.
pub async fn cancel_runtime_tasks_for_session(
    session_id: Option<&str>,
) -> anyhow::Result<Vec<CancelRuntimeTaskResult>> {
    let snapshot = snapshot_runtime_tasks_for_session(session_id).await?;
    cancel_runtime_task_snapshot(snapshot).await
}

/// Capture runtime work associated with a session without performing any
/// cancellation side effects. A caller may safely time this future out: an
/// already-running blocking DB read can finish in the background, but it
/// cannot affect a later turn.
pub async fn snapshot_runtime_tasks_for_session(
    session_id: Option<&str>,
) -> anyhow::Result<RuntimeTaskSnapshot> {
    let owned_session_id = session_id.map(str::to_string);
    let session_for_blocking = owned_session_id.clone();
    let mut tasks = crate::blocking::run_blocking(move || {
        let mut tasks = Vec::new();

        if let Some(db) = crate::async_jobs::get_async_jobs_db() {
            for job in db.list_running()? {
                let matches_session = session_for_blocking
                    .as_deref()
                    .map(|sid| job.session_id.as_deref() == Some(sid))
                    .unwrap_or(true);
                if matches_session {
                    tasks.push((RuntimeTaskKind::AsyncJob, job.job_id));
                }
            }
        }

        if let Some(db) = crate::get_session_db() {
            let runs = match session_for_blocking.as_deref() {
                Some(sid) => db.list_active_subagent_runs(sid)?,
                None => db.list_all_active_subagent_runs()?,
            };
            for run in runs {
                tasks.push((RuntimeTaskKind::Subagent, run.run_id));
            }
        }

        anyhow::Ok(tasks)
    })
    .await?;

    let process_ids = {
        let registry = crate::process_registry::get_registry().lock().await;
        registry.list_running_ids_for_parent_session(owned_session_id.as_deref())
    };
    for process_id in process_ids {
        tasks.push((RuntimeTaskKind::Process, process_id));
    }

    Ok(RuntimeTaskSnapshot { tasks })
}

/// Cancel only the identities in a previously captured snapshot.
pub async fn cancel_runtime_task_snapshot(
    snapshot: RuntimeTaskSnapshot,
) -> anyhow::Result<Vec<CancelRuntimeTaskResult>> {
    let mut results = Vec::with_capacity(snapshot.tasks.len());
    for (kind, id) in snapshot.tasks {
        results.push(cancel_runtime_task(kind, &id).await?);
    }
    Ok(results)
}

fn cancel_async_job(id: &str) -> anyhow::Result<CancelRuntimeTaskResult> {
    let Some(db) = crate::async_jobs::get_async_jobs_db() else {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::AsyncJob,
            id,
            false,
            "not_found",
            "Async jobs DB unavailable",
        ));
    };
    let Some(before) = db.load(id)? else {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::AsyncJob,
            id,
            false,
            "not_found",
            "Async job not found",
        ));
    };
    if before.status.is_terminal() {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::AsyncJob,
            id,
            false,
            before.status.as_str(),
            "Async job is already in a terminal state",
        ));
    }
    match crate::async_jobs::JobManager::cancel(id)? {
        Some(job) => Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::AsyncJob,
            id,
            true,
            job.status.as_str(),
            "Async job cancellation requested",
        )),
        None => Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::AsyncJob,
            id,
            false,
            "not_found",
            "Async job not found",
        )),
    }
}

fn cancel_subagent(id: &str) -> anyhow::Result<CancelRuntimeTaskResult> {
    let db = crate::get_session_db().ok_or_else(|| anyhow::anyhow!("Session DB unavailable"))?;
    let Some(run) = db.get_subagent_run(id)? else {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Subagent,
            id,
            false,
            "not_found",
            "Sub-agent run not found",
        ));
    };
    if run.status.is_terminal() {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Subagent,
            id,
            false,
            run.status.as_str(),
            "Sub-agent is already in a terminal state",
        ));
    }

    // This is the only cancellation entry that atomically claims parked runs,
    // reuses the running token, and synchronizes the background projection.
    let accepted = crate::subagent::request_cancel_run(id);
    Ok(CancelRuntimeTaskResult::new(
        RuntimeTaskKind::Subagent,
        id,
        accepted,
        if accepted {
            "killed"
        } else {
            run.status.as_str()
        },
        if accepted {
            "Sub-agent cancellation requested"
        } else {
            "Sub-agent is no longer active"
        },
    ))
}

async fn cancel_process(id: &str) -> anyhow::Result<CancelRuntimeTaskResult> {
    use crate::process_registry::{get_registry, ProcessStatus};

    crate::process_notification::mark_observed(id);
    let session = {
        let registry = get_registry().lock().await;
        registry.get_session(id).cloned()
    };
    let Some(session) = session else {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Process,
            id,
            false,
            "not_found",
            "Process session not found",
        ));
    };
    if session.exited {
        return Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Process,
            id,
            false,
            session.status.to_string(),
            "Process session has already exited",
        ));
    }
    if let Some(pid) = session.pid {
        crate::blocking::run_blocking(move || crate::platform::terminate_process_tree(pid)).await;
    }
    let mut registry = get_registry().lock().await;
    registry.mark_exited(id, None, Some("SIGKILL".to_string()), ProcessStatus::Failed);
    Ok(CancelRuntimeTaskResult::new(
        RuntimeTaskKind::Process,
        id,
        true,
        "killed",
        "Process session terminated",
    ))
}

fn cancel_cron(id: &str) -> anyhow::Result<CancelRuntimeTaskResult> {
    match crate::cron::cancel_running_job(id)? {
        Some(cancelled) => Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Cron,
            id,
            cancelled,
            if cancelled {
                "cancelling"
            } else {
                "not_running"
            },
            if cancelled {
                "Cron run cancellation requested"
            } else {
                "Cron job is not currently running"
            },
        )),
        None => Ok(CancelRuntimeTaskResult::new(
            RuntimeTaskKind::Cron,
            id,
            false,
            "not_found",
            "Cron job not found",
        )),
    }
}

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
        RuntimeTaskKind::AsyncJob => cancel_async_job(id),
        RuntimeTaskKind::Subagent => cancel_subagent(id),
        RuntimeTaskKind::Process => cancel_process(id).await,
        RuntimeTaskKind::Cron => cancel_cron(id),
    }
}

/// Cancel active runtime work associated with a chat session. `None` is a
/// process-wide emergency stop and intentionally skips cron jobs, which are
/// scheduled runtime work rather than work owned by the visible chat turn.
pub async fn cancel_runtime_tasks_for_session(
    session_id: Option<&str>,
) -> anyhow::Result<Vec<CancelRuntimeTaskResult>> {
    let mut results = Vec::new();

    if let Some(db) = crate::async_jobs::get_async_jobs_db() {
        for job in db.list_running()? {
            let matches_session = session_id
                .map(|sid| job.session_id.as_deref() == Some(sid))
                .unwrap_or(true);
            if matches_session {
                results.push(cancel_async_job(&job.job_id)?);
            }
        }
    }

    if let Some(db) = crate::get_session_db() {
        let runs = match session_id {
            Some(sid) => db.list_active_subagent_runs(sid)?,
            None => db.list_all_active_subagent_runs()?,
        };
        for run in runs {
            results.push(cancel_subagent(&run.run_id)?);
        }
    }

    let process_ids = {
        let registry = crate::process_registry::get_registry().lock().await;
        registry.list_running_ids_for_parent_session(session_id)
    };
    for process_id in process_ids {
        results.push(cancel_process(&process_id).await?);
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
    match crate::async_jobs::cancel_job(id)? {
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

    let signalled = crate::get_subagent_cancels()
        .map(|registry| registry.cancel(id))
        .unwrap_or(false);
    if !signalled {
        let _ = db.update_subagent_status(
            id,
            crate::subagent::SubagentStatus::Killed,
            None,
            Some("Killed by runtime cancel"),
            None,
            None,
        );
    }
    Ok(CancelRuntimeTaskResult::new(
        RuntimeTaskKind::Subagent,
        id,
        true,
        "killed",
        "Sub-agent cancellation requested",
    ))
}

async fn cancel_process(id: &str) -> anyhow::Result<CancelRuntimeTaskResult> {
    use crate::process_registry::{get_registry, ProcessStatus};

    let mut registry = get_registry().lock().await;
    let Some(session) = registry.get_session(id).cloned() else {
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
        crate::platform::terminate_process_tree(pid);
    }
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

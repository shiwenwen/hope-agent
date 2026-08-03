use serde::{Deserialize, Serialize};

const RUNTIME_SNAPSHOT_SOURCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const DEFERRED_SNAPSHOT_CANCEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

fn extend_snapshot_source(
    tasks: &mut Vec<(RuntimeTaskKind, String)>,
    source: &'static str,
    discovered: anyhow::Result<Vec<(RuntimeTaskKind, String)>>,
) {
    match discovered {
        Ok(discovered) => tasks.extend(discovered),
        Err(error) => {
            let message = crate::logging::redact_sensitive(&error.to_string());
            crate::app_warn!(
                "chat",
                "runtime_task_snapshot",
                "Runtime task source snapshot failed; continuing other sources: source={} error={}",
                source,
                message
            );
        }
    }
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

/// Capture runtime work associated with a session without performing immediate
/// cancellation side effects. A source that misses its bound is transferred to
/// a gated continuation, which cancels its exact late identities before
/// replacement foreground work can register.
pub async fn snapshot_runtime_tasks_for_session(
    session_id: Option<&str>,
) -> anyhow::Result<RuntimeTaskSnapshot> {
    let owned_session_id = session_id.map(str::to_string);
    let async_job_session_id = owned_session_id.clone();
    let async_jobs = crate::blocking::run_blocking(move || {
        if let Some(db) = crate::async_jobs::get_async_jobs_db() {
            db.list_running().map(|jobs| {
                jobs.into_iter()
                    .filter(|job| {
                        async_job_session_id
                            .as_deref()
                            .map(|sid| job.session_id.as_deref() == Some(sid))
                            .unwrap_or(true)
                    })
                    .map(|job| (RuntimeTaskKind::AsyncJob, job.job_id))
                    .collect()
            })
        } else {
            Ok(Vec::new())
        }
    });

    let subagent_session_id = owned_session_id.clone();
    let subagents = crate::blocking::run_blocking(move || {
        if let Some(db) = crate::get_session_db() {
            match subagent_session_id.as_deref() {
                Some(sid) => db.list_nonterminal_subagent_runs(sid),
                None => db.list_all_nonterminal_subagent_runs(),
            }
            .map(|runs| {
                runs.into_iter()
                    .map(|run| (RuntimeTaskKind::Subagent, run.run_id))
                    .collect()
            })
        } else {
            Ok(Vec::new())
        }
    });

    let process_session_id = owned_session_id.clone();
    let processes = async move {
        let registry = crate::process_registry::get_registry().lock().await;
        anyhow::Ok(
            registry
                .list_running_ids_for_parent_session(process_session_id.as_deref())
                .into_iter()
                .map(|id| (RuntimeTaskKind::Process, id))
                .collect(),
        )
    };

    // Each source gets its own bound and all sources are polled together. A
    // wedged SQLite read or process-registry lock must not prevent the other
    // exact identities from being captured and cancelled.
    let (async_jobs, subagents, processes) = tokio::join!(
        timeout_snapshot_source(
            "async_jobs",
            owned_session_id.clone(),
            tokio::spawn(async_jobs)
        ),
        timeout_snapshot_source(
            "subagents",
            owned_session_id.clone(),
            tokio::spawn(subagents)
        ),
        timeout_snapshot_source("processes", owned_session_id, tokio::spawn(processes)),
    );
    let mut tasks = Vec::new();
    for discovered in [async_jobs, subagents, processes] {
        tasks.extend(discovered);
    }

    Ok(RuntimeTaskSnapshot { tasks })
}

enum DeferredSnapshotGate {
    Session(crate::chat_engine::active_turn::StopCleanupGuard),
    Global(crate::chat_engine::active_turn::GlobalStopCleanupGuard),
}

impl DeferredSnapshotGate {
    fn begin(session_id: Option<&str>) -> Self {
        match session_id {
            Some(session_id) => Self::Session(crate::chat_engine::active_turn::begin_stop_cleanup(
                session_id,
            )),
            None => Self::Global(crate::chat_engine::active_turn::begin_global_stop_cleanup()),
        }
    }
}

impl Drop for DeferredSnapshotGate {
    fn drop(&mut self) {
        match self {
            Self::Session(guard) => guard.release(),
            Self::Global(guard) => guard.release(),
        }
    }
}

async fn timeout_snapshot_source(
    source: &'static str,
    session_id: Option<String>,
    handle: tokio::task::JoinHandle<anyhow::Result<Vec<(RuntimeTaskKind, String)>>>,
) -> Vec<(RuntimeTaskKind, String)> {
    timeout_snapshot_source_with(source, session_id, handle, RUNTIME_SNAPSHOT_SOURCE_TIMEOUT).await
}

async fn timeout_snapshot_source_with(
    source: &'static str,
    session_id: Option<String>,
    mut handle: tokio::task::JoinHandle<anyhow::Result<Vec<(RuntimeTaskKind, String)>>>,
    timeout: std::time::Duration,
) -> Vec<(RuntimeTaskKind, String)> {
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(joined) => snapshot_source_join_result(source, joined),
        Err(_) => {
            // Transfer an additional generation gate to the continuation
            // before returning. The blocked read may finish later, but a
            // replacement foreground turn cannot register resources that the
            // eventual snapshot might mistake for the stopped generation.
            let gate = DeferredSnapshotGate::begin(session_id.as_deref());
            crate::app_warn!(
                "chat",
                "runtime_task_snapshot",
                "Runtime task source snapshot timed out; continuing it behind the Stop gate: source={} timeout_ms={}",
                source,
                timeout.as_millis()
            );
            tokio::spawn(async move {
                let discovered = snapshot_source_join_result(source, handle.await);
                if !discovered.is_empty() {
                    let snapshot = RuntimeTaskSnapshot { tasks: discovered };
                    if tokio::time::timeout(
                        DEFERRED_SNAPSHOT_CANCEL_TIMEOUT,
                        cancel_runtime_task_snapshot(snapshot),
                    )
                    .await
                    .is_err()
                    {
                        crate::app_warn!(
                            "chat",
                            "runtime_task_cancel",
                            "Deferred runtime cancellation timed out after {}ms: source={}",
                            DEFERRED_SNAPSHOT_CANCEL_TIMEOUT.as_millis(),
                            source
                        );
                    }
                }
                drop(gate);
            });
            Vec::new()
        }
    }
}

fn snapshot_source_join_result(
    source: &'static str,
    joined: Result<anyhow::Result<Vec<(RuntimeTaskKind, String)>>, tokio::task::JoinError>,
) -> Vec<(RuntimeTaskKind, String)> {
    match joined {
        Ok(Ok(discovered)) => discovered,
        Ok(Err(error)) => {
            let mut tasks = Vec::new();
            extend_snapshot_source(&mut tasks, source, Err(error));
            tasks
        }
        Err(error) => {
            let mut tasks = Vec::new();
            extend_snapshot_source(
                &mut tasks,
                source,
                Err(anyhow::anyhow!("snapshot task failed: {error}")),
            );
            tasks
        }
    }
}

/// Cancel only the identities in a previously captured snapshot.
pub async fn cancel_runtime_task_snapshot(
    snapshot: RuntimeTaskSnapshot,
) -> anyhow::Result<Vec<CancelRuntimeTaskResult>> {
    Ok(
        cancel_runtime_task_snapshot_with(snapshot, |kind, id| async move {
            cancel_runtime_task(kind, &id).await
        })
        .await,
    )
}

async fn cancel_runtime_task_snapshot_with<F, Fut>(
    snapshot: RuntimeTaskSnapshot,
    cancel_task: F,
) -> Vec<CancelRuntimeTaskResult>
where
    F: Fn(RuntimeTaskKind, String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<CancelRuntimeTaskResult>> + Send + 'static,
{
    let cancel_task = std::sync::Arc::new(cancel_task);
    let mut cancellations = Vec::with_capacity(snapshot.tasks.len());
    for (kind, id) in snapshot.tasks {
        let task_id = id.clone();
        let cancel_task = cancel_task.clone();
        let handle = tokio::spawn(async move { cancel_task(kind, task_id).await });
        cancellations.push(async move { (kind, id, handle.await) });
    }

    // All exact cancellation tasks have been spawned before the first await.
    // If the caller's overall Stop timeout expires, dropping these JoinHandles
    // detaches (rather than aborts) the tasks, so a slow first cancellation
    // cannot prevent later snapshot members from being attempted.
    futures_util::future::join_all(cancellations)
        .await
        .into_iter()
        .map(|(kind, id, joined)| match joined {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let message = crate::logging::redact_sensitive(&error.to_string());
                crate::app_warn!(
                    "chat",
                    "runtime_task_cancel",
                    "Runtime task cancellation failed; continuing snapshot: kind={} id={} error={}",
                    kind.as_str(),
                    id,
                    message
                );
                CancelRuntimeTaskResult::new(
                    kind,
                    &id,
                    false,
                    "error",
                    format!("Runtime task cancellation failed: {message}"),
                )
            }
            Err(error) => {
                let message = crate::logging::redact_sensitive(&error.to_string());
                crate::app_warn!(
                    "chat",
                    "runtime_task_cancel",
                    "Runtime task cancellation task failed: kind={} id={} error={}",
                    kind.as_str(),
                    id,
                    message
                );
                CancelRuntimeTaskResult::new(
                    kind,
                    &id,
                    false,
                    "error",
                    format!("Runtime task cancellation task failed: {message}"),
                )
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn snapshot_collection_continues_after_source_failure() {
        let mut tasks = Vec::new();
        extend_snapshot_source(
            &mut tasks,
            "async_jobs",
            Err(anyhow::anyhow!("transient database read failure")),
        );
        extend_snapshot_source(
            &mut tasks,
            "subagents",
            Ok(vec![(RuntimeTaskKind::Subagent, "continues".to_string())]),
        );

        assert_eq!(
            tasks,
            vec![(RuntimeTaskKind::Subagent, "continues".to_string())]
        );
    }

    #[tokio::test]
    async fn snapshot_cancellation_continues_after_individual_failure() {
        let snapshot = RuntimeTaskSnapshot {
            tasks: vec![
                (RuntimeTaskKind::AsyncJob, "fails".to_string()),
                (RuntimeTaskKind::Process, "continues".to_string()),
            ],
        };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_cancel = calls.clone();

        let results = cancel_runtime_task_snapshot_with(snapshot, move |kind, id| {
            calls_for_cancel.lock().unwrap().push(id.clone());
            async move {
                if id == "fails" {
                    anyhow::bail!("transient cancellation failure");
                }
                Ok(CancelRuntimeTaskResult::new(
                    kind,
                    &id,
                    true,
                    "killed",
                    "cancelled",
                ))
            }
        })
        .await;

        let mut calls = calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, ["continues", "fails"]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, "error");
        assert!(!results[0].accepted);
        assert_eq!(results[1].status, "killed");
        assert!(results[1].accepted);
    }

    #[tokio::test]
    async fn slow_cancellation_does_not_block_later_snapshot_members() {
        let snapshot = RuntimeTaskSnapshot {
            tasks: vec![
                (RuntimeTaskKind::AsyncJob, "slow".to_string()),
                (RuntimeTaskKind::Process, "later".to_string()),
            ],
        };
        let slow_release = Arc::new(tokio::sync::Notify::new());
        let later_started = Arc::new(tokio::sync::Notify::new());
        let release_for_cancel = slow_release.clone();
        let started_for_cancel = later_started.clone();

        let cancellation = tokio::spawn(cancel_runtime_task_snapshot_with(
            snapshot,
            move |kind, id| {
                let slow_release = release_for_cancel.clone();
                let later_started = started_for_cancel.clone();
                async move {
                    if id == "slow" {
                        slow_release.notified().await;
                    } else {
                        later_started.notify_one();
                    }
                    Ok(CancelRuntimeTaskResult::new(
                        kind,
                        &id,
                        true,
                        "cancelled",
                        "cancelled",
                    ))
                }
            },
        ));

        tokio::time::timeout(std::time::Duration::from_secs(1), later_started.notified())
            .await
            .expect("later snapshot member should start while the first is blocked");
        slow_release.notify_one();
        let results = cancellation.await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.accepted));
    }

    #[tokio::test]
    async fn timed_out_snapshot_source_continues_behind_gate() {
        let session_id = format!("deferred-snapshot-{}", uuid::Uuid::new_v4());
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(tokio::sync::Notify::new());
        let release_for_source = release.clone();
        let completed_for_source = completed.clone();
        let handle = tokio::spawn(async move {
            release_for_source.notified().await;
            completed_for_source.notify_one();
            anyhow::Ok(Vec::new())
        });

        let immediate = timeout_snapshot_source_with(
            "test_source",
            Some(session_id.clone()),
            handle,
            std::time::Duration::from_millis(10),
        )
        .await;
        assert!(immediate.is_empty());
        assert!(crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            "turn-before-late-snapshot".to_string(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .is_err());

        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), completed.notified())
            .await
            .expect("timed-out source should remain attached to its continuation");

        let replacement = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Ok(guard) = crate::chat_engine::active_turn::try_acquire(
                    &session_id,
                    crate::chat_engine::ChatSource::Desktop,
                    "turn-after-late-snapshot".to_string(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ) {
                    break guard;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement should acquire after the late snapshot settles");
        drop(replacement);
    }
}

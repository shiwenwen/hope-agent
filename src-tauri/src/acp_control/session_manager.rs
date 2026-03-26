//! ACP Control Plane — Session manager.
//!
//! Coordinates spawning, monitoring, and lifecycle management of
//! external ACP agent runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use std::collections::HashMap;

use super::events;
use super::registry::AcpRuntimeRegistry;
use super::types::*;

/// Maximum result text stored in the database per run.
const MAX_RESULT_CHARS: usize = 50_000;

/// ACP session manager — the control plane core.
pub struct AcpSessionManager {
    registry: Arc<AcpRuntimeRegistry>,
    /// Active runs keyed by run_id.
    runs: Arc<RwLock<HashMap<String, AcpRun>>>,
    /// Active sessions keyed by run_id → external session handle.
    sessions: Arc<RwLock<HashMap<String, AcpExternalSession>>>,
    /// Cancel flags keyed by run_id.
    cancels: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
}

impl AcpSessionManager {
    pub fn new(registry: Arc<AcpRuntimeRegistry>) -> Self {
        Self {
            registry,
            runs: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cancels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn an external ACP agent to execute a task.
    /// Returns the run_id immediately; execution happens in the background.
    pub async fn spawn_run(
        &self,
        backend_id: &str,
        task: &str,
        params: AcpCreateParams,
        parent_session_id: &str,
        label: Option<String>,
    ) -> anyhow::Result<String> {
        let runtime = self
            .registry
            .get(backend_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("ACP backend '{}' not found", backend_id))?;

        let run_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let cancel = Arc::new(AtomicBool::new(false));

        let run = AcpRun {
            run_id: run_id.clone(),
            parent_session_id: parent_session_id.to_string(),
            backend_id: backend_id.to_string(),
            external_session_id: None,
            task: task.to_string(),
            status: AcpRunStatus::Starting,
            result: None,
            error: None,
            model_used: params.model.clone(),
            started_at: now,
            finished_at: None,
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            label: label.clone(),
            pid: None,
        };

        self.runs.write().await.insert(run_id.clone(), run);
        self.cancels.write().await.insert(run_id.clone(), cancel.clone());

        // Emit spawned event
        events::emit_acp_event(
            &run_id,
            parent_session_id,
            backend_id,
            label.as_deref(),
            "spawned",
            serde_json::json!({ "task": task }),
        );

        // Persist to DB
        if let Some(db) = crate::get_session_db() {
            let _ = db.insert_acp_run(&run_id, parent_session_id, backend_id, task, label.as_deref());
        }

        // Background execution
        let runs = Arc::clone(&self.runs);
        let sessions = Arc::clone(&self.sessions);
        let cancels = Arc::clone(&self.cancels);
        let task_owned = task.to_string();
        let parent_sid = parent_session_id.to_string();
        let backend_owned = backend_id.to_string();
        let label_owned = label.clone();
        let run_id_clone = run_id.clone();

        tokio::spawn(async move {
            let start = std::time::Instant::now();

            // Create session
            let session = match runtime.create_session(params).await {
                Ok(s) => s,
                Err(e) => {
                    let error_msg = format!("Failed to create ACP session: {}", e);
                    Self::finalize_run(
                        &runs, &cancels, &run_id_clone, &parent_sid, &backend_owned,
                        label_owned.as_deref(), AcpRunStatus::Error, None,
                        Some(&error_msg), start, None, None,
                    ).await;
                    return;
                }
            };

            // Update run with PID and external session ID
            {
                let mut w = runs.write().await;
                if let Some(run) = w.get_mut(&run_id_clone) {
                    run.pid = session.pid;
                    run.external_session_id = session.external_session_id.clone();
                    run.status = AcpRunStatus::Running;
                }
            }
            sessions.write().await.insert(run_id_clone.clone(), session.clone());

            if let Some(db) = crate::get_session_db() {
                let _ = db.update_acp_run_status(&run_id_clone, "running", session.pid, session.external_session_id.as_deref());
            }

            // Run the turn
            let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AcpStreamEvent>();

            // Forward events to Tauri in a separate task
            let run_id_for_events = run_id_clone.clone();
            let parent_for_events = parent_sid.clone();
            let backend_for_events = backend_owned.clone();
            let label_for_events = label_owned.clone();
            let events_task = tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    events::emit_stream_event(
                        &run_id_for_events,
                        &parent_for_events,
                        &backend_for_events,
                        label_for_events.as_deref(),
                        &event,
                    );
                }
            });

            let cancel_flag = {
                cancels.read().await.get(&run_id_clone).cloned()
                    .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
            };

            let turn_result = runtime
                .run_turn(&session, &task_owned, event_tx, cancel_flag)
                .await;

            // Wait for events to flush
            let _ = events_task.await;

            // Close session
            let _ = runtime.close_session(&session).await;
            sessions.write().await.remove(&run_id_clone);

            match turn_result {
                Ok(result) => {
                    let truncated = if result.response_text.len() > MAX_RESULT_CHARS {
                        crate::truncate_utf8(&result.response_text, MAX_RESULT_CHARS).to_string()
                    } else {
                        result.response_text.clone()
                    };

                    Self::finalize_run(
                        &runs, &cancels, &run_id_clone, &parent_sid, &backend_owned,
                        label_owned.as_deref(), AcpRunStatus::Completed,
                        Some(&truncated), None, start,
                        result.input_tokens, result.output_tokens,
                    ).await;
                }
                Err(e) => {
                    let error_msg = format!("{}", e);
                    let status = if error_msg.contains("timed out") {
                        AcpRunStatus::Timeout
                    } else {
                        AcpRunStatus::Error
                    };

                    Self::finalize_run(
                        &runs, &cancels, &run_id_clone, &parent_sid, &backend_owned,
                        label_owned.as_deref(), status, None,
                        Some(&error_msg), start, None, None,
                    ).await;
                }
            }
        });

        Ok(run_id)
    }

    /// Check the status of a run.
    pub async fn check_run(&self, run_id: &str) -> Option<AcpRun> {
        self.runs.read().await.get(run_id).cloned()
    }

    /// List all runs, optionally filtered by parent session.
    pub async fn list_runs(&self, parent_session_id: Option<&str>) -> Vec<AcpRun> {
        let runs = self.runs.read().await;
        runs.values()
            .filter(|r| {
                parent_session_id
                    .map(|p| r.parent_session_id == p)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Get the full result text of a completed run.
    pub async fn get_result(&self, run_id: &str) -> anyhow::Result<String> {
        let runs = self.runs.read().await;
        let run = runs
            .get(run_id)
            .ok_or_else(|| anyhow::anyhow!("Run not found: {}", run_id))?;

        if !run.status.is_terminal() {
            return Err(anyhow::anyhow!("Run is still in progress (status: {})", run.status));
        }

        if let Some(error) = &run.error {
            return Err(anyhow::anyhow!("Run failed: {}", error));
        }

        Ok(run.result.clone().unwrap_or_default())
    }

    /// Kill a running ACP run.
    pub async fn kill_run(&self, run_id: &str) -> anyhow::Result<()> {
        // Set cancel flag
        if let Some(cancel) = self.cancels.read().await.get(run_id) {
            cancel.store(true, Ordering::Relaxed);
        }

        // Also try to close the session directly
        if let Some(session) = self.sessions.read().await.get(run_id).cloned() {
            if let Some(runtime) = self.registry.get(&session.backend_id).await {
                let _ = runtime.cancel_turn(&session).await;
                let _ = runtime.close_session(&session).await;
            }
        }

        // Update status
        {
            let mut runs = self.runs.write().await;
            if let Some(run) = runs.get_mut(run_id) {
                if !run.status.is_terminal() {
                    run.status = AcpRunStatus::Killed;
                    run.finished_at = Some(chrono::Utc::now().to_rfc3339());
                }
            }
        }

        if let Some(db) = crate::get_session_db() {
            let _ = db.finish_acp_run(run_id, "killed", None, None, None, None);
        }

        Ok(())
    }

    /// Kill all active runs for a parent session.
    pub async fn kill_all(&self, parent_session_id: &str) -> anyhow::Result<u32> {
        let run_ids: Vec<String> = {
            self.runs.read().await.values()
                .filter(|r| r.parent_session_id == parent_session_id && !r.status.is_terminal())
                .map(|r| r.run_id.clone())
                .collect()
        };

        let count = run_ids.len() as u32;
        for rid in run_ids {
            let _ = self.kill_run(&rid).await;
        }
        Ok(count)
    }

    /// Send a follow-up message to a running ACP session (steer).
    pub async fn steer_run(&self, run_id: &str, message: &str) -> anyhow::Result<()> {
        let session = self
            .sessions
            .read()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No active session for run {}", run_id))?;

        let runtime = self
            .registry
            .get(&session.backend_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Backend not found: {}", session.backend_id))?;

        let (tx, _rx) = mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));

        runtime.run_turn(&session, message, tx, cancel).await?;
        Ok(())
    }

    /// Count active (non-terminal) runs.
    pub async fn active_count(&self) -> usize {
        self.runs.read().await.values()
            .filter(|r| !r.status.is_terminal())
            .count()
    }

    /// Finalize a run (update in-memory state + DB + emit event).
    async fn finalize_run(
        runs: &Arc<RwLock<HashMap<String, AcpRun>>>,
        cancels: &Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
        run_id: &str,
        parent_session_id: &str,
        backend_id: &str,
        label: Option<&str>,
        status: AcpRunStatus,
        result: Option<&str>,
        error: Option<&str>,
        start: std::time::Instant,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let duration_ms = start.elapsed().as_millis() as u64;

        {
            let mut w = runs.write().await;
            if let Some(run) = w.get_mut(run_id) {
                run.status = status.clone();
                run.result = result.map(|s| s.to_string());
                run.error = error.map(|s| s.to_string());
                run.finished_at = Some(now);
                run.duration_ms = Some(duration_ms);
                run.input_tokens = input_tokens;
                run.output_tokens = output_tokens;
            }
        }

        // Clean up cancel flag
        cancels.write().await.remove(run_id);

        // Persist
        if let Some(db) = crate::get_session_db() {
            let _ = db.finish_acp_run(
                run_id,
                status.as_str(),
                result,
                error,
                input_tokens,
                output_tokens,
            );
        }

        // Emit completion event
        let event_type = match &status {
            AcpRunStatus::Completed => "completed",
            AcpRunStatus::Error => "error",
            AcpRunStatus::Timeout => "timeout",
            AcpRunStatus::Killed => "killed",
            _ => "completed",
        };
        events::emit_acp_event(
            run_id,
            parent_session_id,
            backend_id,
            label,
            event_type,
            serde_json::json!({
                "status": status.as_str(),
                "durationMs": duration_ms,
                "inputTokens": input_tokens,
                "outputTokens": output_tokens,
                "error": error,
            }),
        );
    }
}

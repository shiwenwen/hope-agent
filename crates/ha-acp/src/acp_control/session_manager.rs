//! ACP Control Plane — Session manager.
//!
//! Coordinates spawning, monitoring, and lifecycle management of
//! external ACP agent runs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

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

    /// Get a reference to the underlying runtime registry.
    pub fn runtime_registry(&self) -> &Arc<AcpRuntimeRegistry> {
        &self.registry
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
        self.cancels
            .write()
            .await
            .insert(run_id.clone(), cancel.clone());

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
        if let Some(db) = ha_core::get_session_db() {
            let _ = db.insert_acp_run(
                &run_id,
                parent_session_id,
                backend_id,
                task,
                label.as_deref(),
            );
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
        let cancel_for_task = cancel.clone();

        tokio::spawn(async move {
            // Create session
            let session = match runtime.create_session(params).await {
                Ok(s) => s,
                Err(e) => {
                    let error_msg = format!("Failed to create ACP session: {}", e);
                    Self::finalize_run(
                        &runs,
                        &cancels,
                        &run_id_clone,
                        &parent_sid,
                        &backend_owned,
                        label_owned.as_deref(),
                        AcpRunStatus::Error,
                        None,
                        Some(&error_msg),
                        None,
                        None,
                    )
                    .await;
                    return;
                }
            };

            // Register the created session before promoting the run. If a
            // kill raced create_session(), it can now close this session; if
            // the terminal claim already won, the check below closes it and
            // returns without ever issuing a prompt.
            sessions
                .write()
                .await
                .insert(run_id_clone.clone(), session.clone());

            let should_run = {
                let mut w = runs.write().await;
                match w.get_mut(&run_id_clone) {
                    Some(run) if !run.status.is_terminal() => {
                        run.pid = session.pid;
                        run.external_session_id = session.external_session_id.clone();
                        run.status = AcpRunStatus::Running;
                        true
                    }
                    _ => false,
                }
            };

            if !should_run {
                let _ = runtime.close_session(&session).await;
                sessions.write().await.remove(&run_id_clone);
                return;
            }

            if let Some(db) = ha_core::get_session_db() {
                let _ = db.update_acp_run_status(
                    &run_id_clone,
                    "running",
                    session.pid,
                    session.external_session_id.as_deref(),
                );
            }

            // Run the turn
            let (event_tx, mut event_rx) = mpsc::channel::<AcpStreamEvent>(256);

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

            // Keep the original flag even after a terminal claim removes the
            // bookkeeping entry. A kill racing startup must remain visible to
            // the background task instead of silently creating a fresh false
            // flag.
            let cancel_flag = cancel_for_task;

            let timeout_secs = session.timeout_secs;
            let turn_fut = runtime.run_turn(&session, &task_owned, event_tx, cancel_flag.clone());
            let turn_result = if timeout_secs == 0 {
                turn_fut.await
            } else {
                match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), turn_fut)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        cancel_flag.store(true, Ordering::SeqCst);
                        Err(anyhow::anyhow!("Turn timed out after {}s", timeout_secs))
                    }
                }
            };

            // Wait for events to flush
            let _ = events_task.await;

            // Close session
            let _ = runtime.close_session(&session).await;
            sessions.write().await.remove(&run_id_clone);

            match turn_result {
                Ok(result) => {
                    let truncated = if result.response_text.len() > MAX_RESULT_CHARS {
                        ha_core::truncate_utf8(&result.response_text, MAX_RESULT_CHARS).to_string()
                    } else {
                        result.response_text.clone()
                    };

                    // A successful JSON-RPC transport is not a successful
                    // turn: only the backend's stop reason is the failure
                    // truth. Project it once here so in-memory state, DB,
                    // completion event, and get_result() agree.
                    let (status, error) = Self::project_stop_reason(&result.stop_reason);
                    let result_text = if status == AcpRunStatus::Killed {
                        // Mirror kill_run: a cancelled turn keeps no result.
                        None
                    } else {
                        Some(truncated.as_str())
                    };

                    Self::finalize_run(
                        &runs,
                        &cancels,
                        &run_id_clone,
                        &parent_sid,
                        &backend_owned,
                        label_owned.as_deref(),
                        status,
                        result_text,
                        error.as_deref(),
                        result.input_tokens,
                        result.output_tokens,
                    )
                    .await;
                }
                Err(e) => {
                    let error_msg = format!("{}", e);
                    let status = if error_msg.contains("timed out") {
                        AcpRunStatus::Timeout
                    } else {
                        AcpRunStatus::Error
                    };

                    Self::finalize_run(
                        &runs,
                        &cancels,
                        &run_id_clone,
                        &parent_sid,
                        &backend_owned,
                        label_owned.as_deref(),
                        status,
                        None,
                        Some(&error_msg),
                        None,
                        None,
                    )
                    .await;
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

        match run.status {
            AcpRunStatus::Starting | AcpRunStatus::Running => Err(anyhow::anyhow!(
                "Run is still in progress (status: {})",
                run.status
            )),
            AcpRunStatus::Completed => Ok(run.result.clone().unwrap_or_default()),
            AcpRunStatus::Error | AcpRunStatus::Timeout | AcpRunStatus::Killed => {
                if let Some(error) = &run.error {
                    Err(anyhow::anyhow!("Run failed: {}", error))
                } else {
                    Err(anyhow::anyhow!("Run failed with status: {}", run.status))
                }
            }
        }
    }

    /// Kill a running ACP run.
    pub async fn kill_run(&self, run_id: &str) -> anyhow::Result<()> {
        let run_meta = self.runs.read().await.get(run_id).map(|run| {
            (
                run.parent_session_id.clone(),
                run.backend_id.clone(),
                run.label.clone(),
            )
        });

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

        // Use the same terminal exit as turn completion. Whichever writer
        // wins the claim emits the one matching completion event.
        if let Some((parent_session_id, backend_id, label)) = run_meta {
            Self::finalize_run(
                &self.runs,
                &self.cancels,
                run_id,
                &parent_session_id,
                &backend_id,
                label.as_deref(),
                AcpRunStatus::Killed,
                None,
                None,
                None,
                None,
            )
            .await;
        }

        Ok(())
    }

    /// Kill all active runs for a parent session.
    pub async fn kill_all(&self, parent_session_id: &str) -> anyhow::Result<u32> {
        let run_ids: Vec<String> = {
            self.runs
                .read()
                .await
                .values()
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
    ///
    /// Not supported over the stdio control runtime, and reported honestly as
    /// such: ACP sessions process one `session/prompt` at a time. The protocol
    /// has no mid-turn message injection, a second prompt while one is in
    /// flight is rejected by single-turn backends, and the previous
    /// implementation — a second concurrent `run_turn` with a colliding
    /// request id on the same child — could surface a fabricated success
    /// while the child was being torn down. Callers get a real error instead:
    /// wait for the run to finish, read its result, then spawn a follow-up
    /// run if needed.
    pub async fn steer_run(&self, run_id: &str, _message: &str) -> anyhow::Result<()> {
        if self.sessions.read().await.get(run_id).is_none() {
            anyhow::bail!("No active session for run {}", run_id);
        }
        anyhow::bail!(
            "Steering an active ACP run is not supported: ACP sessions process one \
             prompt at a time, so run {run_id} cannot take a follow-up prompt while \
             its current turn is in flight. Wait for the run to finish, read its \
             result, then spawn a follow-up run if needed."
        );
    }

    /// Count active (non-terminal) runs.
    pub async fn active_count(&self) -> usize {
        self.runs
            .read()
            .await
            .values()
            .filter(|r| !r.status.is_terminal())
            .count()
    }

    /// Project an ACP `PromptResponse.stopReason` onto the control-plane
    /// terminal run state.
    ///
    /// The ACP transport layer succeeding does not mean the turn succeeded:
    /// only the backend's stop reason carries the failure truth. ACP v1 (and
    /// the legacy 0.2 dialect this runtime also speaks) define exactly five
    /// stop reasons; anything else is projected fail-closed, never as a
    /// successful completion.
    fn project_stop_reason(stop_reason: &str) -> (AcpRunStatus, Option<String>) {
        match stop_reason {
            // Ordinary protocol completions: the turn ran to a defined end.
            // The token/request caps are completion reasons, not failures —
            // the produced text is the turn's real (possibly truncated) answer.
            "end_turn" | "max_tokens" | "max_turn_requests" => (AcpRunStatus::Completed, None),
            // The agent explicitly declined to continue. Never Completed.
            "refusal" => (
                AcpRunStatus::Error,
                Some(
                    "ACP turn ended with stopReason \"refusal\": \
                     the backend declined to continue the task"
                        .to_string(),
                ),
            ),
            // Only the client cancels a turn; mirror kill_run's terminal state.
            "cancelled" => (AcpRunStatus::Killed, None),
            other => (
                AcpRunStatus::Error,
                Some(format!(
                    "ACP turn ended with unknown stopReason \"{other}\""
                )),
            ),
        }
    }

    /// Completion event type for a terminal run status.
    fn completion_event_type(status: &AcpRunStatus) -> &'static str {
        match status {
            AcpRunStatus::Completed => "completed",
            AcpRunStatus::Error => "error",
            AcpRunStatus::Timeout => "timeout",
            AcpRunStatus::Killed => "killed",
            _ => "completed",
        }
    }

    /// Claim the single terminal transition of a run (first writer wins).
    ///
    /// Whichever terminal writer — turn completion, `kill_run`, timeout, or
    /// spawn failure — observes a non-terminal status first takes the
    /// transition atomically; every later terminal writer is a no-op across
    /// the in-memory run, the `acp_runs` row, and downstream events alike.
    /// Returns the run duration in milliseconds when this call won the claim.
    async fn claim_terminal_run(
        runs: &Arc<RwLock<HashMap<String, AcpRun>>>,
        cancels: &Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
        run_id: &str,
        status: AcpRunStatus,
        result: Option<&str>,
        error: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Option<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let claimed = {
            let mut w = runs.write().await;
            match w.get_mut(run_id) {
                Some(run) if !run.status.is_terminal() => {
                    let duration_ms = chrono::DateTime::parse_from_rfc3339(&run.started_at)
                        .ok()
                        .map(|started| {
                            (chrono::Utc::now().signed_duration_since(started))
                                .num_milliseconds()
                                .max(0) as u64
                        });
                    run.status = status.clone();
                    run.result = result.map(str::to_string);
                    run.error = error.map(str::to_string);
                    run.finished_at = Some(now.clone());
                    run.duration_ms = duration_ms;
                    run.input_tokens = input_tokens;
                    run.output_tokens = output_tokens;
                    Some(duration_ms.unwrap_or(0))
                }
                _ => None,
            }
        };

        let duration_ms = claimed?;

        // Clean up cancel flag
        cancels.write().await.remove(run_id);

        // Persist
        if let Some(db) = ha_core::get_session_db() {
            let _ = db.finish_acp_run(
                run_id,
                status.as_str(),
                result,
                error,
                input_tokens,
                output_tokens,
            );
        }

        Some(duration_ms)
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
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        let Some(duration_ms) = Self::claim_terminal_run(
            runs,
            cancels,
            run_id,
            status.clone(),
            result,
            error,
            input_tokens,
            output_tokens,
        )
        .await
        else {
            return;
        };

        // Emit completion event
        let event_type = Self::completion_event_type(&status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::event_bus::{BroadcastEventBus, EventBus};
    use std::sync::atomic::AtomicUsize;

    /// What a scripted turn does.
    #[derive(Debug, Clone)]
    enum TurnScript {
        /// Resolve immediately with this stop reason.
        Stop(&'static str),
        /// Resolve immediately with a transport/runtime error.
        Fail(&'static str),
        /// Sleep for N seconds before resolving (drives the manager timeout).
        AfterDelay(u64, &'static str),
        /// Resolve with `cancelled` only once the cancel flag is observed.
        UntilCancelled,
    }

    /// In-memory [`AcpRuntime`]: no child process, no I/O, fully deterministic.
    struct FakeRuntime {
        script: TurnScript,
        run_turn_calls: Arc<AtomicUsize>,
        close_calls: Arc<AtomicUsize>,
        turn_done: Arc<AtomicUsize>,
        create_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    }

    impl FakeRuntime {
        fn new(script: TurnScript) -> Arc<Self> {
            Arc::new(Self {
                script,
                run_turn_calls: Arc::new(AtomicUsize::new(0)),
                close_calls: Arc::new(AtomicUsize::new(0)),
                turn_done: Arc::new(AtomicUsize::new(0)),
                create_gate: None,
            })
        }

        fn with_blocked_create(
            script: TurnScript,
        ) -> (
            Arc<Self>,
            Arc<tokio::sync::Notify>,
            Arc<tokio::sync::Notify>,
        ) {
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let runtime = Arc::new(Self {
                script,
                run_turn_calls: Arc::new(AtomicUsize::new(0)),
                close_calls: Arc::new(AtomicUsize::new(0)),
                turn_done: Arc::new(AtomicUsize::new(0)),
                create_gate: Some((entered.clone(), release.clone())),
            });
            (runtime, entered, release)
        }

        async fn manager(fake: Arc<FakeRuntime>) -> AcpSessionManager {
            let registry = Arc::new(AcpRuntimeRegistry::new());
            registry.register(fake).await;
            AcpSessionManager::new(registry)
        }
    }

    fn turn_result(stop_reason: &str, text: &str) -> AcpTurnResult {
        AcpTurnResult {
            stop_reason: stop_reason.to_string(),
            response_text: text.to_string(),
            input_tokens: Some(11),
            output_tokens: Some(7),
            tool_calls: Vec::new(),
        }
    }

    #[async_trait::async_trait]
    impl AcpRuntime for FakeRuntime {
        fn backend_id(&self) -> &str {
            "fake"
        }

        fn display_name(&self) -> &str {
            "Fake"
        }

        async fn is_available(&self) -> bool {
            true
        }

        async fn get_version(&self) -> anyhow::Result<String> {
            Ok("test".to_string())
        }

        async fn create_session(
            &self,
            params: AcpCreateParams,
        ) -> anyhow::Result<AcpExternalSession> {
            if let Some((entered, release)) = &self.create_gate {
                entered.notify_one();
                release.notified().await;
            }
            Ok(AcpExternalSession {
                session_id: uuid::Uuid::new_v4().to_string(),
                backend_id: "fake".to_string(),
                external_session_id: Some("external".to_string()),
                pid: None,
                timeout_secs: params.timeout_secs.unwrap_or(0),
                created_at: chrono::Utc::now().to_rfc3339(),
            })
        }

        async fn run_turn(
            &self,
            _session: &AcpExternalSession,
            _prompt: &str,
            _event_tx: mpsc::Sender<AcpStreamEvent>,
            cancel: Arc<AtomicBool>,
        ) -> anyhow::Result<AcpTurnResult> {
            self.run_turn_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let outcome = match &self.script {
                TurnScript::Stop(reason) => Ok(turn_result(reason, "partial answer")),
                TurnScript::Fail(message) => Err(anyhow::anyhow!("{}", message)),
                TurnScript::AfterDelay(secs, reason) => {
                    tokio::time::sleep(std::time::Duration::from_secs(*secs)).await;
                    Ok(turn_result(reason, "too late"))
                }
                TurnScript::UntilCancelled => loop {
                    if cancel.load(Ordering::Relaxed) {
                        break Ok(turn_result("cancelled", "partial answer"));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                },
            };
            self.turn_done
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            outcome
        }

        async fn cancel_turn(&self, _session: &AcpExternalSession) -> anyhow::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> AcpHealthStatus {
            AcpHealthStatus {
                available: true,
                binary_path: None,
                version: Some("test".to_string()),
                error: None,
                last_checked: chrono::Utc::now().to_rfc3339(),
            }
        }

        async fn close_session(&self, _session: &AcpExternalSession) -> anyhow::Result<()> {
            self.close_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn spawn_params(timeout_secs: Option<u64>) -> AcpCreateParams {
        AcpCreateParams {
            cwd: None,
            system_prompt: None,
            model: None,
            timeout_secs,
            resume_session_id: None,
        }
    }

    /// Wait until the run reaches any terminal state (bounded polling; the
    /// assertions below never depend on *which* poll iteration wins).
    async fn wait_terminal(manager: &AcpSessionManager, run_id: &str) -> AcpRun {
        for _ in 0..2000 {
            if let Some(run) = manager.check_run(run_id).await {
                if run.status.is_terminal() {
                    return run;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("run did not reach a terminal state");
    }

    /// Wait until the fake runtime reports `calls` close_session invocations.
    async fn wait_closes(fake: &FakeRuntime, calls: usize) {
        for _ in 0..2000 {
            if fake.close_calls.load(std::sync::atomic::Ordering::SeqCst) >= calls {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("close_session was not called {calls} times");
    }

    fn subscribe_test_event_bus() -> tokio::sync::broadcast::Receiver<ha_core::event_bus::AppEvent>
    {
        if ha_core::get_event_bus().is_none() {
            let bus: Arc<dyn EventBus> = Arc::new(BroadcastEventBus::new(256));
            ha_core::set_event_bus(bus);
        }
        ha_core::get_event_bus().expect("event bus").subscribe()
    }

    async fn recv_acp_event_for_run(
        rx: &mut tokio::sync::broadcast::Receiver<ha_core::event_bus::AppEvent>,
        run_id: &str,
        event_type: &str,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let event = rx.recv().await.expect("ACP event");
                if event.name == events::ACP_CONTROL_EVENT
                    && event
                        .payload
                        .get("runId")
                        .and_then(serde_json::Value::as_str)
                        == Some(run_id)
                    && event
                        .payload
                        .get("eventType")
                        .and_then(serde_json::Value::as_str)
                        == Some(event_type)
                {
                    return;
                }
            }
        })
        .await
        .expect("matching ACP event");
    }

    async fn assert_no_acp_event_for_run(
        rx: &mut tokio::sync::broadcast::Receiver<ha_core::event_bus::AppEvent>,
        run_id: &str,
        event_type: &str,
    ) {
        let duplicate = tokio::time::timeout(std::time::Duration::from_millis(50), async {
            loop {
                let event = rx.recv().await.expect("ACP event");
                if event.name == events::ACP_CONTROL_EVENT
                    && event
                        .payload
                        .get("runId")
                        .and_then(serde_json::Value::as_str)
                        == Some(run_id)
                    && event
                        .payload
                        .get("eventType")
                        .and_then(serde_json::Value::as_str)
                        == Some(event_type)
                {
                    return;
                }
            }
        })
        .await;
        assert!(
            duplicate.is_err(),
            "duplicate {event_type} event for run {run_id}"
        );
    }

    #[tokio::test]
    async fn end_turn_still_completes_the_run() {
        let fake = FakeRuntime::new(TurnScript::Stop("end_turn"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Completed);
        assert_eq!(run.error, None);
        assert_eq!(run.result.as_deref(), Some("partial answer"));
        assert_eq!(
            manager.get_result(&run_id).await.expect("result"),
            "partial answer"
        );
    }

    #[tokio::test]
    async fn refusal_is_never_completed() {
        let fake = FakeRuntime::new(TurnScript::Stop("refusal"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Error);
        let error = run.error.expect("refusal records an error");
        assert!(error.contains("refusal"), "error message: {error}");
        // The response body stays available as evidence, but get_result()
        // must not hand it back as a success.
        assert_eq!(run.result.as_deref(), Some("partial answer"));
        let result = manager.get_result(&run_id).await;
        assert!(result.is_err(), "get_result must fail for refusal");
        assert!(result.unwrap_err().to_string().contains("refusal"));
    }

    #[tokio::test]
    async fn unknown_stop_reason_fails_closed() {
        let fake = FakeRuntime::new(TurnScript::Stop("agent_quit_somehow"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Error);
        let error = run.error.expect("unknown reason records an error");
        assert!(
            error.contains("agent_quit_somehow"),
            "error message: {error}"
        );
        assert!(manager.get_result(&run_id).await.is_err());
    }

    #[tokio::test]
    async fn cancelled_stop_reason_maps_to_killed() {
        let fake = FakeRuntime::new(TurnScript::Stop("cancelled"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Killed);
        assert_eq!(run.result, None);
        let result = manager.get_result(&run_id).await;
        assert!(result.is_err(), "cancelled runs must not return success");
        assert!(result.unwrap_err().to_string().contains("killed"));
    }

    #[tokio::test]
    async fn transport_error_maps_to_error_status() {
        let fake = FakeRuntime::new(TurnScript::Fail("Child process closed stdout"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Error);
        assert_eq!(run.error.as_deref(), Some("Child process closed stdout"));
        assert!(manager.get_result(&run_id).await.is_err());
    }

    #[tokio::test]
    async fn turn_timeout_maps_to_timeout_status() {
        let fake = FakeRuntime::new(TurnScript::AfterDelay(30, "end_turn"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(Some(1)), "parent", None)
            .await
            .expect("spawn");

        let run = wait_terminal(&manager, &run_id).await;
        assert_eq!(run.status, AcpRunStatus::Timeout);
        let error = run.error.expect("timeout records an error");
        assert!(error.contains("timed out"), "error message: {error}");
    }

    #[tokio::test]
    async fn kill_wins_over_a_late_turn_completion() {
        let fake = FakeRuntime::new(TurnScript::UntilCancelled);
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        // Wait until the turn is actually running, then kill.
        for _ in 0..2000 {
            if matches!(
                manager.check_run(&run_id).await.map(|r| r.status),
                Some(AcpRunStatus::Running)
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        manager.kill_run(&run_id).await.expect("kill");

        // The background task unwinds (run_turn returns, session closes) —
        // its own finalize must not flip the claimed Killed state.
        wait_closes(&fake, 1).await;
        for _ in 0..2000 {
            if fake.turn_done.load(std::sync::atomic::Ordering::SeqCst) >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        tokio::task::yield_now().await;

        let run = manager.check_run(&run_id).await.expect("run");
        assert_eq!(run.status, AcpRunStatus::Killed);
        assert_eq!(
            fake.run_turn_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let result = manager.get_result(&run_id).await;
        assert!(
            result.is_err(),
            "explicitly killed runs must not return success"
        );
        assert!(result.unwrap_err().to_string().contains("killed"));
    }

    #[tokio::test]
    async fn kill_during_create_session_never_reopens_the_run() {
        let (fake, create_entered, create_release) =
            FakeRuntime::with_blocked_create(TurnScript::Stop("end_turn"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        create_entered.notified().await;
        let mut events = subscribe_test_event_bus();
        manager.kill_run(&run_id).await.expect("terminate run");
        recv_acp_event_for_run(&mut events, &run_id, "killed").await;
        assert_eq!(
            manager.check_run(&run_id).await.expect("run").status,
            AcpRunStatus::Killed
        );

        create_release.notify_one();
        wait_closes(&fake, 1).await;
        tokio::task::yield_now().await;
        assert_no_acp_event_for_run(&mut events, &run_id, "killed").await;

        let run = manager.check_run(&run_id).await.expect("run");
        assert_eq!(run.status, AcpRunStatus::Killed);
        assert_eq!(run.external_session_id, None);
        assert_eq!(
            fake.run_turn_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a run killed during create_session must never issue a prompt"
        );
        assert!(manager.sessions.read().await.get(&run_id).is_none());
    }

    #[tokio::test]
    async fn steer_on_an_active_run_is_honestly_unsupported() {
        let fake = FakeRuntime::new(TurnScript::UntilCancelled);
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");

        // Wait until the original turn is actually running.
        for _ in 0..2000 {
            if matches!(
                manager.check_run(&run_id).await.map(|r| r.status),
                Some(AcpRunStatus::Running)
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        let result = manager.steer_run(&run_id, "follow-up").await;
        let error = result.expect_err("steer must not fake success");
        assert!(
            error.to_string().contains("not supported"),
            "error: {error}"
        );

        // Exactly one prompt was ever issued: no second session/prompt was
        // written against the child while the original turn was in flight.
        assert_eq!(
            fake.run_turn_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        // The original run is untouched by the rejected steer.
        let run = manager.check_run(&run_id).await.expect("run");
        assert_eq!(run.status, AcpRunStatus::Running);
        manager.kill_run(&run_id).await.expect("kill to clean up");
    }

    #[tokio::test]
    async fn steer_on_a_finished_run_reports_no_active_session() {
        let fake = FakeRuntime::new(TurnScript::Stop("end_turn"));
        let manager = FakeRuntime::manager(fake.clone()).await;
        let run_id = manager
            .spawn_run("fake", "task", spawn_params(None), "parent", None)
            .await
            .expect("spawn");
        wait_terminal(&manager, &run_id).await;

        let result = manager.steer_run(&run_id, "follow-up").await;
        let error = result.expect_err("finished runs have no session to steer");
        assert!(
            error.to_string().contains("No active session"),
            "error: {error}"
        );
        assert_eq!(
            fake.run_turn_calls
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[test]
    fn project_stop_reason_follows_acp_v1_semantics() {
        // Completion reasons stay successful.
        for reason in ["end_turn", "max_tokens", "max_turn_requests"] {
            let (status, error) = AcpSessionManager::project_stop_reason(reason);
            assert_eq!(status, AcpRunStatus::Completed, "reason: {reason}");
            assert_eq!(error, None, "reason: {reason}");
        }
        // Explicit refusal must never project to Completed.
        let (status, error) = AcpSessionManager::project_stop_reason("refusal");
        assert_eq!(status, AcpRunStatus::Error);
        assert!(error.unwrap().contains("refusal"));
        // Client cancellation mirrors kill semantics.
        let (status, error) = AcpSessionManager::project_stop_reason("cancelled");
        assert_eq!(status, AcpRunStatus::Killed);
        assert_eq!(error, None);
        // Unknown values are fail-closed, never Completed.
        let (status, error) = AcpSessionManager::project_stop_reason("max_snacks");
        assert_eq!(status, AcpRunStatus::Error);
        assert!(error.unwrap().contains("max_snacks"));
    }

    #[test]
    fn completion_event_type_covers_terminal_statuses() {
        assert_eq!(
            AcpSessionManager::completion_event_type(&AcpRunStatus::Completed),
            "completed"
        );
        assert_eq!(
            AcpSessionManager::completion_event_type(&AcpRunStatus::Error),
            "error"
        );
        assert_eq!(
            AcpSessionManager::completion_event_type(&AcpRunStatus::Timeout),
            "timeout"
        );
        assert_eq!(
            AcpSessionManager::completion_event_type(&AcpRunStatus::Killed),
            "killed"
        );
    }
}

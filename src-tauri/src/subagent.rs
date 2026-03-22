use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::session::SessionDB;

// ── Constants ────────────────────────────────────────────────────

/// Maximum nesting depth for sub-agents (desktop app — keep shallow)
pub const MAX_DEPTH: u32 = 3;

/// Default timeout for sub-agent execution (seconds)
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Max result characters stored in DB
const MAX_RESULT_CHARS: usize = 10_000;

/// Max concurrent sub-agents per parent session
pub const MAX_CONCURRENT_PER_SESSION: usize = 5;

// ── Data Structures ─────────────────────────────────────────────

/// Sub-agent run status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Spawning,
    Running,
    Completed,
    Error,
    Timeout,
    Killed,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Spawning => "spawning",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Timeout => "timeout",
            Self::Killed => "killed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "spawning" => Self::Spawning,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "error" => Self::Error,
            "timeout" => Self::Timeout,
            "killed" => Self::Killed,
            _ => Self::Error,
        }
    }

    /// Whether this status represents a terminal (finished) state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Error | Self::Timeout | Self::Killed)
    }
}

/// A sub-agent run record persisted in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub run_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task: String,
    pub status: SubagentStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub depth: u32,
    pub model_used: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
}

/// Parameters for spawning a sub-agent.
#[derive(Debug, Clone)]
pub struct SpawnParams {
    pub task: String,
    pub agent_id: String,
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub depth: u32,
    pub timeout_secs: Option<u64>,
    pub model_override: Option<String>,
}

/// Event payload emitted to the frontend via Tauri events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub event_type: String,
    pub run_id: String,
    pub parent_session_id: String,
    pub child_agent_id: String,
    pub child_session_id: String,
    pub task_preview: String,
    pub status: SubagentStatus,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    /// Full result text — included only in terminal events for push delivery.
    /// Frontend uses this to auto-inject the result into the parent agent's conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_full: Option<String>,
}

// ── Cancel Registry ─────────────────────────────────────────────

/// In-memory registry for active sub-agent cancel flags.
/// Uses AtomicBool (same pattern as chat_cancel in the codebase).
pub struct SubagentCancelRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl SubagentCancelRegistry {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
        }
    }

    /// Register a cancel flag for a run, returning the Arc<AtomicBool>.
    pub fn register(&self, run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.flags.lock() {
            map.insert(run_id.to_string(), flag.clone());
        }
        flag
    }

    /// Signal cancellation for a specific run.
    pub fn cancel(&self, run_id: &str) -> bool {
        if let Ok(map) = self.flags.lock() {
            if let Some(flag) = map.get(run_id) {
                flag.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    /// Cancel all active runs for a given parent session.
    pub fn cancel_all_for_session(&self, parent_session_id: &str, db: &SessionDB) -> u32 {
        let run_ids: Vec<String> = db.list_active_subagent_runs(parent_session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.run_id)
            .collect();

        let mut count = 0u32;
        if let Ok(map) = self.flags.lock() {
            for rid in &run_ids {
                if let Some(flag) = map.get(rid) {
                    flag.store(true, Ordering::SeqCst);
                    count += 1;
                }
            }
        }
        count
    }

    /// Remove a completed/terminated run from the registry.
    pub fn remove(&self, run_id: &str) {
        if let Ok(mut map) = self.flags.lock() {
            map.remove(run_id);
        }
    }
}

// ── Spawn Logic ─────────────────────────────────────────────────

/// Spawn a sub-agent asynchronously. Returns the run_id immediately.
pub async fn spawn_subagent(
    params: SpawnParams,
    session_db: Arc<SessionDB>,
    cancel_registry: Arc<SubagentCancelRegistry>,
) -> Result<String> {
    // 1. Validate depth
    if params.depth > MAX_DEPTH {
        return Err(anyhow::anyhow!(
            "Sub-agent depth limit reached ({}/{}). Cannot spawn further sub-agents.",
            params.depth, MAX_DEPTH
        ));
    }

    // 2. Check concurrent limit
    let active_count = session_db.count_active_subagent_runs(&params.parent_session_id)?;
    if active_count >= MAX_CONCURRENT_PER_SESSION {
        return Err(anyhow::anyhow!(
            "Max concurrent sub-agents reached ({}/{}). Wait for some to complete or kill them.",
            active_count, MAX_CONCURRENT_PER_SESSION
        ));
    }

    // 3. Validate agent exists
    let _agent_def = crate::agent_loader::load_agent(&params.agent_id)
        .map_err(|e| anyhow::anyhow!("Agent '{}' not found: {}", params.agent_id, e))?;

    // 4. Generate run_id and create isolated session (linked to parent)
    let run_id = uuid::Uuid::new_v4().to_string();
    let child_session = session_db.create_session_with_parent(
        &params.agent_id, Some(&params.parent_session_id),
    )?;
    let child_session_id = child_session.id.clone();

    // Set a descriptive title for the sub-agent session
    let task_preview = truncate_str(&params.task, 50);
    let _ = session_db.update_session_title(&child_session_id, &task_preview);

    // 5. Insert run record
    let now = chrono::Utc::now().to_rfc3339();
    let run = SubagentRun {
        run_id: run_id.clone(),
        parent_session_id: params.parent_session_id.clone(),
        parent_agent_id: params.parent_agent_id.clone(),
        child_agent_id: params.agent_id.clone(),
        child_session_id: child_session_id.clone(),
        task: params.task.clone(),
        status: SubagentStatus::Spawning,
        result: None,
        error: None,
        depth: params.depth,
        model_used: None,
        started_at: now,
        finished_at: None,
        duration_ms: None,
    };
    session_db.insert_subagent_run(&run)?;

    // 6. Register cancel flag
    let cancel_flag = cancel_registry.register(&run_id);

    // 7. Emit spawned event
    emit_subagent_event(&SubagentEvent {
        event_type: "spawned".into(),
        run_id: run_id.clone(),
        parent_session_id: params.parent_session_id.clone(),
        child_agent_id: params.agent_id.clone(),
        child_session_id: child_session_id.clone(),
        task_preview: task_preview.clone(),
        status: SubagentStatus::Spawning,
        result_preview: None,
        error: None,
        duration_ms: None,
        result_full: None,
    });

    // 8. Spawn async task
    let run_id_clone = run_id.clone();
    let db = session_db.clone();
    let registry = cancel_registry.clone();
    let agent_id = params.agent_id.clone();
    let task = params.task.clone();
    let depth = params.depth;
    let timeout_secs = params.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let model_override = params.model_override.clone();
    let parent_session_id = params.parent_session_id.clone();
    let child_session_id_clone = child_session_id.clone();

    tokio::spawn(async move {
        let start = std::time::Instant::now();

        // Update status to Running
        let _ = db.update_subagent_status(
            &run_id_clone, SubagentStatus::Running,
            None, None, None, None,
        );

        // Execute sub-agent with timeout, catch_unwind to guarantee completion event
        let agent_id_exec = agent_id.clone();
        let task_exec = task.clone();
        let model_override_exec = model_override.clone();
        let cancel_exec = cancel_flag.clone();

        let exec_result = std::panic::AssertUnwindSafe(
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                execute_subagent(agent_id_exec, task_exec, depth, model_override_exec, cancel_exec),
            )
        );
        let result = futures_util::FutureExt::catch_unwind(exec_result).await;

        let duration_ms = start.elapsed().as_millis() as u64;
        let finished_at = chrono::Utc::now().to_rfc3339();

        // Determine outcome — handles Ok, Err, Timeout, Cancel, and Panic
        let (status, result_text, error_text, model_used) = match result {
            Ok(Ok(Ok((response, model)))) => {
                let truncated = truncate_str(&response, MAX_RESULT_CHARS);
                (SubagentStatus::Completed, Some(truncated), None, model)
            }
            Ok(Ok(Err(e))) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    (SubagentStatus::Killed, None, Some("Killed by parent".into()), None)
                } else {
                    (SubagentStatus::Error, None, Some(e.to_string()), None)
                }
            }
            Ok(Err(_)) => {
                // Timeout
                (SubagentStatus::Timeout, None, Some(format!("Timed out after {}s", timeout_secs)), None)
            }
            Err(_panic) => {
                // Panic caught — still deliver the event
                (SubagentStatus::Error, None, Some("Sub-agent panicked unexpectedly".into()), None)
            }
        };

        // Save messages to child session so they're visible when clicking into it
        let _ = db.append_message(&child_session_id, &crate::session::NewMessage::user(&task));
        let reply_text = result_text.as_deref()
            .or(error_text.as_deref())
            .unwrap_or("(no response)");
        let _ = db.append_message(&child_session_id, &crate::session::NewMessage::assistant(reply_text));

        // Update DB — guaranteed to run even after panic
        let _ = db.update_subagent_status(
            &run_id_clone, status.clone(),
            result_text.as_deref(), error_text.as_deref(),
            model_used.as_deref(), Some(duration_ms),
        );
        let _ = db.set_subagent_finished_at(&run_id_clone, &finished_at);

        // Emit completion event — guaranteed to fire
        let result_preview = result_text.as_ref().map(|r| truncate_str(r, 200));
        emit_subagent_event(&SubagentEvent {
            event_type: status.as_str().to_string(),
            run_id: run_id_clone.clone(),
            parent_session_id,
            child_agent_id: agent_id,
            child_session_id: child_session_id_clone,
            task_preview: truncate_str(&task, 50),
            status,
            result_preview,
            error: error_text.clone(),
            duration_ms: Some(duration_ms),
            result_full: result_text,
        });

        // Cleanup cancel flag
        registry.remove(&run_id_clone);

        app_info!("subagent", "spawn", "Sub-agent run {} finished in {}ms", run_id_clone, duration_ms);
    });

    Ok(run_id)
}

/// Execute the sub-agent (runs within the spawned tokio task).
/// Returns (response_text, model_used).
fn execute_subagent(
    agent_id: String,
    task: String,
    depth: u32,
    model_override: Option<String>,
    cancel: Arc<AtomicBool>,
) -> impl std::future::Future<Output = Result<(String, Option<String>)>> + Send {
    async move {
    use crate::agent::AssistantAgent;
    use crate::failover;
    use crate::provider;

    const MAX_RETRIES: u32 = 2;
    const RETRY_BASE_MS: u64 = 1000;
    const RETRY_MAX_MS: u64 = 10_000;

    let store = provider::load_store().unwrap_or_default();

    // Load agent config for model resolution
    let agent_def = crate::agent_loader::load_agent(&agent_id)?;
    let agent_model_config = if let Some(ref override_str) = model_override {
        let mut cfg = agent_def.config.model.clone();
        cfg.primary = Some(override_str.clone());
        cfg
    } else {
        // Check if the agent's subagent config specifies a model override
        let subagent_model = agent_def.config.subagents.model.clone();
        if let Some(ref m) = subagent_model {
            let mut cfg = agent_def.config.model.clone();
            cfg.primary = Some(m.clone());
            cfg
        } else {
            agent_def.config.model.clone()
        }
    };

    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain.iter().any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        return Err(anyhow::anyhow!("No model configured for sub-agent execution"));
    }

    // Build extra system context for sub-agent
    let depth_info = if depth >= MAX_DEPTH {
        format!("- You are at maximum nesting depth ({}/{}) and CANNOT spawn further sub-agents.", depth, MAX_DEPTH)
    } else {
        format!("- Current nesting depth: {}/{}. You can delegate to sub-agents if needed.", depth, MAX_DEPTH)
    };

    let extra_context = format!(
        "## Execution Context\n\
         You are running as a **sub-agent** spawned by another agent.\n\
         - Task: {}\n\
         - {}\n\
         - Complete the task directly and concisely. Your full response will be returned to the parent agent.\n\
         - You do NOT have access to the parent's conversation history.\n\
         - This is an isolated session.",
        &task, depth_info
    );

    let mut last_error = String::new();
    for (_idx, model_ref) in model_chain.iter().enumerate() {
        let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
            Some(p) => p,
            None => continue,
        };

        let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);
        let mut retry_count: u32 = 0;

        loop {
            // Check cancellation before each attempt
            if cancel.load(Ordering::SeqCst) {
                return Err(anyhow::anyhow!("Sub-agent cancelled"));
            }

            let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id);
            agent.set_agent_id(&agent_id);
            agent.set_extra_system_context(extra_context.clone());
            agent.set_subagent_depth(depth);

            let cancel_clone = cancel.clone();
            match agent.chat(&task, &[], None, cancel_clone, |_delta| {}).await {
                Ok(response) => {
                    return Ok((response, Some(model_label)));
                }
                Err(e) => {
                    last_error = e.to_string();
                    let reason = failover::classify_error(&last_error);

                    if reason.is_terminal() {
                        return Err(anyhow::anyhow!("{}", last_error));
                    }

                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = std::cmp::min(
                            RETRY_BASE_MS * 2u64.pow(retry_count - 1),
                            RETRY_MAX_MS,
                        );
                        app_warn!("subagent", "retry",
                            "Model {} failed ({:?}), retry {}/{} in {}ms: {}",
                            model_label, reason, retry_count, MAX_RETRIES, delay, last_error
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Non-retryable or exhausted retries — try next model
                    app_warn!("subagent", "failover",
                        "Model {} failed ({:?}), moving to next model: {}",
                        model_label, reason, last_error
                    );
                    break;
                }
            }
        }
    }

    Err(anyhow::anyhow!("All models failed for sub-agent: {}", last_error))
    } // async move
}

// ── Startup Recovery ────────────────────────────────────────────

/// Clean up orphan sub-agent runs left in non-terminal state (spawning/running)
/// from a previous app session. Called once at startup.
pub fn cleanup_orphan_runs(session_db: &SessionDB) {
    match session_db.cleanup_orphan_subagent_runs() {
        Ok(affected) if affected > 0 => {
            app_warn!("subagent", "startup", "Cleaned up {} orphan sub-agent run(s)", affected);
        }
        Err(e) => {
            app_error!("subagent", "startup", "Failed to clean up orphan runs: {}", e);
        }
        _ => {}
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Truncate a string to max chars, appending "..." if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", cut)
    }
}

/// Emit a sub-agent event to the frontend via Tauri global event.
fn emit_subagent_event(event: &SubagentEvent) {
    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = handle.emit("subagent_event", event);
    }
}

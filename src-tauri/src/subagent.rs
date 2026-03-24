use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::session::SessionDB;

// ── Constants ────────────────────────────────────────────────────

/// Default maximum nesting depth for sub-agents
const DEFAULT_MAX_DEPTH: u32 = 3;

/// Get the effective max depth, checking global config.
pub fn max_depth() -> u32 {
    // In the future, this could read from a global config.
    // For now, individual agent configs can override via max_spawn_depth.
    DEFAULT_MAX_DEPTH
}

/// Get the effective max depth for a specific agent.
pub fn max_depth_for_agent(agent_id: &str) -> u32 {
    crate::agent_loader::load_agent(agent_id)
        .ok()
        .and_then(|def| def.config.subagents.max_spawn_depth)
        .map(|d| d.clamp(1, 5))
        .unwrap_or(DEFAULT_MAX_DEPTH)
}

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
    /// Optional display label for tracking
    pub label: Option<String>,
    /// Number of file attachments passed to the sub-agent
    pub attachment_count: u32,
    /// Input token usage (if available)
    pub input_tokens: Option<u64>,
    /// Output token usage (if available)
    pub output_tokens: Option<u64>,
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
    /// Optional display label for tracking
    pub label: Option<String>,
    /// File attachments to pass to the sub-agent
    pub attachments: Vec<crate::agent::Attachment>,
}

/// Event payload for streaming parent agent responses back to frontend.
/// Emitted when a sub-agent completes and the backend auto-injects the result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParentAgentStreamEvent {
    pub event_type: String,             // "started" | "delta" | "done" | "error"
    pub parent_session_id: String,
    pub run_id: String,
    pub push_message: Option<String>,   // only for "started"
    pub delta: Option<String>,          // raw JSON delta string, only for "delta"
    pub error: Option<String>,          // only for "error"
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
    /// Optional display label
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Input tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens used (available on terminal events)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Full result text — included only in terminal events for push delivery.
    /// Frontend uses this to auto-inject the result into the parent agent's conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_full: Option<String>,
}

// ── Steer Mailbox ───────────────────────────────────────────────

/// Per-run message queue for steering running sub-agents.
/// Parent agents push steer messages; the child agent's tool loop drains them each round.
pub struct SubagentMailbox {
    messages: Mutex<HashMap<String, Vec<String>>>,
}

impl SubagentMailbox {
    pub fn new() -> Self {
        Self { messages: Mutex::new(HashMap::new()) }
    }

    /// Push a steer message for the given run. Returns Err if run_id not registered.
    pub fn push(&self, run_id: &str, msg: String) -> bool {
        if let Ok(mut map) = self.messages.lock() {
            if let Some(queue) = map.get_mut(run_id) {
                queue.push(msg);
                return true;
            }
        }
        false
    }

    /// Drain all pending steer messages for a run (called by the child agent's tool loop).
    pub fn drain(&self, run_id: &str) -> Vec<String> {
        if let Ok(mut map) = self.messages.lock() {
            if let Some(queue) = map.get_mut(run_id) {
                return std::mem::take(queue);
            }
        }
        Vec::new()
    }

    /// Register a run_id slot (called at spawn time).
    pub fn register(&self, run_id: &str) {
        if let Ok(mut map) = self.messages.lock() {
            map.insert(run_id.to_string(), Vec::new());
        }
    }

    /// Remove a run_id slot (called when run terminates).
    pub fn remove(&self, run_id: &str) {
        if let Ok(mut map) = self.messages.lock() {
            map.remove(run_id);
        }
    }
}

/// Global steer mailbox — accessible from tools and agent providers.
pub static SUBAGENT_MAILBOX: std::sync::LazyLock<SubagentMailbox> =
    std::sync::LazyLock::new(SubagentMailbox::new);

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
    // 1. Validate depth (use parent agent's configured max)
    let effective_max_depth = max_depth_for_agent(&params.parent_agent_id);
    if params.depth > effective_max_depth {
        return Err(anyhow::anyhow!(
            "Sub-agent depth limit reached ({}/{}). Cannot spawn further sub-agents.",
            params.depth, effective_max_depth
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
    let attachment_count = params.attachments.len() as u32;
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
        label: params.label.clone(),
        attachment_count,
        input_tokens: None,
        output_tokens: None,
    };
    session_db.insert_subagent_run(&run)?;

    // 6. Register cancel flag and steer mailbox slot
    let cancel_flag = cancel_registry.register(&run_id);
    SUBAGENT_MAILBOX.register(&run_id);

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
        label: params.label.clone(),
        input_tokens: None,
        output_tokens: None,
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
    let parent_agent_id = params.parent_agent_id.clone();
    let child_session_id_clone = child_session_id.clone();
    let label = params.label.clone();
    let attachments = params.attachments.clone();

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

        let run_id_exec = run_id_clone.clone();
        let attachments_exec = attachments.clone();
        let exec_result = std::panic::AssertUnwindSafe(
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                execute_subagent(agent_id_exec, task_exec, depth, model_override_exec, cancel_exec, run_id_exec, attachments_exec),
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
        // Clone values needed after the move into SubagentEvent
        let status_for_inject = status.clone();
        let agent_id_for_inject = agent_id.clone();
        let result_text_for_inject = result_text.clone();
        let error_text_for_inject = error_text.clone();
        let parent_session_id_for_inject = parent_session_id.clone();
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
            label: label.clone(),
            input_tokens: None,  // TODO: extract from agent usage when available
            output_tokens: None,
            result_full: result_text,
        });

        // Cleanup cancel flag and steer mailbox
        registry.remove(&run_id_clone);
        SUBAGENT_MAILBOX.remove(&run_id_clone);

        app_info!("subagent", "spawn", "Sub-agent run {} finished in {}ms", run_id_clone, duration_ms);

        // Backend-driven result injection: push result to parent agent without relying on frontend.
        // Uses a dedicated OS thread + runtime to avoid the Send cycle:
        // inject_and_run_parent → agent.chat() → action_spawn → spawn_subagent → tokio::spawn
        if matches!(status_for_inject, SubagentStatus::Completed | SubagentStatus::Error | SubagentStatus::Timeout) {
            let push_msg = build_subagent_push_message(
                &run_id_clone, &agent_id_for_inject, &task, &status_for_inject, duration_ms,
                result_text_for_inject.as_deref(), error_text_for_inject.as_deref(),
            );
            let db2 = db.clone();
            let parent_sid2 = parent_session_id_for_inject;
            let parent_agent_id2 = parent_agent_id.clone();
            let child_agent_id2 = agent_id_for_inject.clone();
            let run_id2 = run_id_clone.clone();
            // Spawn on a separate OS thread so the future doesn't need to be Send
            std::thread::spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(inject_and_run_parent(
                        parent_sid2, parent_agent_id2, child_agent_id2, run_id2, push_msg, db2,
                    )),
                    Err(e) => app_error!("subagent", "inject", "Failed to build runtime for injection: {}", e),
                }
            });
        }
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
    run_id: String,
    attachments: Vec<crate::agent::Attachment>,
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
    let effective_max = max_depth_for_agent(&agent_id);
    let depth_info = if depth >= effective_max {
        format!("- You are at maximum nesting depth ({}/{}) and CANNOT spawn further sub-agents.", depth, effective_max)
    } else {
        format!("- Current nesting depth: {}/{}. You can delegate to sub-agents if needed.", depth, effective_max)
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
            agent.set_steer_run_id(run_id.clone());
            // Apply denied_tools from parent agent's subagent config
            if let Ok(parent_def) = crate::agent_loader::load_agent(&agent_id) {
                if !parent_def.config.subagents.denied_tools.is_empty() {
                    agent.set_denied_tools(parent_def.config.subagents.denied_tools.clone());
                }
            }

            let cancel_clone = cancel.clone();
            match agent.chat(&task, &attachments, None, cancel_clone, |_delta| {}).await {
                Ok((response, _thinking)) => {
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

/// Emit a parent agent stream event to the frontend.
fn emit_parent_stream_event(event: &ParentAgentStreamEvent) {
    if let Some(handle) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = handle.emit("parent_agent_stream", event);
    }
}

/// Build the push message text injected into the parent session.
fn build_subagent_push_message(
    run_id: &str,
    agent_id: &str,
    task: &str,
    status: &SubagentStatus,
    duration_ms: u64,
    result: Option<&str>,
    error: Option<&str>,
) -> String {
    let duration = format!("{:.1}s", duration_ms as f64 / 1000.0);
    let content = result.or(error).unwrap_or("(no output)");
    format!(
        "[Sub-Agent Completion — auto-delivered]\nRun ID: {}\nAgent: {}\nTask: {}\nStatus: {}\nDuration: {}\n<<<BEGIN_SUBAGENT_RESULT>>>\n{}\n<<<END_SUBAGENT_RESULT>>>",
        run_id, agent_id, truncate_str(task, 50), status.as_str(), duration, content
    )
}

// Global set tracking which parent sessions currently have an active backend injection.
// Prevents concurrent double-injection for the same session.
static INJECTING_SESSIONS: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// Sessions currently in a user-initiated chat() call.
/// Injection must wait until the session is idle.
pub static ACTIVE_CHAT_SESSIONS: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// Per-session cancel flags for active injections.
/// When the user starts a new chat() on a session, the injection cancel flag is set.
pub static INJECTION_CANCELS: std::sync::LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Run IDs whose results have been read by the parent agent via check/result tool actions.
/// If a run_id is here, auto-injection is skipped.
static FETCHED_RUN_IDS: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// Notify signal: fired when a session becomes idle (ChatSessionGuard dropped).
/// Injection waiters use this instead of polling.
static SESSION_IDLE_NOTIFY: std::sync::LazyLock<tokio::sync::Notify> =
    std::sync::LazyLock::new(|| tokio::sync::Notify::new());

/// RAII guard: marks a session as active in user chat, cancels any running injection.
/// Drop removes the session from the active set.
pub struct ChatSessionGuard {
    session_id: String,
}

impl ChatSessionGuard {
    pub fn new(session_id: &str) -> Self {
        if let Ok(mut set) = ACTIVE_CHAT_SESSIONS.lock() {
            set.insert(session_id.to_string());
        }
        // Cancel any running injection for this session
        if let Ok(map) = INJECTION_CANCELS.lock() {
            if let Some(cancel) = map.get(session_id) {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        Self { session_id: session_id.to_string() }
    }
}

impl Drop for ChatSessionGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = ACTIVE_CHAT_SESSIONS.lock() {
            set.remove(&self.session_id);
        }
        // Wake up any injection waiters (replaces 2s polling)
        SESSION_IDLE_NOTIFY.notify_waiters();
        // Re-trigger any pending injections that were cancelled during this chat
        flush_pending_injections(&self.session_id);
    }
}

/// Mark a run_id as having its result already read by the parent agent.
pub fn mark_run_fetched(run_id: &str) {
    if let Ok(mut set) = FETCHED_RUN_IDS.lock() {
        set.insert(run_id.to_string());
    }
}

/// A deferred injection task that was cancelled and needs to be retried.
#[derive(Clone)]
struct PendingInjection {
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
}

/// Queue of injection tasks that were cancelled (user sent new message) and need retry.
static PENDING_INJECTIONS: std::sync::LazyLock<Mutex<Vec<PendingInjection>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// Drain and re-trigger pending injections for a session.
/// Called from ChatSessionGuard::drop when a user chat completes.
fn flush_pending_injections(session_id: &str) {
    let tasks: Vec<PendingInjection> = {
        let mut queue = match PENDING_INJECTIONS.lock() {
            Ok(q) => q,
            Err(p) => p.into_inner(),
        };
        let mut remaining = Vec::new();
        let mut to_run = Vec::new();
        for task in queue.drain(..) {
            if task.parent_session_id == session_id {
                to_run.push(task);
            } else {
                remaining.push(task);
            }
        }
        *queue = remaining;
        to_run
    };

    for task in tasks {
        // Skip if already fetched, and clean up the entry
        {
            let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
            if set.remove(&task.run_id) {
                continue;
            }
        }
        let t = task.clone();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(inject_and_run_parent(
                    t.parent_session_id, t.parent_agent_id, t.child_agent_id,
                    t.run_id, t.push_message, t.session_db,
                )),
                Err(e) => app_error!("subagent", "inject", "Failed to build runtime for retry: {}", e),
            }
        });
        break; // Only re-trigger one at a time; next one queues on completion
    }
}

/// Backend-driven result injection: wait for idle, then run the parent agent with the push message.
/// Respects user chat priority: waits if busy, cancels if user sends a new message, skips if
/// the agent already fetched the result via check/result tool actions.
async fn inject_and_run_parent(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
) {
    use crate::agent::AssistantAgent;
    use crate::failover;
    use crate::provider;

    // 0. Skip if the parent agent already fetched this result via check/result tool
    {
        let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
        if set.contains(&run_id) {
            app_info!("subagent", "inject", "Run {} already fetched by parent, skipping injection", &run_id);
            set.remove(&run_id); // Clean up — no longer needed
            return;
        }
    }

    // Guard: if another injection is active for this session, queue for later
    {
        let mut guard = INJECTING_SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        if guard.contains(&parent_session_id) {
            app_info!("subagent", "inject",
                "Session {} already has active injection, queuing for later", &parent_session_id);
            if let Ok(mut queue) = PENDING_INJECTIONS.lock() {
                queue.push(PendingInjection {
                    parent_session_id, parent_agent_id, child_agent_id,
                    run_id, push_message, session_db,
                });
            }
            return;
        }
        guard.insert(parent_session_id.clone());
    }
    let _cleanup = CleanupGuard { session_id: parent_session_id.clone() };

    // 1. Wait for parent session to become idle (event-driven with timeout fallback)
    let announce_timeout = crate::agent_loader::load_agent(&parent_agent_id)
        .ok()
        .and_then(|def| def.config.subagents.announce_timeout_secs)
        .unwrap_or(120)
        .clamp(10, 600);
    let max_wait = std::time::Duration::from_secs(announce_timeout);
    let fallback_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let is_busy = ACTIVE_CHAT_SESSIONS.lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&parent_session_id);
        if !is_busy { break; }

        if start.elapsed() > max_wait {
            app_warn!("subagent", "inject",
                "Timed out waiting for session {} to become idle, skipping", &parent_session_id);
            return;
        }
        // Re-check if result was fetched while we were waiting
        if FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner()).contains(&run_id) {
            app_info!("subagent", "inject", "Run {} fetched while waiting, skipping", &run_id);
            return;
        }
        // Wait for notify (instant wake) or fallback timeout (in case notify is missed)
        tokio::select! {
            _ = SESSION_IDLE_NOTIFY.notified() => {}
            _ = tokio::time::sleep(fallback_interval) => {}
        }
    }

    // Final check before proceeding
    if FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner()).contains(&run_id) {
        return;
    }

    // 2. Register cancel flag — user's chat() will set this to abort the injection
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = INJECTION_CANCELS.lock() {
        map.insert(parent_session_id.clone(), cancel.clone());
    }
    // Ensure cancel flag is cleaned up on all exit paths
    let cancel_cleanup_sid = parent_session_id.clone();
    struct CancelCleanup { sid: String }
    impl Drop for CancelCleanup {
        fn drop(&mut self) {
            if let Ok(mut map) = INJECTION_CANCELS.lock() { map.remove(&self.sid); }
        }
    }
    let _cancel_cleanup = CancelCleanup { sid: cancel_cleanup_sid };

    // 3. Emit "started" so frontend can show loading state
    emit_parent_stream_event(&ParentAgentStreamEvent {
        event_type: "started".into(),
        parent_session_id: parent_session_id.clone(),
        run_id: run_id.clone(),
        push_message: Some(push_message.clone()),
        delta: None,
        error: None,
    });

    // 4. Build model chain
    let store = provider::load_store().unwrap_or_default();
    let agent_model_config = crate::agent_loader::load_agent(&parent_agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();
    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);
    let mut model_chain = Vec::new();
    if let Some(p) = primary { model_chain.push(p); }
    for fb in fallbacks {
        if !model_chain.iter().any(|m: &crate::provider::ActiveModel| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        app_error!("subagent", "inject", "No model configured for parent agent {}", &parent_agent_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id: parent_session_id.clone(),
            run_id,
            push_message: None,
            delta: None,
            error: Some("No model configured for parent agent".into()),
        });
        return;
    }

    const MAX_RETRIES: u32 = 2;
    const RETRY_BASE_MS: u64 = 1000;
    const RETRY_MAX_MS: u64 = 10_000;

    let mut last_error = String::new();
    let mut succeeded = false;

    'outer: for model_ref in &model_chain {
        let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
            Some(p) => p,
            None => continue,
        };
        let model_label = format!("{}::{}", model_ref.provider_id, model_ref.model_id);
        let mut retry_count = 0u32;

        loop {
            // Check cancel before each attempt
            if cancel.load(Ordering::SeqCst) {
                app_info!("subagent", "inject", "Injection cancelled before attempt for session {}", &parent_session_id);
                break 'outer;
            }

            let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id);
            agent.set_agent_id(&parent_agent_id);
            agent.set_session_id(&parent_session_id);

            // Restore parent conversation history from DB
            if let Ok(Some(json_str)) = session_db.load_context(&parent_session_id) {
                if let Ok(history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
                    if !history.is_empty() {
                        agent.set_conversation_history(history);
                    }
                }
            }

            let cancel_for_chat = cancel.clone();
            let parent_sid_for_cb = parent_session_id.clone();
            let run_id_for_cb = run_id.clone();

            match agent.chat(&push_message, &[], None, cancel_for_chat, move |delta| {
                emit_parent_stream_event(&ParentAgentStreamEvent {
                    event_type: "delta".into(),
                    parent_session_id: parent_sid_for_cb.clone(),
                    run_id: run_id_for_cb.clone(),
                    push_message: None,
                    delta: Some(delta.to_string()),
                    error: None,
                });
            }).await {
                Ok((response, _thinking)) => {
                    // If cancelled during execution, don't write to DB — user's chat takes over
                    if cancel.load(Ordering::SeqCst) {
                        app_info!("subagent", "inject",
                            "Injection cancelled during execution for session {}", &parent_session_id);
                        break 'outer;
                    }
                    // 5. Success: write push message + assistant response to DB
                    let mut user_msg = crate::session::NewMessage::user(&push_message);
                    user_msg.attachments_meta = Some(serde_json::json!({
                        "subagent_result": {
                            "run_id": &run_id,
                            "agent_id": &child_agent_id,
                        }
                    }).to_string());
                    let _ = session_db.append_message(&parent_session_id, &user_msg);
                    let _ = session_db.append_message(
                        &parent_session_id,
                        &crate::session::NewMessage::assistant(&response),
                    );
                    // Save updated conversation history
                    let history = agent.get_conversation_history();
                    if let Ok(json_str) = serde_json::to_string(&history) {
                        let _ = session_db.save_context(&parent_session_id, &json_str);
                    }
                    app_info!("subagent", "inject",
                        "Parent agent {} responded via model {}", &parent_agent_id, model_label);
                    succeeded = true;
                    break 'outer;
                }
                Err(e) => {
                    if cancel.load(Ordering::SeqCst) {
                        app_info!("subagent", "inject",
                            "Injection cancelled (error path) for session {}", &parent_session_id);
                        break 'outer;
                    }
                    last_error = e.to_string();
                    let reason = failover::classify_error(&last_error);
                    if reason.is_terminal() {
                        app_error!("subagent", "inject",
                            "Terminal error from {}: {}", model_label, last_error);
                        break 'outer;
                    }
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = std::cmp::min(RETRY_BASE_MS * 2u64.pow(retry_count - 1), RETRY_MAX_MS);
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }
                    app_warn!("subagent", "inject",
                        "Model {} failed ({:?}), trying next: {}", model_label, reason, last_error);
                    break;
                }
            }
        }
    }

    // 6. Emit final event
    let was_cancelled = cancel.load(Ordering::SeqCst);
    if was_cancelled {
        // Re-queue for retry after the user's chat completes
        if let Ok(mut queue) = PENDING_INJECTIONS.lock() {
            queue.push(PendingInjection {
                parent_session_id: parent_session_id.clone(),
                parent_agent_id: parent_agent_id.clone(),
                child_agent_id,
                run_id: run_id.clone(),
                push_message,
                session_db,
            });
        }
        app_info!("subagent", "inject",
            "Injection for run {} cancelled, re-queued for next idle", &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some("Cancelled: user started new chat, will retry when idle".into()),
        });
    } else if succeeded {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
    } else {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(format!("All models failed: {}", last_error)),
        });
    }
}

/// RAII guard that removes a session from INJECTING_SESSIONS when dropped.
struct CleanupGuard {
    session_id: String,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = INJECTING_SESSIONS.lock() {
            guard.remove(&self.session_id);
        }
        // Re-trigger next pending injection for this session (serial execution)
        flush_pending_injections(&self.session_id);
    }
}

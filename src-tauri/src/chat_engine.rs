//! Shared chat execution engine.
//!
//! Both `commands/chat.rs` (UI chat) and `channel/worker.rs` (IM channel chat)
//! call `run_chat_engine()` with different EventSink implementations.
//! This avoids duplicating the core Agent execution logic (streaming, failover,
//! tool persistence, context compaction, memory extraction).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::agent::{AssistantAgent, PlanAgentMode};
use crate::context_compact::CompactConfig;
use crate::provider::{self, ActiveModel, ApiType, ProviderConfig};
use crate::session::{self, SessionDB};
use crate::tools::image_generate::ImageGenConfig;
use crate::{agent_loader, context_compact, failover, memory, memory_extract};

// ── Shared Types ────────────────────────────────────────────────────

/// Token usage and metrics captured from streaming callbacks.
#[derive(Default)]
struct CapturedUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    model: Option<String>,
    ttft_ms: Option<i64>,
}

// ── EventSink trait ─────────────────────────────────────────────────

/// Abstract output layer for chat events.
/// UI chat uses `ChannelSink` (wraps `tauri::ipc::Channel<String>`),
/// IM channel worker uses `EmitSink` (Tauri global emit).
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: &str);
}

/// EventSink for UI chat — wraps `tauri::ipc::Channel<String>`.
pub struct ChannelSink {
    pub channel: tauri::ipc::Channel<String>,
}

impl EventSink for ChannelSink {
    fn send(&self, event: &str) {
        let _ = self.channel.send(event.to_string());
    }
}

/// EventSink for IM channel worker — pushes streaming events via Tauri global emit
/// AND forwards them to a background task for progressive Telegram message editing.
pub struct ChannelStreamSink {
    pub session_id: String,
    /// Forwards raw events to the channel streaming background task.
    pub event_tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl ChannelStreamSink {
    pub fn new(session_id: String, event_tx: tokio::sync::mpsc::UnboundedSender<String>) -> Self {
        Self {
            session_id,
            event_tx,
        }
    }
}

impl EventSink for ChannelStreamSink {
    fn send(&self, event: &str) {
        // 1. Emit to frontend for real-time streaming display
        if let Some(handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = handle.emit(
                "channel:stream_delta",
                serde_json::json!({
                    "sessionId": &self.session_id,
                    "event": event,
                }),
            );
        }
        // 2. Forward to background task for progressive IM channel delivery
        let _ = self.event_tx.send(event.to_string());
    }
}

// ── ChatEngineParams ────────────────────────────────────────────────

/// All parameters needed by the chat engine. Callers extract these from
/// `State<AppState>` (UI chat) or disk (channel worker).
pub struct ChatEngineParams {
    // Basic
    pub session_id: String,
    pub agent_id: String,
    pub message: String,
    pub session_db: Arc<SessionDB>,

    // Model chain (pre-resolved by caller)
    pub model_chain: Vec<ActiveModel>,
    /// Provider configs needed to build agents (snapshot, not reference to State)
    pub providers: Vec<ProviderConfig>,
    /// Codex OAuth token, if available
    pub codex_token: Option<(String, String)>,

    // Agent configuration
    pub resolved_temperature: Option<f64>,
    pub web_search_enabled: bool,
    pub notification_enabled: bool,
    pub image_gen_config: Option<ImageGenConfig>,
    pub canvas_enabled: bool,
    pub compact_config: CompactConfig,

    // Optional
    pub extra_system_context: Option<String>,
    pub reasoning_effort: Option<String>,
    pub cancel: Arc<AtomicBool>,
    /// Plan Mode agent configuration (set by chat command, None for channel worker)
    pub plan_agent_mode: Option<PlanAgentMode>,
    pub plan_mode_allow_paths: Option<Vec<String>>,

    // Output
    pub event_sink: Arc<dyn EventSink>,
}

/// Result returned by the chat engine.
pub struct ChatEngineResult {
    pub response: String,
    /// The model that produced the successful response.
    pub model_used: Option<ActiveModel>,
    /// The agent instance after chat (for UI chat to update State).
    pub agent: Option<AssistantAgent>,
}

// ── Agent Construction ──────────────────────────────────────────────

/// Build an AssistantAgent from provider configs (no State dependency).
fn build_agent_from_snapshot(
    model: &ActiveModel,
    providers: &[ProviderConfig],
    codex_token: &Option<(String, String)>,
    compact_config: &CompactConfig,
) -> Option<AssistantAgent> {
    let prov = provider::find_provider(providers, &model.provider_id)?;

    let mut agent = if prov.api_type == ApiType::Codex {
        let (access_token, account_id) = codex_token.as_ref()?;
        AssistantAgent::new_openai(access_token, account_id, &model.model_id)
    } else {
        AssistantAgent::new_from_provider(prov, &model.model_id)
    };
    agent.set_compact_config(compact_config.clone());
    Some(agent)
}

// ── Helper functions (moved from commands/chat.rs) ──────────────────

/// Restore conversation history from DB into the agent.
pub(crate) fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    if let Ok(Some(json_str)) = db.load_context(session_id) {
        if let Ok(history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            if !history.is_empty() {
                app_debug!(
                    "session",
                    "chat_engine",
                    "Restored {} messages for session {} ({}B JSON)",
                    history.len(),
                    session_id,
                    json_str.len()
                );
                agent.set_conversation_history(history);
            }
        }
    }
}

/// Save the agent's conversation history to DB.
pub(crate) fn save_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    let history = agent.get_conversation_history();
    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

/// Parse tool_call and tool_result events from the streaming callback and persist to DB.
pub(crate) fn persist_tool_event(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("tool_result") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                let duration_ms = event.get("duration_ms").and_then(|v| v.as_i64());
                let is_error = event
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let _ = db.update_tool_result(session_id, call_id, result, duration_ms, is_error);
            }
            Some("tool_call") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = event
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_msg = session::NewMessage::tool(call_id, name, arguments, "", None, false);
                let _ = db.append_message(session_id, &tool_msg);
            }
            _ => {}
        }
    }
}

/// If session is linked to an IM channel, forward the assistant reply.
pub(crate) async fn relay_to_channel(session_id: &str, response: &str) {
    let channel_db = match crate::get_channel_db() {
        Some(db) => db,
        None => return,
    };
    let registry = match crate::get_channel_registry() {
        Some(r) => r,
        None => return,
    };

    let conv = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        _ => return,
    };

    let store = crate::provider::load_store().unwrap_or_default();
    let account = match store.channels.find_account(&conv.account_id) {
        Some(a) => a.clone(),
        None => return,
    };

    let plugin = match registry.get_plugin(&account.channel_id) {
        Some(p) => p,
        None => return,
    };

    let native_text = plugin.markdown_to_native(response);
    let chunks = plugin.chunk_message(&native_text);

    for chunk in chunks {
        let payload = crate::channel::types::ReplyPayload {
            text: Some(chunk),
            parse_mode: Some(crate::channel::types::ParseMode::Html),
            thread_id: conv.thread_id.clone(),
            ..crate::channel::types::ReplyPayload::text("")
        };
        if let Err(e) = plugin
            .send_message(&account.id, &conv.chat_id, &payload)
            .await
        {
            app_error!(
                "channel",
                "relay",
                "Failed to relay to {}: {}",
                conv.channel_id,
                e
            );
        }
    }
}

/// Spawn async memory extraction if enabled for this agent.
fn spawn_memory_extraction(
    agent_id: &str,
    session_id: &str,
    model_ref: &ActiveModel,
    agent: &AssistantAgent,
) {
    let global_extract = memory::load_extract_config();
    let agent_def = agent_loader::load_agent(agent_id);
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);

    let auto_extract = agent_mem
        .and_then(|m| m.auto_extract)
        .unwrap_or(global_extract.auto_extract);
    let min_turns = agent_mem
        .and_then(|m| m.extract_min_turns)
        .unwrap_or(global_extract.extract_min_turns);
    let history = agent.get_conversation_history();

    if auto_extract && history.len() >= min_turns * 2 {
        let extract_agent_id = agent_id.to_string();
        let extract_session_id = session_id.to_string();
        let extract_provider_id = agent_mem
            .and_then(|m| m.extract_provider_id.clone())
            .or_else(|| global_extract.extract_provider_id.clone())
            .unwrap_or_else(|| model_ref.provider_id.clone());
        let extract_model_id = agent_mem
            .and_then(|m| m.extract_model_id.clone())
            .or_else(|| global_extract.extract_model_id.clone())
            .unwrap_or_else(|| model_ref.model_id.clone());

        tokio::spawn(async move {
            let store = provider::load_store().unwrap_or_default();
            if let Some(prov) = provider::find_provider(&store.providers, &extract_provider_id) {
                memory_extract::run_extraction(
                    &history,
                    &extract_agent_id,
                    &extract_session_id,
                    prov,
                    &extract_model_id,
                )
                .await;
            }
        });
    }
}

// ── Core Chat Engine ────────────────────────────────────────────────

/// Run the shared chat execution engine.
///
/// Handles: model chain traversal → agent building → config → history restoration
/// → streaming execution → tool persistence → failover → context compaction
/// → response saving → context persistence → memory extraction.
pub async fn run_chat_engine(params: ChatEngineParams) -> Result<ChatEngineResult, String> {
    let ChatEngineParams {
        session_id,
        agent_id,
        message,
        session_db: db,
        model_chain,
        providers,
        codex_token,
        resolved_temperature,
        web_search_enabled,
        notification_enabled,
        image_gen_config,
        canvas_enabled,
        compact_config,
        extra_system_context,
        reasoning_effort,
        cancel,
        plan_agent_mode,
        plan_mode_allow_paths,
        event_sink,
    } = params;

    if model_chain.is_empty() {
        return Err("No model configured for chat execution".to_string());
    }

    let total_models = model_chain.len();
    let mut last_error: Option<String> = None;

    // Build primary model display name for fallback events
    let primary_display = {
        let first = &model_chain[0];
        let prov_name = providers
            .iter()
            .find(|p| p.id == first.provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or(&first.provider_id);
        format!("{} / {}", prov_name, first.model_id)
    };

    let effort_str = reasoning_effort.clone();

    for (idx, model_ref) in model_chain.iter().enumerate() {
        let mut agent =
            match build_agent_from_snapshot(model_ref, &providers, &codex_token, &compact_config) {
                Some(a) => a,
                None => {
                    last_error = Some(format!(
                        "Cannot build agent for {}::{}",
                        model_ref.provider_id, model_ref.model_id
                    ));
                    continue;
                }
            };
        agent.set_agent_id(&agent_id);
        agent.set_session_id(&session_id);
        agent.set_web_search_enabled(web_search_enabled);
        agent.set_notification_enabled(notification_enabled);
        agent.set_image_generate_config(image_gen_config.clone());
        agent.set_canvas_enabled(canvas_enabled);
        agent.set_temperature(resolved_temperature);

        if let Some(ref ctx) = extra_system_context {
            agent.set_extra_system_context(ctx.clone());
        }
        if let Some(ref mode) = plan_agent_mode {
            agent.set_plan_agent_mode(mode.clone());
        }
        if let Some(ref paths) = plan_mode_allow_paths {
            agent.set_plan_mode_allow_paths(paths.clone());
        }

        // Restore conversation history from DB
        restore_agent_context(&db, &session_id, &agent);

        // Update session with current model info
        {
            let provider_name = providers
                .iter()
                .find(|p| p.id == model_ref.provider_id)
                .map(|p| p.name.as_str());
            let _ = db.update_session_model(
                &session_id,
                Some(&model_ref.provider_id),
                provider_name,
                Some(&model_ref.model_id),
            );
        }

        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10000;
        let mut retry_count: u32 = 0;

        loop {
            // Emit fallback event if this is not the first model
            if idx > 0 && retry_count == 0 {
                let display = {
                    let prov_name = providers
                        .iter()
                        .find(|p| p.id == model_ref.provider_id)
                        .map(|p| p.name.as_str())
                        .unwrap_or(&model_ref.provider_id);
                    format!("{} / {}", prov_name, model_ref.model_id)
                };
                let reason_str = last_error
                    .as_deref()
                    .map(|e| failover::classify_error(e))
                    .unwrap_or(failover::FailoverReason::Unknown);
                let event = serde_json::json!({
                    "type": "model_fallback",
                    "model": display,
                    "from_model": primary_display,
                    "provider_id": model_ref.provider_id,
                    "model_id": model_ref.model_id,
                    "reason": reason_str,
                    "attempt": idx + 1,
                    "total": total_models,
                    "error": last_error.as_deref().unwrap_or(""),
                });
                if let Ok(json_str) = serde_json::to_string(&event) {
                    event_sink.send(&json_str);
                    let _ = db.append_message(&session_id, &session::NewMessage::event(&json_str));
                }
            }

            let effort_ref = effort_str.as_deref();
            let cancel_clone = cancel.clone();

            // Shared state for capturing usage/TTFT from on_delta
            let captured_usage: Arc<std::sync::Mutex<CapturedUsage>> =
                Arc::new(std::sync::Mutex::new(CapturedUsage::default()));
            let captured_usage_clone = captured_usage.clone();

            // Accumulate text_delta / thinking_delta for ordering preservation
            let pending_text: Arc<std::sync::Mutex<String>> =
                Arc::new(std::sync::Mutex::new(String::new()));
            let pending_text_clone = pending_text.clone();
            let pending_thinking: Arc<std::sync::Mutex<String>> =
                Arc::new(std::sync::Mutex::new(String::new()));
            let pending_thinking_clone = pending_thinking.clone();
            let thinking_start_time: Arc<std::sync::Mutex<Option<std::time::Instant>>> =
                Arc::new(std::sync::Mutex::new(None));
            let thinking_start_clone = thinking_start_time.clone();
            let had_thinking_blocks: Arc<std::sync::atomic::AtomicBool> =
                Arc::new(std::sync::atomic::AtomicBool::new(false));
            let had_thinking_blocks_clone = had_thinking_blocks.clone();

            let db_for_cb = db.clone();
            let sid_for_cb = session_id.clone();
            let event_sink_clone = event_sink.clone();

            let chat_start = std::time::Instant::now();
            match agent
                .chat(&message, &[], effort_ref, cancel_clone, move |delta| {
                    // Intercept usage events
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                        match event.get("type").and_then(|t| t.as_str()) {
                            Some("usage") => {
                                if let Ok(mut u) = captured_usage_clone.lock() {
                                    if let Some(v) =
                                        event.get("input_tokens").and_then(|v| v.as_i64())
                                    {
                                        u.input_tokens = Some(v);
                                    }
                                    if let Some(v) =
                                        event.get("output_tokens").and_then(|v| v.as_i64())
                                    {
                                        u.output_tokens = Some(v);
                                    }
                                    if let Some(v) = event.get("model").and_then(|v| v.as_str()) {
                                        u.model = Some(v.to_string());
                                    }
                                    if let Some(v) = event.get("ttft_ms").and_then(|v| v.as_i64()) {
                                        u.ttft_ms = Some(v);
                                    }
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                                    // Record start time on first thinking_delta
                                    if let Ok(mut ts) = thinking_start_clone.lock() {
                                        if ts.is_none() {
                                            *ts = Some(std::time::Instant::now());
                                        }
                                    }
                                    if let Ok(mut pk) = pending_thinking_clone.lock() {
                                        pk.push_str(text);
                                    }
                                }
                            }
                            Some("text_delta") => {
                                if let Some(text) = event.get("text").and_then(|t| t.as_str()) {
                                    if let Ok(mut pt) = pending_text_clone.lock() {
                                        pt.push_str(text);
                                    }
                                }
                            }
                            Some("tool_call") => {
                                // Flush accumulated thinking before tool_call
                                if let Ok(mut pk) = pending_thinking_clone.lock() {
                                    if !pk.is_empty() {
                                        let duration = thinking_start_clone.lock().ok()
                                            .and_then(|mut ts| ts.take())
                                            .map(|t| t.elapsed().as_millis() as i64);
                                        let thinking_msg = session::NewMessage::thinking_block_with_duration(&pk, duration);
                                        let _ =
                                            db_for_cb.append_message(&sid_for_cb, &thinking_msg);
                                        pk.clear();
                                        had_thinking_blocks_clone
                                            .store(true, std::sync::atomic::Ordering::SeqCst);
                                    }
                                }
                                // Flush accumulated text before tool_call
                                if let Ok(mut pt) = pending_text_clone.lock() {
                                    if !pt.is_empty() {
                                        let text_msg = session::NewMessage::text_block(&pt);
                                        let _ = db_for_cb.append_message(&sid_for_cb, &text_msg);
                                        pt.clear();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                    event_sink_clone.send(delta);
                })
                .await
            {
                Ok((result, thinking)) => {
                    let duration_ms = chat_start.elapsed().as_millis() as u64;

                    // Emit usage event with duration
                    let usage_event = serde_json::json!({
                        "type": "usage",
                        "duration_ms": duration_ms,
                    });
                    if let Ok(json_str) = serde_json::to_string(&usage_event) {
                        event_sink.send(&json_str);
                    }

                    // Flush remaining pending thinking
                    if let Ok(mut pk) = pending_thinking.lock() {
                        if !pk.is_empty() {
                            let duration = thinking_start_time.lock().ok()
                                .and_then(|mut ts| ts.take())
                                .map(|t| t.elapsed().as_millis() as i64);
                            let thinking_msg = session::NewMessage::thinking_block_with_duration(&pk, duration);
                            let _ = db.append_message(&session_id, &thinking_msg);
                            pk.clear();
                            had_thinking_blocks.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let has_thinking_blocks =
                        had_thinking_blocks.load(std::sync::atomic::Ordering::SeqCst);

                    // Save assistant reply with metadata
                    let mut assistant_msg = session::NewMessage::assistant(&result);
                    assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                    if !has_thinking_blocks {
                        assistant_msg.thinking = thinking;
                    }
                    if let Ok(u) = captured_usage.lock() {
                        assistant_msg.tokens_in = u.input_tokens;
                        assistant_msg.tokens_out = u.output_tokens;
                        assistant_msg.model = u.model.clone();
                        assistant_msg.ttft_ms = u.ttft_ms;
                    }
                    let _ = db.append_message(&session_id, &assistant_msg);

                    // Persist conversation context
                    save_agent_context(&db, &session_id, &agent);

                    // Spawn async memory extraction
                    spawn_memory_extraction(&agent_id, &session_id, model_ref, &agent);

                    return Ok(ChatEngineResult {
                        response: result,
                        model_used: Some(model_ref.clone()),
                        agent: Some(agent),
                    });
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let reason = failover::classify_error(&error_msg);

                    app_warn!(
                        "provider",
                        "failover",
                        "Model {}::{} failed (attempt {}/{}, retry {}, reason {:?}): {}",
                        model_ref.provider_id,
                        model_ref.model_id,
                        idx + 1,
                        total_models,
                        retry_count,
                        reason,
                        error_msg
                    );

                    // Context overflow — try emergency compaction, then retry once
                    if reason.needs_compaction() && retry_count == 0 {
                        app_info!(
                            "context",
                            "compact",
                            "Context overflow on {}::{}, attempting emergency compaction",
                            model_ref.provider_id,
                            model_ref.model_id
                        );
                        let mut history = agent.get_conversation_history();
                        let compact_result =
                            context_compact::emergency_compact(&mut history, &compact_config);
                        agent.set_conversation_history(history);
                        save_agent_context(&db, &session_id, &agent);

                        if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                            "type": "context_compacted",
                            "data": compact_result,
                        })) {
                            event_sink.send(&event_str);
                        }

                        retry_count += 1;
                        continue;
                    }

                    // Terminal errors — surface immediately
                    if reason.is_terminal() || reason.needs_compaction() {
                        save_agent_context(&db, &session_id, &agent);
                        let _ =
                            db.append_message(&session_id, &session::NewMessage::event(&error_msg));
                        return Err(error_msg);
                    }

                    // Retryable errors — retry same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay =
                            failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        app_info!(
                            "provider",
                            "failover",
                            "Retrying {}::{} in {}ms (retry {}/{})",
                            model_ref.provider_id,
                            model_ref.model_id,
                            delay,
                            retry_count,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue;
                    }

                    // Emit codex_auth_expired when a Codex provider gets an Auth error
                    if matches!(reason, failover::FailoverReason::Auth) {
                        let is_codex = providers
                            .iter()
                            .find(|p| p.id == model_ref.provider_id)
                            .map(|p| p.api_type == ApiType::Codex)
                            .unwrap_or(false);
                        if is_codex {
                            if let Ok(json_str) = serde_json::to_string(&serde_json::json!({
                                "type": "codex_auth_expired",
                                "error": &error_msg,
                            })) {
                                event_sink.send(&json_str);
                            }
                        }
                    }

                    // Non-retryable or retries exhausted — move to next model
                    last_error = Some(error_msg);
                    break;
                }
            }
        }
    }

    let final_error =
        last_error.unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    let _ = db.append_message(&session_id, &session::NewMessage::event(&final_error));
    Err(final_error)
}

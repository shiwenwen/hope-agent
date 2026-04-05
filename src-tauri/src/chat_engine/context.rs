use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::context_compact::CompactConfig;
use crate::provider::{self, ActiveModel, ApiType, ProviderConfig};
use crate::session::{self, SessionDB};

// ── Agent Construction ──────────────────────────────────────────────

/// Build an AssistantAgent from provider configs (no State dependency).
pub(super) fn build_agent_from_snapshot(
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

/// Run memory extraction inline (non-spawned) to enable side_query cache sharing.
/// The user's response has already been streamed, so the 1-3s extraction latency
/// is acceptable. Inline execution allows passing `&agent` for prompt cache reuse.
pub(super) async fn run_memory_extraction_inline(
    agent_id: &str,
    session_id: &str,
    model_ref: &ActiveModel,
    agent: &AssistantAgent,
) {
    let global_extract = crate::memory::load_extract_config();
    let agent_def = crate::agent_loader::load_agent(agent_id);
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);

    let auto_extract = agent_mem
        .and_then(|m| m.auto_extract)
        .unwrap_or(global_extract.auto_extract);
    let min_turns = agent_mem
        .and_then(|m| m.extract_min_turns)
        .unwrap_or(global_extract.extract_min_turns);
    let max_per_session = global_extract.max_extractions_per_session;
    let history = agent.get_conversation_history();

    // Gate 1: auto_extract enabled
    if !auto_extract {
        return;
    }

    // Gate 2: minimum conversation length
    if history.len() < min_turns * 2 {
        return;
    }

    // Gate 3: mutual exclusion — skip if save_memory was called this round
    if agent
        .manual_memory_saved
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        app_info!(
            "memory",
            "auto_extract",
            "Skipping extraction: manual save_memory called this round"
        );
        return;
    }

    // Gate 4: frequency cap
    let current_count = agent
        .extraction_count
        .load(std::sync::atomic::Ordering::SeqCst);
    if current_count >= max_per_session as u32 {
        return;
    }

    // Resolve provider/model for extraction
    let extract_provider_id = agent_mem
        .and_then(|m| m.extract_provider_id.clone())
        .or_else(|| global_extract.extract_provider_id.clone())
        .unwrap_or_else(|| model_ref.provider_id.clone());
    let extract_model_id = agent_mem
        .and_then(|m| m.extract_model_id.clone())
        .or_else(|| global_extract.extract_model_id.clone())
        .unwrap_or_else(|| model_ref.model_id.clone());

    let store = provider::load_store().unwrap_or_default();
    if let Some(prov) = provider::find_provider(&store.providers, &extract_provider_id) {
        // Increment count only when extraction actually runs (not on provider lookup failure)
        agent
            .extraction_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // Pass Some(agent) to enable side_query prompt cache sharing
        crate::memory_extract::run_extraction(
            &history,
            agent_id,
            session_id,
            prov,
            &extract_model_id,
            Some(agent),
        )
        .await;
    }
}

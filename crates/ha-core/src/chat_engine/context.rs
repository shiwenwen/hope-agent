use std::sync::Arc;

use crate::agent::AssistantAgent;
use crate::context_compact::CompactConfig;
use crate::provider::{self, ActiveModel, AuthProfile, ProviderConfig};
use crate::session::{self, SessionDB};
use serde_json::{json, Value};

// ── Agent Construction ──────────────────────────────────────────────

/// Build an AssistantAgent from provider configs (no State dependency).
///
/// When `profile` is `Some`, the agent is constructed with that specific
/// auth profile's API key and base_url override. When `None`, the first
/// effective profile (or legacy `api_key`) is used.
pub(super) async fn build_agent_from_snapshot(
    model: &ActiveModel,
    providers: &[ProviderConfig],
    codex_token_hint: Option<(String, String)>,
    compact_config: &CompactConfig,
    profile: Option<&AuthProfile>,
    session_id: &str,
) -> anyhow::Result<AssistantAgent> {
    let prov = provider::find_provider(providers, &model.provider_id)
        .ok_or_else(|| anyhow::anyhow!("Provider {} not found", model.provider_id))?;

    let agent = AssistantAgent::try_new_from_provider_with_codex_hint(
        prov,
        &model.model_id,
        profile,
        codex_token_hint,
    )
    .await?;

    let mut agent = agent.with_failover_context(prov);
    agent.set_compact_config(compact_config.clone());

    if let Some(ref model_ref) = compact_config.summarization_model {
        if let Some(cp) = crate::agent::build_compaction_provider(model_ref, providers, session_id)
        {
            agent.set_compaction_provider(Some(std::sync::Arc::new(cp)));
        }
    }

    Ok(agent)
}

// ── Helper functions (moved from commands/chat.rs) ──────────────────

/// Restore conversation history from DB into the agent.
pub fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    if let Ok(Some(json_str)) = db.load_context(session_id) {
        if let Ok(mut history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            if !history.is_empty() {
                let repaired = repair_failed_prefix_from_messages(db, session_id, &mut history);
                app_debug!(
                    "session",
                    "chat_engine",
                    "Restored {} messages for session {} ({}B JSON)",
                    history.len(),
                    session_id,
                    json_str.len()
                );
                agent.set_conversation_history(history);
                if repaired {
                    save_agent_context(db, session_id, agent);
                }
            }
        }
    }
}

/// Save the agent's conversation history to DB.
pub fn save_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    let history = agent.get_conversation_history();
    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

/// Preserve a failed user turn in the model-facing conversation context.
///
/// The visible message row is inserted before model execution, but
/// `context_json` is normally saved only after a successful assistant turn.
/// If the provider request fails before completion, the next "retry" message
/// would otherwise lose the original task from the model's history. Store a
/// compact assistant-side failure marker as well so the model can distinguish
/// "no answer was produced" from a normal assistant reply.
pub fn persist_failed_turn_context(
    db: &Arc<SessionDB>,
    session_id: &str,
    user_message: &str,
    error: &str,
) {
    let mut history = db
        .load_context(session_id)
        .ok()
        .flatten()
        .and_then(|json_str| serde_json::from_str::<Vec<Value>>(&json_str).ok())
        .unwrap_or_default();

    push_user_for_failed_turn(&mut history, user_message);

    let error = crate::util::truncate_utf8(error.trim(), 2_000);
    let marker = if error.is_empty() {
        "[System event] Previous assistant turn failed before producing a response.".to_string()
    } else {
        format!(
            "[System event] Previous assistant turn failed before producing a response. Error: {}",
            error
        )
    };
    if !last_assistant_message_is(&history, &marker) {
        history.push(json!({ "role": "assistant", "content": marker }));
    }

    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

fn push_user_for_failed_turn(history: &mut Vec<Value>, user_message: &str) {
    let user_message = user_message.trim();
    if user_message.is_empty() {
        return;
    }

    if let Some(last) = history.last_mut() {
        if last.get("role").and_then(|r| r.as_str()) == Some("user") {
            if value_text_contains(last.get("content"), user_message) {
                return;
            }
            let existing = last.get("content").cloned().unwrap_or(Value::Null);
            last["content"] = merge_user_content(existing, user_message);
            return;
        }
    }

    if history.iter().rev().take(4).any(|item| {
        item.get("role").and_then(|r| r.as_str()) == Some("user")
            && value_text_contains(item.get("content"), user_message)
    }) {
        return;
    }

    history.push(json!({ "role": "user", "content": user_message }));
}

fn repair_failed_prefix_from_messages(
    db: &Arc<SessionDB>,
    session_id: &str,
    history: &mut Vec<Value>,
) -> bool {
    let Some(first_history_user) = history.iter().find_map(history_user_text) else {
        return false;
    };

    let Ok(messages) = db.load_session_messages(session_id) else {
        return false;
    };

    let Some(anchor_idx) = messages.iter().position(|msg| {
        matches!(msg.role, session::MessageRole::User)
            && !msg.content.trim().is_empty()
            && first_history_user.contains(msg.content.trim())
    }) else {
        return false;
    };
    if anchor_idx == 0 {
        return false;
    }

    let Some(failed_tail) = failed_turn_tail_before_anchor(&messages[..anchor_idx]) else {
        return false;
    };

    let mut prefix = Vec::new();
    for msg in failed_tail {
        match msg.role {
            session::MessageRole::User => push_user_for_failed_turn(&mut prefix, &msg.content),
            session::MessageRole::Assistant if !msg.content.trim().is_empty() => {
                prefix.push(json!({ "role": "assistant", "content": msg.content.trim() }));
            }
            session::MessageRole::Event if msg.is_error.unwrap_or(false) => {
                let error = crate::util::truncate_utf8(msg.content.trim(), 2_000);
                prefix.push(json!({
                    "role": "assistant",
                    "content": format!(
                        "[System event] Previous assistant turn failed before producing a response. Error: {}",
                        error
                    )
                }));
            }
            _ => {}
        }
    }

    if prefix.is_empty() {
        return false;
    }

    prefix.extend(std::mem::take(history));
    *history = prefix;
    true
}

fn failed_turn_tail_before_anchor(
    messages_before_anchor: &[session::SessionMessage],
) -> Option<&[session::SessionMessage]> {
    let last = messages_before_anchor.last()?;
    if !matches!(last.role, session::MessageRole::Event) || !last.is_error.unwrap_or(false) {
        return None;
    }

    let start = messages_before_anchor
        .iter()
        .rposition(|msg| matches!(msg.role, session::MessageRole::User))?;
    Some(&messages_before_anchor[start..])
}

fn history_user_text(item: &Value) -> Option<String> {
    if item.get("role").and_then(|r| r.as_str()) != Some("user") {
        return None;
    }
    value_text(item.get("content"))
}

fn merge_user_content(existing: Value, user_message: &str) -> Value {
    match existing {
        Value::String(old) if old.is_empty() => json!(user_message),
        Value::String(old) => json!(format!("{}\n\n{}", old, user_message)),
        Value::Array(mut parts) => {
            parts.push(json!({ "type": "text", "text": user_message }));
            Value::Array(parts)
        }
        _ => json!(user_message),
    }
}

fn value_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .or_else(|| part.get("content"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn value_text_contains(value: Option<&Value>, needle: &str) -> bool {
    match value {
        Some(Value::String(text)) => text.contains(needle),
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            part.get("text")
                .or_else(|| part.get("content"))
                .and_then(|v| v.as_str())
                .is_some_and(|text| text.contains(needle))
        }),
        _ => false,
    }
}

fn last_assistant_message_is(history: &[Value], content: &str) -> bool {
    history
        .last()
        .filter(|item| item.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .and_then(|item| item.get("content"))
        .is_some_and(|value| value_text_contains(Some(value), content))
}

/// Parse tool_call and tool_result events from the streaming callback and persist to DB.
pub fn persist_tool_event(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
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
                let metadata_json: Option<String> = event
                    .get("tool_metadata")
                    .filter(|v| !v.is_null())
                    .and_then(|v| serde_json::to_string(v).ok());
                let _ = db.update_tool_result_with_metadata(
                    session_id,
                    call_id,
                    result,
                    duration_ms,
                    is_error,
                    metadata_json.as_deref(),
                );
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
pub async fn relay_to_channel(session_id: &str, response: &str) {
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

    let store = crate::config::cached_config();
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

/// Schedule memory extraction after a successful turn. Returns the resolved
/// idle_timeout_secs so the caller can schedule idle extraction without
/// re-loading config.
///
/// Trigger logic (since last extraction):
/// - Cooldown: elapsed time must >= time threshold (prevents too-frequent extraction)
/// - Trigger: token count >= token threshold OR message count >= message threshold
///
/// Both cooldown AND trigger must be satisfied.
pub(super) fn schedule_memory_extraction_after_turn(
    agent_id: &str,
    session_id: &str,
    model_ref: &ActiveModel,
    agent: &AssistantAgent,
) -> u64 {
    if crate::session::is_session_incognito(Some(session_id)) {
        return 0;
    }
    let global_extract = crate::memory::load_extract_config();
    let agent_def = crate::agent_loader::load_agent(agent_id);
    let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);

    let auto_extract = agent_mem
        .and_then(|m| m.auto_extract)
        .unwrap_or(global_extract.auto_extract);
    let idle_timeout = agent_mem
        .and_then(|m| m.extract_idle_timeout_secs)
        .unwrap_or(global_extract.extract_idle_timeout_secs);

    if !auto_extract {
        return 0;
    }

    if agent
        .manual_memory_saved
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        app_info!(
            "memory",
            "auto_extract",
            "Skipping extraction: manual save_memory called this round"
        );
        return idle_timeout;
    }

    let token_threshold = agent_mem
        .and_then(|m| m.extract_token_threshold)
        .unwrap_or(global_extract.extract_token_threshold);
    let cooldown_secs = agent_mem
        .and_then(|m| m.extract_time_threshold_secs)
        .unwrap_or(global_extract.extract_time_threshold_secs);
    let message_threshold = agent_mem
        .and_then(|m| m.extract_message_threshold)
        .unwrap_or(global_extract.extract_message_threshold);

    let tokens_acc = agent
        .tokens_since_extraction
        .load(std::sync::atomic::Ordering::SeqCst) as usize;
    let messages_acc = agent
        .messages_since_extraction
        .load(std::sync::atomic::Ordering::SeqCst) as usize;
    let elapsed_secs = agent
        .last_extraction_at
        .lock()
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    if elapsed_secs < cooldown_secs {
        return idle_timeout;
    }

    let token_met = tokens_acc >= token_threshold;
    let message_met = messages_acc >= message_threshold;

    if !token_met && !message_met {
        return idle_timeout;
    }

    app_info!(
        "memory",
        "auto_extract",
        "Extraction scheduled: tokens={}/{} msgs={}/{} cooldown={}s/{}s (session: {})",
        tokens_acc,
        token_threshold,
        messages_acc,
        message_threshold,
        elapsed_secs,
        cooldown_secs,
        session_id
    );

    // Resolve provider/model for extraction
    let extract_provider_id = agent_mem
        .and_then(|m| m.extract_provider_id.clone())
        .or_else(|| global_extract.extract_provider_id.clone())
        .unwrap_or_else(|| model_ref.provider_id.clone());
    let extract_model_id = agent_mem
        .and_then(|m| m.extract_model_id.clone())
        .or_else(|| global_extract.extract_model_id.clone())
        .unwrap_or_else(|| model_ref.model_id.clone());

    let history = agent.get_conversation_history();
    let store = crate::config::cached_config();
    if let Some(prov) = provider::find_provider(&store.providers, &extract_provider_id).cloned() {
        let agent_id = agent_id.to_string();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            crate::memory_extract::run_extraction(
                &history,
                &agent_id,
                &session_id,
                &prov,
                &extract_model_id,
                None,
            )
            .await;
        });
        agent.reset_extraction_tracking();
    } else {
        app_warn!(
            "memory",
            "auto_extract",
            "Extraction provider {} not found for session {}",
            extract_provider_id,
            session_id
        );
    }
    idle_timeout
}

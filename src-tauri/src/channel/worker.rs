use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;
use tauri::Emitter;

use super::db::ChannelDB;
use super::registry::ChannelRegistry;
use super::types::*;

/// Notify the frontend that a channel session has new messages.
fn emit_channel_update(session_id: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit("channel:message_update", serde_json::json!({
            "sessionId": session_id,
        }));
    }
}

/// Spawn the inbound message dispatcher as a background tokio task.
///
/// This task receives `MsgContext` from all channel plugins and:
/// 1. Validates access control
/// 2. Resolves or creates a session
/// 3. Builds an AssistantAgent and runs the chat
/// 4. Sends the response back through the channel
pub fn spawn_dispatcher(
    registry: Arc<ChannelRegistry>,
    channel_db: Arc<ChannelDB>,
    mut inbound_rx: mpsc::Receiver<MsgContext>,
) {
    tauri::async_runtime::spawn(async move {
        app_info!("channel", "worker", "Inbound message dispatcher started");

        while let Some(msg) = inbound_rx.recv().await {
            let registry = registry.clone();
            let channel_db = channel_db.clone();

            // Handle each message in a separate task for concurrency
            tauri::async_runtime::spawn(async move {
                if let Err(e) = handle_inbound_message(&registry, &channel_db, msg).await {
                    app_error!("channel", "worker", "Failed to handle inbound message: {}", e);
                }
            });
        }

        app_info!("channel", "worker", "Inbound message dispatcher stopped");
    });
}

/// Process a single inbound message from a channel.
async fn handle_inbound_message(
    registry: &ChannelRegistry,
    channel_db: &ChannelDB,
    msg: MsgContext,
) -> anyhow::Result<()> {
    let channel_id_str = msg.channel_id.to_string();
    let sender_label = msg.sender_name.as_deref()
        .or(msg.sender_username.as_deref())
        .unwrap_or(&msg.sender_id);
    app_info!("channel", "worker", "[{}] Message from {} in {}: {}",
        channel_id_str, sender_label, msg.chat_id,
        crate::truncate_utf8(msg.text.as_deref().unwrap_or("(media)"), 100));

    // 1. Load config and find account
    let store = crate::provider::load_store().unwrap_or_default();
    let account = store.channels.find_account(&msg.account_id)
        .ok_or_else(|| anyhow::anyhow!("Account '{}' not found in config", msg.account_id))?
        .clone();

    // 2. Check access control
    let plugin = registry.get_plugin(&msg.channel_id)
        .ok_or_else(|| anyhow::anyhow!("No plugin for channel: {}", msg.channel_id))?;

    if !plugin.check_access(&account, &msg) {
        app_warn!("channel", "worker", "[{}] Access denied for sender {} in {}",
            channel_id_str, msg.sender_id, msg.chat_id);
        return Ok(());
    }

    // 3. Resolve agent_id: per-account binding > global default
    let agent_id = account.agent_id.as_deref()
        .unwrap_or_else(|| store.channels.agent_id())
        .to_string();
    let session_id = channel_db.resolve_or_create_session(
        &channel_id_str,
        &msg.account_id,
        &msg.chat_id,
        msg.thread_id.as_deref(),
        Some(&msg.sender_id),
        msg.sender_name.as_deref(),
        &msg.chat_type,
        &agent_id,
    )?;

    // 4. Save user message to session
    let session_db = crate::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))?;

    let user_text = msg.text.as_deref().unwrap_or("(media message)");
    let mut user_msg = crate::session::NewMessage::user(user_text);
    user_msg.attachments_meta = Some(serde_json::json!({
        "channel_inbound": {
            "channelId": channel_id_str,
            "accountId": msg.account_id,
            "senderId": msg.sender_id,
            "senderName": msg.sender_name,
            "chatId": msg.chat_id,
            "messageId": msg.message_id,
        }
    }).to_string());
    let _ = session_db.append_message(&session_id, &user_msg);

    // Auto-generate title from first message (same logic as normal chat)
    if let Ok(Some(meta)) = session_db.get_session(&session_id) {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = crate::session::auto_title(user_text);
            let _ = session_db.update_session_title(&session_id, &title);
        }
    }

    emit_channel_update(&session_id);

    // 5. Send typing indicator
    let _ = plugin.send_typing(&account.id, &msg.chat_id).await;

    // 6. Build channel context for prompt injection
    let chat_type_label = match msg.chat_type {
        ChatType::Dm => "direct message",
        ChatType::Group => "group chat",
        ChatType::Forum => "forum",
        ChatType::Channel => "channel",
    };
    let mut channel_context = format!(
        "## IM Channel Context\n\
         You are responding to a message from an **IM channel** ({channel}), not a direct UI chat.\n\
         - **Channel**: {channel}\n\
         - **Chat type**: {chat_type}\n\
         - **Chat ID**: {chat_id}",
        channel = channel_id_str,
        chat_type = chat_type_label,
        chat_id = msg.chat_id,
    );
    if let Some(ref title) = msg.chat_title {
        channel_context.push_str(&format!("\n- **Chat title**: {}", title));
    }
    if let Some(ref name) = msg.sender_name {
        channel_context.push_str(&format!("\n- **Sender**: {} (ID: {})", name, msg.sender_id));
    } else {
        channel_context.push_str(&format!("\n- **Sender ID**: {}", msg.sender_id));
    }
    channel_context.push_str(
        "\n\nBehave exactly as you would in a normal conversation. \
         The message comes through an IM channel but your capabilities and personality remain the same. \
         Keep responses concise and suitable for IM format."
    );

    // 7. Build ChatEngineParams — load config from disk (no State dependency)
    let agent_def = crate::agent_loader::load_agent(&agent_id).ok();
    let agent_model_config = agent_def.as_ref()
        .map(|d| d.config.model.clone())
        .unwrap_or_default();

    let (primary, fallbacks) = crate::provider::resolve_model_chain(&agent_model_config, &store);
    let mut model_chain = Vec::new();
    if let Some(p) = primary { model_chain.push(p); }
    for fb in fallbacks {
        if !model_chain.iter().any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        anyhow::bail!("No model configured for channel chat");
    }

    // Resolve temperature: agent > global
    let resolved_temperature = {
        let agent_temp = agent_def.as_ref().and_then(|d| d.config.model.temperature);
        let global_temp = store.temperature;
        agent_temp.or(global_temp)
    };

    let web_search_enabled = crate::tools::web_search::has_enabled_provider(&store.web_search);
    let notification_enabled = {
        let agent_notify = agent_def.as_ref().and_then(|d| d.config.notify_on_complete);
        store.notification.enabled && agent_notify != Some(false)
    };
    let image_gen_config = if crate::tools::image_generate::has_configured_provider_from_config(&store.image_generate) {
        let mut cfg = store.image_generate.clone();
        crate::tools::image_generate::backfill_providers(&mut cfg);
        Some(cfg)
    } else {
        None
    };
    let canvas_enabled = store.canvas.enabled;

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        message: user_text.to_string(),
        session_db: session_db.clone(),
        model_chain,
        providers: store.providers.clone(),
        codex_token: None, // Channel doesn't support Codex OAuth
        resolved_temperature,
        web_search_enabled,
        notification_enabled,
        image_gen_config,
        canvas_enabled,
        compact_config: store.compact.clone(),
        extra_system_context: Some(channel_context),
        reasoning_effort: None,
        cancel: Arc::new(AtomicBool::new(false)),
        plan_agent_mode: None,
        plan_mode_allow_paths: None,
        event_sink: Arc::new(crate::chat_engine::EmitSink::new(session_id.clone())),
    };

    // 8. Run shared chat engine (streaming, failover, tool persistence, etc.)
    let result = crate::chat_engine::run_chat_engine(engine_params).await;

    // 9. Process result — send response to IM channel
    match result {
        Ok(engine_result) => {
            let response = &engine_result.response;

            // Convert and send through IM channel
            let native_text = plugin.markdown_to_native(response);
            let chunks = plugin.chunk_message(&native_text);

            for (i, chunk) in chunks.iter().enumerate() {
                let payload = if i == 0 {
                    ReplyPayload {
                        text: Some(chunk.clone()),
                        reply_to_message_id: Some(msg.message_id.clone()),
                        thread_id: msg.thread_id.clone(),
                        parse_mode: Some(ParseMode::Html),
                        ..ReplyPayload::text("")
                    }
                } else {
                    ReplyPayload {
                        text: Some(chunk.clone()),
                        thread_id: msg.thread_id.clone(),
                        parse_mode: Some(ParseMode::Html),
                        ..ReplyPayload::text("")
                    }
                };

                match plugin.send_message(&account.id, &msg.chat_id, &payload).await {
                    Ok(r) => {
                        if !r.success {
                            app_warn!("channel", "worker", "[{}] Send failed: {}",
                                channel_id_str, r.error.unwrap_or_default());
                        }
                    }
                    Err(e) => {
                        app_error!("channel", "worker", "[{}] Send error: {}", channel_id_str, e);
                    }
                }
            }

            app_info!("channel", "worker", "[{}] Reply sent to {} ({} chars)",
                channel_id_str, msg.chat_id, response.len());
        }
        Err(e) => {
            app_error!("channel", "worker", "[{}] Agent error: {}", channel_id_str, e);

            // Send error notification to channel
            let error_text = "⚠️ Sorry, I encountered an error processing your message. Please try again.".to_string();
            let payload = ReplyPayload::text(error_text);
            let _ = plugin.send_message(&account.id, &msg.chat_id, &payload).await;
        }
    }

    // Emit final update so frontend reloads complete message from DB
    emit_channel_update(&session_id);
    Ok(())
}

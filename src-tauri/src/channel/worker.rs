use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;
use tauri::Emitter;

use super::db::ChannelDB;
use super::registry::ChannelRegistry;
use super::traits::ChannelPlugin;
use super::types::*;

/// Notify the frontend that a channel session has new messages.
fn emit_channel_update(session_id: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit("channel:message_update", serde_json::json!({
            "sessionId": session_id,
        }));
    }
}

/// Notify the frontend that a channel session started/stopped streaming.
fn emit_stream_lifecycle(event_name: &str, session_id: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit(event_name, serde_json::json!({
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
        .ok_or_else(|| anyhow::anyhow!("No plugin for channel: {}", msg.channel_id))?
        .clone();

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

    // NOTE: We don't emit channel:message_update here because channel:stream_start
    // will handle frontend state. Emitting here would race with the stream placeholder.

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

    // 8. Create ChannelStreamSink + spawn streaming background task
    let (event_tx, event_rx) = mpsc::unbounded_channel::<String>();

    let supports_draft = plugin.capabilities().supports_draft;
    let max_msg_len = plugin.capabilities().max_message_length.unwrap_or(4096);
    let stream_task = spawn_channel_stream_task(
        event_rx,
        plugin.clone(),
        account.id.clone(),
        msg.chat_id.clone(),
        msg.message_id.clone(),
        msg.thread_id.clone(),
        supports_draft,
        max_msg_len,
    );

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
        event_sink: Arc::new(crate::chat_engine::ChannelStreamSink::new(session_id.clone(), event_tx)),
    };

    // Notify frontend that streaming started (loading indicator)
    emit_stream_lifecycle("channel:stream_start", &session_id);

    // 9. Run shared chat engine (streaming, failover, tool persistence, etc.)
    let result = crate::chat_engine::run_chat_engine(engine_params).await;

    // Drop the sink's sender is implicit — engine_params is consumed.
    // Wait for the streaming background task to finish.
    let _ = stream_task.await;

    // 10. Process result — send final formatted response via sendMessage
    match result {
        Ok(engine_result) => {
            let response = &engine_result.response;
            send_final_reply(&plugin, &account.id, &msg, response).await;

            app_info!("channel", "worker", "[{}] Reply sent to {} ({} chars)",
                channel_id_str, msg.chat_id, response.len());
        }
        Err(e) => {
            app_error!("channel", "worker", "[{}] Agent error: {}", channel_id_str, e);

            let error_text = "⚠️ Sorry, I encountered an error processing your message. Please try again.";
            let payload = ReplyPayload {
                text: Some(error_text.to_string()),
                reply_to_message_id: Some(msg.message_id.clone()),
                thread_id: msg.thread_id.clone(),
                ..ReplyPayload::text("")
            };
            let _ = plugin.send_message(&account.id, &msg.chat_id, &payload).await;
        }
    }

    // Notify frontend that streaming ended (triggers DB reload in frontend)
    emit_stream_lifecycle("channel:stream_end", &session_id);

    Ok(())
}

/// Send the final formatted response to the IM channel.
///
/// Converts markdown to native format, chunks if needed, and sends via `send_message`.
/// This is always the last step — drafts are just previews, `sendMessage` commits.
async fn send_final_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    response: &str,
) {
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

        match plugin.send_message(account_id, &msg.chat_id, &payload).await {
            Ok(r) => {
                if !r.success {
                    app_warn!("channel", "worker", "Send failed: {}",
                        r.error.unwrap_or_default());
                }
            }
            Err(e) => {
                app_error!("channel", "worker", "Send error: {}", e);
            }
        }
    }
}

// ── Channel Streaming Background Task ──────────────────────────────

/// Spawn a background task that receives streaming events from the chat engine
/// and sends progressive drafts to the IM channel via `send_draft` (e.g. Telegram's
/// `sendMessageDraft` — Bot API 9.3+).
///
/// Draft-based streaming flow:
/// 1. Accumulate text_delta events from the chat engine
/// 2. Periodically call `send_draft` with accumulated text (no rate limiting, no flicker)
/// 3. When engine finishes, the caller sends the final `send_message` to commit
///
/// For channels without `send_draft` support, events are simply drained (frontend
/// still receives streaming via `channel:stream_delta` Tauri events).
fn spawn_channel_stream_task(
    mut event_rx: mpsc::UnboundedReceiver<String>,
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    reply_to_message_id: String,
    thread_id: Option<String>,
    supports_draft: bool,
    max_msg_len: usize,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // If the channel doesn't support drafts, just drain events and return.
        // Frontend still receives streaming via channel:stream_delta events.
        if !supports_draft {
            while event_rx.recv().await.is_some() {}
            return;
        }

        // Generate a stable draft_id for this streaming session.
        // Must be non-zero. Telegram animates changes to drafts with the same ID.
        let draft_id: i64 = reply_to_message_id.parse::<i64>().unwrap_or_else(|_| {
            // Fallback: use current timestamp as a unique non-zero ID
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(1)
        });
        // Ensure non-zero
        let draft_id = if draft_id == 0 { 1 } else { draft_id };

        let mut accumulated = String::new();
        let mut dirty = false;
        // sendMessageDraft has no rate limiting, but we still batch to avoid
        // excessive network calls. 300ms gives smooth streaming.
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
        // Don't fire immediately
        interval.tick().await;

        loop {
            tokio::select! {
                biased;

                event = event_rx.recv() => {
                    match event {
                        Some(event_str) => {
                            if let Some(text) = extract_text_delta(&event_str) {
                                accumulated.push_str(&text);
                                dirty = true;
                            }
                        }
                        None => {
                            // Channel closed — engine finished. Send one last draft if dirty.
                            if dirty && !accumulated.is_empty() && accumulated.len() <= max_msg_len {
                                send_draft(
                                    &plugin, &account_id, &chat_id,
                                    &reply_to_message_id, thread_id.as_deref(),
                                    &accumulated, draft_id,
                                ).await;
                            }
                            break;
                        }
                    }
                }

                _ = interval.tick() => {
                    if dirty && !accumulated.is_empty() && accumulated.len() <= max_msg_len {
                        send_draft(
                            &plugin, &account_id, &chat_id,
                            &reply_to_message_id, thread_id.as_deref(),
                            &accumulated, draft_id,
                        ).await;
                        dirty = false;
                    }
                }
            }
        }
    })
}

/// Extract text from a `text_delta` event JSON string.
fn extract_text_delta(event_str: &str) -> Option<String> {
    let event: serde_json::Value = serde_json::from_str(event_str).ok()?;
    if event.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    event.get("text")?.as_str().map(|s| s.to_string())
}

/// Send a draft to the IM channel for progressive streaming display.
async fn send_draft(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: &str,
    thread_id: Option<&str>,
    text: &str,
    draft_id: i64,
) {
    let payload = ReplyPayload {
        text: Some(text.to_string()),
        reply_to_message_id: Some(reply_to_message_id.to_string()),
        thread_id: thread_id.map(|s| s.to_string()),
        draft_id: Some(draft_id),
        ..ReplyPayload::text("")
    };
    if let Err(e) = plugin.send_draft(account_id, chat_id, &payload).await {
        app_warn!("channel", "worker", "send_draft failed: {}", e);
    }
}

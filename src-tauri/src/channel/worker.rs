use std::sync::Arc;
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

/// Emit a streaming text delta to the frontend for a channel session.
fn emit_channel_stream_delta(session_id: &str, delta: &str, accumulated: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit("channel:stream_delta", serde_json::json!({
            "sessionId": session_id,
            "delta": delta,
            "accumulated": accumulated,
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

    // 7. Build streaming callback for front-end real-time updates
    let accumulated_text = Arc::new(std::sync::Mutex::new(String::new()));
    let accumulated_clone = accumulated_text.clone();
    let session_id_for_cb = session_id.clone();

    let on_delta: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |delta: &str| {
        // Parse the delta event to extract text
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
            if event.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                if let Some(text) = event.get("text").and_then(|t| t.as_str()) {
                    let mut acc = accumulated_clone.lock().unwrap_or_else(|e| e.into_inner());
                    acc.push_str(text);
                    let current = acc.clone();
                    drop(acc);
                    // Emit to frontend for real-time display
                    emit_channel_stream_delta(&session_id_for_cb, text, &current);
                }
            }
        }
    });

    let result = crate::cron::executor::build_and_run_agent_streaming(
        &agent_id,
        user_text,
        &session_id,
        session_db,
        Some(&channel_context),
        Some(on_delta),
    ).await;

    // 8. Process result
    match result {
        Ok(response) => {
            // Save assistant response to session
            let _ = session_db.append_message(
                &session_id,
                &crate::session::NewMessage::assistant(&response),
            );

            // Send final response through IM channel
            let native_text = plugin.markdown_to_native(&response);
            let chunks = plugin.chunk_message(&native_text);

            // Send first chunk as a new message, then edit it if channel supports edit
            let mut first_message_id: Option<String> = None;
            for (i, chunk) in chunks.iter().enumerate() {
                if i == 0 {
                    // First chunk: send as reply
                    let payload = ReplyPayload {
                        text: Some(chunk.clone()),
                        reply_to_message_id: Some(msg.message_id.clone()),
                        thread_id: msg.thread_id.clone(),
                        parse_mode: Some(ParseMode::Html),
                        ..ReplyPayload::text("")
                    };
                    match plugin.send_message(&account.id, &msg.chat_id, &payload).await {
                        Ok(result) => {
                            if result.success {
                                first_message_id = result.message_id;
                            } else {
                                app_warn!("channel", "worker", "[{}] Send failed: {}",
                                    channel_id_str, result.error.unwrap_or_default());
                            }
                        }
                        Err(e) => {
                            app_error!("channel", "worker", "[{}] Send error: {}", channel_id_str, e);
                        }
                    }
                } else {
                    // Subsequent chunks: send as separate messages
                    let payload = ReplyPayload {
                        text: Some(chunk.clone()),
                        thread_id: msg.thread_id.clone(),
                        parse_mode: Some(ParseMode::Html),
                        ..ReplyPayload::text("")
                    };
                    if let Err(e) = plugin.send_message(&account.id, &msg.chat_id, &payload).await {
                        app_error!("channel", "worker", "[{}] Send error: {}", channel_id_str, e);
                    }
                }
            }

            app_info!("channel", "worker", "[{}] Reply sent to {} ({} chars)",
                channel_id_str, msg.chat_id, response.len());

            // Store the first_message_id so it could be used for streaming edits
            let _ = first_message_id;
        }
        Err(e) => {
            app_error!("channel", "worker", "[{}] Agent error: {}", channel_id_str, e);

            // Save error to session
            let mut err_msg = crate::session::NewMessage::assistant(&format!("Error: {}", e));
            err_msg.is_error = Some(true);
            let _ = session_db.append_message(&session_id, &err_msg);

            // Send error notification to channel
            let error_text = format!("⚠️ Sorry, I encountered an error processing your message. Please try again.");
            let payload = ReplyPayload::text(error_text);
            let _ = plugin.send_message(&account.id, &msg.chat_id, &payload).await;
        }
    }

    // Emit final update so frontend reloads complete message
    emit_channel_update(&session_id);
    Ok(())
}

use std::sync::Arc;
use tokio::sync::mpsc;

use super::db::ChannelDB;
use super::registry::ChannelRegistry;
use super::types::*;

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

    // 3. Resolve or create session
    let agent_id = store.channels.agent_id().to_string();
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

    // 5. Send typing indicator
    let _ = plugin.send_typing(&account.id, &msg.chat_id).await;

    // 6. Build agent and run chat (reuses cron executor pattern)
    let result = crate::cron::executor::build_and_run_agent(
        &agent_id,
        user_text,
        &session_id,
        session_db,
    ).await;

    // 7. Process result
    match result {
        Ok(response) => {
            // Save assistant response to session
            let _ = session_db.append_message(
                &session_id,
                &crate::session::NewMessage::assistant(&response),
            );

            // Convert and send through channel
            let native_text = plugin.markdown_to_native(&response);
            let chunks = plugin.chunk_message(&native_text);

            for chunk in chunks {
                let payload = ReplyPayload {
                    text: Some(chunk),
                    reply_to_message_id: Some(msg.message_id.clone()),
                    thread_id: msg.thread_id.clone(),
                    parse_mode: Some(ParseMode::Html),
                    ..ReplyPayload::text("")
                };

                match plugin.send_message(&account.id, &msg.chat_id, &payload).await {
                    Ok(result) => {
                        if !result.success {
                            app_warn!("channel", "worker", "[{}] Send failed: {}",
                                channel_id_str, result.error.unwrap_or_default());
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

    Ok(())
}

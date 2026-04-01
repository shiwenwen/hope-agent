use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::mpsc;

use super::db::ChannelDB;
use super::registry::ChannelRegistry;
use super::traits::ChannelPlugin;
use super::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPreviewTransport {
    Draft,
    Message,
}

#[derive(Debug, Default)]
struct StreamPreviewOutcome {
    preview_message_id: Option<String>,
}

/// Notify the frontend that a channel session has new messages.
fn emit_channel_update(session_id: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit(
            "channel:message_update",
            serde_json::json!({
                "sessionId": session_id,
            }),
        );
    }
}

/// Notify the frontend that a channel session started/stopped streaming.
fn emit_stream_lifecycle(event_name: &str, session_id: &str) {
    if let Some(handle) = crate::get_app_handle() {
        let _ = handle.emit(
            event_name,
            serde_json::json!({
                "sessionId": session_id,
            }),
        );
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
                    app_error!(
                        "channel",
                        "worker",
                        "Failed to handle inbound message: {}",
                        e
                    );
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
    let sender_label = msg
        .sender_name
        .as_deref()
        .or(msg.sender_username.as_deref())
        .unwrap_or(&msg.sender_id);
    app_info!(
        "channel",
        "worker",
        "[{}] Message from {} in {}: {}",
        channel_id_str,
        sender_label,
        msg.chat_id,
        crate::truncate_utf8(msg.text.as_deref().unwrap_or("(media)"), 100)
    );

    // 1. Load config and find account
    let store = crate::provider::load_store().unwrap_or_default();
    let account = store
        .channels
        .find_account(&msg.account_id)
        .ok_or_else(|| anyhow::anyhow!("Account '{}' not found in config", msg.account_id))?
        .clone();

    // 2. Check access control
    let plugin = registry
        .get_plugin(&msg.channel_id)
        .ok_or_else(|| anyhow::anyhow!("No plugin for channel: {}", msg.channel_id))?
        .clone();

    if !plugin.check_access(&account, &msg) {
        app_warn!(
            "channel",
            "worker",
            "[{}] Access denied for sender {} in {}",
            channel_id_str,
            msg.sender_id,
            msg.chat_id
        );
        return Ok(());
    }

    // 2b. Resolve group/topic/channel config for mention gating & agent routing
    let security = &account.security;
    let group_config = security.groups.get(&msg.chat_id);
    let wildcard_config = security.groups.get("*");
    let effective_group_config = group_config.or(wildcard_config);
    let topic_config = effective_group_config
        .and_then(|g| msg.thread_id.as_ref().and_then(|tid| g.topics.get(tid)));
    let channel_config = security.channels.get(&msg.chat_id);

    // 2c. Mention gating (for groups/forums/channels)
    if matches!(msg.chat_type, ChatType::Group | ChatType::Forum) {
        let require_mention = topic_config
            .and_then(|t| t.require_mention)
            .or_else(|| effective_group_config.and_then(|g| g.require_mention))
            .unwrap_or(true); // default: require mention

        if require_mention && !msg.was_mentioned {
            app_debug!(
                "channel",
                "worker",
                "[{}] Skipping non-mentioned message in {} (requireMention=true)",
                channel_id_str,
                msg.chat_id
            );
            return Ok(());
        }
    } else if matches!(msg.chat_type, ChatType::Channel) {
        let require_mention = channel_config
            .and_then(|c| c.require_mention)
            .unwrap_or(true);

        if require_mention && !msg.was_mentioned {
            app_debug!(
                "channel",
                "worker",
                "[{}] Skipping non-mentioned channel message in {} (requireMention=true)",
                channel_id_str,
                msg.chat_id
            );
            return Ok(());
        }
    }

    // 3. Resolve agent_id: topic > group > channel > per-account > global default
    let base_agent_id = account
        .agent_id
        .as_deref()
        .unwrap_or_else(|| store.channels.agent_id());

    let resolved_agent_id = match msg.chat_type {
        ChatType::Group | ChatType::Forum => {
            topic_config
                .and_then(|t| t.agent_id.as_deref())
                .or_else(|| effective_group_config.and_then(|g| g.agent_id.as_deref()))
                .unwrap_or(base_agent_id)
        }
        ChatType::Channel => {
            channel_config
                .and_then(|c| c.agent_id.as_deref())
                .unwrap_or(base_agent_id)
        }
        ChatType::Dm => base_agent_id,
    };
    let agent_id = resolved_agent_id.to_string();

    // 3b. Resolve extra system prompt from group/topic/channel config
    let config_system_prompt = match msg.chat_type {
        ChatType::Group | ChatType::Forum => {
            topic_config
                .and_then(|t| t.system_prompt.as_deref())
                .or_else(|| effective_group_config.and_then(|g| g.system_prompt.as_deref()))
        }
        ChatType::Channel => channel_config.and_then(|c| c.system_prompt.as_deref()),
        ChatType::Dm => None,
    };

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
    let session_db =
        crate::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))?;

    let user_text = msg.text.as_deref().unwrap_or("(media message)");
    let mut user_msg = crate::session::NewMessage::user(user_text);
    user_msg.attachments_meta = Some(
        serde_json::json!({
            "channel_inbound": {
                "channelId": channel_id_str,
                "accountId": msg.account_id,
                "senderId": msg.sender_id,
                "senderName": msg.sender_name,
                "chatId": msg.chat_id,
                "messageId": msg.message_id,
            }
        })
        .to_string(),
    );
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

    // 5a. Intercept slash commands — dispatch and send reply directly, skip LLM.
    // For PassThrough commands (e.g. skill invocations), use the transformed message as the
    // engine input so the LLM receives the skill instruction rather than the raw "/" text.
    let engine_message: String;
    if crate::slash_commands::parser::is_command(user_text) {
        match dispatch_slash_for_channel(
            channel_db,
            &channel_id_str,
            &msg.account_id,
            &msg.chat_id,
            msg.thread_id.as_deref(),
            &session_id,
            &agent_id,
            user_text,
        )
        .await
        {
            Ok(ChannelSlashOutcome::Reply {
                content,
                new_session_id,
                buttons,
            }) => {
                let effective_sid = new_session_id.as_deref().unwrap_or(&session_id);
                // Only persist reply to the OLD session; skip for new sessions
                // (e.g. /new) so auto_title can work on the first real message.
                if new_session_id.is_none() {
                    let _ = session_db.append_message(
                        effective_sid,
                        &crate::session::NewMessage::event(&content),
                    );
                }
                // Send reply to the IM channel
                let native_text = plugin.markdown_to_native(&content);
                let payload = ReplyPayload {
                    text: Some(native_text),
                    reply_to_message_id: Some(msg.message_id.clone()),
                    thread_id: msg.thread_id.clone(),
                    parse_mode: Some(ParseMode::Html),
                    buttons,
                    ..ReplyPayload::text("")
                };
                let _ = plugin.send_message(&account.id, &msg.chat_id, &payload).await;
                emit_channel_update(effective_sid);
                emit_stream_lifecycle("channel:stream_end", effective_sid);
                return Ok(());
            }
            Ok(ChannelSlashOutcome::PassThrough(message)) => {
                // Fall through to LLM with the transformed message
                engine_message = message;
            }
            Err(e) => {
                let error_reply = format!("⚠️ {}", e);
                let native_text = plugin.markdown_to_native(&error_reply);
                let payload = ReplyPayload {
                    text: Some(native_text),
                    reply_to_message_id: Some(msg.message_id.clone()),
                    thread_id: msg.thread_id.clone(),
                    parse_mode: Some(ParseMode::Html),
                    ..ReplyPayload::text("")
                };
                let _ = plugin.send_message(&account.id, &msg.chat_id, &payload).await;
                emit_stream_lifecycle("channel:stream_end", &session_id);
                return Ok(());
            }
        }
    } else {
        engine_message = user_text.to_string();
    }

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
    // Inject per-group/topic/channel system prompt if configured
    if let Some(prompt) = config_system_prompt {
        channel_context.push_str(&format!("\n\n## Additional Context\n{}", prompt));
    }

    // 7. Build ChatEngineParams — load config from disk (no State dependency)
    let agent_def = crate::agent_loader::load_agent(&agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|d| d.config.model.clone())
        .unwrap_or_default();

    let (primary, fallbacks) = crate::provider::resolve_model_chain(&agent_model_config, &store);
    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain
            .iter()
            .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
        {
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
    let image_gen_config =
        if crate::tools::image_generate::has_configured_provider_from_config(&store.image_generate)
        {
            let mut cfg = store.image_generate.clone();
            crate::tools::image_generate::backfill_providers(&mut cfg);
            Some(cfg)
        } else {
            None
        };
    let canvas_enabled = store.canvas.enabled;

    // 8. Create ChannelStreamSink + spawn streaming background task
    let (event_tx, event_rx) = mpsc::unbounded_channel::<String>();

    let capabilities = plugin.capabilities();
    let preview_transport = select_stream_preview_transport(&msg.chat_type, &capabilities);
    let max_msg_len = capabilities.max_message_length.unwrap_or(4096);
    let stream_task = spawn_channel_stream_task(
        event_rx,
        plugin.clone(),
        account.id.clone(),
        msg.chat_id.clone(),
        msg.message_id.clone(),
        msg.thread_id.clone(),
        preview_transport,
        max_msg_len,
    );

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        message: engine_message,
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
        reasoning_effort: {
            let app_handle = crate::get_app_handle();
            if let Some(ref h) = app_handle {
                let st = h.state::<crate::AppState>();
                let eff = st.reasoning_effort.lock().await.clone();
                if eff == "none" { None } else { Some(eff) }
            } else {
                None
            }
        },
        cancel: {
            let app_handle = crate::get_app_handle();
            if let Some(ref h) = app_handle {
                let st = h.state::<crate::AppState>();
                st.channel_cancels.register(&session_id)
            } else {
                Arc::new(AtomicBool::new(false))
            }
        },
        plan_agent_mode: None,
        plan_mode_allow_paths: None,
        event_sink: Arc::new(crate::chat_engine::ChannelStreamSink::new(
            session_id.clone(),
            event_tx,
        )),
    };

    // Notify frontend that streaming started (loading indicator)
    emit_stream_lifecycle("channel:stream_start", &session_id);

    // 9. Run shared chat engine (streaming, failover, tool persistence, etc.)
    let result = crate::chat_engine::run_chat_engine(engine_params).await;

    // Remove cancel handle now that engine is done
    if let Some(ref h) = crate::get_app_handle() {
        let st = h.state::<crate::AppState>();
        st.channel_cancels.remove(&session_id);
    }

    // Drop the sink's sender is implicit — engine_params is consumed.
    // Wait for the streaming background task to finish.
    let stream_outcome = match stream_task.await {
        Ok(outcome) => outcome,
        Err(e) => {
            app_warn!("channel", "worker", "Streaming preview task failed: {}", e);
            StreamPreviewOutcome::default()
        }
    };

    // 10. Process result — send final formatted response via sendMessage
    match result {
        Ok(engine_result) => {
            let response = &engine_result.response;
            send_final_reply(
                &plugin,
                &account.id,
                &msg,
                response,
                stream_outcome.preview_message_id.as_deref(),
            )
            .await;

            app_info!(
                "channel",
                "worker",
                "[{}] Reply sent to {} ({} chars)",
                channel_id_str,
                msg.chat_id,
                response.len()
            );
        }
        Err(e) => {
            app_error!(
                "channel",
                "worker",
                "[{}] Agent error: {}",
                channel_id_str,
                e
            );

            let error_text =
                "⚠️ Sorry, I encountered an error processing your message. Please try again.";
            let payload = ReplyPayload {
                text: Some(error_text.to_string()),
                reply_to_message_id: Some(msg.message_id.clone()),
                thread_id: msg.thread_id.clone(),
                ..ReplyPayload::text("")
            };
            if let Some(preview_message_id) = stream_outcome.preview_message_id.as_deref() {
                if let Err(edit_err) = plugin
                    .edit_message(&account.id, &msg.chat_id, preview_message_id, &payload)
                    .await
                {
                    app_warn!(
                        "channel",
                        "worker",
                        "Failed to replace preview with error reply: {}",
                        edit_err
                    );
                    let _ = plugin
                        .send_message(&account.id, &msg.chat_id, &payload)
                        .await;
                }
            } else {
                let _ = plugin
                    .send_message(&account.id, &msg.chat_id, &payload)
                    .await;
            }
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
    preview_message_id: Option<&str>,
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

        let delivery = if i == 0 {
            if let Some(message_id) = preview_message_id {
                match plugin
                    .edit_message(account_id, &msg.chat_id, message_id, &payload)
                    .await
                {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "worker",
                            "Failed to finalize preview via edit, falling back to send: {}",
                            e
                        );
                        plugin
                            .send_message(account_id, &msg.chat_id, &payload)
                            .await
                    }
                }
            } else {
                plugin
                    .send_message(account_id, &msg.chat_id, &payload)
                    .await
            }
        } else {
            plugin
                .send_message(account_id, &msg.chat_id, &payload)
                .await
        };

        match delivery {
            Ok(r) => {
                if !r.success {
                    app_warn!(
                        "channel",
                        "worker",
                        "Send failed: {}",
                        r.error.unwrap_or_default()
                    );
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
/// and sends progressive previews to the IM channel.
///
/// Preview flow:
/// 1. Accumulate text_delta events from the chat engine
/// 2. Periodically send the accumulated snapshot via either:
///    - `send_draft` for Telegram private chats, or
///    - `send_message` + `edit_message` for channels that only support message edits
/// 3. When engine finishes, the caller commits the final response
///
/// For channels without any preview transport, events are simply drained while the
/// frontend still receives `channel:stream_delta` events.
fn spawn_channel_stream_task(
    mut event_rx: mpsc::UnboundedReceiver<String>,
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    reply_to_message_id: String,
    thread_id: Option<String>,
    preview_transport: Option<StreamPreviewTransport>,
    max_msg_len: usize,
) -> tokio::task::JoinHandle<StreamPreviewOutcome> {
    tokio::spawn(async move {
        let Some(mut preview_transport) = preview_transport else {
            while event_rx.recv().await.is_some() {}
            return StreamPreviewOutcome::default();
        };

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
        let mut preview_message_id: Option<String> = None;
        let mut dirty = false;
        // 1s aligns better with Telegram edit limits and OpenClaw's draft preview cadence.
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1000));
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
                            if dirty && !accumulated.is_empty() {
                                send_stream_preview(
                                    &plugin, &account_id, &chat_id,
                                    &reply_to_message_id, thread_id.as_deref(), max_msg_len,
                                    &accumulated, draft_id, &mut preview_transport, &mut preview_message_id,
                                ).await;
                            }
                            break;
                        }
                    }
                }

                _ = interval.tick() => {
                    if dirty && !accumulated.is_empty() {
                        send_stream_preview(
                            &plugin, &account_id, &chat_id,
                            &reply_to_message_id, thread_id.as_deref(), max_msg_len,
                            &accumulated, draft_id, &mut preview_transport, &mut preview_message_id,
                        ).await;
                        dirty = false;
                    }
                }
            }
        }

        StreamPreviewOutcome { preview_message_id }
    })
}

/// Extract text from a `text_delta` event JSON string.
fn extract_text_delta(event_str: &str) -> Option<String> {
    let event: serde_json::Value = serde_json::from_str(event_str).ok()?;
    if event.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    event
        .get("content")
        .or_else(|| event.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn select_stream_preview_transport(
    chat_type: &ChatType,
    capabilities: &ChannelCapabilities,
) -> Option<StreamPreviewTransport> {
    if matches!(chat_type, ChatType::Dm) && capabilities.supports_draft {
        return Some(StreamPreviewTransport::Draft);
    }
    if capabilities.supports_edit {
        return Some(StreamPreviewTransport::Message);
    }
    None
}

fn should_fallback_from_draft_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("sendmessagedraft")
        && (lower.contains("unknown method")
            || lower.contains("not found")
            || lower.contains("not available")
            || lower.contains("not supported")
            || lower.contains("unsupported")
            || lower.contains("private chat")
            || lower.contains("can be used only"))
}

fn build_stream_preview_payload(
    plugin: &Arc<dyn ChannelPlugin>,
    reply_to_message_id: &str,
    thread_id: Option<&str>,
    text: &str,
    draft_id: i64,
    max_msg_len: usize,
) -> Option<ReplyPayload> {
    let native_text = plugin.markdown_to_native(text);
    let text = native_text.trim_end();
    if text.is_empty() || text.len() > max_msg_len {
        return None;
    }

    Some(ReplyPayload {
        text: Some(text.to_string()),
        reply_to_message_id: Some(reply_to_message_id.to_string()),
        thread_id: thread_id.map(|s| s.to_string()),
        parse_mode: Some(ParseMode::Html),
        draft_id: Some(draft_id),
        ..ReplyPayload::text("")
    })
}

async fn send_message_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    payload: &ReplyPayload,
    preview_message_id: &mut Option<String>,
) {
    if let Some(message_id) = preview_message_id.as_deref() {
        if let Err(e) = plugin
            .edit_message(account_id, chat_id, message_id, payload)
            .await
        {
            app_warn!("channel", "worker", "stream preview edit failed: {}", e);
        }
        return;
    }

    match plugin.send_message(account_id, chat_id, payload).await {
        Ok(result) => {
            if result.success {
                *preview_message_id = result.message_id;
            } else {
                app_warn!(
                    "channel",
                    "worker",
                    "stream preview send failed: {}",
                    result.error.unwrap_or_default()
                );
            }
        }
        Err(e) => {
            app_warn!("channel", "worker", "stream preview send failed: {}", e);
        }
    }
}

async fn send_stream_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: &str,
    thread_id: Option<&str>,
    max_msg_len: usize,
    text: &str,
    draft_id: i64,
    preview_transport: &mut StreamPreviewTransport,
    preview_message_id: &mut Option<String>,
) {
    let Some(payload) = build_stream_preview_payload(
        plugin,
        reply_to_message_id,
        thread_id,
        text,
        draft_id,
        max_msg_len,
    ) else {
        return;
    };

    match preview_transport {
        StreamPreviewTransport::Draft => {
            if let Err(e) = plugin.send_draft(account_id, chat_id, &payload).await {
                if should_fallback_from_draft_error(&e.to_string()) {
                    app_warn!(
                        "channel",
                        "worker",
                        "send_draft unavailable, falling back to send/edit preview: {}",
                        e
                    );
                    *preview_transport = StreamPreviewTransport::Message;
                    send_message_preview(plugin, account_id, chat_id, &payload, preview_message_id)
                        .await;
                } else {
                    app_warn!("channel", "worker", "send_draft failed: {}", e);
                }
            }
        }
        StreamPreviewTransport::Message => {
            send_message_preview(plugin, account_id, chat_id, &payload, preview_message_id).await;
        }
    }
}

// ── Slash Command Dispatch for IM Channels ─────────────────────────

/// Outcome of dispatching a slash command from an IM channel message.
enum ChannelSlashOutcome {
    /// Send `content` as a direct reply; no LLM call needed.
    /// `new_session_id` is set when the command created a fresh session that should
    /// replace the current channel → session mapping.
    /// `buttons` provides optional inline keyboard buttons for IM channels that support them.
    Reply {
        content: String,
        new_session_id: Option<String>,
        buttons: Vec<Vec<crate::channel::types::InlineButton>>,
    },
    /// The command (e.g. a skill invocation) asks to pass a transformed message
    /// through to the LLM instead of the original "/" text.
    PassThrough(String),
}

/// Dispatch a slash command received via an IM channel.
///
/// Returns a `ChannelSlashOutcome` describing what to do next:
///   - `Reply`       → send the content as a direct reply and skip the LLM.
///   - `PassThrough` → forward the (possibly rewritten) message to the LLM.
async fn dispatch_slash_for_channel(
    channel_db: &ChannelDB,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    session_id: &str,
    agent_id: &str,
    text: &str,
) -> Result<ChannelSlashOutcome, anyhow::Error> {
    use crate::slash_commands::{handlers, parser};

    let (name, args) = parser::parse(text).map_err(|e| anyhow::anyhow!(e))?;

    // For commands with fixed arg_options and no args provided, return inline buttons
    // so IM channel users (e.g. Telegram) can tap to select an option.
    if args.trim().is_empty() {
        use crate::slash_commands::registry;
        let commands = registry::all_commands();
        if let Some(cmd) = commands.iter().find(|c| c.name == name) {
            if let Some(ref options) = cmd.arg_options {
                let buttons: Vec<Vec<crate::channel::types::InlineButton>> = options
                    .iter()
                    .map(|opt| {
                        vec![crate::channel::types::InlineButton {
                            text: opt.clone(),
                            callback_data: Some(format!("slash:{} {}", name, opt)),
                            url: None,
                        }]
                    })
                    .collect();
                return Ok(ChannelSlashOutcome::Reply {
                    content: format!("Select an option for /{}:", name),
                    new_session_id: None,
                    buttons,
                });
            }
        }
    }

    // Obtain a reference to the global AppState so we can reuse the shared handlers.
    let app_handle = crate::get_app_handle()
        .ok_or_else(|| anyhow::anyhow!("App handle not initialized"))?;
    let state = app_handle.state::<crate::AppState>();
    let app_state: &crate::AppState = &state;

    let result = handlers::dispatch(app_state, Some(session_id), agent_id, &name, &args)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    use crate::slash_commands::types::CommandAction;
    match result.action {
        // Pass transformed message to the LLM (skill commands, /search, etc.)
        Some(CommandAction::PassThrough { message }) => {
            Ok(ChannelSlashOutcome::PassThrough(message))
        }

        // A new session was created — remap the channel conversation to it.
        Some(CommandAction::NewSession { session_id: new_sid }) => {
            if let Err(e) = channel_db.update_session(
                channel_id,
                account_id,
                chat_id,
                thread_id,
                &new_sid,
            ) {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to remap channel session after /new: {}",
                    e
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: Some(new_sid),
                buttons: vec![],
            })
        }

        // Agent switch also creates a new session.
        Some(CommandAction::SwitchAgent {
            session_id: new_sid,
            ..
        }) => {
            if let Err(e) = channel_db.update_session(
                channel_id,
                account_id,
                chat_id,
                thread_id,
                &new_sid,
            ) {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to remap channel session after /agent: {}",
                    e
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: Some(new_sid),
                buttons: vec![],
            })
        }

        // ViewSystemPrompt — build and return the system prompt text directly.
        Some(CommandAction::ViewSystemPrompt) => {
            let (model, provider) = {
                let store = app_state.provider_store.lock().await;
                if let Some(ref active) = store.active_model {
                    let prov = store
                        .providers
                        .iter()
                        .find(|p| p.id == active.provider_id);
                    let model_id = active.model_id.clone();
                    let provider_name = prov
                        .map(|p| p.api_type.display_name().to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    (model_id, provider_name)
                } else {
                    ("unknown".to_string(), "Unknown".to_string())
                }
            };
            let prompt =
                crate::agent::build_system_prompt(agent_id, &model, &provider);
            Ok(ChannelSlashOutcome::Reply {
                content: format!("**System Prompt**\n\n```\n{}\n```", prompt),
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Model switch — persist + notify frontend ──
        Some(CommandAction::SwitchModel {
            provider_id,
            model_id,
        }) => {
            if let Err(e) = crate::commands::provider::set_active_model_core(
                &provider_id,
                &model_id,
                app_state,
            )
            .await
            {
                app_warn!("channel", "worker", "Failed to switch model: {}", e);
            } else if let Some(handle) = crate::get_app_handle() {
                let _ = handle.emit(
                    "slash:model_switched",
                    serde_json::json!({
                        "providerId": provider_id,
                        "modelId": model_id,
                    }),
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Reasoning effort — persist + notify frontend ──
        Some(CommandAction::SetEffort { effort }) => {
            if let Err(e) =
                crate::commands::auth::set_reasoning_effort_core(&effort, app_state).await
            {
                app_warn!("channel", "worker", "Failed to set effort: {}", e);
            } else if let Some(handle) = crate::get_app_handle() {
                let _ = handle.emit("slash:effort_changed", &effort);
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Stop stream — cancel via registry ──
        Some(CommandAction::StopStream) => {
            let cancelled = app_state.channel_cancels.cancel(session_id);
            let msg = if cancelled {
                "Stopping current stream...".to_string()
            } else {
                "No active stream to stop.".to_string()
            };
            Ok(ChannelSlashOutcome::Reply {
                content: msg,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Compact — run compaction ──
        Some(CommandAction::Compact) => {
            match crate::commands::config::compact_context_now_core(session_id, app_state).await {
                Ok(r) => {
                    let msg = format!(
                        "Compacted: {} → {} tokens ({} messages affected)",
                        r.tokens_before, r.tokens_after, r.messages_affected
                    );
                    Ok(ChannelSlashOutcome::Reply {
                        content: msg,
                        new_session_id: None,
                        buttons: vec![],
                    })
                }
                Err(e) => Ok(ChannelSlashOutcome::Reply {
                    content: format!("Compaction failed: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                }),
            }
        }

        // ── Session cleared — notify frontend ──
        Some(CommandAction::SessionCleared) => {
            if let Some(handle) = crate::get_app_handle() {
                let _ = handle.emit("slash:session_cleared", session_id);
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Export — write to file ──
        Some(CommandAction::ExportFile { content, filename }) => {
            let msg = match crate::paths::root_dir() {
                Ok(root) => {
                    let export_dir = root.join("exports");
                    let _ = std::fs::create_dir_all(&export_dir);
                    let path = export_dir.join(&filename);
                    match std::fs::write(&path, &content) {
                        Ok(_) => format!("Exported to `{}`", path.display()),
                        Err(e) => format!("Export failed: {}", e),
                    }
                }
                Err(e) => format!("Export failed: {}", e),
            };
            Ok(ChannelSlashOutcome::Reply {
                content: msg,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Tool permission — not applicable in channel context ──
        Some(CommandAction::SetToolPermission { mode }) => Ok(ChannelSlashOutcome::Reply {
            content: format!(
                "Tool permission `{}` is not applicable in channel context (auto-approve).",
                mode
            ),
            new_session_id: None,
            buttons: vec![],
        }),

        // ── Plan: show plan content ──
        Some(CommandAction::ShowPlan { plan_content }) => {
            if let Some(handle) = crate::get_app_handle() {
                let _ = handle.emit("slash:plan_changed", session_id);
            }
            Ok(ChannelSlashOutcome::Reply {
                content: format!("**Current Plan**\n\n{}", plan_content),
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Plan: state transitions (DB already persisted by handler) ──
        Some(CommandAction::EnterPlanMode)
        | Some(CommandAction::ExitPlanMode { .. })
        | Some(CommandAction::ApprovePlan { .. })
        | Some(CommandAction::PausePlan)
        | Some(CommandAction::ResumePlan) => {
            if let Some(handle) = crate::get_app_handle() {
                let _ = handle.emit("slash:plan_changed", session_id);
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── DisplayOnly: for /model with no args, attach model list as inline buttons ──
        Some(CommandAction::DisplayOnly) if name == "model" && args.trim().is_empty() => {
            let model_buttons = build_model_buttons(app_state).await;
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: model_buttons,
            })
        }

        // ── DisplayOnly and any unhandled actions — just return text ──
        _ => Ok(ChannelSlashOutcome::Reply {
            content: result.content,
            new_session_id: None,
            buttons: vec![],
        }),
    }
}

/// Build inline keyboard buttons for the model list.
/// Each available model gets a button with callback_data `slash:model <model_name>`.
/// Telegram limits inline keyboard callback_data to 64 bytes, so we use model_name
/// (the display name the fuzzy matcher accepts) rather than model_id.
async fn build_model_buttons(
    app_state: &crate::AppState,
) -> Vec<Vec<crate::channel::types::InlineButton>> {
    let store = app_state.provider_store.lock().await;
    let models = crate::provider::build_available_models(&store.providers);

    // Group up to 2 models per row, max 20 buttons (Telegram limit is 100 but keep it tidy)
    let mut rows: Vec<Vec<crate::channel::types::InlineButton>> = Vec::new();
    let mut row: Vec<crate::channel::types::InlineButton> = Vec::new();

    for m in models.iter().take(20) {
        let is_active = store
            .active_model
            .as_ref()
            .map(|a| a.provider_id == m.provider_id && a.model_id == m.model_id)
            .unwrap_or(false);
        let label = if is_active {
            format!("✓ {}", m.model_name)
        } else {
            m.model_name.clone()
        };
        // Telegram callback_data max is 64 bytes; truncate if needed
        let cb = format!("slash:model {}", m.model_name);
        let cb = if cb.len() > 64 {
            format!("slash:model {}", &m.model_id)
        } else {
            cb
        };
        row.push(crate::channel::types::InlineButton {
            text: label,
            callback_data: Some(cb),
            url: None,
        });
        if row.len() >= 2 {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(supports_draft: bool, supports_edit: bool) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: vec![ChatType::Dm, ChatType::Group, ChatType::Forum],
            supports_polls: false,
            supports_reactions: false,
            supports_draft,
            supports_edit,
            supports_unsend: false,
            supports_reply: true,
            supports_threads: true,
            supports_media: Vec::new(),
            supports_typing: true,
            max_message_length: Some(4096),
        }
    }

    #[test]
    fn extract_text_delta_reads_content_field() {
        let event = r#"{"type":"text_delta","content":"hello"}"#;
        assert_eq!(extract_text_delta(event).as_deref(), Some("hello"));
    }

    #[test]
    fn extract_text_delta_keeps_legacy_text_field_compatibility() {
        let event = r#"{"type":"text_delta","text":"hello"}"#;
        assert_eq!(extract_text_delta(event).as_deref(), Some("hello"));
    }

    #[test]
    fn select_preview_transport_prefers_draft_only_for_private_chats() {
        assert_eq!(
            select_stream_preview_transport(&ChatType::Dm, &caps(true, true)),
            Some(StreamPreviewTransport::Draft)
        );
        assert_eq!(
            select_stream_preview_transport(&ChatType::Group, &caps(true, true)),
            Some(StreamPreviewTransport::Message)
        );
    }

    #[test]
    fn draft_error_fallback_matches_unsupported_api_responses() {
        let err = "sendMessageDraft failed (404): method sendMessageDraft not found";
        assert!(should_fallback_from_draft_error(err));
    }
}

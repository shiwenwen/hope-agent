use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use super::media::convert_inbound_media_to_attachments;
use super::slash::{dispatch_slash_for_channel, ChannelSlashOutcome};
use super::streaming::{
    select_stream_preview_transport, spawn_channel_stream_task, StreamPreviewOutcome,
};

/// Maximum number of inbound messages processed concurrently.
/// Prevents resource exhaustion (DB lock contention, API rate limits) during message bursts.
const MAX_CONCURRENT_INBOUND: usize = 20;

/// Notify the frontend that a channel session has new messages.
pub(super) fn emit_channel_update(session_id: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "channel:message_update",
            serde_json::json!({
                "sessionId": session_id,
            }),
        );
    }
}

/// Notify the frontend that a channel session started/stopped streaming.
pub(super) fn emit_stream_lifecycle(event_name: &str, session_id: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
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
    // Use a dedicated thread with its own tokio runtime, since this is called
    // during init_app_state() before Tauri's async runtime is available.
    std::thread::Builder::new()
        .name("channel-dispatcher".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("channel dispatcher runtime");
            rt.block_on(async move {
                app_info!(
                    "channel",
                    "worker",
                    "Inbound message dispatcher started (max_concurrent={})",
                    MAX_CONCURRENT_INBOUND
                );
                let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_INBOUND));

                while let Some(msg) = inbound_rx.recv().await {
                    let registry = registry.clone();
                    let channel_db = channel_db.clone();
                    let permit = semaphore.clone().acquire_owned().await;

                    // Handle each message in a separate task, limited by semaphore
                    tokio::spawn(async move {
                        let _permit = permit; // held until task completes
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
        })
        .expect("spawn channel dispatcher thread");
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

    // 0. Check if this message is a text-reply to a pending approval prompt
    if super::approval::try_handle_approval_reply(&msg).await {
        app_info!(
            "channel",
            "worker",
            "[{}] Message consumed as approval reply from {}",
            channel_id_str,
            sender_label
        );
        return Ok(());
    }

    // 0b. Check if this message is a text-reply to a pending ask_user_question
    if super::ask_user::try_handle_ask_user_reply(&msg).await {
        app_info!(
            "channel",
            "worker",
            "[{}] Message consumed as ask_user reply from {}",
            channel_id_str,
            sender_label
        );
        return Ok(());
    }

    // 1. Load config and find account
    let store = crate::config::cached_config();
    app_debug!(
        "channel",
        "worker",
        "Config loaded: {} channel accounts, looking for '{}'",
        store.channels.accounts.len(),
        msg.account_id
    );
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
        ChatType::Group | ChatType::Forum => topic_config
            .and_then(|t| t.agent_id.as_deref())
            .or_else(|| effective_group_config.and_then(|g| g.agent_id.as_deref()))
            .unwrap_or(base_agent_id),
        ChatType::Channel => channel_config
            .and_then(|c| c.agent_id.as_deref())
            .unwrap_or(base_agent_id),
        ChatType::Dm => base_agent_id,
    };
    let agent_id = resolved_agent_id.to_string();

    // 3b. Resolve extra system prompt from group/topic/channel config
    let config_system_prompt = match msg.chat_type {
        ChatType::Group | ChatType::Forum => topic_config
            .and_then(|t| t.system_prompt.as_deref())
            .or_else(|| effective_group_config.and_then(|g| g.system_prompt.as_deref())),
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
                let _ = plugin
                    .send_message(&account.id, &msg.chat_id, &payload)
                    .await;
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
                let _ = plugin
                    .send_message(&account.id, &msg.chat_id, &payload)
                    .await;
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
    let (event_tx, event_rx) = mpsc::channel::<String>(512);

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

    // 8. Convert inbound media to agent Attachments
    let attachments = convert_inbound_media_to_attachments(&msg.media, &session_id);

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        message: engine_message,
        attachments,
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
            if let Some(st) = crate::globals::get_app_state() {
                let eff = st.reasoning_effort.lock().await.clone();
                if eff == "none" {
                    None
                } else {
                    Some(eff)
                }
            } else {
                None
            }
        },
        cancel: {
            if let Some(st) = crate::globals::get_app_state() {
                st.channel_cancels.register(&session_id)
            } else {
                Arc::new(AtomicBool::new(false))
            }
        },
        plan_agent_mode: None,
        plan_mode_allow_paths: None,
        skill_allowed_tools: Vec::new(),
        auto_approve_tools: account.auto_approve_tools,
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
    if let Some(st) = crate::globals::get_app_state() {
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

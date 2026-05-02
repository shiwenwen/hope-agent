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
    if let Err(e) = std::thread::Builder::new()
        .name("channel-dispatcher".into())
        .spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    app_error!(
                        "channel",
                        "worker",
                        "Failed to create channel dispatcher runtime: {}",
                        e
                    );
                    return;
                }
            };
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
    {
        app_error!(
            "channel",
            "worker",
            "Failed to spawn channel dispatcher thread: {}",
            e
        );
    }
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

    // 3. Resolve agent_id:
    // topic > group > channel > per-account > app global default > hardcoded default.
    let app_default_agent_id = store
        .default_agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(crate::agent::resolver::HARDCODED_DEFAULT_AGENT_ID);
    let base_agent_id = account
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(app_default_agent_id);

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

    // Auto-generate fallback title from first message (same logic as normal chat)
    let _ = crate::session::ensure_first_message_title(&session_db, &session_id, user_text);

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

    // 8. Create ChannelStreamSink + spawn streaming background task
    let (event_tx, event_rx) = mpsc::channel::<String>(512);

    // Collected MediaItems from tool_result events (send_attachment / image_generate).
    // Drained after `run_chat_engine` returns to deliver files through the
    // channel's native media API.
    let pending_media = std::sync::Arc::new(std::sync::Mutex::new(Vec::<
        crate::attachments::MediaItem,
    >::new()));

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
        codex_token: None,
        resolved_temperature,
        compact_config: store.compact.clone(),
        extra_system_context: Some(channel_context),
        reasoning_effort: crate::agent::live_reasoning_effort(None).await,
        cancel: match crate::globals::get_channel_cancels() {
            Some(reg) => reg.register(&session_id),
            None => Arc::new(AtomicBool::new(false)),
        },
        plan_context_override: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: account.auto_approve_tools,
        follow_global_reasoning_effort: true,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: crate::chat_engine::stream_seq::ChatSource::Channel,
        event_sink: Arc::new(crate::chat_engine::ChannelStreamSink::new(
            session_id.clone(),
            event_tx,
            pending_media.clone(),
        )),
    };

    // Notify frontend that streaming started (loading indicator)
    emit_stream_lifecycle("channel:stream_start", &session_id);

    // 9. Run shared chat engine (streaming, failover, tool persistence, etc.)
    let result = crate::chat_engine::run_chat_engine(engine_params).await;

    // Remove cancel handle now that engine is done
    if let Some(reg) = crate::globals::get_channel_cancels() {
        reg.remove(&session_id);
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

    // Late async tool completions that land after this drain are deferred
    // to a future turn — intentional; we don't want a stale attachment
    // from turn N leaking into turn N+1.
    let media_snapshot: Vec<crate::attachments::MediaItem> = {
        let mut guard = pending_media.lock().unwrap_or_else(|e| {
            app_warn!("channel", "worker", "pending_media poisoned: {}", e);
            e.into_inner()
        });
        std::mem::take(&mut *guard)
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
                &media_snapshot,
                &capabilities,
            )
            .await;

            app_info!(
                "channel",
                "worker",
                "[{}] Reply sent to {} ({} chars, {} media)",
                channel_id_str,
                msg.chat_id,
                response.len(),
                media_snapshot.len()
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

/// Max number of media items delivered per IM turn. Protects against a
/// runaway tool call blasting the channel. Excess items are logged and
/// silently dropped (the user will still see the link in the text summary
/// if the model appended one).
const MAX_MEDIA_PER_TURN: usize = 5;

/// Hard-limit text appended to the final reply when the channel can't
/// deliver a media item natively (LINE/IRC without public URL, unsupported
/// MIME). Each line: `📎 name — <url>` (or "unavailable" when no public URL
/// is configured).
fn build_media_fallback_lines(items: &[&crate::attachments::MediaItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let cfg = crate::config::cached_config();
    let public_base = cfg.server.public_base_url.as_deref().and_then(|s| {
        let trimmed = s.trim_end_matches('/');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let mut lines = Vec::new();
    lines.push("📎 Attachments:".to_string());
    for it in items {
        let link = public_base
            .map(|base| format!("{}{}", base, it.url))
            .unwrap_or_else(|| "(no public link configured)".to_string());
        lines.push(format!("- {}: {}", it.name, link));
    }
    Some(lines.join("\n"))
}

/// Map a `MediaItem` to `MediaType` based on MIME/kind. Unknown MIMEs fall
/// back to `Document` — a safe default supported by most channels.
fn classify_media_type(it: &crate::attachments::MediaItem) -> MediaType {
    use crate::attachments::MediaKind;
    let mime = it.mime_type.to_ascii_lowercase();
    if it.kind == MediaKind::Image || mime.starts_with("image/") {
        if mime == "image/gif" {
            // Telegram / Discord animate GIFs; `Photo` would lose animation.
            return MediaType::Animation;
        }
        return MediaType::Photo;
    }
    if mime.starts_with("video/") {
        return MediaType::Video;
    }
    if mime.starts_with("audio/") {
        return MediaType::Audio;
    }
    MediaType::Document
}

/// Split MediaItems into (native-supported, fallback) buckets based on the
/// channel's advertised capabilities. Unsupported items fall through to a
/// text link — the dispatcher appends them to the final reply.
///
/// Exposed at module level (rather than hidden inside `send_final_reply`)
/// so tests can pin down the partition behavior without spinning up a
/// full channel plugin.
pub(super) fn partition_media_by_channel<'a>(
    items: &'a [crate::attachments::MediaItem],
    caps: &ChannelCapabilities,
) -> (
    Vec<(&'a crate::attachments::MediaItem, MediaType)>,
    Vec<&'a crate::attachments::MediaItem>,
) {
    let mut native = Vec::new();
    let mut fallback = Vec::new();
    for it in items.iter().take(MAX_MEDIA_PER_TURN) {
        let t = classify_media_type(it);
        if caps.supports_media.contains(&t) {
            native.push((it, t));
        } else if t == MediaType::Animation && caps.supports_media.contains(&MediaType::Photo) {
            // Animation → Photo fallback for channels without native GIF support.
            native.push((it, MediaType::Photo));
        } else {
            fallback.push(it);
        }
    }
    if items.len() > MAX_MEDIA_PER_TURN {
        app_warn!(
            "channel",
            "worker",
            "Dropping {} media item(s) — over MAX_MEDIA_PER_TURN={}",
            items.len() - MAX_MEDIA_PER_TURN,
            MAX_MEDIA_PER_TURN
        );
    }
    (native, fallback)
}

/// Build an `OutboundMedia` from a `MediaItem`, preferring the absolute
/// `local_path` (zero-copy for local-disk delivery). Falls back to the
/// logical URL as a last resort so callers still get a reasonable payload
/// when `local_path` is missing (e.g. re-sent from persisted state).
fn to_outbound_media(it: &crate::attachments::MediaItem, media_type: MediaType) -> OutboundMedia {
    let data = match it.local_path.as_deref() {
        Some(p) if !p.is_empty() => MediaData::FilePath(p.to_string()),
        _ => MediaData::Url(it.url.clone()),
    };
    OutboundMedia {
        media_type,
        data,
        caption: it.caption.clone(),
    }
}

/// Send the final formatted response to the IM channel.
///
/// Order of delivery per turn:
/// 1. Text chunks (markdown → native formatting → split).
/// 2. One `send_message` per native-supported media item.
/// 3. A final text message with download links for unsupported media (if any).
///
/// A 50 ms gap between sends is intentional: most IM APIs rate-limit per
/// chat, and a tight loop trips flood protections on Telegram / LINE.
async fn send_final_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    response: &str,
    preview_message_id: Option<&str>,
    pending_media: &[crate::attachments::MediaItem],
    caps: &ChannelCapabilities,
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

    if pending_media.is_empty() {
        return;
    }

    let (native_items, fallback_items) = partition_media_by_channel(pending_media, caps);

    for (it, t) in &native_items {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let payload = ReplyPayload {
            text: None,
            media: vec![to_outbound_media(it, t.clone())],
            reply_to_message_id: None,
            parse_mode: None,
            buttons: Vec::new(),
            thread_id: msg.thread_id.clone(),
            draft_id: None,
        };
        match plugin
            .send_message(account_id, &msg.chat_id, &payload)
            .await
        {
            Ok(r) if !r.success => {
                app_warn!(
                    "channel",
                    "worker",
                    "Media send failed ({}): {}",
                    it.name,
                    r.error.unwrap_or_default()
                );
            }
            Err(e) => {
                app_error!("channel", "worker", "Media send error ({}): {}", it.name, e);
            }
            Ok(_) => {}
        }
    }

    if let Some(text) = build_media_fallback_lines(&fallback_items) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let payload = ReplyPayload {
            text: Some(text),
            reply_to_message_id: None,
            thread_id: msg.thread_id.clone(),
            parse_mode: None,
            buttons: Vec::new(),
            media: Vec::new(),
            draft_id: None,
        };
        let _ = plugin
            .send_message(account_id, &msg.chat_id, &payload)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{MediaItem, MediaKind};

    fn mk_item(name: &str, mime: &str, kind: MediaKind) -> MediaItem {
        MediaItem {
            url: format!("/api/attachments/s/{}", name),
            local_path: Some(format!("/tmp/{}", name)),
            name: name.to_string(),
            mime_type: mime.to_string(),
            size_bytes: 42,
            kind,
            caption: None,
        }
    }

    fn caps(supported: Vec<MediaType>) -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: Vec::new(),
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: false,
            supports_unsend: false,
            supports_reply: false,
            supports_threads: false,
            supports_media: supported,
            supports_typing: false,
            supports_buttons: false,
            max_message_length: None,
        }
    }

    #[test]
    fn classifies_images_videos_documents() {
        assert_eq!(
            classify_media_type(&mk_item("a.png", "image/png", MediaKind::Image)),
            MediaType::Photo
        );
        assert_eq!(
            classify_media_type(&mk_item("a.gif", "image/gif", MediaKind::Image)),
            MediaType::Animation
        );
        assert_eq!(
            classify_media_type(&mk_item("a.mp4", "video/mp4", MediaKind::File)),
            MediaType::Video
        );
        assert_eq!(
            classify_media_type(&mk_item("a.wav", "audio/wav", MediaKind::File)),
            MediaType::Audio
        );
        assert_eq!(
            classify_media_type(&mk_item("a.pdf", "application/pdf", MediaKind::File)),
            MediaType::Document
        );
    }

    #[test]
    fn partitions_by_capabilities() {
        let items = vec![
            mk_item("a.png", "image/png", MediaKind::Image),
            mk_item("a.mp4", "video/mp4", MediaKind::File),
            mk_item("a.pdf", "application/pdf", MediaKind::File),
        ];
        // Channel supports only Photo.
        let (native, fallback) = partition_media_by_channel(&items, &caps(vec![MediaType::Photo]));
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].1, MediaType::Photo);
        assert_eq!(fallback.len(), 2);
    }

    #[test]
    fn animation_falls_back_to_photo_when_channel_lacks_animation() {
        let items = vec![mk_item("a.gif", "image/gif", MediaKind::Image)];
        let (native, fallback) = partition_media_by_channel(&items, &caps(vec![MediaType::Photo]));
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].1, MediaType::Photo);
        assert!(fallback.is_empty());
    }

    #[test]
    fn drops_media_beyond_max_per_turn() {
        let items: Vec<_> = (0..(MAX_MEDIA_PER_TURN + 3))
            .map(|i| mk_item(&format!("f{}.pdf", i), "application/pdf", MediaKind::File))
            .collect();
        let (native, fallback) =
            partition_media_by_channel(&items, &caps(vec![MediaType::Document]));
        assert_eq!(native.len(), MAX_MEDIA_PER_TURN);
        assert!(fallback.is_empty());
    }

    #[test]
    fn outbound_prefers_local_path() {
        let it = mk_item("x.pdf", "application/pdf", MediaKind::File);
        let out = to_outbound_media(&it, MediaType::Document);
        assert!(matches!(out.data, MediaData::FilePath(_)));
    }
}

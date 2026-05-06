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
    select_stream_preview_transport, spawn_channel_stream_task, PreviewHandle,
    StreamPreviewOutcome, CARD_ELEMENT_MAX_CHARS,
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
        // Channels without inline-button support get the handler's verbose
        // no-arg text response instead of the (un-tappable) `Select an
        // option for /xxx:` shortcut.
        let supports_buttons = plugin.capabilities().supports_buttons;
        match dispatch_slash_for_channel(
            channel_db,
            &channel_id_str,
            &msg.account_id,
            &msg.chat_id,
            msg.thread_id.as_deref(),
            &session_id,
            &agent_id,
            user_text,
            supports_buttons,
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

    // Round-aware accumulator: text + media grouped per LLM round so the
    // dispatcher can deliver them in time order under `ImReplyMode::Split`,
    // or merge them under `Final` / `Preview`. See `RoundTextAccumulator`.
    let round_texts = std::sync::Arc::new(std::sync::Mutex::new(
        crate::chat_engine::RoundTextAccumulator::default(),
    ));

    let capabilities = plugin.capabilities();
    let reply_mode = account.im_reply_mode();
    // Stream preview is meaningful under `Preview` (one growing message)
    // AND `Split` (per-round message with typewriter effect; the stream
    // task closes each round inline). `Final` skips preview entirely so the
    // user only sees the last-round answer at the end.
    let preview_transport = match reply_mode {
        ImReplyMode::Preview | ImReplyMode::Split => {
            select_stream_preview_transport(&msg.chat_type, &capabilities)
        }
        ImReplyMode::Final => None,
    };
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
        reply_mode,
        round_texts.clone(),
        capabilities.clone(),
    );

    // 8. Convert inbound media to agent Attachments
    let attachments = convert_inbound_media_to_attachments(&msg.media, &session_id);
    let reasoning_effort = session_db
        .get_session(&session_id)
        .ok()
        .flatten()
        .and_then(|meta| meta.reasoning_effort)
        .or(crate::agent::live_reasoning_effort(None).await);
    if let (Some(cell), Some(effort)) = (
        crate::get_reasoning_effort_cell(),
        reasoning_effort.as_ref(),
    ) {
        *cell.lock().await = effort.clone();
    }

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
        reasoning_effort,
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
            round_texts.clone(),
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

    // Drain the round-aware accumulator. Late async tool completions that
    // arrive after this drain are deferred to a future turn — intentional,
    // a stale attachment from turn N must not leak into turn N+1.
    let drained_rounds: Vec<crate::chat_engine::RoundOutput> = {
        let mut guard = round_texts.lock().unwrap_or_else(|e| {
            app_warn!("channel", "worker", "round_texts poisoned: {}", e);
            e.into_inner()
        });
        guard.drain()
    };

    // 10. Process result — fan out per `ImReplyMode`.
    match result {
        Ok(engine_result) => {
            let metrics = match reply_mode {
                ImReplyMode::Split => {
                    deliver_split(
                        &plugin,
                        &account.id,
                        &msg,
                        &drained_rounds,
                        &engine_result.response,
                        stream_outcome.preview.as_ref(),
                        stream_outcome.finalized_rounds,
                        &capabilities,
                    )
                    .await
                }
                ImReplyMode::Final => {
                    deliver_final_only(
                        &plugin,
                        &account.id,
                        &msg,
                        &drained_rounds,
                        &engine_result.response,
                        &capabilities,
                    )
                    .await
                }
                ImReplyMode::Preview => {
                    deliver_preview_merged(
                        &plugin,
                        &account.id,
                        &msg,
                        &drained_rounds,
                        &engine_result.response,
                        stream_outcome.preview.as_ref(),
                        &capabilities,
                    )
                    .await
                }
            };

            app_info!(
                "channel",
                "worker",
                "[{}] Reply sent to {} (mode={}, rounds={}, finalized_inline={}, text_chars={}, media={}, preview={})",
                channel_id_str,
                msg.chat_id,
                reply_mode.as_str(),
                drained_rounds.len(),
                stream_outcome.finalized_rounds,
                metrics.text_chars,
                metrics.media_count,
                preview_transport.is_some(),
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
            send_error_reply(
                &plugin,
                &account.id,
                &msg.chat_id,
                stream_outcome.preview.as_ref(),
                &payload,
            )
            .await;
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

/// Replace the current preview (if any) with an error reply, falling back to
/// `send_message` whenever the preview path can't carry the error text. We
/// don't try to keep cardkit alive on the error path — the user should see a
/// plain text error attached to their original message.
async fn send_error_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    preview: Option<&PreviewHandle>,
    payload: &ReplyPayload,
) {
    match preview {
        Some(PreviewHandle::Message { message_id }) => {
            if let Err(edit_err) = plugin
                .edit_message(account_id, chat_id, message_id, payload)
                .await
            {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to replace preview with error reply: {}",
                    edit_err
                );
                let _ = plugin.send_message(account_id, chat_id, payload).await;
            }
        }
        Some(PreviewHandle::Card { .. }) | None => {
            // Card path: leave the half-rendered card alone (it'll auto-close
            // after 10 minutes server-side) and send the error as a fresh
            // text reply so the user sees what went wrong.
            let _ = plugin.send_message(account_id, chat_id, payload).await;
        }
    }
}

/// Write the full response into the streaming card and close streaming.
/// Returns `true` on success — caller skips the chunked-text path. Returns
/// `false` (after a best-effort `close_card_stream`) when the response is
/// too large or `update_card_element` fails; caller must deliver the full
/// response via `send_message` to avoid silent truncation.
async fn finalize_card_stream(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    card_id: &str,
    element_id: &str,
    sequence: i64,
    response: &str,
) -> bool {
    let response_chars = response.chars().count();
    if response_chars > CARD_ELEMENT_MAX_CHARS {
        app_warn!(
            "channel",
            "worker",
            "Final response too large for card element ({} chars > {}), falling back to text chunks",
            response_chars,
            CARD_ELEMENT_MAX_CHARS
        );
        let _ = plugin
            .close_card_stream(account_id, card_id, sequence)
            .await;
        return false;
    }

    if let Err(e) = plugin
        .update_card_element(account_id, card_id, element_id, response, sequence)
        .await
    {
        app_warn!(
            "channel",
            "worker",
            "Final card update failed (seq={}): {} — falling back to text chunks",
            sequence,
            e
        );
        // Best-effort close so the streaming indicator stops. Errors here
        // are cosmetic — the 10-minute auto-close is the safety net.
        let _ = plugin
            .close_card_stream(account_id, card_id, sequence + 1)
            .await;
        return false;
    }

    if let Err(e) = plugin
        .close_card_stream(account_id, card_id, sequence + 1)
        .await
    {
        // Card content was committed; close failure is cosmetic (10-min
        // auto-close is the safety net), no fallback needed.
        app_warn!(
            "channel",
            "worker",
            "close_card_stream failed (seq={}): {}",
            sequence + 1,
            e
        );
    }

    true
}

/// Split the response into native-rendered chunks and deliver them via
/// `send_message`. `preview` only honors the `Message` variant for the
/// first chunk (replaces an existing preview via `edit_message`); all
/// other variants are treated as no preview and send fresh.
///
/// Public to the worker module so the stream task can use it as a
/// guaranteed-delivery fallback when its preview-based path can't carry
/// a round (text > max_msg_len, send/edit error, broken card session).
pub(super) async fn send_text_chunks(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    reply_to_message_id: &str,
    response: &str,
    preview: Option<&PreviewHandle>,
) {
    let native_text = plugin.markdown_to_native(response);
    let chunks = plugin.chunk_message(&native_text);

    for (i, chunk) in chunks.iter().enumerate() {
        let payload = if i == 0 {
            ReplyPayload {
                text: Some(chunk.clone()),
                reply_to_message_id: Some(reply_to_message_id.to_string()),
                thread_id: thread_id.map(|s| s.to_string()),
                parse_mode: Some(ParseMode::Html),
                ..ReplyPayload::text("")
            }
        } else {
            ReplyPayload {
                text: Some(chunk.clone()),
                thread_id: thread_id.map(|s| s.to_string()),
                parse_mode: Some(ParseMode::Html),
                ..ReplyPayload::text("")
            }
        };

        let delivery = if i == 0 {
            match preview {
                Some(PreviewHandle::Message { message_id }) => {
                    match plugin
                        .edit_message(account_id, chat_id, message_id, &payload)
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
                            plugin.send_message(account_id, chat_id, &payload).await
                        }
                    }
                }
                _ => plugin.send_message(account_id, chat_id, &payload).await,
            }
        } else {
            plugin.send_message(account_id, chat_id, &payload).await
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

/// Aggregated counters used by the dispatcher for the post-turn log line.
#[derive(Debug, Default)]
struct DeliveryMetrics {
    text_chars: usize,
    media_count: usize,
}

/// `ImReplyMode::Split`: deliver each round in time order.
///
/// Two execution paths share this function:
///
/// 1. **Streaming-capable channel (`finalized_rounds > 0`)**: the stream
///    task already delivered rounds `0..finalized_rounds` inline (preview
///    + media per round). We only handle `rounds[finalized_rounds..]`,
///    which under normal flow is either empty (model ended on a tool_call)
///    or exactly one entry (the final round whose preview is still open).
///
/// 2. **Non-streaming channel (`finalized_rounds == 0`)**: the stream task
///    drained events without rendering. We iterate every round here, sending
///    pre-final narration as one-shot `send_message` + media fan-out, and
///    routing the last round through `send_final_reply`.
///
/// Falls back to `engine_result.response` only if the sink saw no events at
/// all — defensive for engine paths that don't stream through us.
#[allow(clippy::too_many_arguments)]
async fn deliver_split(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    rounds: &[crate::chat_engine::RoundOutput],
    fallback_response: &str,
    preview: Option<&PreviewHandle>,
    finalized_rounds: usize,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let mut metrics = DeliveryMetrics::default();
    if rounds.is_empty() {
        // Engine produced no rounds (sink never saw events). Use the merged
        // response so the user still gets a reply.
        send_final_reply(
            plugin,
            account_id,
            msg,
            fallback_response,
            preview,
            &[],
            caps,
        )
        .await;
        metrics.text_chars = fallback_response.chars().count();
        return metrics;
    }

    // Tally already-finalized rounds so the post-turn log matches reality.
    let split_at = finalized_rounds.min(rounds.len());
    for r in &rounds[..split_at] {
        metrics.text_chars += r.text.chars().count();
        metrics.media_count += r.medias.len();
    }

    let remaining = &rounds[split_at..];
    if remaining.is_empty() {
        // Stream task finalized everything (model ended on a tool_call,
        // last round had no trailing narration). Nothing left to ship.
        return metrics;
    }

    let last_idx = remaining.len() - 1;
    for (i, round) in remaining.iter().enumerate() {
        if i == last_idx {
            // Final round goes through the canonical finalize-+ -media path.
            send_final_reply(
                plugin,
                account_id,
                msg,
                &round.text,
                preview,
                &round.medias,
                caps,
            )
            .await;
            metrics.text_chars += round.text.chars().count();
            metrics.media_count += round.medias.len();
        } else {
            // Pre-final round (only reached on non-streaming channels — the
            // stream task would have finalized this round inline otherwise).
            if !round.text.trim().is_empty() {
                let payload = ReplyPayload {
                    text: Some(round.text.clone()),
                    reply_to_message_id: None,
                    thread_id: msg.thread_id.clone(),
                    ..ReplyPayload::text("")
                };
                match plugin
                    .send_message(account_id, &msg.chat_id, &payload)
                    .await
                {
                    Ok(r) if !r.success => {
                        app_warn!(
                            "channel",
                            "worker",
                            "split-mode pre-round send failed: {}",
                            r.error.unwrap_or_default()
                        );
                    }
                    Err(e) => {
                        app_warn!(
                            "channel",
                            "worker",
                            "split-mode pre-round send error: {}",
                            e
                        );
                    }
                    _ => {}
                }
                metrics.text_chars += round.text.chars().count();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            deliver_media_to_chat(
                plugin,
                account_id,
                &msg.chat_id,
                msg.thread_id.as_deref(),
                &round.medias,
                caps,
            )
            .await;
            metrics.media_count += round.medias.len();
        }
    }
    metrics
}

/// `ImReplyMode::Final`: send only the final round's narration plus all
/// rounds' media, in one outbound burst. The dispatcher forces
/// `preview_transport=None` for this mode, so no preview handle exists to
/// finalize — go straight through `send_final_reply` with `None`.
async fn deliver_final_only(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    rounds: &[crate::chat_engine::RoundOutput],
    fallback_response: &str,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let final_text: String = rounds
        .last()
        .map(|r| r.text.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback_response.to_string());
    let all_media: Vec<crate::attachments::MediaItem> = rounds
        .iter()
        .flat_map(|r| r.medias.iter().cloned())
        .collect();
    let media_count = all_media.len();
    let text_chars = final_text.chars().count();
    send_final_reply(plugin, account_id, msg, &final_text, None, &all_media, caps).await;
    DeliveryMetrics {
        text_chars,
        media_count,
    }
}

/// `ImReplyMode::Preview`: keep the legacy "one growing preview message"
/// behavior. Uses `engine_result.response` (the merged collected_text) as
/// the canonical text — it matches what the live preview was rendering
/// during the turn — and finalizes via `send_final_reply` so the preview
/// transport's edit/close path runs. All media follow at the end.
///
/// Non-streaming channels reach this branch with `preview = None`; behavior
/// degrades to the same as `Final` minus the "drop pre-final narration"
/// trim — i.e. a single message containing the merged text.
async fn deliver_preview_merged(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    rounds: &[crate::chat_engine::RoundOutput],
    fallback_response: &str,
    preview: Option<&PreviewHandle>,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let all_media: Vec<crate::attachments::MediaItem> = rounds
        .iter()
        .flat_map(|r| r.medias.iter().cloned())
        .collect();
    let media_count = all_media.len();
    let text_chars = fallback_response.chars().count();
    send_final_reply(
        plugin,
        account_id,
        msg,
        fallback_response,
        preview,
        &all_media,
        caps,
    )
    .await;
    DeliveryMetrics {
        text_chars,
        media_count,
    }
}

/// Send the final formatted response to the IM channel.
///
/// Order of delivery per turn:
/// 1. Text content (one of two paths — see below).
/// 2. One `send_message` per native-supported media item.
/// 3. A final text message with download links for unsupported media (if any).
///
/// A 50 ms gap between sends is intentional: most IM APIs rate-limit per
/// chat, and a tight loop trips flood protections on Telegram / LINE.
///
/// Text routing is decided by `preview`:
/// - `Card { broken: false, .. }`: write the **entire** raw response into the
///   card element in one shot (cardkit elements hold ~100k chars, far above
///   any IM `max_message_length`), then close streaming. On any failure
///   (response oversize, update error, etc.) the card is closed best-effort
///   and we fall through to plain text chunks below.
/// - Anything else (`Message`, `Card{broken:true}`, `None`): split the
///   markdown-to-native rendered response into chunks and `send_message` each
///   one. For `Message`, the first chunk replaces the existing preview via
///   `edit_message` (with `send_message` as a fallback).
async fn send_final_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    msg: &MsgContext,
    response: &str,
    preview: Option<&PreviewHandle>,
    pending_media: &[crate::attachments::MediaItem],
    caps: &ChannelCapabilities,
) {
    let card_finalized = match preview {
        Some(PreviewHandle::Card {
            card_id,
            element_id,
            sequence,
            broken: false,
            ..
        }) => {
            finalize_card_stream(plugin, account_id, card_id, element_id, *sequence, response).await
        }
        _ => false,
    };

    if !card_finalized {
        // Card variants here are either `broken=true` or `broken=false` whose
        // finalize just failed. In both cases the half-rendered card stays
        // in chat (cardkit auto-closes it after 10 minutes); we deliver a
        // fresh, complete text reply via send_message so the user sees the
        // full response. Treat the preview as `None` for the chunk loop.
        let chunk_preview = match preview {
            Some(PreviewHandle::Card { .. }) => None,
            other => other,
        };
        send_text_chunks(
            plugin,
            account_id,
            &msg.chat_id,
            msg.thread_id.as_deref(),
            &msg.message_id,
            response,
            chunk_preview,
        )
        .await;
    }

    deliver_media_to_chat(
        plugin,
        account_id,
        &msg.chat_id,
        msg.thread_id.as_deref(),
        pending_media,
        caps,
    )
    .await;
}

/// Send a batch of media items through the channel, falling back to a text
/// download link for unsupported MIME types. Each `send_message` is followed
/// by a 50 ms gap to stay under per-chat rate limits — Telegram and LINE
/// both flood-protect tight loops. Used by `send_final_reply`, the
/// `Split`-mode dispatcher's per-round fan-out, and the stream task's
/// inline per-round delivery.
pub(super) async fn deliver_media_to_chat(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    items: &[crate::attachments::MediaItem],
    caps: &ChannelCapabilities,
) {
    if items.is_empty() {
        return;
    }

    let (native_items, fallback_items) = partition_media_by_channel(items, caps);

    for (it, t) in &native_items {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let payload = ReplyPayload {
            text: None,
            media: vec![to_outbound_media(it, t.clone())],
            reply_to_message_id: None,
            parse_mode: None,
            buttons: Vec::new(),
            thread_id: thread_id.map(|s| s.to_string()),
            draft_id: None,
        };
        match plugin.send_message(account_id, chat_id, &payload).await {
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
            thread_id: thread_id.map(|s| s.to_string()),
            parse_mode: None,
            buttons: Vec::new(),
            media: Vec::new(),
            draft_id: None,
        };
        let _ = plugin.send_message(account_id, chat_id, &payload).await;
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
            supports_card_stream: false,
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

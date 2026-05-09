use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

use crate::channel::db::ChannelDB;
use crate::channel::registry::ChannelRegistry;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

use super::media::convert_inbound_media_to_attachments;
use super::pipeline::{
    await_stream_pipeline, deliver_rounds, spawn_stream_pipeline, DeliveryTarget,
};
use super::slash::{dispatch_slash_for_channel, ChannelSlashOutcome};
use super::streaming::{append_preview_round_text, PreviewHandle, CARD_ELEMENT_MAX_CHARS};

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

/// Spawn the inbound event dispatcher as a background tokio task.
///
/// This task receives [`InboundEvent`] from all channel plugins and:
/// - For [`InboundEvent::Message`] — validates access, resolves or creates a
///   session, runs the chat round, sends the response back.
/// - For non-Message variants (reaction / edited / recalled / membership /
///   read receipt) — currently log-only. Business behavior (sync session
///   messages on edit/recall, BotLeft cleanup, welcome templates) is deferred
///   to v0.3+ Phase B.2.
pub fn spawn_dispatcher(
    registry: Arc<ChannelRegistry>,
    channel_db: Arc<ChannelDB>,
    mut inbound_rx: mpsc::Receiver<InboundEvent>,
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
                    "Inbound event dispatcher started (max_concurrent={})",
                    MAX_CONCURRENT_INBOUND
                );
                let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_INBOUND));

                while let Some(event) = inbound_rx.recv().await {
                    match event {
                        InboundEvent::Message(msg) => {
                            let registry = registry.clone();
                            let channel_db = channel_db.clone();
                            let permit = semaphore.clone().acquire_owned().await;

                            // Handle each message in a separate task, limited by semaphore
                            tokio::spawn(async move {
                                let _permit = permit; // held until task completes
                                if let Err(e) =
                                    handle_inbound_message(&registry, &channel_db, msg).await
                                {
                                    app_error!(
                                        "channel",
                                        "worker",
                                        "Failed to handle inbound message: {}",
                                        e
                                    );
                                }
                            });
                        }
                        InboundEvent::Reaction(ev) => log_reaction(&ev),
                        InboundEvent::MessageEdited(ev) => log_message_edited(&ev),
                        InboundEvent::MessageRecalled(ev) => log_message_recalled(&ev),
                        InboundEvent::Membership(ev) => log_membership(&ev),
                        InboundEvent::ReadReceipt(ev) => log_read_receipt(&ev),
                    }
                }

                app_info!("channel", "worker", "Inbound event dispatcher stopped");
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

// ── Non-Message event handlers (log-only in v0.2.0) ──────────────
// Business behavior (sync edits to messages table, recall removal, BotLeft
// cleanup, auto-welcome on join) is deferred to v0.3+ Phase B.2. For now we
// just surface the event in the application log so operators / agent self-
// diagnosis can see that the plumbing works.

fn log_reaction(ev: &ReactionEvent) {
    app_info!(
        "channel",
        "inbound",
        "[{}/{}] reaction {} {} on msg={} by={}",
        ev.common.channel_id,
        ev.common.account_id,
        if ev.added { "+" } else { "-" },
        ev.emoji,
        ev.message_id,
        ev.sender_id
    );
}

fn log_message_edited(ev: &EditedMessageEvent) {
    app_info!(
        "channel",
        "inbound",
        "[{}/{}] message_edited msg={} by={} edited_at={}",
        ev.common.channel_id,
        ev.common.account_id,
        ev.message_id,
        ev.sender_id,
        ev.edited_at
    );
}

fn log_message_recalled(ev: &RecalledMessageEvent) {
    app_info!(
        "channel",
        "inbound",
        "[{}/{}] message_recalled msg={} by={}",
        ev.common.channel_id,
        ev.common.account_id,
        ev.message_id,
        ev.recalled_by.as_deref().unwrap_or("?")
    );
}

fn log_membership(ev: &MembershipEvent) {
    app_info!(
        "channel",
        "inbound",
        "[{}/{}] membership chat={} action={:?}",
        ev.common.channel_id,
        ev.common.account_id,
        ev.common.chat_id,
        ev.action
    );
}

fn log_read_receipt(ev: &ReadReceiptEvent) {
    app_info!(
        "channel",
        "inbound",
        "[{}/{}] read_receipt msg={} reader={}",
        ev.common.channel_id,
        ev.common.account_id,
        ev.message_id,
        ev.reader_id
    );
}

/// Process a single inbound message from a channel.
async fn handle_inbound_message(
    registry: &ChannelRegistry,
    channel_db: &ChannelDB,
    mut msg: MsgContext,
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

    // 2d. Hydrate any deferred-download attachments now that gating has
    //     cleared. Channels that download eagerly (Telegram, Slack, etc.)
    //     leave the trait method as a no-op; Feishu uses this hook so the
    //     gateway ack isn't blocked on attachment downloads. Failures are
    //     non-fatal — the surrounding text still reaches the agent.
    if let Err(e) = plugin.materialize_pending_media(&account, &mut msg).await {
        app_warn!(
            "channel",
            "worker",
            "[{}] Failed to materialize pending media for {} in {}: {}",
            channel_id_str,
            msg.message_id,
            msg.chat_id,
            e
        );
    }

    // 3. Resolve agent_id via the central resolver — the precedence chain
    //    (project > topic > group > channel-override > channel-account >
    //    global > hardcoded) lives in `agent::resolver` so /status, IM
    //    dispatch, and desktop / HTTP all share one source of truth.
    //    Only the IM-relevant levels are passed in here; project routing
    //    is now explicit (`/project <id>` from inside the chat).
    let (agent_id, _agent_source) = match msg.chat_type {
        ChatType::Group | ChatType::Forum => crate::agent::resolver::resolve_default_agent_id_full(
            None,
            None,
            topic_config,
            effective_group_config,
            None,
            Some(&account),
        ),
        ChatType::Channel => crate::agent::resolver::resolve_default_agent_id_full(
            None,
            None,
            None,
            None,
            channel_config,
            Some(&account),
        ),
        ChatType::Dm => crate::agent::resolver::resolve_default_agent_id_full(
            None,
            None,
            None,
            None,
            None,
            Some(&account),
        ),
    };

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

    // 4. Prepare inbound text. Reply-only slash commands (e.g. /status)
    // are persisted as event history below, but never as user turns and
    // never into model-facing context.
    let session_db =
        crate::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))?;

    let user_text = msg.text.as_deref().unwrap_or("(media message)");

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
            &plugin,
            &account,
            &channel_id_str,
            &msg.account_id,
            &msg.chat_id,
            msg.thread_id.as_deref(),
            &msg.chat_type,
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
                if new_session_id.is_none() {
                    if let Err(e) = crate::slash_commands::append_slash_history_events(
                        &session_db,
                        effective_sid,
                        user_text,
                        Some(&content),
                        crate::chat_engine::ChatSource::Channel,
                    ) {
                        app_warn!(
                            "channel",
                            "worker",
                            "Failed to persist slash command history: {}",
                            e
                        );
                    }
                }
                let slash_target = DeliveryTarget {
                    account_id: &account.id,
                    chat_id: &msg.chat_id,
                    thread_id: msg.thread_id.as_deref(),
                    reply_to_message_id: Some(&msg.message_id),
                };
                send_text_chunks(&plugin, &slash_target, &content, None, &buttons).await;
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
                let err_target = DeliveryTarget {
                    account_id: &account.id,
                    chat_id: &msg.chat_id,
                    thread_id: msg.thread_id.as_deref(),
                    reply_to_message_id: Some(&msg.message_id),
                };
                send_text_chunks(&plugin, &err_target, &error_reply, None, &[]).await;
                emit_stream_lifecycle("channel:stream_end", &session_id);
                return Ok(());
            }
        }
    } else {
        engine_message = user_text.to_string();
    }

    // 5b. Persist only messages that will enter the chat engine. Reply-only
    // slash commands returned above after writing event history.
    let mut user_msg = crate::session::NewMessage::user(user_text)
        .with_source(crate::chat_engine::ChatSource::Channel);
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

    // Auto-generate fallback title from the first real message (same logic as normal chat).
    let _ = crate::session::ensure_first_message_title(&session_db, &session_id, user_text);

    // Notify the desktop / web side that a fresh user message landed on
    // this session from IM, so an attached GUI view can pull it into
    // the conversation timeline without waiting for the stream-start
    // round-trip. `channel:stream_start` covers the assistant side a
    // moment later — this event is purely about the inbound user turn.
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "chat:user_message_appended",
            serde_json::json!({
                "sessionId": &session_id,
                "source": "channel",
                "channelId": &channel_id_str,
                "accountId": &msg.account_id,
                "chatId": &msg.chat_id,
                "senderName": msg.sender_name.as_deref(),
                "text": user_text,
            }),
        );
    }

    // NOTE: We don't emit channel:message_update here because channel:stream_start
    // will handle frontend state. Emitting here would race with the stream placeholder.

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

    // 8. Spawn the shared streaming pipeline (preview task + sink). The
    // chat engine writes events into `pipeline.event_sink`; we await the
    // stream task and deliver rounds after `run_chat_engine` returns.
    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id: &msg.chat_id,
        thread_id: msg.thread_id.as_deref(),
        reply_to_message_id: Some(msg.message_id.as_str()),
    };
    // Inbound IM turns broadcast on `channel:stream_delta` so the GUI can
    // mirror the IM session live.
    let pipeline = spawn_stream_pipeline(
        &plugin,
        &account,
        &msg.chat_type,
        &session_id,
        &target,
        true,
    );
    let event_sink = pipeline.event_sink.clone();

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

    // Snapshot whether the *entire* fallback chain is Codex before
    // `model_chain` is moved into engine_params. Drives the `🔐 Codex
    // session expired` headline in the error path below.
    //
    // Conservative `all` rather than `primary-only`: engine returns
    // `Result<_, String>` and erases which model in the chain actually
    // failed (see F-072). With a mixed chain (e.g. OpenAI primary +
    // Codex fallback) we'd guess wrong either way — falling through to
    // the generic Auth headline ("re-check the API key in settings") is
    // strictly better than directing the user to re-auth Codex when the
    // OpenAI primary actually 401'd.
    let chain_is_all_codex = !model_chain.is_empty()
        && model_chain.iter().all(|m| {
            store
                .providers
                .iter()
                .find(|p| p.id == m.provider_id)
                .is_some_and(|p| p.api_type.is_codex())
        });

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        message: engine_message,
        // IM-inbound turns echo the user's typed text via the IM client
        // itself (no GUI display_text mechanism); leave the mirror quote on
        // the raw message.
        display_text: None,
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
        event_sink,
    };

    emit_stream_lifecycle("channel:stream_start", &session_id);

    let result = crate::chat_engine::run_chat_engine(engine_params).await;

    if let Some(reg) = crate::globals::get_channel_cancels() {
        reg.remove(&session_id);
    }

    // Late async tool completions arriving after this drain are deferred to
    // a future turn — a stale attachment from turn N must not leak into N+1.
    let outcome = await_stream_pipeline(pipeline).await;

    match result {
        Ok(engine_result) => {
            let metrics = deliver_rounds(&plugin, &target, &outcome, &engine_result.response).await;

            app_info!(
                "channel",
                "worker",
                "[{}] Reply sent to {} (mode={}, rounds={}, finalized_inline={}, text_chars={}, media={}, preview={})",
                channel_id_str,
                msg.chat_id,
                outcome.reply_mode.as_str(),
                outcome.drained_rounds.len(),
                outcome.stream_outcome.finalized_rounds,
                metrics.text_chars,
                metrics.media_count,
                outcome.preview_active,
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

            // Classify on the way out — engine erases the typed reason when
            // it folds `ExecutorError` into `String`. IM-inbound has
            // `abort_on_cancel=false`, so any error reaching here is a real
            // failure (not a user cancel).
            let raw = e.to_string();
            let reason = crate::failover::classify_error(&raw);
            let is_codex_auth =
                matches!(reason, crate::failover::FailoverReason::Auth) && chain_is_all_codex;
            let body = crate::chat_engine::im_error_message::format_im_engine_error(
                crate::chat_engine::im_error_message::ImErrorContext {
                    reason,
                    raw: &raw,
                    is_codex_auth,
                },
            );
            let err_target = DeliveryTarget {
                account_id: &account.id,
                chat_id: &msg.chat_id,
                thread_id: msg.thread_id.as_deref(),
                reply_to_message_id: Some(&msg.message_id),
            };
            send_error_reply(
                &plugin,
                &err_target,
                outcome.stream_outcome.preview.as_ref(),
                &body,
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

/// Replace the current preview (if any) with an error reply, routing through
/// `send_text_chunks` so long error text (rare but possible — formatted
/// engine traces) is markdown-to-native rendered + chunked. We don't try to
/// keep cardkit alive on the error path — the user should see a plain text
/// error attached to their original message; the half-rendered card auto-
/// closes server-side after 10 minutes.
async fn send_error_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    preview: Option<&PreviewHandle>,
    error_text: &str,
) {
    let chunk_preview = match preview {
        // Card path: pass `None` so chunk-send opens a fresh message;
        // half-rendered card is left to auto-close.
        Some(PreviewHandle::Card { .. }) => None,
        other => other,
    };
    send_text_chunks(plugin, target, error_text, chunk_preview, &[]).await;
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
/// `reply_to_message_id` only applies to chunk 0; pass `None` to send
/// without quoting (catch-up / mirror paths that have no inbound message
/// to reply to).
///
/// `buttons` are appended only to the **last** chunk so the inline buttons
/// stick to the trailing message (the chunk visually closest to the user's
/// next interaction). Pass `&[]` for plain text.
///
/// Visible to the rest of the crate so attach catch-up + future mirror
/// paths can reuse the same chunk-aware `markdown_to_native` →
/// `chunk_message` → `send_message` sequence used by the live dispatcher.
pub(crate) async fn send_text_chunks(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    response: &str,
    preview: Option<&PreviewHandle>,
    buttons: &[Vec<InlineButton>],
) {
    let native_text = plugin.markdown_to_native(response);
    let chunks = plugin.chunk_message(&native_text);
    let last_idx = chunks.len().saturating_sub(1);

    for (i, chunk) in chunks.iter().enumerate() {
        // Per-chunk throttle: same 50ms gap deliver_media_to_chat uses to
        // dodge Telegram / LINE / WeChat per-chat flood protections.
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let chunk_buttons = if i == last_idx {
            buttons.to_vec()
        } else {
            Vec::new()
        };
        let payload = if i == 0 {
            ReplyPayload {
                text: Some(chunk.clone()),
                reply_to_message_id: target.reply_to_message_id.map(str::to_string),
                thread_id: target.thread_id.map(|s| s.to_string()),
                parse_mode: Some(ParseMode::Html),
                buttons: chunk_buttons,
                ..ReplyPayload::text("")
            }
        } else {
            ReplyPayload {
                text: Some(chunk.clone()),
                thread_id: target.thread_id.map(|s| s.to_string()),
                parse_mode: Some(ParseMode::Html),
                buttons: chunk_buttons,
                ..ReplyPayload::text("")
            }
        };

        let delivery = if i == 0 {
            match preview {
                Some(PreviewHandle::Message { message_id }) => {
                    match plugin
                        .edit_message(target.account_id, target.chat_id, message_id, &payload)
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
                                .send_message(target.account_id, target.chat_id, &payload)
                                .await
                        }
                    }
                }
                _ => {
                    plugin
                        .send_message(target.account_id, target.chat_id, &payload)
                        .await
                }
            }
        } else {
            plugin
                .send_message(target.account_id, target.chat_id, &payload)
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

/// Aggregated counters used by the dispatcher for the post-turn log line.
#[derive(Debug, Default)]
pub(crate) struct DeliveryMetrics {
    pub text_chars: usize,
    pub media_count: usize,
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
pub(super) async fn deliver_split(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    rounds: &[crate::chat_engine::RoundOutput],
    fallback_response: &str,
    preview: Option<&PreviewHandle>,
    finalized_rounds: usize,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let mut metrics = DeliveryMetrics::default();
    if rounds.is_empty() {
        send_final_reply(plugin, target, fallback_response, preview, &[], caps).await;
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
        return metrics;
    }

    let last_idx = remaining.len() - 1;
    for (i, round) in remaining.iter().enumerate() {
        if i == last_idx {
            send_final_reply(plugin, target, &round.text, preview, &round.medias, caps).await;
            metrics.text_chars += round.text.chars().count();
            metrics.media_count += round.medias.len();
        } else {
            // Pre-final round only reaches here on non-streaming channels —
            // streaming channels finalize per-round inline.
            if !round.text.trim().is_empty() {
                let pre_target = DeliveryTarget {
                    account_id: target.account_id,
                    chat_id: target.chat_id,
                    thread_id: target.thread_id,
                    reply_to_message_id: None,
                };
                send_text_chunks(plugin, &pre_target, &round.text, None, &[]).await;
                metrics.text_chars += round.text.chars().count();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            deliver_media_to_chat(
                plugin,
                target.account_id,
                target.chat_id,
                target.thread_id,
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
pub(super) async fn deliver_final_only(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
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
    send_final_reply(plugin, target, &final_text, None, &all_media, caps).await;
    DeliveryMetrics {
        text_chars,
        media_count,
    }
}

/// `ImReplyMode::Preview`: keep the legacy "one growing preview message"
/// behavior. Joins per-round narration in time order to reconstruct the
/// canonical final text with one newline at tool-round boundaries — matching
/// what the live preview task rendered. Falls back to `engine_result.response`
/// only when `rounds` is empty (the engine bailed before any text streamed).
/// All media follow at the end via `send_final_reply`.
///
/// Non-streaming channels reach this branch with `preview = None`; behavior
/// degrades to the same as `Final` minus the "drop pre-final narration"
/// trim — i.e. a single message containing the merged text.
pub(super) async fn deliver_preview_merged(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    rounds: &[crate::chat_engine::RoundOutput],
    fallback_response: &str,
    preview: Option<&PreviewHandle>,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let final_text: String = if rounds.is_empty() {
        fallback_response.to_string()
    } else {
        let merged = merge_preview_round_texts(rounds);
        if merged.is_empty() {
            fallback_response.to_string()
        } else {
            merged
        }
    };
    let all_media: Vec<crate::attachments::MediaItem> = rounds
        .iter()
        .flat_map(|r| r.medias.iter().cloned())
        .collect();
    let media_count = all_media.len();
    let text_chars = final_text.chars().count();
    send_final_reply(plugin, target, &final_text, preview, &all_media, caps).await;
    DeliveryMetrics {
        text_chars,
        media_count,
    }
}

pub(super) fn merge_preview_round_texts(rounds: &[crate::chat_engine::RoundOutput]) -> String {
    let mut merged = String::new();
    for round in rounds {
        let new_round = !merged.is_empty();
        append_preview_round_text(&mut merged, &round.text, new_round);
    }
    merged
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
///   any IM per-send byte ceiling), then close streaming. On any failure
///   (response oversize, update error, etc.) the card is closed best-effort
///   and we fall through to plain text chunks below.
/// - Anything else (`Message`, `Card{broken:true}`, `None`): split the
///   markdown-to-native rendered response into chunks and `send_message` each
///   one. For `Message`, the first chunk replaces the existing preview via
///   `edit_message` (with `send_message` as a fallback).
pub(super) async fn send_final_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
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
            finalize_card_stream(
                plugin,
                target.account_id,
                card_id,
                element_id,
                *sequence,
                response,
            )
            .await
        }
        _ => false,
    };

    if !card_finalized {
        // Half-rendered card stays in chat (cardkit auto-closes after 10
        // min); deliver a fresh, complete text reply via send_message.
        let chunk_preview = match preview {
            Some(PreviewHandle::Card { .. }) => None,
            other => other,
        };
        send_text_chunks(plugin, target, response, chunk_preview, &[]).await;
    }

    deliver_media_to_chat(
        plugin,
        target.account_id,
        target.chat_id,
        target.thread_id,
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
pub(crate) async fn deliver_media_to_chat(
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
        // Route through send_text_chunks so an oversized URL list (lots of
        // attachments × long URLs) is split per the channel's chunk ceiling.
        let target = DeliveryTarget {
            account_id,
            chat_id,
            thread_id,
            reply_to_message_id: None,
        };
        send_text_chunks(plugin, &target, &text, None, &[]).await;
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
            streaming_preview_max_bytes: None,
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

    use crate::chat_engine::RoundOutput;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    struct CountingPlugin {
        max_bytes: usize,
        sends: Mutex<Vec<String>>,
        send_count: AtomicUsize,
    }

    impl CountingPlugin {
        fn new(max_bytes: usize) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChannelPlugin for CountingPlugin {
        fn meta(&self) -> ChannelMeta {
            ChannelMeta {
                id: ChannelId::Custom("test".to_string()),
                display_name: "Test".to_string(),
                description: String::new(),
                version: "0".to_string(),
            }
        }

        fn capabilities(&self) -> ChannelCapabilities {
            let mut c = caps(Vec::new());
            c.chat_types = vec![ChatType::Dm];
            c.streaming_preview_max_bytes = Some(self.max_bytes);
            c
        }

        async fn start_account(
            &self,
            _account: &ChannelAccountConfig,
            _inbound_tx: mpsc::Sender<InboundEvent>,
            _cancel: CancellationToken,
        ) -> Result<()> {
            Ok(())
        }

        async fn stop_account(&self, _account_id: &str) -> Result<()> {
            Ok(())
        }

        async fn send_message(
            &self,
            _account_id: &str,
            _chat_id: &str,
            payload: &ReplyPayload,
        ) -> Result<DeliveryResult> {
            let n = self.send_count.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(text) = payload.text.as_ref() {
                self.sends.lock().unwrap().push(text.clone());
            }
            Ok(DeliveryResult::ok(format!("msg-{}", n)))
        }

        async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
            Ok(())
        }

        async fn probe(&self, _account: &ChannelAccountConfig) -> Result<ChannelHealth> {
            Ok(ChannelHealth::default())
        }

        fn check_access(&self, _account: &ChannelAccountConfig, _msg: &MsgContext) -> bool {
            true
        }

        fn markdown_to_native(&self, markdown: &str) -> String {
            markdown.to_string()
        }

        async fn validate_credentials(&self, _credentials: &serde_json::Value) -> Result<String> {
            Ok("test-bot".to_string())
        }
    }

    #[tokio::test]
    async fn deliver_split_chunks_pre_final_round_text() {
        // 2 rounds. Pre-final narration = 200 chars, max chunk = 100 bytes.
        // Final round = short narration, no media. Validates the regression
        // where pre-final round used to raw `send_message` past the byte
        // ceiling on non-streaming channels.
        let plugin_concrete = Arc::new(CountingPlugin::new(100));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            thread_id: None,
            reply_to_message_id: None,
        };
        let pre_final_text = "A".repeat(200);
        let rounds = vec![
            RoundOutput {
                text: pre_final_text.clone(),
                medias: Vec::new(),
            },
            RoundOutput {
                text: "final.".to_string(),
                medias: Vec::new(),
            },
        ];
        let caps = plugin.capabilities();

        let _ = deliver_split(&plugin, &target, &rounds, "fallback", None, 0, &caps).await;

        let sends = plugin_concrete.sends.lock().unwrap().clone();

        // Pre-final 200 bytes / 100 byte ceiling => >=2 chunks.
        // Final 1 chunk. Total >= 3 send_message calls.
        assert!(
            sends.len() >= 3,
            "expected >=3 send_message calls, got {}: {:?}",
            sends.len(),
            sends.iter().map(|s| s.len()).collect::<Vec<_>>()
        );
        for (i, s) in sends.iter().enumerate() {
            assert!(
                s.len() <= 100,
                "chunk {} exceeded 100 bytes: {} bytes",
                i,
                s.len()
            );
        }
        // Concatenated pre-final chunks should reconstruct the original text
        // (chunk_text trims leading whitespace between chunks; our input is
        // pure 'A's so no whitespace effects).
        let prefinal_chunks: String = sends.iter().take(sends.len() - 1).cloned().collect();
        assert_eq!(prefinal_chunks, pre_final_text);
        assert_eq!(sends.last().unwrap(), "final.");
    }
}

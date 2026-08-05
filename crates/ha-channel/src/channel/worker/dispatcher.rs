use anyhow::Context;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use ha_core::channel::db::ChannelDB;
use ha_core::channel::registry::ChannelRegistry;
use ha_core::channel::traits::ChannelPlugin;
use ha_core::channel::types::*;

use super::media::{convert_inbound_media_to_attachments, transcribe_inbound_voice_attachments};
use super::pipeline::{
    abort_pipeline_outcome, await_stream_pipeline_until_cancel, deliver_rounds,
    spawn_stream_pipeline, DeliveryTarget,
};
use super::slash::{dispatch_slash_for_channel, render_slash_button_fallback, ChannelSlashOutcome};
use super::streaming::{
    append_preview_round_text, claim_unopened_native_error, fail_native_preview,
    try_claim_native_final, PreviewHandle, CARD_ELEMENT_MAX_CHARS, NATIVE_ACTIVE, NATIVE_AMBIGUOUS,
    NATIVE_BROKEN, NATIVE_OPENING, NATIVE_SELECTED, NATIVE_TERMINAL,
};

/// Maximum number of inbound messages processed concurrently.
/// Prevents resource exhaustion (DB lock contention, API rate limits) during message bursts.
const MAX_CONCURRENT_INBOUND: usize = 20;
static CHANNEL_DISPATCH_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_INBOUND)));
const CONTROL_DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const CANCELLED_MESSAGE_RETRY_MIN: std::time::Duration = std::time::Duration::from_millis(250);
const CANCELLED_MESSAGE_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(5);

async fn wait_for_channel_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

pub(super) async fn acquire_channel_dispatch_permit() -> OwnedSemaphorePermit {
    CHANNEL_DISPATCH_SEMAPHORE
        .clone()
        .acquire_owned()
        .await
        .expect("Channel dispatch semaphore is never closed")
}

#[allow(clippy::too_many_arguments)]
async fn enqueue_channel_message(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    msg: &mut MsgContext,
    session_db: &Arc<ha_core::session::SessionDB>,
    session_id: &str,
    engine_message: &str,
    user_text: &str,
    cancel: &Arc<AtomicBool>,
    attempt_insert: bool,
) -> anyhow::Result<()> {
    let channel_id = msg.channel_id.to_string();
    let origin = super::turn_queue::ChannelQueuedOrigin::from_message(msg);
    let request_id = origin.request_id();
    let consumed_request = request_id.clone();
    if session_db
        .run(move |db| db.queue_request_was_consumed(&consumed_request))
        .await?
    {
        app_debug!(
            "channel",
            "turn_queue_dedup",
            "Ignoring already-consumed queued IM request {}",
            request_id
        );
        return Ok(());
    }
    let media_result = {
        let hydration = plugin.materialize_pending_media(account, msg);
        tokio::pin!(hydration);
        tokio::select! {
            biased;
            _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
            result = &mut hydration => result,
        }
    };
    if let Err(error) = media_result {
        app_warn!(
            "channel",
            "turn_queue_media",
            "[{}] Failed to materialize queued media {}: {}",
            channel_id,
            msg.message_id,
            error
        );
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    let queued_media = msg.media.clone();
    let sid_for_files = session_id.to_string();
    let request_for_files = request_id.clone();
    let queued_attachments = ha_core::blocking::run_blocking(move || {
        let mut queued_attachments =
            convert_inbound_media_to_attachments(&queued_media, &sid_for_files);
        for attachment in &mut queued_attachments {
            if attachment.source.is_none() {
                attachment.source = Some(ha_core::attachments::CHANNEL_QUEUE_SOURCE.to_string());
            }
        }
        ha_core::attachments::persist_queued_chat_attachments(
            &sid_for_files,
            &request_for_files,
            &mut queued_attachments,
        )?;
        anyhow::Ok(queued_attachments)
    })
    .await?;
    let cleanup_attachments = queued_attachments.clone();
    if cancel.load(Ordering::Acquire) {
        ha_core::attachments::remove_discarded_queued_attachments(
            session_id,
            &request_id,
            &cleanup_attachments,
        );
        return Ok(());
    }
    let queue_input = ha_core::session::NewQueuedTurnMessage {
        request_id: request_id.clone(),
        session_id: session_id.to_string(),
        message: engine_message.to_string(),
        display_text: Some(user_text.to_string()),
        attachments: queued_attachments,
        is_plan_trigger: false,
        goal_trigger: false,
        plan_comment: None,
        plan_mode: None,
        workflow_mode: None,
        source: ha_core::session::QueuedTurnMessageSource::Channel,
        channel_origin: Some(serde_json::to_value(origin)?),
    };
    let enqueue = session_db
        .run(move |db| db.enqueue_turn_user_message(queue_input))
        .await;
    let outcome = match enqueue {
        Ok(outcome) => outcome,
        Err(error) => {
            ha_core::attachments::remove_discarded_queued_attachments(
                session_id,
                &request_id,
                &cleanup_attachments,
            );
            let target = DeliveryTarget {
                account_id: &account.id,
                chat_id: &msg.chat_id,
                chat_type: &msg.chat_type,
                thread_id: msg.thread_id.as_deref(),
                reply_to_message_id: Some(&msg.message_id),
                recipient_user_id: Some(&msg.sender_id),
                recipient_tenant_id: msg.sender_tenant_id.as_deref(),
            };
            let notice = format!("⚠️ I couldn't queue this message: {error}");
            let _ = send_text_chunks(plugin, &target, &notice, None, &[]).await;
            return Ok(());
        }
    };
    if !outcome.inserted {
        ha_core::attachments::remove_discarded_queued_attachments(
            session_id,
            &request_id,
            &cleanup_attachments,
        );
    }

    // Close the enqueue-vs-Stop race. Either shared Stop sees this row in its
    // DB transition, or this post-insert check observes the cancel flag and
    // moves it to the same held state before any backend pump is kicked.
    if cancel.load(Ordering::Acquire) {
        let stopped_session_id = session_id.to_string();
        session_db
            .run(move |db| db.hold_channel_turn_messages_after_stop(&stopped_session_id))
            .await?;
        return Ok(());
    }

    if attempt_insert {
        if let Some(active) = ha_core::chat_engine::active_turn::current(session_id) {
            let sid = session_id.to_string();
            let request = request_id.clone();
            let active_turn_id = active.turn_id.clone();
            let _ = session_db
                .run(move |db| {
                    ha_core::chat_engine::turn_injection::request_channel_insertion(
                        db,
                        &sid,
                        &active_turn_id,
                        &request,
                    )
                })
                .await;
        }
    }
    super::turn_queue::kick(session_id);
    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id: &msg.chat_id,
        chat_type: &msg.chat_type,
        thread_id: msg.thread_id.as_deref(),
        reply_to_message_id: Some(&msg.message_id),
        recipient_user_id: Some(&msg.sender_id),
        recipient_tenant_id: msg.sender_tenant_id.as_deref(),
    };
    let delivery = send_text_chunks(
        plugin,
        &target,
        "⏳ Message queued. I'll add it at the next safe tool boundary, or continue with it after the current reply.",
        None,
        &[],
    );
    tokio::pin!(delivery);
    tokio::select! {
        biased;
        _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
        result = tokio::time::timeout(CONTROL_DELIVERY_TIMEOUT, &mut delivery) => {
            if result.is_err() {
                app_warn!(
                    "channel",
                    "turn_queue_ack",
                    "Timed out delivering queue acknowledgement for session {}",
                    session_id
                );
            }
        }
    }
    Ok(())
}

struct ChannelCancelHandleGuard {
    session_id: String,
    registration_id: Option<String>,
}

impl Drop for ChannelCancelHandleGuard {
    fn drop(&mut self) {
        if let (Some(registry), Some(registration_id)) = (
            ha_core::globals::get_channel_cancels(),
            self.registration_id.as_deref(),
        ) {
            registry.remove(&self.session_id, registration_id);
        }
    }
}

struct ChannelTurnInsertionGuard {
    session_id: String,
    turn_id: String,
}

impl Drop for ChannelTurnInsertionGuard {
    fn drop(&mut self) {
        ha_core::chat_engine::turn_injection::clear_turn(&self.session_id, &self.turn_id);
    }
}

fn is_stop_command(text: Option<&str>) -> bool {
    text.and_then(|text| ha_core::slash_defs::parser::parse(text).ok())
        .is_some_and(|(name, _)| name == "stop")
}

fn spawn_cancelled_channel_message_rollback(
    session_db: Arc<ha_core::session::SessionDB>,
    session_id: String,
    message_id: i64,
) -> tokio::task::JoinHandle<()> {
    // Install the replacement gate synchronously while the cancelled active
    // turn still owns admission, then transfer it to retry cleanup. A transient
    // SQLite failure must never expose the stale prompt to a later inbound.
    let cleanup_gate = ha_core::chat_engine::active_turn::begin_stop_cleanup(&session_id);
    tokio::spawn(async move {
        let mut retry_delay = CANCELLED_MESSAGE_RETRY_MIN;
        let mut attempt = 0_u64;
        loop {
            attempt = attempt.saturating_add(1);
            let result = session_db
                .run(move |db| db.delete_message_by_id(message_id))
                .await;
            match result {
                Ok(()) => {
                    if attempt > 1 {
                        app_info!(
                            "channel",
                            "cancelled_message_rollback",
                            "Rolled back cancelled Channel message after retry: session={} message_id={} attempts={}",
                            session_id,
                            message_id,
                            attempt
                        );
                    }
                    emit_channel_update(&session_id);
                    drop(cleanup_gate);
                    return;
                }
                Err(error) => {
                    if attempt == 1 || attempt.is_power_of_two() {
                        let message = ha_core::logging::redact_sensitive(&error.to_string());
                        app_warn!(
                            "channel",
                            "cancelled_message_rollback",
                            "Failed to roll back cancelled Channel message; retaining Stop gate and retrying: session={} message_id={} attempt={} error={}",
                            session_id,
                            message_id,
                            attempt,
                            message
                        );
                    }
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = retry_delay
                        .saturating_mul(2)
                        .min(CANCELLED_MESSAGE_RETRY_MAX);
                }
            }
        }
    })
}

/// Notify the frontend that a channel session has new messages.
pub(super) fn emit_channel_update(session_id: &str) {
    if let Some(bus) = ha_core::get_event_bus() {
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
    if let Some(bus) = ha_core::get_event_bus() {
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
                super::turn_queue::recover_all();

                while let Some(event) = inbound_rx.recv().await {
                    match event {
                        InboundEvent::Message(msg) => {
                            let registry = registry.clone();
                            let channel_db = channel_db.clone();
                            let permit = acquire_channel_dispatch_permit().await;

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
    msg: MsgContext,
) -> anyhow::Result<()> {
    handle_inbound_message_inner(registry, channel_db, msg, None).await
}

async fn discard_unroutable_channel_queue_record(
    record: ha_core::session::QueuedTurnMessageRecord,
    reason: &'static str,
) -> anyhow::Result<()> {
    let session_db =
        ha_core::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))?;
    let session_id = record.session_id.clone();
    let request_id = record.request_id.clone();
    session_db
        .run(move |db| {
            // A malformed / no-longer-authorized row must never remain at the
            // FIFO head merely because writing its diagnostic event failed.
            db.remove_claimed_turn_message(&session_id, &request_id)?;
            let notice = ha_core::session::NewMessage::error_event(reason)
                .with_source(ha_core::chat_engine::ChatSource::Channel);
            let _ = db.append_message(&session_id, &notice);
            anyhow::Ok(())
        })
        .await?;
    Ok(())
}

pub(super) async fn handle_queued_channel_message(
    record: ha_core::session::QueuedTurnMessageRecord,
) -> anyhow::Result<()> {
    let origin = record
        .channel_origin
        .clone()
        .ok_or_else(|| anyhow::anyhow!("queued Channel row has no routing origin"))
        .and_then(|value| {
            serde_json::from_value::<super::turn_queue::ChannelQueuedOrigin>(value)
                .context("invalid queued Channel routing origin")
        });
    let mut msg = match origin.and_then(super::turn_queue::ChannelQueuedOrigin::into_message) {
        Ok(msg) => msg,
        Err(error) => {
            app_warn!(
                "channel",
                "turn_queue_origin",
                "Discarding invalid queued IM routing envelope for session {}: {}",
                record.session_id,
                ha_core::logging::redact_sensitive(&error.to_string())
            );
            discard_unroutable_channel_queue_record(
                record,
                "⚠️ A queued IM message could not be processed because its routing data was invalid.",
            )
            .await?;
            return Ok(());
        }
    };
    msg.text = Some(
        record
            .display_text
            .clone()
            .unwrap_or_else(|| record.message.clone()),
    );
    let registry = ha_core::globals::get_channel_registry()
        .ok_or_else(|| anyhow::anyhow!("Channel registry not initialized"))?;
    let channel_db = ha_core::globals::get_channel_db()
        .ok_or_else(|| anyhow::anyhow!("Channel DB not initialized"))?;
    handle_inbound_message_inner(registry, channel_db, msg, Some(record)).await
}

async fn handle_inbound_message_inner(
    registry: &ChannelRegistry,
    channel_db: &ChannelDB,
    mut msg: MsgContext,
    queued_record: Option<ha_core::session::QueuedTurnMessageRecord>,
) -> anyhow::Result<()> {
    let is_queued_replay = queued_record.is_some();
    // Capture before approval/ask-user routing performs its first await. A
    // process-wide Stop that runs during those preludes must still reject this
    // inbound if it later reaches active-turn registration after the bounded
    // global cleanup gate has already closed.
    let foreground_admission = ha_core::chat_engine::active_turn::begin_foreground_request();
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
        ha_core::truncate_utf8(msg.text.as_deref().unwrap_or("(media)"), 100)
    );

    // `/stop` is control-plane input, never an approval answer or a custom
    // ask_user response. In particular, a free-form question used to consume
    // the literal "/stop" as its answer and leave the turn running.
    let is_stop_command = !is_queued_replay && is_stop_command(msg.text.as_deref());

    // 0. Check if this message is a text-reply to a pending approval prompt
    if !is_queued_replay
        && !is_stop_command
        && super::approval::try_handle_approval_reply(&msg).await
    {
        app_info!(
            "channel",
            "worker",
            "[{}] Message consumed as approval reply from {}",
            channel_id_str,
            sender_label
        );
        return Ok(());
    }
    // 0a. Not an approval reply, but a text-mode approval is still pending in
    // this chat — nudge the user once per minute so they don't accidentally
    // start a side conversation while the prompt is still open.
    if !is_queued_replay && !is_stop_command {
        super::approval::maybe_send_pending_hint(&msg, registry).await;
    }

    // 0b. Check if this message is a text-reply to a pending ask_user_question
    if !is_queued_replay
        && !is_stop_command
        && super::ask_user::try_handle_ask_user_reply(&msg).await
    {
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
    let store = ha_core::config::cached_config();
    app_debug!(
        "channel",
        "worker",
        "Config loaded: {} channel accounts, looking for '{}'",
        store.channels.accounts.len(),
        msg.account_id
    );
    let account = match store.channels.find_account(&msg.account_id) {
        Some(account) => account.clone(),
        None if queued_record.is_some() => {
            discard_unroutable_channel_queue_record(
                queued_record.expect("checked queued record"),
                "⚠️ A queued IM message was discarded because its Channel account no longer exists.",
            )
            .await?;
            return Ok(());
        }
        None => {
            return Err(anyhow::anyhow!(
                "Account '{}' not found in config",
                msg.account_id
            ));
        }
    };

    // 2. Check access control
    let plugin = match registry.get_plugin(&msg.channel_id) {
        Some(plugin) => plugin.clone(),
        None if queued_record.is_some() => {
            discard_unroutable_channel_queue_record(
                queued_record.expect("checked queued record"),
                "⚠️ A queued IM message was discarded because its Channel plugin is unavailable.",
            )
            .await?;
            return Ok(());
        }
        None => {
            return Err(anyhow::anyhow!("No plugin for channel: {}", msg.channel_id));
        }
    };

    if !plugin.check_access(&account, &msg) {
        app_warn!(
            "channel",
            "worker",
            "[{}] Access denied for sender {} in {}",
            channel_id_str,
            msg.sender_id,
            msg.chat_id
        );
        if let Some(record) = queued_record {
            discard_unroutable_channel_queue_record(
                record,
                "⚠️ A queued IM message was discarded because Channel access is no longer allowed.",
            )
            .await?;
        }
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
            if let Some(record) = queued_record {
                discard_unroutable_channel_queue_record(
                    record,
                    "⚠️ A queued IM message was discarded because the mention policy changed.",
                )
                .await?;
            }
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
            if let Some(record) = queued_record {
                discard_unroutable_channel_queue_record(
                    record,
                    "⚠️ A queued IM message was discarded because the mention policy changed.",
                )
                .await?;
            }
            return Ok(());
        }
    }

    // 3. Resolve agent_id via the central resolver — the precedence chain
    //    (project > topic > group > channel-override > channel-account >
    //    global > hardcoded) lives in `agent::resolver` so /status, IM
    //    dispatch, and desktop / HTTP all share one source of truth.
    //    Only the IM-relevant levels are passed in here; project routing
    //    is now explicit (`/project <id>` from inside the chat).
    let (agent_id, _agent_source) = match msg.chat_type {
        ChatType::Group | ChatType::Forum => {
            ha_core::agent::resolver::resolve_default_agent_id_full(
                None,
                None,
                topic_config,
                effective_group_config,
                None,
                Some(&account),
            )
        }
        ChatType::Channel => ha_core::agent::resolver::resolve_default_agent_id_full(
            None,
            None,
            None,
            None,
            channel_config,
            Some(&account),
        ),
        ChatType::Dm => ha_core::agent::resolver::resolve_default_agent_id_full(
            None,
            None,
            None,
            None,
            None,
            Some(&account),
        ),
    };

    // Hold lifecycle admission before the channel mapping can materialize a
    // session for this Agent. `run_chat_engine` acquires a second guard as a
    // shared backstop once the turn reaches the engine.
    let _agent_admission = ha_core::agent_lifecycle::begin_agent_run(&agent_id)?;

    // 3b. Resolve extra system prompt from group/topic/channel config
    let config_system_prompt = match msg.chat_type {
        ChatType::Group | ChatType::Forum => topic_config
            .and_then(|t| t.system_prompt.as_deref())
            .or_else(|| effective_group_config.and_then(|g| g.system_prompt.as_deref())),
        ChatType::Channel => channel_config.and_then(|c| c.system_prompt.as_deref()),
        ChatType::Dm => None,
    };

    let session_db =
        ha_core::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))?;
    let session_id = if let Some(record) = queued_record.as_ref() {
        let channel_db_for_lookup = channel_db.clone();
        let lookup_session_id = record.session_id.clone();
        let attach_matches = session_db
            .run(move |_| channel_db_for_lookup.get_conversation_by_session(&lookup_session_id))
            .await?
            .is_some_and(|conversation| {
                conversation.channel_id == channel_id_str
                    && conversation.account_id == msg.account_id
                    && conversation.chat_id == msg.chat_id
                    && conversation.thread_id == msg.thread_id
            });
        if !attach_matches {
            let remove_session_id = record.session_id.clone();
            let remove_request_id = record.request_id.clone();
            let _ = session_db
                .run(move |db| {
                    db.remove_claimed_turn_message(&remove_session_id, &remove_request_id)
                })
                .await;
            let target = DeliveryTarget {
                account_id: &account.id,
                chat_id: &msg.chat_id,
                chat_type: &msg.chat_type,
                thread_id: msg.thread_id.as_deref(),
                reply_to_message_id: Some(&msg.message_id),
                recipient_user_id: Some(&msg.sender_id),
                recipient_tenant_id: msg.sender_tenant_id.as_deref(),
            };
            let _ = send_text_chunks(
                &plugin,
                &target,
                "⚠️ This queued message was not processed because the conversation was attached elsewhere. Please send it again.",
                None,
                &[],
            )
            .await;
            return Ok(());
        }
        record.session_id.clone()
    } else {
        channel_db.resolve_or_create_session(
            &channel_id_str,
            &msg.account_id,
            &msg.chat_id,
            msg.thread_id.as_deref(),
            Some(&msg.sender_id),
            msg.sender_name.as_deref(),
            &msg.chat_type,
            &agent_id,
        )?
    };

    // 4. Prepare inbound text. Reply-only slash commands (e.g. /status)
    // are persisted as event history below, but never as user turns and
    // never into model-facing context.
    let user_text = msg
        .text
        .clone()
        .unwrap_or_else(|| "(media message)".to_string());

    // Register before slash dispatch: commands such as /compact can execute
    // long-running work before the chat engine exists. `/stop` deliberately
    // does not register itself; it cancels every other registration owned by
    // the attached session through the shared Stop service.
    let channel_cancel_registration = if is_stop_command {
        None
    } else {
        ha_core::globals::get_channel_cancels().map(|registry| registry.register(&session_id))
    };
    let channel_cancel_registration_id = channel_cancel_registration
        .as_ref()
        .map(|registration| registration.id.clone());
    let cancel = channel_cancel_registration
        .as_ref()
        .map(|registration| registration.cancel.clone())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let _cancel_handle_guard = ChannelCancelHandleGuard {
        session_id: session_id.clone(),
        registration_id: channel_cancel_registration.map(|registration| registration.id),
    };

    // 5a. Intercept slash commands — dispatch and send reply directly, skip LLM.
    // For PassThrough commands (e.g. skill invocations), use the transformed message as the
    // engine input so the LLM receives the skill instruction rather than the raw "/" text.
    let engine_message: String;
    if let Some(record) = queued_record.as_ref() {
        // Slash/control routing happened on original receipt. Replaying it
        // would repeat side effects; only the already-normalized model input
        // is allowed through the durable queue.
        engine_message = record.message.clone();
    } else if ha_core::slash_defs::parser::is_command(&user_text) {
        // Channels without inline-button support get the handler's verbose
        // no-arg text response instead of the (un-tappable) `Select an
        // option for /xxx:` shortcut.
        let supports_buttons = plugin.supports_reply_buttons(&account.id, &msg.chat_id);
        let slash_dispatch = dispatch_slash_for_channel(
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
            &user_text,
            &msg.sender_id,
            supports_buttons,
        );
        tokio::pin!(slash_dispatch);
        let slash_outcome = tokio::select! {
            biased;
            _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
            outcome = &mut slash_dispatch => outcome,
        };
        match slash_outcome {
            Ok(ChannelSlashOutcome::Reply {
                mut content,
                new_session_id,
                mut buttons,
            }) => {
                if let Err(error) = plugin.validate_reply_buttons(&buttons) {
                    app_warn!(
                        "channel",
                        "worker",
                        "Slash picker failed provider button preflight; using text options: {}",
                        ha_core::logging::redact_sensitive(&error.to_string())
                    );
                    content = render_slash_button_fallback(&content, &buttons);
                    buttons.clear();
                }
                let effective_sid = new_session_id.as_deref().unwrap_or(&session_id);
                if new_session_id.is_none() {
                    let history_db = session_db.clone();
                    let history_sid = effective_sid.to_string();
                    let history_command = user_text.clone();
                    let history_reply = content.clone();
                    let persistence = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        history_db.run(move |db| {
                            ha_core::slash_defs::append_slash_history_events(
                                db,
                                &history_sid,
                                &history_command,
                                Some(&history_reply),
                                ha_core::chat_engine::ChatSource::Channel,
                            )
                        }),
                    );
                    tokio::pin!(persistence);
                    match tokio::select! {
                        biased;
                        _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
                        result = &mut persistence => result,
                    } {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => app_warn!(
                            "channel",
                            "worker",
                            "Failed to persist slash command history: {}",
                            error
                        ),
                        Err(_) => app_warn!(
                            "channel",
                            "worker",
                            "Timed out persisting slash command history for session {}",
                            effective_sid
                        ),
                    }
                }
                let slash_target = DeliveryTarget {
                    account_id: &account.id,
                    chat_id: &msg.chat_id,
                    chat_type: &msg.chat_type,
                    thread_id: msg.thread_id.as_deref(),
                    reply_to_message_id: Some(&msg.message_id),
                    recipient_user_id: Some(&msg.sender_id),
                    recipient_tenant_id: msg.sender_tenant_id.as_deref(),
                };
                let delivery = send_text_chunks(&plugin, &slash_target, &content, None, &buttons);
                tokio::pin!(delivery);
                tokio::select! {
                    biased;
                    _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
                    result = tokio::time::timeout(CONTROL_DELIVERY_TIMEOUT, &mut delivery) => {
                        match result {
                            Ok(report) if !report.is_success() => app_warn!(
                                "channel",
                                "worker",
                                "Slash reply delivery was incomplete for session {}: {} failure(s)",
                                effective_sid,
                                report.failures.len()
                            ),
                            Ok(_) => {}
                            Err(_) => app_warn!("channel", "worker", "Timed out delivering slash reply for session {}", effective_sid),
                        }
                    }
                }
                emit_channel_update(effective_sid);
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
                    chat_type: &msg.chat_type,
                    thread_id: msg.thread_id.as_deref(),
                    reply_to_message_id: Some(&msg.message_id),
                    recipient_user_id: Some(&msg.sender_id),
                    recipient_tenant_id: msg.sender_tenant_id.as_deref(),
                };
                let delivery = send_text_chunks(&plugin, &err_target, &error_reply, None, &[]);
                tokio::pin!(delivery);
                tokio::select! {
                    biased;
                    _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
                    result = tokio::time::timeout(CONTROL_DELIVERY_TIMEOUT, &mut delivery) => {
                        if result.is_err() {
                            app_warn!("channel", "worker", "Timed out delivering slash error for session {}", session_id);
                        }
                    }
                }
                return Ok(());
            }
        }
    } else {
        engine_message = user_text.clone();
    }

    let resumed_held_messages = if !is_queued_replay {
        let sid = session_id.clone();
        let registration_id = channel_cancel_registration_id.clone();
        let cancel_registry = ha_core::globals::get_channel_cancels().cloned();
        session_db
            .run(move |db| {
                let Some((registry, registration_id)) =
                    cancel_registry.as_ref().zip(registration_id.as_deref())
                else {
                    return Ok(0);
                };
                registry
                    .with_registration_if_live(&sid, registration_id, || {
                        db.resume_channel_turn_messages_after_stop(&sid)
                    })
                    .transpose()
                    .map(|count| count.unwrap_or(0))
            })
            .await?
            > 0
    } else {
        false
    };
    if resumed_held_messages {
        // Resume is itself a durable state transition. Kick immediately so
        // the older rows cannot remain stranded if staging the new inbound
        // attachment or appending the tail row later fails.
        super::turn_queue::kick(&session_id);
    }

    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    // An idle session may still have a backend-owned FIFO head between the
    // pump's DB claim and its active-turn admission (especially at restart).
    // A fresh inbound must join behind that durable head rather than starting
    // a newer turn merely because the in-memory active registry is empty.
    if !is_queued_replay && !resumed_held_messages {
        let sid = session_id.clone();
        if session_db
            .run(move |db| db.has_channel_turn_messages(&sid))
            .await?
        {
            enqueue_channel_message(
                &plugin,
                &account,
                &mut msg,
                &session_db,
                &session_id,
                &engine_message,
                &user_text,
                &cancel,
                false,
            )
            .await?;
            return Ok(());
        }
    }

    // Acquire before the user message becomes durable. Besides enforcing
    // single-flight, this is the generation check that prevents an inbound
    // which predates global Stop from leaking into the next turn's context.
    // Queued replays retain their claim turn id; fresh messages mint one here.
    let turn_id = queued_record
        .as_ref()
        .and_then(|record| record.turn_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let _active_turn_guard = match ha_core::chat_engine::active_turn::try_acquire_foreground_request(
        foreground_admission,
        &session_id,
        ha_core::chat_engine::stream_seq::ChatSource::Channel,
        turn_id.clone(),
        None,
        cancel.clone(),
    ) {
        Ok(guard) => guard,
        Err(e) => {
            if is_queued_replay {
                return Err(anyhow::anyhow!("queued IM session is still busy: {e}"));
            }
            if e.cancelled_by_global_stop() {
                app_info!(
                    "channel",
                    "stop",
                    "[{}] discarded inbound for session {} because it predated global Stop",
                    channel_id_str,
                    session_id
                );
                emit_channel_update(&session_id);
                return Ok(());
            }
            app_info!(
                "channel",
                "worker",
                "[{}] inbound for session {} queued: a turn is already active ({})",
                channel_id_str,
                session_id,
                e
            );

            enqueue_channel_message(
                &plugin,
                &account,
                &mut msg,
                &session_db,
                &session_id,
                &engine_message,
                &user_text,
                &cancel,
                !resumed_held_messages,
            )
            .await?;
            return Ok(());
        }
    };
    let _insertion_lifecycle = ChannelTurnInsertionGuard {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
    };
    if cancel.load(Ordering::Acquire) {
        emit_channel_update(&session_id);
        return Ok(());
    }

    if let Some(record) = queued_record.as_ref() {
        let sid = session_id.clone();
        let request_id = record.request_id.clone();
        let claim_turn_id = turn_id.clone();
        let registration_id = channel_cancel_registration_id.clone();
        let cancel_registry = ha_core::globals::get_channel_cancels().cloned();
        let claim_is_live = session_db
            .run(move |db| {
                let Some((registry, registration_id)) =
                    cancel_registry.as_ref().zip(registration_id.as_deref())
                else {
                    return Ok(false);
                };
                registry
                    .with_registration_if_live(&sid, registration_id, || {
                        db.channel_dispatch_claim_is_active(&sid, &request_id, &claim_turn_id)
                    })
                    .transpose()
                    .map(|active| active.unwrap_or(false))
            })
            .await?;
        if !claim_is_live {
            app_info!(
                "channel",
                "turn_queue_stop",
                "Queued IM claim for session {} was held or cancelled before admission",
                session_id
            );
            return Ok(());
        }
    }

    if resumed_held_messages {
        enqueue_channel_message(
            &plugin,
            &account,
            &mut msg,
            &session_db,
            &session_id,
            &engine_message,
            &user_text,
            &cancel,
            false,
        )
        .await?;
        return Ok(());
    }

    // Typing and deferred media hydration are network-facing preparation, so
    // they must be inside the same cancellable lifetime as preflight/engine.
    {
        let typing = plugin.send_typing(&account.id, &msg.chat_id);
        tokio::pin!(typing);
        tokio::select! {
            biased;
            _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
            _ = &mut typing => {}
        }
    }

    if !is_queued_replay {
        let media_result = {
            let hydration = plugin.materialize_pending_media(&account, &mut msg);
            tokio::pin!(hydration);
            tokio::select! {
                biased;
                _ = wait_for_channel_cancel(cancel.as_ref()) => return Ok(()),
                result = &mut hydration => result,
            }
        };
        if let Err(e) = media_result {
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
    }

    // Preflight chokepoint: pass-through in Phase 0.1; PR 1.2 runs the
    // `UserPromptSubmit` hook here. The raw prompt is the persisted `user_text`
    // (not the LLM-bound `engine_message`), keeping transcript + hook input
    // consistent with what lands in history. `turn_id` was minted / recovered
    // at the queue-aware `try_acquire_foreground_request` above so the hook
    // and engine share the same `prompt_id`; do not re-mint here.
    let preflight = ha_core::agent::preflight::user_prompt_preflight_cancellable(
        ha_core::agent::preflight::PreflightArgs {
            session_id: &session_id,
            agent_id: Some(agent_id.as_str()),
            raw_prompt: &user_text,
            turn_id: &turn_id,
        },
        cancel.as_ref(),
    )
    .await;
    let Some(preflight) = preflight else {
        app_info!(
            "channel",
            "stop",
            "Stopped channel prompt during preflight for session {}",
            session_id
        );
        return Ok(());
    };
    let effective_prompt = match preflight {
        ha_core::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
            effective_prompt
        }
        ha_core::agent::preflight::PreflightOutcome::Block { reason } => {
            // A UserPromptSubmit hook blocked the prompt: reply to the chat and
            // record a UI-only event marker (excluded from LLM context); the
            // prompt is neither persisted as a user message nor run. If this
            // was a queue replay, first clear the claimed queue row so it
            // doesn't retry on next idle.
            let notice = format!("🚫 {reason}");
            let blocked_session_id = session_id.clone();
            let blocked_notice = notice.clone();
            let blocked_request_id = queued_record
                .as_ref()
                .map(|record| record.request_id.clone());
            let _ = session_db
                .run(move |db| {
                    if let Some(request_id) = blocked_request_id {
                        db.remove_claimed_turn_message(&blocked_session_id, &request_id)?;
                    }
                    db.append_message(
                        &blocked_session_id,
                        &ha_core::session::NewMessage::event(&blocked_notice)
                            .with_source(ha_core::chat_engine::ChatSource::Channel),
                    )?;
                    anyhow::Ok(())
                })
                .await;
            let target = DeliveryTarget {
                account_id: &account.id,
                chat_id: &msg.chat_id,
                chat_type: &msg.chat_type,
                thread_id: msg.thread_id.as_deref(),
                reply_to_message_id: Some(&msg.message_id),
                recipient_user_id: Some(&msg.sender_id),
                recipient_tenant_id: msg.sender_tenant_id.as_deref(),
            };
            let _ = send_text_chunks(&plugin, &target, &notice, None, &[]).await;
            return Ok(());
        }
    };
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }

    let attachments = if let Some(record) = queued_record.as_ref() {
        record.attachments.clone()
    } else {
        convert_inbound_media_to_attachments(&msg.media, &session_id)
    };
    let attachment_session_id = session_id.clone();
    let (attachments, attachment_meta_result) = ha_core::blocking::run_blocking(move || {
        let mut attachments = attachments;
        let result = ha_core::attachments::persist_chat_user_attachments_meta(
            &attachment_session_id,
            &mut attachments,
        );
        (attachments, result)
    })
    .await;
    let user_attachments_meta = match attachment_meta_result {
        Ok(meta) => meta,
        Err(e) => {
            app_warn!(
                "channel",
                "worker",
                "[{}] Failed to persist inbound attachment metadata for {}: {}",
                channel_id_str,
                session_id,
                e
            );
            None
        }
    };
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut attachments_meta = serde_json::json!({
        "channel_inbound": {
            "channelId": channel_id_str,
            "accountId": msg.account_id,
            "senderId": msg.sender_id,
            "senderName": msg.sender_name,
            "chatId": msg.chat_id,
            "messageId": msg.message_id,
        }
    });
    if let Some(meta_json) = user_attachments_meta.as_deref() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(meta_json) {
            if value.as_array().is_some_and(|items| !items.is_empty()) {
                attachments_meta["user_attachments"] = value;
            }
        }
    }
    let attachments_meta_json = attachments_meta.to_string();
    let mut user_msg = ha_core::session::NewMessage::user(&effective_prompt)
        .with_source(ha_core::chat_engine::ChatSource::Channel);
    user_msg.attachments_meta = Some(attachments_meta_json.clone());
    // Archive is not a mute boundary. A fresh inbound IM message restores the
    // attached conversation so it cannot continue accumulating out of sight.
    // Keep all synchronous SessionDB work off this async dispatcher's Tokio
    // workers and preserve restore-before-append ordering in one blocking task.
    let persist_session_id = session_id.clone();
    let persist_turn_id = turn_id.clone();
    let queued_for_persist = queued_record.clone();
    let persist_result = session_db
        .run(move |db| {
            ha_core::chat_engine::active_turn::with_persistence_target(
                &persist_session_id,
                &persist_turn_id,
                || -> anyhow::Result<i64> {
                    db.set_session_archived(&persist_session_id, false)?;
                    if let Some(record) = queued_for_persist.as_ref() {
                        db.complete_inserted_turn_message(record, &user_msg)
                    } else {
                        db.append_message(&persist_session_id, &user_msg)
                    }
                },
            )
        })
        .await;
    let _user_message_id = match persist_result {
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::Committed(message_id)) => {
            message_id
        }
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::CommittedAfterCancel(
            message_id,
        )) => {
            if is_queued_replay {
                // The backend queue pump had already promoted this row to the
                // active user turn. A concurrent /stop owns the cancellation;
                // keep the visible user message consumed instead of silently
                // replaying it after the user explicitly stopped the turn.
                emit_channel_update(&session_id);
            } else {
                spawn_cancelled_channel_message_rollback(
                    session_db.clone(),
                    session_id.clone(),
                    message_id,
                );
            }
            return Ok(());
        }
        Ok(ha_core::chat_engine::active_turn::PersistenceTargetOutcome::CancelledBeforeCommit) => {
            emit_channel_update(&session_id);
            return Ok(());
        }
        Err(error) => {
            app_warn!(
                "channel",
                "worker",
                "[{}] Failed to restore/persist inbound conversation {}: {}",
                channel_id_str,
                session_id,
                error
            );
            return Err(error);
        }
    };

    // Auto-generate fallback title only after persistence has won over Stop;
    // a cancelled stale inbound must not leave either a message or its title.
    let title_session_id = session_id.clone();
    let title_prompt = effective_prompt.clone();
    let title_attachments_meta = attachments_meta_json.clone();
    let _ = session_db
        .run(move |db| {
            ha_core::session::ensure_first_message_title(
                db,
                &title_session_id,
                &title_prompt,
                Some(&title_attachments_meta),
            )
        })
        .await;

    // Notify the desktop / web side that a fresh user message landed on
    // this session from IM, so an attached GUI view can pull it into
    // the conversation timeline without waiting for the stream-start
    // round-trip. `channel:stream_start` covers the assistant side a
    // moment later — this event is purely about the inbound user turn.
    if let Some(bus) = ha_core::globals::get_event_bus() {
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
                "attachments_meta": attachments_meta_json,
            }),
        );
    }

    if cancel.load(Ordering::Acquire) {
        emit_channel_update(&session_id);
        return Ok(());
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
    let agent_def = ha_core::agent_loader::load_agent(&agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|d| d.config.model.clone())
        .unwrap_or_default();

    // Session-scoped model pin — IM `/model` writes sessions.provider_id/model_id;
    // we have to read it back here so the next inbound message actually uses the
    // pinned model. Mirrors the same `session_pinned_model` injection in
    // src-tauri/src/commands/chat.rs and crates/ha-server/src/routes/chat.rs.
    let runtime_defaults =
        ha_core::session::ensure_session_runtime_defaults(&session_db, &session_id)?;
    let session_pinned_model = runtime_defaults
        .preferred_model
        .as_ref()
        .map(|model| format!("{}::{}", model.provider_id, model.model_id));
    let (primary, fallbacks) = ha_core::provider::resolve_model_chain_with_preferred(
        session_pinned_model.as_deref(),
        &agent_model_config,
        &store,
    );
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

    let resolved_temperature = runtime_defaults.temperature;

    // 8a. Auto-transcribe voice / audio attachments when the account opts
    // in. The prefix gets prepended to the engine message so the LLM sees
    // a text version of the spoken content; the original audio is kept as
    // an attachment so multimodal models (or downstream re-transcribe)
    // can still fall back to listening. Failure is non-blocking — logged
    // and dropped.
    let engine_message = if account.auto_transcribe_voice() {
        let transcription = transcribe_inbound_voice_attachments(&attachments, &store.language);
        tokio::pin!(transcription);
        let prefix = tokio::select! {
            biased;
            _ = wait_for_channel_cancel(cancel.as_ref()) => {
                emit_channel_update(&session_id);
                return Ok(());
            }
            prefix = &mut transcription => prefix,
        };
        match prefix {
            Some(prefix) => format!("{}{}", prefix, engine_message),
            None => engine_message,
        }
    } else {
        engine_message
    };

    // 8. Spawn the shared streaming pipeline (preview task + sink) only after
    // cancellable preparation finishes. The engine writes events into
    // `pipeline.event_sink`; we await it and deliver rounds after the engine.
    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id: &msg.chat_id,
        chat_type: &msg.chat_type,
        thread_id: msg.thread_id.as_deref(),
        reply_to_message_id: Some(msg.message_id.as_str()),
        recipient_user_id: Some(&msg.sender_id),
        recipient_tenant_id: msg.sender_tenant_id.as_deref(),
    };
    // Inbound IM turns broadcast on `channel:stream_delta` so the GUI can
    // mirror the IM session live.
    let pipeline = spawn_stream_pipeline(&plugin, &account, &session_id, &target, true, true);
    let event_sink = pipeline.event_sink.clone();
    let reasoning_effort = Some(runtime_defaults.reasoning_effort);

    // Snapshot whether the *entire* fallback chain is Codex before
    // `model_chain` is moved into engine_params. Drives the `🔐 Codex
    // session expired` headline in the error path below.
    //
    // Conservative `all` rather than `primary-only`: engine returns
    // `Result<_, String>` and erases which model in the chain actually
    // failed. With a mixed chain (e.g. OpenAI primary +
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

    let engine_params = ha_core::chat_engine::ChatEngineParams {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        turn_id: None,
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
        // Shared with the single-flight guard above (registered once in the
        // channel cancel registry); removal is handled by `_cancel_handle_guard`.
        cancel: cancel.clone(),
        plan_context_override: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        tool_scope: None,
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: account.auto_approve_tools,
        follow_global_reasoning_effort: false,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: ha_core::chat_engine::stream_seq::ChatSource::Channel,
        ui_surface: None,
        origin_source: None,
        // WS8: carry the IM origin identity so `effective_kb_access` can apply the
        // per-account (+ per-group-chat) KB opt-in. `is_group` = any non-DM chat
        // (group / forum / broadcast channel), which needs separate per-chat
        // confirmation on top of the account opt-in.
        channel_kb_context: Some(ha_core::knowledge::ChannelKbContext {
            channel_id: channel_id_str.clone(),
            account_id: account.id.clone(),
            chat_id: msg.chat_id.clone(),
            is_group: !matches!(msg.chat_type, ChatType::Dm),
        }),
        event_sink,
    };

    emit_stream_lifecycle("channel:stream_start", &session_id);

    let result = ha_core::chat_engine::run_chat_engine(engine_params).await;

    // (channel cancel handle removal now happens via `_cancel_handle_guard` on
    // every exit path, including early bails above.)

    // Late async tool completions arriving after this drain are deferred to
    // a future turn — a stale attachment from turn N must not leak into N+1.
    let Some(outcome) = await_stream_pipeline_until_cancel(pipeline, cancel.as_ref()).await else {
        app_info!(
            "channel",
            "stop",
            "Stopped channel preview finalization for session {}",
            session_id
        );
        emit_stream_lifecycle("channel:stream_end", &session_id);
        return Ok(());
    };

    match result {
        Ok(engine_result) => {
            let delivery = deliver_rounds(&plugin, &target, &outcome, &engine_result.response);
            tokio::pin!(delivery);
            let metrics = tokio::select! {
                biased;
                _ = wait_for_channel_cancel(cancel.as_ref()) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        abort_pipeline_outcome(&outcome, ReplyAbortReason::Cancelled),
                    ).await;
                    app_info!(
                        "channel",
                        "stop",
                        "Stopped channel final delivery for session {}",
                        session_id
                    );
                    emit_stream_lifecycle("channel:stream_end", &session_id);
                    return Ok(());
                }
                metrics = &mut delivery => metrics,
            };

            if metrics.report.is_success() {
                app_info!(
                    "channel",
                    "worker",
                    "[{}] Reply delivered to {} (mode={}, rounds={}, finalized_inline={}, text_chars={}, media={}, preview={}, sends={})",
                    channel_id_str,
                    msg.chat_id,
                    outcome.reply_mode.as_str(),
                    outcome.drained_rounds.len(),
                    outcome.stream_outcome.finalized_rounds,
                    metrics.text_chars,
                    metrics.media_count,
                    outcome.preview_active,
                    metrics.report.succeeded,
                );
            } else {
                let warn_context = format!(
                    "[{}] Reply generated but delivery failed for session {}",
                    channel_id_str, session_id,
                );
                super::report_delivery_failure(
                    &session_db,
                    &session_id,
                    &warn_context,
                    "⚠️ The assistant reply was generated, but IM delivery failed or was incomplete. Check the Channel logs and resend if needed.",
                    &metrics.report,
                )
                .await;
            }
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
            let reason = ha_core::failover::classify_error(&raw);
            let is_codex_auth =
                matches!(reason, ha_core::failover::FailoverReason::Auth) && chain_is_all_codex;
            let body = ha_core::chat_engine::im_error_message::format_im_engine_error(
                ha_core::chat_engine::im_error_message::ImErrorContext {
                    reason,
                    raw: &raw,
                    is_codex_auth,
                },
            );
            let err_target = DeliveryTarget {
                account_id: &account.id,
                chat_id: &msg.chat_id,
                chat_type: &msg.chat_type,
                thread_id: msg.thread_id.as_deref(),
                reply_to_message_id: Some(&msg.message_id),
                recipient_user_id: Some(&msg.sender_id),
                recipient_tenant_id: msg.sender_tenant_id.as_deref(),
            };
            let error_delivery =
                super::pipeline::deliver_error_reply(&plugin, &err_target, &outcome, &body);
            tokio::pin!(error_delivery);
            tokio::select! {
                biased;
                _ = wait_for_channel_cancel(cancel.as_ref()) => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        abort_pipeline_outcome(&outcome, ReplyAbortReason::Cancelled),
                    ).await;
                    app_info!(
                        "channel",
                        "stop",
                        "Stopped channel error delivery for session {}",
                        session_id
                    );
                    emit_stream_lifecycle("channel:stream_end", &session_id);
                    return Ok(());
                }
                _ = &mut error_delivery => {}
            }
        }
    }

    // Notify frontend that streaming ended (triggers DB reload in frontend)
    emit_stream_lifecycle("channel:stream_end", &session_id);

    Ok(())
}

/// Max number of media items attempted as standalone legacy provider sends in
/// one IM turn. The remaining ordered suffix is retained as link fallback.
const MAX_MEDIA_PER_TURN: usize = 5;

/// Hard safety ceiling for provider-native rich replies. Individual adapters
/// advertise a lower/equal budget through `max_embedded_media_items`; keeping
/// a common ceiling prevents a bad capability declaration from turning one
/// model turn into an unbounded upload batch.
const MAX_NATIVE_MEDIA_PER_TURN: usize = 50;

fn native_media_budget(caps: &NativeReplyCapabilities) -> usize {
    caps.max_embedded_media_items
        .map(usize::from)
        .unwrap_or(0)
        .min(MAX_NATIVE_MEDIA_PER_TURN)
}

/// Hard-limit text appended to the final reply when the channel can't
/// deliver a media item natively (LINE/IRC without public URL, unsupported
/// MIME). Each line: `📎 name — <url>` (or "unavailable" when no public URL
/// is configured).
fn build_media_fallback_lines(items: &[&ha_core::attachments::MediaItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    lines.push("📎 Attachments:".to_string());
    for it in items {
        let link = public_attachment_url(it, false)
            .unwrap_or_else(|| "(no public link configured)".to_string());
        lines.push(format!("- {}: {}", it.name, link));
    }
    Some(lines.join("\n"))
}

/// Map a `MediaItem` to `MediaType` based on MIME/kind. Unknown MIMEs fall
/// back to `Document` — a safe default supported by most channels.
fn classify_media_type(it: &ha_core::attachments::MediaItem) -> MediaType {
    use ha_core::attachments::MediaKind;
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

/// Resolve the provider media type for one legacy attachment without changing
/// its position in the ordered delivery sequence.
fn legacy_native_media_type(
    item: &ha_core::attachments::MediaItem,
    caps: &ChannelCapabilities,
) -> Option<MediaType> {
    let classified = classify_media_type(item);
    if caps.supports_media.contains(&classified) {
        Some(classified)
    } else if classified == MediaType::Animation && caps.supports_media.contains(&MediaType::Photo)
    {
        // Animation → Photo fallback for channels without native GIF support.
        Some(MediaType::Photo)
    } else {
        None
    }
}

fn configured_public_base_url(require_https: bool) -> Option<String> {
    let cfg = ha_core::config::cached_config();
    let trimmed = cfg.server.public_base_url.as_deref()?.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    if require_https && !trimmed.starts_with("https://") {
        return None;
    }
    Some(trimmed.to_string())
}

fn public_attachment_url(
    it: &ha_core::attachments::MediaItem,
    require_https: bool,
) -> Option<String> {
    public_attachment_url_with_base(
        it,
        configured_public_base_url(require_https).as_deref(),
        require_https,
    )
}

fn public_attachment_url_with_base(
    it: &ha_core::attachments::MediaItem,
    public_base: Option<&str>,
    require_https: bool,
) -> Option<String> {
    let logical = it.url.trim();
    if logical.starts_with("https://") {
        return Some(logical.to_string());
    }
    if logical.starts_with("http://") {
        return (!require_https).then(|| logical.to_string());
    }
    if !logical.starts_with('/') {
        return None;
    }
    let base = public_base?.trim_end_matches('/');
    if base.is_empty() || (require_https && !base.starts_with("https://")) {
        return None;
    }
    Some(format!("{}{}", base, logical))
}

fn native_media_requires_public_url(channel_id: &ChannelId) -> bool {
    matches!(channel_id, ChannelId::Line | ChannelId::QqBot)
}

fn native_media_supported_for_target(
    channel_id: &ChannelId,
    chat_id: &str,
    item: &ha_core::attachments::MediaItem,
) -> bool {
    match channel_id {
        ChannelId::Line => public_attachment_url(item, true).is_some(),
        ChannelId::QqBot => {
            (chat_id.starts_with("c2c:") || chat_id.starts_with("group:"))
                && public_attachment_url(item, true).is_some()
        }
        _ => true,
    }
}

/// Build an `OutboundMedia` from a `MediaItem`. Most channels prefer the
/// absolute `local_path` (zero-copy for local-disk delivery). LINE and QQ Bot
/// are URL-only for outbound media, so they use a public HTTPS attachment URL.
fn to_outbound_media(
    it: &ha_core::attachments::MediaItem,
    media_type: MediaType,
    channel_id: &ChannelId,
) -> OutboundMedia {
    let data = if native_media_requires_public_url(channel_id) {
        MediaData::Url(public_attachment_url(it, true).unwrap_or_else(|| it.url.clone()))
    } else {
        match it.local_path.as_deref() {
            Some(p) if !p.is_empty() => MediaData::FilePath(p.to_string()),
            _ => MediaData::Url(it.url.clone()),
        }
    };
    OutboundMedia {
        media_type,
        data,
        caption: it.caption.clone(),
    }
}

enum CardFinalizeOutcome {
    Confirmed,
    Unsafe(String),
}

/// Replace the current preview (if any) with an error reply. Native and card
/// previews retain terminal ownership: if their mutation cannot be confirmed,
/// no fresh message is sent because it could duplicate accepted content.
pub(super) async fn send_error_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    preview: Option<&PreviewHandle>,
    error_text: &str,
) -> DeliveryReport {
    if let Some(native @ PreviewHandle::Native { state, .. }) = preview {
        if claim_unopened_native_error(native) {
            // No provider identity exists: this is the one native-preview state
            // where a standalone error is both necessary and provably safe.
            return send_text_chunks(plugin, target, error_text, None, &[]).await;
        }
        if matches!(
            state.load(Ordering::Acquire),
            NATIVE_OPENING | NATIVE_AMBIGUOUS
        ) {
            let _ = super::streaming::abort_native_preview(native, ReplyAbortReason::Failed).await;
            return DeliveryReport {
                attempted: 1,
                succeeded: 0,
                failures: vec![
                    "native preview identity is opening or ambiguous; standalone error suppressed"
                        .to_string(),
                ],
                unsafe_to_continue: true,
            };
        }
        let confirmed = fail_native_preview(native, error_text).await;
        return DeliveryReport {
            attempted: 1,
            succeeded: usize::from(confirmed),
            failures: (!confirmed)
                .then(|| {
                    "native preview error terminal could not be confirmed; standalone fallback suppressed"
                        .to_string()
                })
                .into_iter()
                .collect(),
            unsafe_to_continue: !confirmed,
        };
    }
    if let Some(PreviewHandle::Card {
        card_id,
        element_id,
        sequence,
        broken,
    }) = preview
    {
        if *broken {
            app_warn!(
                "channel",
                "worker",
                "Suppressing error fallback for an ambiguous visible card"
            );
            return DeliveryReport {
                attempted: 1,
                succeeded: 0,
                failures: vec![
                    "visible card preview is ambiguous; error fallback suppressed".to_string(),
                ],
                unsafe_to_continue: true,
            };
        }
        return match finalize_card_stream(
            plugin,
            target.account_id,
            card_id,
            element_id,
            *sequence,
            error_text,
        )
        .await
        {
            CardFinalizeOutcome::Confirmed => DeliveryReport {
                attempted: 1,
                succeeded: 1,
                failures: Vec::new(),
                unsafe_to_continue: false,
            },
            CardFinalizeOutcome::Unsafe(error) => {
                let error = ha_core::logging::redact_sensitive(&error);
                app_warn!(
                    "channel",
                    "worker",
                    "Unable to finalize visible card with error reply: {}",
                    error
                );
                DeliveryReport {
                    attempted: 1,
                    succeeded: 0,
                    failures: vec![error],
                    unsafe_to_continue: true,
                }
            }
        };
    }
    send_text_chunks(plugin, target, error_text, preview, &[]).await
}

/// Write the full response into the streaming card and close streaming.
/// Once a card is visible, any unconfirmed final update is unsafe to retry as
/// a fresh message. `Unsafe` therefore stops all later text/media/action
/// mutations for the turn; server-side expiry clears the streaming indicator.
async fn finalize_card_stream(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    card_id: &str,
    element_id: &str,
    sequence: i64,
    response: &str,
) -> CardFinalizeOutcome {
    let response_chars = response.chars().count();
    if response_chars > CARD_ELEMENT_MAX_CHARS {
        app_warn!(
            "channel",
            "worker",
            "Final response too large for visible card element ({} chars > {}); suppressing duplicate fallback",
            response_chars,
            CARD_ELEMENT_MAX_CHARS
        );
        return CardFinalizeOutcome::Unsafe(format!(
            "visible card final snapshot has {response_chars} characters; maximum is {CARD_ELEMENT_MAX_CHARS}"
        ));
    }

    if let Err(e) = plugin
        .update_card_element(account_id, card_id, element_id, response, sequence)
        .await
    {
        app_warn!(
            "channel",
            "worker",
            "Final card update outcome is ambiguous (seq={}): {}; suppressing duplicate fallback",
            sequence,
            e
        );
        return CardFinalizeOutcome::Unsafe(format!("update_card_element (seq={sequence}): {e}"));
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

    CardFinalizeOutcome::Confirmed
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
) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    if let Err(error) = plugin.validate_reply_buttons(buttons) {
        let error = ha_core::logging::redact_sensitive(&error.to_string());
        app_warn!(
            "channel",
            "worker",
            "Outbound actions failed provider preflight: {}",
            error
        );
        report.attempted = 1;
        report.failures.push(error);
        report.unsafe_to_continue = true;
        return report;
    }
    let native_text = plugin.markdown_to_native(response);
    let chunks = plugin.chunk_message(&native_text);
    if !response.trim().is_empty() && chunks.is_empty() {
        report.attempted = 1;
        report
            .failures
            .push("channel produced no outbound message chunks".to_string());
        report.unsafe_to_continue = true;
        return report;
    }
    let last_idx = chunks.len().saturating_sub(1);

    for (i, chunk) in chunks.iter().enumerate() {
        report.attempted += 1;
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
                        Ok(result) if result.success => Ok(result),
                        Ok(result) => {
                            app_warn!(
                                "channel",
                                "worker",
                                "Preview final edit was not acknowledged; suppressing duplicate send: {}",
                                ha_core::logging::redact_sensitive(
                                    &result
                                        .error
                                        .as_deref()
                                        .unwrap_or("platform rejected edit")
                                )
                            );
                            Ok(result)
                        }
                        Err(e) => {
                            app_warn!(
                                "channel",
                                "worker",
                                "Preview final edit outcome is ambiguous; suppressing duplicate send: {}",
                                e
                            );
                            Err(e)
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
                    let error = ha_core::logging::redact_sensitive(
                        &r.error
                            .unwrap_or_else(|| "platform rejected message".to_string()),
                    );
                    app_warn!("channel", "worker", "Send failed: {}", error);
                    report.failures.push(error);
                    report.unsafe_to_continue = true;
                    break;
                } else {
                    report.succeeded += 1;
                }
            }
            Err(e) => {
                let error = ha_core::logging::redact_sensitive(&e.to_string());
                app_error!("channel", "worker", "Send error: {}", error);
                report.failures.push(error);
                report.unsafe_to_continue = true;
                break;
            }
        }
    }
    report
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DeliveryReport {
    pub attempted: usize,
    pub succeeded: usize,
    pub failures: Vec<String>,
    /// The ordered delivery sequence cannot safely continue: either a
    /// non-idempotent outcome is ambiguous or a required prefix/fallback send
    /// failed. Later suffix/media/action sends must stop.
    pub unsafe_to_continue: bool,
}

impl DeliveryReport {
    pub(super) fn merge(&mut self, other: DeliveryReport) {
        self.attempted += other.attempted;
        self.succeeded += other.succeeded;
        self.failures.extend(other.failures);
        self.unsafe_to_continue |= other.unsafe_to_continue;
    }

    pub fn is_success(&self) -> bool {
        self.failures.is_empty() && self.attempted == self.succeeded
    }
}

/// Aggregated counters used by the dispatcher for the post-turn log line.
#[derive(Debug, Default)]
pub(crate) struct DeliveryMetrics {
    pub text_chars: usize,
    pub media_count: usize,
    pub report: DeliveryReport,
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
    rounds: &[ha_core::chat_engine::RoundOutput],
    fallback_response: &str,
    preview: Option<&PreviewHandle>,
    finalized_rounds: usize,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let mut metrics = DeliveryMetrics::default();
    if rounds.is_empty() {
        metrics.report = send_final_reply(
            plugin,
            target,
            fallback_response,
            preview,
            &[],
            &[],
            false,
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
        return metrics;
    }

    let last_idx = remaining.len() - 1;
    for (i, round) in remaining.iter().enumerate() {
        if i == last_idx {
            // The quote marks the turn's first message. When the stream task
            // already shipped round 0 inline (`finalized_rounds > 0`), the
            // trailing round must not stack a second quote — only quote here
            // when nothing was finalized inline (this is the first message).
            let final_target = DeliveryTarget {
                account_id: target.account_id,
                chat_id: target.chat_id,
                chat_type: target.chat_type,
                thread_id: target.thread_id,
                reply_to_message_id: if finalized_rounds == 0 {
                    target.reply_to_message_id
                } else {
                    None
                },
                recipient_user_id: target.recipient_user_id,
                recipient_tenant_id: target.recipient_tenant_id,
            };
            let report = send_final_reply(
                plugin,
                &final_target,
                &round.text,
                preview,
                &round.medias,
                &[],
                false,
                caps,
            )
            .await;
            metrics.report.merge(report);
            metrics.text_chars += round.text.chars().count();
            metrics.media_count += round.medias.len();
        } else {
            // Pre-final round only reaches here on non-streaming channels —
            // streaming channels finalize per-round inline.
            if !round.text.trim().is_empty() {
                let pre_target = DeliveryTarget {
                    account_id: target.account_id,
                    chat_id: target.chat_id,
                    chat_type: target.chat_type,
                    thread_id: target.thread_id,
                    reply_to_message_id: None,
                    recipient_user_id: target.recipient_user_id,
                    recipient_tenant_id: target.recipient_tenant_id,
                };
                let report = send_text_chunks(plugin, &pre_target, &round.text, None, &[]).await;
                let can_continue = !report.unsafe_to_continue;
                metrics.report.merge(report);
                metrics.text_chars += round.text.chars().count();
                if !can_continue {
                    return metrics;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let report = deliver_media_to_chat(plugin, target, &round.medias, caps).await;
            let can_continue = !report.unsafe_to_continue;
            metrics.report.merge(report);
            metrics.media_count += round.medias.len();
            if !can_continue {
                return metrics;
            }
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
    rounds: &[ha_core::chat_engine::RoundOutput],
    fallback_response: &str,
    caps: &ChannelCapabilities,
) -> DeliveryMetrics {
    let final_text: String = rounds
        .last()
        .map(|r| r.text.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback_response.to_string());
    let all_media: Vec<ha_core::attachments::MediaItem> = rounds
        .iter()
        .flat_map(|r| r.medias.iter().cloned())
        .collect();
    let media_count = all_media.len();
    let text_chars = final_text.chars().count();
    let report = send_final_reply(
        plugin,
        target,
        &final_text,
        None,
        &all_media,
        &[],
        true,
        caps,
    )
    .await;
    DeliveryMetrics {
        text_chars,
        media_count,
        report,
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
    rounds: &[ha_core::chat_engine::RoundOutput],
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
    let all_media: Vec<ha_core::attachments::MediaItem> = rounds
        .iter()
        .flat_map(|r| r.medias.iter().cloned())
        .collect();
    let media_count = all_media.len();
    let text_chars = final_text.chars().count();
    let report = send_final_reply(
        plugin,
        target,
        &final_text,
        preview,
        &all_media,
        &[],
        false,
        caps,
    )
    .await;
    DeliveryMetrics {
        text_chars,
        media_count,
        report,
    }
}

pub(super) fn merge_preview_round_texts(rounds: &[ha_core::chat_engine::RoundOutput]) -> String {
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
///   any IM per-send byte ceiling), then close streaming. Once the card is
///   visible, oversize/update failure is terminal and suppresses any fresh
///   fallback that could duplicate an accepted update.
/// - `Card { broken: true, .. }`: fail closed without another mutation.
/// - Anything else (`Message`, `None`): split the
///   markdown-to-native rendered response into chunks and `send_message` each
///   one. For `Message`, the first chunk replaces the existing preview via
///   `edit_message`; an unacknowledged edit is never duplicated through a
///   fresh send because the legacy result has no zero-mutation proof.
async fn run_native_commit(
    stream: Box<dyn ha_core::channel::traits::ChannelReplyStream>,
    reply: RichReply,
) -> Result<RichReplyReceipt, Option<ReplyStreamError>> {
    tokio::spawn(async move { stream.commit(&reply).await })
        .await
        .map_err(|error| {
            Some(ReplyStreamError::new(
                ReplyStreamErrorKind::Ambiguous,
                format!("native commit task failed: {error}"),
            ))
        })?
        .map_err(Some)
}

async fn run_native_send(
    plugin: Arc<dyn ChannelPlugin>,
    target: ReplyStreamTarget,
    reply: RichReply,
) -> Result<RichReplyReceipt, Option<ReplyStreamError>> {
    tokio::spawn(async move { plugin.send_rich_reply(&target, &reply).await })
        .await
        .map_err(|error| {
            Some(ReplyStreamError::new(
                ReplyStreamErrorKind::Ambiguous,
                format!("native final task failed: {error}"),
            ))
        })?
        .map_err(Some)
}

fn safe_final_fallback(kind: ReplyStreamErrorKind) -> bool {
    matches!(
        kind,
        ReplyStreamErrorKind::Unsupported
            | ReplyStreamErrorKind::InvalidTarget
            | ReplyStreamErrorKind::InvalidContent
            | ReplyStreamErrorKind::Rejected
            | ReplyStreamErrorKind::RateLimited
            | ReplyStreamErrorKind::Transient
    )
}

fn validate_native_receipt(
    receipt: &RichReplyReceipt,
    offered_media: usize,
) -> std::result::Result<usize, String> {
    if receipt.message_id.trim().is_empty() {
        return Err("native rich reply returned an empty message identifier".to_string());
    }
    let is_prefix = receipt
        .consumed_media
        .iter()
        .copied()
        .eq(0..receipt.consumed_media.len())
        && receipt.consumed_media.len() <= offered_media;
    if !is_prefix {
        return Err(format!(
            "native rich reply returned a non-prefix media receipt (consumed={:?}, offered={offered_media})",
            receipt.consumed_media
        ));
    }
    Ok(receipt.consumed_media.len())
}

/// Inline actions must remain the visually terminal element of a reply. When
/// media is delivered through a later legacy lane, send the actions as their
/// own final message after every media/fallback message has settled.
async fn deliver_deferred_buttons(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    buttons: &[Vec<InlineButton>],
) -> DeliveryReport {
    if buttons.is_empty() {
        return DeliveryReport::default();
    }
    let action_target = DeliveryTarget {
        account_id: target.account_id,
        chat_id: target.chat_id,
        chat_type: target.chat_type,
        thread_id: target.thread_id,
        // The content reply already carries the inbound anchor. Repeating it
        // on the terminal action row adds visual noise and can fork threads on
        // providers that distinguish reply anchors from thread identifiers.
        reply_to_message_id: None,
        recipient_user_id: target.recipient_user_id,
        recipient_tenant_id: target.recipient_tenant_id,
    };
    send_text_chunks(plugin, &action_target, "Actions", None, buttons).await
}

pub(crate) async fn send_final_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    response: &str,
    preview: Option<&PreviewHandle>,
    pending_media: &[ha_core::attachments::MediaItem],
    buttons: &[Vec<InlineButton>],
    allow_standalone_native: bool,
    caps: &ChannelCapabilities,
) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    if let Err(error) = plugin.validate_reply_buttons(buttons) {
        if let Some(native @ PreviewHandle::Native { .. }) = preview {
            let _ = super::streaming::abort_native_preview(native, ReplyAbortReason::Failed).await;
        }
        let error = ha_core::logging::redact_sensitive(&error.to_string());
        app_warn!(
            "channel",
            "worker",
            "Final reply actions failed provider preflight: {}",
            error
        );
        report.attempted = 1;
        report.failures.push(error);
        report.unsafe_to_continue = true;
        return report;
    }
    let response_has_text = !response.trim().is_empty();
    let legacy_preview_needs_terminal_actions = matches!(
        preview,
        Some(PreviewHandle::Message { .. } | PreviewHandle::Card { .. })
    );
    let defer_buttons = !buttons.is_empty()
        && (!pending_media.is_empty()
            || legacy_preview_needs_terminal_actions
            || !response_has_text);
    let content_buttons = if defer_buttons { &[] } else { buttons };

    if let Some(native_caps) = caps.native_reply.as_ref() {
        let stream_target = target.to_reply_stream_target();
        let channel_id = plugin.meta().id;
        let embedded_media_limit = native_media_budget(native_caps);
        let mut offered_media = Vec::new();
        let mut offered_original_indices = Vec::new();
        for (index, item) in pending_media.iter().take(embedded_media_limit).enumerate() {
            let media_type = classify_media_type(item);
            if native_caps.embedded_media_types.contains(&media_type)
                && native_media_supported_for_target(&channel_id, target.chat_id, item)
            {
                offered_media.push(to_outbound_media(item, media_type, &channel_id));
                offered_original_indices.push(index);
            } else {
                // Native content is delivered before the legacy media lane.
                // Offering only a contiguous prefix preserves the user's
                // original attachment order across both lanes.
                break;
            }
        }
        let rich_reply = RichReply {
            markdown: response.to_string(),
            media: offered_media,
            buttons: content_buttons.to_vec(),
        };

        let native_result = match preview {
            Some(PreviewHandle::Native {
                session,
                state,
                terminal_owner,
                preview_persistence: ReplyStreamPreviewPersistence::Ephemeral,
            }) => {
                if !try_claim_native_final(terminal_owner) {
                    return DeliveryReport {
                        attempted: 1,
                        succeeded: 0,
                        failures: vec![
                            "native reply terminal mutation was already claimed".to_string()
                        ],
                        unsafe_to_continue: true,
                    };
                }
                // Preview state is never a durable delivery outcome for an
                // ephemeral stream. Use the existing stream as the terminal
                // commit carrier when available; if open never produced one,
                // send the independent rich final through the plugin.
                let stream = session.lock().await.take();
                state.store(NATIVE_TERMINAL, Ordering::Release);
                if native_caps.final_chat_types.contains(target.chat_type) {
                    Some((
                        match stream {
                            Some(stream) => run_native_commit(stream, rich_reply.clone()).await,
                            None => {
                                run_native_send(
                                    plugin.clone(),
                                    stream_target.clone(),
                                    rich_reply.clone(),
                                )
                                .await
                            }
                        },
                        true,
                    ))
                } else {
                    if let Some(stream) = stream {
                        // Capability selection requires preview types to be a
                        // subset of final types. Keep this defensive mismatch
                        // cleanup detached so a future adapter regression can
                        // neither hang nor cancel legacy final delivery.
                        drop(tokio::spawn(async move {
                            let _ = stream.abort(ReplyAbortReason::Detached).await;
                        }));
                    }
                    None
                }
            }
            Some(PreviewHandle::Native {
                session,
                state,
                terminal_owner,
                ..
            }) => {
                if !try_claim_native_final(terminal_owner) {
                    return DeliveryReport {
                        attempted: 1,
                        succeeded: 0,
                        failures: vec![
                            "native reply terminal mutation was already claimed".to_string()
                        ],
                        unsafe_to_continue: true,
                    };
                }
                match state.load(Ordering::Acquire) {
                    NATIVE_AMBIGUOUS | NATIVE_BROKEN | NATIVE_TERMINAL => Some((Err(None), false)),
                    NATIVE_ACTIVE => {
                        let stream = session.lock().await.take();
                        state.store(NATIVE_TERMINAL, Ordering::Release);
                        match stream {
                            Some(stream) => {
                                Some((run_native_commit(stream, rich_reply.clone()).await, false))
                            }
                            None => Some((Err(None), false)),
                        }
                    }
                    NATIVE_SELECTED => {
                        state.store(NATIVE_TERMINAL, Ordering::Release);
                        if native_caps.final_chat_types.contains(target.chat_type) {
                            Some((
                                run_native_send(
                                    plugin.clone(),
                                    stream_target.clone(),
                                    rich_reply.clone(),
                                )
                                .await,
                                true,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => Some((Err(None), false)),
                }
            }
            _ if preview.is_none()
                && allow_standalone_native
                && native_caps.final_chat_types.contains(target.chat_type) =>
            {
                Some((
                    run_native_send(plugin.clone(), stream_target.clone(), rich_reply.clone())
                        .await,
                    true,
                ))
            }
            _ => None,
        };

        if let Some((native_result, allow_safe_fallback)) = native_result {
            match native_result {
                Ok(receipt) => {
                    report.attempted += 1;
                    let consumed_prefix_len =
                        match validate_native_receipt(&receipt, offered_original_indices.len()) {
                            Ok(consumed) => consumed,
                            Err(error) => {
                                app_warn!("channel", "worker", "{}", error);
                                // The provider has already accepted the terminal
                                // mutation. Do not risk duplicate/reordered media or
                                // actions after a malformed adapter receipt.
                                report.failures.push(error);
                                report.unsafe_to_continue = true;
                                return report;
                            }
                        };
                    report.succeeded += 1;
                    let remaining: Vec<_> = pending_media
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index >= consumed_prefix_len)
                        .map(|(_, item)| item.clone())
                        .collect();
                    let media_report =
                        deliver_media_to_chat(plugin, target, &remaining, caps).await;
                    let can_continue = !media_report.unsafe_to_continue;
                    report.merge(media_report);
                    if defer_buttons && can_continue {
                        report.merge(deliver_deferred_buttons(plugin, target, buttons).await);
                    }
                    return report;
                }
                Err(Some(error)) if allow_safe_fallback && safe_final_fallback(error.kind) => {
                    let error = ha_core::logging::redact_sensitive(&error.to_string());
                    app_warn!(
                        "channel",
                        "worker",
                        "Native final reply failed safely; using legacy delivery: {}",
                        error
                    );
                    // Continue into the complete legacy text + media path.
                }
                Err(Some(error)) => {
                    report.attempted += 1;
                    let error = ha_core::logging::redact_sensitive(&error.to_string());
                    app_warn!(
                        "channel",
                        "worker",
                        "Native final reply outcome is ambiguous; suppressing fallback: {}",
                        error
                    );
                    report.failures.push(error);
                    report.unsafe_to_continue = true;
                    return report;
                }
                Err(None) => {
                    report.attempted += 1;
                    report
                        .failures
                        .push("native reply terminal outcome was ambiguous".to_string());
                    report.unsafe_to_continue = true;
                    return report;
                }
            }
        }
    }

    let card_finalized = if response_has_text {
        match preview {
            Some(PreviewHandle::Card {
                card_id,
                element_id,
                sequence,
                broken: false,
                ..
            }) => match finalize_card_stream(
                plugin,
                target.account_id,
                card_id,
                element_id,
                *sequence,
                response,
            )
            .await
            {
                CardFinalizeOutcome::Confirmed => true,
                CardFinalizeOutcome::Unsafe(error) => {
                    report.attempted += 1;
                    report
                        .failures
                        .push(ha_core::logging::redact_sensitive(&error));
                    report.unsafe_to_continue = true;
                    return report;
                }
            },
            Some(PreviewHandle::Card { broken: true, .. }) => {
                report.attempted += 1;
                report.failures.push(
                    "visible card preview is in an ambiguous state; duplicate fallback suppressed"
                        .to_string(),
                );
                report.unsafe_to_continue = true;
                return report;
            }
            _ => false,
        }
    } else {
        false
    };

    if response_has_text && !card_finalized {
        let chunk_preview = preview;
        let text_report =
            send_text_chunks(plugin, target, response, chunk_preview, content_buttons).await;
        let can_continue = !text_report.unsafe_to_continue;
        report.merge(text_report);
        if !can_continue {
            return report;
        }
    } else if card_finalized {
        report.attempted += 1;
        report.succeeded += 1;
    }

    let media_report = deliver_media_to_chat(plugin, target, pending_media, caps).await;
    let can_continue = !media_report.unsafe_to_continue;
    report.merge(media_report);
    if defer_buttons && can_continue {
        report.merge(deliver_deferred_buttons(plugin, target, buttons).await);
    }
    report
}

async fn deliver_media_fallback_links(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    items: &[&ha_core::attachments::MediaItem],
) -> DeliveryReport {
    let Some(text) = build_media_fallback_lines(items) else {
        return DeliveryReport::default();
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let fallback_target = DeliveryTarget {
        account_id: target.account_id,
        chat_id: target.chat_id,
        chat_type: target.chat_type,
        thread_id: target.thread_id,
        reply_to_message_id: None,
        recipient_user_id: target.recipient_user_id,
        recipient_tenant_id: target.recipient_tenant_id,
    };
    send_text_chunks(plugin, &fallback_target, &text, None, &[]).await
}

/// Send media in original attachment order. At most five items are attempted
/// as standalone provider media; the remaining suffix is retained as link
/// fallback instead of being dropped. Any attempted send without a confirmed
/// success stops the ordered sequence: legacy `DeliveryResult` has no typed
/// zero-delivery proof, so retrying as a link could duplicate accepted media.
pub(crate) async fn deliver_media_to_chat(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    items: &[ha_core::attachments::MediaItem],
    caps: &ChannelCapabilities,
) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    if items.is_empty() {
        return report;
    }

    let channel_id = plugin.meta().id;
    for (index, it) in items.iter().enumerate() {
        if index >= MAX_MEDIA_PER_TURN {
            let suffix = items[index..].iter().collect::<Vec<_>>();
            let fallback = deliver_media_fallback_links(plugin, target, &suffix).await;
            report.merge(fallback);
            return report;
        }

        let Some(native_type) = legacy_native_media_type(it, caps)
            .filter(|_| native_media_supported_for_target(&channel_id, target.chat_id, it))
        else {
            let fallback = deliver_media_fallback_links(plugin, target, &[it]).await;
            let can_continue = !fallback.unsafe_to_continue;
            report.merge(fallback);
            if !can_continue {
                return report;
            }
            continue;
        };

        report.attempted += 1;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let payload = ReplyPayload {
            text: None,
            media: vec![to_outbound_media(it, native_type, &channel_id)],
            reply_to_message_id: None,
            parse_mode: None,
            buttons: Vec::new(),
            thread_id: target.thread_id.map(str::to_string),
            draft_id: None,
        };
        match plugin
            .send_message(target.account_id, target.chat_id, &payload)
            .await
        {
            Ok(r) if !r.success => {
                let error = ha_core::logging::redact_sensitive(
                    &r.error
                        .unwrap_or_else(|| "platform rejected media".to_string()),
                );
                app_warn!(
                    "channel",
                    "worker",
                    "Media send failed ({}): {}",
                    it.name,
                    error
                );
                report.failures.push(error);
                // `success=false` is a generic negative result, not proof
                // that the provider performed no mutation. Without a typed
                // zero-delivery acknowledgement, link fallback could duplicate
                // an accepted attachment.
                report.unsafe_to_continue = true;
                return report;
            }
            Err(e) => {
                let error = ha_core::logging::redact_sensitive(&e.to_string());
                app_error!(
                    "channel",
                    "worker",
                    "Media send error ({}): {}",
                    it.name,
                    error
                );
                report.failures.push(error);
                report.unsafe_to_continue = true;
                return report;
            }
            Ok(_) => report.succeeded += 1,
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::attachments::{MediaItem, MediaKind};

    #[test]
    fn stop_command_is_reserved_from_interactive_reply_parsing() {
        assert!(is_stop_command(Some("/stop")));
        assert!(is_stop_command(Some("  /STOP  ")));
        assert!(!is_stop_command(Some("stop")));
        assert!(!is_stop_command(Some("/status")));
        assert!(!is_stop_command(None));
    }

    #[tokio::test]
    async fn cancelled_channel_message_retry_holds_gate_until_delete_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("sessions.db");
        let db = Arc::new(
            ha_core::session::SessionDB::open_ephemeral_for_test(&db_path)
                .expect("open session db"),
        );
        let session = db.create_session("ha-main").expect("create session");
        let message_id = db
            .append_message(
                &session.id,
                &ha_core::session::NewMessage::user("cancelled inbound"),
            )
            .expect("append message");
        db.with_conn_for_test(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER block_cancelled_message_delete
                 BEFORE DELETE ON messages
                 BEGIN SELECT RAISE(FAIL, 'delete blocked'); END;",
            )?;
            Ok(())
        })
        .expect("install delete blocker");

        let cleanup =
            spawn_cancelled_channel_message_rollback(db.clone(), session.id.clone(), message_id);
        assert!(ha_core::chat_engine::active_turn::stop_cleanup_active(
            &session.id
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(db.get_message(message_id).expect("read message").is_some());

        db.with_conn_for_test(|conn| {
            conn.execute_batch("DROP TRIGGER block_cancelled_message_delete;")?;
            Ok(())
        })
        .expect("remove delete blocker");
        tokio::time::timeout(std::time::Duration::from_secs(2), cleanup)
            .await
            .expect("cleanup retry should settle")
            .expect("cleanup task should not panic");
        assert!(db.get_message(message_id).expect("read message").is_none());
    }

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
            native_reply: None,
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
    fn legacy_media_capability_classification_is_per_item() {
        let photo = mk_item("a.png", "image/png", MediaKind::Image);
        let video = mk_item("a.mp4", "video/mp4", MediaKind::File);
        let document = mk_item("a.pdf", "application/pdf", MediaKind::File);
        let capabilities = caps(vec![MediaType::Photo]);

        assert_eq!(
            legacy_native_media_type(&photo, &capabilities),
            Some(MediaType::Photo)
        );
        assert_eq!(legacy_native_media_type(&video, &capabilities), None);
        assert_eq!(legacy_native_media_type(&document, &capabilities), None);
    }

    #[test]
    fn animation_falls_back_to_photo_when_channel_lacks_animation() {
        let animation = mk_item("a.gif", "image/gif", MediaKind::Image);
        assert_eq!(
            legacy_native_media_type(&animation, &caps(vec![MediaType::Photo])),
            Some(MediaType::Photo)
        );
    }

    #[test]
    fn outbound_prefers_local_path() {
        let it = mk_item("x.pdf", "application/pdf", MediaKind::File);
        let out = to_outbound_media(&it, MediaType::Document, &ChannelId::Telegram);
        assert!(matches!(out.data, MediaData::FilePath(_)));
    }

    #[test]
    fn public_attachment_url_requires_https_when_native_media_needs_it() {
        let it = mk_item("x.png", "image/png", MediaKind::Image);
        assert_eq!(
            public_attachment_url_with_base(&it, Some("https://files.example"), true).as_deref(),
            Some("https://files.example/api/attachments/s/x.png")
        );
        assert!(public_attachment_url_with_base(&it, Some("http://files.example"), true).is_none());
        assert_eq!(
            public_attachment_url_with_base(&it, Some("http://files.example"), false).as_deref(),
            Some("http://files.example/api/attachments/s/x.png")
        );
    }

    #[test]
    fn line_and_qqbot_use_public_url_for_native_media() {
        let mut it = mk_item("x.png", "image/png", MediaKind::Image);
        it.url = "https://files.example/x.png".to_string();
        let line = to_outbound_media(&it, MediaType::Photo, &ChannelId::Line);
        assert!(matches!(
            line.data,
            MediaData::Url(ref url) if url == "https://files.example/x.png"
        ));
        assert!(native_media_supported_for_target(
            &ChannelId::Line,
            "user",
            &it
        ));
        assert!(native_media_supported_for_target(
            &ChannelId::QqBot,
            "group:g1",
            &it
        ));
        assert!(!native_media_supported_for_target(
            &ChannelId::QqBot,
            "channel:c1",
            &it
        ));
    }

    use anyhow::Result;
    use async_trait::async_trait;
    use ha_core::chat_engine::RoundOutput;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    struct RecordingErrorTerminalStream {
        errors: Arc<Mutex<Vec<String>>>,
        reject: bool,
    }

    #[async_trait]
    impl ha_core::channel::traits::ChannelReplyStream for RecordingErrorTerminalStream {
        async fn push(
            &mut self,
            _frame: &ReplyStreamFrame,
        ) -> std::result::Result<(), ReplyStreamError> {
            Ok(())
        }

        async fn commit(
            self: Box<Self>,
            _final_reply: &RichReply,
        ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
            unreachable!("error-terminal contract must not call commit")
        }

        async fn fail(
            self: Box<Self>,
            error_text: &str,
        ) -> std::result::Result<(), ReplyStreamError> {
            self.errors.lock().unwrap().push(error_text.to_string());
            if self.reject {
                Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::Ambiguous,
                    "synthetic ambiguous native terminal",
                ))
            } else {
                Ok(())
            }
        }

        async fn abort(
            self: Box<Self>,
            _reason: ReplyAbortReason,
        ) -> std::result::Result<(), ReplyStreamError> {
            unreachable!("visible error terminal must not degrade to plain abort")
        }
    }

    struct CountingPlugin {
        max_bytes: usize,
        sends: Mutex<Vec<String>>,
        delivery_order: Mutex<Vec<&'static str>>,
        reply_tos: Mutex<Vec<Option<String>>>,
        send_count: AtomicUsize,
        fail_media: bool,
        error_media: bool,
        reject_buttons: bool,
        native_media_limit: Option<u16>,
        rich_media_count: AtomicUsize,
    }

    impl CountingPlugin {
        fn new(max_bytes: usize) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                delivery_order: Mutex::new(Vec::new()),
                reply_tos: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
                fail_media: false,
                error_media: false,
                reject_buttons: false,
                native_media_limit: None,
                rich_media_count: AtomicUsize::new(0),
            }
        }

        fn failing_media(max_bytes: usize) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                delivery_order: Mutex::new(Vec::new()),
                reply_tos: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
                fail_media: true,
                error_media: false,
                reject_buttons: false,
                native_media_limit: None,
                rich_media_count: AtomicUsize::new(0),
            }
        }

        fn erroring_media(max_bytes: usize) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                delivery_order: Mutex::new(Vec::new()),
                reply_tos: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
                fail_media: false,
                error_media: true,
                reject_buttons: false,
                native_media_limit: None,
                rich_media_count: AtomicUsize::new(0),
            }
        }

        fn rejecting_buttons(max_bytes: usize) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                delivery_order: Mutex::new(Vec::new()),
                reply_tos: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
                fail_media: false,
                error_media: false,
                reject_buttons: true,
                native_media_limit: None,
                rich_media_count: AtomicUsize::new(0),
            }
        }

        fn native_media(max_bytes: usize, limit: u16) -> Self {
            Self {
                max_bytes,
                sends: Mutex::new(Vec::new()),
                delivery_order: Mutex::new(Vec::new()),
                reply_tos: Mutex::new(Vec::new()),
                send_count: AtomicUsize::new(0),
                fail_media: false,
                error_media: false,
                reject_buttons: false,
                native_media_limit: Some(limit),
                rich_media_count: AtomicUsize::new(0),
            }
        }

        /// reply_to_message_id of the last `send_message` call (outer `None` =
        /// nothing was sent).
        fn last_reply_to(&self) -> Option<Option<String>> {
            self.reply_tos.lock().unwrap().last().cloned()
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
            c.native_reply = self
                .native_media_limit
                .map(|limit| NativeReplyCapabilities {
                    preview_chat_types: Vec::new(),
                    final_chat_types: vec![ChatType::Dm],
                    update_mode: ReplyStreamUpdateMode::Snapshot,
                    preview_persistence: ReplyStreamPreviewPersistence::Persistent,
                    requires_reply_anchor: false,
                    requires_recipient_user_id: false,
                    requires_recipient_tenant_id: false,
                    supports_task_updates: false,
                    supports_plan_updates: false,
                    supports_blocks: true,
                    embedded_media_types: vec![MediaType::Document],
                    max_embedded_media_items: Some(limit),
                    refresh_after_secs: None,
                    max_delta_chars: None,
                });
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
            self.delivery_order
                .lock()
                .unwrap()
                .push(if !payload.buttons.is_empty() {
                    "buttons"
                } else if !payload.media.is_empty() {
                    "media"
                } else {
                    "text"
                });
            if !payload.media.is_empty() && self.fail_media {
                return Ok(DeliveryResult::err("native media failed"));
            }
            if !payload.media.is_empty() && self.error_media {
                anyhow::bail!("ambiguous native media error");
            }
            if let Some(text) = payload.text.as_ref() {
                self.sends.lock().unwrap().push(text.clone());
            }
            self.reply_tos
                .lock()
                .unwrap()
                .push(payload.reply_to_message_id.clone());
            Ok(DeliveryResult::ok(format!("msg-{}", n)))
        }

        async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
            Ok(())
        }

        fn validate_reply_buttons(
            &self,
            _buttons: &[Vec<InlineButton>],
        ) -> std::result::Result<(), ReplyStreamError> {
            if self.reject_buttons {
                Err(ReplyStreamError::new(
                    ReplyStreamErrorKind::InvalidContent,
                    "synthetic invalid buttons",
                ))
            } else {
                Ok(())
            }
        }

        async fn send_rich_reply(
            &self,
            _target: &ReplyStreamTarget,
            reply: &RichReply,
        ) -> std::result::Result<RichReplyReceipt, ReplyStreamError> {
            if self.native_media_limit.is_none() {
                return Err(ReplyStreamError::unsupported(
                    "synthetic native final is disabled",
                ));
            }
            self.delivery_order.lock().unwrap().push("native");
            self.rich_media_count
                .store(reply.media.len(), Ordering::SeqCst);
            Ok(RichReplyReceipt {
                message_id: "native-final".to_string(),
                consumed_media: (0..reply.media.len()).collect(),
            })
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
    async fn native_error_terminates_same_stream_without_standalone_send() {
        let terminal_errors = Arc::new(Mutex::new(Vec::new()));
        let session: super::super::streaming::SharedNativeReplySession = Arc::new(
            tokio::sync::Mutex::new(Some(Box::new(RecordingErrorTerminalStream {
                errors: terminal_errors.clone(),
                reject: false,
            }))),
        );
        let preview = PreviewHandle::Native {
            session,
            state: Arc::new(std::sync::atomic::AtomicU8::new(NATIVE_ACTIVE)),
            terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            preview_persistence: ReplyStreamPreviewPersistence::Persistent,
        };
        let concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        let report = send_error_reply(&plugin, &target, Some(&preview), "terminal error").await;

        assert!(report.is_success());
        assert_eq!(
            terminal_errors.lock().unwrap().as_slice(),
            ["terminal error"]
        );
        assert!(
            concrete.sends.lock().unwrap().is_empty(),
            "native error terminal must not open a standalone legacy message"
        );
        assert_eq!(concrete.send_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn selected_native_error_sends_exactly_one_safe_standalone() {
        let state = Arc::new(std::sync::atomic::AtomicU8::new(NATIVE_SELECTED));
        let terminal_owner = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let preview = PreviewHandle::Native {
            session: Arc::new(tokio::sync::Mutex::new(None)),
            state: state.clone(),
            terminal_owner: terminal_owner.clone(),
            preview_persistence: ReplyStreamPreviewPersistence::Persistent,
        };
        let concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        let report = send_error_reply(&plugin, &target, Some(&preview), "terminal error").await;

        assert!(report.is_success());
        assert_eq!(
            concrete.sends.lock().unwrap().as_slice(),
            ["terminal error"]
        );
        assert_eq!(concrete.send_count.load(Ordering::SeqCst), 1);
        assert_eq!(state.load(Ordering::Acquire), NATIVE_TERMINAL);
        assert!(!try_claim_native_final(&terminal_owner));
    }

    #[tokio::test]
    async fn opening_or_ambiguous_native_error_never_uses_standalone() {
        for lifecycle in [NATIVE_OPENING, NATIVE_AMBIGUOUS] {
            let preview = PreviewHandle::Native {
                session: Arc::new(tokio::sync::Mutex::new(None)),
                state: Arc::new(std::sync::atomic::AtomicU8::new(lifecycle)),
                terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                preview_persistence: ReplyStreamPreviewPersistence::Persistent,
            };
            let concrete = Arc::new(CountingPlugin::new(4096));
            let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
            let target = DeliveryTarget {
                account_id: "acc",
                chat_id: "chat",
                chat_type: &ChatType::Dm,
                thread_id: None,
                reply_to_message_id: Some("incoming"),
                recipient_user_id: None,
                recipient_tenant_id: None,
            };

            let report = send_error_reply(&plugin, &target, Some(&preview), "terminal error").await;

            assert!(!report.is_success());
            assert!(report.unsafe_to_continue);
            assert_eq!(concrete.send_count.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn ambiguous_native_error_suppresses_standalone_fallback() {
        let session: super::super::streaming::SharedNativeReplySession = Arc::new(
            tokio::sync::Mutex::new(Some(Box::new(RecordingErrorTerminalStream {
                errors: Arc::new(Mutex::new(Vec::new())),
                reject: true,
            }))),
        );
        let preview = PreviewHandle::Native {
            session,
            state: Arc::new(std::sync::atomic::AtomicU8::new(NATIVE_ACTIVE)),
            terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            preview_persistence: ReplyStreamPreviewPersistence::Persistent,
        };
        let concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        let report = send_error_reply(&plugin, &target, Some(&preview), "terminal error").await;

        assert!(!report.is_success());
        assert!(report.unsafe_to_continue);
        assert!(concrete.sends.lock().unwrap().is_empty());
        assert_eq!(concrete.send_count.load(Ordering::SeqCst), 0);
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
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
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

    #[tokio::test]
    async fn deliver_split_quotes_final_round_only_when_nothing_finalized_inline() {
        let rounds = vec![
            RoundOutput {
                text: "round 0".to_string(),
                medias: Vec::new(),
            },
            RoundOutput {
                text: "final.".to_string(),
                medias: Vec::new(),
            },
        ];
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("m1"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        // Nothing finalized inline (finalized_rounds = 0): the final round is
        // the turn's first outbound message, so it carries the quote.
        let p0 = Arc::new(CountingPlugin::new(4096));
        let dyn0: Arc<dyn ChannelPlugin> = p0.clone();
        let _ = deliver_split(&dyn0, &target, &rounds, "fb", None, 0, &dyn0.capabilities()).await;
        assert_eq!(p0.last_reply_to(), Some(Some("m1".to_string())));

        // Stream task already shipped (and quoted) round 0 inline
        // (finalized_rounds = 1): the trailing round must not stack a 2nd quote.
        let p1 = Arc::new(CountingPlugin::new(4096));
        let dyn1: Arc<dyn ChannelPlugin> = p1.clone();
        let _ = deliver_split(&dyn1, &target, &rounds, "fb", None, 1, &dyn1.capabilities()).await;
        assert_eq!(p1.last_reply_to(), Some(None));
    }

    #[tokio::test]
    async fn final_preview_edit_error_suppresses_an_ambiguous_duplicate_send() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        let report = send_text_chunks(
            &plugin,
            &target,
            "complete answer",
            Some(&PreviewHandle::Message {
                message_id: "stale-preview".to_string(),
            }),
            &[],
        )
        .await;

        assert!(!report.is_success());
        assert!(report.unsafe_to_continue);
        assert!(plugin_concrete.sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unacknowledged_media_send_suppresses_unsafe_link_fallback() {
        let plugin_concrete = Arc::new(CountingPlugin::failing_media(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let items = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];
        let caps = caps(vec![MediaType::Document]);
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };

        let report = deliver_media_to_chat(&plugin, &target, &items, &caps).await;

        let sends = plugin_concrete.sends.lock().unwrap().clone();
        assert!(sends.is_empty());
        assert!(!report.is_success());
        assert!(report.unsafe_to_continue);
        assert_eq!(report.attempted, 1);
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.failures, vec!["native media failed"]);
    }

    #[test]
    fn native_media_budget_is_explicit_and_bounded() {
        let mut native = NativeReplyCapabilities {
            preview_chat_types: Vec::new(),
            final_chat_types: Vec::new(),
            update_mode: ReplyStreamUpdateMode::Snapshot,
            preview_persistence: ReplyStreamPreviewPersistence::Persistent,
            requires_reply_anchor: false,
            requires_recipient_user_id: false,
            requires_recipient_tenant_id: false,
            supports_task_updates: false,
            supports_plan_updates: false,
            supports_blocks: true,
            embedded_media_types: vec![MediaType::Photo],
            max_embedded_media_items: None,
            refresh_after_secs: None,
            max_delta_chars: None,
        };
        assert_eq!(native_media_budget(&native), 0);

        native.max_embedded_media_items = Some(50);
        assert_eq!(native_media_budget(&native), 50);

        native.max_embedded_media_items = Some(u16::MAX);
        assert_eq!(native_media_budget(&native), MAX_NATIVE_MEDIA_PER_TURN);
    }

    #[test]
    fn native_receipt_requires_message_id_and_ordered_media_prefix() {
        let valid = RichReplyReceipt {
            message_id: "m1".to_string(),
            consumed_media: vec![0, 1],
        };
        assert_eq!(validate_native_receipt(&valid, 2), Ok(2));

        let empty_message_id = RichReplyReceipt {
            message_id: "  ".to_string(),
            consumed_media: Vec::new(),
        };
        assert!(validate_native_receipt(&empty_message_id, 0).is_err());

        for malformed in [vec![1, 0], vec![0, 0], vec![0, 2], vec![0, 1, 2]] {
            let receipt = RichReplyReceipt {
                message_id: "m1".to_string(),
                consumed_media: malformed,
            };
            assert!(validate_native_receipt(&receipt, 2).is_err());
        }
    }

    #[tokio::test]
    async fn native_media_cap_keeps_the_full_legacy_suffix() {
        let plugin_concrete = Arc::new(CountingPlugin::native_media(4096, 50));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media: Vec<_> = (0..55)
            .map(|index| mk_item(&format!("f{index}.pdf"), "application/pdf", MediaKind::File))
            .collect();
        let capabilities = plugin.capabilities();

        let report = send_final_reply(
            &plugin,
            &target,
            "complete answer",
            None,
            &media,
            &[],
            true,
            &capabilities,
        )
        .await;

        assert!(report.is_success());
        assert_eq!(plugin_concrete.rich_media_count.load(Ordering::SeqCst), 50);
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["native", "text", "text", "text", "text", "text"]
        );
        let sends = plugin_concrete.sends.lock().unwrap().join("\n");
        for name in ["f50.pdf", "f51.pdf", "f52.pdf", "f53.pdf", "f54.pdf"] {
            assert!(sends.contains(name));
        }
    }

    #[tokio::test]
    async fn invalid_deferred_buttons_fail_before_any_terminal_mutation() {
        let plugin_concrete = Arc::new(CountingPlugin::rejecting_buttons(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];
        let buttons = vec![vec![InlineButton {
            text: "invalid".to_string(),
            callback_data: Some("invalid".to_string()),
            url: None,
        }]];

        let report = send_final_reply(
            &plugin,
            &target,
            "complete answer",
            None,
            &media,
            &buttons,
            false,
            &caps(vec![MediaType::Document]),
        )
        .await;

        assert!(!report.is_success());
        assert!(report.unsafe_to_continue);
        assert_eq!(report.attempted, 1);
        assert_eq!(plugin_concrete.send_count.load(Ordering::SeqCst), 0);
        assert!(plugin_concrete.delivery_order.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ambiguous_media_error_stops_fallback_and_terminal_buttons() {
        let plugin_concrete = Arc::new(CountingPlugin::erroring_media(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: Some("thread"),
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];
        let buttons = vec![vec![InlineButton {
            text: "Continue".to_string(),
            callback_data: Some("continue".to_string()),
            url: None,
        }]];

        let report = send_final_reply(
            &plugin,
            &target,
            "complete answer",
            None,
            &media,
            &buttons,
            false,
            &caps(vec![MediaType::Document]),
        )
        .await;

        assert!(!report.is_success());
        assert!(report.unsafe_to_continue);
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["text", "media"]
        );
        assert_eq!(
            plugin_concrete.sends.lock().unwrap().as_slice(),
            ["complete answer"]
        );
    }

    #[tokio::test]
    async fn mixed_legacy_media_preserves_fallback_then_native_order() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![
            mk_item("first.png", "image/png", MediaKind::Image),
            mk_item("second.pdf", "application/pdf", MediaKind::File),
        ];

        let report =
            deliver_media_to_chat(&plugin, &target, &media, &caps(vec![MediaType::Document])).await;

        assert!(report.is_success());
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["text", "media"]
        );
        let sends = plugin_concrete.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        assert!(sends[0].contains("first.png"));
    }

    #[tokio::test]
    async fn legacy_media_over_five_retains_the_ordered_suffix_as_links() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media: Vec<_> = (0..8)
            .map(|index| mk_item(&format!("f{index}.pdf"), "application/pdf", MediaKind::File))
            .collect();

        let report =
            deliver_media_to_chat(&plugin, &target, &media, &caps(vec![MediaType::Document])).await;

        assert!(report.is_success());
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["media", "media", "media", "media", "media", "text"]
        );
        let sends = plugin_concrete.sends.lock().unwrap();
        assert_eq!(sends.len(), 1);
        for name in ["f5.pdf", "f6.pdf", "f7.pdf"] {
            assert!(sends[0].contains(name));
        }
    }

    #[tokio::test]
    async fn media_only_final_skips_the_empty_text_lane() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];

        let report = send_final_reply(
            &plugin,
            &target,
            "",
            None,
            &media,
            &[],
            false,
            &caps(vec![MediaType::Document]),
        )
        .await;

        assert!(report.is_success());
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["media"]
        );
    }

    #[tokio::test]
    async fn media_only_unsupported_item_uses_link_without_empty_message() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];

        let report = send_final_reply(
            &plugin,
            &target,
            "",
            None,
            &media,
            &[],
            false,
            &caps(Vec::new()),
        )
        .await;

        assert!(report.is_success());
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["text"]
        );
        assert!(plugin_concrete.sends.lock().unwrap()[0].contains("x.pdf"));
    }

    #[tokio::test]
    async fn final_buttons_follow_all_legacy_media() {
        let plugin_concrete = Arc::new(CountingPlugin::new(4096));
        let plugin: Arc<dyn ChannelPlugin> = plugin_concrete.clone();
        let target = DeliveryTarget {
            account_id: "acc",
            chat_id: "chat",
            chat_type: &ChatType::Dm,
            thread_id: Some("thread"),
            reply_to_message_id: Some("incoming"),
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let media = vec![mk_item("x.pdf", "application/pdf", MediaKind::File)];
        let buttons = vec![vec![InlineButton {
            text: "Continue".to_string(),
            callback_data: Some("continue".to_string()),
            url: None,
        }]];
        let caps = caps(vec![MediaType::Document]);

        let report = send_final_reply(
            &plugin,
            &target,
            "complete answer",
            None,
            &media,
            &buttons,
            false,
            &caps,
        )
        .await;

        assert!(report.is_success());
        assert_eq!(
            plugin_concrete.delivery_order.lock().unwrap().as_slice(),
            ["text", "media", "buttons"]
        );
        assert_eq!(plugin_concrete.last_reply_to(), Some(None));
    }
}

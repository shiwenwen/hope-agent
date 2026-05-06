use std::sync::Arc;
use tokio::sync::mpsc;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

/// Conservative byte ceiling for a single cardkit markdown element.
/// Cardkit documents ~100k characters per element; we leave headroom for
/// protocol overhead. Independent of the IM-text `max_message_length`
/// (cardkit elements aren't subject to that limit) so streaming previews
/// keep flowing on responses larger than the channel's text-message cap.
pub(super) const CARD_ELEMENT_MAX_BYTES: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamPreviewTransport {
    /// Telegram-style draft API: `send_draft` repeatedly with the same
    /// `draft_id`. Free of edit-rate limits, leaves no "edited" marker.
    Draft,
    /// Standard `send_message` + `edit_message` cycle. Works on most
    /// channels but typically flags the host message as edited.
    Message,
    /// Card-streaming API (currently Feishu cardkit). Creates an
    /// interactive card and updates a single element in place — the host
    /// message is never edited, so no "edited" marker appears.
    Card,
}

/// Persistent identity for the rendered preview, returned to the caller so
/// `send_final_reply` can finalize using the matching path.
#[derive(Debug, Clone)]
pub(super) enum PreviewHandle {
    /// `edit_message` rewrites this message_id at finalization.
    Message { message_id: String },
    /// Card-stream session. `broken=true` means an irrecoverable update
    /// error occurred mid-stream — finalization should fall back to a new
    /// `send_message` rather than continuing the cardkit dance.
    Card {
        card_id: String,
        element_id: String,
        sequence: i64,
        broken: bool,
    },
}

#[derive(Debug, Default)]
pub(super) struct StreamPreviewOutcome {
    pub preview: Option<PreviewHandle>,
}

/// Spawn a background task that receives streaming events from the chat engine
/// and sends progressive previews to the IM channel.
///
/// Preview flow (one of three branches per session):
/// 1. Accumulate `text_delta` events from the chat engine
/// 2. Periodically render the accumulated snapshot via:
///    - **Draft**: `send_draft` for Telegram private chats (no rate limit), or
///    - **Card**: cardkit `create_card_stream` + `update_card_element` for
///      Feishu (host message never marked as edited), or
///    - **Message**: `send_message` + `edit_message` for channels that only
///      support message edits (host message ends up showing "edited" badge)
/// 3. Caller commits the final response via `send_final_reply` using the
///    `PreviewHandle` returned in `StreamPreviewOutcome`
///
/// For channels without any preview transport, events are simply drained while
/// the frontend still receives `channel:stream_delta` events.
pub(super) fn spawn_channel_stream_task(
    mut event_rx: mpsc::Receiver<String>,
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
        let mut card_session: Option<CardSession> = None;
        let mut dirty = false;
        // 1s cadence keeps us under Telegram's edit rate limit while feeling live.
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
                                    &accumulated, draft_id, &mut preview_transport,
                                    &mut preview_message_id, &mut card_session,
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
                            &accumulated, draft_id, &mut preview_transport,
                            &mut preview_message_id, &mut card_session,
                        ).await;
                        dirty = false;
                    }
                }
            }
        }

        let preview = match (&card_session, &preview_message_id) {
            (Some(session), _) => Some(PreviewHandle::Card {
                card_id: session.card_id.clone(),
                element_id: session.element_id.clone(),
                sequence: session.sequence,
                broken: session.broken,
            }),
            (None, Some(message_id)) => Some(PreviewHandle::Message {
                message_id: message_id.clone(),
            }),
            _ => None,
        };

        StreamPreviewOutcome { preview }
    })
}

/// Mutable state for an active card-streaming session. Only used inside
/// `spawn_channel_stream_task`; finalization-time fields are exported via
/// `PreviewHandle::Card`.
#[derive(Debug)]
struct CardSession {
    card_id: String,
    element_id: String,
    /// Next sequence number to use on `update_card_element`. Strictly
    /// monotonic per cardkit's contract.
    sequence: i64,
    /// True once an `update_card_element` failure made further preview
    /// updates pointless. Finalization should switch to `send_message`.
    broken: bool,
}

/// Extract text from a `text_delta` event JSON string.
pub(super) fn extract_text_delta(event_str: &str) -> Option<String> {
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

pub(super) fn select_stream_preview_transport(
    chat_type: &ChatType,
    capabilities: &ChannelCapabilities,
) -> Option<StreamPreviewTransport> {
    if matches!(chat_type, ChatType::Dm) && capabilities.supports_draft {
        return Some(StreamPreviewTransport::Draft);
    }
    if capabilities.supports_card_stream {
        return Some(StreamPreviewTransport::Card);
    }
    if capabilities.supports_edit {
        return Some(StreamPreviewTransport::Message);
    }
    None
}

pub(super) fn should_fallback_from_draft_error(error: &str) -> bool {
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

pub(super) fn build_stream_preview_payload(
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

/// Lazy-create the card on first preview, then update its single
/// element on subsequent ticks. Returns `Err(_)` only when the create
/// phase fails — caller should switch transport to `Message` and retry.
/// Mid-stream `update_card_element` errors flip `broken=true` but return
/// `Ok(())` to keep the loop running (final delivery handles broken cards).
async fn send_card_preview(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: &str,
    thread_id: Option<&str>,
    raw_text: &str,
    card_session: &mut Option<CardSession>,
) -> Result<(), String> {
    if raw_text.is_empty() || raw_text.len() > CARD_ELEMENT_MAX_BYTES {
        return Ok(());
    }

    if let Some(session) = card_session.as_mut() {
        if session.broken {
            return Ok(());
        }
        let next_seq = session.sequence;
        match plugin
            .update_card_element(
                account_id,
                &session.card_id,
                &session.element_id,
                raw_text,
                next_seq,
            )
            .await
        {
            Ok(()) => {
                session.sequence = next_seq + 1;
            }
            Err(e) => {
                app_warn!(
                    "channel",
                    "worker",
                    "card stream update failed (seq={}): {} — marking broken",
                    next_seq,
                    e
                );
                session.broken = true;
            }
        }
        return Ok(());
    }

    let handle = plugin
        .create_card_stream(account_id, raw_text)
        .await
        .map_err(|e| format!("create_card_stream: {}", e))?;
    let delivery = plugin
        .send_card_message(
            account_id,
            chat_id,
            &handle.card_id,
            Some(reply_to_message_id),
            thread_id,
        )
        .await
        .map_err(|e| format!("send_card_message: {}", e))?;
    if !delivery.success {
        return Err(format!(
            "send_card_message failed: {}",
            delivery.error.unwrap_or_default()
        ));
    }
    *card_session = Some(CardSession {
        card_id: handle.card_id,
        element_id: handle.element_id,
        // Initial content was set during create. First explicit update
        // starts at sequence=1 (cardkit treats create as sequence-less).
        sequence: 1,
        broken: false,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
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
    card_session: &mut Option<CardSession>,
) {
    // Lazy native-format payload for Draft / Message paths. The Card path
    // sends the raw markdown directly (cardkit markdown elements don't
    // want HTML conversion), so it skips this builder unless it has to
    // degrade to Message mid-flight.
    let build_payload = || {
        build_stream_preview_payload(
            plugin,
            reply_to_message_id,
            thread_id,
            text,
            draft_id,
            max_msg_len,
        )
    };

    match preview_transport {
        StreamPreviewTransport::Draft => {
            let Some(payload) = build_payload() else {
                return;
            };
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
        StreamPreviewTransport::Card => {
            if let Err(e) = send_card_preview(
                plugin,
                account_id,
                chat_id,
                reply_to_message_id,
                thread_id,
                text,
                card_session,
            )
            .await
            {
                // Any create/attach error → degrade to Message. The card
                // hasn't been shown yet, so degrading is harmless. Mid-stream
                // update errors are handled via card_session.broken instead
                // and never bubble here.
                app_warn!(
                    "channel",
                    "worker",
                    "card stream create failed, falling back to message edit: {}",
                    e
                );
                *preview_transport = StreamPreviewTransport::Message;
                if let Some(payload) = build_payload() {
                    send_message_preview(plugin, account_id, chat_id, &payload, preview_message_id)
                        .await;
                }
            }
        }
        StreamPreviewTransport::Message => {
            let Some(payload) = build_payload() else {
                return;
            };
            send_message_preview(plugin, account_id, chat_id, &payload, preview_message_id).await;
        }
    }
}

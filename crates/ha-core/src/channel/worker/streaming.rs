use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::dispatcher::deliver_media_to_chat;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;
use crate::chat_engine::RoundTextAccumulator;

/// Cardkit single-element character ceiling, per Feishu docs (100,000
/// characters per markdown element). Counted in `chars()` not bytes —
/// CJK glyphs are 3 bytes UTF-8, so a byte-based limit would silently
/// truncate at ~33k Chinese characters. Independent of IM-text
/// `max_message_length` (cardkit elements aren't subject to that limit)
/// so streaming previews keep flowing on responses larger than the
/// channel's text-message cap.
pub(super) const CARD_ELEMENT_MAX_CHARS: usize = 100_000;

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
    /// Number of LLM rounds the stream task already finalized inline (only
    /// non-zero under `ImReplyMode::Split` on streaming-capable channels).
    /// The dispatcher must skip these in `deliver_split` to avoid sending
    /// duplicate text or media; the caller's `drained_rounds[finalized_rounds..]`
    /// slice is what's left for it to deliver.
    pub finalized_rounds: usize,
}

/// Spawn a background task that receives streaming events from the chat engine
/// and sends progressive previews to the IM channel.
///
/// Two distinct preview behaviors driven by `reply_mode`:
///
/// - **`Preview` mode**: legacy single-growing-message behavior. Text deltas
///   from every round accumulate into one buffer that the preview transport
///   keeps re-rendering. Caller commits via `send_final_reply` using the
///   `PreviewHandle` returned in `StreamPreviewOutcome`.
///
/// - **`Split` mode + streaming-capable channel**: per-round preview. Each
///   round gets its own preview message that streams typewriter-style; on
///   round boundary (next round's first `text_delta` after a `tool_call`)
///   the task finalizes the current preview, delivers that round's media,
///   and resets state for the next round. The final round's preview is left
///   open so the caller can finalize it via `send_final_reply` (matching
///   the canonical chunk-or-card path). `finalized_rounds` reports how many
///   rounds the task already shipped, so the dispatcher only delivers the
///   trailing round.
///
/// - **`Final` / `Split` mode + non-streaming channel**: events are drained
///   without rendering any preview. Dispatcher then ships rounds as one-shot
///   `send_message` calls.
///
/// Preview transport selection (when active):
/// - **Draft**: `send_draft` for Telegram private chats (no rate limit)
/// - **Card**: cardkit `create_card_stream` + `update_card_element` for
///   Feishu (host message never marked as edited)
/// - **Message**: `send_message` + `edit_message` for channels that only
///   support message edits (host message ends up showing "edited" badge)
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_channel_stream_task(
    mut event_rx: mpsc::Receiver<String>,
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    reply_to_message_id: String,
    thread_id: Option<String>,
    preview_transport: Option<StreamPreviewTransport>,
    max_msg_len: usize,
    reply_mode: ImReplyMode,
    round_texts: Arc<Mutex<RoundTextAccumulator>>,
    capabilities: ChannelCapabilities,
) -> tokio::task::JoinHandle<StreamPreviewOutcome> {
    tokio::spawn(async move {
        let Some(mut preview_transport) = preview_transport else {
            // Preview disabled: drain events. The dispatcher still gets
            // round-aware text/media via `round_texts` (filled by the sink).
            while event_rx.recv().await.is_some() {}
            return StreamPreviewOutcome::default();
        };

        // Per-round preview only kicks in under split mode. Preview / Final
        // keep the legacy single-growing-buffer flow inside this task.
        let split_streaming = matches!(reply_mode, ImReplyMode::Split);

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
        // Tracks "saw a tool_call but not yet the next text_delta" — the
        // signal that the current round has closed and the next text_delta
        // (under split-streaming) must finalize this round before starting
        // the next preview.
        let mut in_tool_phase = false;
        // Number of rounds we've already shipped via per-round finalize.
        let mut finalized_rounds: usize = 0;
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
                            // Detect round boundaries on the same cheap-string
                            // contract the sink uses (BTreeMap key order
                            // means `"type":"…"` lands mid-string). Order
                            // checks rarer-needle-first.
                            if event_str.contains("\"type\":\"tool_call\"") {
                                in_tool_phase = true;
                            } else if let Some(text) = extract_text_delta(&event_str) {
                                if in_tool_phase && split_streaming {
                                    // Round just ended: flush + close current
                                    // preview, deliver this round's media,
                                    // then start a fresh preview for the new
                                    // round's first chunk.
                                    finalize_split_round(
                                        &plugin, &account_id, &chat_id,
                                        &reply_to_message_id, thread_id.as_deref(), max_msg_len,
                                        &accumulated, draft_id, &mut preview_transport,
                                        &mut preview_message_id, &mut card_session,
                                        finalized_rounds, &round_texts, &capabilities,
                                    ).await;
                                    accumulated.clear();
                                    finalized_rounds += 1;
                                }
                                in_tool_phase = false;
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
                            // Split mode + model ended on a tool_call: the
                            // last "round" has narration in `accumulated`
                            // and no further text will ever come. Finalize
                            // it inline so the dispatcher has nothing left
                            // to do.
                            if in_tool_phase && split_streaming {
                                finalize_split_round(
                                    &plugin, &account_id, &chat_id,
                                    &reply_to_message_id, thread_id.as_deref(), max_msg_len,
                                    &accumulated, draft_id, &mut preview_transport,
                                    &mut preview_message_id, &mut card_session,
                                    finalized_rounds, &round_texts, &capabilities,
                                ).await;
                                accumulated.clear();
                                preview_message_id = None;
                                card_session = None;
                                finalized_rounds += 1;
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

        StreamPreviewOutcome {
            preview,
            finalized_rounds,
        }
    })
}

/// Close the current round's preview and deliver its media. Called from
/// inside the stream task at split-streaming round boundaries (and at end
/// of stream when the model finished on a tool_call).
///
/// Delivery contract: always either ships the round's full narration via
/// the preview transport, or falls back to chunked `send_text_chunks`. The
/// preview path silently drops oversized text (`build_stream_preview_payload`
/// returns `None` when `text.len() > max_msg_len`) and turns transient
/// send/edit errors into log-only warnings, so the stream task can NOT
/// trust "preview ran" as proof of delivery. We detect that case explicitly
/// and fall back to chunked send so the dispatcher's `finalized_rounds`
/// skip is safe to act on.
///
/// Per transport:
/// - **Message**: if `accumulated` fits and the preview message exists,
///   the preview already wrote the final text; just drop `preview_message_id`.
///   Otherwise (oversized, or initial send never succeeded), chunk-send.
/// - **Card**: cardkit elements hold ~100k chars (`CARD_ELEMENT_MAX_CHARS`),
///   normally enough; if the session was never created or went broken,
///   chunk-send; either way close the card best-effort.
/// - **Draft**: drafts are typing-indicators, not real messages. Always
///   chunk-send (handles oversized text correctly via `chunk_message`).
///
/// Then deliver this round's media items (read from `round_texts.completed`,
/// where the sink stashed them on tool_result events).
#[allow(clippy::too_many_arguments)]
async fn finalize_split_round(
    plugin: &Arc<dyn ChannelPlugin>,
    account_id: &str,
    chat_id: &str,
    reply_to_message_id: &str,
    thread_id: Option<&str>,
    max_msg_len: usize,
    accumulated: &str,
    draft_id: i64,
    preview_transport: &mut StreamPreviewTransport,
    preview_message_id: &mut Option<String>,
    card_session: &mut Option<CardSession>,
    round_idx: usize,
    round_texts: &Arc<Mutex<RoundTextAccumulator>>,
    capabilities: &ChannelCapabilities,
) {
    // 1. Flush latest accumulated text into the preview (best-effort —
    //    oversized text returns None here and is rescued by the chunk
    //    fallback below; transient errors are log-only).
    if !accumulated.is_empty() {
        send_stream_preview(
            plugin,
            account_id,
            chat_id,
            reply_to_message_id,
            thread_id,
            max_msg_len,
            accumulated,
            draft_id,
            preview_transport,
            preview_message_id,
            card_session,
        )
        .await;
    }

    // 2. Decide whether the preview path actually carried this round's
    //    text. When it didn't, fall through to a chunked send so the
    //    user sees the full content.
    let preview_carried_text = preview_carried_full_text(
        *preview_transport,
        accumulated,
        plugin.markdown_to_native(accumulated).len(),
        preview_message_id.as_deref(),
        card_session.as_ref().map(|s| s.broken),
        max_msg_len,
    );

    if !preview_carried_text {
        // No preview message to edit (or it can't carry this size); send
        // fresh chunks. Pass `preview=None` so `send_text_chunks` doesn't
        // try to edit a broken/oversized preview.
        super::dispatcher::send_text_chunks(
            plugin,
            account_id,
            chat_id,
            thread_id,
            reply_to_message_id,
            accumulated,
            None,
        )
        .await;
    }

    // 3. Transport-specific close. Best-effort: any error here is
    //    cosmetic (the text is already delivered above), so log + continue.
    if let StreamPreviewTransport::Card = preview_transport {
        if let Some(session) = card_session.take() {
            if !session.broken {
                if let Err(e) = plugin
                    .close_card_stream(account_id, &session.card_id, session.sequence)
                    .await
                {
                    app_warn!(
                        "channel",
                        "worker",
                        "split-streaming close_card_stream failed (seq={}): {}",
                        session.sequence,
                        e
                    );
                }
            }
        }
    }
    *preview_message_id = None;
    *card_session = None;

    // 3. Deliver this round's media. The sink attached items to
    //    `round_texts.completed[round_idx]` on `tool_result` arrival.
    //    Dispatcher's end-of-turn `deliver_split` only iterates rounds
    //    past `finalized_rounds`, so this round's media won't be redelivered.
    let medias = {
        let guard = round_texts.lock().unwrap_or_else(|e| {
            app_warn!(
                "channel",
                "worker",
                "round_texts mutex poisoned in stream task: {}",
                e
            );
            e.into_inner()
        });
        guard.round_medias(round_idx)
    };
    if !medias.is_empty() {
        deliver_media_to_chat(
            plugin,
            account_id,
            chat_id,
            thread_id,
            &medias,
            capabilities,
        )
        .await;
    }
}

/// Pure helper for the split-streaming round-finalize delivery decision.
///
/// `accumulated_native_len` is the length of `markdown_to_native(accumulated)`
/// in bytes (matches what `build_stream_preview_payload` checks against
/// `max_msg_len`). `card_session_broken` is `Some(broken_flag)` if a card
/// session exists, `None` otherwise.
///
/// Returns `true` when the existing preview state has demonstrably carried
/// the full round narration — caller can stop. `false` means caller must
/// chunk-and-send `accumulated` itself; the preview either silently dropped
/// oversized content or never opened (initial send/edit error, oversized
/// from the first delta).
pub(super) fn preview_carried_full_text(
    transport: StreamPreviewTransport,
    accumulated: &str,
    accumulated_native_len: usize,
    preview_message_id: Option<&str>,
    card_session_broken: Option<bool>,
    max_msg_len: usize,
) -> bool {
    if accumulated.is_empty() {
        return true;
    }
    match transport {
        StreamPreviewTransport::Message => {
            preview_message_id.is_some() && accumulated_native_len <= max_msg_len
        }
        StreamPreviewTransport::Card => {
            // `Some(false)` = card exists and isn't broken
            matches!(card_session_broken, Some(false))
                && accumulated.chars().count() <= CARD_ELEMENT_MAX_CHARS
        }
        // Drafts are typing indicators, not real messages — always need a
        // real `send_message` (which the chunk fallback does, correctly
        // splitting oversized text).
        StreamPreviewTransport::Draft => false,
    }
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
    if raw_text.is_empty() || raw_text.chars().count() > CARD_ELEMENT_MAX_CHARS {
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

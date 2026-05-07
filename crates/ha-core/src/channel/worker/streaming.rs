use std::sync::Arc;
use tokio::sync::mpsc;

use crate::channel::traits::ChannelPlugin;
use crate::channel::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamPreviewTransport {
    Draft,
    Message,
}

#[derive(Debug, Default)]
pub struct StreamPreviewOutcome {
    pub preview_message_id: Option<String>,
}

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
pub fn spawn_channel_stream_task(
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

pub fn select_stream_preview_transport(
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

//! Live GUI / HTTP → IM streaming mirror. desktop / HTTP-triggered turns
//! that have an IM attach get rendered into the IM chat with the same
//! per-round typewriter UX as IM-inbound turns, driven by the account's
//! `imReplyMode` (`split` / `preview` / `final`).
//!
//! When attaching, if there's a user-message snapshot, this module emits
//! a standalone markdown blockquote (`> 💬 ...`) into the IM chat **before**
//! the stream pipeline starts. That keeps the quote at the top of the IM
//! exchange across all three reply modes — `split` fans out per-round
//! messages after it, `preview` grows a single message after it, `final`
//! sends one message after it. The quote is a separate IM message;
//! `messages.context_json` and the persisted assistant row stay clean of
//! it so context windows + desktop history are unaffected.

use std::sync::Arc;

use crate::channel::db::ChannelConversation;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::ChatType;
use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_rounds, spawn_stream_pipeline, DeliveryTarget, StreamPipeline,
};
use crate::channel::worker::send_text_chunks;
use crate::chat_engine::quote::{build_user_quote_prefix, LastUserView};
use crate::chat_engine::sink_registry::{sink_registry, SinkHandle};
use crate::chat_engine::stream_seq::ChatSource;

/// Owned snapshot of the user message that triggered a desktop / HTTP
/// turn. Captured at `attach_im_live_mirror` entry. Owned (not borrowed)
/// because callers may want to construct it from values they don't keep
/// alive across the await; in practice the engine builds it inline.
#[derive(Debug, Clone)]
pub struct LastUserSnapshot {
    pub source: String,
    pub text: String,
    pub attachment_count: usize,
}

pub(crate) struct ImLiveMirrorState {
    sink_handle: SinkHandle,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    attach: ChannelConversation,
    /// Whether `attach` already pushed a user-quote message into the IM
    /// chat. Drives the abort path: if `true` and the engine cancels /
    /// fails before `finalize`, we follow up with an "interrupted"
    /// notice so the IM thread doesn't show a dangling quote with no
    /// answer underneath it.
    quote_sent: bool,
}

pub(crate) async fn attach_im_live_mirror(
    session_id: &str,
    source: ChatSource,
    last_user: Option<LastUserSnapshot>,
) -> Option<ImLiveMirrorState> {
    if !matches!(source, ChatSource::Desktop | ChatSource::Http) {
        return None;
    }

    let store = crate::config::cached_config();
    if store.channels.accounts.is_empty() {
        // Desktop-only deployments skip the SQL probe entirely.
        return None;
    }

    let channel_db = crate::globals::get_channel_db()?;
    let registry = crate::globals::get_channel_registry()?;

    let attach = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return None,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "get_conversation_by_session({}) failed: {}",
                session_id,
                e
            );
            return None;
        }
    };

    let account = store.channels.find_account(&attach.account_id)?.clone();
    let plugin = registry.get_plugin(&account.channel_id)?.clone();
    let chat_type = ChatType::from_lowercase(&attach.chat_type);

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
    };

    // Emit the user-message quote as its own IM message before the stream
    // pipeline opens. Awaited so the quote lands above the first stream
    // frame in `split` mode (where stream task starts fanning out per-round
    // messages immediately after this returns).
    let quote_view = last_user.as_ref().map(|s| LastUserView {
        source: s.source.as_str(),
        text: s.text.as_str(),
        attachment_count: s.attachment_count,
    });
    let mut quote_sent = false;
    if let Some(quote) = build_user_quote_prefix(quote_view.as_ref()) {
        let body = quote.trim_end();
        if !body.is_empty() {
            send_text_chunks(&plugin, &target, body, None).await;
            quote_sent = true;
        }
    }

    // The originating Desktop / Http turn already drives the
    // `chat:stream_delta` path; suppress the secondary sink's bus emit so
    // the GUI doesn't render every frame twice.
    let pipeline = spawn_stream_pipeline(&plugin, &account, &chat_type, session_id, &target, false);
    let sink_handle = sink_registry().attach(session_id.to_string(), pipeline.event_sink.clone());

    Some(ImLiveMirrorState {
        sink_handle,
        pipeline,
        plugin,
        attach,
        quote_sent,
    })
}

pub(crate) async fn finalize_im_live_mirror(state: ImLiveMirrorState, response: &str) {
    let ImLiveMirrorState {
        sink_handle,
        pipeline,
        plugin,
        attach,
        quote_sent: _,
    } = state;

    drop(sink_handle);

    let outcome = await_stream_pipeline(pipeline).await;

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
    };

    let metrics = deliver_rounds(&plugin, &target, &outcome, response).await;
    crate::app_info!(
        "channel",
        "mirror",
        "[{}] Mirrored GUI reply to {} (response_chars={}, delivered_text_chars={}, media={})",
        attach.channel_id,
        attach.chat_id,
        response.chars().count(),
        metrics.text_chars,
        metrics.media_count,
    );
}

/// Drain + clean up a live mirror without a final response. Called from
/// engine cancel / final-failure paths in place of `finalize_im_live_mirror`.
///
/// If `attach_im_live_mirror` already emitted a user-quote message into
/// the IM chat, follow up with a short "(answering interrupted)" notice
/// so the thread doesn't show a dangling quote with no answer underneath.
/// If no quote was sent (no `LastUserSnapshot`, or `build_user_quote_prefix`
/// returned `None`), there's nothing visible to orphan — drain the
/// pipeline and bail without polluting the chat.
///
/// Like `finalize`, drops the sink handle first so the stream task
/// observes channel-close cleanly, then awaits the pipeline.
pub(crate) async fn abort_im_live_mirror(state: ImLiveMirrorState) {
    let ImLiveMirrorState {
        sink_handle,
        pipeline,
        plugin,
        attach,
        quote_sent,
    } = state;

    drop(sink_handle);
    let _outcome = await_stream_pipeline(pipeline).await;

    if !quote_sent {
        return;
    }

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
    };
    send_text_chunks(&plugin, &target, "_(answering interrupted)_", None).await;
    crate::app_info!(
        "channel",
        "mirror",
        "[{}] Aborted GUI mirror to {} — followed up with interrupted notice",
        attach.channel_id,
        attach.chat_id,
    );
}

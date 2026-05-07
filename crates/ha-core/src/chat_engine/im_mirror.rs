//! Live GUI / HTTP → IM streaming mirror. desktop / HTTP-triggered turns
//! that have an IM attach get rendered into the IM chat with the same
//! per-round typewriter UX as IM-inbound turns, driven by the account's
//! `imReplyMode` (`split` / `preview` / `final`).
//!
//! When the engine finishes, the final assistant text passed to
//! [`finalize_im_live_mirror`] is prepended with a markdown blockquote
//! of the user message that triggered the turn (`build_user_quote_prefix`)
//! so the IM user has context for what's being answered. The quote is
//! **only** added to the IM-bound chunk — `messages.context_json` and the
//! persisted assistant row keep the unmodified text so future context
//! windows + desktop history aren't polluted.

use std::sync::Arc;

use crate::channel::db::ChannelConversation;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::ChatType;
use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_rounds, spawn_stream_pipeline, DeliveryTarget, StreamPipeline,
};
use crate::chat_engine::quote::{build_user_quote_prefix, LastUserView};
use crate::chat_engine::sink_registry::{sink_registry, SinkHandle};
use crate::chat_engine::stream_seq::ChatSource;

/// Owned snapshot of the user message that triggered a desktop / HTTP
/// turn. Captured at `attach_im_live_mirror` entry and consumed in
/// `finalize_im_live_mirror`. Owned (not borrowed) because the state
/// outlives the engine's local `message` / `attachments` borrows — it
/// crosses await points until the engine returns.
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
    last_user: Option<LastUserSnapshot>,
}

pub(crate) fn attach_im_live_mirror(
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
        last_user,
    })
}

pub(crate) async fn finalize_im_live_mirror(state: ImLiveMirrorState, response: &str) {
    let ImLiveMirrorState {
        sink_handle,
        pipeline,
        plugin,
        attach,
        last_user,
    } = state;

    drop(sink_handle);

    let outcome = await_stream_pipeline(pipeline).await;

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
    };

    let view = last_user.as_ref().map(|s| LastUserView {
        source: s.source.as_str(),
        text: s.text.as_str(),
        attachment_count: s.attachment_count,
    });
    let prefix = build_user_quote_prefix(view.as_ref()).unwrap_or_default();
    let prefixed: String;
    let response_for_delivery: &str = if prefix.is_empty() {
        response
    } else {
        prefixed = format!("{prefix}{response}");
        prefixed.as_str()
    };

    deliver_rounds(&plugin, &target, &outcome, response_for_delivery).await;
}

//! Live GUI / HTTP → IM streaming mirror. desktop / HTTP-triggered turns
//! that have an IM attach get rendered into the IM chat with the same
//! per-round typewriter UX as IM-inbound turns, driven by the account's
//! `imReplyMode` (`split` / `preview` / `final`).

use std::sync::Arc;

use crate::channel::db::ChannelConversation;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::ChatType;
use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_rounds, spawn_stream_pipeline, DeliveryTarget, StreamPipeline,
};
use crate::chat_engine::sink_registry::{sink_registry, SinkHandle};
use crate::chat_engine::stream_seq::ChatSource;

pub(crate) struct ImLiveMirrorState {
    sink_handle: SinkHandle,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    attach: ChannelConversation,
}

pub(crate) fn attach_im_live_mirror(
    session_id: &str,
    source: ChatSource,
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
    })
}

pub(crate) async fn finalize_im_live_mirror(state: ImLiveMirrorState, response: &str) {
    let ImLiveMirrorState {
        sink_handle,
        pipeline,
        plugin,
        attach,
    } = state;

    drop(sink_handle);

    let outcome = await_stream_pipeline(pipeline).await;

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
    };

    deliver_rounds(&plugin, &target, &outcome, response).await;
}

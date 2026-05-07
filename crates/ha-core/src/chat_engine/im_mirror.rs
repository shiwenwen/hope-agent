//! GUI → IM live-stream mirror — secondary `ChannelStreamSink`s that mirror
//! a desktop / web chat turn through the `SinkRegistry` to whichever IM
//! chat is currently primary-attached to the same session.
//!
//! Lifecycle:
//! 1. `attach_im_mirrors(session_id, source)` runs at the start of
//!    `run_chat_engine` for `Desktop` / `Http` turns. For each primary
//!    attach row it:
//!    - resolves the channel plugin
//!    - spawns a `spawn_channel_stream_task` (handles progressive
//!      preview-message editing on the IM side)
//!    - registers a `ChannelStreamSink` with the global `SinkRegistry`
//!      so `emit_stream_event` fans every event to it
//! 2. Mirror events flow naturally — `emit_stream_event` forwards to
//!    every sink in the registry; each `ChannelStreamSink` push events
//!    into its own `event_tx` which the per-mirror stream task drains
//!    into preview-message edits.
//! 3. `finalize_im_mirrors(state, response)` runs at the success path
//!    of `run_chat_engine`. It drops every mirror's `event_tx` (so each
//!    stream task observes channel-close, finishes its progressive
//!    work, and returns the resulting `preview_message_id`), awaits
//!    those join handles, and then either edits the preview into the
//!    final response or sends the response fresh — same fork dispatcher
//!    uses for IM-driven turns.
//!
//! IM-driven turns (`ChatSource::Channel`) skip this entirely; the
//! dispatcher already handles preview / final delivery for the
//! primary attach. Background turns (`Subagent` / `ParentInjection`)
//! also skip — they shouldn't surface in user IM chats.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::attachments::MediaItem;
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ChatType, ParseMode, ReplyPayload};
use crate::channel::worker::{
    select_stream_preview_transport, spawn_channel_stream_task, StreamPreviewOutcome,
};
use crate::chat_engine::sink_registry::{sink_registry, SinkHandle};
use crate::chat_engine::stream_seq::ChatSource;
use crate::chat_engine::types::{ChannelStreamSink, EventSink};

/// One mirror tied to a single primary attach row. Field order is
/// load-bearing: `_sink_handle` drops before `stream_handle` so the
/// sink leaves the registry before the join handle is dropped (which
/// closes the spawn channel and lets the task exit cleanly).
pub(crate) struct ImMirror {
    plugin: Arc<dyn ChannelPlugin>,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
    stream_handle: JoinHandle<StreamPreviewOutcome>,
    /// Held alive for the duration of the chat turn. Drop detaches from
    /// the registry; we explicitly drop senders in `finalize` so the
    /// stream task observes channel-close and returns a preview id we
    /// can edit.
    _sink_handle: SinkHandle,
}

/// Aggregate state held by `run_chat_engine` for the duration of a
/// `Desktop` / `Http` turn. Default is empty — IM-only or background
/// turns end up with this and skip both attach + finalize fast paths.
#[derive(Default)]
pub(crate) struct ImMirrorState {
    mirrors: Vec<ImMirror>,
    /// Cloned `event_tx` per mirror. Dropping these signals the per-mirror
    /// stream task that no more events are coming, so it can flush its
    /// last preview edit and return.
    event_txs: Vec<mpsc::Sender<String>>,
}

impl ImMirrorState {
    pub(crate) fn is_empty(&self) -> bool {
        self.mirrors.is_empty()
    }
}

// On Drop, `event_txs` and the sink handles inside `mirrors` close out
// — error paths therefore clean up implicitly without a manual cancel
// step in `run_chat_engine`.

/// Walk the session's primary IM attaches and spawn a streaming mirror
/// for each. No-op for non-`Desktop` / non-`Http` turns or when channel
/// globals aren't initialised. Per-attach failures are logged and
/// skipped so one broken plugin can't disable mirrors for the others.
pub(crate) fn attach_im_mirrors(session_id: &str, source: ChatSource) -> ImMirrorState {
    if !matches!(source, ChatSource::Desktop | ChatSource::Http) {
        return ImMirrorState::default();
    }
    let Some(channel_db) = crate::globals::get_channel_db() else {
        return ImMirrorState::default();
    };
    let Some(registry) = crate::globals::get_channel_registry() else {
        return ImMirrorState::default();
    };

    // Cheap EXISTS probe before materialising rows + spinning tasks
    // — the no-attach case (95%+ of desktop turns) avoids the ORDER BY
    // and the per-attach plugin lookup.
    match channel_db.has_attached(session_id) {
        Ok(false) => return ImMirrorState::default(),
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "has_attached({}) failed: {}",
                session_id,
                e
            );
            return ImMirrorState::default();
        }
        Ok(true) => {}
    }

    let attaches = match channel_db.list_attached(session_id) {
        Ok(v) => v,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "list_attached({}) failed: {}",
                session_id,
                e
            );
            return ImMirrorState::default();
        }
    };

    let mut state = ImMirrorState::default();
    for attach in attaches.iter() {
        if !attach.is_primary {
            // Observers stay silent (plan: only primary receives outbound).
            continue;
        }

        let channel_id_typed: crate::channel::types::ChannelId =
            match serde_json::from_value(serde_json::Value::String(attach.channel_id.clone())) {
                Ok(c) => c,
                Err(e) => {
                    crate::app_warn!(
                        "channel",
                        "mirror",
                        "Unknown channel id {} on attach: {}",
                        attach.channel_id,
                        e
                    );
                    continue;
                }
            };

        let plugin = match registry.get_plugin(&channel_id_typed) {
            Some(p) => p,
            None => continue,
        };

        let chat_type_enum = ChatType::from_lowercase(&attach.chat_type);
        let capabilities = plugin.capabilities();
        let preview_transport = select_stream_preview_transport(&chat_type_enum, &capabilities);
        let max_msg_len = capabilities.max_message_length.unwrap_or(4096);

        let (event_tx, event_rx) = mpsc::channel::<String>(512);
        let pending_media = Arc::new(Mutex::new(Vec::<MediaItem>::new()));

        // GUI handover has no inbound message — pass empty string so the
        // plugin treats this as a fresh send rather than a reply.
        let stream_handle = spawn_channel_stream_task(
            event_rx,
            plugin.clone(),
            attach.account_id.clone(),
            attach.chat_id.clone(),
            String::new(),
            attach.thread_id.clone(),
            preview_transport,
            max_msg_len,
        );

        let sink: Arc<dyn EventSink> = Arc::new(ChannelStreamSink::with_primary(
            session_id.to_string(),
            event_tx.clone(),
            pending_media,
            true,
        ));
        let sink_handle = sink_registry().attach(session_id.to_string(), sink);

        state.mirrors.push(ImMirror {
            plugin: plugin.clone(),
            account_id: attach.account_id.clone(),
            chat_id: attach.chat_id.clone(),
            thread_id: attach.thread_id.clone(),
            stream_handle,
            _sink_handle: sink_handle,
        });
        state.event_txs.push(event_tx);
    }
    state
}

/// Convert mirrors into final-reply delivery: drop all event senders so
/// stream tasks finish, await each preview message id, then either
/// `edit_message` (preview existed) or `send_message` (no preview) for
/// chunk #0 and `send_message` for the remaining chunks.
///
/// Always best-effort — per-mirror plugin failures are logged and the
/// caller never sees the error. The chat engine has already persisted
/// the assistant response, so a missed mirror means a missed echo,
/// not a missed turn.
pub(crate) async fn finalize_im_mirrors(state: ImMirrorState, response: &str) {
    let ImMirrorState {
        mirrors,
        event_txs,
    } = state;
    // Drop every sender so each stream task sees channel-close.
    drop(event_txs);

    // Run mirrors in parallel — preview-message edits + chunk
    // sends to one IM platform shouldn't block delivery to another.
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::with_capacity(mirrors.len());
    for mirror in mirrors {
        let response = response.to_string();
        tasks.push(tokio::spawn(async move {
            finalize_one_mirror(mirror, &response).await;
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
}

async fn finalize_one_mirror(mirror: ImMirror, response: &str) {
    let outcome = match mirror.stream_handle.await {
        Ok(o) => o,
        Err(e) => {
            crate::app_warn!(
                "channel",
                "mirror",
                "Stream task join failed for {}/{}: {}",
                mirror.account_id,
                mirror.chat_id,
                e
            );
            return;
        }
    };

    let native_text = mirror.plugin.markdown_to_native(response);
    let chunks = mirror.plugin.chunk_message(&native_text);

    for (i, chunk) in chunks.iter().enumerate() {
        let payload = ReplyPayload {
            text: Some(chunk.clone()),
            thread_id: mirror.thread_id.clone(),
            parse_mode: Some(ParseMode::Html),
            ..ReplyPayload::text("")
        };
        let result = deliver_chunk(&mirror, i, &payload, outcome.preview_message_id.as_deref()).await;
        if let Err(e) = result {
            crate::app_warn!(
                "channel",
                "mirror",
                "Mirror final-reply chunk {} to {}/{} failed: {}",
                i,
                mirror.account_id,
                mirror.chat_id,
                e
            );
        }
    }
}

/// First chunk replaces the preview if one exists; everything else is a
/// plain send. Edit failures fall back to send so a dropped preview
/// doesn't lose the response.
async fn deliver_chunk(
    mirror: &ImMirror,
    index: usize,
    payload: &ReplyPayload,
    preview_id: Option<&str>,
) -> anyhow::Result<crate::channel::types::DeliveryResult> {
    if index == 0 {
        if let Some(preview_id) = preview_id {
            match mirror
                .plugin
                .edit_message(&mirror.account_id, &mirror.chat_id, preview_id, payload)
                .await
            {
                Ok(r) => return Ok(r),
                Err(e) => {
                    crate::app_warn!(
                        "channel",
                        "mirror",
                        "edit_message failed for {}/{}, falling back to send: {}",
                        mirror.account_id,
                        mirror.chat_id,
                        e
                    );
                }
            }
        }
    }
    mirror
        .plugin
        .send_message(&mirror.account_id, &mirror.chat_id, payload)
        .await
}

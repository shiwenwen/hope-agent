//! Shared streaming + delivery pipeline used by both IM-inbound turns
//! ([`super::dispatcher::handle_inbound_message`]) and GUI / HTTP live
//! mirror ([`crate::chat_engine::im_mirror`]). Owning the spawn-task,
//! await, drain, and `ImReplyMode`-driven fan-out in one place keeps the
//! two paths from drifting.

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::dispatcher::{
    deliver_final_only, deliver_preview_merged, deliver_split, DeliveryMetrics,
};
use super::streaming::{
    select_stream_preview_transport, spawn_channel_stream_task, StreamPreviewOutcome,
};
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ChannelAccountConfig, ChannelCapabilities, ChatType, ImReplyMode};
use crate::chat_engine::{ChannelStreamSink, EventSink, RoundOutput, RoundTextAccumulator};

/// Coordinates of one IM chat the pipeline writes to. All fields are
/// borrowed; callers store the owned forms (in `MsgContext` for inbound
/// or `ImLiveMirrorState` for live mirror) and build a `DeliveryTarget`
/// at the boundary.
pub(crate) struct DeliveryTarget<'a> {
    pub account_id: &'a str,
    pub chat_id: &'a str,
    pub thread_id: Option<&'a str>,
    /// `None` when there's no inbound message to reply to (live mirror).
    pub reply_to_message_id: Option<&'a str>,
}

/// Handles to a running stream pipeline. The caller plugs `event_sink`
/// into the chat engine, then hands the rest back to
/// [`await_stream_pipeline`] when the engine returns.
pub(crate) struct StreamPipeline {
    pub event_sink: Arc<dyn EventSink>,
    stream_task: JoinHandle<StreamPreviewOutcome>,
    round_texts: Arc<Mutex<RoundTextAccumulator>>,
    reply_mode: ImReplyMode,
    capabilities: ChannelCapabilities,
    preview_active: bool,
}

/// Drained outputs from a finished pipeline. Borrow into
/// [`deliver_rounds`] for the success path; inspect `stream_outcome`
/// directly on the error path (the preview handle is needed to commit a
/// half-rendered preview into a fallback error message).
pub(crate) struct PipelineOutcome {
    pub(super) stream_outcome: StreamPreviewOutcome,
    pub(super) drained_rounds: Vec<RoundOutput>,
    pub(super) reply_mode: ImReplyMode,
    pub(super) capabilities: ChannelCapabilities,
    pub(super) preview_active: bool,
}

/// Spawn the IM streaming-preview task and build a `ChannelStreamSink`
/// for the chat engine to write into. Honors the account's `imReplyMode`
/// and `showThinking`.
///
/// `broadcast_to_bus` controls whether the sink also re-emits each event
/// on the `channel:stream_delta` EventBus topic. Inbound IM turns set it
/// to true so the GUI can mirror the IM session live; the GUI / HTTP →
/// IM live mirror sets it to false because the originating turn already
/// drives `chat:stream_delta` (re-emitting would double-render the same
/// frames in the desktop view of an IM-attached session).
pub(crate) fn spawn_stream_pipeline(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    chat_type: &ChatType,
    session_id: &str,
    target: &DeliveryTarget<'_>,
    broadcast_to_bus: bool,
) -> StreamPipeline {
    let reply_mode = account.im_reply_mode();
    let capabilities = plugin.capabilities();
    let max_msg_len = capabilities.max_message_length.unwrap_or(4096);
    let preview_transport = match reply_mode {
        ImReplyMode::Preview | ImReplyMode::Split => {
            select_stream_preview_transport(chat_type, &capabilities)
        }
        ImReplyMode::Final => None,
    };
    let preview_active = preview_transport.is_some();

    // `EventSink::send` is synchronous. A bounded `try_send` would silently
    // drop bursty text deltas while the preview task awaits IM network IO,
    // which can make split-mode inline finalization skip incomplete text.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<String>();
    let round_texts = Arc::new(Mutex::new(RoundTextAccumulator::default()));

    let stream_task = spawn_channel_stream_task(
        event_rx,
        plugin.clone(),
        target.account_id.to_string(),
        target.chat_id.to_string(),
        target.reply_to_message_id.map(str::to_string),
        target.thread_id.map(str::to_string),
        preview_transport,
        max_msg_len,
        reply_mode,
        round_texts.clone(),
        capabilities.clone(),
    );

    let event_sink: Arc<dyn EventSink> = Arc::new(ChannelStreamSink::new(
        session_id.to_string(),
        event_tx,
        round_texts.clone(),
        account.show_thinking(),
        broadcast_to_bus,
    ));

    StreamPipeline {
        event_sink,
        stream_task,
        round_texts,
        reply_mode,
        capabilities,
        preview_active,
    }
}

/// Await the spawned stream task and drain the round accumulator. Mutex
/// poison is treated as recoverable — same contract as the inbound path.
///
/// The pipeline's `event_sink` Arc must be dropped **before** awaiting
/// `stream_task`: the spawned task's mpsc receiver only sees EOF once
/// every clone of its sender (held inside `event_sink`) has been
/// released, so awaiting while we still hold one would deadlock the
/// caller indefinitely. The engine path released its own clone when
/// `run_chat_engine` returned; releasing ours here unblocks the await.
pub(crate) async fn await_stream_pipeline(pipeline: StreamPipeline) -> PipelineOutcome {
    let StreamPipeline {
        event_sink,
        stream_task,
        round_texts,
        reply_mode,
        capabilities,
        preview_active,
    } = pipeline;
    drop(event_sink);

    let stream_outcome = match stream_task.await {
        Ok(outcome) => outcome,
        Err(e) => {
            crate::app_warn!("channel", "worker", "Streaming preview task failed: {}", e);
            StreamPreviewOutcome::default()
        }
    };

    let drained_rounds: Vec<RoundOutput> = {
        let mut guard = round_texts.lock().unwrap_or_else(|e| {
            crate::app_warn!("channel", "worker", "round_texts poisoned: {}", e);
            e.into_inner()
        });
        guard.drain()
    };

    PipelineOutcome {
        stream_outcome,
        drained_rounds,
        reply_mode,
        capabilities,
        preview_active,
    }
}

/// Fan a finished outcome into the IM channel per `ImReplyMode`.
pub(crate) async fn deliver_rounds(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    outcome: &PipelineOutcome,
    response: &str,
) -> DeliveryMetrics {
    match outcome.reply_mode {
        ImReplyMode::Split => {
            deliver_split(
                plugin,
                target,
                &outcome.drained_rounds,
                response,
                outcome.stream_outcome.preview.as_ref(),
                outcome.stream_outcome.finalized_rounds,
                &outcome.capabilities,
            )
            .await
        }
        ImReplyMode::Final => {
            deliver_final_only(
                plugin,
                target,
                &outcome.drained_rounds,
                response,
                &outcome.capabilities,
            )
            .await
        }
        ImReplyMode::Preview => {
            deliver_preview_merged(
                plugin,
                target,
                &outcome.drained_rounds,
                response,
                outcome.stream_outcome.preview.as_ref(),
                &outcome.capabilities,
            )
            .await
        }
    }
}

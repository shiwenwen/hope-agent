//! Shared streaming + delivery pipeline used by both IM-inbound turns
//! ([`super::dispatcher::handle_inbound_message`]) and GUI / HTTP live
//! mirror ([`crate::im_mirror`]). Owning the spawn-task,
//! await, drain, and `ImReplyMode`-driven fan-out in one place keeps the
//! two paths from drifting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::dispatcher::{
    deliver_final_only, deliver_preview_merged, deliver_split, send_error_reply, send_final_reply,
    DeliveryMetrics, DeliveryReport,
};
use super::streaming::{
    abort_native_preview, select_stream_preview_transport, spawn_channel_stream_task,
    PreviewHandle, StreamPreviewOutcome,
};
use ha_core::channel::traits::ChannelPlugin;
use ha_core::channel::types::{
    ChannelAccountConfig, ChannelCapabilities, ChatType, ImReplyMode, ReplyAbortReason,
    ReplyStreamPreviewPersistence, ReplyStreamTarget,
};
use ha_core::chat_engine::{ChannelStreamSink, EventSink, RoundOutput, RoundTextAccumulator};

/// Coordinates of one IM chat the pipeline writes to. All fields are
/// borrowed; callers store the owned forms (in `MsgContext` for inbound
/// or `ImLiveMirrorState` for live mirror) and build a `DeliveryTarget`
/// at the boundary.
pub(crate) struct DeliveryTarget<'a> {
    pub account_id: &'a str,
    pub chat_id: &'a str,
    pub chat_type: &'a ChatType,
    pub thread_id: Option<&'a str>,
    /// `None` when there's no inbound message to reply to (live mirror).
    pub reply_to_message_id: Option<&'a str>,
    pub recipient_user_id: Option<&'a str>,
    pub recipient_tenant_id: Option<&'a str>,
}

impl DeliveryTarget<'_> {
    pub(crate) fn to_reply_stream_target(&self) -> ReplyStreamTarget {
        ReplyStreamTarget {
            account_id: self.account_id.to_string(),
            chat_id: self.chat_id.to_string(),
            chat_type: self.chat_type.clone(),
            thread_id: self.thread_id.map(str::to_string),
            reply_to_message_id: self.reply_to_message_id.map(str::to_string),
            recipient_user_id: self.recipient_user_id.map(str::to_string),
            recipient_tenant_id: self.recipient_tenant_id.map(str::to_string),
        }
    }
}

impl<'a> From<&'a ReplyStreamTarget> for DeliveryTarget<'a> {
    fn from(target: &'a ReplyStreamTarget) -> Self {
        Self {
            account_id: &target.account_id,
            chat_id: &target.chat_id,
            chat_type: &target.chat_type,
            thread_id: target.thread_id.as_deref(),
            reply_to_message_id: target.reply_to_message_id.as_deref(),
            recipient_user_id: target.recipient_user_id.as_deref(),
            recipient_tenant_id: target.recipient_tenant_id.as_deref(),
        }
    }
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
    native_preview: Option<PreviewHandle>,
    native_preview_relinquished: Option<Arc<AtomicBool>>,
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
    session_id: &str,
    target: &DeliveryTarget<'_>,
    history_complete: bool,
    broadcast_to_bus: bool,
) -> StreamPipeline {
    let reply_mode = account.im_reply_mode();
    let capabilities = plugin.capabilities();
    let max_msg_len = capabilities.streaming_preview_max_bytes.unwrap_or(4096);
    let preview_transport = match reply_mode {
        ImReplyMode::Preview | ImReplyMode::Split => select_stream_preview_transport(
            &target.to_reply_stream_target(),
            &capabilities,
            history_complete,
        ),
        ImReplyMode::Final => None,
    };
    let preview_active = preview_transport.is_some();
    let native_preview = preview_transport
        .as_ref()
        .and_then(|transport| transport.native_preview_handle());
    let native_preview_relinquished = preview_transport
        .as_ref()
        .and_then(|transport| transport.native_preview_relinquished_flag());

    // `EventSink::send` is synchronous. A bounded `try_send` would silently
    // drop bursty text deltas while the preview task awaits IM network IO,
    // which can make split-mode inline finalization skip incomplete text.
    let (event_tx, event_rx) = mpsc::unbounded_channel::<String>();
    // Out-of-band channel for friendly status notices (model_fallback /
    // profile_rotation / context_compacted / thinking_auto_disabled). Kept
    // separate from `event_tx` so notices ship as standalone IM messages
    // and don't mix into the per-round LLM text accumulator or the
    // typewriter preview.
    let (system_notice_tx, system_notice_rx) = mpsc::unbounded_channel::<String>();
    let round_texts = Arc::new(Mutex::new(RoundTextAccumulator::default()));

    let stream_task = spawn_channel_stream_task(
        event_rx,
        system_notice_rx,
        plugin.clone(),
        target.to_reply_stream_target(),
        preview_transport,
        max_msg_len,
        reply_mode,
        round_texts.clone(),
        capabilities.clone(),
    );

    let event_sink: Arc<dyn EventSink> = Arc::new(ChannelStreamSink::new(
        session_id.to_string(),
        event_tx,
        system_notice_tx,
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
        native_preview,
        native_preview_relinquished,
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
        native_preview,
        native_preview_relinquished,
    } = pipeline;
    drop(event_sink);

    pipeline_outcome(
        stream_task.await,
        round_texts,
        reply_mode,
        capabilities,
        preview_active,
        native_preview,
        native_preview_relinquished,
    )
    .await
}

/// Await an inbound pipeline while the owning Channel turn remains live.
/// A provider/tool cancellation can otherwise finish while the preview task is
/// still blocked in an IM API request, keeping the session single-flight guard
/// occupied indefinitely. Aborting leaves the already-rendered partial preview
/// in place, which is the Channel visual contract for a stopped turn.
pub(crate) async fn await_stream_pipeline_until_cancel(
    pipeline: StreamPipeline,
    cancel: &AtomicBool,
) -> Option<PipelineOutcome> {
    let StreamPipeline {
        event_sink,
        mut stream_task,
        round_texts,
        reply_mode,
        capabilities,
        preview_active,
        native_preview,
        native_preview_relinquished,
    } = pipeline;
    drop(event_sink);

    let join_result = tokio::select! {
        biased;
        _ = async {
            while !cancel.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        } => {
            let mut detach_native_task = false;
            if let Some(preview) = native_preview.as_ref() {
                let abort = abort_native_preview(preview, ReplyAbortReason::Cancelled);
                let safely_aborted = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    abort,
                )
                .await;
                if !matches!(safely_aborted, Ok(true)) {
                    app_warn!(
                        "channel",
                        "worker",
                        "Native reply abort did not settle safely after cancellation; detaching stream task"
                    );
                    // A timeout or explicit unsafe result means a provider
                    // mutation may still own the session lock/handle. Detach
                    // the task so it can observe Terminal and hand off abort
                    // after the in-flight open/push returns.
                    detach_native_task = true;
                }
            }
            if !detach_native_task {
                stream_task.abort();
            }
            return None;
        }
        result = &mut stream_task => result,
    };

    Some(
        pipeline_outcome(
            join_result,
            round_texts,
            reply_mode,
            capabilities,
            preview_active,
            native_preview,
            native_preview_relinquished,
        )
        .await,
    )
}

async fn pipeline_outcome(
    join_result: Result<StreamPreviewOutcome, tokio::task::JoinError>,
    round_texts: Arc<Mutex<RoundTextAccumulator>>,
    reply_mode: ImReplyMode,
    capabilities: ChannelCapabilities,
    preview_active: bool,
    native_preview: Option<PreviewHandle>,
    native_preview_relinquished: Option<Arc<AtomicBool>>,
) -> PipelineOutcome {
    let stream_outcome = match join_result {
        Ok(outcome) => outcome,
        Err(e) => {
            app_warn!("channel", "worker", "Streaming preview task failed: {}", e);
            if native_preview_relinquished
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Acquire))
            {
                // The task crossed from native into a legacy transport, whose
                // message/card identity and delivery ledger live inside that
                // task. A panic loses those facts. Never revive the stale
                // native handle or open a standalone final reply.
                StreamPreviewOutcome {
                    delivery_report: DeliveryReport {
                        attempted: 1,
                        succeeded: 0,
                        failures: vec![format!(
                            "streaming preview task failed after native preview was relinquished; delivery state is unknown: {e}"
                        )],
                        unsafe_to_continue: true,
                    },
                    ..StreamPreviewOutcome::default()
                }
            } else if let Some(preview) = native_preview {
                // Preview infrastructure does not know the engine terminal
                // outcome. Preserve the native handle for both persistent
                // and ephemeral transports so eventual engine success can
                // claim FINAL, while engine failure/cancellation claims
                // ABORT. Pre-claiming here could swallow a successful reply.
                StreamPreviewOutcome {
                    preview: Some(preview),
                    ..StreamPreviewOutcome::default()
                }
            } else if preview_active {
                // A legacy preview task owns message/card identifiers and the
                // per-round delivery ledger internally. A panic loses that
                // evidence, so replaying from round zero could duplicate an
                // accepted mutation. Fail closed until the ledger is moved to
                // shared state rather than guessing that no preview existed.
                StreamPreviewOutcome {
                    delivery_report: DeliveryReport {
                        attempted: 1,
                        succeeded: 0,
                        failures: vec![format!(
                            "legacy streaming preview task failed; delivery state is unknown: {e}"
                        )],
                        unsafe_to_continue: true,
                    },
                    ..StreamPreviewOutcome::default()
                }
            } else {
                StreamPreviewOutcome::default()
            }
        }
    };

    let drained_rounds: Vec<RoundOutput> = {
        let mut guard = round_texts.lock().unwrap_or_else(|e| {
            app_warn!("channel", "worker", "round_texts poisoned: {}", e);
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
    if outcome.stream_outcome.delivery_report.unsafe_to_continue {
        let finalized = outcome
            .stream_outcome
            .finalized_rounds
            .min(outcome.drained_rounds.len());
        return DeliveryMetrics {
            text_chars: outcome.drained_rounds[..finalized]
                .iter()
                .map(|round| round.text.chars().count())
                .sum(),
            media_count: outcome.drained_rounds[..finalized]
                .iter()
                .map(|round| round.medias.len())
                .sum(),
            report: outcome.stream_outcome.delivery_report.clone(),
        };
    }
    let native_whole_turn = matches!(
        outcome.stream_outcome.preview.as_ref(),
        Some(PreviewHandle::Native { .. })
    );
    let mut metrics = if native_whole_turn {
        deliver_preview_merged(
            plugin,
            target,
            &outcome.drained_rounds,
            response,
            outcome.stream_outcome.preview.as_ref(),
            &outcome.capabilities,
        )
        .await
    } else {
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
    };
    metrics
        .report
        .merge(outcome.stream_outcome.delivery_report.clone());
    metrics
}

/// Explicitly terminate a native preview when a caller cannot enter normal
/// final delivery (attach moved, turn failed, or mirror aborted). Legacy
/// previews have no provider-owned terminal handle and remain unchanged.
pub(crate) async fn abort_pipeline_outcome(
    outcome: &PipelineOutcome,
    reason: ReplyAbortReason,
) -> bool {
    if let Some(preview) = outcome.stream_outcome.preview.as_ref() {
        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            abort_native_preview(preview, reason),
        )
        .await
        {
            Ok(safely_aborted) => safely_aborted,
            Err(_) => {
                // The provider terminal call owns the stream in a detached
                // task once the handle is available. Bound only the caller's
                // wait; never cancel or duplicate the remote mutation.
                app_warn!(
                    "channel",
                    "worker",
                    "Timed out waiting for native reply stream termination"
                );
                false
            }
        }
    } else {
        true
    }
}

/// Abort a pipeline specifically as a prerequisite for replaying the same
/// logical result. Unlike [`abort_pipeline_outcome`], a legacy preview or an
/// already-finalized split round is not considered safe merely because there
/// is no provider-owned native handle to abort: that content remains visible
/// and replay would create partial + full double delivery.
pub(crate) async fn abort_pipeline_outcome_for_replay(
    outcome: &PipelineOutcome,
    reason: ReplyAbortReason,
) -> bool {
    if outcome.stream_outcome.delivery_report.unsafe_to_continue {
        let _ = abort_pipeline_outcome(outcome, reason).await;
        return false;
    }

    match outcome.stream_outcome.preview.as_ref() {
        Some(PreviewHandle::Native {
            preview_persistence,
            ..
        }) => {
            let terminal_confirmed = abort_pipeline_outcome(outcome, reason).await;
            terminal_confirmed
                && matches!(
                    preview_persistence,
                    ReplyStreamPreviewPersistence::Ephemeral
                )
        }
        Some(PreviewHandle::Message { .. } | PreviewHandle::Card { .. }) => false,
        None => {
            outcome.stream_outcome.finalized_rounds == 0
                && outcome.stream_outcome.delivery_report.succeeded == 0
        }
    }
}

/// Whether a confirmed, in-place error terminal removed the old partial well
/// enough for the same logical result to be retried later. Message/Card
/// terminals replace their preview, and ephemeral native previews expire.
/// Persistent append streams (Slack) only stop: their old markdown remains
/// visible even when the provider acknowledges the terminal mutation.
pub(crate) fn error_terminal_allows_replay(outcome: &PipelineOutcome) -> bool {
    !matches!(
        outcome.stream_outcome.preview.as_ref(),
        Some(PreviewHandle::Native {
            preview_persistence: ReplyStreamPreviewPersistence::Persistent,
            ..
        })
    )
}

/// Finalize a failed turn through the same preview identity used while it was
/// streaming. A prior ambiguous legacy/native mutation is a turn-wide fuse:
/// terminate a provider-owned native handle if possible, but never open a
/// fresh error message that could race or duplicate the unknown outcome.
pub(crate) async fn deliver_error_reply(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    outcome: &PipelineOutcome,
    error_text: &str,
) -> DeliveryReport {
    let mut report = outcome.stream_outcome.delivery_report.clone();
    if report.unsafe_to_continue {
        let _ = abort_pipeline_outcome(outcome, ReplyAbortReason::Failed).await;
        return report;
    }
    report.merge(
        send_error_reply(
            plugin,
            target,
            outcome.stream_outcome.preview.as_ref(),
            error_text,
        )
        .await,
    );
    report
}

/// Deliver a complete final response through a pipeline that may have
/// attached after the turn had already started. Unlike [`deliver_rounds`],
/// this intentionally ignores `drained_rounds` for text reconstruction:
/// those rounds only contain deltas observed after the late attach point.
/// The preview handle is still honored, so a half-rendered IM preview is
/// replaced with the complete final answer when possible.
pub(crate) async fn deliver_full_response(
    plugin: &Arc<dyn ChannelPlugin>,
    target: &DeliveryTarget<'_>,
    outcome: &PipelineOutcome,
    response: &str,
    media: &[ha_core::attachments::MediaItem],
) -> DeliveryMetrics {
    if outcome.stream_outcome.delivery_report.unsafe_to_continue {
        return DeliveryMetrics {
            text_chars: 0,
            media_count: 0,
            report: outcome.stream_outcome.delivery_report.clone(),
        };
    }
    let report = send_final_reply(
        plugin,
        target,
        response,
        outcome.stream_outcome.preview.as_ref(),
        media,
        &[],
        true,
        &outcome.capabilities,
    )
    .await;
    let mut metrics = DeliveryMetrics {
        text_chars: response.chars().count(),
        media_count: media.len(),
        report,
    };
    metrics
        .report
        .merge(outcome.stream_outcome.delivery_report.clone());
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_capabilities() -> ChannelCapabilities {
        ChannelCapabilities {
            chat_types: Vec::new(),
            supports_polls: false,
            supports_reactions: false,
            supports_draft: false,
            supports_edit: false,
            supports_unsend: false,
            supports_reply: false,
            supports_threads: false,
            supports_media: Vec::new(),
            supports_typing: false,
            supports_buttons: false,
            streaming_preview_max_bytes: None,
            supports_card_stream: false,
            native_reply: None,
        }
    }

    #[tokio::test]
    async fn cancelled_pipeline_does_not_wait_for_a_stuck_preview_task() {
        let pipeline = StreamPipeline {
            event_sink: Arc::new(ha_core::chat_engine::NoopEventSink),
            stream_task: tokio::spawn(async {
                std::future::pending::<()>().await;
                StreamPreviewOutcome::default()
            }),
            round_texts: Arc::new(Mutex::new(RoundTextAccumulator::default())),
            reply_mode: ImReplyMode::Final,
            capabilities: empty_capabilities(),
            preview_active: false,
            native_preview: None,
            native_preview_relinquished: None,
        };
        let cancel = AtomicBool::new(true);

        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            await_stream_pipeline_until_cancel(pipeline, &cancel),
        )
        .await
        .expect("cancelled pipeline must return promptly");

        assert!(outcome.is_none());
    }

    #[tokio::test]
    async fn legacy_preview_task_panic_fails_closed() {
        let pipeline = StreamPipeline {
            event_sink: Arc::new(ha_core::chat_engine::NoopEventSink),
            stream_task: tokio::spawn(async {
                panic!("synthetic preview failure");
            }),
            round_texts: Arc::new(Mutex::new(RoundTextAccumulator::default())),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
            native_preview: None,
            native_preview_relinquished: None,
        };

        let outcome = await_stream_pipeline(pipeline).await;

        assert!(outcome.stream_outcome.delivery_report.unsafe_to_continue);
        assert_eq!(outcome.stream_outcome.delivery_report.attempted, 1);
        assert!(outcome.stream_outcome.preview.is_none());
    }

    #[tokio::test]
    async fn relinquished_native_preview_task_panic_does_not_revive_native_handle() {
        let pipeline = StreamPipeline {
            event_sink: Arc::new(ha_core::chat_engine::NoopEventSink),
            stream_task: tokio::spawn(async {
                panic!("synthetic post-fallback failure");
            }),
            round_texts: Arc::new(Mutex::new(RoundTextAccumulator::default())),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
            native_preview: Some(PreviewHandle::Native {
                session: Arc::new(tokio::sync::Mutex::new(None)),
                state: Arc::new(std::sync::atomic::AtomicU8::new(
                    super::super::streaming::NATIVE_SELECTED,
                )),
                terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                preview_persistence:
                    ha_core::channel::types::ReplyStreamPreviewPersistence::Persistent,
            }),
            native_preview_relinquished: Some(Arc::new(AtomicBool::new(true))),
        };

        let outcome = await_stream_pipeline(pipeline).await;

        assert!(outcome.stream_outcome.delivery_report.unsafe_to_continue);
        assert_eq!(outcome.stream_outcome.delivery_report.attempted, 1);
        assert!(outcome.stream_outcome.preview.is_none());
        assert!(outcome.stream_outcome.delivery_report.failures[0]
            .contains("native preview was relinquished"));
    }

    #[tokio::test]
    async fn replay_requires_proof_that_no_legacy_partial_is_visible() {
        let base = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome::default(),
            drained_rounds: Vec::new(),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
        };
        assert!(abort_pipeline_outcome_for_replay(&base, ReplyAbortReason::Cancelled).await);

        let message_preview = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome {
                preview: Some(PreviewHandle::Message {
                    message_id: "visible".to_string(),
                }),
                ..StreamPreviewOutcome::default()
            },
            ..base
        };
        assert!(
            !abort_pipeline_outcome_for_replay(&message_preview, ReplyAbortReason::Cancelled).await
        );

        let finalized_split = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome {
                finalized_rounds: 1,
                ..StreamPreviewOutcome::default()
            },
            ..PipelineOutcome {
                stream_outcome: StreamPreviewOutcome::default(),
                drained_rounds: Vec::new(),
                reply_mode: ImReplyMode::Split,
                capabilities: empty_capabilities(),
                preview_active: true,
            }
        };
        assert!(
            !abort_pipeline_outcome_for_replay(&finalized_split, ReplyAbortReason::Cancelled).await
        );
    }

    #[tokio::test]
    async fn persistent_native_terminal_ack_is_not_replay_safe() {
        let persistent = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome {
                preview: Some(PreviewHandle::Native {
                    session: Arc::new(tokio::sync::Mutex::new(None)),
                    state: Arc::new(std::sync::atomic::AtomicU8::new(
                        super::super::streaming::NATIVE_ACTIVE,
                    )),
                    terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                    preview_persistence: ReplyStreamPreviewPersistence::Persistent,
                }),
                ..StreamPreviewOutcome::default()
            },
            drained_rounds: Vec::new(),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
        };

        assert!(abort_pipeline_outcome(&persistent, ReplyAbortReason::Cancelled).await);
        assert!(!error_terminal_allows_replay(&persistent));

        let replay_attempt = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome {
                preview: Some(PreviewHandle::Native {
                    session: Arc::new(tokio::sync::Mutex::new(None)),
                    state: Arc::new(std::sync::atomic::AtomicU8::new(
                        super::super::streaming::NATIVE_ACTIVE,
                    )),
                    terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                    preview_persistence: ReplyStreamPreviewPersistence::Persistent,
                }),
                ..StreamPreviewOutcome::default()
            },
            drained_rounds: Vec::new(),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
        };
        assert!(
            !abort_pipeline_outcome_for_replay(&replay_attempt, ReplyAbortReason::Cancelled).await
        );
    }

    #[tokio::test]
    async fn ephemeral_native_terminal_remains_replay_safe() {
        let outcome = PipelineOutcome {
            stream_outcome: StreamPreviewOutcome {
                preview: Some(PreviewHandle::Native {
                    session: Arc::new(tokio::sync::Mutex::new(None)),
                    state: Arc::new(std::sync::atomic::AtomicU8::new(
                        super::super::streaming::NATIVE_ACTIVE,
                    )),
                    terminal_owner: Arc::new(std::sync::atomic::AtomicU8::new(0)),
                    preview_persistence: ReplyStreamPreviewPersistence::Ephemeral,
                }),
                ..StreamPreviewOutcome::default()
            },
            drained_rounds: Vec::new(),
            reply_mode: ImReplyMode::Preview,
            capabilities: empty_capabilities(),
            preview_active: true,
        };

        assert!(error_terminal_allows_replay(&outcome));
        assert!(abort_pipeline_outcome_for_replay(&outcome, ReplyAbortReason::Cancelled).await);
    }
}

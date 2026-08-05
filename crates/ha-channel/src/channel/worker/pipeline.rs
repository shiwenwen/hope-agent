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
use super::provider_lane::{
    reserve_provider_lane, ProviderLaneLease, ProviderLaneTaskHold, ProviderLaneWaiter,
    ProviderMutationGuard,
};
use super::streaming::{
    abort_native_preview_with_lane, select_stream_preview_transport, spawn_channel_stream_task,
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

const MIRROR_ACCOUNT_READY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const MIRROR_ACCOUNT_READY_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Account-worker gate evaluated inside the provider lane.  Foreground
/// Desktop/HTTP mirrors can be created while startup is still connecting the
/// IM account; their engine sink starts buffering immediately while this gate
/// waits in the background.
pub(crate) enum PipelineAccountReadiness {
    RequireRunning { account_id: String },
    WaitForRunning { account_id: String },
}

impl PipelineAccountReadiness {
    fn remains_configured(&self) -> bool {
        let account_id = self.account_id();
        ha_core::config::cached_config()
            .channels
            .find_account(account_id)
            .is_some_and(|account| account.enabled)
    }

    async fn is_running_now(&self) -> bool {
        let account_id = match self {
            Self::RequireRunning { account_id } | Self::WaitForRunning { account_id } => account_id,
        };
        let Some(registry) = ha_core::globals::get_channel_registry() else {
            return false;
        };
        registry.health(account_id).await.is_running
    }

    async fn wait_until_running(&self) -> bool {
        if !matches!(self, Self::WaitForRunning { .. }) {
            return self.is_running_now().await;
        }
        let deadline = tokio::time::Instant::now() + MIRROR_ACCOUNT_READY_WAIT;
        loop {
            if !self.remains_configured() {
                return false;
            }
            if self.is_running_now().await {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep(MIRROR_ACCOUNT_READY_POLL.min(deadline - now)).await;
        }
    }

    fn account_id(&self) -> &str {
        match self {
            Self::RequireRunning { account_id } | Self::WaitForRunning { account_id } => account_id,
        }
    }
}

/// Prefix work that must happen before a mirror may mutate its provider
/// target.  Deltas and system notices can enter the pipeline while this waits;
/// the unbounded receivers retain them until the quote/prefix has landed.
pub(crate) struct StreamPipelinePrelude {
    lane_waiter: ProviderLaneWaiter,
    provider_guard: ProviderMutationGuard,
    readiness: Option<PipelineAccountReadiness>,
    initial_notices: Vec<String>,
    quote: Option<String>,
    quote_sent: Arc<AtomicBool>,
}

impl StreamPipelinePrelude {
    fn ordered_only(provider_lane: &ProviderLaneLease) -> Self {
        Self::new(
            provider_lane.waiter(),
            provider_lane.task_hold(),
            None,
            Vec::new(),
            None,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|| true),
        )
    }

    pub(crate) fn new(
        lane_waiter: ProviderLaneWaiter,
        lane_task_hold: ProviderLaneTaskHold,
        readiness: Option<PipelineAccountReadiness>,
        initial_notices: Vec<String>,
        quote: Option<String>,
        quote_sent: Arc<AtomicBool>,
        still_valid: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            provider_guard: ProviderMutationGuard::new(
                lane_waiter.clone(),
                lane_task_hold,
                still_valid,
            ),
            lane_waiter,
            readiness,
            initial_notices,
            quote,
            quote_sent,
        }
    }

    fn provider_guard(&self) -> ProviderMutationGuard {
        self.provider_guard.clone()
    }

    pub(super) async fn run(
        self,
        plugin: &Arc<dyn ChannelPlugin>,
        target: &ha_core::channel::types::ReplyStreamTarget,
    ) -> (DeliveryReport, ProviderMutationGuard) {
        let Self {
            lane_waiter,
            provider_guard,
            readiness,
            initial_notices,
            quote,
            quote_sent,
        } = self;
        // Start account readiness and predecessor waiting together. A failed
        // readiness/validity check returns immediately even if the predecessor
        // is stuck. The linked lane chain retains this cancelled node until
        // that predecessor completes, so successors still cannot overtake it.
        let readiness_wait = async {
            match readiness.as_ref() {
                Some(readiness) => readiness.wait_until_running().await,
                None => true,
            }
        };
        tokio::pin!(readiness_wait);
        let lane_wait = lane_waiter.wait_turn();
        tokio::pin!(lane_wait);
        let mut readiness_complete = false;
        let mut lane_complete = false;
        let mut validity_tick = tokio::time::interval(std::time::Duration::from_millis(100));
        validity_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while !readiness_complete || !lane_complete {
            tokio::select! {
                ready = &mut readiness_wait, if !readiness_complete => {
                    if !ready {
                        let account = readiness
                            .as_ref()
                            .map(PipelineAccountReadiness::account_id)
                            .unwrap_or("unknown");
                        return (
                            DeliveryReport {
                                failures: vec![format!(
                                    "IM account {account} was not running at mirror delivery time"
                                )],
                                unsafe_to_continue: true,
                                ..DeliveryReport::default()
                            },
                            provider_guard,
                        );
                    }
                    readiness_complete = true;
                }
                () = &mut lane_wait, if !lane_complete => lane_complete = true,
                _ = validity_tick.tick() => {
                    if !provider_guard.is_valid_async().await {
                        return (
                            DeliveryReport {
                                failures: vec![
                                    "IM mirror attach changed before provider delivery".to_string()
                                ],
                                unsafe_to_continue: true,
                                ..DeliveryReport::default()
                            },
                            provider_guard,
                        );
                    }
                    if readiness
                        .as_ref()
                        .is_some_and(|readiness| !readiness.remains_configured())
                    {
                        let account = readiness
                            .as_ref()
                            .map(PipelineAccountReadiness::account_id)
                            .unwrap_or("unknown");
                        return (
                            DeliveryReport {
                                failures: vec![format!(
                                    "IM account {account} was removed or disabled before mirror delivery"
                                )],
                                unsafe_to_continue: true,
                                ..DeliveryReport::default()
                            },
                            provider_guard,
                        );
                    }
                }
            }
        }

        // Re-probe at the actual mutation boundary: an account may have become
        // ready after its timer elapsed while waiting for a slow predecessor,
        // or may have stopped after an earlier successful probe.
        let running_now = match readiness.as_ref() {
            Some(readiness) => readiness.remains_configured() && readiness.is_running_now().await,
            None => true,
        };
        if !running_now {
            let account = readiness
                .as_ref()
                .map(PipelineAccountReadiness::account_id)
                .unwrap_or("unknown");
            return (
                DeliveryReport {
                    failures: vec![format!(
                        "IM account {account} was not running at mirror delivery time"
                    )],
                    unsafe_to_continue: true,
                    ..DeliveryReport::default()
                },
                provider_guard,
            );
        }
        if !provider_guard.is_valid_async().await {
            return (
                DeliveryReport {
                    failures: vec!["IM mirror attach changed before provider delivery".to_string()],
                    unsafe_to_continue: true,
                    ..DeliveryReport::default()
                },
                provider_guard,
            );
        }

        let target = DeliveryTarget::from(target);
        let mut report = DeliveryReport::default();
        for notice in initial_notices {
            let mut notice_report = super::dispatcher::send_text_chunks_with_guard(
                plugin,
                &target,
                &notice,
                None,
                &[],
                Some(&provider_guard),
            )
            .await;
            // A prefix is a separate message identity.  Its ambiguous failure
            // must be diagnosed, but cannot fuse the assistant's own preview /
            // final identity as a duplicate risk.
            notice_report.unsafe_to_continue = false;
            report.merge(notice_report);
        }
        if let Some(quote) = quote.filter(|body| !body.trim().is_empty()) {
            let mut quote_report = super::dispatcher::send_text_chunks_with_guard(
                plugin,
                &target,
                quote.trim_end(),
                None,
                &[],
                Some(&provider_guard),
            )
            .await;
            let quote_delivered = quote_report.is_success();
            quote_report.unsafe_to_continue = false;
            report.merge(quote_report);
            quote_sent.store(quote_delivered, Ordering::Release);
        }
        (report, provider_guard)
    }
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
/// [`await_stream_pipeline`] when the engine returns.  The physical-target
/// provider lane moves with it into [`PipelineOutcome`] and therefore remains
/// held through the caller's final/error/abort delivery.
pub(crate) struct StreamPipeline {
    pub event_sink: Arc<dyn EventSink>,
    stream_task: JoinHandle<StreamPreviewOutcome>,
    round_texts: Arc<Mutex<RoundTextAccumulator>>,
    reply_mode: ImReplyMode,
    capabilities: ChannelCapabilities,
    preview_active: bool,
    native_preview: Option<PreviewHandle>,
    native_preview_relinquished: Option<Arc<AtomicBool>>,
    provider_guard: Option<ProviderMutationGuard>,
    _provider_lane: Option<ProviderLaneLease>,
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
    provider_guard: Option<ProviderMutationGuard>,
    _provider_lane: Option<ProviderLaneLease>,
}

impl PipelineOutcome {
    fn provider_guard(&self) -> Option<&ProviderMutationGuard> {
        self.provider_guard.as_ref()
    }

    /// Whether the streaming/prelude phase attempted any visible provider
    /// mutation before terminal delivery took ownership. Catch-up mirrors use
    /// this to decide whether an exact generation must retain its no-replay
    /// tombstone even when no durable terminal snapshot exists.
    pub(crate) fn has_provider_attempts(&self) -> bool {
        self.stream_outcome.delivery_report.attempted > 0
    }
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
    let provider_lane = reserve_provider_lane(target);
    let prelude = StreamPipelinePrelude::ordered_only(&provider_lane);
    spawn_stream_pipeline_inner(
        plugin,
        account,
        session_id,
        target,
        history_complete,
        broadcast_to_bus,
        Some(provider_lane),
        Some(prelude),
    )
}

pub(crate) fn spawn_stream_pipeline_with_prelude(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    target: &DeliveryTarget<'_>,
    history_complete: bool,
    broadcast_to_bus: bool,
    provider_lane: ProviderLaneLease,
    prelude: StreamPipelinePrelude,
) -> StreamPipeline {
    spawn_stream_pipeline_inner(
        plugin,
        account,
        session_id,
        target,
        history_complete,
        broadcast_to_bus,
        Some(provider_lane),
        Some(prelude),
    )
}

fn spawn_stream_pipeline_inner(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    target: &DeliveryTarget<'_>,
    history_complete: bool,
    broadcast_to_bus: bool,
    provider_lane: Option<ProviderLaneLease>,
    prelude: Option<StreamPipelinePrelude>,
) -> StreamPipeline {
    let reply_mode = account.im_reply_mode();
    let capabilities = plugin.capabilities();
    let max_msg_len = capabilities.streaming_preview_max_bytes.unwrap_or(4096);
    let mut preview_transport = match reply_mode {
        ImReplyMode::Preview | ImReplyMode::Split => select_stream_preview_transport(
            &target.to_reply_stream_target(),
            &capabilities,
            history_complete,
        ),
        ImReplyMode::Final => None,
    };
    let provider_guard = prelude.as_ref().map(StreamPipelinePrelude::provider_guard);
    if let Some(transport) = preview_transport.as_mut() {
        transport.set_provider_guard(provider_guard.clone());
    }
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
        prelude,
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
        provider_guard,
        _provider_lane: provider_lane,
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
        provider_guard,
        _provider_lane,
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
        provider_guard,
        _provider_lane,
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
        provider_guard,
        _provider_lane,
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
                let abort = abort_native_preview_with_lane(
                    preview,
                    ReplyAbortReason::Cancelled,
                    provider_guard.clone(),
                );
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
            provider_guard,
            _provider_lane,
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
    provider_guard: Option<ProviderMutationGuard>,
    provider_lane: Option<ProviderLaneLease>,
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
        provider_guard,
        _provider_lane: provider_lane,
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
    let provider_guard = outcome.provider_guard();
    let mut metrics = if native_whole_turn {
        deliver_preview_merged(
            plugin,
            target,
            &outcome.drained_rounds,
            response,
            outcome.stream_outcome.preview.as_ref(),
            &outcome.capabilities,
            provider_guard,
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
                    provider_guard,
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
                    provider_guard,
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
                    provider_guard,
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
            abort_native_preview_with_lane(preview, reason, outcome.provider_guard().cloned()),
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
            outcome.provider_guard(),
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
        outcome.provider_guard(),
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
    use anyhow::Result;
    use async_trait::async_trait;
    use ha_core::channel::types::{
        ChannelHealth, ChannelId, ChannelMeta, DeliveryResult, InboundEvent, MsgContext,
        ReplyPayload, SecurityConfig,
    };
    use tokio_util::sync::CancellationToken;

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

    struct PreludeRecordingPlugin {
        sends: Mutex<Vec<String>>,
        fail_next_send: AtomicBool,
    }

    impl PreludeRecordingPlugin {
        fn new() -> Self {
            Self {
                sends: Mutex::new(Vec::new()),
                fail_next_send: AtomicBool::new(false),
            }
        }

        fn failing_first_send() -> Self {
            Self {
                sends: Mutex::new(Vec::new()),
                fail_next_send: AtomicBool::new(true),
            }
        }

        fn sends(&self) -> Vec<String> {
            self.sends.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChannelPlugin for PreludeRecordingPlugin {
        fn meta(&self) -> ChannelMeta {
            ChannelMeta {
                id: ChannelId::Custom("prelude-test".to_string()),
                display_name: "Prelude test".to_string(),
                description: String::new(),
                version: "0".to_string(),
            }
        }

        fn capabilities(&self) -> ChannelCapabilities {
            empty_capabilities()
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
            if self.fail_next_send.swap(false, Ordering::AcqRel) {
                anyhow::bail!("synthetic ambiguous prefix failure");
            }
            if let Some(text) = payload.text.as_ref() {
                self.sends.lock().unwrap().push(text.clone());
            }
            Ok(DeliveryResult::ok("message"))
        }

        async fn send_typing(&self, _account_id: &str, _chat_id: &str) -> Result<()> {
            Ok(())
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
            Ok("test".to_string())
        }
    }

    fn final_mode_account(id: &str) -> ChannelAccountConfig {
        ChannelAccountConfig {
            id: id.to_string(),
            channel_id: ChannelId::Custom("prelude-test".to_string()),
            label: "Prelude test".to_string(),
            enabled: true,
            agent_id: None,
            credentials: serde_json::Value::Null,
            settings: serde_json::json!({"imReplyMode": "final"}),
            security: SecurityConfig::default(),
            auto_approve_tools: false,
            notify_session_eviction: true,
            notify_startup: true,
        }
    }

    #[tokio::test]
    async fn mirror_quote_prelude_buffers_delta_without_blocking_the_engine_sink() {
        let suffix = uuid::Uuid::new_v4();
        let account_id = format!("prelude-account-{suffix}");
        let chat_id = format!("prelude-chat-{suffix}");
        let session_id = format!("prelude-session-{suffix}");
        let chat_type = ChatType::Dm;
        let target = DeliveryTarget {
            account_id: &account_id,
            chat_id: &chat_id,
            chat_type: &chat_type,
            thread_id: Some("thread"),
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let predecessor = super::super::provider_lane::reserve_provider_lane(&target);
        let mirror_lane = super::super::provider_lane::reserve_provider_lane(&target);
        let concrete = Arc::new(PreludeRecordingPlugin::new());
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let quote_sent = Arc::new(AtomicBool::new(false));
        let prelude = StreamPipelinePrelude::new(
            mirror_lane.waiter(),
            mirror_lane.task_hold(),
            None,
            Vec::new(),
            Some("> 💬 buffered question".to_string()),
            quote_sent.clone(),
            Arc::new(|| true),
        );
        let pipeline = spawn_stream_pipeline_with_prelude(
            &plugin,
            &final_mode_account(&account_id),
            &session_id,
            &target,
            true,
            false,
            mirror_lane,
            prelude,
        );

        // EventSink::send remains synchronous while the provider task waits.
        // The delta must be retained, and no provider mutation may overtake the
        // predecessor or the quote.
        pipeline
            .event_sink
            .send(r#"{"type":"text_delta","content":"buffered answer"}"#);
        let mut pipeline_task = tokio::spawn(await_stream_pipeline(pipeline));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut pipeline_task,)
                .await
                .is_err(),
            "provider task must remain behind the predecessor while deltas buffer"
        );
        assert!(concrete.sends().is_empty());

        drop(predecessor);
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), pipeline_task)
            .await
            .expect("pipeline should resume after predecessor")
            .expect("pipeline task should not panic");
        assert!(quote_sent.load(Ordering::Acquire));
        assert_eq!(
            outcome
                .drained_rounds
                .iter()
                .map(|round| round.text.as_str())
                .collect::<Vec<_>>(),
            ["buffered answer"]
        );

        let metrics = deliver_rounds(&plugin, &target, &outcome, "buffered answer").await;
        assert!(metrics.report.is_success());
        assert_eq!(
            concrete.sends(),
            ["> 💬 buffered question", "buffered answer"]
        );
    }

    #[tokio::test]
    async fn mirror_quote_failure_is_diagnostic_but_does_not_fuse_assistant_final() {
        let suffix = uuid::Uuid::new_v4();
        let account_id = format!("prefix-failure-account-{suffix}");
        let chat_id = format!("prefix-failure-chat-{suffix}");
        let session_id = format!("prefix-failure-session-{suffix}");
        let target = DeliveryTarget {
            account_id: &account_id,
            chat_id: &chat_id,
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let mirror_lane = super::super::provider_lane::reserve_provider_lane(&target);
        let concrete = Arc::new(PreludeRecordingPlugin::failing_first_send());
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let quote_sent = Arc::new(AtomicBool::new(false));
        let prelude = StreamPipelinePrelude::new(
            mirror_lane.waiter(),
            mirror_lane.task_hold(),
            None,
            Vec::new(),
            Some("> 💬 prefix".to_string()),
            quote_sent.clone(),
            Arc::new(|| true),
        );
        let pipeline = spawn_stream_pipeline_with_prelude(
            &plugin,
            &final_mode_account(&account_id),
            &session_id,
            &target,
            true,
            false,
            mirror_lane,
            prelude,
        );
        pipeline
            .event_sink
            .send(r#"{"type":"text_delta","content":"assistant final"}"#);

        let outcome = await_stream_pipeline(pipeline).await;
        assert!(!outcome.stream_outcome.delivery_report.unsafe_to_continue);
        assert!(!outcome.stream_outcome.delivery_report.failures.is_empty());
        assert!(!quote_sent.load(Ordering::Acquire));

        let metrics = deliver_rounds(&plugin, &target, &outcome, "assistant final").await;
        assert_eq!(concrete.sends(), ["assistant final"]);
        assert!(
            !metrics.report.is_success(),
            "prefix failure remains visible in diagnostics"
        );
    }

    #[tokio::test]
    async fn invalid_prelude_exits_before_stuck_predecessor_and_drops_buffered_delta() {
        let suffix = uuid::Uuid::new_v4();
        let account_id = format!("invalid-prelude-account-{suffix}");
        let chat_id = format!("invalid-prelude-chat-{suffix}");
        let session_id = format!("invalid-prelude-session-{suffix}");
        let target = DeliveryTarget {
            account_id: &account_id,
            chat_id: &chat_id,
            chat_type: &ChatType::Dm,
            thread_id: None,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let predecessor = super::super::provider_lane::reserve_provider_lane(&target);
        let mirror_lane = super::super::provider_lane::reserve_provider_lane(&target);
        let concrete = Arc::new(PreludeRecordingPlugin::new());
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let prelude = StreamPipelinePrelude::new(
            mirror_lane.waiter(),
            mirror_lane.task_hold(),
            None,
            Vec::new(),
            Some("> stale quote".to_string()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(|| false),
        );
        let pipeline = spawn_stream_pipeline_with_prelude(
            &plugin,
            &final_mode_account(&account_id),
            &session_id,
            &target,
            true,
            false,
            mirror_lane,
            prelude,
        );
        pipeline
            .event_sink
            .send(r#"{"type":"text_delta","content":"stale buffered answer"}"#);

        // Invalidity is independent of a stuck predecessor. The cancelled
        // lane node remains linked, but no old-target provider write may run.
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            await_stream_pipeline(pipeline),
        )
        .await
        .expect("invalid prelude should fail closed without waiting for predecessor");
        assert!(outcome.stream_outcome.delivery_report.unsafe_to_continue);

        let metrics = deliver_rounds(&plugin, &target, &outcome, "stale buffered answer").await;
        assert!(metrics.report.unsafe_to_continue);
        assert!(concrete.sends().is_empty());

        drop(outcome);
        drop(predecessor);
    }

    #[tokio::test]
    async fn production_pipeline_lane_holds_inbound_successor_until_outer_terminal_drops() {
        let suffix = uuid::Uuid::new_v4();
        let account_id = format!("inbound-lane-account-{suffix}");
        let chat_id = format!("inbound-lane-chat-{suffix}");
        let chat_type = ChatType::Dm;
        let target = DeliveryTarget {
            account_id: &account_id,
            chat_id: &chat_id,
            chat_type: &chat_type,
            thread_id: Some("thread"),
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        };
        let concrete = Arc::new(PreludeRecordingPlugin::new());
        let plugin: Arc<dyn ChannelPlugin> = concrete.clone();
        let account = final_mode_account(&account_id);

        let first = spawn_stream_pipeline(
            &plugin,
            &account,
            &format!("inbound-first-{suffix}"),
            &target,
            true,
            false,
        );
        first
            .event_sink
            .send(r#"{"type":"text_delta","content":"first"}"#);
        let first_outcome = await_stream_pipeline(first).await;

        let second = spawn_stream_pipeline(
            &plugin,
            &account,
            &format!("inbound-second-{suffix}"),
            &target,
            true,
            false,
        );
        second
            .event_sink
            .send(r#"{"type":"text_delta","content":"second"}"#);
        let mut second_task = tokio::spawn(await_stream_pipeline(second));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut second_task,)
                .await
                .is_err(),
            "stream EOF must not release the first pipeline's outer terminal lane"
        );

        let first_metrics = deliver_rounds(&plugin, &target, &first_outcome, "first").await;
        assert!(first_metrics.report.is_success());
        drop(first_outcome);

        let second_outcome = tokio::time::timeout(std::time::Duration::from_secs(1), second_task)
            .await
            .expect("second inbound pipeline should resume after first terminal drop")
            .expect("second pipeline task should not panic");
        let second_metrics = deliver_rounds(&plugin, &target, &second_outcome, "second").await;
        assert!(second_metrics.report.is_success());
        assert_eq!(concrete.sends(), ["first", "second"]);
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
            provider_guard: None,
            _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
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
                provider_guard: None,
                _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
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
            provider_guard: None,
            _provider_lane: None,
        };

        assert!(error_terminal_allows_replay(&outcome));
        assert!(abort_pipeline_outcome_for_replay(&outcome, ReplyAbortReason::Cancelled).await);
    }
}

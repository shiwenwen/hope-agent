//! Attach catch-up — when an IM chat takes over an existing session
//! (`/session <id>` from IM, GUI / desktop `/handover`, HTTP handover
//! route), the new chat had zero context for the conversation that's
//! been happening up to that point. This helper reads the session's
//! latest completed turn from the `messages` table and replays it as
//! one Final-mode delivery so the IM user sees where the conversation
//! left off.
//!
//! Best-effort by design: the provider lane is reserved before the durable
//! attach, while the active generation and ordered snapshot watermark are
//! captured in the same database critical section that publishes the attach.
//! No visible provider mutation starts until that atomic boundary succeeds.
//! Any delivery failure is logged and swallowed because missing the catch-up
//! is a missed echo, not a missed turn.
//!
//! Desktop / HTTP turns that are already in flight when the attach happens
//! get a late IM mirror registered through `SinkRegistry`; it streams any
//! remaining deltas and replaces the preview with the complete final answer
//! when the turn finishes.

use std::sync::Arc;
use std::time::Duration;

use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_full_response, spawn_stream_pipeline_with_prelude,
    DeliveryTarget, PipelineAccountReadiness, StreamPipeline, StreamPipelinePrelude,
};
use crate::channel::worker::provider_lane::{
    reserve_provider_lane, spawn_provider_process_task, ProviderLaneLease, ProviderMutationGuard,
};
use crate::channel::worker::{deliver_media_to_chat_with_guard, send_text_chunks_with_guard};
use crate::im_mirror::{
    attach_still_matches, attach_still_matches_async, guarded_mirror_sink,
    mirror_attach_claim_is_active, try_claim_mirror_attach, MirrorAttachClaim, MirrorAttachGuard,
    MirrorGeneration,
};
use ha_core::attachments::MediaItem;
use ha_core::channel::db::ChannelDB;
use ha_core::channel::traits::ChannelPlugin;
use ha_core::channel::types::{ChannelAccountConfig, ChatType, ImReplyMode};
use ha_core::chat_engine::sink_registry::{sink_registry, SinkHandle};
use ha_core::chat_engine::stream_seq::ChatSource;
use ha_core::session::{ChatTurnStatus, MessageRole, SessionDB};

const CATCHUP_WINDOW: u32 = 50;
const ACTIVE_TURN_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachKind {
    SessionAttach,
    Handover,
}

struct AttachCatchupCapture {
    active: Option<ActiveAttachedReply>,
    message_watermark: Option<i64>,
    snapshot: Option<TurnSnapshot>,
}

struct AttachCatchupStart {
    active_before: Option<ActiveAttachedReply>,
    session_db: Option<Arc<SessionDB>>,
    message_watermark: Option<i64>,
    snapshot: Option<TurnSnapshot>,
}

/// Opaque pre-attach provider reservation for one catch-up delivery.
///
/// Callers create this before publishing the binding, then must consume it via
/// [`AttachCatchupReservation::attach`]. That transition captures the active
/// generation and message watermark in the same DB critical section as the
/// durable attach, closing both the replay and omission sides of the boundary.
pub struct AttachCatchupReservation {
    session_id: String,
    channel_id: String,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
    provider_lane: ProviderLaneLease,
}

impl AttachCatchupReservation {
    /// Publish the durable binding and atomically fix its catch-up boundary.
    ///
    /// This is synchronous because rusqlite is synchronous. Async callers must
    /// invoke it through `ha_core::blocking::run_blocking`.
    pub fn attach(
        self,
        channel_db: &ChannelDB,
        source: &str,
        sender_id: Option<&str>,
        sender_name: Option<&str>,
        chat_type: &ChatType,
    ) -> anyhow::Result<AttachedCatchupReservation> {
        let AttachCatchupReservation {
            session_id,
            channel_id,
            account_id,
            chat_id,
            thread_id,
            provider_lane,
        } = self;
        let boundary = channel_db.attach_session_with_boundary(
            &channel_id,
            &account_id,
            &chat_id,
            thread_id.as_deref(),
            &session_id,
            source,
            sender_id,
            sender_name,
            chat_type,
            || active_attached_reply(&session_id),
        )?;
        let start = capture_attach_catchup_at_boundary(
            &session_id,
            boundary.same_binding,
            boundary.captured,
            boundary.message_watermark,
        );
        Ok(AttachedCatchupReservation {
            session_id,
            channel_id,
            account_id,
            chat_id,
            thread_id,
            provider_lane,
            start,
        })
    }
}

/// Opaque post-attach catch-up capability.
///
/// This type can only be created by [`AttachCatchupReservation::attach`], so a
/// delivery call cannot accidentally run against a non-atomic plain attach.
pub struct AttachedCatchupReservation {
    session_id: String,
    channel_id: String,
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
    provider_lane: ProviderLaneLease,
    start: AttachCatchupStart,
}

impl AttachedCatchupReservation {
    fn matches(
        &self,
        session_id: &str,
        channel_id: &str,
        account_id: &str,
        chat_id: &str,
        thread_id: Option<&str>,
    ) -> bool {
        self.session_id == session_id
            && self.channel_id == channel_id
            && self.account_id == account_id
            && self.chat_id == chat_id
            && self.thread_id.as_deref() == thread_id
    }
}

#[derive(Clone)]
enum ActiveAttachedReply {
    Foreground(ha_core::chat_engine::active_turn::ActiveTurnSnapshot),
    Injection {
        run_id: String,
        /// `None` covers the narrow, valid window after the initial injection
        /// hook resolved but before `StreamLifecycle::begin`. The shared core
        /// coordinator — not stream presence alone — arbitrates that race.
        stream_id: Option<String>,
    },
}

/// Reserve provider order before mutating the durable IM attach.
///
/// The returned value is intentionally opaque and single-use. It cannot be
/// delivered until [`AttachCatchupReservation::attach`] has atomically
/// published the binding and fixed its replay boundary. Dropping it after an
/// attach failure releases provider order without a visible mutation.
pub fn prepare_attach_catchup(
    session_id: &str,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
) -> AttachCatchupReservation {
    // `ProviderTargetKey` only uses account/chat/thread. A DM placeholder
    // establishes physical ordering without needing the future attach row.
    let lane_chat_type = ChatType::Dm;
    let lane_target = DeliveryTarget {
        account_id,
        chat_id,
        chat_type: &lane_chat_type,
        thread_id,
        reply_to_message_id: None,
        recipient_user_id: None,
        recipient_tenant_id: None,
    };
    let provider_lane = reserve_provider_lane(&lane_target);
    AttachCatchupReservation {
        session_id: session_id.to_string(),
        channel_id: channel_id.to_string(),
        account_id: account_id.to_string(),
        chat_id: chat_id.to_string(),
        thread_id: thread_id.map(str::to_string),
        provider_lane,
    }
}

/// Read the latest completed turn fixed by
/// [`AttachCatchupReservation::attach`] and deliver assistant final text +
/// media to the chat as a one-shot `Final`-mode delivery.
///
/// Skips silently when the session has no assistant text and no media yet.
/// If a desktop / HTTP turn is already active, registers a late mirror so
/// the IM chat receives the rest of the stream plus the complete final
/// answer for that turn.
pub async fn deliver_attach_catchup(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    reservation: AttachedCatchupReservation,
) {
    if !reservation.matches(
        session_id,
        &account.channel_id.to_string(),
        &account.id,
        chat_id,
        thread_id,
    ) {
        app_warn!(
            "channel",
            "attach_sync",
            "Attach catch-up reservation target mismatch for session {}; skipping delivery",
            session_id
        );
        return;
    }
    let AttachedCatchupReservation {
        session_id: _,
        channel_id: _,
        account_id: _,
        chat_id: _,
        thread_id: _,
        provider_lane,
        start,
    } = reservation;
    let capture = complete_attach_catchup_capture(session_id, start).await;
    deliver_attach_catchup_inner(
        plugin,
        account,
        session_id,
        chat_id,
        thread_id,
        AttachKind::SessionAttach,
        provider_lane,
        capture,
    )
    .await;
}

/// Schedule the same catch-up path as [`deliver_attach_catchup`], with a
/// GUI/HTTP handover notice sent into the receiving IM chat.
///
/// The supplied post-attach capability proves provider order was reserved and
/// the binding was published through the atomic catch-up boundary, so a
/// subsequent IM turn cannot overtake it. The replay itself runs as a detached
/// task on the provider process executor: handover persistence is the request
/// boundary and slow provider I/O must not delay a GUI command or HTTP
/// response. Dropping a request-scoped Tokio runtime therefore cannot cancel
/// an accepted catch-up.
pub async fn deliver_handover_catchup(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    reservation: AttachedCatchupReservation,
) {
    if !reservation.matches(
        session_id,
        &account.channel_id.to_string(),
        &account.id,
        chat_id,
        thread_id,
    ) {
        app_warn!(
            "channel",
            "attach_sync",
            "Handover catch-up reservation target mismatch for session {}; skipping delivery",
            session_id
        );
        return;
    }
    let AttachedCatchupReservation {
        session_id: _,
        channel_id: _,
        account_id: _,
        chat_id: _,
        thread_id: _,
        provider_lane,
        start,
    } = reservation;
    // Take the post-attach active sample before detaching the orchestration.
    // Provider I/O remains entirely on the process-lifetime executor below.
    let capture = complete_attach_catchup_capture(session_id, start).await;
    let plugin = plugin.clone();
    let account = account.clone();
    let session_id = session_id.to_string();
    let chat_id = chat_id.to_string();
    let thread_id = thread_id.map(str::to_string);
    spawn_provider_process_task(async move {
        deliver_attach_catchup_inner(
            &plugin,
            &account,
            &session_id,
            &chat_id,
            thread_id.as_deref(),
            AttachKind::Handover,
            provider_lane,
            capture,
        )
        .await;
    });
}

async fn deliver_attach_catchup_inner(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    kind: AttachKind,
    mut provider_lane: ProviderLaneLease,
    capture: AttachCatchupCapture,
) {
    let session_db = match ha_core::globals::get_session_db() {
        Some(db) => db,
        None => {
            app_warn!(
                "channel",
                "attach_sync",
                "session_db not initialised; skipping attach catch-up for {}",
                session_id
            );
            return;
        }
    };

    let AttachCatchupCapture {
        active,
        message_watermark,
        snapshot,
    } = capture;
    let conversation = match ha_core::globals::get_channel_db() {
        Some(db) => {
            let lookup_session_id = session_id.to_string();
            tokio::task::spawn_blocking(move || {
                db.get_conversation_by_session(&lookup_session_id)
                    .ok()
                    .flatten()
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    }
    .filter(|conv| {
        conv.account_id == account.id
            && conv.chat_id == chat_id
            && conv.thread_id.as_deref() == thread_id
    });
    let Some(conversation) = conversation else {
        app_warn!(
            "channel",
            "attach_sync",
            "Attach binding missing or changed before catch-up for session {}",
            session_id
        );
        return;
    };
    let chat_type = ChatType::from_lowercase(&conversation.chat_type);
    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id,
        chat_type: &chat_type,
        thread_id,
        reply_to_message_id: None,
        recipient_user_id: conversation.sender_id.as_deref(),
        recipient_tenant_id: None,
    };
    let handover_notice =
        (kind == AttachKind::Handover).then(|| handover_notice_text(active.is_some()).to_string());
    if let Some(active) = active {
        match start_late_mirror(
            plugin,
            account,
            session_db.clone(),
            session_id,
            chat_id,
            thread_id,
            active,
            handover_notice.clone().into_iter().collect(),
            provider_lane,
        )
        .await
        {
            LateMirrorStart::Handled => return,
            LateMirrorStart::NotStarted(lane) => provider_lane = lane,
        }
    }

    let catchup_session_id = session_id.to_string();
    let catchup_attach_id = conversation.id;
    let catchup_valid: Arc<dyn Fn() -> bool + Send + Sync> =
        Arc::new(move || attach_still_matches(&catchup_session_id, catchup_attach_id));
    let provider_guard = ProviderMutationGuard::new(
        provider_lane.waiter(),
        provider_lane.task_hold(),
        catchup_valid,
    );
    // Handover catch-up is already detached from its request. Keep the ordered
    // snapshot instead of timing it out: a slow predecessor must not silently
    // discard the handover context. SessionAttach remains awaited so its slash
    // confirmation cannot bypass this lane and appear before the snapshot.
    let lane_waiter = provider_lane.waiter();
    let lane_wait = lane_waiter.wait_turn();
    tokio::pin!(lane_wait);
    let mut validity_tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            () = &mut lane_wait => break,
            _ = validity_tick.tick() => {
                if !provider_guard.is_valid_async().await {
                    return;
                }
            }
        }
    }

    if let Some(notice) = handover_notice {
        let report =
            send_text_chunks_with_guard(plugin, &target, &notice, None, &[], Some(&provider_guard))
                .await;
        if report.unsafe_to_continue {
            return;
        }
    }

    let snapshot = match snapshot {
        Some(s) => s,
        None => return,
    };
    app_debug!(
        "channel",
        "attach_sync",
        "Delivering fixed attach snapshot for session {} through message {:?}",
        session_id,
        message_watermark
    );

    let caps = plugin.capabilities();

    // 1. Send the assistant final text (if any). Re-uses the dispatcher's
    //    `send_text_chunks` so the markdown → native → chunk_message
    //    sequence + error logging stays in one place. Catch-up has no
    //    inbound message to quote, so `reply_to_message_id=None` and
    //    `preview=None` (no live preview to edit).
    if !snapshot.text.is_empty() {
        let report = send_text_chunks_with_guard(
            plugin,
            &target,
            &snapshot.text,
            None,
            &[],
            Some(&provider_guard),
        )
        .await;
        if report.unsafe_to_continue {
            return;
        }
    }

    // 2. Re-send the latest turn's media. We do not regenerate or
    //    re-upload — `deliver_media_to_chat` resolves each MediaItem's
    //    `local_path` through the plugin's normal native-vs-fallback
    //    partition (same path used by every live IM round delivery).
    if !snapshot.medias.is_empty() {
        deliver_media_to_chat_with_guard(
            plugin,
            &target,
            &snapshot.medias,
            &caps,
            Some(&provider_guard),
        )
        .await;
    }
}

fn capture_attach_catchup_at_boundary(
    session_id: &str,
    same_binding: bool,
    active_before: Option<ActiveAttachedReply>,
    message_watermark: Option<i64>,
) -> AttachCatchupStart {
    // Reattaching the exact same target/session must not replay content that
    // chat has already received. Its ordinary live mirror already owns any
    // active generation as well.
    if same_binding {
        return AttachCatchupStart {
            active_before: None,
            session_db: None,
            message_watermark: None,
            snapshot: None,
        };
    }
    let Some(session_db) = ha_core::globals::get_session_db() else {
        return AttachCatchupStart {
            active_before,
            session_db: None,
            message_watermark,
            snapshot: None,
        };
    };
    let result = match message_watermark {
        Some(through_id) => session_db
            .load_session_messages_latest_through(session_id, through_id, CATCHUP_WINDOW)
            .map(|messages| latest_turn_snapshot(&messages)),
        None => Ok(None),
    };
    match result {
        Ok(snapshot) => AttachCatchupStart {
            active_before,
            session_db: Some(session_db.clone()),
            message_watermark,
            snapshot,
        },
        Err(error) => {
            app_warn!(
                "channel",
                "attach_sync",
                "Failed to load atomic attach snapshot for session {}: {}",
                session_id,
                error
            );
            AttachCatchupStart {
                active_before,
                session_db: Some(session_db.clone()),
                message_watermark,
                snapshot: None,
            }
        }
    }
}

async fn complete_attach_catchup_capture(
    session_id: &str,
    start: AttachCatchupStart,
) -> AttachCatchupCapture {
    let AttachCatchupStart {
        active_before,
        session_db,
        message_watermark,
        snapshot,
    } = start;

    // The pre-sample and watermark were taken while the DB connection mutex
    // prevented any turn from crossing the durable attach lookup. A generation
    // already active there gets a LateMirror (or exact terminal recovery). A
    // generation starting afterwards blocks on that mutex and observes the new
    // binding through the ordinary live-delivery path, so it must not be added
    // to the static snapshot.
    let active_after = active_attached_reply(session_id);
    let (active, ended_before) = resolve_active_capture(active_before, active_after);
    let exact_ended_snapshot = if let (Some(ended), Some(session_db)) = (ended_before, session_db) {
        let exact_db = session_db;
        let exact_session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            completed_snapshot_for_active(&exact_db, &exact_session_id, &ended)
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    AttachCatchupCapture {
        active,
        message_watermark,
        snapshot: exact_ended_snapshot.or(snapshot),
    }
}

fn resolve_active_capture(
    active_before: Option<ActiveAttachedReply>,
    active_after: Option<ActiveAttachedReply>,
) -> (Option<ActiveAttachedReply>, Option<ActiveAttachedReply>) {
    match (&active_before, &active_after) {
        (Some(before), Some(after)) if same_active_generation(before, after) => {
            (active_after, None)
        }
        // The pre-watermark generation ended, including A→B where B began
        // after the durable attach and therefore owns its normal engine mirror.
        // Keep A as the catch-up target and load its exact terminal anchor; do
        // not let B make A's W-after rows disappear.
        (Some(before), _) => (Some(before.clone()), Some(before.clone())),
        (None, _) => (active_after, None),
    }
}

fn same_active_generation(left: &ActiveAttachedReply, right: &ActiveAttachedReply) -> bool {
    match (left, right) {
        (ActiveAttachedReply::Foreground(left), ActiveAttachedReply::Foreground(right)) => {
            left.turn_id == right.turn_id
        }
        (
            ActiveAttachedReply::Injection { run_id: left, .. },
            ActiveAttachedReply::Injection { run_id: right, .. },
        ) => left == right,
        _ => false,
    }
}

fn completed_snapshot_for_active(
    session_db: &SessionDB,
    session_id: &str,
    active: &ActiveAttachedReply,
) -> Option<TurnSnapshot> {
    let user_message_id = match active {
        ActiveAttachedReply::Foreground(active) => {
            let turn = session_db.get_chat_turn(&active.turn_id).ok().flatten()?;
            if !turn.status.is_terminal() {
                return None;
            }
            turn.user_message_id
        }
        ActiveAttachedReply::Injection { run_id, .. } => session_db
            .injection_user_message_id(session_id, run_id)
            .ok()
            .flatten(),
    }?;
    turn_snapshot_after_user(session_db, session_id, user_message_id)
}

fn active_attached_reply(session_id: &str) -> Option<ActiveAttachedReply> {
    if let Some(active) = ha_core::chat_engine::active_turn::current(session_id)
        .filter(|active| matches!(active.source, ChatSource::Desktop | ChatSource::Http))
    {
        return Some(ActiveAttachedReply::Foreground(active));
    }
    let run_id = ha_core::subagent::active_injection_run_id(session_id)?;
    let stream_id = match ha_core::chat_engine::stream_seq::stream_identity(session_id) {
        Some((ChatSource::ParentInjection, stream_id)) => Some(stream_id),
        Some(_) => return None,
        None => None,
    };
    Some(ActiveAttachedReply::Injection { run_id, stream_id })
}

fn handover_notice_text(in_flight: bool) -> &'static str {
    if in_flight {
        "📨 Session handed over from Hope Agent. A reply is already in progress; live updates will continue here."
    } else {
        "📨 Session handed over from Hope Agent."
    }
}

async fn start_late_mirror(
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    active: ActiveAttachedReply,
    initial_notices: Vec<String>,
    provider_lane: ProviderLaneLease,
) -> LateMirrorStart {
    let channel_db = match ha_core::globals::get_channel_db() {
        Some(db) => db,
        None => return LateMirrorStart::NotStarted(provider_lane),
    };
    let attach_db = channel_db.clone();
    let attach_session_id = session_id.to_string();
    let attach = match tokio::task::spawn_blocking(move || {
        attach_db.get_conversation_by_session(&attach_session_id)
    })
    .await
    {
        Ok(Ok(Some(conv))) => conv,
        Ok(Ok(None)) => return LateMirrorStart::NotStarted(provider_lane),
        Ok(Err(e)) => {
            app_warn!(
                "channel",
                "attach_sync",
                "get_conversation_by_session({}) failed before late mirror: {}",
                session_id,
                e
            );
            return LateMirrorStart::NotStarted(provider_lane);
        }
        Err(e) => {
            app_warn!(
                "channel",
                "attach_sync",
                "get_conversation_by_session({}) task failed before late mirror: {}",
                session_id,
                e
            );
            return LateMirrorStart::NotStarted(provider_lane);
        }
    };
    if attach.account_id != account.id
        || attach.chat_id != chat_id
        || attach.thread_id.as_deref() != thread_id
    {
        return LateMirrorStart::NotStarted(provider_lane);
    }
    let (generation, foreground, mut late_injection_install) = match active {
        ActiveAttachedReply::Foreground(active) => (
            MirrorGeneration::turn(active.turn_id.clone()),
            Some(active),
            None,
        ),
        ActiveAttachedReply::Injection { run_id, stream_id } => {
            if !crate::im_mirror::owns_parent_injection_delivery() {
                return LateMirrorStart::NotStarted(provider_lane);
            }
            let stream_matches = match (
                stream_id.as_deref(),
                ha_core::chat_engine::stream_seq::stream_identity(session_id),
            ) {
                (Some(expected), Some((ChatSource::ParentInjection, current))) => {
                    expected == current
                }
                // The coordinator's Initializing/LateRequested handshake covers
                // the pre-StreamLifecycle window. Once a stream exists it must
                // be ParentInjection; a different source can never inherit the
                // captured run generation.
                (None, None) | (None, Some((ChatSource::ParentInjection, _))) => true,
                // Stream teardown can precede the core terminal handoff by a
                // few instructions. The exact active run + coordinator still
                // fence that terminal/install race.
                (Some(_), None) => ha_core::subagent::active_injection_run_id(session_id)
                    .is_some_and(|active_run| active_run == run_id),
                _ => false,
            };
            if !stream_matches {
                return LateMirrorStart::NotStarted(provider_lane);
            }
            let registry = match ha_core::globals::get_channel_registry() {
                Some(registry) => registry,
                None => return LateMirrorStart::NotStarted(provider_lane),
            };
            if !registry.health(&account.id).await.is_running {
                return LateMirrorStart::NotStarted(provider_lane);
            }
            if !attach_still_matches_async(session_id, attach.id).await {
                return LateMirrorStart::Handled;
            }
            let mut reservation = ha_core::subagent::injection::reserve_active_injection_im_mirror(
                session_id, &run_id,
            );
            let install = loop {
                match reservation {
                    ha_core::subagent::injection::LateInjectionMirrorReservation::Reserved(
                        install,
                    ) => break install,
                    ha_core::subagent::injection::LateInjectionMirrorReservation::Busy(retry) => {
                        // A→B→C handovers may overlap while B is still
                        // retiring/installing. Keep C registered with the core
                        // terminal coordinator instead of treating Busy as an
                        // already-owned C mirror. Re-check the durable binding
                        // before and after the wait so an intermediate target
                        // never retires or writes ahead of the actual latest
                        // attach.
                        if !attach_still_matches_async(session_id, attach.id).await {
                            drop(retry);
                            return LateMirrorStart::Handled;
                        }
                        reservation = retry.wait().await;
                        if !attach_still_matches_async(session_id, attach.id).await {
                            drop(reservation);
                            return LateMirrorStart::Handled;
                        }
                    }
                    ha_core::subagent::injection::LateInjectionMirrorReservation::Stale => {
                        // The initial injection mirror may have finalized and
                        // released the core coordinator before this catch-up
                        // task starts. Its channel claim keeps a short-lived
                        // completed tombstone; consult it before falling back
                        // to the exact static snapshot, otherwise that same
                        // run terminal would be delivered twice.
                        return match try_claim_mirror_attach(
                            session_id,
                            attach.id,
                            MirrorGeneration::injection(run_id.clone()),
                        ) {
                            MirrorAttachClaim::Busy | MirrorAttachClaim::Completed => {
                                LateMirrorStart::Handled
                            }
                            MirrorAttachClaim::Attached(guard) => {
                                drop(guard);
                                LateMirrorStart::NotStarted(provider_lane)
                            }
                            MirrorAttachClaim::Unavailable => {
                                LateMirrorStart::NotStarted(provider_lane)
                            }
                        };
                    }
                }
            };
            (
                MirrorGeneration::injection(run_id.clone()),
                None,
                Some((run_id, install)),
            )
        }
    };
    let mirror_guard = match try_claim_mirror_attach(session_id, attach.id, generation.clone()) {
        MirrorAttachClaim::Attached(guard) => guard,
        // The engine-side attach for this exact turn generation already owns
        // the mirror. Treat it as handled so catch-up cannot duplicate the
        // same logical reply. A previous/next turn uses another generation and
        // therefore never lands here.
        MirrorAttachClaim::Busy | MirrorAttachClaim::Completed => {
            return LateMirrorStart::Handled;
        }
        MirrorAttachClaim::Unavailable => return LateMirrorStart::NotStarted(provider_lane),
    };
    let chat_type = ChatType::from_lowercase(&attach.chat_type);

    if let Some((run_id, install)) = late_injection_install.as_mut() {
        if !attach_still_matches_async(session_id, attach.id).await {
            drop(mirror_guard);
            return LateMirrorStart::Handled;
        }
        // `try_claim` proved that this is a replacement binding. Retire the
        // stale owner before arming the new provider path: if arming fails,
        // token Drop leaves the coordinator open so core terminal handling
        // reattaches the current DB binding instead of restoring the old one.
        if !install.retire_previous().await || !install.arm_no_replay(run_id).await {
            // The generation claim drops here before any pipeline/provider
            // mutation. Do not fall back to a partial static injection turn.
            drop(mirror_guard);
            return LateMirrorStart::Handled;
        }
        if !attach_still_matches_async(session_id, attach.id).await {
            drop(mirror_guard);
            return LateMirrorStart::Handled;
        }
    }

    let quote = if let Some(active) = foreground.as_ref() {
        let quote_db = session_db.clone();
        let quote_session_id = session_id.to_string();
        let quote_source = active.source;
        tokio::task::spawn_blocking(move || {
            latest_user_quote(&quote_db, &quote_session_id, quote_source)
        })
        .await
        .ok()
        .flatten()
        .map(|body| body.trim_end().to_string())
        .filter(|body| !body.is_empty())
    } else {
        None
    };

    let mut mirror_account = account.clone();
    if matches!(mirror_account.im_reply_mode(), ImReplyMode::Split) {
        mirror_account.set_im_reply_mode(ImReplyMode::Preview);
    }

    let target = DeliveryTarget {
        account_id: &account.id,
        chat_id,
        chat_type: &chat_type,
        thread_id,
        reply_to_message_id: None,
        recipient_user_id: attach.sender_id.as_deref(),
        recipient_tenant_id: None,
    };
    let quote_sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prelude_session_id = session_id.to_string();
    let prelude_attach_id = attach.id;
    let prelude_generation = generation.clone();
    let still_valid = Arc::new(move || {
        attach_still_matches(&prelude_session_id, prelude_attach_id)
            && mirror_attach_claim_is_active(
                &prelude_session_id,
                prelude_attach_id,
                &prelude_generation,
            )
    });
    let prelude = StreamPipelinePrelude::new(
        provider_lane.waiter(),
        provider_lane.task_hold(),
        Some(PipelineAccountReadiness::RequireRunning {
            account_id: account.id.clone(),
        }),
        initial_notices,
        quote,
        quote_sent,
        still_valid,
    );
    let pipeline = spawn_stream_pipeline_with_prelude(
        plugin,
        &mirror_account,
        session_id,
        &target,
        false,
        false,
        provider_lane,
        prelude,
    );
    let guarded_sink = guarded_mirror_sink(
        session_id.to_string(),
        attach.id,
        generation,
        pipeline.event_sink.clone(),
    );
    let sink_handle = sink_registry().attach(session_id.to_string(), guarded_sink);

    if let Some(active) = foreground {
        let mirror = LateMirror {
            sink_handle,
            _mirror_guard: mirror_guard,
            pipeline,
            plugin: plugin.clone(),
            account: mirror_account,
            session_db,
            session_id: session_id.to_string(),
            turn_id: active.turn_id,
            attach_id: attach.id,
            chat_id: chat_id.to_string(),
            chat_type,
            thread_id: thread_id.map(str::to_string),
            recipient_user_id: attach.sender_id.clone(),
        };
        tokio::spawn(async move {
            mirror.run().await;
        });
    } else if let Some((run_id, install)) = late_injection_install {
        let mirror = LateInjectionMirror {
            sink_handle,
            _mirror_guard: mirror_guard,
            pipeline,
            plugin: plugin.clone(),
            account: mirror_account,
            session_db,
            session_id: session_id.to_string(),
            run_id,
            attach_id: attach.id,
            chat_id: chat_id.to_string(),
            chat_type,
            thread_id: thread_id.map(str::to_string),
            recipient_user_id: attach.sender_id.clone(),
        };
        if !attach_still_matches_async(session_id, attach.id).await {
            let _ = ha_core::channel_hooks::ImLiveMirror::abort(Box::new(mirror), None).await;
            drop(install);
            return LateMirrorStart::Handled;
        }
        let _ = install.install(Box::new(mirror)).await;
    }
    LateMirrorStart::Handled
}

enum LateMirrorStart {
    Handled,
    NotStarted(ProviderLaneLease),
}

struct LateMirror {
    sink_handle: SinkHandle,
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    account: ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: String,
    turn_id: String,
    attach_id: i64,
    chat_id: String,
    chat_type: ChatType,
    thread_id: Option<String>,
    recipient_user_id: Option<String>,
}

impl LateMirror {
    async fn run(self) {
        let LateMirror {
            sink_handle,
            _mirror_guard: mut mirror_guard,
            pipeline,
            plugin,
            account,
            session_db,
            session_id,
            turn_id,
            attach_id,
            chat_id,
            chat_type,
            thread_id,
            recipient_user_id,
        } = self;

        let mut detached = false;
        loop {
            tokio::time::sleep(ACTIVE_TURN_POLL_INTERVAL).await;
            if !attach_still_matches_async(&session_id, attach_id).await {
                detached = true;
                break;
            }
            match ha_core::chat_engine::active_turn::current(&session_id) {
                Some(active) if active.turn_id == turn_id => continue,
                _ => break,
            }
        }

        drop(sink_handle);
        let outcome = await_stream_pipeline(pipeline).await;
        if detached || !attach_still_matches_async(&session_id, attach_id).await {
            crate::channel::worker::pipeline::abort_pipeline_outcome(
                &outcome,
                ha_core::channel::types::ReplyAbortReason::Detached,
            )
            .await;
            return;
        }

        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id: &chat_id,
            chat_type: &chat_type,
            thread_id: thread_id.as_deref(),
            reply_to_message_id: None,
            recipient_user_id: recipient_user_id.as_deref(),
            recipient_tenant_id: None,
        };

        let turn_db = session_db.clone();
        let turn_lookup_id = turn_id.clone();
        let turn = tokio::task::spawn_blocking(move || {
            turn_db.get_chat_turn(&turn_lookup_id).ok().flatten()
        })
        .await
        .ok()
        .flatten();
        let snapshot = turn
            .as_ref()
            .and_then(|t| t.user_message_id)
            .map(|user_id| {
                let snapshot_db = session_db.clone();
                let snapshot_session_id = session_id.clone();
                tokio::task::spawn_blocking(move || {
                    turn_snapshot_after_user(&snapshot_db, &snapshot_session_id, user_id)
                })
            });
        let snapshot = match snapshot {
            Some(task) => task.await.ok().flatten(),
            None => None,
        };

        if let Some(snapshot) = snapshot {
            let metrics =
                deliver_full_response(&plugin, &target, &outcome, &snapshot.text, &snapshot.medias)
                    .await;
            if metrics.report.attempted > 0 {
                mirror_guard.complete();
            }
            if metrics.report.is_success() {
                app_info!(
                    "channel",
                    "attach_sync",
                    "Delivered late handover mirror for session {} turn {} (text_chars={}, media={}, sends={})",
                    session_id,
                    turn_id,
                    metrics.text_chars,
                    metrics.media_count,
                    metrics.report.succeeded,
                );
            } else {
                let warn_context = format!(
                    "Late handover mirror failed for session {} turn {}",
                    session_id, turn_id,
                );
                crate::channel::worker::report_delivery_failure(
                    &session_db,
                    &session_id,
                    &warn_context,
                    "⚠️ Hope finished the reply, but the late IM handover delivery failed or was incomplete.",
                    &metrics.report,
                )
                .await;
            }
            return;
        }

        if let Some(turn) = turn {
            if matches!(turn.status, ChatTurnStatus::Failed) {
                let body = turn
                    .error
                    .as_deref()
                    .map(|raw| {
                        let reason = ha_core::failover::classify_error(raw);
                        ha_core::chat_engine::im_error_message::format_im_engine_error(
                            ha_core::chat_engine::im_error_message::ImErrorContext {
                                reason,
                                raw,
                                // A late-handover turn snapshot no longer carries the
                                // exact provider chain. Keep the generic auth guidance;
                                // never guess the Codex-specific re-authorization path.
                                is_codex_auth: false,
                            },
                        )
                    })
                    .unwrap_or_else(|| "⚠️ **Something went wrong**.".to_string());
                let report = crate::channel::worker::pipeline::deliver_error_reply(
                    &plugin, &target, &outcome, &body,
                )
                .await;
                if report.attempted > 0 {
                    mirror_guard.complete();
                }
                if !report.is_success() {
                    app_warn!(
                        "channel",
                        "attach_sync",
                        "Late handover error terminal was incomplete for session {} turn {}",
                        session_id,
                        turn_id
                    );
                }
                return;
            }
        }
        if outcome.has_provider_attempts() {
            // A preview may already be visible even when the turn has no
            // replayable terminal snapshot. Fence the exact generation before
            // aborting that identity so a delayed engine/catch-up claimant
            // cannot open a second one.
            mirror_guard.complete();
        }
        let _ = crate::channel::worker::pipeline::abort_pipeline_outcome(
            &outcome,
            ha_core::channel::types::ReplyAbortReason::Failed,
        )
        .await;
    }
}

/// ParentInjection late mirror installed into the core-owned terminal slot.
/// Unlike a foreground LateMirror it never polls terminal state: the injection
/// owner calls the ordinary `ImLiveMirror` finalize/abort contract, preserving
/// the existing retry-safety verdict.
struct LateInjectionMirror {
    sink_handle: SinkHandle,
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    account: ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: String,
    run_id: String,
    attach_id: i64,
    chat_id: String,
    chat_type: ChatType,
    thread_id: Option<String>,
    recipient_user_id: Option<String>,
}

struct DetachedLateInjectionMirror {
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    account: ChannelAccountConfig,
    session_db: Arc<SessionDB>,
    session_id: String,
    run_id: String,
    attach_id: i64,
    chat_id: String,
    chat_type: ChatType,
    thread_id: Option<String>,
    recipient_user_id: Option<String>,
}

impl LateInjectionMirror {
    fn into_detached(self) -> DetachedLateInjectionMirror {
        let Self {
            sink_handle,
            _mirror_guard,
            pipeline,
            plugin,
            account,
            session_db,
            session_id,
            run_id,
            attach_id,
            chat_id,
            chat_type,
            thread_id,
            recipient_user_id,
        } = self;
        // ImLiveMirror requires synchronous detach before returning its future;
        // otherwise the terminal generation can consume the next turn's delta.
        drop(sink_handle);
        DetachedLateInjectionMirror {
            _mirror_guard,
            pipeline,
            plugin,
            account,
            session_db,
            session_id,
            run_id,
            attach_id,
            chat_id,
            chat_type,
            thread_id,
            recipient_user_id,
        }
    }
}

impl DetachedLateInjectionMirror {
    async fn finalize(self, fallback_response: String) {
        let Self {
            _mirror_guard: mut mirror_guard,
            pipeline,
            plugin,
            account,
            session_db,
            session_id,
            run_id,
            attach_id,
            chat_id,
            chat_type,
            thread_id,
            recipient_user_id,
        } = self;
        let outcome = await_stream_pipeline(pipeline).await;
        if !attach_still_matches_async(&session_id, attach_id).await {
            crate::channel::worker::pipeline::abort_pipeline_outcome(
                &outcome,
                ha_core::channel::types::ReplyAbortReason::Detached,
            )
            .await;
            return;
        }

        let snapshot_db = session_db.clone();
        let snapshot_session_id = session_id.clone();
        let snapshot_run_id = run_id.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            let user_id =
                snapshot_db.injection_user_message_id(&snapshot_session_id, &snapshot_run_id)?;
            Ok::<_, anyhow::Error>(user_id.and_then(|user_id| {
                turn_snapshot_after_user(&snapshot_db, &snapshot_session_id, user_id)
            }))
        })
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
        .or_else(|| {
            (!fallback_response.is_empty()).then(|| TurnSnapshot {
                text: fallback_response,
                medias: Vec::new(),
            })
        });

        let Some(snapshot) = snapshot else {
            if outcome.has_provider_attempts() {
                mirror_guard.complete();
            }
            let _ = crate::channel::worker::pipeline::abort_pipeline_outcome(
                &outcome,
                ha_core::channel::types::ReplyAbortReason::Failed,
            )
            .await;
            return;
        };
        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id: &chat_id,
            chat_type: &chat_type,
            thread_id: thread_id.as_deref(),
            reply_to_message_id: None,
            recipient_user_id: recipient_user_id.as_deref(),
            recipient_tenant_id: None,
        };
        let metrics =
            deliver_full_response(&plugin, &target, &outcome, &snapshot.text, &snapshot.medias)
                .await;
        if metrics.report.attempted > 0 {
            mirror_guard.complete();
        }
        if metrics.report.is_success() {
            app_info!(
                "channel",
                "attach_sync::injection",
                "Delivered late injection mirror for session {} run {} (text_chars={}, media={}, sends={})",
                session_id,
                run_id,
                metrics.text_chars,
                metrics.media_count,
                metrics.report.succeeded
            );
        } else {
            let warn_context = format!(
                "Late injection mirror failed for session {} run {}",
                session_id, run_id
            );
            crate::channel::worker::report_delivery_failure(
                &session_db,
                &session_id,
                &warn_context,
                "⚠️ Hope finished the background follow-up, but the late IM delivery failed or was incomplete.",
                &metrics.report,
            )
            .await;
        }
    }

    async fn abort(self, body: Option<String>) -> ha_core::channel_hooks::ImLiveMirrorAbortStatus {
        let Self {
            _mirror_guard: mut mirror_guard,
            pipeline,
            plugin,
            account,
            session_db: _,
            session_id,
            run_id: _,
            attach_id,
            chat_id,
            chat_type,
            thread_id,
            recipient_user_id,
        } = self;
        let outcome = await_stream_pipeline(pipeline).await;
        if !attach_still_matches_async(&session_id, attach_id).await {
            let confirmed = crate::channel::worker::pipeline::abort_pipeline_outcome_for_replay(
                &outcome,
                ha_core::channel::types::ReplyAbortReason::Detached,
            )
            .await;
            return ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(confirmed);
        }
        let Some(body) = body else {
            let confirmed = crate::channel::worker::pipeline::abort_pipeline_outcome_for_replay(
                &outcome,
                ha_core::channel::types::ReplyAbortReason::Cancelled,
            )
            .await;
            if !confirmed {
                mirror_guard.complete();
            }
            return ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(confirmed);
        };
        let target = DeliveryTarget {
            account_id: &account.id,
            chat_id: &chat_id,
            chat_type: &chat_type,
            thread_id: thread_id.as_deref(),
            reply_to_message_id: None,
            recipient_user_id: recipient_user_id.as_deref(),
            recipient_tenant_id: None,
        };
        let report = crate::channel::worker::pipeline::deliver_error_reply(
            &plugin, &target, &outcome, &body,
        )
        .await;
        let confirmed = report.is_success()
            && !report.unsafe_to_continue
            && crate::channel::worker::pipeline::error_terminal_allows_replay(&outcome);
        if !confirmed {
            mirror_guard.complete();
        }
        ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(confirmed)
    }
}

impl ha_core::channel_hooks::ImLiveMirror for LateInjectionMirror {
    fn finalize(
        self: Box<Self>,
        response: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let detached = (*self).into_detached();
        Box::pin(async move { detached.finalize(response).await })
    }

    fn abort(
        self: Box<Self>,
        body: Option<String>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = ha_core::channel_hooks::ImLiveMirrorAbortStatus>
                + Send
                + 'static,
        >,
    > {
        let detached = (*self).into_detached();
        Box::pin(async move { detached.abort(body).await })
    }
}

fn latest_user_quote(
    session_db: &SessionDB,
    session_id: &str,
    source: ChatSource,
) -> Option<String> {
    let (messages, _total, _has_more) = session_db
        .load_session_messages_latest(session_id, CATCHUP_WINDOW)
        .ok()?;
    let user = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::User))?;
    ha_core::chat_engine::quote::build_user_quote_prefix(Some(
        &ha_core::chat_engine::quote::LastUserView {
            source: source.as_str(),
            text: &user.content,
            attachment_count: user_attachment_count(user.attachments_meta.as_deref()),
        },
    ))
}

fn user_attachment_count(meta: Option<&str>) -> usize {
    let Some(meta) = meta else { return 0 };
    match serde_json::from_str::<serde_json::Value>(meta) {
        Ok(serde_json::Value::Array(items)) => items.len(),
        _ => 0,
    }
}

pub(crate) fn turn_snapshot_after_user(
    session_db: &SessionDB,
    session_id: &str,
    user_message_id: i64,
) -> Option<TurnSnapshot> {
    let (messages, _has_more) = session_db
        .load_session_messages_after(session_id, user_message_id, 200)
        .ok()?;
    turn_snapshot_until_next_user(&messages)
}

fn turn_snapshot_until_next_user(
    messages: &[ha_core::session::SessionMessage],
) -> Option<TurnSnapshot> {
    let end = messages
        .iter()
        .position(|message| matches!(message.role, MessageRole::User))
        .unwrap_or(messages.len());
    turn_snapshot_from_slice(&messages[..end])
}

/// Walk a session's messages bottom-up and return the latest turn's
/// assistant text + the media items emitted by tool calls in that turn.
///
/// "Latest turn" = everything with id strictly greater than the last
/// `user` row (or the entire vec when no `user` row exists). Returns
/// `None` when the latest turn has neither assistant text nor media —
/// the IM user has nothing to catch up on (fresh session, or only a
/// dangling user prompt with no model output yet).
fn latest_turn_snapshot(messages: &[ha_core::session::SessionMessage]) -> Option<TurnSnapshot> {
    if messages.is_empty() {
        return None;
    }

    let last_user_idx = messages
        .iter()
        .rposition(|m| matches!(m.role, MessageRole::User));
    let start = last_user_idx.map(|i| i + 1).unwrap_or(0);
    let turn = &messages[start..];
    turn_snapshot_from_slice(turn)
}

fn turn_snapshot_from_slice(messages: &[ha_core::session::SessionMessage]) -> Option<TurnSnapshot> {
    if messages.is_empty() {
        return None;
    }

    // Take the very last assistant row's content as the final answer.
    // Earlier `text_block` rows in the same turn are intermediate
    // narration that already streamed (and would have been delivered to
    // the IM live in `split` mode on a normal turn) — replaying them
    // would double-print to a user who's just attaching, so we keep it
    // simple and align with `ImReplyMode::Final` semantics.
    let text = messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, MessageRole::Assistant))
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // Collect every tool result's media in turn order. Reuses
    // `agent::events::extract_media_items` so the parsing rules track
    // whatever the tool-event side emits (`__MEDIA_ITEMS__<json>\n…`).
    let mut medias: Vec<MediaItem> = Vec::new();
    for m in messages {
        if !matches!(m.role, MessageRole::Tool) {
            continue;
        }
        let Some(result) = m.tool_result.as_deref() else {
            continue;
        };
        let (_, items) = ha_core::agent::extract_media_items(result);
        medias.extend(items);
    }

    if text.is_empty() && medias.is_empty() {
        return None;
    }

    Some(TurnSnapshot { text, medias })
}

pub(crate) struct TurnSnapshot {
    pub(crate) text: String,
    pub(crate) medias: Vec<MediaItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ha_core::session::{MessageRole, SessionMessage};

    fn mk_msg(id: i64, role: MessageRole, content: &str) -> SessionMessage {
        SessionMessage {
            id,
            session_id: "s1".into(),
            role,
            content: content.into(),
            timestamp: "2025-01-01T00:00:00Z".into(),
            attachments_meta: None,
            model: None,
            tokens_in: None,
            tokens_out: None,
            reasoning_effort: None,
            tool_call_id: None,
            tool_name: None,
            tool_arguments: None,
            tool_result: None,
            tool_duration_ms: None,
            is_error: None,
            thinking: None,
            ttft_ms: None,
            tokens_in_last: None,
            tokens_cache_creation: None,
            tokens_cache_read: None,
            tool_metadata: None,
            stream_status: None,
            persistence_run_id: None,
        }
    }

    fn mk_tool(id: i64, result: &str) -> SessionMessage {
        let mut m = mk_msg(id, MessageRole::Tool, "");
        m.tool_call_id = Some("call_1".into());
        m.tool_name = Some("send_attachment".into());
        m.tool_result = Some(result.to_string());
        m
    }

    fn injection_active(run_id: &str, stream_id: Option<&str>) -> ActiveAttachedReply {
        ActiveAttachedReply::Injection {
            run_id: run_id.to_string(),
            stream_id: stream_id.map(str::to_string),
        }
    }

    fn injection_run(active: Option<&ActiveAttachedReply>) -> Option<&str> {
        match active {
            Some(ActiveAttachedReply::Injection { run_id, .. }) => Some(run_id),
            Some(ActiveAttachedReply::Foreground(_)) | None => None,
        }
    }

    #[test]
    fn ended_pre_watermark_generation_keeps_exact_anchor() {
        let (active, ended) =
            resolve_active_capture(Some(injection_active("run-a", Some("stream-a"))), None);
        assert_eq!(injection_run(active.as_ref()), Some("run-a"));
        assert_eq!(injection_run(ended.as_ref()), Some("run-a"));
    }

    #[test]
    fn a_to_b_sampling_preserves_a_and_leaves_b_to_ordinary_engine_mirror() {
        let (active, ended) = resolve_active_capture(
            Some(injection_active("run-a", Some("stream-a"))),
            Some(injection_active("run-b", Some("stream-b"))),
        );
        assert_eq!(injection_run(active.as_ref()), Some("run-a"));
        assert_eq!(injection_run(ended.as_ref()), Some("run-a"));
    }

    #[test]
    fn same_generation_after_sample_remains_live_without_exact_replay() {
        let (active, ended) = resolve_active_capture(
            Some(injection_active("run-a", None)),
            Some(injection_active("run-a", Some("stream-a"))),
        );
        assert_eq!(injection_run(active.as_ref()), Some("run-a"));
        assert!(ended.is_none());
    }

    #[test]
    fn empty_messages_returns_none() {
        assert!(latest_turn_snapshot(&[]).is_none());
    }

    #[test]
    fn fresh_user_only_returns_none() {
        let messages = vec![mk_msg(1, MessageRole::User, "hello")];
        assert!(latest_turn_snapshot(&messages).is_none());
    }

    #[test]
    fn assistant_only_text_no_media() {
        let messages = vec![
            mk_msg(1, MessageRole::User, "hi"),
            mk_msg(2, MessageRole::Assistant, "hello there"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "hello there");
        assert!(snap.medias.is_empty());
    }

    #[test]
    fn picks_only_last_turn_text() {
        let messages = vec![
            mk_msg(1, MessageRole::User, "u1"),
            mk_msg(2, MessageRole::Assistant, "old answer"),
            mk_msg(3, MessageRole::User, "u2"),
            mk_msg(4, MessageRole::Assistant, "new answer"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "new answer");
    }

    #[test]
    fn pinned_user_snapshot_stops_before_the_next_user_turn() {
        let messages = vec![
            mk_msg(2, MessageRole::Assistant, "injection answer"),
            mk_msg(3, MessageRole::User, "new GUI question"),
            mk_msg(4, MessageRole::Assistant, "new GUI answer"),
        ];
        let snap = turn_snapshot_until_next_user(&messages).expect("injection snapshot");
        assert_eq!(snap.text, "injection answer");
    }

    #[test]
    fn picks_final_assistant_after_intermediate_text_block() {
        // Intermediate text_block + tool round, then final assistant text.
        let messages = vec![
            mk_msg(1, MessageRole::User, "u"),
            mk_msg(2, MessageRole::TextBlock, "let me think..."),
            mk_msg(3, MessageRole::Assistant, "final answer"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "final answer");
    }

    #[test]
    fn extracts_media_from_tool_result() {
        let media_json = r#"[{"url":"/api/attachments/s/foo.png","localPath":"/tmp/foo.png","name":"foo.png","mimeType":"image/png","sizeBytes":1024,"kind":"image"}]"#;
        let result = format!("{}{}\nok", ha_core::agent::MEDIA_ITEMS_PREFIX, media_json);
        let messages = vec![
            mk_msg(1, MessageRole::User, "u"),
            mk_tool(2, &result),
            mk_msg(3, MessageRole::Assistant, "here"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "here");
        assert_eq!(snap.medias.len(), 1);
        assert_eq!(snap.medias[0].name, "foo.png");
    }

    #[test]
    fn ignores_old_turn_media() {
        let media_json = r#"[{"url":"/api/attachments/s/old.png","localPath":"/tmp/old.png","name":"old.png","mimeType":"image/png","sizeBytes":1,"kind":"image"}]"#;
        let result = format!("{}{}\nok", ha_core::agent::MEDIA_ITEMS_PREFIX, media_json);
        let messages = vec![
            mk_msg(1, MessageRole::User, "u1"),
            mk_tool(2, &result),
            mk_msg(3, MessageRole::Assistant, "old"),
            mk_msg(4, MessageRole::User, "u2"),
            mk_msg(5, MessageRole::Assistant, "new"),
        ];
        let snap = latest_turn_snapshot(&messages).unwrap();
        assert_eq!(snap.text, "new");
        assert!(snap.medias.is_empty(), "old turn media should be dropped");
    }
}

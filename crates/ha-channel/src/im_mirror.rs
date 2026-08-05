//! Live GUI / HTTP → IM streaming mirror. desktop / HTTP-triggered turns
//! that have an IM attach get rendered into the IM chat with the same
//! per-round typewriter UX as IM-inbound turns, driven by the account's
//! `imReplyMode` (`split` / `preview` / `final`).
//!
//! When attaching, if there's a user-message snapshot, this module queues a
//! standalone markdown blockquote (`> 💬 ...`) as the stream pipeline's
//! provider prelude. That keeps the quote at the top of the IM
//! exchange across all three reply modes — `split` fans out per-round
//! messages after it, `preview` grows a single message after it, `final`
//! sends one message after it — without making GUI first-token delivery wait
//! on account startup or remote IM I/O. The quote is a separate IM message;
//! `messages.context_json` and the persisted assistant row stay clean of
//! it so context windows + desktop history are unaffected.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::channel::worker::pipeline::{
    await_stream_pipeline, deliver_full_response, deliver_rounds,
    spawn_stream_pipeline_with_prelude, DeliveryTarget, PipelineAccountReadiness, StreamPipeline,
    StreamPipelinePrelude,
};
use crate::channel::worker::provider_lane::reserve_provider_lane;
use ha_core::channel::db::ChannelConversation;
use ha_core::channel::traits::ChannelPlugin;
use ha_core::channel::types::ChatType;
use ha_core::channel_hooks::{ImDeliveryOwnership, ImLiveMirrorAttach};
// Notice rendering (CANCEL_NOTICE / format_im_engine_error) now lives
// behind `finalize::copy::im_notice` — kept only for type imports that
// other modules may still reference.
use ha_core::channel_hooks::LastUserSnapshot;
use ha_core::chat_engine::quote::{build_user_quote_prefix, LastUserView};
use ha_core::chat_engine::sink_registry::{sink_registry, SinkHandle};
use ha_core::chat_engine::stream_seq::ChatSource;
use ha_core::chat_engine::EventSink;

/// Owned snapshot of the user message that triggered a desktop / HTTP
/// turn. Captured at `attach_im_live_mirror` entry. Owned (not borrowed)
/// because callers may want to construct it from values they don't keep
/// alive across the await; in practice the engine builds it inline.
pub(crate) struct ImLiveMirrorState {
    sink_handle: SinkHandle,
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    attach: ChannelConversation,
    injection_run_id: Option<String>,
    /// Whether the provider prelude confirmed the user-quote message.
    /// Kept for terminal diagnostics; failure delivery cannot be gated on it
    /// because ParentInjection intentionally has no quote while its assistant
    /// preview may already be visible.
    quote_sent: Arc<AtomicBool>,
}

/// Terminal mirror state after its session-wide fan-out sink has been
/// synchronously detached. Keeping this as a distinct type makes it
/// impossible for the async provider-finalize future to retain the old
/// generation's sink while it waits on remote I/O.
struct DetachedImLiveMirrorState {
    _mirror_guard: MirrorAttachGuard,
    pipeline: StreamPipeline,
    plugin: Arc<dyn ChannelPlugin>,
    attach: ChannelConversation,
    injection_run_id: Option<String>,
    quote_sent: Arc<AtomicBool>,
}

fn detach_sink_before<T>(sink_handle: SinkHandle, detached: T) -> T {
    drop(sink_handle);
    detached
}

impl ImLiveMirrorState {
    fn into_detached(self) -> DetachedImLiveMirrorState {
        let Self {
            sink_handle,
            _mirror_guard,
            pipeline,
            plugin,
            attach,
            injection_run_id,
            quote_sent,
        } = self;
        detach_sink_before(
            sink_handle,
            DetachedImLiveMirrorState {
                _mirror_guard,
                pipeline,
                plugin,
                attach,
                injection_run_id,
                quote_sent,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MirrorGeneration {
    Turn(String),
    Stream(String),
    Injection(String),
}

impl MirrorGeneration {
    pub(crate) fn turn(turn_id: impl Into<String>) -> Self {
        Self::Turn(turn_id.into())
    }

    pub(crate) fn injection(run_id: impl Into<String>) -> Self {
        Self::Injection(run_id.into())
    }

    fn is_current(&self, session_id: &str) -> bool {
        match self {
            Self::Turn(turn_id) => ha_core::chat_engine::active_turn::current(session_id)
                .is_some_and(|active| active.turn_id == *turn_id),
            Self::Stream(stream_id) => ha_core::chat_engine::stream_seq::stream_id(session_id)
                .is_some_and(|active_stream_id| active_stream_id == *stream_id),
            Self::Injection(run_id) => ha_core::subagent::active_injection_run_id(session_id)
                .is_some_and(|active_run_id| active_run_id == *run_id),
        }
    }
}

#[derive(Clone, Copy)]
enum MirrorAvailability {
    /// Foreground GUI/HTTP turns attach immediately. Their pipeline owns the
    /// bounded channel-readiness wait so engine streaming never blocks.
    WaitInPipeline,
    /// Replayable ParentInjection must not mutate/persist anything until its
    /// durable IM account worker is known running.
    RequireRunning,
}

fn unavailable_surface_outcome(
    ownership: ImDeliveryOwnership,
    availability: MirrorAvailability,
    account_id: Option<String>,
) -> ImLiveMirrorAttach {
    match (ownership, availability) {
        (ImDeliveryOwnership::LocalOwner, _) => ImLiveMirrorAttach::Unavailable { account_id },
        (ImDeliveryOwnership::RemoteOwner, MirrorAvailability::RequireRunning) => {
            ImLiveMirrorAttach::DeferredToPrimary { account_id }
        }
        (ImDeliveryOwnership::RemoteOwner, MirrorAvailability::WaitInPipeline)
        | (ImDeliveryOwnership::Disabled, _) => ImLiveMirrorAttach::Absent,
    }
}

/// ParentInjection ownership is a process boundary, not a surface lookup.
/// A Secondary must hand durable sources to the Primary even when this process
/// currently sees no configured account, ChannelDB, or session attachment;
/// otherwise both processes can classify a GUI-only async completion as
/// `Absent` and run the same billed parent turn. Foreground Desktop/HTTP
/// mirrors never call this gate and retain their local best-effort behavior.
fn parent_injection_ownership_gate(
    ownership: ImDeliveryOwnership,
    tier: Option<ha_core::runtime_lock::Tier>,
) -> Option<ImLiveMirrorAttach> {
    if matches!(tier, Some(ha_core::runtime_lock::Tier::Secondary)) {
        return Some(ImLiveMirrorAttach::DeferredToPrimary { account_id: None });
    }
    match ownership {
        ImDeliveryOwnership::RemoteOwner => {
            Some(ImLiveMirrorAttach::DeferredToPrimary { account_id: None })
        }
        ImDeliveryOwnership::Disabled => Some(ImLiveMirrorAttach::Absent),
        ImDeliveryOwnership::LocalOwner => None,
    }
}

pub(crate) fn owns_parent_injection_delivery() -> bool {
    parent_injection_ownership_gate(
        ha_core::channel_hooks::im_delivery_ownership(),
        ha_core::runtime_lock::tier(),
    )
    .is_none()
}

type MirrorAttachKey = (String, i64, MirrorGeneration);

/// A completed claim must outlive the provider future that produced it: attach
/// catch-up can observe the same generation immediately after that future
/// drops and must not acquire a second mirror for the already-delivered reply.
/// Five minutes is deliberately much longer than the process-local late-mirror
/// handshake (including its 30-second account-readiness window).
const COMPLETED_MIRROR_ATTACH_RETENTION: Duration = Duration::from_secs(5 * 60);
/// Keep the tombstone registry bounded even if a long-lived process serves many
/// distinct sessions. Under extreme bursts the oldest completion is evicted;
/// the generous cap preserves every realistic in-flight handover window.
const MAX_COMPLETED_MIRROR_ATTACH_TOMBSTONES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MirrorAttachEntry {
    Active,
    Completed { sequence: u64 },
}

struct CompletedMirrorAttach {
    key: MirrorAttachKey,
    sequence: u64,
    completed_at: Instant,
}

#[derive(Default)]
struct MirrorAttachRegistry {
    entries: HashMap<MirrorAttachKey, MirrorAttachEntry>,
    completed: VecDeque<CompletedMirrorAttach>,
    next_sequence: u64,
}

impl MirrorAttachRegistry {
    fn prune_completed(&mut self, now: Instant) {
        while self.completed.front().is_some_and(|entry| {
            now.saturating_duration_since(entry.completed_at) >= COMPLETED_MIRROR_ATTACH_RETENTION
        }) {
            self.remove_oldest_completed();
        }
        while self.completed.len() > MAX_COMPLETED_MIRROR_ATTACH_TOMBSTONES {
            self.remove_oldest_completed();
        }
    }

    fn remove_oldest_completed(&mut self) {
        let Some(oldest) = self.completed.pop_front() else {
            return;
        };
        if matches!(
            self.entries.get(&oldest.key),
            Some(MirrorAttachEntry::Completed { sequence }) if *sequence == oldest.sequence
        ) {
            self.entries.remove(&oldest.key);
        }
    }

    fn mark_completed(&mut self, key: &MirrorAttachKey) -> bool {
        if !matches!(self.entries.get(key), Some(MirrorAttachEntry::Active)) {
            return false;
        }
        let completed_at = Instant::now();
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let sequence = self.next_sequence;
        self.entries
            .insert(key.clone(), MirrorAttachEntry::Completed { sequence });
        self.completed.push_back(CompletedMirrorAttach {
            key: key.clone(),
            sequence,
            completed_at,
        });
        self.prune_completed(completed_at);
        true
    }
}

static MIRROR_ATTACH_REGISTRY: OnceLock<Mutex<MirrorAttachRegistry>> = OnceLock::new();

fn mirror_attaches() -> &'static Mutex<MirrorAttachRegistry> {
    MIRROR_ATTACH_REGISTRY.get_or_init(|| Mutex::new(MirrorAttachRegistry::default()))
}

pub(crate) struct MirrorAttachGuard {
    session_id: String,
    attach_id: i64,
    generation: MirrorGeneration,
    released: bool,
}

impl MirrorAttachGuard {
    fn release(&mut self) {
        if self.released {
            return;
        }
        if let Ok(mut registry) = mirror_attaches().lock() {
            let key = (
                self.session_id.clone(),
                self.attach_id,
                self.generation.clone(),
            );
            if matches!(registry.entries.get(&key), Some(MirrorAttachEntry::Active)) {
                registry.entries.remove(&key);
            }
        }
        self.released = true;
    }

    /// Retain a bounded tombstone after an attempted provider terminal must
    /// fence replay (whether confirmed or ambiguous). Drop deliberately
    /// defaults to [`Self::release`]; construction failures, moved attaches,
    /// and paths that produced no terminal must remain retryable rather than
    /// being mistaken for delivered output.
    pub(crate) fn complete(&mut self) {
        if self.released {
            return;
        }
        if let Ok(mut registry) = mirror_attaches().lock() {
            let key = (
                self.session_id.clone(),
                self.attach_id,
                self.generation.clone(),
            );
            if registry.mark_completed(&key) {
                self.released = true;
            }
        }
    }
}

impl Drop for MirrorAttachGuard {
    fn drop(&mut self) {
        self.release();
    }
}

pub(crate) enum MirrorAttachClaim {
    Busy,
    Completed,
    Unavailable,
    Attached(MirrorAttachGuard),
}

pub(crate) fn try_claim_mirror_attach(
    session_id: &str,
    attach_id: i64,
    generation: MirrorGeneration,
) -> MirrorAttachClaim {
    let mut registry = match mirror_attaches().lock() {
        Ok(registry) => registry,
        Err(_) => {
            app_error!(
                "channel",
                "mirror",
                "IM mirror generation registry lock is poisoned; refusing attach for session {}",
                session_id
            );
            return MirrorAttachClaim::Unavailable;
        }
    };
    let key = (session_id.to_string(), attach_id, generation);
    registry.prune_completed(Instant::now());
    match registry.entries.get(&key) {
        Some(MirrorAttachEntry::Active) => return MirrorAttachClaim::Busy,
        Some(MirrorAttachEntry::Completed { .. }) => return MirrorAttachClaim::Completed,
        None => {}
    }
    // The DB attach can move between a caller's live-row lookup and this
    // in-memory claim. Keep different attach ids for the same generation
    // independent: provider mutations re-check the live DB binding at their
    // boundary, so only the current attach may write. Letting the last claimant
    // evict the other would make a stale pre-handover caller suppress the valid
    // post-handover mirror (or vice versa).
    registry
        .entries
        .insert(key.clone(), MirrorAttachEntry::Active);
    MirrorAttachClaim::Attached(MirrorAttachGuard {
        session_id: key.0,
        attach_id: key.1,
        generation: key.2,
        released: false,
    })
}

pub(crate) fn mirror_attach_claim_is_active(
    session_id: &str,
    attach_id: i64,
    generation: &MirrorGeneration,
) -> bool {
    mirror_attaches()
        .lock()
        .map(|registry| {
            matches!(
                registry
                    .entries
                    .get(&(session_id.to_string(), attach_id, generation.clone())),
                Some(MirrorAttachEntry::Active)
            )
        })
        .unwrap_or(false)
}

struct AttachGuardedSink {
    session_id: String,
    attach_id: i64,
    generation: MirrorGeneration,
    inner: Arc<dyn EventSink>,
}

impl EventSink for AttachGuardedSink {
    fn send(&self, event: &str) {
        if self.generation.is_current(&self.session_id)
            && mirror_attach_claim_is_active(&self.session_id, self.attach_id, &self.generation)
        {
            self.inner.send(event);
        }
    }
}

pub(crate) fn attach_still_matches(session_id: &str, attach_id: i64) -> bool {
    ha_core::globals::get_channel_db()
        .and_then(|db| db.get_conversation_by_session(session_id).ok().flatten())
        .map(|conv| conv.id == attach_id)
        .unwrap_or(false)
}

pub(crate) async fn attach_still_matches_async(session_id: &str, attach_id: i64) -> bool {
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || attach_still_matches(&session_id, attach_id))
        .await
        .unwrap_or(false)
}

pub(crate) fn guarded_mirror_sink(
    session_id: String,
    attach_id: i64,
    generation: MirrorGeneration,
    inner: Arc<dyn EventSink>,
) -> Arc<dyn EventSink> {
    Arc::new(AttachGuardedSink {
        session_id,
        attach_id,
        generation,
        inner,
    })
}

pub(crate) async fn attach_im_live_mirror(
    session_id: &str,
    source: ChatSource,
    generation: MirrorGeneration,
    last_user: Option<LastUserSnapshot>,
) -> ImLiveMirrorAttach {
    if !matches!(source, ChatSource::Desktop | ChatSource::Http) {
        return ImLiveMirrorAttach::Absent;
    }
    attach_im_live_mirror_inner(
        session_id,
        last_user,
        generation,
        MirrorAvailability::WaitInPipeline,
    )
    .await
}

/// G1: mirror a background-completion **injection** turn (`ParentInjection`)
/// into the parent session's attached IM chat. The engine's own
/// [`attach_im_live_mirror`] gates `ParentInjection` out (it's not a
/// `Desktop`/`Http` turn), so `inject_and_run_parent` calls this directly and
/// **awaits** finalize itself — the injection runs on a short-lived
/// current-thread runtime whose drop would cancel a `tokio::spawn`ed finalize.
/// No user-quote is sent: the injection's "message" is the internal
/// `<subagent-result>` envelope, not something to render to the IM user.
pub(crate) async fn attach_im_injection_mirror(
    session_id: &str,
    generation_id: &str,
) -> ImLiveMirrorAttach {
    if let Some(outcome) = parent_injection_ownership_gate(
        ha_core::channel_hooks::im_delivery_ownership(),
        ha_core::runtime_lock::tier(),
    ) {
        return outcome;
    }
    attach_im_live_mirror_inner(
        session_id,
        None,
        MirrorGeneration::injection(generation_id),
        MirrorAvailability::RequireRunning,
    )
    .await
}

async fn attach_im_live_mirror_inner(
    session_id: &str,
    last_user: Option<LastUserSnapshot>,
    generation: MirrorGeneration,
    availability: MirrorAvailability,
) -> ImLiveMirrorAttach {
    let ownership = ha_core::channel_hooks::im_delivery_ownership();
    if ownership == ImDeliveryOwnership::Disabled {
        return ImLiveMirrorAttach::Absent;
    }

    let store = ha_core::config::cached_config();
    if store.channels.accounts.is_empty() {
        // Desktop-only deployments skip the SQL probe entirely.
        return ImLiveMirrorAttach::Absent;
    }

    let Some(channel_db) = ha_core::globals::get_channel_db() else {
        return unavailable_surface_outcome(ownership, availability, None);
    };

    let attach = match channel_db.get_conversation_by_session(session_id) {
        Ok(Some(c)) => c,
        Ok(None) => return ImLiveMirrorAttach::Absent,
        Err(e) => {
            app_warn!(
                "channel",
                "mirror",
                "get_conversation_by_session({}) failed: {}",
                session_id,
                e
            );
            return unavailable_surface_outcome(ownership, availability, None);
        }
    };

    let Some(account) = store.channels.find_account(&attach.account_id).cloned() else {
        // A stale binding whose account was removed no longer represents an
        // external delivery surface.
        return ImLiveMirrorAttach::Absent;
    };
    if !account.enabled {
        return ImLiveMirrorAttach::Absent;
    }
    if ownership == ImDeliveryOwnership::RemoteOwner
        && matches!(availability, MirrorAvailability::RequireRunning)
    {
        // A Secondary never owns a durable ParentInjection IM claim, even if a
        // user manually started a local worker. The Primary's periodic durable
        // sweep is the cross-process handoff backstop.
        return ImLiveMirrorAttach::DeferredToPrimary {
            account_id: Some(account.id.clone()),
        };
    }
    let Some(registry) = ha_core::globals::get_channel_registry() else {
        return unavailable_surface_outcome(ownership, availability, Some(account.id.clone()));
    };
    let Some(plugin) = registry.get_plugin(&account.channel_id).cloned() else {
        return unavailable_surface_outcome(ownership, availability, Some(account.id.clone()));
    };
    let account_is_running = registry.health(&account.id).await.is_running;
    let local_foreground_can_wait = ownership == ImDeliveryOwnership::LocalOwner
        && matches!(availability, MirrorAvailability::WaitInPipeline);
    if !account_is_running && !local_foreground_can_wait {
        // The binding and enabled account both exist, but startup has not made
        // its local worker available yet. Only the full-background Primary may
        // retain work locally; a Secondary delegates ParentInjection and a
        // disabled delivery role proceeds GUI-only.
        return unavailable_surface_outcome(ownership, availability, Some(account.id.clone()));
    }

    let injection_run_id = match &generation {
        MirrorGeneration::Injection(run_id) => Some(run_id.clone()),
        MirrorGeneration::Turn(_) | MirrorGeneration::Stream(_) => None,
    };
    let mirror_guard = match try_claim_mirror_attach(session_id, attach.id, generation.clone()) {
        MirrorAttachClaim::Attached(guard) => guard,
        MirrorAttachClaim::Busy | MirrorAttachClaim::Completed => {
            return ImLiveMirrorAttach::Busy;
        }
        MirrorAttachClaim::Unavailable => {
            return unavailable_surface_outcome(ownership, availability, Some(account.id.clone()));
        }
    };
    let chat_type = ChatType::from_lowercase(&attach.chat_type);

    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        chat_type: &chat_type,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
        recipient_user_id: attach.sender_id.as_deref(),
        recipient_tenant_id: attach.sender_tenant_id.as_deref(),
    };

    // Reserve provider order synchronously, then let the pipeline wait/send
    // asynchronously.  Engine deltas can enter its unbounded queue at once, so
    // a slow quote or account startup never delays GUI first-token delivery.
    let provider_lane = reserve_provider_lane(&target);
    let quote_view = last_user.as_ref().map(|s| LastUserView {
        source: s.source.as_str(),
        text: s.text.as_str(),
        attachment_count: s.attachment_count,
    });
    let quote = build_user_quote_prefix(quote_view.as_ref())
        .map(|body| body.trim_end().to_string())
        .filter(|body| !body.is_empty());
    let quote_sent = Arc::new(AtomicBool::new(false));
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
    let readiness = match (ownership, availability) {
        (ImDeliveryOwnership::LocalOwner, MirrorAvailability::WaitInPipeline) => {
            PipelineAccountReadiness::WaitForRunning {
                account_id: account.id.clone(),
            }
        }
        _ => PipelineAccountReadiness::RequireRunning {
            account_id: account.id.clone(),
        },
    };
    let prelude = StreamPipelinePrelude::new(
        provider_lane.waiter(),
        provider_lane.task_hold(),
        Some(readiness),
        Vec::new(),
        quote,
        quote_sent.clone(),
        still_valid,
    );

    // The originating Desktop / Http turn already drives the
    // `chat:stream_delta` path; suppress the secondary sink's bus emit so
    // the GUI doesn't render every frame twice.
    let pipeline = spawn_stream_pipeline_with_prelude(
        &plugin,
        &account,
        session_id,
        &target,
        true,
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

    ImLiveMirrorAttach::Attached(Box::new(ImLiveMirrorState {
        sink_handle,
        _mirror_guard: mirror_guard,
        pipeline,
        plugin,
        attach,
        injection_run_id,
        quote_sent,
    }))
}

async fn finalize_im_live_mirror(state: DetachedImLiveMirrorState, response: &str) {
    let DetachedImLiveMirrorState {
        _mirror_guard: mut mirror_guard,
        pipeline,
        plugin,
        attach,
        injection_run_id,
        quote_sent: _,
    } = state;

    let outcome = await_stream_pipeline(pipeline).await;

    let chat_type = ChatType::from_lowercase(&attach.chat_type);
    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        chat_type: &chat_type,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
        recipient_user_id: attach.sender_id.as_deref(),
        recipient_tenant_id: attach.sender_tenant_id.as_deref(),
    };

    if !attach_still_matches_async(&attach.session_id, attach.id).await {
        crate::channel::worker::pipeline::abort_pipeline_outcome(
            &outcome,
            ha_core::channel::types::ReplyAbortReason::Detached,
        )
        .await;
        app_info!(
            "channel",
            "mirror",
            "[{}] Skipped GUI mirror finalization to {} because attach moved",
            attach.channel_id,
            attach.chat_id,
        );
        return;
    }

    let injection_snapshot = match (injection_run_id.as_deref(), ha_core::get_session_db()) {
        (Some(run_id), Some(db)) => {
            let snapshot_session_id = attach.session_id.clone();
            let snapshot_run_id = run_id.to_string();
            tokio::task::spawn_blocking(move || {
                let user_id =
                    db.injection_user_message_id(&snapshot_session_id, &snapshot_run_id)?;
                Ok::<_, anyhow::Error>(user_id.and_then(|user_id| {
                    crate::channel::attach_sync::turn_snapshot_after_user(
                        &db,
                        &snapshot_session_id,
                        user_id,
                    )
                }))
            })
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
        }
        _ => None,
    };
    let response_chars = injection_snapshot
        .as_ref()
        .map(|snapshot| snapshot.text.chars().count())
        .unwrap_or_else(|| response.chars().count());
    let metrics = match injection_snapshot {
        Some(snapshot) => {
            deliver_full_response(&plugin, &target, &outcome, &snapshot.text, &snapshot.medias)
                .await
        }
        None => deliver_rounds(&plugin, &target, &outcome, response).await,
    };
    // The attach was still current and at least one provider mutation was
    // attempted. Retain the exact generation even when its outcome was
    // ambiguous: foreground turns are never automatically replayed, and a
    // catch-up retry could otherwise duplicate a partially accepted terminal.
    // A zero-attempt empty/no-preview result produced no IM terminal and keeps
    // the claim retryable.
    if metrics.report.attempted > 0 {
        mirror_guard.complete();
    }
    if metrics.report.is_success() {
        app_info!(
            "channel",
            "mirror",
            "[{}] Mirrored GUI reply to {} (response_chars={}, delivered_text_chars={}, media={}, sends={})",
            attach.channel_id,
            attach.chat_id,
            response_chars,
            metrics.text_chars,
            metrics.media_count,
            metrics.report.succeeded,
        );
    } else if let Some(db) = ha_core::get_session_db() {
        let warn_context = format!(
            "[{}] GUI reply generated but IM mirror failed for session {}",
            attach.channel_id, attach.session_id,
        );
        crate::channel::worker::report_delivery_failure(
            &db,
            &attach.session_id,
            &warn_context,
            "⚠️ The reply was generated in Hope, but its IM mirror delivery failed or was incomplete.",
            &metrics.report,
        )
        .await;
    }
}

/// Drain + clean up a live mirror without a final response. Called from
/// engine cancel / final-failure paths in place of `finalize_im_live_mirror`.
///
/// `body: Some(_)` finalizes the currently visible preview with bounded,
/// caller-rendered terminal copy. This also applies when there is no user quote:
/// ParentInjection deliberately omits the quote, but its assistant preview can
/// already be visible and must not be left dangling before a retry.
/// `body: None` remains the no-follow-up cancellation path.
///
/// Like `finalize`, drops the sink handle first so the stream task
/// observes channel-close cleanly, then awaits the pipeline.
/// Owned-string variant for the unified finalize path. The unified
/// path renders the IM-side notice via `finalize::copy::im_notice`
/// (which itself dispatches to `format_im_engine_error` / `CANCEL_NOTICE`
/// for provider-failure / user-cancel reasons) and passes the result
/// here as `body`. Passing `None` skips the follow-up message —
/// equivalent to the no-op-on-no-quote case the old per-reason
/// dispatcher had.
///
/// Pre-finalize callers (subagent / channel inbound paths that don't
/// build an `ImLiveMirrorState` from a `ChatSource::Desktop`/`Http`
/// turn) never instantiate this state at all, so this is the sole
/// entry point.
async fn abort_im_live_mirror_with_body(
    state: DetachedImLiveMirrorState,
    body: Option<String>,
) -> ha_core::channel_hooks::ImLiveMirrorAbortStatus {
    let DetachedImLiveMirrorState {
        _mirror_guard: mut mirror_guard,
        pipeline,
        plugin,
        attach,
        injection_run_id,
        quote_sent,
    } = state;
    let outcome = await_stream_pipeline(pipeline).await;
    let Some(body) = body else {
        let confirmed = crate::channel::worker::pipeline::abort_pipeline_outcome_for_replay(
            &outcome,
            ha_core::channel::types::ReplyAbortReason::Cancelled,
        )
        .await;
        if !confirmed {
            // `Unsafe` is itself proof that a persistent provider mutation may
            // remain visible, even when no standalone terminal send was
            // attempted. Keep the generation fenced; only a confirmed abort
            // may release a ParentInjection receipt for same-run replay.
            mirror_guard.complete();
        }
        return ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(confirmed);
    };
    if !attach_still_matches_async(&attach.session_id, attach.id).await {
        let confirmed = crate::channel::worker::pipeline::abort_pipeline_outcome_for_replay(
            &outcome,
            ha_core::channel::types::ReplyAbortReason::Failed,
        )
        .await;
        return ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(confirmed);
    }
    let chat_type = ChatType::from_lowercase(&attach.chat_type);
    let target = DeliveryTarget {
        account_id: &attach.account_id,
        chat_id: &attach.chat_id,
        chat_type: &chat_type,
        thread_id: attach.thread_id.as_deref(),
        reply_to_message_id: None,
        recipient_user_id: attach.sender_id.as_deref(),
        recipient_tenant_id: attach.sender_tenant_id.as_deref(),
    };
    let report =
        crate::channel::worker::pipeline::deliver_error_reply(&plugin, &target, &outcome, &body)
            .await;
    let terminal_confirmed = report.is_success()
        && !report.unsafe_to_continue
        && crate::channel::worker::pipeline::error_terminal_allows_replay(&outcome);
    match injection_run_id {
        // Foreground Desktop/HTTP turns never replay this generation. A sent
        // or ambiguous provider terminal must fence a later handover catch-up
        // from opening a second identity.
        None if report.attempted > 0 || !terminal_confirmed => mirror_guard.complete(),
        // ParentInjection deliberately reuses its run generation after a
        // confirmed, in-place abort. Leave that claim releasable so the core
        // coordinator can retry the same durable receipt. `Unsafe` itself is
        // enough to fence, even when the report recorded no fresh send.
        Some(_) if !terminal_confirmed => mirror_guard.complete(),
        None | Some(_) => {}
    }
    app_info!(
        "channel",
        "mirror",
        "[{}] Aborted GUI mirror to {} — error terminal success={} quote_sent={}",
        attach.channel_id,
        attach.chat_id,
        terminal_confirmed,
        quote_sent.load(Ordering::Acquire),
    );
    ha_core::channel_hooks::ImLiveMirrorAbortStatus::from_confirmed(terminal_confirmed)
}

// ── kernel 回调面适配 ────────────────────────────────────────────────
//
// `chat_engine` 只 attach → 持有 → 收尾，从不读状态字段，故整体以 trait
// object 过边界（见 `ha_core::channel_hooks::ImLiveMirror`）。

impl ha_core::channel_hooks::ImLiveMirror for ImLiveMirrorState {
    fn finalize(
        self: Box<Self>,
        response: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        // This conversion drops SinkHandle now, before the returned future can
        // be spawned or polled. A following generation therefore cannot fan
        // out its first delta to this already-terminal mirror.
        let state = (*self).into_detached();
        Box::pin(async move { finalize_im_live_mirror(state, &response).await })
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
        let state = (*self).into_detached();
        Box::pin(async move { abort_im_live_mirror_with_body(state, body).await })
    }
}

pub(crate) fn attach_live_hook<'a>(
    session_id: &'a str,
    source: ha_core::chat_engine::stream_seq::ChatSource,
    generation: ha_core::channel_hooks::ImLiveMirrorGeneration,
    last_user: Option<LastUserSnapshot>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = ha_core::channel_hooks::ImLiveMirrorAttach> + Send + 'a>,
> {
    let generation = match generation {
        ha_core::channel_hooks::ImLiveMirrorGeneration::Turn(turn_id) => {
            MirrorGeneration::Turn(turn_id)
        }
        ha_core::channel_hooks::ImLiveMirrorGeneration::Stream(stream_id) => {
            MirrorGeneration::Stream(stream_id)
        }
    };
    Box::pin(async move { attach_im_live_mirror(session_id, source, generation, last_user).await })
}

pub(crate) fn attach_injection_hook<'a>(
    session_id: &'a str,
    generation_id: &'a str,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = ha_core::channel_hooks::ImLiveMirrorAttach> + Send + 'a>,
> {
    Box::pin(async move { attach_im_injection_mirror(session_id, generation_id).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct CountingSink(AtomicUsize);

    impl CountingSink {
        fn new() -> Self {
            Self(AtomicUsize::new(0))
        }

        fn count(&self) -> usize {
            self.0.load(Ordering::Relaxed)
        }
    }

    impl EventSink for CountingSink {
        fn send(&self, _event: &str) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn stopped_surface_respects_process_delivery_ownership() {
        assert!(matches!(
            unavailable_surface_outcome(
                ImDeliveryOwnership::LocalOwner,
                MirrorAvailability::RequireRunning,
                Some("account".to_string()),
            ),
            ImLiveMirrorAttach::Unavailable { .. }
        ));
        assert!(matches!(
            unavailable_surface_outcome(
                ImDeliveryOwnership::RemoteOwner,
                MirrorAvailability::RequireRunning,
                Some("account".to_string()),
            ),
            ImLiveMirrorAttach::DeferredToPrimary { .. }
        ));
        assert!(matches!(
            unavailable_surface_outcome(
                ImDeliveryOwnership::RemoteOwner,
                MirrorAvailability::WaitInPipeline,
                Some("account".to_string()),
            ),
            ImLiveMirrorAttach::Absent
        ));
        assert!(matches!(
            unavailable_surface_outcome(
                ImDeliveryOwnership::Disabled,
                MirrorAvailability::RequireRunning,
                Some("account".to_string()),
            ),
            ImLiveMirrorAttach::Absent
        ));
    }

    #[test]
    fn parent_injection_ownership_precedes_account_and_attach_lookup() {
        assert!(matches!(
            parent_injection_ownership_gate(ImDeliveryOwnership::RemoteOwner, None),
            Some(ImLiveMirrorAttach::DeferredToPrimary { account_id: None })
        ));
        assert!(
            parent_injection_ownership_gate(ImDeliveryOwnership::LocalOwner, None).is_none(),
            "Primary must continue to the normal account/attach readiness lookup"
        );
        assert!(matches!(
            parent_injection_ownership_gate(ImDeliveryOwnership::Disabled, None),
            Some(ImLiveMirrorAttach::Absent)
        ));
        for ownership in [
            ImDeliveryOwnership::LocalOwner,
            ImDeliveryOwnership::RemoteOwner,
            ImDeliveryOwnership::Disabled,
        ] {
            assert!(matches!(
                parent_injection_ownership_gate(
                    ownership,
                    Some(ha_core::runtime_lock::Tier::Secondary),
                ),
                Some(ImLiveMirrorAttach::DeferredToPrimary { account_id: None })
            ));
        }
        assert!(matches!(
            parent_injection_ownership_gate(
                ImDeliveryOwnership::Disabled,
                Some(ha_core::runtime_lock::Tier::Primary),
            ),
            Some(ImLiveMirrorAttach::Absent)
        ));

        // The dedicated gate is not used by attach_im_live_mirror: a Secondary
        // foreground Desktop/HTTP turn still degrades locally when its surface
        // is unavailable instead of being handed off as ParentInjection work.
        assert!(matches!(
            unavailable_surface_outcome(
                ImDeliveryOwnership::RemoteOwner,
                MirrorAvailability::WaitInPipeline,
                None,
            ),
            ImLiveMirrorAttach::Absent
        ));
    }

    fn attached(claim: MirrorAttachClaim) -> MirrorAttachGuard {
        match claim {
            MirrorAttachClaim::Attached(guard) => guard,
            MirrorAttachClaim::Busy
            | MirrorAttachClaim::Completed
            | MirrorAttachClaim::Unavailable => {
                panic!("expected mirror attach claim")
            }
        }
    }

    #[test]
    fn same_turn_generation_keeps_engine_and_late_mirror_exactly_once() {
        let session_id = format!("mirror-same-turn-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let _owner = attached(try_claim_mirror_attach(&session_id, 17, generation.clone()));

        assert!(matches!(
            try_claim_mirror_attach(&session_id, 17, generation),
            MirrorAttachClaim::Busy
        ));
    }

    #[test]
    fn completed_generation_cannot_be_reacquired_after_guard_drop() {
        let session_id = format!("mirror-completed-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let mut owner = attached(try_claim_mirror_attach(&session_id, 17, generation.clone()));

        owner.complete();
        drop(owner);

        assert!(matches!(
            try_claim_mirror_attach(&session_id, 17, generation),
            MirrorAttachClaim::Completed
        ));
    }

    #[test]
    fn completed_generation_does_not_block_a_different_attach() {
        let session_id = format!("mirror-completed-rebind-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let mut old = attached(try_claim_mirror_attach(&session_id, 17, generation.clone()));
        old.complete();
        drop(old);

        let _new = attached(try_claim_mirror_attach(&session_id, 18, generation));
    }

    #[test]
    fn ordinary_guard_release_allows_the_same_key_to_retry() {
        let session_id = format!("mirror-released-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let owner = attached(try_claim_mirror_attach(&session_id, 17, generation.clone()));
        drop(owner);

        let _retry = attached(try_claim_mirror_attach(&session_id, 17, generation));
    }

    #[test]
    fn completed_tombstones_are_capacity_bounded() {
        let mut registry = MirrorAttachRegistry::default();
        let prefix = format!("mirror-bounded-{}", uuid::Uuid::new_v4());
        let mut latest_key = None;
        for index in 0..=MAX_COMPLETED_MIRROR_ATTACH_TOMBSTONES {
            let key = (
                format!("{prefix}-{index}"),
                17,
                MirrorGeneration::turn(format!("turn-{index}")),
            );
            registry
                .entries
                .insert(key.clone(), MirrorAttachEntry::Active);
            assert!(registry.mark_completed(&key));
            latest_key = Some(key);
        }

        assert!(registry.completed.len() <= MAX_COMPLETED_MIRROR_ATTACH_TOMBSTONES);
        assert!(
            registry
                .entries
                .keys()
                .filter(|(session_id, _, _)| session_id.starts_with(&prefix))
                .count()
                <= MAX_COMPLETED_MIRROR_ATTACH_TOMBSTONES
        );
        assert!(matches!(
            latest_key
                .as_ref()
                .and_then(|key| registry.entries.get(key)),
            Some(MirrorAttachEntry::Completed { .. })
        ));
    }

    #[test]
    fn expired_completed_tombstone_becomes_retryable() {
        let mut registry = MirrorAttachRegistry::default();
        let key = (
            "mirror-expired".to_string(),
            17,
            MirrorGeneration::turn("turn-a"),
        );
        registry
            .entries
            .insert(key.clone(), MirrorAttachEntry::Active);
        assert!(registry.mark_completed(&key));
        registry
            .completed
            .front_mut()
            .expect("completed tombstone")
            .completed_at = Instant::now()
            .checked_sub(COMPLETED_MIRROR_ATTACH_RETENTION)
            .expect("retention duration must fit");

        registry.prune_completed(Instant::now());

        assert!(!registry.entries.contains_key(&key));
        assert!(registry.completed.is_empty());
    }

    #[test]
    fn slow_previous_finalize_does_not_hide_the_next_gui_turn() {
        let session_id = format!("mirror-next-turn-{}", uuid::Uuid::new_v4());
        // Keeping this guard alive models turn A's detached finalizer blocking
        // on a slow provider mutation.
        let _slow_previous = attached(try_claim_mirror_attach(
            &session_id,
            23,
            MirrorGeneration::turn("turn-a"),
        ));

        let _next = attached(try_claim_mirror_attach(
            &session_id,
            23,
            MirrorGeneration::turn("turn-b"),
        ));
    }

    #[test]
    fn slow_previous_finalize_does_not_hide_parent_injection() {
        let session_id = format!("mirror-injection-{}", uuid::Uuid::new_v4());
        let _slow_previous = attached(try_claim_mirror_attach(
            &session_id,
            29,
            MirrorGeneration::turn("turn-a"),
        ));

        let _injection = attached(try_claim_mirror_attach(
            &session_id,
            29,
            MirrorGeneration::injection("run-b"),
        ));
    }

    #[test]
    fn same_generation_old_then_new_attach_claims_coexist() {
        let session_id = format!("mirror-rebind-old-new-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let old = attached(try_claim_mirror_attach(&session_id, 31, generation.clone()));
        let new = attached(try_claim_mirror_attach(&session_id, 32, generation.clone()));

        assert!(mirror_attach_claim_is_active(&session_id, 31, &generation));
        assert!(mirror_attach_claim_is_active(&session_id, 32, &generation));
        drop(old);
        assert!(
            mirror_attach_claim_is_active(&session_id, 32, &generation),
            "old guard drop must not remove the new attach claim"
        );
        drop(new);
    }

    #[test]
    fn same_generation_new_then_stale_old_attach_claims_coexist() {
        let session_id = format!("mirror-rebind-new-old-{}", uuid::Uuid::new_v4());
        let generation = MirrorGeneration::turn("turn-a");
        let new = attached(try_claim_mirror_attach(&session_id, 32, generation.clone()));
        // Models a caller that read attach 31 before the handover, then resumed
        // and claimed only after the valid attach-32 mirror was installed.
        let stale_old = attached(try_claim_mirror_attach(&session_id, 31, generation.clone()));

        assert!(mirror_attach_claim_is_active(&session_id, 32, &generation));
        assert!(mirror_attach_claim_is_active(&session_id, 31, &generation));
        drop(stale_old);
        assert!(
            mirror_attach_claim_is_active(&session_id, 32, &generation),
            "stale old guard drop must not remove the new attach claim"
        );
        drop(new);
    }

    #[test]
    fn terminal_future_detaches_a_before_b_can_emit() {
        let session_id = format!("mirror-terminal-detach-{}", uuid::Uuid::new_v4());
        let registry = sink_registry();
        let sink_a = Arc::new(CountingSink::new());
        let sink_a_dyn: Arc<dyn EventSink> = sink_a.clone();
        let handle_a = registry.attach(session_id.clone(), sink_a_dyn);

        registry.emit(&session_id, "a-delta");
        assert_eq!(sink_a.count(), 1);

        // The continuation intentionally remains unpolled. This is the same
        // helper used by ImLiveMirrorState::into_detached when finalize/abort
        // constructs its future, so detach must already be complete here.
        let _unpolled_terminal = detach_sink_before(handle_a, std::future::pending::<()>());

        let sink_b = Arc::new(CountingSink::new());
        let sink_b_dyn: Arc<dyn EventSink> = sink_b.clone();
        let _handle_b = registry.attach(session_id.clone(), sink_b_dyn);
        registry.emit(&session_id, "b-delta");

        assert_eq!(sink_a.count(), 1, "terminal A received generation B");
        assert_eq!(sink_b.count(), 1);
    }

    #[test]
    fn late_a_sink_rejects_b_before_a_poll_observes_terminal() {
        let session_id = format!("late-mirror-generation-{}", uuid::Uuid::new_v4());
        let turn_a = "turn-a".to_string();
        let guard_a = ha_core::chat_engine::active_turn::try_acquire(
            &session_id,
            ChatSource::Desktop,
            turn_a.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("turn A should acquire");
        let generation = MirrorGeneration::turn(turn_a);
        let _claim = attached(try_claim_mirror_attach(&session_id, 41, generation.clone()));
        let inner = Arc::new(CountingSink::new());
        let guarded = AttachGuardedSink {
            session_id: session_id.clone(),
            attach_id: 41,
            generation,
            inner: inner.clone(),
        };

        guarded.send("a-delta");
        assert_eq!(inner.count(), 1);

        // Model LateMirror's polling window: A has ended and B is active, but
        // the A task has not yet reached its 100ms poll/drop of SinkHandle.
        drop(guard_a);
        let _guard_b = ha_core::chat_engine::active_turn::try_acquire(
            &session_id,
            ChatSource::Desktop,
            "turn-b".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("turn B should acquire");
        guarded.send("b-first-delta");

        assert_eq!(inner.count(), 1, "late A sink consumed B's first delta");
    }
}

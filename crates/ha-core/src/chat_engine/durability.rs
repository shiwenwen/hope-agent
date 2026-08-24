use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use crate::session::{
    CreateStreamRun, JournalBatch, JournalEvent, SessionDB, StreamRunRegistration,
};
use crate::turn_durability::{
    DispatchClaim, DurableRequestRole, FlushReason, PrepareRequestPlan, RequestTerminalOutcome,
    ResponseStarted, StreamSnapshot, TurnDurabilitySink,
};

use super::sink_registry;
use super::stream_broadcast;
use super::stream_seq::ChatSource;
use super::CapturedUsage;
use super::EventSink;

const FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const FLUSH_BYTES: usize = 16 * 1024;
const SOFT_LAG: Duration = Duration::from_secs(2);
const HARD_LAG: Duration = Duration::from_secs(10);
const SOFT_DIRTY_BYTES: usize = 1024 * 1024;
const HARD_DIRTY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct QueuedEvent {
    seq: u64,
    payload: String,
    bytes: usize,
    merge_role: Option<MergeRole>,
}

struct PreparedBatch {
    batch: JournalBatch,
    queued: Vec<QueuedEvent>,
    journal_events: Vec<JournalEvent>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MergeRole {
    Text,
    Thinking,
}

fn coalesce_journal_events(queued: &[QueuedEvent]) -> Result<Vec<JournalEvent>> {
    type PendingSegment = (MergeRole, u64, u64, serde_json::Value, String);

    let mut out = Vec::new();
    let mut pending: Option<PendingSegment> = None;
    let flush_pending =
        |pending: &mut Option<PendingSegment>, out: &mut Vec<JournalEvent>| -> Result<()> {
            let Some((_role, seq_start, seq_end, mut template, content)) = pending.take() else {
                return Ok(());
            };
            let object = template
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("mergeable journal event is not an object"))?;
            object.insert("content".to_string(), serde_json::json!(content));
            object.insert("_oc_seq".to_string(), serde_json::json!(seq_end));
            out.push(JournalEvent::range(
                seq_start,
                seq_end,
                template.to_string(),
            ));
            Ok(())
        };

    for event in queued {
        let Some(role) = event.merge_role else {
            flush_pending(&mut pending, &mut out)?;
            out.push(JournalEvent::single(event.seq, event.payload.clone()));
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&event.payload)?;
        let Some(content) = value
            .get("content")
            .and_then(|content| content.as_str())
            .map(ToOwned::to_owned)
        else {
            flush_pending(&mut pending, &mut out)?;
            out.push(JournalEvent::single(event.seq, event.payload.clone()));
            continue;
        };
        if let Some((pending_role, _, seq_end, _, merged)) = pending.as_mut() {
            if *pending_role == role && event.seq == seq_end.saturating_add(1) {
                merged.push_str(&content);
                *seq_end = event.seq;
                continue;
            }
        }
        flush_pending(&mut pending, &mut out)?;
        pending = Some((role, event.seq, event.seq, value, content));
    }
    flush_pending(&mut pending, &mut out)?;
    Ok(out)
}

struct CoordinatorState {
    pending: Vec<QueuedEvent>,
    queued_bytes: usize,
    oldest_pending: Option<Instant>,
    next_block_no: u64,
    attempt_no: u32,
    durable_events: Vec<JournalEvent>,
    spool_active: bool,
    fatal_error: Option<String>,
    status: String,
    last_merge_role: Option<MergeRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeRequestPlanState {
    ContextCommitted,
    Dispatching,
    ResponseStarted,
    SendUnknown,
    Terminal,
    Superseded,
}

impl RuntimeRequestPlanState {
    fn possibly_sent(self) -> bool {
        matches!(
            self,
            Self::Dispatching | Self::ResponseStarted | Self::SendUnknown
        )
    }
}

impl From<RuntimeRequestPlanState> for crate::session::InterruptedRequestPlanState {
    fn from(value: RuntimeRequestPlanState) -> Self {
        match value {
            RuntimeRequestPlanState::ContextCommitted => Self::Unsent,
            RuntimeRequestPlanState::Dispatching => Self::Dispatching,
            RuntimeRequestPlanState::ResponseStarted => Self::ResponseStarted,
            RuntimeRequestPlanState::SendUnknown => Self::SendUnknown,
            RuntimeRequestPlanState::Terminal | RuntimeRequestPlanState::Superseded => {
                unreachable!("active request snapshot cannot contain a terminal plan")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveRequestPlanSnapshot {
    pub request_plan_id: String,
    pub role: DurableRequestRole,
    pub attempt_no: u32,
    pub state: RuntimeRequestPlanState,
}

struct RuntimeRequestPlan {
    role: DurableRequestRole,
    attempt_no: u32,
    sequence: u64,
    state: RuntimeRequestPlanState,
    body_keyed_fingerprint: String,
    body_len: u64,
    endpoint_kind: String,
    content_type: String,
    exact_payload_id: Option<String>,
}

#[derive(Default)]
struct RuntimeRequestPlans {
    next_sequence: u64,
    plans: HashMap<String, RuntimeRequestPlan>,
}

impl Default for CoordinatorState {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            queued_bytes: 0,
            oldest_pending: None,
            next_block_no: 1,
            attempt_no: 0,
            durable_events: Vec::new(),
            spool_active: false,
            fatal_error: None,
            status: "running".to_string(),
            last_merge_role: None,
        }
    }
}

fn lock_state(mutex: &Mutex<CoordinatorState>) -> MutexGuard<'_, CoordinatorState> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn request_projection_role(
    role: DurableRequestRole,
) -> crate::session::context_projection::RequestProjectionRole {
    use crate::session::context_projection::RequestProjectionRole as StoredRole;
    match role {
        DurableRequestRole::MainContinuation => StoredRole::MainContinuation,
        DurableRequestRole::Tier3SummaryInput => StoredRole::Tier3SummaryInput,
        DurableRequestRole::SideQuery => StoredRole::SideQuery,
    }
}

fn projection_action(action: &str) -> Result<crate::session::context_projection::ProjectionAction> {
    use crate::session::context_projection::ProjectionAction as StoredAction;
    match action {
        "tier0_omit" => Ok(StoredAction::Tier0Omit),
        "tier2_soft" => Ok(StoredAction::Tier2Soft),
        "tier2_minimal" => Ok(StoredAction::Tier2Minimal),
        other => anyhow::bail!("unknown durable projection action: {other}"),
    }
}

fn verify_exact_body(input: &PrepareRequestPlan, exact_body: &[u8]) -> Result<()> {
    let actual_len = u64::try_from(exact_body.len())
        .map_err(|_| anyhow::anyhow!("prepared request body length exceeds u64"))?;
    if actual_len != input.body_len {
        anyhow::bail!(
            "prepared request body length changed: planned {}, actual {actual_len}",
            input.body_len
        );
    }
    let actual_fingerprint =
        crate::cache_routing::audit_fingerprint("prepared-provider-request-body-v1", exact_body);
    if actual_fingerprint != input.body_keyed_fingerprint {
        anyhow::bail!("prepared request body fingerprint changed before staging");
    }
    Ok(())
}

pub struct StreamCoordinator {
    db: Arc<SessionDB>,
    session_id: String,
    source: ChatSource,
    stream_id: Option<String>,
    turn_id: Option<String>,
    run_id: String,
    admitted_stop_epoch: u64,
    admitted_global_stop_epoch: u64,
    admitted_global_stop_receipt_count: u64,
    persistent: bool,
    event_sink: Arc<dyn EventSink>,
    cancel: Arc<AtomicBool>,
    state: Mutex<CoordinatorState>,
    backpressure: Condvar,
    durable_notify: tokio::sync::Notify,
    accepted_seq: AtomicU64,
    durable_seq: AtomicU64,
    committed_seq: AtomicU64,
    context_revision: AtomicI64,
    attempt_base_context_json: Mutex<Option<String>>,
    attempt_base_contains_current_user: AtomicBool,
    attempt_no: AtomicU32,
    provider_shape: Mutex<Option<String>>,
    closed: AtomicBool,
    captured_usage: Mutex<CapturedUsage>,
    had_thinking: AtomicBool,
    had_text: AtomicBool,
    had_tool_activity: AtomicBool,
    had_non_replayable_tool_activity: AtomicBool,
    request_plans: Mutex<RuntimeRequestPlans>,
    incognito_payloads: Mutex<crate::session::request_payload_store::IncognitoExactPayloadStore>,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<StreamCoordinator>>>> = OnceLock::new();
static GLOBAL_WRITER_NOTIFY: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();
static GLOBAL_WRITER_STARTED: AtomicBool = AtomicBool::new(false);
static GLOBAL_WRITER_START_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static GROUP_COMMIT_ENABLED: OnceLock<bool> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Weak<StreamCoordinator>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn global_writer_start_lock() -> &'static Mutex<()> {
    GLOBAL_WRITER_START_LOCK.get_or_init(|| Mutex::new(()))
}

fn global_writer_notify() -> &'static Arc<tokio::sync::Notify> {
    GLOBAL_WRITER_NOTIFY.get_or_init(|| Arc::new(tokio::sync::Notify::new()))
}

fn group_commit_enabled() -> bool {
    *GROUP_COMMIT_ENABLED.get_or_init(|| {
        std::env::var("HA_STREAM_DURABILITY_LEGACY_WRITER")
            .ok()
            .as_deref()
            != Some("1")
    })
}

impl StreamCoordinator {
    pub async fn create(
        db: Arc<SessionDB>,
        session_id: String,
        source: ChatSource,
        stream_id: Option<String>,
        turn_id: Option<String>,
        event_sink: Arc<dyn EventSink>,
        cancel: Arc<AtomicBool>,
        stop_admission: Option<crate::session::ForegroundStopAdmission>,
    ) -> Result<Arc<Self>> {
        let run_id = uuid::Uuid::new_v4().to_string();
        let create = CreateStreamRun {
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            source: source.as_str().to_string(),
            stream_id: stream_id.clone(),
            turn_id: turn_id.clone(),
            provider_shape: None,
        };
        let registration = db
            .run(move |db| db.create_stream_run_with_stop_admission(&create, stop_admission))
            .await?;
        Self::from_admission(
            db,
            session_id,
            source,
            stream_id,
            turn_id,
            event_sink,
            cancel,
            registration,
        )
        .await
    }

    /// Attach the runtime coordinator to a stream row already committed by
    /// TurnKernel's atomic interactive admission.
    #[allow(clippy::too_many_arguments)]
    #[doc(hidden)]
    pub async fn from_admission(
        db: Arc<SessionDB>,
        session_id: String,
        source: ChatSource,
        stream_id: Option<String>,
        turn_id: Option<String>,
        event_sink: Arc<dyn EventSink>,
        cancel: Arc<AtomicBool>,
        registration: StreamRunRegistration,
    ) -> Result<Arc<Self>> {
        let run_id = registration.run_id.clone();
        let coordinator = Arc::new(Self {
            db,
            session_id: session_id.clone(),
            source,
            stream_id,
            turn_id,
            run_id,
            admitted_stop_epoch: registration.admitted_stop_epoch,
            admitted_global_stop_epoch: registration.admitted_global_stop_epoch,
            admitted_global_stop_receipt_count: registration.admitted_global_stop_receipt_count,
            persistent: registration.persistent,
            event_sink,
            cancel,
            state: Mutex::new(CoordinatorState::default()),
            backpressure: Condvar::new(),
            durable_notify: tokio::sync::Notify::new(),
            accepted_seq: AtomicU64::new(0),
            durable_seq: AtomicU64::new(0),
            committed_seq: AtomicU64::new(0),
            context_revision: AtomicI64::new(registration.context_revision),
            attempt_base_context_json: Mutex::new(registration.initial_context_json),
            attempt_base_contains_current_user: AtomicBool::new(false),
            attempt_no: AtomicU32::new(0),
            provider_shape: Mutex::new(None),
            closed: AtomicBool::new(false),
            captured_usage: Mutex::new(CapturedUsage::default()),
            had_thinking: AtomicBool::new(false),
            had_text: AtomicBool::new(false),
            had_tool_activity: AtomicBool::new(false),
            had_non_replayable_tool_activity: AtomicBool::new(false),
            request_plans: Mutex::new(RuntimeRequestPlans::default()),
            incognito_payloads: Mutex::new(
                crate::session::request_payload_store::IncognitoExactPayloadStore::new(
                    session_id.clone(),
                    None,
                )?,
            ),
        });
        let registered = {
            let mut map = registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let occupied = map
                .get(&session_id)
                .and_then(Weak::upgrade)
                .is_some_and(|existing| !existing.closed.load(Ordering::SeqCst));
            if occupied {
                false
            } else {
                map.insert(session_id.clone(), Arc::downgrade(&coordinator));
                true
            }
        };
        if !registered {
            if registration.persistent {
                let run_id_for_cleanup = coordinator.run_id.clone();
                let cleanup_db = coordinator.db.clone();
                cleanup_db
                    .run(move |db| {
                        db.interrupt_stream_run(
                            &run_id_for_cleanup,
                            0,
                            crate::session::ChatTurnStatus::Failed,
                            Some("concurrent_stream"),
                            Some("another durability coordinator is already active"),
                        )
                    })
                    .await?;
            }
            anyhow::bail!(
                "another durability coordinator is already active for session {session_id}"
            );
        }
        if registration.persistent && source.carries_foreground_user_intent() {
            let resolution_db = coordinator.db.clone();
            let resolution_session_id = session_id.clone();
            let new_run_id = coordinator.run_id.clone();
            let resolution = resolution_db
                .run(move |db| {
                    db.resolve_send_unknown_for_manual_foreground_run(
                        &resolution_session_id,
                        &new_run_id,
                    )
                })
                .await;
            match resolution {
                Ok(0) => {}
                Ok(count) => app_info!(
                    "provider",
                    "manual_retry_as_new",
                    "foreground run {} resolved {} ambiguous prior request plan(s)",
                    coordinator.run_id,
                    count
                ),
                Err(error) => {
                    // Admission of the new foreground run is the only manual
                    // retry authority. If its atomic WAL convergence fails,
                    // stop before any Provider body can be prepared/sent.
                    Self::unregister(&session_id, &coordinator.run_id);
                    let cleanup_db = coordinator.db.clone();
                    let cleanup_run_id = coordinator.run_id.clone();
                    let _ = cleanup_db
                        .run(move |db| {
                            db.interrupt_stream_run(
                                &cleanup_run_id,
                                0,
                                crate::session::ChatTurnStatus::Failed,
                                Some("manual_retry_resolution_failed"),
                                Some("ambiguous request WAL convergence failed"),
                            )
                        })
                        .await;
                    return Err(error.context(
                        "cannot begin foreground turn until ambiguous prior request state converges",
                    ));
                }
            }
        } else if !registration.persistent {
            // Incognito has no cross-turn request WAL by design. Its
            // SendUnknown plan/body live only in this coordinator's bounded
            // memory store and are zeroized when the turn owner is dropped;
            // a later foreground turn is necessarily a brand-new request.
        }
        if let Err(error) = Self::spawn_global_writer() {
            Self::unregister(&session_id, &coordinator.run_id);
            if registration.persistent {
                let run_id_for_cleanup = coordinator.run_id.clone();
                let cleanup_db = coordinator.db.clone();
                let _ = cleanup_db
                    .run(move |db| {
                        db.interrupt_stream_run(
                            &run_id_for_cleanup,
                            0,
                            crate::session::ChatTurnStatus::Failed,
                            Some("writer_start_failed"),
                            Some("global durability writer could not start"),
                        )
                    })
                    .await;
            }
            return Err(error);
        }
        Ok(coordinator)
    }

    fn spawn_global_writer() -> Result<()> {
        let _start_guard = global_writer_start_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if GLOBAL_WRITER_STARTED.load(Ordering::SeqCst) {
            return Ok(());
        }
        let notify = global_writer_notify().clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| anyhow::anyhow!("cannot build global durability writer: {error}"))?;
        std::thread::Builder::new()
            .name("ha-stream-writer".to_string())
            .spawn(move || {
                runtime.block_on(async move {
                    // The writer owns an independent runtime. Backpressure may
                    // synchronously pause a provider callback (including ACP's
                    // current-thread runtime) without starving the task which
                    // has to release that pressure.
                    let mut interval = tokio::time::interval_at(
                        tokio::time::Instant::now() + FLUSH_INTERVAL,
                        FLUSH_INTERVAL,
                    );
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {}
                            _ = notify.notified() => {}
                        }
                        Self::flush_all_pending().await;
                    }
                });
            })
            .map_err(|error| anyhow::anyhow!("cannot start global durability writer: {error}"))?;
        GLOBAL_WRITER_STARTED.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn fail_fatal(&self, error: String) {
        {
            let mut state = lock_state(&self.state);
            if state.fatal_error.is_none() {
                state.fatal_error = Some(error.clone());
                state.status = "persistence_unavailable".to_string();
            }
        }
        self.cancel.store(true, Ordering::SeqCst);
        self.backpressure.notify_all();
        self.durable_notify.notify_waiters();
        app_error!(
            "chat",
            "stream_durability",
            "run {} stopped because neither SQLite nor spool was durable: {}",
            self.run_id,
            error
        );
    }

    fn should_flush_immediately(raw_event: &str) -> bool {
        raw_event.contains("\"type\":\"tool_call\"")
            || raw_event.contains("\"type\":\"tool_result\"")
            || raw_event.contains("\"type\":\"round_limit_reached\"")
            || raw_event.contains("\"type\":\"context_compacted\"")
            || raw_event.contains("\"type\":\"model_retry\"")
            || raw_event.contains("\"type\":\"model_chain_retry\"")
            || raw_event.contains("\"type\":\"model_fallback\"")
            || raw_event.contains("\"type\":\"profile_rotation\"")
    }

    fn payload_with_seq(&self, raw_event: &str, seq: u64) -> String {
        stream_broadcast::envelope_with_seq(
            raw_event,
            seq,
            self.stream_id.as_deref(),
            self.turn_id.as_deref(),
        )
    }

    fn apply_backpressure<'a>(
        &self,
        mut state: MutexGuard<'a, CoordinatorState>,
    ) -> Result<MutexGuard<'a, CoordinatorState>> {
        loop {
            if let Some(error) = state.fatal_error.as_ref() {
                anyhow::bail!("persistence unavailable: {error}");
            }
            let lag = state
                .oldest_pending
                .map(|at| at.elapsed())
                .unwrap_or_default();
            let hard = state.queued_bytes >= HARD_DIRTY_BYTES || lag >= HARD_LAG;
            if hard {
                let message = format!(
                    "durable stream lag exceeded hard limit (bytes={}, lag_ms={})",
                    state.queued_bytes,
                    lag.as_millis()
                );
                state.fatal_error = Some(message.clone());
                state.status = "persistence_unavailable".to_string();
                self.cancel.store(true, Ordering::SeqCst);
                self.backpressure.notify_all();
                self.durable_notify.notify_waiters();
                anyhow::bail!(message);
            }
            let soft = state.queued_bytes >= SOFT_DIRTY_BYTES || lag >= SOFT_LAG;
            if !soft {
                return Ok(state);
            }
            global_writer_notify().notify_one();
            let (next, _) = self
                .backpressure
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
        }
    }

    fn take_pending_batch(&self) -> Result<Option<PreparedBatch>> {
        let mut state = lock_state(&self.state);
        if state.pending.is_empty() {
            return Ok(None);
        }
        if self.persistent && state.attempt_no == 0 {
            anyhow::bail!("durability attempt not initialized");
        }
        let queued = std::mem::take(&mut state.pending);
        let seq_start = queued.first().map(|event| event.seq).unwrap_or(0);
        let seq_end = queued.last().map(|event| event.seq).unwrap_or(0);
        let journal_events = coalesce_journal_events(&queued)?;
        let batch = JournalBatch {
            run_id: self.run_id.clone(),
            attempt_no: state.attempt_no,
            block_no: state.next_block_no,
            seq_start,
            seq_end,
            events: journal_events.clone(),
        };
        state.next_block_no = state.next_block_no.saturating_add(1);
        Ok(Some(PreparedBatch {
            batch,
            queued,
            journal_events,
        }))
    }

    fn finish_prepared(&self, prepared: PreparedBatch) {
        let released_bytes = prepared
            .queued
            .iter()
            .map(|event| event.bytes)
            .sum::<usize>();
        for event in &prepared.queued {
            self.deliver_durable_event(&event.payload, event.seq);
        }
        {
            let mut state = lock_state(&self.state);
            state.queued_bytes = state.queued_bytes.saturating_sub(released_bytes);
            if state.queued_bytes == 0 {
                state.oldest_pending = None;
            }
            if state.attempt_no == prepared.batch.attempt_no {
                state.durable_events.extend(prepared.journal_events);
            }
        }
        // Publish the in-memory durability barrier only after every event in
        // the committed batch has been delivered. A finalizer waiting in
        // `flush()` therefore cannot overtake the last delta and emit
        // `stream_end` first.
        self.durable_seq
            .store(prepared.batch.seq_end, Ordering::SeqCst);
        self.backpressure.notify_all();
        self.durable_notify.notify_waiters();
    }

    async fn persist_prepared_to_spool(self: &Arc<Self>, prepared: PreparedBatch) {
        let batch = prepared.batch.clone();
        match crate::blocking::run_blocking(move || super::spool::append_batch(&batch)).await {
            Ok(()) => {
                lock_state(&self.state).spool_active = true;
                self.finish_prepared(prepared);
            }
            Err(error) => self.fail_fatal(error.to_string()),
        }
    }

    async fn flush_all_pending() {
        let coordinators = {
            let mut map = registry()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let live = map.values().filter_map(Weak::upgrade).collect::<Vec<_>>();
            map.retain(|_, weak| weak.strong_count() > 0);
            live
        };

        let mut sqlite_groups =
            HashMap::<usize, (Arc<SessionDB>, Vec<(Arc<StreamCoordinator>, PreparedBatch)>)>::new();
        let mut spool_batches = Vec::<(Arc<StreamCoordinator>, PreparedBatch)>::new();
        for coordinator in coordinators {
            let prepared = match coordinator.take_pending_batch() {
                Ok(Some(prepared)) => prepared,
                Ok(None) => continue,
                Err(error) => {
                    coordinator.fail_fatal(error.to_string());
                    continue;
                }
            };
            if !coordinator.persistent || lock_state(&coordinator.state).spool_active {
                spool_batches.push((coordinator, prepared));
                continue;
            }
            // Internal kill switch keeps the append-only protocol and final
            // atomic transaction, but falls back to one SQLite transaction per
            // run if group commit needs to be disabled in a release.
            let key = if group_commit_enabled() {
                Arc::as_ptr(&coordinator.db) as usize
            } else {
                Arc::as_ptr(&coordinator) as usize
            };
            sqlite_groups
                .entry(key)
                .or_insert_with(|| (coordinator.db.clone(), Vec::new()))
                .1
                .push((coordinator, prepared));
        }

        for (_key, (db, group)) in sqlite_groups {
            let batches = group
                .iter()
                .map(|(_, prepared)| prepared.batch.clone())
                .collect::<Vec<_>>();
            let batch_count = batches.len();
            let payload_bytes = group
                .iter()
                .flat_map(|(_, prepared)| prepared.queued.iter())
                .map(|event| event.bytes)
                .sum::<usize>();
            let flush_started = Instant::now();
            let result = db
                .run(move |db| db.append_stream_journal_batches(&batches))
                .await;
            match result {
                Ok(_) => {
                    app_debug!(
                        "chat",
                        "stream_durability",
                        "journal_flush batches={} bytes={} latency_ms={}",
                        batch_count,
                        payload_bytes,
                        flush_started.elapsed().as_millis()
                    );
                    for (coordinator, prepared) in group {
                        coordinator.finish_prepared(prepared);
                    }
                }
                Err(error) => {
                    for (coordinator, prepared) in group {
                        app_warn!(
                            "chat",
                            "stream_spool",
                            "SQLite journal group unavailable for run {}, switching to spool: {}",
                            coordinator.run_id,
                            error
                        );
                        spool_batches.push((coordinator, prepared));
                    }
                }
            }
        }

        for (coordinator, prepared) in spool_batches {
            coordinator.persist_prepared_to_spool(prepared).await;
        }
    }

    fn deliver_durable_event(&self, payload: &str, seq: u64) {
        self.event_sink.send(payload);
        if self.source.broadcasts_to_user_ui() {
            stream_broadcast::broadcast_delta(
                &self.session_id,
                payload,
                seq,
                self.stream_id.as_deref(),
            );
        }
        sink_registry::sink_registry().emit(&self.session_id, payload);
    }

    pub async fn reconcile_spool_to_sqlite(&self) -> Result<()> {
        if !self.persistent || !lock_state(&self.state).spool_active {
            return Ok(());
        }
        let run_id = self.run_id.clone();
        let spool =
            crate::blocking::run_blocking(move || super::spool::read_batches(&run_id)).await?;
        if let Some(error) = spool.integrity_error {
            anyhow::bail!(
                "cannot import damaged emergency spool for run {}: {}",
                self.run_id,
                error
            );
        }
        if !spool.batches.is_empty() {
            let db = self.db.clone();
            let batches = spool.batches;
            db.run(move |db| db.append_stream_journal_batches(&batches))
                .await?;
        }
        let run_id = self.run_id.clone();
        crate::blocking::run_blocking(move || super::spool::remove(&run_id)).await?;
        lock_state(&self.state).spool_active = false;
        Ok(())
    }

    pub fn mark_committed(&self, seq: u64) {
        self.committed_seq.store(seq, Ordering::SeqCst);
        lock_state(&self.state).status = "committed".to_string();
        self.closed.store(true, Ordering::SeqCst);
        global_writer_notify().notify_one();
        Self::unregister(&self.session_id, &self.run_id);
    }

    pub fn mark_interrupted(&self, status: &str) {
        lock_state(&self.state).status = status.to_string();
        self.closed.store(true, Ordering::SeqCst);
        global_writer_notify().notify_one();
        Self::unregister(&self.session_id, &self.run_id);
    }

    fn unregister(session_id: &str, run_id: &str) {
        let mut map = registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remove = map
            .get(session_id)
            .and_then(Weak::upgrade)
            .is_none_or(|coordinator| coordinator.run_id == run_id);
        if remove {
            map.remove(session_id);
        }
    }

    pub fn is_persistent(&self) -> bool {
        self.persistent
    }

    pub fn admitted_stop_epoch(&self) -> u64 {
        self.admitted_stop_epoch
    }

    pub fn stop_admission(&self) -> (u64, u64, u64) {
        (
            self.admitted_stop_epoch,
            self.admitted_global_stop_epoch,
            self.admitted_global_stop_receipt_count,
        )
    }

    pub fn current_provider_shape(&self) -> Option<String> {
        self.provider_shape
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Latest nonterminal main request owned by the current attempt. Final
    /// turn committers use this to bind the assistant/context transaction to
    /// the response-started plan instead of terminalizing it early in the
    /// streaming loop.
    pub(crate) fn active_main_request_plan(&self) -> Option<ActiveRequestPlanSnapshot> {
        let attempt_no = self.current_attempt_no();
        let plans = self
            .request_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        plans
            .plans
            .iter()
            .filter(|(_, plan)| {
                plan.attempt_no == attempt_no
                    && plan.role == DurableRequestRole::MainContinuation
                    && !matches!(
                        plan.state,
                        RuntimeRequestPlanState::Terminal | RuntimeRequestPlanState::Superseded
                    )
            })
            .max_by_key(|(_, plan)| plan.sequence)
            .map(|(request_plan_id, plan)| ActiveRequestPlanSnapshot {
                request_plan_id: request_plan_id.clone(),
                role: plan.role,
                attempt_no: plan.attempt_no,
                state: plan.state,
            })
    }

    /// Bind the live request-plan state to an interrupted turn transaction.
    /// Persistent callers must pass this value into `CommitInterruptedTurn`;
    /// the DB re-validates every identity/state field under its transaction.
    pub fn interrupted_request_plan_commit(
        &self,
        response_outcome: crate::session::RequestPlanResponseOutcome,
    ) -> crate::session::RequestPlanCommit {
        if !self.persistent {
            return crate::session::RequestPlanCommit::None;
        }
        let Some(plan) = self.active_main_request_plan() else {
            return crate::session::RequestPlanCommit::None;
        };
        crate::session::RequestPlanCommit::ConvergeInterrupted {
            request_plan_id: plan.request_plan_id,
            attempt_no: plan.attempt_no,
            expected_state: plan.state.into(),
            response_outcome,
        }
    }

    /// Bind the final assistant/context commit to its response-started plan.
    /// The DB refuses success for an unsent, ambiguous, or unrelated request.
    pub fn successful_request_plan_commit(&self) -> Result<crate::session::RequestPlanCommit> {
        let plan = self.active_main_request_plan().ok_or_else(|| {
            anyhow::anyhow!("successful Provider turn has no active main request plan")
        })?;
        if plan.state != RuntimeRequestPlanState::ResponseStarted {
            anyhow::bail!(
                "successful Provider turn requires response-start proof; found {:?}",
                plan.state
            );
        }
        if !self.persistent {
            return Ok(crate::session::RequestPlanCommit::None);
        }
        Ok(crate::session::RequestPlanCommit::CompleteResponseStarted {
            request_plan_id: plan.request_plan_id,
            attempt_no: plan.attempt_no,
        })
    }

    /// Synchronize the coordinator after the assistant/request-plan
    /// transaction wins. Persistent payload cleanup is best-effort because
    /// the durable scrub-pending claim was already part of that transaction;
    /// startup reconciliation remains its crash-safe backstop.
    pub async fn finalize_successful_request_after_turn_commit(&self) -> Result<()> {
        let Some(plan) = self.active_main_request_plan() else {
            return Ok(());
        };
        if plan.state != RuntimeRequestPlanState::ResponseStarted {
            anyhow::bail!(
                "successful turn requires response-start proof; found {:?}",
                plan.state
            );
        }
        if !self.persistent {
            return self
                .mark_request_terminal(&plan.request_plan_id, RequestTerminalOutcome::Success)
                .await;
        }
        let payload_id = {
            let mut plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let runtime = plans
                .plans
                .get_mut(&plan.request_plan_id)
                .ok_or_else(|| anyhow::anyhow!("request plan disappeared after turn commit"))?;
            if runtime.state != RuntimeRequestPlanState::ResponseStarted {
                anyhow::bail!("request plan changed while finalizing successful turn");
            }
            runtime.state = RuntimeRequestPlanState::Terminal;
            runtime.exact_payload_id.clone()
        };
        if let Some(payload_id) = payload_id {
            let db = self.db.clone();
            let owner_id = plan.request_plan_id;
            if let Err(error) = db
                .run(move |db| {
                    db.request_exact_payload_scrub(
                        &owner_id,
                        &payload_id,
                        crate::session::request_payload_store::ExactPayloadScrubReason::RequestTerminal,
                    )?;
                    db.scrub_exact_request_payload(&owner_id, &payload_id)
                })
                .await
            {
                app_warn!(
                    "request_payload",
                    "terminal_cleanup",
                    "deferred exact request payload cleanup after successful turn: {error:#}"
                );
            }
        }
        Ok(())
    }

    /// Synchronize the coordinator after an interrupted turn transaction.
    /// Ambiguous sends remain held as `SendUnknown`; they are never replayed.
    pub async fn finalize_interrupted_request_after_turn_commit(
        &self,
        response_outcome: crate::session::RequestPlanResponseOutcome,
    ) -> Result<()> {
        let Some(plan) = self.active_main_request_plan() else {
            return Ok(());
        };
        if self.persistent {
            enum PayloadAction {
                Scrub(
                    String,
                    crate::session::request_payload_store::ExactPayloadScrubReason,
                ),
                Hold(String),
                None,
            }
            let action = {
                let mut plans = self
                    .request_plans
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let runtime = plans
                    .plans
                    .get_mut(&plan.request_plan_id)
                    .ok_or_else(|| anyhow::anyhow!("request plan disappeared after turn commit"))?;
                match runtime.state {
                    RuntimeRequestPlanState::ContextCommitted => {
                        runtime.state = RuntimeRequestPlanState::Superseded;
                        runtime.exact_payload_id.clone().map_or(PayloadAction::None, |id| {
                            PayloadAction::Scrub(
                                id,
                                crate::session::request_payload_store::ExactPayloadScrubReason::RequestSuperseded,
                            )
                        })
                    }
                    RuntimeRequestPlanState::Dispatching => {
                        runtime.state = RuntimeRequestPlanState::SendUnknown;
                        runtime
                            .exact_payload_id
                            .clone()
                            .map_or(PayloadAction::None, PayloadAction::Hold)
                    }
                    RuntimeRequestPlanState::ResponseStarted => {
                        runtime.state = RuntimeRequestPlanState::Terminal;
                        runtime.exact_payload_id.clone().map_or(PayloadAction::None, |id| {
                            PayloadAction::Scrub(
                                id,
                                crate::session::request_payload_store::ExactPayloadScrubReason::RequestTerminal,
                            )
                        })
                    }
                    RuntimeRequestPlanState::SendUnknown => PayloadAction::None,
                    RuntimeRequestPlanState::Terminal | RuntimeRequestPlanState::Superseded => {
                        PayloadAction::None
                    }
                }
            };
            let db = self.db.clone();
            let owner_id = plan.request_plan_id;
            let cleanup = match action {
                PayloadAction::Scrub(payload_id, reason) => db
                    .run(move |db| {
                        db.request_exact_payload_scrub(&owner_id, &payload_id, reason)?;
                        db.scrub_exact_request_payload(&owner_id, &payload_id)
                    })
                    .await
                    .map(|_| ()),
                PayloadAction::Hold(payload_id) => db
                    .run(move |db| db.hold_exact_payload_for_send_unknown(&owner_id, &payload_id))
                    .await
                    .map(|_| ()),
                PayloadAction::None => Ok(()),
            };
            if let Err(error) = cleanup {
                app_warn!(
                    "request_payload",
                    "terminal_cleanup",
                    "deferred exact request payload cleanup after interrupted turn: {error:#}"
                );
            }
            return Ok(());
        }
        match plan.state {
            RuntimeRequestPlanState::ContextCommitted => {
                self.supersede_request_plan(&plan.request_plan_id).await
            }
            RuntimeRequestPlanState::Dispatching => {
                self.mark_request_send_unknown(
                    &plan.request_plan_id,
                    Some("incognito_turn_ended_after_dispatch_claim"),
                )
                .await
            }
            RuntimeRequestPlanState::ResponseStarted => {
                let outcome = match response_outcome {
                    crate::session::RequestPlanResponseOutcome::CancelledAfterResponse => {
                        RequestTerminalOutcome::CancelledAfterResponse
                    }
                    crate::session::RequestPlanResponseOutcome::ResponseIncomplete => {
                        RequestTerminalOutcome::ResponseIncomplete
                    }
                };
                self.mark_request_terminal(&plan.request_plan_id, outcome)
                    .await
            }
            RuntimeRequestPlanState::SendUnknown => Ok(()),
            RuntimeRequestPlanState::Terminal | RuntimeRequestPlanState::Superseded => Ok(()),
        }
    }

    pub fn trailing_text(&self) -> String {
        let state = lock_state(&self.state);
        let mut text = String::new();
        for journal_event in &state.durable_events {
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&journal_event.event) else {
                continue;
            };
            match event.get("type").and_then(|value| value.as_str()) {
                Some("tool_call" | "thinking_delta") => text.clear(),
                Some("text_delta") => {
                    if let Some(content) = event.get("content").and_then(|value| value.as_str()) {
                        text.push_str(content);
                    }
                }
                _ => {}
            }
        }
        text
    }

    pub fn usage(&self) -> CapturedUsage {
        self.captured_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[doc(hidden)]
    pub fn had_thinking(&self) -> bool {
        self.had_thinking.load(Ordering::SeqCst)
    }

    pub fn had_text_output(&self) -> bool {
        self.had_text.load(Ordering::SeqCst)
    }

    /// Emergency compaction establishes a new stable retry base. Ordinary
    /// round checkpoints deliberately do not call this: model/profile
    /// failover must still discard a failed attempt's provider-native tail.
    #[doc(hidden)]
    pub fn adopt_attempt_base_context(&self, history: &[serde_json::Value]) -> Result<()> {
        let json = serde_json::to_string(history)?;
        *self
            .attempt_base_context_json
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(json);
        self.attempt_base_contains_current_user
            .store(true, Ordering::Release);
        Ok(())
    }

    /// Whether the currently adopted retry base already contains this turn's
    /// canonical user item. This state travels with the base so failover never
    /// re-appends the user after Tier 3 or Tier 4 has published a post-user
    /// checkpoint.
    pub fn attempt_base_contains_current_user(&self) -> bool {
        self.attempt_base_contains_current_user
            .load(Ordering::Acquire)
    }
}

#[async_trait]
impl TurnDurabilitySink for StreamCoordinator {
    fn accept_event(&self, raw_event: &str) -> Result<u64> {
        if self.closed.load(Ordering::SeqCst) {
            anyhow::bail!("stream durability sink is closed");
        }
        let parsed = serde_json::from_str::<serde_json::Value>(raw_event)
            .map_err(|error| anyhow::anyhow!("invalid stream event: {error}"))?;
        let event_type = parsed.get("type").and_then(|value| value.as_str());
        match event_type {
            Some("usage") => self
                .captured_usage
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .absorb_event(&parsed),
            Some("thinking_delta") => self.had_thinking.store(true, Ordering::SeqCst),
            Some("text_delta") => self.had_text.store(true, Ordering::SeqCst),
            Some("tool_call" | "tool_result") => {
                self.had_tool_activity.store(true, Ordering::SeqCst);
                if parsed.get("replay_safe").and_then(|value| value.as_bool()) != Some(true) {
                    // Events produced before replay-safety metadata existed
                    // remain conservative. Only explicitly read-only/
                    // concurrent-safe tool work may cross a model boundary.
                    self.had_non_replayable_tool_activity
                        .store(true, Ordering::SeqCst);
                }
            }
            _ => {}
        }
        let current_role = match event_type {
            Some("text_delta") => Some(MergeRole::Text),
            Some("thinking_delta") => Some(MergeRole::Thinking),
            _ => None,
        };
        let mut state = self.apply_backpressure(lock_state(&self.state))?;
        if self.persistent && state.attempt_no == 0 {
            anyhow::bail!("stream attempt has not begun");
        }
        // The state lock serializes allocation so concurrent tool callbacks
        // cannot observe the same next value. Store accepted only after the
        // event has actually entered the queue, avoiding crash-replay gaps.
        let seq = self.accepted_seq.load(Ordering::SeqCst).saturating_add(1);
        let payload = self.payload_with_seq(raw_event, seq);
        let bytes = payload.len();
        if state.pending.is_empty() && state.queued_bytes == 0 {
            state.oldest_pending = Some(Instant::now());
        }
        state.pending.push(QueuedEvent {
            seq,
            payload,
            bytes,
            merge_role: current_role,
        });
        state.queued_bytes = state.queued_bytes.saturating_add(bytes);
        self.accepted_seq.store(seq, Ordering::SeqCst);

        let role_switch = current_role.is_some_and(|role| {
            state
                .last_merge_role
                .replace(role)
                .is_some_and(|previous| previous != role)
        });
        if matches!(event_type, Some("tool_call" | "tool_result")) {
            state.last_merge_role = None;
        }

        if !self.persistent {
            let queued = state.pending.pop().expect("just pushed");
            state.queued_bytes = state.queued_bytes.saturating_sub(queued.bytes);
            state.oldest_pending = None;
            state
                .durable_events
                .push(JournalEvent::single(seq, queued.payload.clone()));
            drop(state);
            self.durable_seq.store(seq, Ordering::SeqCst);
            self.deliver_durable_event(&queued.payload, seq);
            self.durable_notify.notify_waiters();
            return Ok(seq);
        }

        let immediate = state.queued_bytes >= FLUSH_BYTES
            || role_switch
            || Self::should_flush_immediately(raw_event);
        drop(state);
        if immediate {
            global_writer_notify().notify_one();
        }
        Ok(seq)
    }

    async fn flush(&self, _reason: FlushReason) -> Result<u64> {
        let target = self.accepted_seq.load(Ordering::SeqCst);
        if self.durable_seq.load(Ordering::SeqCst) >= target {
            return Ok(target);
        }
        global_writer_notify().notify_one();
        let wait = async {
            loop {
                // Arm the waiter BEFORE reading the watermark. `durable_notify` is
                // only ever signalled with `notify_waiters()`, which stores no
                // permit, so a publish landing between the read and the arming is
                // lost outright — and nothing re-fires it: once the queue drains,
                // `take_pending_batch` returns None and `finish_prepared` is never
                // reached again. The waiter would then burn the full `HARD_LAG`
                // and `fail_fatal`, skipping the terminal commit that materializes
                // the turn. Same register-then-recheck shape as `tools::job_status`.
                let notified = self.durable_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                if let Some(error) = lock_state(&self.state).fatal_error.clone() {
                    anyhow::bail!("persistence unavailable: {error}");
                }
                let durable = self.durable_seq.load(Ordering::SeqCst);
                if durable >= target {
                    return Ok(durable);
                }

                // Re-poll on a bounded interval so any wakeup we still manage to
                // miss costs latency rather than the whole turn.
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(FLUSH_INTERVAL) => {}
                }
            }
        };
        match tokio::time::timeout(HARD_LAG, wait).await {
            Ok(result) => result,
            Err(_) => {
                self.fail_fatal("durability flush timed out".to_string());
                anyhow::bail!("durability flush timed out")
            }
        }
    }

    async fn checkpoint_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64> {
        let through_seq = self.flush(FlushReason::RoundEnd).await?;
        if !self.persistent {
            // Incognito checkpoints are intentionally memory-only. Keep the
            // DB revision unchanged so the one final in-session materialization
            // can still CAS against the revision captured at turn start.
            return Ok(expected_revision);
        }
        self.reconcile_spool_to_sqlite().await?;
        let context_json = serde_json::to_string(history)?;
        let run_id = self.run_id.clone();
        let attempt_no = self.current_attempt_no();
        let db = self.db.clone();
        let revision = db
            .run(move |db| {
                db.checkpoint_stream_context(
                    &run_id,
                    attempt_no,
                    expected_revision,
                    &context_json,
                    through_seq,
                    crate::session::Tier3RecoveryCommit::Unchanged,
                )
            })
            .await?;
        self.context_revision.store(revision, Ordering::SeqCst);
        Ok(revision)
    }

    async fn checkpoint_emergency_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64> {
        let through_seq = self.flush(FlushReason::RoundEnd).await?;
        if !self.persistent {
            crate::session::require_incognito_tier3_recovery(&self.session_id);
            return Ok(expected_revision);
        }
        self.reconcile_spool_to_sqlite().await?;
        let context_json = serde_json::to_string(history)?;
        let run_id = self.run_id.clone();
        let attempt_no = self.current_attempt_no();
        let db = self.db.clone();
        let revision = db
            .run(move |db| {
                db.checkpoint_stream_context(
                    &run_id,
                    attempt_no,
                    expected_revision,
                    &context_json,
                    through_seq,
                    crate::session::Tier3RecoveryCommit::RequireAfterEmergency,
                )
            })
            .await?;
        self.context_revision.store(revision, Ordering::SeqCst);
        Ok(revision)
    }

    async fn checkpoint_summarized_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64> {
        let through_seq = self.flush(FlushReason::RoundEnd).await?;
        let context_json = serde_json::to_string(history)?;
        if !self.persistent {
            // Incognito has no durable row; publication is the in-memory
            // history assignment owned by the agent. Still promote the exact
            // summary to the failover base before clearing the recovery mark.
            *self
                .attempt_base_context_json
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(context_json);
            self.attempt_base_contains_current_user
                .store(true, Ordering::Release);
            crate::session::clear_incognito_tier3_recovery(&self.session_id);
            return Ok(expected_revision);
        }
        self.reconcile_spool_to_sqlite().await?;
        let published_context_json = context_json.clone();
        let run_id = self.run_id.clone();
        let attempt_no = self.current_attempt_no();
        let db = self.db.clone();
        let revision = db
            .run(move |db| {
                db.checkpoint_stream_context(
                    &run_id,
                    attempt_no,
                    expected_revision,
                    &context_json,
                    through_seq,
                    crate::session::Tier3RecoveryCommit::ClearAfterSummary,
                )
            })
            .await?;
        self.context_revision.store(revision, Ordering::SeqCst);
        // Only after the durable summary+marker transaction wins may a later
        // provider/profile failover roll back to this new canonical boundary.
        // Ordinary round checkpoints intentionally never update this base.
        *self
            .attempt_base_context_json
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(published_context_json);
        self.attempt_base_contains_current_user
            .store(true, Ordering::Release);
        Ok(revision)
    }

    async fn prepare_request_plan(
        &self,
        input: &PrepareRequestPlan,
        exact_body: Arc<[u8]>,
    ) -> Result<()> {
        verify_exact_body(input, exact_body.as_ref())?;
        let attempt_no = self.current_attempt_no();
        if attempt_no == 0 {
            anyhow::bail!("cannot prepare a Provider request before the stream attempt begins");
        }
        {
            let plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if plans.plans.contains_key(&input.request_plan_id) {
                anyhow::bail!("request plan already exists: {}", input.request_plan_id);
            }
            if plans.plans.values().any(|plan| {
                plan.attempt_no == attempt_no
                    && plan.role == DurableRequestRole::MainContinuation
                    && input.role == DurableRequestRole::MainContinuation
                    && !matches!(
                        plan.state,
                        RuntimeRequestPlanState::Terminal | RuntimeRequestPlanState::Superseded
                    )
            }) {
                anyhow::bail!("another main Provider request plan is still active");
            }
        }

        let (sequence, exact_payload_id) = if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let run_id = self.run_id.clone();
            let input = input.clone();
            let body = exact_body;
            db.run(move |db| {
                use crate::session::context_projection::{
                    ExactPayloadAvailability as PlanPayloadAvailability, NewProjectionEpoch,
                    NewProjectionItem, NewRequestProjectionPlan, ProjectionEpochScope,
                    ProjectionReplayability, ProjectionTrigger,
                };
                use crate::session::request_payload_store::{
                    ExactPayloadAvailability as StorePayloadAvailability, StageExactPayloadOutcome,
                };

                let version = db.get_request_projection_version(&session_id)?;
                let staged = db.stage_exact_request_payload(
                    &session_id,
                    &input.request_plan_id,
                    body.as_ref(),
                    None,
                )?;
                let (
                    payload_availability,
                    exact_payload_id,
                    exact_payload_reservation_id,
                    exact_payload_keyed_digest,
                    exact_payload_storage_kind,
                    exact_payload_stored_bytes,
                ) = match staged {
                    StageExactPayloadOutcome::Stored(hold) => {
                        debug_assert_eq!(hold.availability, StorePayloadAvailability::Stored);
                        (
                            PlanPayloadAvailability::Stored,
                            Some(hold.payload_id),
                            Some(hold.reservation_id),
                            Some(hold.keyed_digest),
                            Some(hold.storage_kind.as_str().to_string()),
                            Some(hold.plaintext_bytes),
                        )
                    }
                    StageExactPayloadOutcome::Unavailable(_) => (
                        PlanPayloadAvailability::Unavailable,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ),
                    StageExactPayloadOutcome::Lost(_) => {
                        (PlanPayloadAvailability::Lost, None, None, None, None, None)
                    }
                };

                let projection_json = serde_json::to_vec(&input.projection)?;
                let projection_bytes = u64::try_from(projection_json.len())
                    .map_err(|_| anyhow::anyhow!("projection manifest exceeds u64"))?;
                let policy_fingerprint = crate::cache_routing::audit_fingerprint(
                    "request-projection-policy-v1",
                    b"request-projection-policy-v1",
                );
                let action_digest = crate::cache_routing::audit_fingerprint(
                    "request-projection-actions-v1",
                    &projection_json,
                );
                let projection_epoch_id =
                    (!input.projection.is_empty()).then(|| uuid::Uuid::new_v4().to_string());
                let plan = NewRequestProjectionPlan {
                    request_plan_id: input.request_plan_id.clone(),
                    session_id: session_id.clone(),
                    // The current conversation branch is represented by its
                    // session row. Forked sessions receive independent ids.
                    branch_id: session_id.clone(),
                    run_id: run_id.clone(),
                    attempt_no,
                    request_role: request_projection_role(input.role),
                    expected_canonical_generation: version.canonical_generation,
                    expected_context_revision: version.context_revision,
                    projection_epoch_id: projection_epoch_id.clone(),
                    cache_identity_hash: input.cache_identity_hash.clone(),
                    provider_id: input.provider_id.clone(),
                    provider_profile_id: input.provider_profile_id.clone(),
                    model_id: input.model_id.clone(),
                    request_shape: input.request_shape.clone(),
                    writer_version: 1,
                    renderer_version: 1,
                    policy_fingerprint: policy_fingerprint.clone(),
                    counter_profile: "final-capacity-count-v1".to_string(),
                    exact_payload_id: exact_payload_id.clone(),
                    exact_payload_reservation_id,
                    exact_payload_keyed_digest,
                    exact_payload_storage_kind,
                    exact_payload_stored_bytes,
                    payload_availability,
                    projection_bytes,
                    expires_at: None,
                    final_capacity_count_json: input.final_capacity_count_json.clone(),
                    prepared_body_fingerprint: input.body_keyed_fingerprint.clone(),
                    prepared_body_bytes: input.body_len,
                    endpoint_kind: input.endpoint_kind.clone(),
                    content_type: input.content_type.clone(),
                };

                let record = if let Some(epoch_id) = projection_epoch_id {
                    let items = input
                        .projection
                        .iter()
                        .map(|item| {
                            Ok(NewProjectionItem {
                                projection_item_key: item.projection_item_key.clone(),
                                result_id: item.result_id.clone(),
                                stable_ordinal: item.stable_ordinal,
                                action: projection_action(&item.action)?,
                                source_guard: item.source_guard.clone(),
                                replacement_fingerprint: item.replacement_fingerprint.clone(),
                                replayability: if item.result_id.is_some() {
                                    ProjectionReplayability::ManagedResult
                                } else {
                                    ProjectionReplayability::ExactPlanOnly
                                },
                                renderer_profile: "request-projection-v1".to_string(),
                                target_variant: item.action.clone(),
                                source_plan_id: item
                                    .result_id
                                    .is_none()
                                    .then(|| input.request_plan_id.clone()),
                                source_projection_item_key: None,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let earliest_changed_item_key = input
                        .projection
                        .iter()
                        .min_by_key(|item| item.stable_ordinal)
                        .map(|item| item.projection_item_key.clone());
                    let max_tier = if input
                        .projection
                        .iter()
                        .any(|item| item.action.starts_with("tier2_"))
                    {
                        2
                    } else {
                        0
                    };
                    let epoch = NewProjectionEpoch {
                        epoch_id,
                        session_id: session_id.clone(),
                        branch_id: session_id.clone(),
                        scope: ProjectionEpochScope::RequestLocal,
                        owner_request_plan_id: Some(input.request_plan_id.clone()),
                        cache_identity_hash: input.cache_identity_hash.clone(),
                        parent_epoch_id: None,
                        canonical_generation: version.canonical_generation,
                        created_at_revision: version.context_revision,
                        provider_request_shape: input.request_shape.clone(),
                        policy_fingerprint,
                        renderer_version: 1,
                        counter_profile: "final-capacity-count-v1".to_string(),
                        trigger: if input.round == 0 {
                            ProjectionTrigger::TurnStart
                        } else {
                            ProjectionTrigger::ToolLoop
                        },
                        max_tier,
                        earliest_changed_item_key,
                        action_digest,
                    };
                    db.create_context_committed_request_local_projection_plan_with_followup(
                        &epoch,
                        &items,
                        &plan,
                        input.tier3_followup_after_capacity_projection,
                    )?
                } else {
                    db.create_context_committed_request_projection_plan_with_followup(
                        &plan,
                        input.tier3_followup_after_capacity_projection,
                    )?
                };
                Ok::<(u64, Option<String>), anyhow::Error>((
                    record.request_sequence,
                    exact_payload_id,
                ))
            })
            .await?
        } else {
            let staged = self
                .incognito_payloads
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .stage(
                    &self.session_id,
                    &input.request_plan_id,
                    exact_body.as_ref(),
                )?;
            let payload_id = match staged {
                crate::session::request_payload_store::StageExactPayloadOutcome::Stored(hold) => {
                    Some(hold.payload_id)
                }
                crate::session::request_payload_store::StageExactPayloadOutcome::Unavailable(_)
                | crate::session::request_payload_store::StageExactPayloadOutcome::Lost(_) => None,
            };
            let plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (plans.next_sequence.saturating_add(1), payload_id)
        };

        let mut plans = self
            .request_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if plans.plans.contains_key(&input.request_plan_id) {
            anyhow::bail!("request plan was concurrently prepared");
        }
        plans.next_sequence = plans.next_sequence.max(sequence);
        plans.plans.insert(
            input.request_plan_id.clone(),
            RuntimeRequestPlan {
                role: input.role,
                attempt_no,
                sequence,
                state: RuntimeRequestPlanState::ContextCommitted,
                body_keyed_fingerprint: input.body_keyed_fingerprint.clone(),
                body_len: input.body_len,
                endpoint_kind: input.endpoint_kind.clone(),
                content_type: input.content_type.clone(),
                exact_payload_id,
            },
        );
        if !self.persistent && input.tier3_followup_after_capacity_projection {
            crate::session::require_incognito_tier3_after_capacity_projection(
                &self.session_id,
                &input.request_plan_id,
            );
        }
        Ok(())
    }

    async fn claim_request_dispatch(
        &self,
        request_plan_id: &str,
        claim: &DispatchClaim,
    ) -> Result<()> {
        let body = crate::session::context_projection::PreparedRequestBodyIdentity {
            fingerprint: claim.body_keyed_fingerprint.clone(),
            bytes: claim.body_len,
            endpoint_kind: claim.endpoint_kind.clone(),
            content_type: claim.content_type.clone(),
        };
        if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let request_plan_id = request_plan_id.to_string();
            let claim = claim.clone();
            let changed = db
                .run(move |db| {
                    db.claim_request_dispatch(
                        &session_id,
                        &request_plan_id,
                        &claim.request_attempt_id,
                        claim.provider_idempotency_key.as_deref(),
                        &body,
                    )
                })
                .await?;
            if !changed {
                anyhow::bail!("request dispatch claim lost its state/version/body CAS");
            }
        }
        let mut plans = self
            .request_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let plan = plans
            .plans
            .get_mut(request_plan_id)
            .ok_or_else(|| anyhow::anyhow!("unknown request plan: {request_plan_id}"))?;
        if plan.state != RuntimeRequestPlanState::ContextCommitted
            || plan.body_keyed_fingerprint != claim.body_keyed_fingerprint
            || plan.body_len != claim.body_len
            || plan.endpoint_kind != claim.endpoint_kind
            || plan.content_type != claim.content_type
        {
            anyhow::bail!("request dispatch claim does not match the prepared body/state");
        }
        plan.state = RuntimeRequestPlanState::Dispatching;
        Ok(())
    }

    async fn mark_request_response_started(
        &self,
        request_plan_id: &str,
        response: &ResponseStarted,
    ) -> Result<()> {
        if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let request_plan_id = request_plan_id.to_string();
            let provider_request_id = response.provider_request_id.clone();
            let provider_attempt = response.provider_attempt;
            let status = response.status;
            let changed = db
                .run(move |db| {
                    db.mark_request_response_started(
                        &session_id,
                        &request_plan_id,
                        provider_attempt,
                        status,
                        provider_request_id.as_deref(),
                    )
                })
                .await?;
            if !changed {
                anyhow::bail!("request response-start transition lost its dispatch CAS");
            }
        }
        let mut plans = self
            .request_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let plan = plans
            .plans
            .get_mut(request_plan_id)
            .ok_or_else(|| anyhow::anyhow!("unknown request plan: {request_plan_id}"))?;
        if plan.state != RuntimeRequestPlanState::Dispatching {
            anyhow::bail!("request response started from an invalid state");
        }
        plan.state = RuntimeRequestPlanState::ResponseStarted;
        Ok(())
    }

    async fn mark_request_send_unknown(
        &self,
        request_plan_id: &str,
        diagnostic: Option<&str>,
    ) -> Result<()> {
        let reason = diagnostic.unwrap_or("transport_interrupted_before_response_proof");
        if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let request_plan_id = request_plan_id.to_string();
            let reason = reason.to_string();
            let changed = db
                .run(move |db| db.mark_request_send_unknown(&session_id, &request_plan_id, &reason))
                .await?;
            if !changed {
                anyhow::bail!("request send-unknown transition lost its dispatch CAS");
            }
        }
        let payload_id = {
            let mut plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let plan = plans
                .plans
                .get_mut(request_plan_id)
                .ok_or_else(|| anyhow::anyhow!("unknown request plan: {request_plan_id}"))?;
            if plan.state != RuntimeRequestPlanState::Dispatching {
                anyhow::bail!("request send became unknown from an invalid state");
            }
            plan.state = RuntimeRequestPlanState::SendUnknown;
            plan.exact_payload_id.clone()
        };
        if let Some(payload_id) = payload_id {
            if !self.persistent {
                let held = self
                    .incognito_payloads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .hold_send_unknown(request_plan_id, &payload_id);
                if !held {
                    anyhow::bail!("incognito send-unknown payload hold was lost");
                }
            }
        }
        Ok(())
    }

    async fn mark_request_terminal(
        &self,
        request_plan_id: &str,
        outcome: RequestTerminalOutcome,
    ) -> Result<()> {
        if outcome == RequestTerminalOutcome::CancelledBeforeSend {
            anyhow::bail!("pre-send cancellation must supersede the unsent request plan");
        }
        let outcome_label = match outcome {
            RequestTerminalOutcome::Success => "success",
            RequestTerminalOutcome::ProviderRejected => "provider_rejected",
            RequestTerminalOutcome::CancelledBeforeSend => unreachable!(),
            RequestTerminalOutcome::CancelledAfterResponse => "cancelled_after_response",
            RequestTerminalOutcome::ResponseIncomplete => "response_incomplete",
        };
        if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let request_plan_id = request_plan_id.to_string();
            let changed = db
                .run(move |db| {
                    db.complete_response_started_request(
                        &session_id,
                        &request_plan_id,
                        outcome_label,
                    )
                })
                .await?;
            if !changed {
                anyhow::bail!("request terminal transition requires response-start proof");
            }
        }
        let payload_id = {
            let mut plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let plan = plans
                .plans
                .get_mut(request_plan_id)
                .ok_or_else(|| anyhow::anyhow!("unknown request plan: {request_plan_id}"))?;
            if plan.state != RuntimeRequestPlanState::ResponseStarted {
                anyhow::bail!("request terminal transition requires response-start proof");
            }
            plan.state = RuntimeRequestPlanState::Terminal;
            plan.exact_payload_id.clone()
        };
        if let Some(payload_id) = payload_id {
            if self.persistent {
                let db = self.db.clone();
                let owner_id = request_plan_id.to_string();
                db.run(move |db| {
                    db.request_exact_payload_scrub(
                        &owner_id,
                        &payload_id,
                        crate::session::request_payload_store::ExactPayloadScrubReason::RequestTerminal,
                    )?;
                    db.scrub_exact_request_payload(&owner_id, &payload_id)
                })
                .await?;
            } else {
                let _ = self
                    .incognito_payloads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .scrub(request_plan_id, &payload_id, false);
            }
        }
        Ok(())
    }

    async fn supersede_request_plan(&self, request_plan_id: &str) -> Result<()> {
        if self.persistent {
            let db = self.db.clone();
            let session_id = self.session_id.clone();
            let request_plan_id = request_plan_id.to_string();
            let changed = db
                .run(move |db| {
                    db.supersede_unsent_request(
                        &session_id,
                        &request_plan_id,
                        "superseded_before_dispatch",
                    )
                })
                .await?;
            if !changed {
                anyhow::bail!("request plan is not provably unsent");
            }
        }
        let payload_id = {
            let mut plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let plan = plans
                .plans
                .get_mut(request_plan_id)
                .ok_or_else(|| anyhow::anyhow!("unknown request plan: {request_plan_id}"))?;
            if !matches!(plan.state, RuntimeRequestPlanState::ContextCommitted) {
                anyhow::bail!("request plan is not provably unsent");
            }
            plan.state = RuntimeRequestPlanState::Superseded;
            plan.exact_payload_id.clone()
        };
        if !self.persistent {
            crate::session::clear_incognito_capacity_projection_recovery(
                &self.session_id,
                request_plan_id,
            );
        }
        if let Some(payload_id) = payload_id {
            if self.persistent {
                let db = self.db.clone();
                let owner_id = request_plan_id.to_string();
                db.run(move |db| {
                    db.request_exact_payload_scrub(
                        &owner_id,
                        &payload_id,
                        crate::session::request_payload_store::ExactPayloadScrubReason::RequestSuperseded,
                    )?;
                    db.scrub_exact_request_payload(&owner_id, &payload_id)
                })
                .await?;
            } else {
                let _ = self
                    .incognito_payloads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .scrub(request_plan_id, &payload_id, false);
            }
        }
        Ok(())
    }

    async fn supersede_attempt(&self, error: Option<&str>) -> Result<()> {
        self.flush(FlushReason::RoundEnd).await?;
        let attempt_no = self.attempt_no.load(Ordering::SeqCst);
        if !self.persistent {
            let plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if plans
                .plans
                .values()
                .any(|plan| plan.attempt_no == attempt_no && plan.state.possibly_sent())
            {
                anyhow::bail!(
                    "cannot supersede stream attempt {attempt_no}: a Provider request may have been sent"
                );
            }
        }
        if self.persistent {
            self.reconcile_spool_to_sqlite().await?;
            let db = self.db.clone();
            let run_id = self.run_id.clone();
            let expected_revision = self.context_revision();
            let base_context_json = self
                .attempt_base_context_json
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let error = error.map(ToOwned::to_owned);
            let revision = db
                .run(move |db| {
                    db.supersede_stream_attempt(
                        &run_id,
                        attempt_no,
                        expected_revision,
                        base_context_json.as_deref(),
                        error.as_deref(),
                    )
                })
                .await?;
            self.context_revision.store(revision, Ordering::SeqCst);
        }
        let payloads = {
            let mut plans = self
                .request_plans
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut payloads = Vec::new();
            for (request_plan_id, plan) in &mut plans.plans {
                if plan.attempt_no != attempt_no {
                    continue;
                }
                if matches!(plan.state, RuntimeRequestPlanState::ContextCommitted) {
                    plan.state = RuntimeRequestPlanState::Superseded;
                    if let Some(payload_id) = plan.exact_payload_id.clone() {
                        payloads.push((request_plan_id.clone(), payload_id));
                    }
                }
            }
            payloads
        };
        for (request_plan_id, payload_id) in payloads {
            if self.persistent {
                let db = self.db.clone();
                db.run(move |db| {
                    db.request_exact_payload_scrub(
                        &request_plan_id,
                        &payload_id,
                        crate::session::request_payload_store::ExactPayloadScrubReason::RequestSuperseded,
                    )?;
                    db.scrub_exact_request_payload(&request_plan_id, &payload_id)
                })
                .await?;
            } else {
                let _ = self
                    .incognito_payloads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .scrub(&request_plan_id, &payload_id, false);
            }
        }
        lock_state(&self.state).durable_events.clear();
        Ok(())
    }

    async fn begin_attempt(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
        provider_shape: Option<&str>,
    ) -> Result<u32> {
        let supersedes_previous = self.attempt_no.load(Ordering::SeqCst) > 0;
        if supersedes_previous {
            self.supersede_attempt(None).await?;
        }
        let attempt_no = self.attempt_no.load(Ordering::SeqCst).saturating_add(1);
        if self.persistent {
            let db = self.db.clone();
            let run_id = self.run_id.clone();
            let provider_id = provider_id.map(ToOwned::to_owned);
            let model_id = model_id.map(ToOwned::to_owned);
            let provider_shape = provider_shape.map(ToOwned::to_owned);
            db.run(move |db| {
                db.begin_stream_attempt(
                    &run_id,
                    attempt_no,
                    provider_id.as_deref(),
                    model_id.as_deref(),
                    provider_shape.as_deref(),
                )
            })
            .await?;
        }
        self.attempt_no.store(attempt_no, Ordering::SeqCst);
        *self
            .provider_shape
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            provider_shape.map(ToOwned::to_owned);
        let mut state = lock_state(&self.state);
        state.attempt_no = attempt_no;
        state.next_block_no = 1;
        state.durable_events.clear();
        state.last_merge_role = None;
        drop(state);
        *self
            .captured_usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = CapturedUsage::default();
        self.had_thinking.store(false, Ordering::SeqCst);
        self.had_text.store(false, Ordering::SeqCst);
        if supersedes_previous {
            // The previous attempt may already have been rendered because it
            // was itself durable. This ordered marker tells every live client
            // to discard that superseded tail before applying the new attempt.
            let marker = serde_json::json!({
                "type": "stream_attempt_started",
                "attempt_no": attempt_no,
                "reset_superseded": true,
            })
            .to_string();
            self.accept_event(&marker)?;
        }
        Ok(attempt_no)
    }

    fn persistence_run_id(&self) -> &str {
        &self.run_id
    }

    fn current_attempt_no(&self) -> u32 {
        self.attempt_no.load(Ordering::SeqCst)
    }

    fn context_revision(&self) -> i64 {
        self.context_revision.load(Ordering::SeqCst)
    }

    fn had_tool_activity(&self) -> bool {
        self.had_tool_activity.load(Ordering::SeqCst)
    }

    fn had_non_replayable_tool_activity(&self) -> bool {
        self.had_non_replayable_tool_activity.load(Ordering::SeqCst)
    }

    fn snapshot(&self) -> StreamSnapshot {
        let state = lock_state(&self.state);
        StreamSnapshot {
            session_id: self.session_id.clone(),
            stream_id: self.stream_id.clone(),
            turn_id: self.turn_id.clone(),
            persistence_run_id: self.run_id.clone(),
            accepted_seq: self.accepted_seq.load(Ordering::SeqCst),
            durable_seq: self.durable_seq.load(Ordering::SeqCst),
            committed_seq: self.committed_seq.load(Ordering::SeqCst),
            status: state.status.clone(),
            events: state.durable_events.clone(),
        }
    }
}

pub(crate) fn active(session_id: &str) -> Option<Arc<StreamCoordinator>> {
    let mut map = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let coordinator = map.get(session_id).and_then(Weak::upgrade);
    if coordinator.is_none() {
        map.remove(session_id);
    }
    coordinator
}

pub(crate) fn active_snapshot(session_id: &str) -> Option<StreamSnapshot> {
    active(session_id).map(|coordinator| coordinator.snapshot())
}

/// Immutable local foreground generations consumed by the cross-process Stop
/// watcher. Include the run id so a delayed resolution cannot cancel a
/// replacement coordinator for the same session.
pub(crate) fn active_foreground_stop_generations() -> Vec<(String, String, u64, u64, u64)> {
    let mut map = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut generations = Vec::new();
    map.retain(|_, weak| {
        let Some(coordinator) = weak.upgrade() else {
            return false;
        };
        if !coordinator.closed.load(Ordering::SeqCst)
            && coordinator.source.carries_foreground_user_intent()
        {
            generations.push((
                coordinator.run_id.clone(),
                coordinator.session_id.clone(),
                coordinator.admitted_stop_epoch,
                coordinator.admitted_global_stop_epoch,
                coordinator.admitted_global_stop_receipt_count,
            ));
        }
        true
    });
    generations
}

pub(crate) fn request_foreground_stop_for_run(run_id: &str) -> bool {
    let map = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    map.values().filter_map(Weak::upgrade).any(|coordinator| {
        if coordinator.run_id == run_id
            && coordinator.source.carries_foreground_user_intent()
            && !coordinator.closed.load(Ordering::SeqCst)
        {
            coordinator.cancel.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CheckingSink {
        db: Arc<SessionDB>,
        session_id: String,
        seen: AtomicU64,
        observed_before_durable: AtomicBool,
    }

    impl EventSink for CheckingSink {
        fn send(&self, event: &str) {
            let seq = serde_json::from_str::<serde_json::Value>(event)
                .ok()
                .and_then(|value| value.get("_oc_seq").and_then(|seq| seq.as_u64()))
                .unwrap_or(0);
            let durable = self
                .db
                .latest_stream_run_snapshot(&self.session_id)
                .ok()
                .flatten()
                .map(|snapshot| snapshot.run.durable_seq)
                .unwrap_or(0);
            if durable < seq {
                self.observed_before_durable.store(true, Ordering::SeqCst);
            }
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_barrier_delivers_only_after_journal_commit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("barrier.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let sink = Arc::new(CheckingSink {
            db: db.clone(),
            session_id: session.id.clone(),
            seen: AtomicU64::new(0),
            observed_before_durable: AtomicBool::new(false),
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let coordinator = StreamCoordinator::create(
            db.clone(),
            session.id.clone(),
            ChatSource::Subagent,
            None,
            None,
            sink.clone(),
            cancel,
            None,
        )
        .await
        .expect("coordinator");
        coordinator
            .begin_attempt(Some("p"), Some("m"), Some("anthropic"))
            .await
            .expect("attempt");
        coordinator
            .accept_event(
                &serde_json::json!({
                    "type":"tool_call","call_id":"c1","name":"write_file",
                    "arguments":"{}"
                })
                .to_string(),
            )
            .expect("accept tool call");
        assert!(coordinator.had_tool_activity());

        let durable = coordinator
            .flush(FlushReason::ToolBoundary)
            .await
            .expect("tool barrier");
        assert_eq!(durable, 1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while sink.seen.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("post-durable delivery");
        assert_eq!(sink.seen.load(Ordering::SeqCst), 1);
        assert!(!sink.observed_before_durable.load(Ordering::SeqCst));
        let snapshot = db
            .stream_run_snapshot(coordinator.persistence_run_id())
            .expect("snapshot")
            .expect("run");
        assert_eq!(snapshot.run.durable_seq, 1);
        assert_eq!(snapshot.journal.len(), 1);

        db.interrupt_stream_run(
            coordinator.persistence_run_id(),
            coordinator.current_attempt_no(),
            crate::session::ChatTurnStatus::Interrupted,
            Some("test"),
            None,
        )
        .expect("close run");
        coordinator.mark_interrupted("interrupted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn adjacent_text_deltas_share_one_compact_journal_segment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("coalesce.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let sink = Arc::new(CheckingSink {
            db: db.clone(),
            session_id: session.id.clone(),
            seen: AtomicU64::new(0),
            observed_before_durable: AtomicBool::new(false),
        });
        let coordinator = StreamCoordinator::create(
            db.clone(),
            session.id,
            ChatSource::Subagent,
            None,
            None,
            sink.clone(),
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await
        .expect("coordinator");
        coordinator
            .begin_attempt(Some("p"), Some("m"), Some("anthropic"))
            .await
            .expect("attempt");
        for content in ["a", "b", "c"] {
            coordinator
                .accept_event(
                    &serde_json::json!({"type":"text_delta","content":content}).to_string(),
                )
                .expect("accept text");
        }
        assert_eq!(
            coordinator
                .flush(FlushReason::RoundEnd)
                .await
                .expect("flush"),
            3
        );
        let snapshot = db
            .stream_run_snapshot(coordinator.persistence_run_id())
            .expect("snapshot")
            .expect("run");
        let events: Vec<JournalEvent> =
            serde_json::from_str(&snapshot.journal[0].payload).expect("payload");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start_seq(), 1);
        assert_eq!(events[0].seq, 3);
        let payload: serde_json::Value =
            serde_json::from_str(&events[0].event).expect("merged event");
        assert_eq!(
            payload.get("content").and_then(|value| value.as_str()),
            Some("abc")
        );
        assert_eq!(coordinator.trailing_text(), "abc");
        tokio::time::timeout(Duration::from_secs(1), async {
            while sink.seen.load(Ordering::SeqCst) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("raw live delivery");
        assert_eq!(
            sink.seen.load(Ordering::SeqCst),
            3,
            "live sink keeps raw deltas"
        );
        assert!(!sink.observed_before_durable.load(Ordering::SeqCst));
        coordinator
            .checkpoint_context(&[], coordinator.context_revision())
            .await
            .expect("coalesced checkpoint");
        let checkpointed = db
            .stream_run_snapshot(coordinator.persistence_run_id())
            .expect("snapshot")
            .expect("run");
        assert_eq!(checkpointed.run.checkpoint_seq, 3);
        assert_eq!(checkpointed.attempts[0].checkpoint_seq, 3);

        db.interrupt_stream_run(
            coordinator.persistence_run_id(),
            coordinator.current_attempt_no(),
            crate::session::ChatTurnStatus::Interrupted,
            Some("test"),
            None,
        )
        .expect("close run");
        coordinator.mark_interrupted("interrupted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarized_context_is_the_failover_base_without_adopting_later_tail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("summary-base.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let coordinator = StreamCoordinator::create(
            db.clone(),
            session.id.clone(),
            ChatSource::Desktop,
            None,
            None,
            Arc::new(crate::chat_engine::NoopEventSink),
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await
        .expect("coordinator");
        coordinator
            .begin_attempt(Some("primary"), Some("model-a"), Some("openai_chat"))
            .await
            .expect("first attempt");

        let summary = vec![serde_json::json!({
            "role": "user",
            "content": "validated compact summary with current request"
        })];
        let summary_revision = coordinator
            .checkpoint_summarized_context(&summary, coordinator.context_revision())
            .await
            .expect("publish summary");
        assert!(coordinator.attempt_base_contains_current_user());

        let mut failed_attempt_tail = summary.clone();
        failed_attempt_tail.push(serde_json::json!({
            "role": "assistant",
            "content": "uncommitted provider tail"
        }));
        coordinator
            .checkpoint_context(&failed_attempt_tail, summary_revision)
            .await
            .expect("ordinary round checkpoint");

        coordinator
            .begin_attempt(Some("fallback"), Some("model-b"), Some("anthropic"))
            .await
            .expect("fallback attempt");

        let (stored, _) = db
            .load_context_with_revision(&session.id)
            .expect("load context");
        let restored: Vec<serde_json::Value> =
            serde_json::from_str(stored.as_deref().expect("stored summary"))
                .expect("parse context");
        assert_eq!(restored, summary);
        assert!(coordinator.attempt_base_contains_current_user());

        db.interrupt_stream_run(
            coordinator.persistence_run_id(),
            coordinator.current_attempt_no(),
            crate::session::ChatTurnStatus::Interrupted,
            Some("test"),
            None,
        )
        .expect("close run");
        coordinator.mark_interrupted("interrupted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_coordinator_for_same_session_is_rejected_without_replacing_active_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(SessionDB::open(&dir.path().join("exclusive.db")).expect("db"));
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .expect("session");
        let first = StreamCoordinator::create(
            db.clone(),
            session.id.clone(),
            ChatSource::Subagent,
            None,
            None,
            Arc::new(crate::chat_engine::NoopEventSink),
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await
        .expect("first coordinator");

        let second = StreamCoordinator::create(
            db.clone(),
            session.id.clone(),
            ChatSource::Subagent,
            None,
            None,
            Arc::new(crate::chat_engine::NoopEventSink),
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await;
        assert!(second.is_err());
        assert_eq!(
            active(&session.id)
                .expect("active coordinator")
                .persistence_run_id(),
            first.persistence_run_id(),
        );

        db.interrupt_stream_run(
            first.persistence_run_id(),
            0,
            crate::session::ChatTurnStatus::Interrupted,
            Some("test"),
            None,
        )
        .expect("close first run");
        first.mark_interrupted("interrupted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn incognito_stream_leaves_no_run_journal_or_spool_trace() {
        let root = tempfile::tempdir().expect("tempdir");
        crate::test_support::with_env_vars_async(&[("HA_DATA_DIR", root.path())], || async {
            let db = Arc::new(SessionDB::open(&root.path().join("incognito.db")).expect("db"));
            let session = db
                .create_session_with_project(
                    crate::agent_loader::DEFAULT_AGENT_ID,
                    None,
                    Some(true),
                )
                .expect("incognito session");
            let coordinator = StreamCoordinator::create(
                db.clone(),
                session.id.clone(),
                ChatSource::Desktop,
                Some("private".to_string()),
                None,
                Arc::new(crate::chat_engine::NoopEventSink),
                Arc::new(AtomicBool::new(false)),
                None,
            )
            .await
            .expect("coordinator");
            assert!(!coordinator.is_persistent());
            coordinator
                .begin_attempt(Some("p"), Some("m"), Some("anthropic"))
                .await
                .expect("attempt");
            coordinator
                .accept_event(
                    &serde_json::json!({"type":"text_delta","content":"private"}).to_string(),
                )
                .expect("event");
            assert_eq!(coordinator.flush(FlushReason::FinalEnd).await.unwrap(), 1);
            let spool_path = crate::paths::stream_spool_path(coordinator.persistence_run_id())
                .expect("spool path");
            coordinator.mark_interrupted("interrupted");

            assert!(db
                .latest_stream_run_snapshot(&session.id)
                .expect("snapshot")
                .is_none());
            assert!(!spool_path.exists());
        })
        .await;
    }
}

//! Per-session guard for user-facing chat turns.
//!
//! This sits one layer above `stream_seq`: callers acquire it before they
//! persist the user message, so reloads or duplicate "continue" clicks cannot
//! create a second main turn for the same session.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::stream_seq::{ChatSource, ACTIVE_STREAM_ERROR_CODE};

#[derive(Debug, Clone)]
pub struct ActiveTurnError {
    pub session_id: String,
    pub existing_source: ChatSource,
    cancelled_by_global_stop: bool,
}

impl ActiveTurnError {
    /// The request was admitted before (or during) an emergency global Stop
    /// and was rejected before its user message could be persisted.
    pub fn cancelled_by_global_stop(&self) -> bool {
        self.cancelled_by_global_stop
    }
}

impl fmt::Display for ActiveTurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{ACTIVE_STREAM_ERROR_CODE}: session {} already has an active {} chat turn",
            self.session_id, self.existing_source
        )
    }
}

impl std::error::Error for ActiveTurnError {}

#[derive(Debug, Clone)]
struct Entry {
    token: String,
    turn_id: String,
    client_request_id: Option<String>,
    stream_id: Option<String>,
    source: ChatSource,
    cancel: Arc<AtomicBool>,
    persistence_state: Arc<AtomicU8>,
    accepting_insertions: bool,
}

const PERSISTENCE_PENDING: u8 = 0;
const PERSISTENCE_RUNNING: u8 = 1;
const PERSISTENCE_DURABLE: u8 = 2;
const PERSISTENCE_CANCELLED: u8 = 3;

static ACTIVE_TURNS: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
/// A Stop request keeps this short-lived gate armed while it snapshots and
/// settles work owned by the old turn. Without the gate, the old cleanup can
/// race a freshly-acquired turn in the same session and mistake its resources
/// for leftovers.
static STOP_CLEANUPS: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
/// Process-wide emergency Stop gate. This closes the enumeration gap in
/// `stop_all_sessions`: a turn must not acquire after the active snapshot yet
/// before runtime work is captured, otherwise global cleanup could cancel its
/// resources without signalling its foreground token.
static GLOBAL_STOP_CLEANUPS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
/// Monotonic emergency-Stop generation. Desktop/HTTP requests snapshot this at
/// entry, before any project bootstrap or other pre-registration await. A Stop
/// advances it while holding the active registry lock, so an older request can
/// never register after the bounded global cleanup gate has been released.
static GLOBAL_STOP_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Request-scoped stops may arrive before the shell has registered a turn.
/// When the caller already knows the target session, retain that binding so a
/// reused/malformed request id cannot cancel a turn in another session.
static PENDING_CLIENT_CANCELS: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
const PENDING_CLIENT_CANCELS_MAX: usize = 4096;

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    ACTIVE_TURNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending_client_cancels() -> &'static Mutex<HashMap<String, Option<String>>> {
    PENDING_CLIENT_CANCELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn stop_cleanups() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    STOP_CLEANUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn global_stop_cleanups() -> &'static Mutex<HashSet<String>> {
    GLOBAL_STOP_CLEANUPS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Poison-tolerant lock on the active-turn registry. A panic while another
/// thread held this lock must NOT cascade into callers — most critically
/// `session::cleanup_watcher::cleanup_session` (live-cancel step) runs BEFORE
/// the incognito on-disk scrub, so a poison-panic here would skip burning a
/// session's artifacts. The guarded sections are short, panic-free HashMap ops,
/// so recovering the inner data is always safe (matches the `unwrap_or_else(|p|
/// p.into_inner())` idiom the other cleanup steps already use).
fn registry_lock() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    registry().lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug)]
pub struct ActiveTurnGuard {
    session_id: String,
    token: String,
    released: bool,
}

impl ActiveTurnGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut map = registry_lock();
        if map
            .get(&self.session_id)
            .map(|entry| entry.token.as_str() == self.token)
            .unwrap_or(false)
        {
            map.remove(&self.session_id);
        }
        self.released = true;
    }
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
pub struct StopCleanupGuard {
    session_id: String,
    token: String,
    released: bool,
}

impl StopCleanupGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        let mut stopping = stop_cleanups()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(tokens) = stopping.get_mut(&self.session_id) {
            tokens.remove(&self.token);
            if tokens.is_empty() {
                stopping.remove(&self.session_id);
            }
        }
        self.released = true;
    }
}

impl Drop for StopCleanupGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug)]
pub struct GlobalStopCleanupGuard {
    token: String,
    released: bool,
}

impl GlobalStopCleanupGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        global_stop_cleanups()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.token);
        self.released = true;
    }
}

impl Drop for GlobalStopCleanupGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// Prevent a replacement foreground turn from acquiring this session until
/// the Stop caller has captured the old turn's exact cleanup targets.
///
/// Acquisition and gate installation both serialize through `ACTIVE_TURNS`,
/// so either a racing turn is visible to Stop and gets cancelled, or it sees
/// this gate and is rejected. They cannot pass each other unseen.
pub fn begin_stop_cleanup(session_id: &str) -> StopCleanupGuard {
    let token = uuid::Uuid::new_v4().to_string();
    let _active = registry_lock();
    stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(session_id.to_string())
        .or_default()
        .insert(token.clone());
    StopCleanupGuard {
        session_id: session_id.to_string(),
        token,
        released: false,
    }
}

/// Prevent any replacement foreground turn from entering while a process-wide
/// emergency Stop enumerates and settles its exact cleanup targets.
pub fn begin_global_stop_cleanup() -> GlobalStopCleanupGuard {
    let token = uuid::Uuid::new_v4().to_string();
    let _active = registry_lock();
    GLOBAL_STOP_GENERATION.fetch_add(1, Ordering::SeqCst);
    global_stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(token.clone());
    GlobalStopCleanupGuard {
        token,
        released: false,
    }
}

/// Whether session-scoped or process-wide Stop cleanup still owns admission
/// for this session. ACP does not register an [`ActiveTurnGuard`], so it uses
/// this read-only gate before accepting a replacement prompt.
pub fn stop_cleanup_active(session_id: &str) -> bool {
    let _active = registry_lock();
    if !global_stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
    {
        return true;
    }
    stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .is_some_and(|tokens| !tokens.is_empty())
}

/// Admission snapshot for a foreground request that may perform async work
/// before it can register its active turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForegroundRequestAdmission {
    global_stop_generation: u64,
}

/// Capture at the transport entry point, before its first await.
pub fn begin_foreground_request() -> ForegroundRequestAdmission {
    ForegroundRequestAdmission {
        global_stop_generation: GLOBAL_STOP_GENERATION.load(Ordering::SeqCst),
    }
}

pub fn try_acquire(
    session_id: &str,
    source: ChatSource,
    turn_id: String,
    cancel: Arc<AtomicBool>,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    try_acquire_with_client_request_id(session_id, source, turn_id, None, cancel)
}

/// Acquire a foreground turn and associate it with the client request that
/// initiated it. The request id lets a draft-session Stop target this exact
/// turn before `session_created` / `turn_started` has reached the UI.
pub fn try_acquire_with_client_request_id(
    session_id: &str,
    source: ChatSource,
    turn_id: String,
    client_request_id: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    try_acquire_inner(session_id, source, turn_id, client_request_id, None, cancel)
}

/// Register a Desktop/HTTP request against the generation captured at its
/// transport entry point. If an emergency Stop began while the request was in
/// bootstrap or another pre-registration await, reject it even after the
/// short-lived global cleanup gate has already been released.
pub fn try_acquire_foreground_request(
    admission: ForegroundRequestAdmission,
    session_id: &str,
    source: ChatSource,
    turn_id: String,
    client_request_id: Option<String>,
    cancel: Arc<AtomicBool>,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    try_acquire_inner(
        session_id,
        source,
        turn_id,
        client_request_id,
        Some(admission.global_stop_generation),
        cancel,
    )
}

fn try_acquire_inner(
    session_id: &str,
    source: ChatSource,
    turn_id: String,
    client_request_id: Option<String>,
    request_global_stop_generation: Option<u64>,
    cancel: Arc<AtomicBool>,
) -> Result<ActiveTurnGuard, ActiveTurnError> {
    let token = uuid::Uuid::new_v4().to_string();
    let mut map = registry_lock();
    if let Some(existing) = map.get(session_id) {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: existing.source,
            cancelled_by_global_stop: false,
        });
    }
    if request_global_stop_generation
        .is_some_and(|generation| generation != GLOBAL_STOP_GENERATION.load(Ordering::SeqCst))
    {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: source,
            cancelled_by_global_stop: true,
        });
    }
    if !global_stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
    {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: source,
            cancelled_by_global_stop: true,
        });
    }
    if stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(session_id)
        .is_some_and(|tokens| !tokens.is_empty())
    {
        return Err(ActiveTurnError {
            session_id: session_id.to_string(),
            existing_source: source,
            cancelled_by_global_stop: false,
        });
    }
    if client_request_id.as_deref().is_some_and(|request_id| {
        let mut pending = pending_client_cancels()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matches_session = pending.get(request_id).is_some_and(|expected_session_id| {
            expected_session_id
                .as_deref()
                .is_none_or(|expected| expected == session_id)
        });
        if matches_session {
            pending.remove(request_id);
        }
        matches_session
    }) {
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let persistence_state = Arc::new(AtomicU8::new(if cancel.load(Ordering::SeqCst) {
        PERSISTENCE_CANCELLED
    } else {
        PERSISTENCE_PENDING
    }));
    map.insert(
        session_id.to_string(),
        Entry {
            token: token.clone(),
            turn_id,
            client_request_id,
            stream_id: None,
            source,
            cancel,
            persistence_state,
            accepting_insertions: true,
        },
    );
    Ok(ActiveTurnGuard {
        session_id: session_id.to_string(),
        token,
        released: false,
    })
}

#[derive(Debug, Clone)]
pub struct ActiveTurnSnapshot {
    pub session_id: String,
    pub turn_id: String,
    pub stream_id: Option<String>,
    pub source: ChatSource,
    pub cancel: Arc<AtomicBool>,
}

/// Resolve the exact Stop target shared by the desktop and HTTP adapters.
///
/// Before `turn_started` reaches the client, a Stop may know the session and
/// client request id but not the turn id. If the request id already resolves
/// to an active turn, preserve that turn id even when the caller supplied the
/// session explicitly; falling back to a session-wide Stop could otherwise
/// race and cancel a replacement turn.
pub fn resolve_stop_target(
    explicit_session_id: Option<&str>,
    explicit_turn_id: Option<&str>,
    request_target: Option<&ActiveTurnSnapshot>,
) -> (Option<String>, Option<String>) {
    let session_id = explicit_session_id
        .map(str::to_string)
        .or_else(|| request_target.map(|active| active.session_id.clone()));
    let turn_id = explicit_turn_id
        .map(str::to_string)
        .or_else(|| request_target.map(|active| active.turn_id.clone()));
    (session_id, turn_id)
}

pub fn current(session_id: &str) -> Option<ActiveTurnSnapshot> {
    let map = registry_lock();
    map.get(session_id).map(|entry| ActiveTurnSnapshot {
        session_id: session_id.to_string(),
        turn_id: entry.turn_id.clone(),
        stream_id: entry.stream_id.clone(),
        source: entry.source,
        cancel: Arc::clone(&entry.cancel),
    })
}

/// Resolve an active foreground turn by the opaque client request id.
pub fn current_for_client_request(client_request_id: &str) -> Option<ActiveTurnSnapshot> {
    let map = registry_lock();
    map.iter().find_map(|(session_id, entry)| {
        (entry.client_request_id.as_deref() == Some(client_request_id)).then(|| {
            ActiveTurnSnapshot {
                session_id: session_id.clone(),
                turn_id: entry.turn_id.clone(),
                stream_id: entry.stream_id.clone(),
                source: entry.source,
                cancel: Arc::clone(&entry.cancel),
            }
        })
    })
}

#[derive(Debug, Clone)]
pub enum ActiveTurnCancelOutcome {
    Cancelled(ActiveTurnSnapshot),
    NotFound,
    TurnMismatch,
}

/// Signal one active turn while still holding the registry lock shared with
/// the pre-turn persistence boundary. This makes Stop linearizable with the
/// user-message/chat-turn transaction: either persistence commits first and
/// Stop observes a durable turn, or cancellation wins and persistence is
/// rejected.
pub fn cancel_current(session_id: &str, expected_turn_id: Option<&str>) -> ActiveTurnCancelOutcome {
    let map = registry_lock();
    let Some(entry) = map.get(session_id) else {
        return ActiveTurnCancelOutcome::NotFound;
    };
    if expected_turn_id.is_some_and(|expected| expected != entry.turn_id) {
        return ActiveTurnCancelOutcome::TurnMismatch;
    }
    entry.cancel.store(true, Ordering::SeqCst);
    let _ = entry.persistence_state.compare_exchange(
        PERSISTENCE_PENDING,
        PERSISTENCE_CANCELLED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    ActiveTurnCancelOutcome::Cancelled(ActiveTurnSnapshot {
        session_id: session_id.to_string(),
        turn_id: entry.turn_id.clone(),
        stream_id: entry.stream_id.clone(),
        source: entry.source,
        cancel: Arc::clone(&entry.cancel),
    })
}

/// Signal every active turn under the same registry lock used by persistence.
pub fn cancel_all_current() -> Vec<ActiveTurnSnapshot> {
    let map = registry_lock();
    map.iter()
        .map(|(session_id, entry)| {
            entry.cancel.store(true, Ordering::SeqCst);
            let _ = entry.persistence_state.compare_exchange(
                PERSISTENCE_PENDING,
                PERSISTENCE_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            ActiveTurnSnapshot {
                session_id: session_id.clone(),
                turn_id: entry.turn_id.clone(),
                stream_id: entry.stream_id.clone(),
                source: entry.source,
                cancel: Arc::clone(&entry.cancel),
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub enum ClientRequestCancelOutcome {
    Active(ActiveTurnSnapshot),
    Latched,
    SessionMismatch,
}

/// Cancel an already-registered client request, or latch the cancellation if
/// the Stop request won the transport race and arrived first. Both this path
/// and acquisition take the active registry before the pending map, so the
/// lookup/insert and consume/register transitions cannot pass each other.
pub fn cancel_or_latch_client_request(
    client_request_id: &str,
    expected_session_id: Option<&str>,
) -> ClientRequestCancelOutcome {
    let map = registry_lock();
    if let Some((session_id, entry)) = map
        .iter()
        .find(|(_, entry)| entry.client_request_id.as_deref() == Some(client_request_id))
    {
        if expected_session_id.is_some_and(|expected| expected != session_id) {
            return ClientRequestCancelOutcome::SessionMismatch;
        }
        entry
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = entry.persistence_state.compare_exchange(
            PERSISTENCE_PENDING,
            PERSISTENCE_CANCELLED,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        return ClientRequestCancelOutcome::Active(ActiveTurnSnapshot {
            session_id: session_id.clone(),
            turn_id: entry.turn_id.clone(),
            stream_id: entry.stream_id.clone(),
            source: entry.source,
            cancel: Arc::clone(&entry.cancel),
        });
    }

    let mut pending = pending_client_cancels()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing_session_id) = pending.get(client_request_id) {
        if existing_session_id.as_deref().is_some()
            && expected_session_id.is_some()
            && existing_session_id.as_deref() != expected_session_id
        {
            return ClientRequestCancelOutcome::SessionMismatch;
        }
        if existing_session_id.is_none() && expected_session_id.is_some() {
            pending.insert(
                client_request_id.to_string(),
                expected_session_id.map(str::to_string),
            );
        }
        return ClientRequestCancelOutcome::Latched;
    }
    if pending.len() >= PENDING_CLIENT_CANCELS_MAX {
        if let Some(evicted) = pending.keys().next().cloned() {
            pending.remove(&evicted);
        }
    }
    pending.insert(
        client_request_id.to_string(),
        expected_session_id.map(str::to_string),
    );
    ClientRequestCancelOutcome::Latched
}

/// Observe a pre-registration request cancellation without consuming it.
/// Project bootstrap uses this after publishing its client-request mapping so
/// a Stop that arrived even earlier can also latch bootstrap cancellation.
pub fn has_latched_client_cancel(client_request_id: &str) -> bool {
    let _map = registry_lock();
    pending_client_cancels()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(client_request_id)
}

/// Execute a short synchronous operation only while this exact user-facing
/// turn still accepts queued-message insertion. The registry lock deliberately
/// spans `operation`: turn finalization takes the same lock to close insertion
/// first, then falls queued rows back after the lock is released. Therefore an
/// insertion either commits before cleanup observes it or is rejected after
/// cleanup has closed the turn; it cannot be written behind cleanup's back.
pub fn with_insertion_target<T>(
    session_id: &str,
    turn_id: &str,
    operation: impl FnOnce() -> T,
) -> Result<T, &'static str> {
    with_insertion_target_for(session_id, turn_id, operation, |source| {
        matches!(
            source,
            ChatSource::Desktop | ChatSource::Http | ChatSource::SessionTool
        )
    })
}

/// Channel prompts carry a different permission / KB origin from owner
/// Desktop and HTTP prompts. They may only steer an already-Channel turn;
/// otherwise the durable queue creates a fresh least-privilege Channel turn
/// after the owner turn finishes.
pub(crate) fn with_channel_insertion_target<T>(
    session_id: &str,
    turn_id: &str,
    operation: impl FnOnce() -> T,
) -> Result<T, &'static str> {
    with_insertion_target_for(session_id, turn_id, operation, |source| {
        matches!(source, ChatSource::Channel)
    })
}

fn with_insertion_target_for<T>(
    session_id: &str,
    turn_id: &str,
    operation: impl FnOnce() -> T,
    accepts_source: impl FnOnce(ChatSource) -> bool,
) -> Result<T, &'static str> {
    let map = registry_lock();
    let Some(entry) = map.get(session_id) else {
        return Err("no active turn for session");
    };
    if entry.turn_id != turn_id {
        return Err("active turn id does not match");
    }
    if entry.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("active turn is cancelling");
    }
    if !entry.accepting_insertions {
        return Err("active turn is finishing");
    }
    if !accepts_source(entry.source) {
        return Err("active turn source does not support insertion");
    }
    Ok(operation())
}

/// Claim and execute the transaction that first makes a foreground prompt
/// durable. The atomic state is the ordering point with Stop; the registry
/// lock is deliberately released before the potentially blocking operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceTargetOutcome<T> {
    Committed(T),
    CommittedAfterCancel(T),
    CancelledBeforeCommit,
}

pub fn with_persistence_target<T>(
    session_id: &str,
    turn_id: &str,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<PersistenceTargetOutcome<T>> {
    let (cancel, persistence_state) = {
        let map = registry_lock();
        let Some(entry) = map.get(session_id) else {
            return Ok(PersistenceTargetOutcome::CancelledBeforeCommit);
        };
        if entry.turn_id != turn_id {
            return Ok(PersistenceTargetOutcome::CancelledBeforeCommit);
        }
        (
            Arc::clone(&entry.cancel),
            Arc::clone(&entry.persistence_state),
        )
    };

    // This atomic claim is the ordering point with Stop. The registry mutex is
    // released before SQLite/filesystem work starts, so a stalled persistence
    // cannot block cancellation or admission for unrelated sessions.
    if persistence_state
        .compare_exchange(
            PERSISTENCE_PENDING,
            PERSISTENCE_RUNNING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return Ok(PersistenceTargetOutcome::CancelledBeforeCommit);
    }

    let result = operation();
    match result {
        Ok(value) => {
            persistence_state.store(PERSISTENCE_DURABLE, Ordering::SeqCst);
            if cancel.load(Ordering::SeqCst) {
                Ok(PersistenceTargetOutcome::CommittedAfterCancel(value))
            } else {
                Ok(PersistenceTargetOutcome::Committed(value))
            }
        }
        Err(error) => {
            persistence_state.store(PERSISTENCE_CANCELLED, Ordering::SeqCst);
            Err(error)
        }
    }
}

/// Close the insertion gate for an exact turn before its durable queue rows
/// are converted to ordinary after-reply sends.
pub fn stop_accepting_insertions(session_id: &str, turn_id: &str) -> bool {
    let mut map = registry_lock();
    let Some(entry) = map.get_mut(session_id) else {
        return false;
    };
    if entry.turn_id != turn_id {
        return false;
    }
    entry.accepting_insertions = false;
    true
}

/// Fast-path check for the per-token streaming hot loop: returns
/// `Some(accepting)` (`accepting = !cancel`) when `(session_id, turn_id)` is
/// the live active turn, **without** cloning the snapshot's Strings + Arc that
/// [`current`] allocates. Returns `None` when no entry matches that exact turn
/// (the caller decides the fallback — see `turn_accepts_stream_event`).
pub fn is_accepting(session_id: &str, turn_id: &str) -> Option<bool> {
    let map = registry_lock();
    map.get(session_id).and_then(|entry| {
        if entry.turn_id == turn_id {
            Some(!entry.cancel.load(std::sync::atomic::Ordering::SeqCst))
        } else {
            None
        }
    })
}

/// True when the session has *any* live active-turn entry (turn_id agnostic).
/// Lets `turn_accepts_stream_event` preserve the original "a different turn is
/// live → reject without a DB probe" semantics without cloning a snapshot.
pub fn has_entry(session_id: &str) -> bool {
    registry_lock().contains_key(session_id)
}

pub fn all_current() -> Vec<ActiveTurnSnapshot> {
    let map = registry_lock();
    map.iter()
        .map(|(session_id, entry)| ActiveTurnSnapshot {
            session_id: session_id.clone(),
            turn_id: entry.turn_id.clone(),
            stream_id: entry.stream_id.clone(),
            source: entry.source,
            cancel: Arc::clone(&entry.cancel),
        })
        .collect()
}

pub fn all_current_turn_ids() -> Vec<String> {
    let map = registry_lock();
    map.values().map(|entry| entry.turn_id.clone()).collect()
}

/// Force-release one active turn by `(session_id, turn_id)`.
///
/// Used by the user-stop watchdog after it has already finalized the turn in
/// persistent state. The turn id guard prevents an old watchdog from clearing
/// a newer turn that started in the same session.
pub fn force_release(session_id: &str, turn_id: &str) -> bool {
    let mut map = registry_lock();
    let matches = map
        .get(session_id)
        .map(|entry| entry.turn_id == turn_id)
        .unwrap_or(false);
    if matches {
        map.remove(session_id);
    }
    matches
}

/// Clear all in-memory active turn entries.
///
/// Used during runtime startup after persisted `running` / `cancelling` turns
/// have been marked interrupted. This is mostly relevant for hot-reload/dev
/// processes where Rust statics can outlive a logical app restart.
pub fn clear_all() -> usize {
    let mut map = registry_lock();
    let n = map.len();
    map.clear();
    pending_client_cancels()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    global_stop_cleanups()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    n
}

// ── Finalize re-entry guard ───────────────────────────────────────────
//
// `finalize_turn_context` can plausibly be invoked twice for the same
// turn — engine.rs failure convergence races with a SIGTERM signal
// handler walking `all_current()`; startup sweep races with a
// crash-flush left over from the previous run. The second call must
// be a no-op (already wrote `[系统事件]` marker, already wrote event
// row, already finished chat_turn). Re-entry guard is keyed by turn
// id so cross-session pairs don't interfere.

/// Bounded FIFO so a long-running process doesn't accumulate every
/// finalized turn id forever. 4096 × ~50 bytes ≈ 200 KiB worst case,
/// well above realistic re-entry windows (the same turn id is only
/// reused at process restart, and we want re-entry detection during
/// the **same** process lifetime).
const FINALIZED_RING_MAX: usize = 4096;

struct FinalizedRing {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl FinalizedRing {
    fn new() -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` if `id` was newly inserted.
    fn insert(&mut self, id: String) -> bool {
        if !self.set.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        while self.order.len() > FINALIZED_RING_MAX {
            if let Some(evicted) = self.order.pop_front() {
                self.set.remove(&evicted);
            }
        }
        true
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.set.clear();
        self.order.clear();
    }
}

static FINALIZED_TURNS: OnceLock<Mutex<FinalizedRing>> = OnceLock::new();

fn finalized_ring() -> &'static Mutex<FinalizedRing> {
    FINALIZED_TURNS.get_or_init(|| Mutex::new(FinalizedRing::new()))
}

/// Test-and-insert: returns `true` if this is the *first* finalize
/// call for `turn_id`; subsequent calls return `false` and the caller
/// must short-circuit. Passing `None` (sweep paths with no `turn_id`)
/// always returns `true` — those callers handle idempotency by other
/// means (DB UPDATE conditions, mostly).
pub fn mark_finalized(turn_id: Option<&str>) -> bool {
    let Some(id) = turn_id else { return true };
    let mut ring = finalized_ring().lock().unwrap_or_else(|p| p.into_inner());
    ring.insert(id.to_string())
}

/// Reset the re-entry guard. Test-only.
#[cfg(test)]
pub(crate) fn reset_finalized_for_test() {
    if let Ok(mut ring) = finalized_ring().lock() {
        ring.clear();
    }
}

pub fn set_stream_id(session_id: &str, turn_id: &str, stream_id: &str) -> bool {
    let mut map = registry_lock();
    match map.get_mut(session_id) {
        Some(entry) if entry.turn_id == turn_id => {
            entry.stream_id = Some(stream_id.to_string());
            true
        }
        _ => false,
    }
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("active turn test lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_turn_until_guard_drops() {
        let _lock = test_lock();
        let sid = "test-active-turn-rejects-second";
        {
            let _guard = try_acquire(
                sid,
                ChatSource::Desktop,
                "turn-1".to_string(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            let err = try_acquire(
                sid,
                ChatSource::Http,
                "turn-2".to_string(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap_err();
            assert_eq!(err.session_id, sid);
            assert_eq!(err.existing_source, ChatSource::Desktop);
        }

        let _guard = try_acquire(
            sid,
            ChatSource::Http,
            "turn-3".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    }

    #[test]
    fn current_snapshot_tracks_stream_id() {
        let _lock = test_lock();
        let sid = "test-active-turn-current-snapshot";
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-current".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        assert_eq!(current(sid).unwrap().turn_id, "turn-current");
        assert!(set_stream_id(sid, "turn-current", "stream-current"));
        let snapshot = current(sid).unwrap();
        assert_eq!(snapshot.stream_id.as_deref(), Some("stream-current"));
        assert!(Arc::ptr_eq(&snapshot.cancel, &cancel));
        assert!(!set_stream_id(sid, "other-turn", "stream-other"));
    }

    #[test]
    fn request_id_targets_a_draft_turn_before_session_events_arrive() {
        let _lock = test_lock();
        let sid = "test-active-turn-request-target";
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire_with_client_request_id(
            sid,
            ChatSource::Desktop,
            "turn-request-target".to_string(),
            Some("request-target".to_string()),
            Arc::clone(&cancel),
        )
        .unwrap();

        assert!(current_for_client_request("unknown-request").is_none());
        let snapshot = current_for_client_request("request-target").unwrap();
        assert_eq!(snapshot.session_id, sid);
        assert_eq!(snapshot.turn_id, "turn-request-target");
        assert!(Arc::ptr_eq(&snapshot.cancel, &cancel));

        let cancelled = cancel_or_latch_client_request("request-target", Some(sid));
        let ClientRequestCancelOutcome::Active(cancelled) = cancelled else {
            panic!("expected an active request cancellation");
        };
        assert_eq!(cancelled.turn_id, "turn-request-target");
        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));

        let (target_session_id, target_turn_id) =
            resolve_stop_target(Some(sid), None, Some(&cancelled));
        assert_eq!(target_session_id.as_deref(), Some(sid));
        assert_eq!(target_turn_id.as_deref(), Some("turn-request-target"));
    }

    #[test]
    fn request_stop_latches_when_it_arrives_before_turn_registration() {
        let _lock = test_lock();
        let sid = "test-active-turn-request-latch";
        let cancel = Arc::new(AtomicBool::new(false));

        assert!(matches!(
            cancel_or_latch_client_request("request-latch", Some(sid)),
            ClientRequestCancelOutcome::Latched
        ));
        let _guard = try_acquire_with_client_request_id(
            sid,
            ChatSource::Http,
            "turn-request-latch".to_string(),
            Some("request-latch".to_string()),
            Arc::clone(&cancel),
        )
        .unwrap();

        assert!(cancel.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn latched_request_stop_does_not_signal_another_turn_in_the_session() {
        let _lock = test_lock();
        let sid = "test-active-turn-request-latch-isolation";
        let active_cancel = Arc::new(AtomicBool::new(false));
        let _active_guard = try_acquire_with_client_request_id(
            sid,
            ChatSource::Http,
            "turn-a".to_string(),
            Some("request-a".to_string()),
            Arc::clone(&active_cancel),
        )
        .expect("active request A");

        assert!(matches!(
            cancel_or_latch_client_request("request-b", Some(sid)),
            ClientRequestCancelOutcome::Latched
        ));
        assert!(
            !active_cancel.load(std::sync::atomic::Ordering::SeqCst),
            "request B's latch must not become a session-wide signal for request A"
        );
    }

    #[test]
    fn request_stop_for_known_session_does_not_cancel_another_session() {
        let _lock = test_lock();
        let expected_sid = "test-active-turn-request-expected-session";
        let other_sid = "test-active-turn-request-other-session";

        assert!(matches!(
            cancel_or_latch_client_request("request-session-bound", Some(expected_sid)),
            ClientRequestCancelOutcome::Latched
        ));

        let other_cancel = Arc::new(AtomicBool::new(false));
        let _other_guard = try_acquire_with_client_request_id(
            other_sid,
            ChatSource::Http,
            "turn-other-session".to_string(),
            Some("request-session-bound".to_string()),
            Arc::clone(&other_cancel),
        )
        .unwrap();
        assert!(!other_cancel.load(std::sync::atomic::Ordering::SeqCst));

        let expected_cancel = Arc::new(AtomicBool::new(false));
        let _expected_guard = try_acquire_with_client_request_id(
            expected_sid,
            ChatSource::Desktop,
            "turn-expected-session".to_string(),
            Some("request-session-bound".to_string()),
            Arc::clone(&expected_cancel),
        )
        .unwrap();
        assert!(expected_cancel.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn is_accepting_and_has_entry_match_current_semantics() {
        let _lock = test_lock();
        let sid = "test-active-turn-is-accepting";
        // No entry yet.
        assert_eq!(is_accepting(sid, "turn-x"), None);
        assert!(!has_entry(sid));

        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-acc".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        // Matching live turn, not cancelled → Some(true).
        assert_eq!(is_accepting(sid, "turn-acc"), Some(true));
        assert!(has_entry(sid));
        // Session has an entry but under a *different* turn → None (caller
        // rejects without a DB probe, preserving old semantics).
        assert_eq!(is_accepting(sid, "turn-other"), None);
        // Cancelled → Some(false).
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(is_accepting(sid, "turn-acc"), Some(false));
    }

    #[test]
    fn all_current_returns_cancel_handles() {
        let _lock = test_lock();
        let sid = "test-active-turn-all-current";
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-all-current".to_string(),
            Arc::clone(&cancel),
        )
        .unwrap();

        let snapshot = all_current()
            .into_iter()
            .find(|snapshot| snapshot.session_id == sid)
            .unwrap();
        assert_eq!(snapshot.turn_id, "turn-all-current");
        assert!(Arc::ptr_eq(&snapshot.cancel, &cancel));
    }

    #[test]
    fn mark_finalized_is_one_shot_per_turn_id() {
        let _lock = test_lock();
        reset_finalized_for_test();
        assert!(mark_finalized(Some("t-first")));
        assert!(!mark_finalized(Some("t-first")));
        assert!(mark_finalized(Some("t-second")));
        // None means "no turn id" — always proceed (callers handle
        // idempotency another way).
        assert!(mark_finalized(None));
        assert!(mark_finalized(None));
    }

    #[test]
    fn clear_all_removes_active_turns() {
        let _lock = test_lock();
        let sid = "test-active-turn-clear-all";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-clear".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(current(sid).is_some());
        assert!(clear_all() >= 1);
        assert!(current(sid).is_none());
    }

    #[test]
    fn force_release_requires_matching_turn_id() {
        let _lock = test_lock();
        let sid = "test-active-turn-force-release";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-force".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!force_release(sid, "other-turn"));
        assert!(current(sid).is_some());
        assert!(force_release(sid, "turn-force"));
        assert!(current(sid).is_none());
    }

    #[test]
    fn stop_cleanup_gate_blocks_only_until_old_snapshot_finishes() {
        let _lock = test_lock();
        let sid = "test-active-turn-stop-cleanup-gate";
        let gate = begin_stop_cleanup(sid);

        assert!(try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-during-stop".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .is_err());

        drop(gate);
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-after-stop".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("replacement turn should acquire after cleanup snapshot");
    }

    #[test]
    fn global_stop_cleanup_gate_blocks_every_session_until_release() {
        let _lock = test_lock();
        let gate = begin_global_stop_cleanup();
        for (session_id, source) in [
            ("test-global-stop-a", ChatSource::Desktop),
            ("test-global-stop-b", ChatSource::Http),
        ] {
            assert!(try_acquire(
                session_id,
                source,
                format!("turn-{session_id}"),
                Arc::new(AtomicBool::new(false)),
            )
            .is_err());
        }

        drop(gate);
        let _guard = try_acquire(
            "test-global-stop-a",
            ChatSource::Desktop,
            "turn-after-global-stop".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("replacement turn should acquire after global cleanup");
    }

    #[test]
    fn global_stop_generation_rejects_requests_that_predate_cleanup() {
        let _lock = test_lock();
        let stale_request = begin_foreground_request();

        let gate = begin_global_stop_cleanup();
        drop(gate);

        let error = try_acquire_foreground_request(
            stale_request,
            "test-global-stop-stale-request",
            ChatSource::Desktop,
            "turn-stale-request".to_string(),
            Some("request-stale".to_string()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("request predating global Stop must be rejected");
        assert!(error.cancelled_by_global_stop());

        let fresh_request = begin_foreground_request();
        let _guard = try_acquire_foreground_request(
            fresh_request,
            "test-global-stop-fresh-request",
            ChatSource::Http,
            "turn-fresh-request".to_string(),
            Some("request-fresh".to_string()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("request entering after global Stop should acquire");
    }

    #[test]
    fn global_stop_generation_rejects_stale_channel_admission() {
        let _lock = test_lock();
        let stale_request = begin_foreground_request();
        drop(begin_global_stop_cleanup());

        let error = try_acquire_foreground_request(
            stale_request,
            "test-global-stop-stale-channel",
            ChatSource::Channel,
            "turn-stale-channel".to_string(),
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .expect_err("Channel request predating global Stop must be rejected");
        assert!(error.cancelled_by_global_stop());
    }

    #[test]
    fn cancelled_turn_cannot_cross_prompt_persistence_boundary() {
        let _lock = test_lock();
        let sid = "test-active-turn-persistence-boundary";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-persistence".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(matches!(
            cancel_current(sid, Some("turn-persistence")),
            ActiveTurnCancelOutcome::Cancelled(_)
        ));
        assert_eq!(
            with_persistence_target(sid, "turn-persistence", || Ok("persisted")).unwrap(),
            PersistenceTargetOutcome::CancelledBeforeCommit
        );
    }

    #[test]
    fn persistence_does_not_hold_registry_mutex_during_slow_io() {
        let _lock = test_lock();
        let sid = "test-active-turn-persistence-nonblocking";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-persistence-nonblocking".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            with_persistence_target(sid, "turn-persistence-nonblocking", || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok("persisted")
            })
            .unwrap()
        });
        started_rx.recv().unwrap();

        assert!(matches!(
            cancel_current(sid, Some("turn-persistence-nonblocking")),
            ActiveTurnCancelOutcome::Cancelled(_)
        ));
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            PersistenceTargetOutcome::CommittedAfterCancel("persisted")
        );
    }

    #[test]
    fn finishing_turn_closes_insertion_gate() {
        let _lock = test_lock();
        let sid = "test-active-turn-insertion-gate";
        let _guard = try_acquire(
            sid,
            ChatSource::Desktop,
            "turn-insertion".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert_eq!(
            with_insertion_target(sid, "turn-insertion", || "accepted"),
            Ok("accepted")
        );
        assert!(stop_accepting_insertions(sid, "turn-insertion"));
        assert_eq!(
            with_insertion_target(sid, "turn-insertion", || "late"),
            Err("active turn is finishing")
        );
    }

    #[test]
    fn insertion_targets_do_not_cross_owner_and_channel_trust_domains() {
        let _lock = test_lock();
        let desktop_sid = "test-active-turn-desktop-insertion-domain";
        {
            let _guard = try_acquire(
                desktop_sid,
                ChatSource::Desktop,
                "desktop-turn".to_string(),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();

            assert_eq!(
                with_insertion_target(desktop_sid, "desktop-turn", || "owner"),
                Ok("owner")
            );
            assert!(
                with_channel_insertion_target(desktop_sid, "desktop-turn", || "channel").is_err()
            );
        }

        let channel_sid = "test-active-turn-channel-insertion-domain";
        let _guard = try_acquire(
            channel_sid,
            ChatSource::Channel,
            "channel-turn".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert_eq!(
            with_channel_insertion_target(channel_sid, "channel-turn", || "channel"),
            Ok("channel")
        );
        assert!(with_insertion_target(channel_sid, "channel-turn", || "owner").is_err());

        let session_tool_sid = "test-active-turn-session-tool-insertion-domain";
        let _guard = try_acquire(
            session_tool_sid,
            ChatSource::SessionTool,
            "session-tool-turn".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert_eq!(
            with_insertion_target(session_tool_sid, "session-tool-turn", || "owner"),
            Ok("owner")
        );
        assert!(
            with_channel_insertion_target(session_tool_sid, "session-tool-turn", || "channel")
                .is_err()
        );
    }
}

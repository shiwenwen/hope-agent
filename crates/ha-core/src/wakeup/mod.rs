//! Agent self-scheduled wakeups (R10 — aligns with Claude Code's
//! `ScheduleWakeup`).
//!
//! Lets an agent ask to be woken back into **the current session** after a
//! delay — to poll an external state the harness can't track (a CI run, a
//! remote queue), or to re-check something later. The agent calls
//! `schedule_wakeup(delay_secs, note)`, ends its turn, and at `fire_at` a
//! `<wakeup>` message is injected back through the **shared injection pipeline**
//! (`subagent::injection::inject_and_run_parent`) — so it inherits R2's
//! idle-gating (waits for a live turn to finish), cancellation, and retry, and
//! runs a fresh parent turn carrying the note.
//!
//! This is deliberately NOT cron: cron is user-configured, periodic, and may
//! target a separate session with delivery fan-out; a wakeup is agent-initiated,
//! one-shot, and continues the originating session's context. The two don't
//! share an entry point.
//!
//! ## Lifecycle & cross-process model
//! - **Creation** persists a row (unless incognito) and arms a process-local
//!   timer in the *creating* process.
//! - **Restart recovery** (`replay_pending`) is **Primary-only** (mirrors
//!   `async_jobs::replay_pending_jobs`): it re-arms unfired rows; past-due ones
//!   fire immediately. Secondary processes don't re-arm shared rows.
//! - **Delivery** normally deletes the row only when the injection lands. When
//!   an IM mirror attaches, a durable `fired=1` write-ahead claim happens before
//!   engine deltas can mutate the provider; restart recovery deliberately skips
//!   that tombstone even if the process crashes before ordinary settlement.
//!   Confirmed cancellation can still retry from the process-local injection
//!   queue, but crash recovery favors at-most-once IM delivery over duplication.
//! - **Incognito** wakeups are in-memory only (no row) — close-and-burn.

pub(crate) mod db;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

pub use db::{Wakeup, WakeupDB};

/// Lower bound on the wakeup delay (seconds). **Non-configurable safety floor** —
/// guards against busy-polling; an agent that wants "almost immediately" should
/// just keep working this turn. The configurable upper bound is
/// `async_tools.wakeup_max_delay_secs` (R9, read via [`max_delay_secs`]).
pub const MIN_DELAY_SECS: i64 = 10;
/// Hard ceiling (seconds, 7d) on the *configurable* max wakeup delay. The
/// configured `wakeup_max_delay_secs` is clamped to `[MIN_DELAY_SECS, this]`;
/// guards against zombie timers pinning a session — longer cadences belong to
/// cron.
const MAX_DELAY_CEILING_SECS: i64 = 7 * 86_400;
/// Hard ceiling on the *configurable* per-session pending cap.
const MAX_PENDING_CEILING: usize = 100;

/// Clamp a configured wakeup delay (seconds) to the safe band. Pure + clamps in
/// `u64` space BEFORE the `i64` cast, so a value above `i64::MAX` can't wrap
/// negative and collapse to the floor (it pins to the ceiling, as intended).
fn clamp_wakeup_delay(raw: u64) -> i64 {
    raw.clamp(MIN_DELAY_SECS as u64, MAX_DELAY_CEILING_SECS as u64) as i64
}

/// The configured upper bound (seconds) on a self-scheduled wakeup delay (R9),
/// clamped to `[MIN_DELAY_SECS, MAX_DELAY_CEILING_SECS]`.
pub fn max_delay_secs() -> i64 {
    clamp_wakeup_delay(
        crate::config::cached_config()
            .async_tools
            .wakeup_max_delay_secs,
    )
}

/// The configured per-session cap on pending wakeups (R9, structural reject —
/// exceeding errors, it does NOT queue). Clamped to `[1, MAX_PENDING_CEILING]`
/// (`0` is not "unlimited" — that would let an agent self-schedule a flood of
/// billed turns).
pub fn max_pending_per_session() -> usize {
    crate::config::cached_config()
        .async_tools
        .wakeup_max_pending_per_session
        .clamp(1, MAX_PENDING_CEILING)
}

static WAKEUP_DB: OnceLock<Arc<WakeupDB>> = OnceLock::new();

/// Set the global wakeup DB. Called once during app initialization.
pub fn set_wakeup_db(db: Arc<WakeupDB>) {
    let _ = WAKEUP_DB.set(db);
}

/// Get the global wakeup DB (None until init / if it failed to open).
pub fn get_wakeup_db() -> Option<&'static Arc<WakeupDB>> {
    WAKEUP_DB.get()
}

struct ArmedTimer {
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
    abort: tokio::task::AbortHandle,
}

#[derive(Clone)]
struct SuspendedTimer {
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    fenced_global_stop_epoch: u64,
}

#[derive(Clone)]
struct WakeupDescriptor {
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
}

struct DeliveringWakeup {
    descriptor: WakeupDescriptor,
    /// The latest Stop for this session has fenced this delivery. A subsequent
    /// Continue clears it; a newer Stop clears an older resume request again.
    paused: bool,
    fenced_global_stop_epoch: Option<u64>,
    /// Continue observed this exact in-flight source. If the old injection is
    /// later abandoned by the monotonic Stop epoch, its descriptor must be
    /// re-armed instead of waiting for a process restart.
    resume_requested: bool,
}

/// Live process-local timers, keyed by wakeup id. Used to count per-session
/// pending wakeups (the cap source of truth, covering both persisted and
/// incognito) and to cancel timers on session delete / burn.
static ARMED_TIMERS: LazyLock<Mutex<HashMap<String, ArmedTimer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// In-memory-only wakeups parked by Stop. Durable wakeups remain represented by
/// their database rows; incognito and persistence-degraded wakeups need this
/// process-local receipt so Continue can reconstruct the exact timer.
static SUSPENDED_TIMERS: LazyLock<Mutex<HashMap<String, SuspendedTimer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-process in-flight deliveries, including enough source data to hand an
/// abandoned pre-Stop injection to the post-Continue generation. Keeping the
/// session id here also makes an in-flight incognito wakeup visible to global
/// Stop even though it has neither an armed timer nor a durable database row.
static DELIVERING: LazyLock<Mutex<HashMap<String, DeliveringWakeup>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn count_pending_for_session(session_id: &str) -> usize {
    let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    armed
        .values()
        .filter(|t| t.session_id == session_id)
        .count()
        + suspended
            .values()
            .filter(|t| t.session_id == session_id)
            .count()
        + delivering
            .values()
            .filter(|delivery| delivery.descriptor.session_id == session_id)
            .count()
}

/// Sessions with pending or in-flight wakeups that global Stop must fence.
///
/// Process-local state is captured first so a missing or temporarily locked
/// WakeupDB cannot hide incognito work. SQLite access stays on the blocking
/// pool; a read failure is diagnostic-only and preserves the local snapshot.
pub(crate) async fn pending_session_ids_for_global_stop() -> Vec<String> {
    let mut session_ids = {
        let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
        let mut local = armed
            .values()
            .map(|timer| timer.session_id.clone())
            .collect::<std::collections::HashSet<_>>();
        local.extend(suspended.values().map(|timer| timer.session_id.clone()));
        local.extend(
            delivering
                .values()
                .map(|delivery| delivery.descriptor.session_id.clone()),
        );
        local
    };

    if let Some(db) = get_wakeup_db().cloned() {
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crate::blocking::run_blocking(move || db.list_pending()),
        )
        .await
        {
            Ok(Ok(rows)) => {
                session_ids.extend(rows.into_iter().map(|row| row.session_id));
            }
            Ok(Err(error)) => {
                app_warn!(
                    "wakeup",
                    "global_stop",
                    "Failed to enumerate durable wakeups for global Stop; preserving process-local sessions: {}",
                    error
                );
            }
            Err(_) => {
                app_warn!(
                    "wakeup",
                    "global_stop",
                    "Timed out enumerating durable wakeups for global Stop; preserving process-local sessions"
                );
            }
        }
    }

    let mut session_ids = session_ids.into_iter().collect::<Vec<_>>();
    session_ids.sort();
    session_ids
}

/// Process-local wakeups that have no durable WakeupDB row. The session-free
/// global epoch lets their owner process observe a Stop without publishing
/// incognito timer identity. `paused=true` carries the epoch of the Stop that
/// parked the timer so a Continue handled by another process can re-arm it.
pub(crate) fn volatile_global_stop_states() -> Vec<(String, u64, bool)> {
    let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    let mut states = armed
        .values()
        .filter(|timer| !timer.persisted)
        .map(|timer| {
            (
                timer.session_id.clone(),
                timer.admitted_global_stop_epoch,
                false,
            )
        })
        .collect::<Vec<_>>();
    states.extend(suspended.values().map(|timer| {
        (
            timer.session_id.clone(),
            timer.fenced_global_stop_epoch,
            true,
        )
    }));
    states.extend(
        delivering
            .values()
            .filter(|delivery| !delivery.descriptor.persisted)
            .map(|delivery| {
                (
                    delivery.descriptor.session_id.clone(),
                    delivery
                        .fenced_global_stop_epoch
                        .unwrap_or(delivery.descriptor.admitted_global_stop_epoch),
                    delivery.paused,
                )
            }),
    );
    states
}

/// Resume only process-local volatile wakeups after their durable session
/// receipt has been consumed, regardless of whether this process is Primary.
/// Re-admit each source at the epoch that actually fenced it, never at a live
/// epoch read after Continue: a concurrent newer global Stop must remain able
/// to identify and park the older source.
pub(crate) fn resume_volatile_for_session(session_id: &str) -> usize {
    let mut armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    let ids = suspended
        .iter()
        .filter(|(_, timer)| timer.session_id == session_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &ids {
        if let Some(timer) = suspended.remove(id) {
            arm_timer_locked(
                &mut armed,
                id.clone(),
                timer.session_id,
                timer.agent_id,
                timer.note,
                timer.fire_at,
                false,
                timer.fenced_global_stop_epoch,
                true,
            );
        }
    }
    let mut resumed = ids.len();
    for delivery in delivering.values_mut().filter(|delivery| {
        delivery.descriptor.session_id == session_id
            && !delivery.descriptor.persisted
            && delivery.paused
    }) {
        let admitted_global_stop_epoch = delivery
            .fenced_global_stop_epoch
            .unwrap_or(delivery.descriptor.admitted_global_stop_epoch);
        delivery.paused = false;
        delivery.fenced_global_stop_epoch = None;
        delivery.descriptor.admitted_global_stop_epoch = admitted_global_stop_epoch;
        delivery.resume_requested = true;
        resumed += 1;
    }
    resumed
}

/// Count unique durable or process-local wakeups that still target an Agent.
/// Used by the owner lifecycle plane to prevent disabling a live route and to
/// surface everything deletion will migrate or block.
pub(crate) fn count_pending_for_agent(agent_id: &str) -> anyhow::Result<usize> {
    let mut ids: std::collections::HashSet<String> = {
        let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
        let mut ids = armed
            .iter()
            .filter(|(_, timer)| timer.agent_id == agent_id)
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        ids.extend(
            suspended
                .iter()
                .filter(|(_, timer)| timer.agent_id == agent_id)
                .map(|(id, _)| id.clone()),
        );
        ids.extend(
            delivering
                .iter()
                .filter(|(_, delivery)| delivery.descriptor.agent_id == agent_id)
                .map(|(id, _)| id.clone()),
        );
        ids
    };
    if let Some(db) = get_wakeup_db() {
        ids.extend(
            db.list_pending()?
                .into_iter()
                .filter(|row| row.agent_id == agent_id)
                .map(|row| row.id),
        );
    }
    Ok(ids.len())
}

/// In-memory-only wakeups cannot be durably rebound. They therefore count as
/// active lifecycle work and must fire or be cancelled before Agent deletion.
pub(crate) fn count_unpersisted_for_agent(agent_id: &str) -> usize {
    let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    armed
        .values()
        .filter(|timer| timer.agent_id == agent_id && !timer.persisted)
        .count()
        + suspended
            .values()
            .filter(|timer| timer.agent_id == agent_id)
            .count()
        + delivering
            .values()
            .filter(|delivery| {
                delivery.descriptor.agent_id == agent_id && !delivery.descriptor.persisted
            })
            .count()
}

/// Keep the process-local timer index aligned with a durable lifecycle
/// rewrite. The timer task itself resolves the authoritative row at delivery;
/// this metadata is used for later lifecycle previews and admission checks.
pub(crate) fn update_armed_agent(rows: &[Wakeup], expected_current: &str, replacement: &str) {
    if rows.is_empty() {
        return;
    }
    let ids: std::collections::HashSet<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    let mut timers = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    for (id, timer) in timers.iter_mut() {
        if ids.contains(id.as_str()) && timer.persisted && timer.agent_id == expected_current {
            timer.agent_id = replacement.to_string();
        }
    }
}

/// Outcome of a successful schedule call (returned to the tool layer).
#[derive(Debug, Clone)]
pub struct ScheduleOutcome {
    pub id: String,
    pub fire_at: i64,
    pub delay_secs: i64,
}

/// Why a schedule request was rejected (structural — never queued).
#[derive(Debug)]
pub enum ScheduleError {
    /// The per-session pending cap (`max_pending_per_session`) is reached.
    TooManyPending { limit: usize },
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::TooManyPending { limit } => write!(
                f,
                "too many pending wakeups for this session (limit {limit}); \
                 wait for one to fire or cancel before scheduling another"
            ),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Schedule a one-shot wakeup for `session_id`. `delay_secs` is clamped to
/// `[MIN_DELAY_SECS, max_delay_secs()]` (R9). Persists a row unless `incognito`,
/// then arms a process-local timer. Returns `Err` if the per-session pending cap
/// (`max_pending_per_session()`) is hit (structural reject).
pub fn schedule(
    session_id: &str,
    agent_id: &str,
    delay_secs: i64,
    note: Option<String>,
    incognito: bool,
    admitted_global_stop_epoch: u64,
) -> Result<ScheduleOutcome, ScheduleError> {
    let cap = max_pending_per_session();
    if count_pending_for_session(session_id) >= cap {
        return Err(ScheduleError::TooManyPending { limit: cap });
    }

    let delay = delay_secs.clamp(MIN_DELAY_SECS, max_delay_secs());
    let now = now_secs();
    let fire_at = now.saturating_add(delay);
    let id = format!("wakeup_{}", uuid::Uuid::new_v4().simple());

    let mut persisted = false;
    if !incognito {
        // Best-effort persistence: if the DB is missing we still arm the live
        // timer so the wakeup works this session — it just won't survive a
        // restart (degrades to incognito-like behavior).
        if let Some(db) = get_wakeup_db() {
            let row = Wakeup {
                id: id.clone(),
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                note: note.clone(),
                fire_at,
                created_at: now,
            };
            match db.insert(&row) {
                Ok(()) => persisted = true,
                Err(e) => {
                    app_warn!(
                        "wakeup",
                        "schedule",
                        "Failed to persist wakeup {} (arming in-memory only): {}",
                        id,
                        e
                    );
                }
            }
        }
    }

    arm_timer(
        id.clone(),
        session_id.to_string(),
        agent_id.to_string(),
        note,
        fire_at,
        persisted,
        admitted_global_stop_epoch,
    );

    app_info!(
        "wakeup",
        "schedule",
        "Scheduled wakeup {} for session {} in {}s (incognito={})",
        id,
        session_id,
        delay,
        incognito
    );

    Ok(ScheduleOutcome {
        id,
        fire_at,
        delay_secs: delay,
    })
}

/// Spawn the live timer task and register it in `ARMED_TIMERS`. The map lock is
/// held across `tokio::spawn` so a delay==0 task (past-due replay) can't remove
/// itself before this insert lands (its `remove_armed` blocks on the same lock).
fn arm_timer(
    id: String,
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
) {
    arm_timer_with_resume_hint(
        id,
        session_id,
        agent_id,
        note,
        fire_at,
        persisted,
        admitted_global_stop_epoch,
        false,
    );
}

/// Arm a timer created by explicit Continue. The hint matters only if this
/// timer races an older delivery of the same id: the duplicate timer exits,
/// but transfers the resume request to the older delivery's descriptor.
fn arm_timer_after_continue(
    id: String,
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
) {
    arm_timer_with_resume_hint(
        id,
        session_id,
        agent_id,
        note,
        fire_at,
        persisted,
        admitted_global_stop_epoch,
        true,
    );
}

fn arm_timer_with_resume_hint(
    id: String,
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
    resume_hint: bool,
) {
    let mut map = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    suspended.remove(&id);
    arm_timer_locked(
        &mut map,
        id,
        session_id,
        agent_id,
        note,
        fire_at,
        persisted,
        admitted_global_stop_epoch,
        resume_hint,
    );
}

/// Register a timer while the caller holds the armed-map lock. Keeping Stop,
/// Continue, purge, and lifecycle counts on the same lock order makes movement
/// between armed and suspended states observable as one atomic transition.
fn arm_timer_locked(
    map: &mut HashMap<String, ArmedTimer>,
    id: String,
    session_id: String,
    agent_id: String,
    note: Option<String>,
    fire_at: i64,
    persisted: bool,
    admitted_global_stop_epoch: u64,
    resume_hint: bool,
) {
    let delay = (fire_at - now_secs()).max(0) as u64;
    let task_id = id.clone();
    // Hold the map lock across spawn so a delay==0 task cannot promote itself
    // before this insert lands. Promotion atomically moves the descriptor from
    // ARMED to DELIVERING under that same lock.
    let handle = tokio::spawn(async move {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        if let Some(descriptor) = promote_armed_to_delivering(&task_id, resume_hint) {
            fire(task_id, descriptor).await;
        }
    });
    // Defensive: ids are fresh uuids so a collision shouldn't happen, but if one
    // ever did, abort the displaced timer rather than silently dropping its
    // AbortHandle (which would leak an un-cancellable task).
    if let Some(old) = map.insert(
        id,
        ArmedTimer {
            session_id,
            agent_id,
            note,
            fire_at,
            persisted,
            admitted_global_stop_epoch,
            abort: handle.abort_handle(),
        },
    ) {
        old.abort.abort();
    }
}

fn promote_armed_to_delivering(id: &str, resume_hint: bool) -> Option<WakeupDescriptor> {
    use std::collections::hash_map::Entry;

    let mut armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    let timer = armed.remove(id)?;
    let descriptor = WakeupDescriptor {
        session_id: timer.session_id,
        agent_id: timer.agent_id,
        note: timer.note,
        fire_at: timer.fire_at,
        persisted: timer.persisted,
        admitted_global_stop_epoch: timer.admitted_global_stop_epoch,
    };
    match delivering.entry(id.to_string()) {
        Entry::Vacant(entry) => {
            entry.insert(DeliveringWakeup {
                descriptor: descriptor.clone(),
                paused: false,
                fenced_global_stop_epoch: None,
                // A fresh post-Continue delivery is already the resumed
                // generation. The hint is only transferred to an older
                // occupied generation in the duplicate branch below.
                resume_requested: false,
            });
            Some(descriptor)
        }
        Entry::Occupied(mut entry) => {
            if resume_hint {
                entry.get_mut().resume_requested = true;
            }
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveringFinish {
    Released,
    Parked,
    Rearmed,
}

/// Finish one in-flight source while still entered in its Tokio runtime.
///
/// The three locks follow the module-wide order ARMED -> SUSPENDED ->
/// DELIVERING. Moving/removing the descriptor and optionally arming its next
/// timer is atomic with `purge_for_session`: purge either removes the delivery
/// first, or observes and aborts the replacement timer, so an incognito source
/// cannot resurrect after close-and-burn.
fn finish_delivering(id: &str, abandoned: bool) -> DeliveringFinish {
    let mut armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    let Some(delivery) = delivering.remove(id) else {
        return DeliveringFinish::Released;
    };
    if !abandoned {
        return DeliveringFinish::Released;
    }

    let descriptor = delivery.descriptor;
    if delivery.resume_requested && !delivery.paused {
        arm_timer_locked(
            &mut armed,
            id.to_string(),
            descriptor.session_id,
            descriptor.agent_id,
            descriptor.note,
            descriptor.fire_at,
            descriptor.persisted,
            descriptor.admitted_global_stop_epoch,
            false,
        );
        DeliveringFinish::Rearmed
    } else if !descriptor.persisted {
        // An incognito source has no database row for a Continue that consumes
        // the receipt just after this Abandoned outcome. Park the descriptor so
        // that replay_pending_for_session can still claim it.
        suspended.insert(
            id.to_string(),
            SuspendedTimer {
                session_id: descriptor.session_id,
                agent_id: descriptor.agent_id,
                note: descriptor.note,
                fire_at: descriptor.fire_at,
                fenced_global_stop_epoch: delivery
                    .fenced_global_stop_epoch
                    .unwrap_or(descriptor.admitted_global_stop_epoch),
            },
        );
        DeliveringFinish::Parked
    } else {
        // Durable source remains in WakeupDB for Continue or restart replay.
        DeliveringFinish::Released
    }
}

/// Deliver a fired wakeup: inject a `<wakeup>` message back into the session via
/// the shared injection pipeline. Runs on a detached thread (its own runtime),
/// exactly like `async_jobs::injection::dispatch_injection`.
async fn fire(id: String, descriptor: WakeupDescriptor) {
    let WakeupDescriptor {
        session_id,
        mut agent_id,
        note,
        persisted,
        admitted_global_stop_epoch,
        ..
    } = descriptor;
    let session_db = match crate::get_session_db() {
        Some(db) => db.clone(),
        None => {
            finish_delivering(&id, true);
            return;
        }
    };

    // Durable wakeups are fenced by their shared row/session receipt. A
    // volatile wakeup has no cross-process identity, so compare only the
    // session-free global generation captured when it was admitted. If Stop
    // advanced first, park it under the old epoch; the owner watcher will
    // publish the matching session receipt before updating the parked fence.
    if !persisted {
        let global_stop_epoch = session_db
            .clone()
            .run(|db| db.global_stop_epoch())
            .await
            .unwrap_or(u64::MAX);
        if global_stop_epoch > admitted_global_stop_epoch {
            app_info!(
                "wakeup",
                "fire",
                "Deferring volatile wakeup {} after global Stop generation {}",
                id,
                global_stop_epoch
            );
            suspend_for_session_at_global_epoch(&session_id, None);
            finish_delivering(&id, true);
            return;
        }
    }

    // A session Stop is a durable pause, not a deletion. Leave the wakeup row
    // pending so explicit Continue can re-arm it; never inject an autonomous
    // turn while the pause receipt is active.
    if session_db
        .is_session_or_ancestor_autonomy_paused(&session_id)
        .unwrap_or(true)
    {
        app_info!(
            "wakeup",
            "fire",
            "Deferring wakeup {} while session {} is paused",
            id,
            session_id
        );
        // Treat the pre-injection pause as an abandoned old generation. This
        // atomically parks incognito state, or re-arms if Continue already
        // marked the in-flight descriptor.
        finish_delivering(&id, true);
        return;
    }

    // A lifecycle delete may have rebound this durable wakeup after its live
    // timer was armed. Read the row at delivery time so the task cannot replay
    // into an Agent that has since moved to trash.
    if persisted {
        let Some(db) = get_wakeup_db().cloned() else {
            app_warn!(
                "wakeup",
                "fire",
                "Wakeup {} lost its durable database; leaving it for restart recovery",
                id
            );
            finish_delivering(&id, true);
            return;
        };
        let lookup_id = id.clone();
        match crate::blocking::run_blocking(move || db.get_pending(&lookup_id)).await {
            Ok(Some(row)) => {
                agent_id = row.agent_id;
                if let Some(delivery) = DELIVERING
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_mut(&id)
                {
                    delivery.descriptor.agent_id = agent_id.clone();
                }
            }
            Ok(None) => {
                finish_delivering(&id, false);
                return;
            }
            Err(error) => {
                app_warn!(
                    "wakeup",
                    "fire",
                    "Failed to resolve durable wakeup {} before delivery: {}",
                    id,
                    error
                );
                finish_delivering(&id, true);
                return;
            }
        }
    }

    let push_message = build_wakeup_message(note.as_deref());
    let id_for_mark = id.clone();
    let id_for_release = id.clone();

    std::thread::spawn(move || {
        /// Releases a freshly promoted descriptor if setup unwinds before an
        /// OnInjected receipt owns it. A normal injection outcome transfers
        /// the claim to that clone-stable receipt, including FIFO deferral.
        struct DeliverGuard {
            id: String,
            armed: bool,
        }

        impl DeliverGuard {
            fn new(id: String) -> Self {
                Self { id, armed: true }
            }

            fn transfer_to_receipt(&mut self) {
                self.armed = false;
            }
        }

        impl Drop for DeliverGuard {
            fn drop(&mut self) {
                if self.armed {
                    release_delivering(&self.id);
                }
            }
        }
        let mut guard = DeliverGuard::new(id_for_release);

        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let id_for_arm = id_for_mark.clone();
                let id_for_settle = id_for_mark.clone();
                let id_for_process_release = id_for_mark.clone();
                let on_injected = crate::subagent::injection::OnInjected::new(
                    move || {
                        if persisted {
                            claim_no_replay_with_retry(&id_for_arm)?;
                        }
                        finish_delivering(&id_for_arm, false);
                        Ok(())
                    },
                    move || {
                        if persisted {
                            delete_delivered_with_retry(&id_for_settle)?;
                        }
                        finish_delivering(&id_for_settle, false);
                        Ok(())
                    },
                )
                .with_process_dispatch_release(move || {
                    finish_delivering(&id_for_process_release, true);
                });
                let release_receipt = on_injected.clone();
                let receipt_run_id = id_for_mark.clone();
                let outcome = rt.block_on(async move {
                    let outcome = crate::subagent::injection::inject_and_run_parent(
                        session_id,
                        agent_id,
                        crate::subagent::injection::WAKEUP_CHILD_AGENT_ID.to_string(),
                        id,
                        push_message,
                        session_db,
                        Some(on_injected),
                    )
                    .await;
                    if matches!(
                        outcome,
                        crate::subagent::injection::InjectionOutcome::Abandoned
                    ) {
                        crate::subagent::injection::release_unarmed_injection_source(
                            Some(&release_receipt),
                            &receipt_run_id,
                        );
                    }
                    outcome
                });
                // Queued attempts retain the exact descriptor through the
                // receipt cloned into PendingInjection. Successful arm/settle
                // callbacks already released it; an Abandoned attempt released
                // it through the process-dispatch callback above. A failed DB
                // callback intentionally keeps the claim pinned until restart.
                guard.transfer_to_receipt();
                if matches!(
                    outcome,
                    crate::subagent::injection::InjectionOutcome::Abandoned
                ) {
                    app_info!(
                        "wakeup",
                        "fire",
                        "Wakeup {} abandoned; source returned to its Stop/Continue recovery state",
                        id_for_mark
                    );
                }
            }
            Err(e) => app_error!("wakeup", "fire", "Failed to build runtime: {}", e),
        }
    });
}

fn release_delivering(id: &str) {
    DELIVERING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id);
}

/// Delete a delivered wakeup row with retry — mirrors `async_jobs::injection::
/// mark_injected_with_retry`'s robustness. Ordinary delivery deletes and
/// auto-GCs the row; an IM write-ahead claim first sets `fired=1`, so even a
/// failed delete remains excluded from restart replay. Without an IM claim, a
/// silently-swallowed delete failure could still cause a duplicate billed turn,
/// so retry transient SQLite errors and log loudly if all fail.
fn delete_delivered_with_retry(id: &str) -> anyhow::Result<()> {
    const BACKOFFS_MS: &[u64] = &[0, 100, 500, 2_000];
    let Some(db) = get_wakeup_db() else {
        app_error!(
            "wakeup",
            "fire",
            "Cannot delete delivered wakeup {}: wakeup DB not initialized (may re-fire on restart)",
            id
        );
        anyhow::bail!("wakeup DB is not initialized");
    };
    let mut last_err: Option<String> = None;
    for (attempt, delay_ms) in BACKOFFS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
        }
        match db.delete(id) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e.to_string());
                app_warn!(
                    "wakeup",
                    "fire",
                    "delete delivered wakeup {} attempt {} failed: {}",
                    id,
                    attempt + 1,
                    e
                );
            }
        }
    }
    let error = last_err.unwrap_or_else(|| "unknown".to_string());
    app_error!(
        "wakeup",
        "fire",
        "delete delivered wakeup {} failed after all retries ({}); it may re-fire (duplicate turn) on next Primary restart",
        id,
        &error
    );
    anyhow::bail!("delete delivered wakeup failed after retries: {error}")
}

/// Persist the IM at-most-once fence before the parent engine can emit a
/// provider-visible delta. A false CAS means another process already owns the
/// wakeup and this attempt must abort locally.
fn claim_no_replay_with_retry(id: &str) -> anyhow::Result<()> {
    const BACKOFFS_MS: &[u64] = &[0, 100, 500, 2_000];
    let Some(db) = get_wakeup_db() else {
        anyhow::bail!("wakeup DB is not initialized");
    };
    let mut last_err: Option<String> = None;
    for (attempt, delay_ms) in BACKOFFS_MS.iter().enumerate() {
        if *delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
        }
        match db.claim_no_replay(id) {
            Ok(true) => return Ok(()),
            Ok(false) => {
                anyhow::bail!("wakeup {id} is already fenced by another IM injection owner")
            }
            Err(error) => {
                last_err = Some(error.to_string());
                app_warn!(
                    "wakeup",
                    "fire",
                    "claim no-replay wakeup {} attempt {} failed: {}",
                    id,
                    attempt + 1,
                    error
                );
            }
        }
    }
    anyhow::bail!(
        "claim no-replay wakeup failed after retries: {}",
        last_err.unwrap_or_else(|| "unknown".to_string())
    )
}

/// Build the injected `<wakeup>` user message carrying the agent's own note.
pub(crate) fn build_wakeup_message(note: Option<&str>) -> String {
    let note_block = note
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("<note>\n{}\n</note>\n", escape_xml(s)))
        .unwrap_or_default();
    format!(
        "<wakeup>\n\
         A wakeup you scheduled earlier has fired. Continue the work you set this \
         timer for. Your note to self:\n\
         {note_block}\
         </wakeup>"
    )
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Re-arm unfired wakeups after a restart. **Primary-only** — call sites are
/// gated by `runtime_lock::is_primary()`. Past-due wakeups fire promptly.
pub fn replay_pending() {
    let Some(db) = get_wakeup_db() else {
        return;
    };
    let pending = match db.list_pending() {
        Ok(p) => p,
        Err(e) => {
            app_error!("wakeup", "replay", "Failed to list pending wakeups: {}", e);
            return;
        }
    };
    // Respect the per-session cap on re-arm. The in-memory cap (counted over
    // ARMED_TIMERS at schedule time) can drift below the persisted count — an
    // Abandoned firing drops the in-memory timer but leaves the row — letting a
    // session accumulate more persisted rows than the cap. Bound it here: rows
    // are ordered fire_at ASC, so the soonest survive; over-cap rows are dropped
    // (the configured cap is the policy).
    let cap = max_pending_per_session();
    let mut per_session: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut armed = 0usize;
    let mut dropped = 0usize;
    for w in pending {
        if crate::get_session_db()
            .and_then(|session_db| {
                session_db
                    .is_session_or_ancestor_autonomy_paused(&w.session_id)
                    .ok()
            })
            .unwrap_or(true)
        {
            continue;
        }
        let c = per_session.entry(w.session_id.clone()).or_insert(0);
        if *c >= cap {
            let _ = db.delete(&w.id);
            dropped += 1;
            continue;
        }
        *c += 1;
        arm_timer(w.id, w.session_id, w.agent_id, w.note, w.fire_at, true, 0);
        armed += 1;
    }
    if armed > 0 || dropped > 0 {
        app_info!(
            "wakeup",
            "replay",
            "Re-armed {} pending wakeup(s); dropped {} over per-session cap",
            armed,
            dropped
        );
    }
}

/// Abort live timers for a stopped session without deleting durable rows.
/// Continue re-arms those exact rows; session deletion still uses
/// [`purge_for_session`] and removes them permanently.
pub fn suspend_for_session(session_id: &str) -> usize {
    suspend_for_session_at_global_epoch(session_id, None)
}

pub(crate) fn suspend_for_session_at_global_epoch(
    session_id: &str,
    global_stop_epoch: Option<u64>,
) -> usize {
    let mut map = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut suspended = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
    let mut delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
    let ids = map
        .iter()
        .filter(|(_, timer)| timer.session_id == session_id)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in &ids {
        if let Some(timer) = map.remove(id) {
            if !timer.persisted {
                suspended.insert(
                    id.clone(),
                    SuspendedTimer {
                        session_id: timer.session_id,
                        agent_id: timer.agent_id,
                        note: timer.note,
                        fire_at: timer.fire_at,
                        fenced_global_stop_epoch: global_stop_epoch
                            .unwrap_or(timer.admitted_global_stop_epoch),
                    },
                );
            }
            timer.abort.abort();
        }
    }
    let mut affected = ids.len();
    for timer in suspended
        .values_mut()
        .filter(|timer| timer.session_id == session_id)
    {
        if let Some(epoch) = global_stop_epoch {
            timer.fenced_global_stop_epoch = epoch;
        }
    }
    for delivery in delivering
        .values_mut()
        .filter(|delivery| delivery.descriptor.session_id == session_id)
    {
        if !delivery.paused {
            affected += 1;
        }
        delivery.paused = true;
        delivery.fenced_global_stop_epoch =
            Some(global_stop_epoch.unwrap_or(delivery.descriptor.admitted_global_stop_epoch));
        // A newer Stop supersedes any Continue that targeted the prior pause
        // generation. The next explicit Continue may set this again.
        delivery.resume_requested = false;
    }
    affected
}

/// Re-arm wakeups for one explicitly continued session. **Primary-only**:
/// Secondary adapters may consume the shared pause receipt, but must not arm a
/// second process-local timer for a durable row already owned by the Primary.
pub async fn replay_pending_for_session(session_id: &str) -> anyhow::Result<()> {
    replay_pending_for_session_for_tier(session_id, crate::runtime_lock::is_primary()).await
}

async fn replay_pending_for_session_for_tier(
    session_id: &str,
    primary: bool,
) -> anyhow::Result<()> {
    resume_volatile_for_session(session_id);
    if !primary {
        app_debug!(
            "wakeup",
            "resume",
            "Skipped wakeup replay for continued session {} on Secondary; Primary owns durable replay",
            session_id
        );
        return Ok(());
    }

    // Mark in-flight sources before looking at parked/durable state. If an old
    // generation settles concurrently, the fixed lock order makes it either
    // observe this flag and re-arm itself, or move to SUSPENDED/DB before the
    // snapshots below.
    let delivering_ids = {
        let mut delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
        delivering
            .iter_mut()
            .filter_map(|(id, delivery)| {
                (delivery.descriptor.session_id == session_id && delivery.descriptor.persisted)
                    .then(|| {
                        delivery.paused = false;
                        delivery.fenced_global_stop_epoch = None;
                        delivery.resume_requested = true;
                        id.clone()
                    })
            })
            .collect::<std::collections::HashSet<_>>()
    };
    let Some(db) = get_wakeup_db().cloned() else {
        return Ok(());
    };
    let pending = match crate::blocking::run_blocking(move || db.list_pending()).await {
        Ok(rows) => rows,
        Err(error) => {
            app_warn!(
                "wakeup",
                "resume",
                "Failed to list wakeups for continued session {}: {}",
                session_id,
                error
            );
            return Err(anyhow::anyhow!(error));
        }
    };
    for wakeup in pending
        .into_iter()
        .filter(|wakeup| wakeup.session_id == session_id && !delivering_ids.contains(&wakeup.id))
    {
        arm_timer_after_continue(
            wakeup.id,
            wakeup.session_id,
            wakeup.agent_id,
            wakeup.note,
            wakeup.fire_at,
            true,
            0,
        );
    }
    Ok(())
}

/// Cancel & delete all wakeups for a session (session delete / incognito burn).
pub fn purge_for_session(session_id: &str) {
    // Abort live timers and consume both Stop receipts and in-flight delivery
    // descriptors atomically. The lock order matches finish_delivering, so a
    // late Abandoned outcome cannot re-arm an incognito timer after burn.
    let (aborted, suspended, delivering): (Vec<String>, Vec<String>, Vec<String>) = {
        let mut map = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let mut suspended_map = SUSPENDED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let mut delivering_map = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
        let ids: Vec<String> = map
            .iter()
            .filter(|(_, t)| t.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            if let Some(t) = map.remove(id) {
                t.abort.abort();
            }
        }
        let suspended_ids = suspended_map
            .iter()
            .filter(|(_, timer)| timer.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &suspended_ids {
            suspended_map.remove(id);
        }
        let delivering_ids = delivering_map
            .iter()
            .filter(|(_, delivery)| delivery.descriptor.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &delivering_ids {
            delivering_map.remove(id);
        }
        (ids, suspended_ids, delivering_ids)
    };
    if let Some(db) = get_wakeup_db() {
        if let Err(e) = db.delete_for_session(session_id) {
            app_warn!(
                "wakeup",
                "purge",
                "Failed to delete wakeups for session {}: {}",
                session_id,
                e
            );
        }
    }
    if !aborted.is_empty() || !suspended.is_empty() || !delivering.is_empty() {
        app_info!(
            "wakeup",
            "purge",
            "Cancelled {} wakeup(s) for session {}",
            aborted.len() + suspended.len() + delivering.len(),
            session_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_wakeup_message_escapes_and_wraps_note() {
        let msg = build_wakeup_message(Some("check <CI> & retry"));
        assert!(msg.starts_with("<wakeup>"));
        assert!(msg.contains("<note>\ncheck &lt;CI&gt; &amp; retry\n</note>"));
        assert!(msg.trim_end().ends_with("</wakeup>"));
    }

    #[test]
    fn build_wakeup_message_handles_empty_note() {
        let msg = build_wakeup_message(None);
        assert!(msg.starts_with("<wakeup>"));
        assert!(!msg.contains("<note>"));
    }

    #[tokio::test]
    async fn schedule_clamps_delay_and_enforces_per_session_cap() {
        // Unique session id isolates the global ARMED_TIMERS count from other
        // parallel tests. Incognito → no DB needed (in-memory timers only).
        let sid = "test-wakeup-cap-session";
        purge_for_session(sid); // ensure clean slate

        // Sub-minimum delay is clamped up to MIN_DELAY_SECS (not busy-polled).
        let out = schedule(sid, "ha-main", 2, Some("note".into()), true, 0).unwrap();
        assert_eq!(out.delay_secs, MIN_DELAY_SECS);

        // Fill to the configured cap (we already armed 1). Reading the live cap
        // (not a hardcoded 5) keeps the test correct whatever config is loaded.
        let cap = max_pending_per_session();
        for _ in 1..cap {
            schedule(sid, "ha-main", 60, None, true, 0).unwrap();
        }
        assert_eq!(count_pending_for_session(sid), cap);

        // One past the cap is a structural reject (not queued).
        let err = schedule(sid, "ha-main", 60, None, true, 0).unwrap_err();
        assert!(matches!(err, ScheduleError::TooManyPending { .. }));

        // Purge aborts every live timer and frees the session's budget.
        purge_for_session(sid);
        assert_eq!(count_pending_for_session(sid), 0);
        // Scheduling works again after purge.
        schedule(sid, "ha-main", 60, None, true, 0).unwrap();
        assert_eq!(count_pending_for_session(sid), 1);
        purge_for_session(sid); // leave no lingering timers for sibling tests
    }

    #[tokio::test]
    async fn schedule_clamps_oversized_delay_to_max() {
        let sid = "test-wakeup-maxclamp-session";
        purge_for_session(sid);
        let max = max_delay_secs();
        let out = schedule(sid, "ha-main", max + 10_000, None, true, 0).unwrap();
        assert_eq!(out.delay_secs, max);
        purge_for_session(sid);
    }

    #[tokio::test]
    async fn secondary_continue_rearms_owner_local_volatile_wakeup() {
        let sid = "test-wakeup-secondary-continue-session";
        let id = "test-wakeup-secondary-continue-id";
        purge_for_session(sid);
        SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id.into(),
                SuspendedTimer {
                    session_id: sid.into(),
                    agent_id: "ha-main".into(),
                    note: Some("Primary owns replay".into()),
                    fire_at: now_secs() + 60,
                    fenced_global_stop_epoch: 0,
                },
            );

        replay_pending_for_session_for_tier(sid, false)
            .await
            .expect("Secondary resumes its process-local wakeup");

        assert!(!SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));
        assert!(ARMED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));
        purge_for_session(sid);
    }

    #[tokio::test]
    async fn incognito_wakeup_survives_suspend_and_replay() {
        let sid = "test-wakeup-incognito-pause-session";
        purge_for_session(sid);

        let scheduled = schedule(
            sid,
            "ha-main",
            60,
            Some("resume the original check".into()),
            true,
            0,
        )
        .expect("schedule incognito wakeup");

        assert_eq!(suspend_for_session(sid), 1);
        assert!(!ARMED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&scheduled.id));
        let suspended_note = SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&scheduled.id)
            .and_then(|timer| timer.note.as_deref())
            .map(str::to_string);
        assert_eq!(suspended_note.as_deref(), Some("resume the original check"));
        assert_eq!(count_pending_for_session(sid), 1);

        replay_pending_for_session_for_tier(sid, true)
            .await
            .expect("replay incognito wakeup");
        assert!(ARMED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&scheduled.id));
        assert!(!SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&scheduled.id));

        purge_for_session(sid);
        assert_eq!(count_pending_for_session(sid), 0);
    }

    #[tokio::test]
    async fn volatile_wakeup_tracks_global_stop_and_continue_generations() {
        let sid = "test-wakeup-volatile-global-generation-session";
        purge_for_session(sid);
        let scheduled =
            schedule(sid, "ha-main", 60, None, true, 7).expect("schedule volatile wakeup");

        assert!(volatile_global_stop_states()
            .iter()
            .any(|state| state == &(sid.to_string(), 7, false)));

        assert_eq!(suspend_for_session_at_global_epoch(sid, Some(8)), 1);
        assert!(volatile_global_stop_states()
            .iter()
            .any(|state| state == &(sid.to_string(), 8, true)));

        assert_eq!(resume_volatile_for_session(sid), 1);
        let armed = ARMED_TIMERS.lock().unwrap_or_else(|p| p.into_inner());
        let timer = armed.get(&scheduled.id).expect("continued wakeup is armed");
        assert_eq!(timer.admitted_global_stop_epoch, 8);
        drop(armed);

        purge_for_session(sid);
    }

    #[tokio::test]
    async fn abandoned_delivery_rearms_after_continue() {
        let sid = "test-wakeup-delivery-continue-session";
        let id = "test-wakeup-delivery-continue-id";
        purge_for_session(sid);
        DELIVERING.lock().unwrap_or_else(|p| p.into_inner()).insert(
            id.into(),
            DeliveringWakeup {
                descriptor: WakeupDescriptor {
                    session_id: sid.into(),
                    agent_id: "ha-main".into(),
                    note: Some("continue the exact wakeup".into()),
                    fire_at: now_secs() + 60,
                    persisted: false,
                    admitted_global_stop_epoch: 0,
                },
                paused: true,
                fenced_global_stop_epoch: None,
                resume_requested: false,
            },
        );

        replay_pending_for_session_for_tier(sid, true)
            .await
            .expect("resume in-flight wakeup");
        {
            let delivering = DELIVERING.lock().unwrap_or_else(|p| p.into_inner());
            let delivery = delivering.get(id).expect("delivery remains in flight");
            assert!(!delivery.paused);
            assert!(delivery.resume_requested);
        }

        assert_eq!(finish_delivering(id, true), DeliveringFinish::Rearmed);
        assert!(ARMED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));
        assert!(!DELIVERING
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));
        assert!(!SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));

        purge_for_session(sid);
    }

    #[tokio::test]
    async fn purge_invalidates_delivering_resume_request() {
        let sid = "test-wakeup-delivery-purge-session";
        let id = "test-wakeup-delivery-purge-id";
        purge_for_session(sid);
        DELIVERING.lock().unwrap_or_else(|p| p.into_inner()).insert(
            id.into(),
            DeliveringWakeup {
                descriptor: WakeupDescriptor {
                    session_id: sid.into(),
                    agent_id: "ha-main".into(),
                    note: None,
                    fire_at: now_secs() + 60,
                    persisted: false,
                    admitted_global_stop_epoch: 0,
                },
                paused: false,
                fenced_global_stop_epoch: None,
                resume_requested: true,
            },
        );

        purge_for_session(sid);
        assert_eq!(finish_delivering(id, true), DeliveringFinish::Released);
        assert_eq!(count_pending_for_session(sid), 0);
    }

    #[tokio::test]
    async fn global_stop_enumerates_inflight_wakeup() {
        let sid = "test-wakeup-global-stop-delivery-session";
        let id = "test-wakeup-global-stop-delivery-id";
        purge_for_session(sid);
        DELIVERING.lock().unwrap_or_else(|p| p.into_inner()).insert(
            id.into(),
            DeliveringWakeup {
                descriptor: WakeupDescriptor {
                    session_id: sid.into(),
                    agent_id: "ha-main".into(),
                    note: None,
                    fire_at: now_secs() + 60,
                    persisted: false,
                    admitted_global_stop_epoch: 0,
                },
                paused: false,
                fenced_global_stop_epoch: None,
                resume_requested: false,
            },
        );

        assert!(pending_session_ids_for_global_stop()
            .await
            .iter()
            .any(|candidate| candidate == sid));

        purge_for_session(sid);
    }

    #[tokio::test]
    async fn newer_stop_cancels_older_delivery_resume_request() {
        let sid = "test-wakeup-delivery-restop-session";
        let id = "test-wakeup-delivery-restop-id";
        purge_for_session(sid);
        DELIVERING.lock().unwrap_or_else(|p| p.into_inner()).insert(
            id.into(),
            DeliveringWakeup {
                descriptor: WakeupDescriptor {
                    session_id: sid.into(),
                    agent_id: "ha-main".into(),
                    note: None,
                    fire_at: now_secs() + 60,
                    persisted: false,
                    admitted_global_stop_epoch: 0,
                },
                paused: true,
                fenced_global_stop_epoch: None,
                resume_requested: false,
            },
        );

        replay_pending_for_session_for_tier(sid, true)
            .await
            .expect("resume in-flight wakeup");
        assert_eq!(suspend_for_session(sid), 1);
        assert_eq!(finish_delivering(id, true), DeliveringFinish::Parked);
        assert!(!ARMED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));
        assert!(SUSPENDED_TIMERS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id));

        purge_for_session(sid);
    }

    #[test]
    fn configured_bounds_are_clamped_to_safe_bands() {
        // R9: whatever the loaded config holds, the live bounds stay sane —
        // delay floored at MIN, ceiled at 7d; pending cap in [1, 100].
        let max = max_delay_secs();
        assert!((MIN_DELAY_SECS..=MAX_DELAY_CEILING_SECS).contains(&max));
        let cap = max_pending_per_session();
        assert!((1..=MAX_PENDING_CEILING).contains(&cap));
    }

    #[test]
    fn clamp_wakeup_delay_pins_huge_values_to_ceiling_not_floor() {
        // Review fix: clamp in u64 space before the i64 cast. A value above
        // i64::MAX must pin to the 7d ceiling (the user wants "very long"), NOT
        // wrap negative and collapse to the 10s floor.
        assert_eq!(clamp_wakeup_delay(0), MIN_DELAY_SECS);
        assert_eq!(clamp_wakeup_delay(3600), 3600);
        assert_eq!(
            clamp_wakeup_delay(MAX_DELAY_CEILING_SECS as u64),
            MAX_DELAY_CEILING_SECS
        );
        assert_eq!(clamp_wakeup_delay(u64::MAX), MAX_DELAY_CEILING_SECS);
        assert_eq!(
            clamp_wakeup_delay(i64::MAX as u64 + 1),
            MAX_DELAY_CEILING_SECS
        );
    }
}

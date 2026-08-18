//! Shared foreground-session Stop orchestration.
//!
//! Transport/UI entry points may differ in how they identify a turn, but once
//! a session target is known they must converge through this module so Desktop,
//! HTTP, and IM `/stop` resolve interaction waits and owned runtime work with
//! the same semantics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::{collections::HashSet, future::Future};

use crate::runtime_tasks::CancelRuntimeTaskResult;
use crate::session::{ChatTurnInterruptReason, ChatTurnStatus, SessionDB};
use crate::tools::ApprovalResolutionSource;

const STOP_DB_MARK_TIMEOUT: Duration = Duration::from_secs(2);
const PRE_TURN_QUEUE_RELEASE_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_INTERACTION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_RUNTIME_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const AUTONOMY_STOP_FENCE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const AUTONOMY_RESUME_REPLAY_INTERVAL: Duration = Duration::from_secs(2);
const AUTONOMY_RESUME_REPLAY_BATCH_SIZE: usize = 100;

/// Serializes durable Stop generations and exact Continue consumption. The DB
/// transaction creates the durable ordering; this process-local lock also
/// keeps controller state convergence from interleaving around that ordering.
static AUTONOMY_TRANSITION_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
static PENDING_AUTONOMY_STOPS: OnceLock<Mutex<PendingAutonomyStops>> = OnceLock::new();
static AUTONOMY_STOP_FENCE_WATCHER_STARTED: AtomicBool = AtomicBool::new(false);
static AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);
static AUTONOMY_RESUME_REPLAY_LOOP_STARTED: AtomicBool = AtomicBool::new(false);
static AUTONOMY_RESUME_REPLAY_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct PendingAutonomyStops {
    global: usize,
    sessions: std::collections::HashMap<String, usize>,
}

struct SessionAutonomyStopClaim {
    session_id: String,
}

impl Drop for SessionAutonomyStopClaim {
    fn drop(&mut self) {
        let mut pending = pending_autonomy_stops()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = pending.sessions.get_mut(&self.session_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                pending.sessions.remove(&self.session_id);
            }
        }
    }
}

struct GlobalAutonomyStopClaim;

impl Drop for GlobalAutonomyStopClaim {
    fn drop(&mut self) {
        let mut pending = pending_autonomy_stops()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.global = pending.global.saturating_sub(1);
    }
}

fn autonomy_transition_lock() -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(AUTONOMY_TRANSITION_LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(()))))
}

fn pending_autonomy_stops() -> &'static Mutex<PendingAutonomyStops> {
    PENDING_AUTONOMY_STOPS.get_or_init(|| Mutex::new(PendingAutonomyStops::default()))
}

fn begin_session_autonomy_stop(session_id: &str) -> SessionAutonomyStopClaim {
    let mut pending = pending_autonomy_stops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *pending.sessions.entry(session_id.to_string()).or_default() += 1;
    SessionAutonomyStopClaim {
        session_id: session_id.to_string(),
    }
}

fn begin_global_autonomy_stop() -> GlobalAutonomyStopClaim {
    let mut pending = pending_autonomy_stops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.global += 1;
    GlobalAutonomyStopClaim
}

fn autonomy_stop_pending(session_id: &str) -> bool {
    let pending = pending_autonomy_stops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.global > 0 || pending.sessions.get(session_id).copied().unwrap_or(0) > 0
}

#[derive(Debug, Default)]
pub struct StopSessionOutcome {
    pub stopped: bool,
    pub turn_mismatch: bool,
    /// The active-turn registry still held an entry for this session. `false`
    /// proves the backend had no live foreground turn to stop, which is the
    /// signal a stale UI needs to converge itself: no terminal stream event can
    /// arrive for a turn that is already durable-terminal.
    pub active_turn_found: bool,
    /// The executor crossed its cancellation point before this Stop claimed the
    /// registry. Semantically distinct from a stale/vanished turn: a terminal
    /// event is still on its way, so callers must keep waiting.
    pub completion_sealed: bool,
    /// This Stop armed the `cancelling` broadcast plus the durable watchdog for
    /// a user-visible turn, so a terminal stream event will follow. When it is
    /// `false` and nothing was sealed, no event can ever arrive and a UI still
    /// showing activity must reconcile itself against the authoritative state.
    pub terminal_event_pending: bool,
    pub denied_approvals: usize,
    pub cancelled_questions: usize,
    pub runtime_cancellations: Vec<CancelRuntimeTaskResult>,
    pub runtime_cancellation_error: Option<String>,
    pub autonomy_pause: Option<crate::session::SessionAutonomyPause>,
    pub autonomy_pause_error: Option<String>,
}

/// Wire shape shared by the Tauri command and `POST /api/chat/stop`. Stop
/// reports only what this call did; "what is the session doing now" stays the
/// sole responsibility of `get_session_stream_state` so the two cannot drift.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopChatResult {
    pub stopped: bool,
    /// `request` | `session` | `all`.
    pub scope: String,
    pub reason: Option<String>,
    pub turn_mismatch: bool,
    pub active_turn_found: bool,
    pub completion_sealed: bool,
    pub terminal_event_pending: bool,
    /// A pre-registration latch consumed this Stop. The turn has not announced
    /// itself yet, so an idle-looking backend here is expected and callers must
    /// not treat it as stale state.
    pub latched: bool,
    pub runtime_cancellations: Vec<CancelRuntimeTaskResult>,
    pub runtime_cancellation_error: Option<String>,
    pub autonomy_paused: bool,
    pub autonomy_pause: Option<crate::session::SessionAutonomyPause>,
    pub autonomy_pause_error: Option<String>,
    /// Global Stop only: how many sessions were settled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped_session_count: Option<usize>,
}

impl StopChatResult {
    /// Terminal outcome for a Stop that found no target at all.
    pub fn no_target(scope: &str, reason: Option<&str>) -> Self {
        Self {
            scope: scope.to_string(),
            reason: reason.map(str::to_string),
            ..Default::default()
        }
    }

    /// The opaque client request was latched before its turn registered.
    pub fn latched() -> Self {
        Self {
            stopped: true,
            scope: "request".to_string(),
            latched: true,
            ..Default::default()
        }
    }

    pub fn from_session_outcome(scope: &str, outcome: StopSessionOutcome) -> Self {
        Self {
            stopped: outcome.stopped,
            scope: scope.to_string(),
            reason: (!outcome.stopped).then(|| "no matching active chat for target".to_string()),
            turn_mismatch: outcome.turn_mismatch,
            active_turn_found: outcome.active_turn_found,
            completion_sealed: outcome.completion_sealed,
            terminal_event_pending: outcome.terminal_event_pending,
            latched: false,
            runtime_cancellations: outcome.runtime_cancellations,
            runtime_cancellation_error: outcome.runtime_cancellation_error,
            autonomy_paused: outcome.autonomy_pause.is_some(),
            autonomy_pause: outcome.autonomy_pause,
            autonomy_pause_error: outcome.autonomy_pause_error,
            stopped_session_count: None,
        }
    }

    pub fn from_all_outcome(outcome: StopAllOutcome) -> Self {
        Self {
            stopped: outcome.stopped,
            scope: "all".to_string(),
            reason: (!outcome.stopped).then(|| "no running chat".to_string()),
            // A global Stop never targets one turn, so the exact-turn fields
            // stay false and callers fall back to the session-scoped reconcile.
            runtime_cancellations: outcome.runtime_cancellations,
            runtime_cancellation_error: outcome.runtime_cancellation_error,
            stopped_session_count: Some(outcome.stopped_session_count),
            ..Default::default()
        }
    }
}

fn prepare_and_pause_session_autonomy_blocking(
    db: &SessionDB,
    session_id: &str,
    global_stop_epoch: Option<u64>,
) -> anyhow::Result<crate::session::SessionAutonomyPause> {
    // The receipt lands first and is the durable restart fence. Controller
    // transitions then make the live process converge promptly.
    let pause = match global_stop_epoch {
        Some(epoch) => db.prepare_session_autonomy_pause_for_global(session_id, epoch)?,
        None => db.prepare_session_autonomy_pause(session_id)?,
    };
    if let Some(goal_id) = pause.goal_id.as_deref() {
        if let Some(goal) = db.get_goal(goal_id)? {
            if goal.state != crate::goal::GoalState::Paused && !goal.state.is_terminal() {
                db.pause_goal(goal_id)?;
            }
        }
    }
    for run_id in &pause.workflow_run_ids {
        if let Some(run) = db.get_workflow_run(run_id)? {
            if run.state != crate::workflow::WorkflowRunState::Paused && !run.state.is_terminal() {
                db.pause_workflow_run(run_id)?;
            }
        }
        crate::workflow::request_workflow_runtime_pause(run_id);
    }
    for owned_session_id in db.list_session_autonomy_tree_ids(&pause.session_id)? {
        crate::wakeup::suspend_for_session_at_global_epoch(&owned_session_id, global_stop_epoch);
        crate::subagent::request_pause_parent_injection(&owned_session_id);
    }
    Ok(pause)
}

async fn prepare_and_pause_session_autonomy(
    db: Arc<SessionDB>,
    session_id: String,
    cleanup_gate: Arc<super::active_turn::StopCleanupGuard>,
    stop_claim: Arc<SessionAutonomyStopClaim>,
) -> anyhow::Result<Option<crate::session::SessionAutonomyPause>> {
    // Detach the transition task from the bounded transport wait. If timeout
    // drops this JoinHandle while it is waiting for another Continue/Stop, the
    // task still owns the admission gate and must eventually publish this
    // newer Stop generation.
    tokio::spawn(async move {
        let transition_guard = autonomy_transition_lock().lock_owned().await;
        db.run(move |db| {
            // `SessionDB::run` uses spawn_blocking, which cannot be cancelled
            // by dropping an async waiter. Keep both guards in the blocking
            // closure so a late completion cannot pause a replacement turn.
            let _cleanup_gate = cleanup_gate;
            let _stop_claim = stop_claim;
            let _transition_guard = transition_guard;
            prepare_and_pause_session_autonomy_blocking(db, &session_id, None).map(Some)
        })
        .await
    })
    .await
    .map_err(|error| anyhow::anyhow!("session autonomy pause task failed: {error}"))?
}

async fn recover_not_found_durable_turn(
    db: Arc<SessionDB>,
    session_id: &str,
    expected_turn_id: Option<&str>,
) -> Option<(String, crate::chat_engine::ChatSource)> {
    let turn_id = expected_turn_id?.to_string();
    let lookup_turn_id = turn_id.clone();
    let turn = match tokio::time::timeout(
        STOP_DB_MARK_TIMEOUT,
        db.run(move |db| db.get_chat_turn(&lookup_turn_id)),
    )
    .await
    {
        Ok(Ok(Some(turn))) => turn,
        Ok(Ok(None)) => return None,
        Ok(Err(error)) => {
            crate::app_warn!(
                "chat",
                "stop_session",
                "Failed to resolve vanished active turn {} for session {}: {}",
                turn_id,
                session_id,
                error
            );
            return None;
        }
        Err(_) => {
            crate::app_warn!(
                "chat",
                "stop_session",
                "Timed out resolving vanished active turn {} for session {}",
                turn_id,
                session_id
            );
            return None;
        }
    };
    if turn.session_id != session_id || turn.status.is_terminal() {
        return None;
    }
    Some((
        turn_id,
        crate::chat_engine::ChatSource::from_db_string(&turn.source),
    ))
}

/// Resume only the controllers captured by the supplied durable session Stop.
/// Manually paused Goal / Workflow work is absent from the receipt, and an old
/// pause id cannot consume the user's newer Stop decision.
pub async fn continue_session(
    db: Arc<SessionDB>,
    session_id: &str,
    pause_id: &str,
) -> anyhow::Result<crate::session::SessionAutonomyResumeOutcome> {
    continue_session_inner_for_tier(db, session_id, pause_id, crate::runtime_lock::is_primary())
        .await
}

/// Model-facing Continue binds the decision to the exact receipt included in
/// that round's trusted system reminder. A stale tool call must not consume a
/// different Stop generation.
pub(crate) async fn continue_session_from_pause(
    db: Arc<SessionDB>,
    session_id: &str,
    pause_id: &str,
) -> anyhow::Result<crate::session::SessionAutonomyResumeOutcome> {
    continue_session_inner_for_tier(db, session_id, pause_id, crate::runtime_lock::is_primary())
        .await
}

async fn continue_session_inner_for_tier(
    db: Arc<SessionDB>,
    session_id: &str,
    expected_pause_id: &str,
    primary: bool,
) -> anyhow::Result<crate::session::SessionAutonomyResumeOutcome> {
    let transition_guard = autonomy_transition_lock().lock_owned().await;
    if autonomy_stop_pending(session_id) || super::active_turn::stop_cleanup_active(session_id) {
        anyhow::bail!("Continue was superseded by a newer Stop; the pause fence remains active");
    }
    let sid = session_id.to_string();
    let expected_pause_id = expected_pause_id.to_string();
    let outcome = db
        .clone()
        .run(move |db| {
            let Some(pause) = db.active_session_autonomy_pause(&sid)? else {
                return Ok(crate::session::SessionAutonomyResumeOutcome::default());
            };
            if expected_pause_id != pause.id {
                anyhow::bail!("session pause receipt changed before Continue could consume it");
            }

            if let Some(goal_id) = pause.goal_id.as_deref() {
                if let Some(goal) = db.get_goal(goal_id)? {
                    if goal.state == crate::goal::GoalState::Paused {
                        db.resume_goal(goal_id)?;
                    }
                }
            }

            let mut workflow_run_ids = Vec::new();
            for run_id in &pause.workflow_run_ids {
                let Some(run) = db.get_workflow_run(run_id)? else {
                    continue;
                };
                if run.state == crate::workflow::WorkflowRunState::Paused {
                    db.resume_workflow_run(run_id)?;
                    workflow_run_ids.push(run_id.clone());
                } else if matches!(
                    run.state,
                    crate::workflow::WorkflowRunState::Draft
                        | crate::workflow::WorkflowRunState::Running
                        | crate::workflow::WorkflowRunState::Recovering
                ) {
                    // Crash window: the durable receipt landed before the
                    // controller transition. Once the fence is consumed this
                    // exact run still needs a runtime launch.
                    workflow_run_ids.push(run_id.clone());
                }
            }

            if !db.finish_session_autonomy_resume(&pause.id)? {
                anyhow::bail!("session pause receipt was consumed concurrently");
            }
            Ok(crate::session::SessionAutonomyResumeOutcome {
                resumed: true,
                pause_id: Some(pause.id),
                goal_id: pause.goal_id,
                workflow_run_ids,
                subagent_run_ids: pause.subagent_run_ids,
            })
        })
        .await?;

    // Stop installs its admission gate before waiting for the transition lock.
    // If it arrived while this Continue was converging controller state, do not
    // launch any replay work; the newer Stop will publish the next generation.
    if autonomy_stop_pending(session_id) || super::active_turn::stop_cleanup_active(session_id) {
        anyhow::bail!("Continue was superseded by a newer Stop; the pause fence remains active");
    }
    drop(transition_guard);

    if outcome.resumed && primary {
        replay_pending_session_autonomy_resumes_for_tier(db.clone(), true).await;
    }
    Ok(outcome)
}

struct AutonomyResumeReplayPermit;

impl Drop for AutonomyResumeReplayPermit {
    fn drop(&mut self) {
        AUTONOMY_RESUME_REPLAY_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn try_acquire_autonomy_resume_replay() -> Option<AutonomyResumeReplayPermit> {
    AUTONOMY_RESUME_REPLAY_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| AutonomyResumeReplayPermit)
}

/// Drain durable Continue handoffs in the Primary process. Secondary adapters
/// only publish the request; wakeup timers and workflow runtimes are strictly
/// process-local and must be restarted by their single owner.
async fn replay_pending_session_autonomy_resumes(db: Arc<SessionDB>) {
    replay_pending_session_autonomy_resumes_for_tier(db, crate::runtime_lock::is_primary()).await;
}

async fn replay_pending_session_autonomy_resumes_for_tier(db: Arc<SessionDB>, primary: bool) {
    if !primary {
        return;
    }
    let Some(_permit) = try_acquire_autonomy_resume_replay() else {
        return;
    };
    let pending = match db
        .clone()
        .run(|db| {
            db.list_pending_session_autonomy_resume_replays(AUTONOMY_RESUME_REPLAY_BATCH_SIZE)
        })
        .await
    {
        Ok(pending) => pending,
        Err(error) => {
            crate::app_warn!(
                "chat",
                "continue_handoff",
                "Failed to list durable Continue handoffs: {error:#}"
            );
            return;
        }
    };

    for pause in pending {
        // Re-check the durable fence immediately before touching process-local
        // sources. A newer Stop also marks this request superseded in its DB
        // transaction; this check covers a replay snapshot racing that write.
        let sid = pause.session_id.clone();
        let replay_pause_id = pause.id.clone();
        let (replay_session_ids, resume_global_stop_epoch) = match db
            .clone()
            .run(move |db| {
                if db.is_session_autonomy_paused(&sid)? {
                    return Ok::<_, anyhow::Error>(None);
                }
                let session_ids = db.list_session_autonomy_tree_ids(&sid)?;
                let global_stop_epoch =
                    db.session_autonomy_resume_global_stop_epoch(&replay_pause_id)?;
                Ok(Some((session_ids, global_stop_epoch)))
            })
            .await
        {
            Ok(Some(ids)) => ids,
            Ok(None) => continue,
            Err(error) => {
                let pause_id = pause.id.clone();
                let message = format!("resolve continued session tree: {error:#}");
                let message_for_db = message.clone();
                let _ = db
                    .clone()
                    .run(move |db| {
                        db.record_session_autonomy_resume_replay_error(&pause_id, &message_for_db)
                    })
                    .await;
                crate::app_warn!(
                    "chat",
                    "continue_handoff",
                    "Failed to replay Continue handoff {}: {}",
                    pause.id,
                    message
                );
                continue;
            }
        };

        let mut replay_error = None;
        for replay_session_id in replay_session_ids {
            if let Err(error) = crate::wakeup::replay_pending_for_session(
                &replay_session_id,
                resume_global_stop_epoch,
            )
            .await
            {
                replay_error = Some(format!(
                    "replay wakeups for session {}: {error:#}",
                    replay_session_id
                ));
                break;
            }
            let replay_db = db.clone();
            crate::blocking::run_blocking(move || {
                crate::subagent::replay_pending_parent_deliveries_for_session(
                    &replay_db,
                    &replay_session_id,
                )
            })
            .await;
        }
        if let Some(error) = replay_error {
            let pause_id = pause.id.clone();
            let error_for_db = error.clone();
            let _ = db
                .clone()
                .run(move |db| {
                    db.record_session_autonomy_resume_replay_error(&pause_id, &error_for_db)
                })
                .await;
            crate::app_warn!(
                "chat",
                "continue_handoff",
                "Deferred Continue handoff {} retry: {}",
                pause.id,
                error
            );
            continue;
        }

        for run_id in &pause.workflow_run_ids {
            crate::workflow::schedule_workflow_resume_after_pause(db.clone(), run_id.clone());
        }
        let pause_id = pause.id.clone();
        match db
            .clone()
            .run(move |db| db.finish_session_autonomy_resume_replay(&pause_id))
            .await
        {
            Ok(true) => crate::app_info!(
                "chat",
                "continue_handoff",
                "Primary replayed durable Continue handoff {} for session {}",
                pause.id,
                pause.session_id
            ),
            Ok(false) => {}
            Err(error) => crate::app_warn!(
                "chat",
                "continue_handoff",
                "Failed to acknowledge Continue handoff {}: {error:#}",
                pause.id
            ),
        }
    }
}

/// Install the Primary-owned cross-process Continue replay loop. The immediate
/// first drain covers requests written while no Primary was alive; subsequent
/// ticks hand off Secondary Continue calls without requiring a restart.
pub fn spawn_session_autonomy_resume_replay_loop() {
    if !crate::runtime_lock::is_primary()
        || AUTONOMY_RESUME_REPLAY_LOOP_STARTED.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let Some(db) = crate::get_session_db() else {
        AUTONOMY_RESUME_REPLAY_LOOP_STARTED.store(false, Ordering::SeqCst);
        crate::app_warn!(
            "chat",
            "continue_handoff",
            "Session DB unavailable; durable Continue replay loop not started"
        );
        return;
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(AUTONOMY_RESUME_REPLAY_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if !crate::runtime_lock::is_primary() {
                AUTONOMY_RESUME_REPLAY_LOOP_STARTED.store(false, Ordering::SeqCst);
                break;
            }
            replay_pending_session_autonomy_resumes(db.clone()).await;
        }
    });
}

async fn converge_local_session_autonomy_stop_fences(db: Arc<SessionDB>) {
    let foreground_generations = super::durability::active_foreground_stop_generations();
    let injection_generations = crate::subagent::active_parent_injection_generations();
    let subagent_run_ids = crate::get_subagent_cancels()
        .map(|registry| registry.active_run_ids())
        .unwrap_or_default();
    let workflow_generations = crate::workflow::active_workflow_runtime_generations();
    let local_wakeup_states = crate::wakeup::local_wakeup_global_stop_states();
    if foreground_generations.is_empty()
        && injection_generations.is_empty()
        && subagent_run_ids.is_empty()
        && workflow_generations.is_empty()
        && local_wakeup_states.is_empty()
    {
        return;
    }

    let fences = db
        .run(move |db| {
            db.resolve_local_autonomy_stop_fences(
                &foreground_generations,
                &injection_generations,
                &subagent_run_ids,
                &workflow_generations,
            )
        })
        .await;
    let (stale_injections, captured_subagents, captured_workflows, stale_foreground_runs) =
        match fences {
            Ok(fences) => {
                AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED.store(false, Ordering::SeqCst);
                fences
            }
            Err(error) => {
                if !AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                    crate::app_warn!(
                        "chat",
                        "stop_handoff",
                        "Failed to resolve durable Stop handoffs for local runtimes: {error:#}"
                    );
                }
                return;
            }
        };

    for run_id in stale_foreground_runs {
        super::durability::request_foreground_stop_for_run(&run_id);
    }

    for session_id in stale_injections {
        crate::subagent::request_pause_parent_injection(&session_id);
    }
    for run_id in captured_subagents {
        crate::subagent::request_pause_run(&run_id);
    }
    for run_id in captured_workflows {
        crate::workflow::request_workflow_runtime_pause(&run_id);
    }

    if local_wakeup_states.is_empty() {
        return;
    }

    // Collapse owner-local wakeup snapshots by session. The session-free epoch
    // backs up volatile timers and durable timers alike when another process's
    // WakeupDB enumeration is unavailable.
    let mut local_wakeup_sessions = std::collections::HashMap::<String, (u64, bool)>::new();
    for (session_id, observed_epoch, paused) in local_wakeup_states {
        let entry = local_wakeup_sessions
            .entry(session_id)
            .or_insert((observed_epoch, paused));
        entry.0 = entry.0.min(observed_epoch);
        entry.1 |= paused;
    }
    let decisions = db
        .clone()
        .run(move |db| {
            let global_stop_epoch = db.global_stop_epoch()?;
            let mut publish = Vec::new();
            let mut fence = Vec::new();
            let mut resume = Vec::new();
            let mut purge = Vec::new();
            for (session_id, (observed_epoch, paused)) in local_wakeup_sessions {
                if db.get_session(&session_id)?.is_none() {
                    purge.push(session_id);
                    continue;
                }
                let any_active = db.is_session_or_ancestor_autonomy_paused(&session_id)?;
                let (active_global, resumed_global) =
                    db.session_lineage_global_stop_receipt_state(&session_id, global_stop_epoch)?;
                if global_stop_epoch > observed_epoch {
                    if any_active || active_global {
                        fence.push(session_id);
                    } else if resumed_global {
                        // The owner missed both Stop and Continue. Briefly park
                        // the old generation before re-admitting it at the
                        // consumed generation.
                        fence.push(session_id.clone());
                        resume.push(session_id);
                    } else {
                        publish.push(session_id);
                    }
                } else if !paused && any_active {
                    // A receipt may land after the timer was admitted at the
                    // same global epoch (for example a targeted Stop, or the
                    // durable-enumeration owner finishing late).
                    fence.push(session_id);
                } else if paused && !any_active {
                    // A parked owner-local timer can also belong to a targeted
                    // Stop handled by another process. Once its durable
                    // lineage has no active receipt, the final user decision
                    // is Continue regardless of how that receipt was scoped.
                    resume.push(session_id);
                }
            }
            Ok::<_, anyhow::Error>((global_stop_epoch, publish, fence, resume, purge))
        })
        .await;
    let (global_stop_epoch, publish, fence, resume, purge) = match decisions {
        Ok(decisions) => decisions,
        Err(error) => {
            if !AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                crate::app_warn!(
                    "chat",
                    "stop_handoff",
                    "Failed to resolve owner-local wakeup Stop handoffs: {error:#}"
                );
            }
            return;
        }
    };

    for session_id in purge {
        crate::wakeup::purge_for_session(&session_id);
    }
    for session_id in fence {
        crate::wakeup::suspend_for_session_at_global_epoch(&session_id, Some(global_stop_epoch));
    }
    for session_id in resume {
        crate::wakeup::resume_local_for_session(&session_id);
    }

    if !publish.is_empty() {
        let transition_guard = autonomy_transition_lock().lock_owned().await;
        let publish_result = db
            .run(move |db| {
                let _transition_guard = transition_guard;
                let current_global_stop_epoch = db.global_stop_epoch()?;
                for session_id in publish {
                    prepare_and_pause_session_autonomy_blocking(
                        db,
                        &session_id,
                        Some(current_global_stop_epoch),
                    )?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await;
        if let Err(error) = publish_result {
            if !AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED.swap(true, Ordering::SeqCst) {
                crate::app_warn!(
                    "chat",
                    "stop_handoff",
                    "Failed to publish owner-local wakeup Stop receipts: {error:#}"
                );
            }
        } else {
            AUTONOMY_STOP_FENCE_WATCH_ERROR_LOGGED.store(false, Ordering::SeqCst);
        }
    }
}

/// Every process owns its own foreground/injection/sub-agent/workflow cancel flags, while
/// Stop receipts live in the shared SQLite database. Poll only while a local
/// stoppable runtime exists, and compare immutable generations/run ids so a
/// quick Continue cannot hide the Stop before the owning process observes it.
pub fn spawn_session_autonomy_stop_fence_watcher() {
    if AUTONOMY_STOP_FENCE_WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(db) = crate::get_session_db() else {
        AUTONOMY_STOP_FENCE_WATCHER_STARTED.store(false, Ordering::SeqCst);
        crate::app_warn!(
            "chat",
            "stop_handoff",
            "Session DB unavailable; durable Stop fence watcher not started"
        );
        return;
    };
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(AUTONOMY_STOP_FENCE_POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            converge_local_session_autonomy_stop_fences(db.clone()).await;
        }
    });
}

#[derive(Debug, Default)]
pub struct StopAllOutcome {
    pub stopped: bool,
    pub stopped_session_count: usize,
    pub denied_approvals: usize,
    pub cancelled_questions: usize,
    pub runtime_cancellations: Vec<CancelRuntimeTaskResult>,
    pub runtime_cancellation_error: Option<String>,
}

async fn prepare_global_session_pauses(
    db: Arc<SessionDB>,
    session_ids: Vec<String>,
    excluded_root_session_ids: HashSet<String>,
    global_stop_epoch: Option<u64>,
    cleanup_gate: Arc<super::active_turn::GlobalStopCleanupGuard>,
    stop_claim: Arc<GlobalAutonomyStopClaim>,
) -> Vec<String> {
    if session_ids.is_empty() {
        return Vec::new();
    }
    let task = tokio::spawn(async move {
        let transition_guard = autonomy_transition_lock().lock_owned().await;
        db.run(move |db| {
            // See the session-scoped path above: these guards must live in
            // the blocking closure after an async timeout detaches its work.
            let _cleanup_gate = cleanup_gate;
            let _stop_claim = stop_claim;
            let _transition_guard = transition_guard;
            let mut paused = Vec::new();
            let mut errors = Vec::new();
            let mut root_session_ids = HashSet::new();
            for session_id in session_ids {
                match db.resolve_session_root_id(&session_id) {
                    Ok(root_session_id) => {
                        root_session_ids.insert(root_session_id);
                    }
                    Err(error) => errors.push((session_id, error.to_string())),
                }
            }
            root_session_ids.retain(|session_id| !excluded_root_session_ids.contains(session_id));
            let mut root_session_ids = root_session_ids.into_iter().collect::<Vec<_>>();
            root_session_ids.sort();
            for session_id in root_session_ids {
                match prepare_and_pause_session_autonomy_blocking(
                    db,
                    &session_id,
                    global_stop_epoch,
                ) {
                    Ok(_) => paused.push(session_id),
                    Err(error) => errors.push((session_id, error.to_string())),
                }
            }
            (paused, errors)
        })
        .await
    });
    let result = tokio::time::timeout(STOP_DB_MARK_TIMEOUT, task).await;
    match result {
        Ok(Ok((paused, errors))) => {
            for (session_id, error) in errors {
                crate::app_warn!(
                    "chat",
                    "stop_all_sessions",
                    "Failed to persist global Stop pause for session {}: {}",
                    session_id,
                    crate::logging::redact_sensitive(&error)
                );
            }
            paused
        }
        Ok(Err(error)) => {
            crate::app_warn!(
                "chat",
                "stop_all_sessions",
                "Global Stop pause task failed: {}",
                error
            );
            Vec::new()
        }
        Err(_) => {
            crate::app_warn!(
                "chat",
                "stop_all_sessions",
                "Timed out after {}ms persisting global Stop pauses",
                STOP_DB_MARK_TIMEOUT.as_millis()
            );
            Vec::new()
        }
    }
}

struct GlobalStopGenerationOutcome {
    epoch: u64,
    session_ids: Vec<String>,
    paused_session_ids: Vec<String>,
}

enum GlobalStopGenerationDispatch {
    Settled(GlobalStopGenerationOutcome),
    Detached,
}

async fn begin_global_stop_generation(
    db: Arc<SessionDB>,
    seed_session_ids: Vec<String>,
    cleanup_gate: Arc<super::active_turn::GlobalStopCleanupGuard>,
    stop_claim: Arc<GlobalAutonomyStopClaim>,
) -> anyhow::Result<GlobalStopGenerationDispatch> {
    let task = tokio::spawn(async move {
        let transition_guard = autonomy_transition_lock().lock_owned().await;
        db.run(move |db| {
            let _cleanup_gate = cleanup_gate;
            let _stop_claim = stop_claim;
            let _transition_guard = transition_guard;
            let (epoch, enumerated_session_ids) = db.begin_global_stop_enumeration()?;
            crate::app_info!(
                "chat",
                "stop_all_sessions",
                "Published cross-process global Stop generation {}",
                epoch
            );
            // This task owns both the durable enumeration and every local
            // surface known before epoch publication, even after the transport
            // timeout detaches its JoinHandle. The caller must never recreate
            // these as un-attributed targeted receipts.
            let mut session_ids = enumerated_session_ids.into_iter().collect::<HashSet<_>>();
            for session_id in seed_session_ids {
                match db.resolve_session_root_id(&session_id) {
                    Ok(root_session_id) => {
                        session_ids.insert(root_session_id);
                    }
                    Err(error) => crate::app_warn!(
                        "chat",
                        "stop_all_sessions",
                        "Failed to resolve global Stop session {}: {}",
                        session_id,
                        crate::logging::redact_sensitive(&error.to_string())
                    ),
                }
            }
            let mut session_ids = session_ids.into_iter().collect::<Vec<_>>();
            session_ids.sort();

            // A detached owner is the only component that still has the exact
            // generation and session snapshot. Keep the Stop gate/claim while
            // transient receipt failures retry; otherwise a remote controller
            // could outlive a successful global epoch publication.
            let mut paused_session_ids = Vec::new();
            let mut pending_session_ids = session_ids.clone();
            let mut retry_delay = Duration::from_millis(25);
            let mut retry_round = 0_u64;
            while !pending_session_ids.is_empty() {
                let mut retry = Vec::new();
                for session_id in pending_session_ids {
                    match prepare_and_pause_session_autonomy_blocking(
                        db,
                        &session_id,
                        Some(epoch),
                    ) {
                        Ok(_) => paused_session_ids.push(session_id),
                        Err(error) => match db
                            .session_lineage_global_stop_receipt_state(&session_id, epoch)
                        {
                            Ok((true, _)) => {
                                // The authoritative restart fence committed;
                                // only an eager controller transition failed.
                                // Admission remains fail-closed on the receipt,
                                // so do not pin the global gate forever on a
                                // non-persistence error.
                                crate::app_warn!(
                                    "chat",
                                    "stop_all_sessions",
                                    "Global Stop receipt committed for session {} but eager controller pause failed: {}",
                                    session_id,
                                    crate::logging::redact_sensitive(&error.to_string())
                                );
                                paused_session_ids.push(session_id);
                            }
                            _ => match db.get_session(&session_id) {
                            Ok(None) => crate::app_info!(
                                "chat",
                                "stop_all_sessions",
                                "Stopped retrying global Stop receipt for deleted session {}",
                                session_id
                            ),
                            _ => {
                                if retry_round == 0 || retry_round.is_power_of_two() {
                                    crate::app_warn!(
                                        "chat",
                                        "stop_all_sessions",
                                        "Retrying global Stop receipt for session {} after persistence failure (round {}): {}",
                                        session_id,
                                        retry_round + 1,
                                        crate::logging::redact_sensitive(&error.to_string())
                                    );
                                }
                                retry.push(session_id);
                            }
                            },
                        },
                    }
                }
                if retry.is_empty() {
                    break;
                }
                std::thread::sleep(retry_delay);
                retry_delay = (retry_delay * 2).min(Duration::from_millis(500));
                retry_round = retry_round.saturating_add(1);
                pending_session_ids = retry;
            }
            Ok::<_, anyhow::Error>(GlobalStopGenerationOutcome {
                epoch,
                session_ids,
                paused_session_ids,
            })
        })
        .await
    });
    match tokio::time::timeout(STOP_DB_MARK_TIMEOUT, task).await {
        Ok(Ok(Ok(outcome))) => Ok(GlobalStopGenerationDispatch::Settled(outcome)),
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "global Stop generation task failed: {error}"
        )),
        Err(_) => {
            crate::app_warn!(
                "chat",
                "stop_all_sessions",
                "Global Stop generation exceeded {}ms; detached owner will finish enumeration and receipts",
                STOP_DB_MARK_TIMEOUT.as_millis()
            );
            Ok(GlobalStopGenerationDispatch::Detached)
        }
    }
}

/// Persistence won its atomic claim, but Stop arrived while the blocking DB
/// operation was still running. Converge the now-durable turn off the async
/// worker so the earlier watchdog cannot miss a row that did not exist yet.
pub async fn finalize_persisted_user_stop(
    db: Arc<SessionDB>,
    session_id: String,
    turn_id: String,
    user_message: String,
    source: crate::chat_engine::ChatSource,
) -> crate::chat_engine::finalize::FinalizeOutcome {
    crate::blocking::run_blocking(move || {
        crate::chat_engine::finalize::finalize_turn_context_blocking(
            &db,
            &session_id,
            crate::chat_engine::finalize::TerminationReason::UserStop,
            crate::chat_engine::finalize::PartialMeta {
                user_message: Some(user_message),
                turn_id: Some(turn_id),
                ..Default::default()
            },
            source,
        )
    })
    .await
}

/// Cleanup owned state for a prompt that was cancelled before a durable chat
/// turn existed. Construction installs the session cleanup gate immediately;
/// callers may then publish/release any transport-visible lifecycle before
/// spawning the potentially blocking Git/SQLite work.
pub struct PreTurnCancelCleanup {
    db: Arc<SessionDB>,
    session_id: String,
    bootstrap_request_id: Option<String>,
    delete_empty_session: bool,
    queued_dispatch: Option<(String, String)>,
    cleanup_gate: super::active_turn::StopCleanupGuard,
}

impl PreTurnCancelCleanup {
    pub fn begin(
        db: Arc<SessionDB>,
        session_id: String,
        bootstrap_request_id: Option<String>,
        delete_empty_session: bool,
        queued_dispatch: Option<(String, String)>,
    ) -> Option<Self> {
        if bootstrap_request_id.is_none() && !delete_empty_session && queued_dispatch.is_none() {
            return None;
        }
        let cleanup_gate = super::active_turn::begin_stop_cleanup(&session_id);
        Some(Self {
            db,
            session_id,
            bootstrap_request_id,
            delete_empty_session,
            queued_dispatch,
            cleanup_gate,
        })
    }

    pub fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(self) {
        let Self {
            db,
            session_id,
            bootstrap_request_id,
            delete_empty_session,
            queued_dispatch,
            cleanup_gate,
        } = self;
        let queued_release_requested = queued_dispatch.is_some();
        let queued_release_succeeded = if let Some((request_id, turn_id)) = queued_dispatch {
            let sid_for_release = session_id.clone();
            match tokio::time::timeout(
                PRE_TURN_QUEUE_RELEASE_TIMEOUT,
                db.clone().run(move |db| {
                    db.release_queued_turn_message_dispatch(&sid_for_release, &request_id, &turn_id)
                }),
            )
            .await
            {
                Ok(Ok(_)) => true,
                Ok(Err(error)) => {
                    crate::app_warn!(
                        "chat",
                        "pre_turn_cancel_cleanup",
                        "Failed to release queued dispatch for stopped chat {}: {}",
                        session_id,
                        error
                    );
                    false
                }
                Err(_) => {
                    // The exact session/request/turn CAS may still finish on
                    // the detached blocking worker. Releasing this cleanup
                    // gate is safe because it cannot mutate a replacement
                    // turn's queue claim.
                    crate::app_warn!(
                        "chat",
                        "pre_turn_cancel_cleanup",
                        "Timed out after {}ms releasing queued dispatch for stopped chat {}",
                        PRE_TURN_QUEUE_RELEASE_TIMEOUT.as_millis(),
                        session_id
                    );
                    false
                }
            }
        } else {
            true
        };
        let had_bootstrap = bootstrap_request_id.is_some();
        let bootstrap_rollback_succeeded = if let Some(request_id) = bootstrap_request_id {
            match db
                .clone()
                .run(move |db| db.rollback_project_bootstrap_after_chat_cancel(&request_id))
                .await
            {
                Ok(()) => true,
                Err(error) => {
                    // Preserve the Session + managed-worktree row when
                    // Git-aware cleanup fails; startup recovery can retry it
                    // without orphaning the physical worktree.
                    crate::app_warn!(
                        "project",
                        "bootstrap_cancel_rollback",
                        "Failed to roll back project bootstrap for stopped chat {}: {}",
                        session_id,
                        error
                    );
                    false
                }
            }
        } else {
            true
        };
        let mut session_deleted = false;
        if delete_empty_session && bootstrap_rollback_succeeded {
            let sid_for_delete = session_id.clone();
            match db.run(move |db| db.delete_session(&sid_for_delete)).await {
                Ok(()) => session_deleted = true,
                Err(error) => {
                    crate::app_warn!(
                        "chat",
                        "pre_turn_cancel_cleanup",
                        "Failed to delete empty stopped session {}: {}",
                        session_id,
                        error
                    );
                }
            }
        }
        crate::app_info!(
            "chat",
            "pre_turn_cancel_cleanup",
            "Pre-turn Stop cleanup settled: session={} queued_release_requested={} queued_release_succeeded={} bootstrap={} bootstrap_rollback_succeeded={} delete_requested={} session_deleted={}",
            session_id,
            queued_release_requested,
            queued_release_succeeded,
            had_bootstrap,
            bootstrap_rollback_succeeded,
            delete_empty_session,
            session_deleted
        );
        drop(cleanup_gate);
    }
}

async fn timeout_count(operation: &'static str, future: impl Future<Output = usize>) -> usize {
    // Time the operation itself rather than a detached JoinHandle. Both
    // interaction cleanup functions drain their exact entries before their
    // first post-lock await, so dropping this future cannot later consume an
    // interaction registered by a replacement turn.
    match tokio::time::timeout(STOP_INTERACTION_CLEANUP_TIMEOUT, future).await {
        Ok(count) => count,
        Err(_) => {
            crate::app_warn!(
                "chat",
                "stop",
                "Timed out after {}ms during {}",
                STOP_INTERACTION_CLEANUP_TIMEOUT.as_millis(),
                operation
            );
            0
        }
    }
}

async fn timeout_runtime_cleanup(
    future: impl Future<Output = anyhow::Result<Vec<CancelRuntimeTaskResult>>>,
) -> anyhow::Result<Vec<CancelRuntimeTaskResult>> {
    match tokio::time::timeout(STOP_RUNTIME_CLEANUP_TIMEOUT, future).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "runtime cancellation timed out after {}ms",
            STOP_RUNTIME_CLEANUP_TIMEOUT.as_millis()
        )),
    }
}

/// Stop the foreground work owned by one session.
///
/// `already_signalled` covers transport-specific pre-registration handles
/// outside core (for example an HTTP request handle). Channel preflight handles
/// are resolved here so GUI, HTTP, and IM `/stop` cannot diverge. An
/// `expected_turn_id` keeps an exact stale Stop from cancelling a newer turn;
/// `None` intentionally means "whatever is active in this session" and still
/// reaps interaction/runtime work when the active-turn entry disappeared.
pub async fn stop_session(
    db: Arc<SessionDB>,
    session_id: &str,
    expected_turn_id: Option<&str>,
    already_signalled: bool,
) -> StopSessionOutcome {
    // Keep replacement turns out until the runtime snapshot below has captured
    // only work belonging to the stopped generation.
    let mut stop_cleanup_guard = Some(Arc::new(super::active_turn::begin_stop_cleanup(session_id)));
    let mut outcome = StopSessionOutcome {
        stopped: already_signalled,
        ..Default::default()
    };
    let mut matched_active = false;
    let mut active_not_found = false;
    let mut durable_turn_id = None;
    let mut exact_cron_turn = false;

    match super::active_turn::cancel_current(session_id, expected_turn_id) {
        super::active_turn::ActiveTurnCancelOutcome::Cancelled(active) => {
            matched_active = true;
            outcome.stopped = true;
            outcome.active_turn_found = true;
            exact_cron_turn =
                expected_turn_id.is_some() && matches!(active.source, super::ChatSource::Cron);

            // Channel has its own stream lifecycle bus and deliberately does
            // not create a GUI chat_turn row. Desktop/HTTP use the normal
            // chat:* status + durable watchdog path. Broadcast and arm the
            // watchdog before any DB/cleanup await so Stop feedback and guard
            // release cannot be delayed by a saturated blocking pool.
            if active.source.broadcasts_to_user_ui() {
                super::stream_broadcast::broadcast_turn_status(
                    session_id,
                    &active.turn_id,
                    ChatTurnStatus::Cancelling,
                    Some(ChatTurnInterruptReason::UserStop),
                );
                super::spawn_user_stop_watchdog(
                    db.clone(),
                    session_id.to_string(),
                    active.turn_id.clone(),
                    active.source,
                );
                outcome.terminal_event_pending = true;
                durable_turn_id = Some(active.turn_id);
            }
        }
        super::active_turn::ActiveTurnCancelOutcome::TurnMismatch => {
            outcome.turn_mismatch = true;
            outcome.active_turn_found = true;
        }
        super::active_turn::ActiveTurnCancelOutcome::CompletionSealed => {
            // Completion and Stop serialize through the active-turn registry.
            // The executor crossed its cancellation point first, so this exact
            // Stop must not claim success or fall through to durable recovery.
            outcome.active_turn_found = true;
            outcome.completion_sealed = true;
        }
        super::active_turn::ActiveTurnCancelOutcome::NotFound => {
            active_not_found = true;
        }
    }

    let recovered_not_found_turn = if active_not_found && expected_turn_id.is_some() {
        recover_not_found_durable_turn(db.clone(), session_id, expected_turn_id).await
    } else {
        None
    };
    exact_cron_turn |= recovered_not_found_turn
        .as_ref()
        .is_some_and(|(_, source)| matches!(source, super::ChatSource::Cron));

    // A session-only Stop is authoritative even if the active entry vanished
    // between UI observation and this call. An exact stale turn must not tear
    // down Channel preflight, waits, or runtime that may belong to a newer turn.
    // Once this function acquired the Stop cleanup gate, `NotFound` proves no
    // replacement turn is active: a replacement admitted just before the gate
    // would have produced `TurnMismatch`, while one after it cannot acquire.
    // For an exact NotFound, additionally require that the expected durable
    // turn still belongs to this session and is non-terminal. This fixes a stale
    // spinner without letting a very late Stop for an old completed turn pause
    // autonomous work created by a newer generation.
    let settle_session = expected_turn_id.is_none()
        || matched_active
        || already_signalled
        || recovered_not_found_turn.is_some();
    // Stopping one exact Cron occurrence must not create a session-wide pause:
    // later scheduled occurrences remain eligible while this turn still gets
    // the ordinary exact-turn cancellation and runtime cleanup below.
    let persist_autonomy_pause = settle_session && !exact_cron_turn;
    let autonomy_stop_claim =
        settle_session.then(|| Arc::new(begin_session_autonomy_stop(session_id)));
    if !settle_session {
        stop_cleanup_guard.take();
    }
    if settle_session {
        let channel_signalled = crate::globals::get_channel_cancels()
            .map(|registry| registry.cancel(session_id))
            .unwrap_or(false);
        outcome.stopped |= channel_signalled;
    }

    if let Some((turn_id, source)) = recovered_not_found_turn {
        outcome.stopped = true;
        if source.broadcasts_to_user_ui() {
            super::stream_broadcast::broadcast_turn_status(
                session_id,
                &turn_id,
                ChatTurnStatus::Cancelling,
                Some(ChatTurnInterruptReason::UserStop),
            );
            super::spawn_user_stop_watchdog(
                db.clone(),
                session_id.to_string(),
                turn_id.clone(),
                source,
            );
            outcome.terminal_event_pending = true;
        }
        durable_turn_id = Some(turn_id);
    }

    // Publish the durable pause fence before taking the runtime snapshot. A
    // Workflow recovery or Goal wakeup racing this Stop must observe the fence
    // even if controller-specific convergence later times out.
    if persist_autonomy_pause {
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            prepare_and_pause_session_autonomy(
                db.clone(),
                session_id.to_string(),
                Arc::clone(
                    stop_cleanup_guard
                        .as_ref()
                        .expect("settled Stop retains its cleanup gate"),
                ),
                Arc::clone(
                    autonomy_stop_claim
                        .as_ref()
                        .expect("settled Stop retains its autonomy claim"),
                ),
            ),
        )
        .await
        {
            Ok(Ok(pause)) => {
                if pause.is_some() {
                    outcome.stopped = true;
                    crate::wakeup::suspend_for_session(session_id);
                }
                outcome.autonomy_pause = pause;
            }
            Ok(Err(error)) => {
                let message = crate::logging::redact_sensitive(&error.to_string());
                crate::app_warn!(
                    "chat",
                    "stop_session",
                    "Failed to persist session autonomy pause for {}: {}",
                    session_id,
                    message
                );
                outcome.autonomy_pause_error = Some(message);
            }
            Err(_) => {
                let message = format!(
                    "session autonomy pause timed out after {}ms",
                    STOP_DB_MARK_TIMEOUT.as_millis()
                );
                crate::app_warn!("chat", "stop_session", "{}: {}", message, session_id);
                outcome.autonomy_pause_error = Some(message);
            }
        }

        // Parent result injections are not part of the generic runtime task
        // registry and use turn_id=None. Signal their dedicated cancel token
        // only after the durable pause generation exists, so their normal
        // requeue path cannot race ahead of Stop and issue another API round.
        outcome.stopped |= crate::subagent::request_pause_parent_injection(session_id);
    }

    // Durable marking, interaction resolution and runtime reaping are
    // independent. Run them concurrently and bound each wait: the synchronous
    // cancel flags above are authoritative, while these convergence steps must
    // never make the Stop request itself unbounded.
    let mark_turn = async {
        let Some(turn_id) = durable_turn_id else {
            return;
        };
        let turn_id_for_mark = turn_id.clone();
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            db.clone().run(move |db| {
                db.mark_chat_turn_cancelling(&turn_id_for_mark, ChatTurnInterruptReason::UserStop)
            }),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => crate::app_warn!(
                "chat",
                "stop_session",
                "Failed to mark stopped turn {} for session {} as cancelling: {}",
                turn_id,
                session_id,
                error
            ),
            Err(_) => crate::app_warn!(
                "chat",
                "stop_session",
                "Timed out after {}ms marking stopped turn {} for session {} as cancelling",
                STOP_DB_MARK_TIMEOUT.as_millis(),
                turn_id,
                session_id
            ),
        }
    };
    let approval_session_id = session_id.to_string();
    let deny_approvals = async move {
        if !settle_session {
            return 0;
        }
        timeout_count("denying approvals for stopped session", async move {
            crate::tools::deny_pending_for_session(
                &approval_session_id,
                ApprovalResolutionSource::UserStop,
            )
            .await
        })
        .await
    };
    let question_session_id = session_id.to_string();
    let cancel_questions = async move {
        if !settle_session {
            return 0;
        }
        timeout_count("cancelling questions for stopped session", async move {
            crate::ask_user::cancel_pending_ask_user_questions_for_session(
                &question_session_id,
                "user_stop",
            )
            .await
        })
        .await
    };
    let runtime_session_id = session_id.to_string();
    let cancel_runtime = async move {
        if !settle_session {
            return Ok(Vec::new());
        }
        timeout_runtime_cleanup(async move {
            let snapshot =
                crate::runtime_tasks::snapshot_runtime_tasks_for_session_stop(&runtime_session_id)
                    .await?;
            crate::runtime_tasks::cancel_runtime_task_snapshot(snapshot).await
        })
        .await
    };
    let queue_db = db.clone();
    let queue_session_id = session_id.to_string();
    let hold_channel_queue = async move {
        if !settle_session {
            return 0;
        }
        let sid = queue_session_id.clone();
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            queue_db.run(move |db| db.hold_channel_turn_messages_after_stop(&sid)),
        )
        .await
        {
            Ok(Ok(count)) => count,
            Ok(Err(error)) => {
                crate::app_warn!(
                    "chat",
                    "stop_session",
                    "Failed to hold queued IM messages for stopped session {}: {}",
                    queue_session_id,
                    error
                );
                0
            }
            Err(_) => {
                crate::app_warn!(
                    "chat",
                    "stop_session",
                    "Timed out after {}ms holding queued IM messages for stopped session {}",
                    STOP_DB_MARK_TIMEOUT.as_millis(),
                    queue_session_id
                );
                0
            }
        }
    };
    let ((), denied_approvals, cancelled_questions, runtime_result, held_channel_messages) = tokio::join!(
        mark_turn,
        deny_approvals,
        cancel_questions,
        cancel_runtime,
        hold_channel_queue
    );
    outcome.denied_approvals = denied_approvals;
    outcome.cancelled_questions = cancelled_questions;
    match runtime_result {
        Ok(results) => outcome.runtime_cancellations = results,
        Err(error) => {
            crate::app_warn!(
                "chat",
                "stop_session",
                "Runtime cancellation failed after stopping session {}: {}",
                session_id,
                error
            );
            outcome.runtime_cancellation_error = Some(error.to_string());
        }
    }

    if settle_session {
        outcome.stopped = outcome.stopped
            || held_channel_messages > 0
            || outcome.denied_approvals > 0
            || outcome.cancelled_questions > 0
            || outcome
                .runtime_cancellations
                .iter()
                .any(|result| result.accepted);
    }

    drop(autonomy_stop_claim);
    drop(stop_cleanup_guard);

    crate::app_info!(
        "chat",
        "stop_session",
        "Session stop settled: session={} expected_turn={:?} stopped={} turn_mismatch={} held_channel_messages={} approvals_denied={} questions_cancelled={} runtime_cancellations={}",
        session_id,
        expected_turn_id,
        outcome.stopped,
        outcome.turn_mismatch,
        held_channel_messages,
        outcome.denied_approvals,
        outcome.cancelled_questions,
        outcome.runtime_cancellations.len()
    );

    outcome
}

/// Emergency Stop for every foreground surface.
///
/// Shells must synchronously flip any transport-local handles first and pass
/// their session ids in `pre_signalled_sessions`. Core then applies the exact
/// same active-turn, Channel, durable-state, interaction and runtime cleanup
/// semantics for Desktop and HTTP. Every fallible convergence step is bounded;
/// the cancel flags and watchdogs are installed before the first await.
pub async fn stop_all_sessions(
    db: Arc<SessionDB>,
    pre_signalled_sessions: impl IntoIterator<Item = String>,
    already_signalled: bool,
) -> StopAllOutcome {
    let global_stop_cleanup_guard = Arc::new(super::active_turn::begin_global_stop_cleanup());
    let global_autonomy_stop_claim = Arc::new(begin_global_autonomy_stop());
    let mut stopped_sessions: HashSet<String> = pre_signalled_sessions.into_iter().collect();
    let mut durable_turn_ids = Vec::new();

    for active in super::active_turn::cancel_all_current() {
        stopped_sessions.insert(active.session_id.clone());
        if active.source.broadcasts_to_user_ui() {
            super::stream_broadcast::broadcast_turn_status(
                &active.session_id,
                &active.turn_id,
                ChatTurnStatus::Cancelling,
                Some(ChatTurnInterruptReason::UserStop),
            );
            super::spawn_user_stop_watchdog(
                db.clone(),
                active.session_id,
                active.turn_id.clone(),
                active.source,
            );
            durable_turn_ids.push(active.turn_id);
        }
    }
    if let Some(registry) = crate::globals::get_channel_cancels() {
        stopped_sessions.extend(registry.cancel_all());
    }
    stopped_sessions.extend(crate::subagent::active_parent_injection_session_ids());
    stopped_sessions.extend(crate::wakeup::pending_session_ids_for_global_stop().await);
    let (global_stop_epoch, autonomy_sessions, enumerated_paused_sessions, global_stop_dispatched) =
        match begin_global_stop_generation(
            db.clone(),
            stopped_sessions.iter().cloned().collect(),
            Arc::clone(&global_stop_cleanup_guard),
            Arc::clone(&global_autonomy_stop_claim),
        )
        .await
        {
            Ok(GlobalStopGenerationDispatch::Settled(outcome)) => (
                Some(outcome.epoch),
                outcome.session_ids,
                outcome.paused_session_ids,
                true,
            ),
            Ok(GlobalStopGenerationDispatch::Detached) => (None, Vec::new(), Vec::new(), true),
            Err(error) => {
                crate::app_warn!(
                    "chat",
                    "stop_all_sessions",
                    "Failed to publish/enumerate cross-process global Stop: {}",
                    error
                );
                (None, Vec::new(), Vec::new(), false)
            }
        };
    stopped_sessions.extend(autonomy_sessions);
    if !global_stop_dispatched {
        let _ = prepare_global_session_pauses(
            db.clone(),
            stopped_sessions.iter().cloned().collect(),
            enumerated_paused_sessions.into_iter().collect(),
            global_stop_epoch,
            Arc::clone(&global_stop_cleanup_guard),
            Arc::clone(&global_autonomy_stop_claim),
        )
        .await;
    }
    // Cancel same-process ACP/incognito coordinators immediately; other
    // processes observe the session-free global epoch on their next watcher
    // tick without ever persisting incognito session identity.
    converge_local_session_autonomy_stop_fences(db.clone()).await;
    // Parent injections live outside the generic runtime registry. Signal only
    // after their session receipts are durable so ordinary delivery requeue
    // cannot cross this global Stop generation.
    for session_id in &stopped_sessions {
        crate::subagent::request_pause_parent_injection(session_id);
    }
    let mark_db = db.clone();
    let mark_turns = async move {
        if durable_turn_ids.is_empty() {
            return;
        }
        let turn_ids_for_mark = durable_turn_ids.clone();
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            mark_db.run(move |db| {
                for turn_id in &turn_ids_for_mark {
                    if let Err(error) =
                        db.mark_chat_turn_cancelling(turn_id, ChatTurnInterruptReason::UserStop)
                    {
                        crate::app_warn!(
                            "chat",
                            "stop_all_sessions",
                            "Failed to mark stopped turn {} as cancelling: {}",
                            turn_id,
                            error
                        );
                    }
                }
            }),
        )
        .await
        {
            Ok(()) => {}
            Err(_) => crate::app_warn!(
                "chat",
                "stop_all_sessions",
                "Timed out after {}ms marking stopped turns as cancelling",
                STOP_DB_MARK_TIMEOUT.as_millis()
            ),
        }
    };
    let deny_approvals = timeout_count(
        "denying all pending approvals",
        crate::tools::deny_all_pending(ApprovalResolutionSource::UserStop),
    );
    let cancel_questions = timeout_count(
        "cancelling all pending questions",
        crate::ask_user::cancel_all_pending_ask_user_questions("user_stop"),
    );
    let cancel_runtime = timeout_runtime_cleanup(async {
        let snapshot = crate::runtime_tasks::snapshot_runtime_tasks_for_global_stop().await?;
        crate::runtime_tasks::cancel_runtime_task_snapshot(snapshot).await
    });
    let queue_db = db.clone();
    let hold_channel_queues = async move {
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            queue_db.run(|db| db.hold_all_channel_turn_messages_after_stop()),
        )
        .await
        {
            Ok(Ok(session_ids)) => session_ids,
            Ok(Err(error)) => {
                crate::app_warn!(
                    "chat",
                    "stop_all_sessions",
                    "Failed to hold queued IM messages during global Stop: {}",
                    error
                );
                Vec::new()
            }
            Err(_) => {
                crate::app_warn!(
                    "chat",
                    "stop_all_sessions",
                    "Timed out after {}ms holding queued IM messages during global Stop",
                    STOP_DB_MARK_TIMEOUT.as_millis()
                );
                Vec::new()
            }
        }
    };
    let ((), denied_approvals, cancelled_questions, runtime_result, held_channel_sessions) = tokio::join!(
        mark_turns,
        deny_approvals,
        cancel_questions,
        cancel_runtime,
        hold_channel_queues
    );
    let held_channel_session_count = held_channel_sessions.len();
    stopped_sessions.extend(held_channel_sessions);
    let mut outcome = StopAllOutcome {
        stopped: already_signalled || global_stop_dispatched || !stopped_sessions.is_empty(),
        stopped_session_count: stopped_sessions.len(),
        denied_approvals,
        cancelled_questions,
        ..Default::default()
    };
    match runtime_result {
        Ok(results) => outcome.runtime_cancellations = results,
        Err(error) => {
            crate::app_warn!(
                "chat",
                "stop_all_sessions",
                "Global runtime cancellation failed after stop signals: {}",
                error
            );
            outcome.runtime_cancellation_error = Some(error.to_string());
        }
    }
    outcome.stopped = outcome.stopped
        || outcome.denied_approvals > 0
        || outcome.cancelled_questions > 0
        || outcome
            .runtime_cancellations
            .iter()
            .any(|result| result.accepted);
    drop(global_autonomy_stop_claim);
    drop(global_stop_cleanup_guard);
    crate::app_info!(
        "chat",
        "stop_all_sessions",
        "Global stop settled: stopped={} sessions={} held_channel_sessions={} approvals_denied={} questions_cancelled={} runtime_cancellations={}",
        outcome.stopped,
        outcome.stopped_session_count,
        held_channel_session_count,
        outcome.denied_approvals,
        outcome.cancelled_questions,
        outcome.runtime_cancellations.len()
    );
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fixture() -> (tempfile::TempDir, Arc<SessionDB>, String, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("stop.db")).expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let turn = db
            .create_chat_turn(&session.id, "desktop", None, None)
            .expect("turn");
        (dir, db, session.id, turn.id)
    }

    #[tokio::test]
    async fn shared_stop_marks_and_signals_the_exact_turn() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            turn_id.clone(),
            cancel.clone(),
        )
        .expect("active turn");

        let outcome = stop_session(db.clone(), &session_id, Some(&turn_id), false).await;

        assert!(outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(cancel.load(Ordering::SeqCst));
        assert_eq!(
            db.get_chat_turn(&turn_id).unwrap().unwrap().status,
            ChatTurnStatus::Cancelling
        );
    }

    #[tokio::test]
    async fn exact_stop_after_completion_seal_does_not_claim_success() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Cron,
            turn_id.clone(),
            cancel.clone(),
        )
        .expect("active turn");
        assert!(guard.seal_completion(&turn_id));

        let outcome = stop_session(db.clone(), &session_id, Some(&turn_id), false).await;

        assert!(!outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(!cancel.load(Ordering::SeqCst));
        assert_eq!(
            db.get_chat_turn(&turn_id).unwrap().unwrap().status,
            ChatTurnStatus::Running
        );
    }

    #[tokio::test]
    async fn exact_cron_stop_does_not_pause_future_session_work() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("cron-stop.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let turn = db
            .create_chat_turn(&session.id, "cron", None, None)
            .expect("turn");
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Cron,
            turn.id.clone(),
            cancel.clone(),
        )
        .expect("active cron turn");

        let outcome = stop_session(db.clone(), &session.id, Some(&turn.id), false).await;

        assert!(outcome.stopped);
        assert!(cancel.load(Ordering::SeqCst));
        assert!(outcome.autonomy_pause.is_none());
        assert!(!db.is_session_autonomy_paused(&session.id).unwrap());
    }

    #[tokio::test]
    async fn exact_stale_stop_does_not_cancel_a_newer_turn() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            turn_id,
            cancel.clone(),
        )
        .expect("active turn");

        let outcome = stop_session(db, &session_id, Some("older-turn"), false).await;

        assert!(!outcome.stopped);
        assert!(outcome.turn_mismatch);
        assert!(!cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn exact_stop_converges_a_durable_turn_after_live_entry_vanished() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();

        let outcome = stop_session(db.clone(), &session_id, Some(&turn_id), false).await;

        assert!(outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(outcome.autonomy_pause.is_some());
        assert_eq!(
            db.get_chat_turn(&turn_id).unwrap().unwrap().status,
            ChatTurnStatus::Cancelling
        );
        assert!(db.is_session_autonomy_paused(&session_id).unwrap());
    }

    #[tokio::test]
    async fn exact_stop_for_a_terminal_old_turn_does_not_pause_the_session() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        db.finish_chat_turn_once(&turn_id, ChatTurnStatus::Completed, None, None, None)
            .expect("finish old turn");

        let outcome = stop_session(db.clone(), &session_id, Some(&turn_id), false).await;

        assert!(!outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(outcome.autonomy_pause.is_none());
        assert!(!db.is_session_autonomy_paused(&session_id).unwrap());
    }

    #[tokio::test]
    async fn global_stop_publishes_resumable_receipts_for_autonomous_sessions() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("global-stop.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Keep autonomous work resumable".to_string(),
                completion_criteria: "Continue restores the controller".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("goal");

        let outcome = stop_all_sessions(db.clone(), Vec::<String>::new(), false).await;

        assert!(outcome.stopped);
        assert_eq!(outcome.stopped_session_count, 1);
        assert!(db.is_session_autonomy_paused(&session.id).unwrap());
        assert_eq!(
            db.get_goal(&goal.goal.id).unwrap().unwrap().state,
            crate::goal::GoalState::Paused
        );

        let pause_id = db
            .active_session_autonomy_pause(&session.id)
            .expect("load pause")
            .expect("active pause")
            .id;
        let resumed = continue_session(db.clone(), &session.id, &pause_id)
            .await
            .expect("continue session");
        assert!(resumed.resumed);
        assert!(!db.is_session_autonomy_paused(&session.id).unwrap());
        assert_eq!(
            db.get_goal(&goal.goal.id).unwrap().unwrap().state,
            crate::goal::GoalState::Active
        );
    }

    #[tokio::test]
    async fn detached_global_generation_keeps_enumeration_connected_to_receipts() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("detached-global-stop.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let seed_only_session = db.create_session("ha-main").expect("seed-only session");
        let goal = db
            .create_goal(crate::goal::CreateGoalInput {
                session_id: session.id.clone(),
                objective: "Keep detached Stop enumeration durable".to_string(),
                completion_criteria: "The enumerated session receives a receipt".to_string(),
                domain: None,
                workflow_template_id: None,
                workflow_template_version: None,
                workflow_task_type: None,
                budget_token_limit: None,
                budget_time_limit_secs: None,
                budget_turn_limit: None,
            })
            .expect("goal");
        let transition_guard = autonomy_transition_lock().lock_owned().await;

        let dispatch = begin_global_stop_generation(
            db.clone(),
            vec![seed_only_session.id.clone()],
            Arc::new(crate::chat_engine::active_turn::begin_global_stop_cleanup()),
            Arc::new(begin_global_autonomy_stop()),
        )
        .await
        .expect("global Stop dispatch");
        assert!(matches!(dispatch, GlobalStopGenerationDispatch::Detached));
        drop(transition_guard);

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let receipt_active = db.is_session_autonomy_paused(&session.id).unwrap();
                let seed_receipt_active = db
                    .is_session_autonomy_paused(&seed_only_session.id)
                    .unwrap();
                let goal_paused = db
                    .get_goal(&goal.goal.id)
                    .unwrap()
                    .is_some_and(|goal| goal.state == crate::goal::GoalState::Paused);
                if receipt_active
                    && seed_receipt_active
                    && goal_paused
                    && !autonomy_stop_pending(&session.id)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("detached owner finishes receipt and controller pause");
        let global_stop_epoch = db.global_stop_epoch().expect("global Stop epoch");
        assert_eq!(
            db.session_lineage_global_stop_receipt_state(&seed_only_session.id, global_stop_epoch,)
                .expect("seed receipt attribution"),
            (true, false)
        );
    }

    #[tokio::test]
    async fn repeated_stop_supersedes_a_delayed_exact_continue() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("stop-generations.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");

        let first = stop_session(db.clone(), &session.id, None, false)
            .await
            .autonomy_pause
            .expect("first Stop receipt");
        let second = stop_session(db.clone(), &session.id, None, false)
            .await
            .autonomy_pause
            .expect("second Stop receipt");

        assert_ne!(first.id, second.id);
        assert!(continue_session(db.clone(), &session.id, &first.id)
            .await
            .is_err());
        assert_eq!(
            db.active_session_autonomy_pause(&session.id)
                .unwrap()
                .expect("new generation remains active")
                .id,
            second.id
        );
        assert!(
            continue_session(db.clone(), &session.id, &second.id)
                .await
                .expect("exact latest Continue")
                .resumed
        );
    }

    #[tokio::test]
    async fn secondary_continue_leaves_a_durable_primary_replay_handoff() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("secondary-continue.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("pause receipt");

        let resumed = continue_session_inner_for_tier(db.clone(), &session.id, &pause.id, false)
            .await
            .expect("Secondary Continue publishes durable handoff");

        assert!(resumed.resumed);
        assert!(!db.is_session_autonomy_paused(&session.id).unwrap());
        let pending = db
            .list_pending_session_autonomy_resume_replays(10)
            .expect("list durable resume handoffs");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, pause.id);

        replay_pending_session_autonomy_resumes_for_tier(db.clone(), true).await;

        assert!(db
            .list_pending_session_autonomy_resume_replays(10)
            .expect("Primary acknowledged handoff")
            .is_empty());
    }

    #[tokio::test]
    async fn cross_process_stop_epoch_cancels_an_injection_after_fast_continue() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("stop-handoff.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let admitted_epoch = db
            .session_autonomy_lineage_pause_epoch(&session.id)
            .expect("initial Stop epoch");
        let cancel = Arc::new(AtomicBool::new(false));
        crate::subagent::INJECTION_CANCELS
            .lock()
            .expect("injection registry")
            .insert(
                session.id.clone(),
                crate::subagent::ActiveInjection {
                    run_id: "run-cross-process-stop".into(),
                    cancel: cancel.clone(),
                    admitted_pause_epoch: admitted_epoch,
                    im_mirror: Arc::new(
                        crate::subagent::injection::ActiveInjectionMirrorCoordinator::new(None),
                    ),
                },
            );
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("durable Stop receipt");
        assert!(db.finish_session_autonomy_resume(&pause.id).unwrap());

        converge_local_session_autonomy_stop_fences(db).await;

        assert!(cancel.load(Ordering::SeqCst));
        crate::subagent::INJECTION_CANCELS
            .lock()
            .expect("injection registry")
            .remove(&session.id);
    }

    #[test]
    fn cross_process_stop_epoch_resolves_an_old_foreground_stream() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("foreground-stop-handoff.db"))
            .expect("session db");
        let session = db.create_session("ha-main").expect("session");
        let registration = db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: "remote-foreground-stop-run".to_string(),
                session_id: session.id.clone(),
                source: "acp".to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect("foreground stream admission");
        assert_eq!(registration.admitted_stop_epoch, 0);
        let pause = db
            .prepare_session_autonomy_pause(&session.id)
            .expect("durable Stop receipt");
        assert!(db.finish_session_autonomy_resume(&pause.id).unwrap());

        let (_, _, _, stale_foreground_runs) = db
            .resolve_local_autonomy_stop_fences(
                &[(
                    registration.run_id.clone(),
                    session.id,
                    registration.admitted_stop_epoch,
                    registration.admitted_global_stop_epoch,
                    registration.admitted_global_stop_receipt_count,
                )],
                &[],
                &[],
                &[],
            )
            .expect("resolve foreground Stop generation after fast Continue");
        assert_eq!(stale_foreground_runs, vec![registration.run_id]);
    }

    #[test]
    fn global_stop_epoch_cancels_incognito_without_persisting_stream_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = SessionDB::open_ephemeral_for_test(&dir.path().join("incognito-global-stop.db"))
            .expect("session db");
        let session = db
            .create_session_with_project("ha-main", None, Some(true))
            .expect("incognito session");
        let registration = db
            .create_stream_run(&crate::session::CreateStreamRun {
                run_id: "incognito-global-stop-run".to_string(),
                session_id: session.id.clone(),
                source: "http".to_string(),
                stream_id: None,
                turn_id: None,
                provider_shape: None,
            })
            .expect("incognito foreground admission");
        assert!(!registration.persistent);
        assert!(db.recoverable_stream_runs().unwrap().is_empty());

        let (epoch, durable_sessions) = db
            .begin_global_stop_enumeration()
            .expect("publish global Stop epoch");
        assert_eq!(epoch, 1);
        assert!(durable_sessions.is_empty());
        let (_, _, _, stale_foreground_runs) = db
            .resolve_local_autonomy_stop_fences(
                &[(
                    registration.run_id.clone(),
                    session.id.clone(),
                    registration.admitted_stop_epoch,
                    registration.admitted_global_stop_epoch,
                    registration.admitted_global_stop_receipt_count,
                )],
                &[],
                &[],
                &[],
            )
            .expect("resolve incognito global Stop fence");
        assert_eq!(stale_foreground_runs, vec![registration.run_id]);
    }

    #[tokio::test]
    async fn global_stop_normalizes_hidden_child_targets_to_the_visible_root() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("global-stop-root.db"))
                .expect("session db"),
        );
        let root = db.create_session("ha-main").expect("root session");
        let child = db
            .create_session_with_parent("helper", Some(&root.id))
            .expect("hidden child session");

        let outcome = stop_all_sessions(db.clone(), vec![child.id.clone()], false).await;

        assert!(outcome.stopped);
        assert!(db.is_session_autonomy_paused(&root.id).unwrap());
        assert!(!db.is_session_autonomy_paused(&child.id).unwrap());
        let inherited = db
            .active_session_or_ancestor_autonomy_pause(&child.id)
            .unwrap()
            .expect("child inherits root receipt");
        assert_eq!(inherited.session_id, root.id);
    }

    #[tokio::test]
    async fn shared_stop_cancels_channel_turn_without_a_gui_turn_row() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("channel-stop.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        db.enqueue_turn_user_message(crate::session::NewQueuedTurnMessage {
            request_id: "channel-queued-after-stop".to_string(),
            session_id: session.id.clone(),
            message: "next".to_string(),
            display_text: Some("next".to_string()),
            attachments: Vec::new(),
            is_plan_trigger: false,
            goal_trigger: false,
            plan_comment: None,
            plan_mode: None,
            workflow_mode: None,
            incoming_turn: None,
            skill_allowed_tools: Vec::new(),
            ui_dispatch_fingerprint: None,
            source: crate::session::QueuedTurnMessageSource::Channel,
            channel_origin: Some(serde_json::json!({"channelId": "wechat"})),
        })
        .expect("enqueue channel row");
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Channel,
            "channel-synthetic-turn".to_string(),
            cancel.clone(),
        )
        .expect("active turn");

        let outcome = stop_session(db.clone(), &session.id, None, true).await;

        assert!(outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(cancel.load(Ordering::SeqCst));
        let queued = db
            .list_queued_turn_user_messages(&session.id)
            .expect("list queue");
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued[0].status,
            crate::session::QueuedTurnMessageStatus::HeldAfterStop
        );
    }

    #[tokio::test]
    async fn pre_turn_cancel_cleanup_deletes_lazy_session_and_releases_gate() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("pre-turn-cleanup.db"))
                .expect("session db"),
        );
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("channel schema");
        let session = db.create_session("ha-main").expect("session");
        let cleanup = PreTurnCancelCleanup::begin(db.clone(), session.id.clone(), None, true, None)
            .expect("new lazy sessions require cleanup");

        assert!(crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Desktop,
            "blocked-during-cleanup".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .is_err());

        cleanup.run().await;

        let sid = session.id.clone();
        assert!(db
            .clone()
            .run(move |db| db.get_session(&sid))
            .await
            .expect("read session")
            .is_none());
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Desktop,
            "after-cleanup".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("cleanup gate must be released");
    }

    #[tokio::test]
    async fn pre_turn_cancel_cleanup_releases_exact_queued_dispatch() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("pre-turn-queue-cleanup.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        db.enqueue_turn_user_message(crate::session::NewQueuedTurnMessage {
            request_id: "queued-request".to_string(),
            session_id: session.id.clone(),
            message: "queued message".to_string(),
            display_text: None,
            attachments: Vec::new(),
            is_plan_trigger: false,
            goal_trigger: false,
            plan_comment: None,
            plan_mode: None,
            workflow_mode: None,
            incoming_turn: None,
            skill_allowed_tools: Vec::new(),
            ui_dispatch_fingerprint: None,
            source: crate::session::QueuedTurnMessageSource::Desktop,
            channel_origin: None,
        })
        .expect("enqueue");
        db.claim_queued_turn_message_for_dispatch(
            &session.id,
            "queued-request",
            "stopped-turn",
            crate::session::QueuedTurnMessageSource::Desktop,
        )
        .expect("claim queue row")
        .expect("queued row");

        let cleanup = PreTurnCancelCleanup::begin(
            db.clone(),
            session.id.clone(),
            None,
            false,
            Some(("queued-request".to_string(), "stopped-turn".to_string())),
        )
        .expect("queued dispatch requires cleanup");
        cleanup.run().await;

        let queue = db
            .list_queued_turn_user_messages(&session.id)
            .expect("list queue");
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue[0].status,
            crate::session::QueuedTurnMessageStatus::Queued
        );
        assert_eq!(queue[0].turn_id, None);
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Desktop,
            "after-queue-cleanup".to_string(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("cleanup gate must be released");
    }

    /// Issue #657: a crash-recovered turn is durable-terminal, so Stop settles
    /// nothing and — critically — arms no broadcast/watchdog. The outcome must
    /// say so, otherwise a UI still showing the spinner waits for an event that
    /// can never arrive and the user is left clicking Stop forever.
    #[tokio::test]
    async fn exact_stop_on_a_crash_recovered_turn_reports_nothing_to_await() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        assert_eq!(db.recover_stale_chat_turns().expect("recover"), 1);
        let recovered = db.get_chat_turn(&turn_id).unwrap().unwrap();
        assert_eq!(recovered.status, ChatTurnStatus::Interrupted);
        assert_eq!(
            recovered.interrupt_reason,
            Some(ChatTurnInterruptReason::CrashRecovery)
        );

        let outcome = stop_session(db.clone(), &session_id, Some(&turn_id), false).await;

        assert!(!outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(!outcome.active_turn_found);
        assert!(!outcome.completion_sealed);
        assert!(!outcome.terminal_event_pending);
        // A stale exact Stop still must not fence autonomous work created by a
        // newer generation.
        assert!(outcome.autonomy_pause.is_none());
        assert!(!db.is_session_autonomy_paused(&session_id).unwrap());
        assert_eq!(
            db.get_chat_turn(&turn_id).unwrap().unwrap().status,
            ChatTurnStatus::Interrupted
        );
    }

    /// The session-scoped variant does settle the session (durable pause), but
    /// there is no turn to broadcast for, so it too promises no terminal event.
    #[tokio::test]
    async fn session_stop_without_a_live_turn_reports_nothing_to_await() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        db.recover_stale_chat_turns().expect("recover");

        let outcome = stop_session(db.clone(), &session_id, None, false).await;

        assert!(outcome.stopped);
        assert!(!outcome.active_turn_found);
        assert!(!outcome.terminal_event_pending);
        assert!(outcome.autonomy_pause.is_some());
        assert_eq!(
            db.get_chat_turn(&turn_id).unwrap().unwrap().status,
            ChatTurnStatus::Interrupted
        );
    }

    /// Positive control for the two cases above: a live user-visible turn arms
    /// the `cancelling` broadcast plus the watchdog, so callers must keep
    /// waiting instead of reconciling the session out from under it.
    #[tokio::test]
    async fn live_turn_stop_promises_a_terminal_event() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            turn_id.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("active turn");

        let outcome = stop_session(db, &session_id, Some(&turn_id), false).await;

        assert!(outcome.stopped);
        assert!(outcome.active_turn_found);
        assert!(outcome.terminal_event_pending);
        assert!(!outcome.completion_sealed);
    }

    /// A sealed completion is not stale state: the executor is already on its
    /// way to a terminal event, so it must stay distinguishable from the
    /// crash-recovery case above.
    #[tokio::test]
    async fn completion_sealed_stop_is_distinguishable_from_a_stale_turn() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let (_dir, db, session_id, turn_id) = fixture();
        let guard = crate::chat_engine::active_turn::try_acquire(
            &session_id,
            crate::chat_engine::ChatSource::Desktop,
            turn_id.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("active turn");
        assert!(guard.seal_completion(&turn_id));

        let outcome = stop_session(db, &session_id, Some(&turn_id), false).await;

        assert!(!outcome.stopped);
        assert!(outcome.active_turn_found);
        assert!(outcome.completion_sealed);
        assert!(!outcome.terminal_event_pending);
    }

    #[test]
    fn stop_chat_result_projects_the_session_outcome_faithfully() {
        let latched = StopChatResult::latched();
        assert!(latched.stopped && latched.latched);
        assert!(!latched.terminal_event_pending);

        let stale = StopChatResult::from_session_outcome(
            "session",
            StopSessionOutcome {
                stopped: false,
                ..Default::default()
            },
        );
        assert!(!stale.stopped);
        assert!(!stale.latched);
        assert!(!stale.active_turn_found);
        assert!(!stale.terminal_event_pending);
        assert_eq!(
            stale.reason.as_deref(),
            Some("no matching active chat for target")
        );

        let live = StopChatResult::from_session_outcome(
            "session",
            StopSessionOutcome {
                stopped: true,
                active_turn_found: true,
                terminal_event_pending: true,
                ..Default::default()
            },
        );
        assert!(live.stopped && live.terminal_event_pending);
        assert!(live.reason.is_none());
    }
}

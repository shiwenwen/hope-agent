//! Shared foreground-session Stop orchestration.
//!
//! Transport/UI entry points may differ in how they identify a turn, but once
//! a session target is known they must converge through this module so Desktop,
//! HTTP, and IM `/stop` resolve interaction waits and owned runtime work with
//! the same semantics.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashSet, future::Future};

use crate::runtime_tasks::CancelRuntimeTaskResult;
use crate::session::{ChatTurnInterruptReason, ChatTurnStatus, SessionDB};
use crate::tools::ApprovalResolutionSource;

const STOP_DB_MARK_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_INTERACTION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_RUNTIME_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct StopSessionOutcome {
    pub stopped: bool,
    pub turn_mismatch: bool,
    pub denied_approvals: usize,
    pub cancelled_questions: usize,
    pub runtime_cancellations: Vec<CancelRuntimeTaskResult>,
    pub runtime_cancellation_error: Option<String>,
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

/// Cleanup owned state for a prompt that was cancelled before a durable chat
/// turn existed. Construction installs the session cleanup gate immediately;
/// callers may then publish/release any transport-visible lifecycle before
/// spawning the potentially blocking Git/SQLite work.
pub struct PreTurnCancelCleanup {
    db: Arc<SessionDB>,
    session_id: String,
    bootstrap_request_id: Option<String>,
    delete_empty_session: bool,
    cleanup_gate: super::active_turn::StopCleanupGuard,
}

impl PreTurnCancelCleanup {
    pub fn begin(
        db: Arc<SessionDB>,
        session_id: String,
        bootstrap_request_id: Option<String>,
        delete_empty_session: bool,
    ) -> Option<Self> {
        if bootstrap_request_id.is_none() && !delete_empty_session {
            return None;
        }
        let cleanup_gate = super::active_turn::begin_stop_cleanup(&session_id);
        Some(Self {
            db,
            session_id,
            bootstrap_request_id,
            delete_empty_session,
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
            cleanup_gate,
        } = self;
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
            "Pre-turn Stop cleanup settled: session={} bootstrap={} bootstrap_rollback_succeeded={} delete_requested={} session_deleted={}",
            session_id,
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
    let mut stop_cleanup_guard = Some(super::active_turn::begin_stop_cleanup(session_id));
    let mut outcome = StopSessionOutcome {
        stopped: already_signalled,
        ..Default::default()
    };
    let mut matched_active = false;
    let mut durable_turn_id = None;

    if let Some(active) = super::active_turn::current(session_id) {
        let matches_turn = expected_turn_id
            .map(|turn_id| turn_id == active.turn_id)
            .unwrap_or(true);
        if matches_turn {
            matched_active = true;
            outcome.stopped = true;
            active.cancel.store(true, Ordering::SeqCst);

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
                durable_turn_id = Some(active.turn_id);
            }
        } else {
            outcome.turn_mismatch = true;
        }
    }

    // A session-only Stop is authoritative even if the active entry vanished
    // between UI observation and this call. An exact stale turn must not tear
    // down Channel preflight, waits, or runtime that may belong to a newer turn.
    let settle_session = expected_turn_id.is_none() || matched_active;
    if !settle_session {
        stop_cleanup_guard.take();
    }
    if settle_session {
        let channel_signalled = crate::globals::get_channel_cancels()
            .map(|registry| registry.cancel(session_id))
            .unwrap_or(false);
        outcome.stopped |= channel_signalled;
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
                crate::runtime_tasks::snapshot_runtime_tasks_for_session(Some(&runtime_session_id))
                    .await?;
            crate::runtime_tasks::cancel_runtime_task_snapshot(snapshot).await
        })
        .await
    };
    let ((), denied_approvals, cancelled_questions, runtime_result) =
        tokio::join!(mark_turn, deny_approvals, cancel_questions, cancel_runtime);
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
            || outcome.denied_approvals > 0
            || outcome.cancelled_questions > 0
            || outcome
                .runtime_cancellations
                .iter()
                .any(|result| result.accepted);
    }

    drop(stop_cleanup_guard);

    crate::app_info!(
        "chat",
        "stop_session",
        "Session stop settled: session={} expected_turn={:?} stopped={} turn_mismatch={} approvals_denied={} questions_cancelled={} runtime_cancellations={}",
        session_id,
        expected_turn_id,
        outcome.stopped,
        outcome.turn_mismatch,
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
    let global_stop_cleanup_guard = super::active_turn::begin_global_stop_cleanup();
    let mut stopped_sessions: HashSet<String> = pre_signalled_sessions.into_iter().collect();
    let mut durable_turn_ids = Vec::new();

    for active in super::active_turn::all_current() {
        active.cancel.store(true, Ordering::SeqCst);
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
    let mark_turns = async {
        if durable_turn_ids.is_empty() {
            return;
        }
        let turn_ids_for_mark = durable_turn_ids.clone();
        match tokio::time::timeout(
            STOP_DB_MARK_TIMEOUT,
            db.run(move |db| {
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
        let snapshot = crate::runtime_tasks::snapshot_runtime_tasks_for_session(None).await?;
        crate::runtime_tasks::cancel_runtime_task_snapshot(snapshot).await
    });

    let ((), denied_approvals, cancelled_questions, runtime_result) =
        tokio::join!(mark_turns, deny_approvals, cancel_questions, cancel_runtime);
    let mut outcome = StopAllOutcome {
        stopped: already_signalled || !stopped_sessions.is_empty(),
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
    drop(global_stop_cleanup_guard);
    crate::app_info!(
        "chat",
        "stop_all_sessions",
        "Global stop settled: stopped={} sessions={} approvals_denied={} questions_cancelled={} runtime_cancellations={}",
        outcome.stopped,
        outcome.stopped_session_count,
        outcome.denied_approvals,
        outcome.cancelled_questions,
        outcome.runtime_cancellations.len()
    );
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

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
    async fn exact_stale_stop_does_not_cancel_a_newer_turn() {
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
    async fn shared_stop_cancels_channel_turn_without_a_gui_turn_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("channel-stop.db"))
                .expect("session db"),
        );
        let session = db.create_session("ha-main").expect("session");
        let cancel = Arc::new(AtomicBool::new(false));
        let _guard = crate::chat_engine::active_turn::try_acquire(
            &session.id,
            crate::chat_engine::ChatSource::Channel,
            "channel-synthetic-turn".to_string(),
            cancel.clone(),
        )
        .expect("active turn");

        let outcome = stop_session(db, &session.id, None, true).await;

        assert!(outcome.stopped);
        assert!(!outcome.turn_mismatch);
        assert!(cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn pre_turn_cancel_cleanup_deletes_lazy_session_and_releases_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            SessionDB::open_ephemeral_for_test(&dir.path().join("pre-turn-cleanup.db"))
                .expect("session db"),
        );
        crate::channel::ChannelDB::new(db.clone())
            .migrate()
            .expect("channel schema");
        let session = db.create_session("ha-main").expect("session");
        let cleanup = PreTurnCancelCleanup::begin(db.clone(), session.id.clone(), None, true)
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
}

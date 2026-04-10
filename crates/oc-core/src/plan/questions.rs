use anyhow::Result;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use super::types::{PlanQuestionAnswer, PlanQuestionGroup};

// ── EventBus event names ─────────────────────────────────────────
//
// The tool backend emits BOTH names for every group so historical listeners
// keep working (`plan_question_request` is the legacy name that pre-dates the
// rename to the generic `ask_user_question` tool).

/// Canonical event name for an interactive user-question request.
pub const EVENT_ASK_USER_REQUEST: &str = "ask_user_request";
/// Legacy alias for [`EVENT_ASK_USER_REQUEST`]. Still emitted for
/// backwards compatibility with older frontend code paths.
pub const EVENT_PLAN_QUESTION_REQUEST: &str = "plan_question_request";

// ── Pending Plan Questions Registry (oneshot pattern) ────────────

static PENDING_PLAN_QUESTIONS: OnceLock<
    TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>>>,
> = OnceLock::new();

fn get_pending_questions(
) -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>>> {
    PENDING_PLAN_QUESTIONS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Register a pending question and return the receiver.
pub async fn register_plan_question(
    request_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<PlanQuestionAnswer>>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(request_id, sender);
}

/// Submit answers from the frontend (called by Tauri command).
pub async fn submit_plan_question_response(
    request_id: &str,
    answers: Vec<PlanQuestionAnswer>,
) -> Result<()> {
    let mut pending = get_pending_questions().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(answers);
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "No pending plan question request: {}",
            request_id
        ))
    }
}

/// Cancel a pending question (e.g., on plan exit).
pub async fn cancel_pending_plan_question(request_id: &str) {
    let mut pending = get_pending_questions().lock().await;
    pending.remove(request_id);
}

/// Check whether a request_id is currently awaited by a live tool call
/// (in-memory oneshot registered). Used to filter out zombie rows left over
/// from a previous process that can no longer receive answers.
pub async fn is_plan_question_live(request_id: &str) -> bool {
    get_pending_questions().lock().await.contains_key(request_id)
}

/// Return the most recent still-pending question group for the given session
/// that is also awaited by a live in-memory oneshot. Zombie DB rows whose
/// tool call no longer exists are skipped so the frontend never tries to
/// answer them.
pub async fn find_live_pending_group_for_session(
    db: &crate::session::SessionDB,
    session_id: &str,
) -> anyhow::Result<Option<PlanQuestionGroup>> {
    let groups = db.list_pending_ask_user_groups_for_session(session_id)?;
    for group in groups.into_iter().rev() {
        if is_plan_question_live(&group.request_id).await {
            return Ok(Some(group));
        }
    }
    Ok(None)
}

// ── SQLite Persistence ──────────────────────────────────────────

/// Persist a pending question group so a restart can resume it.
/// No-op when the session DB isn't initialised (e.g. during tests).
pub fn persist_pending_group(group: &PlanQuestionGroup) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        return Ok(());
    };
    db.save_ask_user_group(group)
}

/// Mark a persisted question group as answered so it won't be replayed on
/// next startup. No-op when the session DB isn't initialised.
pub fn mark_group_answered(request_id: &str) -> Result<()> {
    let Some(db) = crate::get_session_db() else {
        return Ok(());
    };
    db.mark_ask_user_answered(request_id)
}

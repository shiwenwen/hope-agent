use anyhow::Result;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use super::types::{AskUserQuestionAnswer, AskUserQuestionGroup};

// ── EventBus event names ─────────────────────────────────────────

/// Canonical event name for an interactive user-question request.
pub const EVENT_ASK_USER_REQUEST: &str = "ask_user_request";

// ── Pending Ask-User Questions Registry (oneshot pattern) ────────

static PENDING_ASK_USER_QUESTIONS: OnceLock<
    TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>>>,
> = OnceLock::new();

fn get_pending_questions(
) -> &'static TokioMutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>>>
{
    PENDING_ASK_USER_QUESTIONS.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Register a pending question and return the receiver.
pub async fn register_ask_user_question(
    request_id: String,
    sender: tokio::sync::oneshot::Sender<Vec<AskUserQuestionAnswer>>,
) {
    let mut pending = get_pending_questions().lock().await;
    pending.insert(request_id, sender);
}

/// Submit answers from the frontend (called by Tauri command).
pub async fn submit_ask_user_question_response(
    request_id: &str,
    answers: Vec<AskUserQuestionAnswer>,
) -> Result<()> {
    let mut pending = get_pending_questions().lock().await;
    if let Some(sender) = pending.remove(request_id) {
        let _ = sender.send(answers);
        drop(pending);
        crate::tools::approval::emit_pending_interactions_changed(None);
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "No pending ask_user_question request: {}",
            request_id
        ))
    }
}

/// Cancel a pending question (e.g., on timeout or user cancel).
pub async fn cancel_pending_ask_user_question(request_id: &str) {
    let mut pending = get_pending_questions().lock().await;
    pending.remove(request_id);
    drop(pending);
    crate::tools::approval::emit_pending_interactions_changed(None);
}

/// Check whether a request_id is currently awaited by a live tool call
/// (in-memory oneshot registered). Used to filter out zombie rows left over
/// from a previous process that can no longer receive answers.
pub async fn is_ask_user_question_live(request_id: &str) -> bool {
    get_pending_questions().lock().await.contains_key(request_id)
}

/// Return the most recent still-pending question group for the given session
/// that is also awaited by a live in-memory oneshot. Zombie DB rows whose
/// tool call no longer exists are skipped so the frontend never tries to
/// answer them.
pub async fn find_live_pending_group_for_session(
    session_id: &str,
) -> anyhow::Result<Option<AskUserQuestionGroup>> {
    let Some(db) = crate::get_session_db() else {
        return Ok(None);
    };
    let groups = db.list_pending_ask_user_groups_for_session(session_id)?;
    for group in groups.into_iter().rev() {
        if is_ask_user_question_live(&group.request_id).await {
            return Ok(Some(group));
        }
    }
    Ok(None)
}

// ── SQLite Persistence ──────────────────────────────────────────

/// Persist a pending question group so a restart can resume it.
/// No-op when the session DB isn't initialised (e.g. during tests).
pub fn persist_pending_group(group: &AskUserQuestionGroup) -> Result<()> {
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

use anyhow::Result;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex as TokioMutex;

use super::types::PlanQuestionAnswer;

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

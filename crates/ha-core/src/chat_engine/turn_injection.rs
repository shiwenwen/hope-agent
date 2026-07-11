//! Durable user-message queue orchestration for active desktop / HTTP turns.

use serde::Serialize;

use super::active_turn;

pub type QueuedTurnUserMessage = crate::session::QueuedTurnMessageRecord;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTurnUserMessageResult {
    pub queued: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<crate::session::QueuedTurnMessageView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnMessageResult {
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn request_insertion(
    db: &crate::session::SessionDB,
    session_id: &str,
    turn_id: &str,
    request_id: &str,
) -> anyhow::Result<QueueTurnUserMessageResult> {
    let queued = match active_turn::with_insertion_target(session_id, turn_id, || {
        db.request_turn_message_insertion(session_id, request_id, turn_id)
    }) {
        Ok(result) => result?,
        Err(reason) => {
            return Ok(QueueTurnUserMessageResult {
                queued: false,
                request_id: request_id.to_string(),
                reason: Some(reason.to_string()),
                item: db
                    .get_queued_turn_user_message(session_id, request_id)?
                    .as_ref()
                    .map(crate::session::QueuedTurnMessageView::from),
            });
        }
    };
    let item = db
        .get_queued_turn_user_message(session_id, request_id)?
        .as_ref()
        .map(crate::session::QueuedTurnMessageView::from);
    Ok(QueueTurnUserMessageResult {
        queued,
        request_id: request_id.to_string(),
        reason: (!queued).then(|| "queued message is no longer insertable".to_string()),
        item,
    })
}

pub fn cancel_insertion(
    db: &crate::session::SessionDB,
    session_id: &str,
    turn_id: &str,
    request_id: &str,
) -> anyhow::Result<CancelQueuedTurnMessageResult> {
    let cancelled = db.cancel_turn_message_insertion(session_id, request_id, turn_id)?;
    Ok(CancelQueuedTurnMessageResult {
        cancelled,
        reason: (!cancelled).then(|| "message already entered an insertion boundary".to_string()),
    })
}

pub(crate) fn drain(session_id: &str, turn_id: &str) -> Vec<QueuedTurnUserMessage> {
    crate::get_session_db()
        .and_then(|db| {
            db.claim_turn_messages_for_insertion(session_id, turn_id)
                .ok()
        })
        .unwrap_or_default()
}

pub(crate) fn clear_turn(session_id: &str, turn_id: &str) {
    // Close the active-turn gate first. `request_insertion` holds that gate
    // through its DB transition, so this fallback cannot miss a late writer.
    active_turn::stop_accepting_insertions(session_id, turn_id);
    if let Some(db) = crate::get_session_db() {
        let _ = db.fallback_turn_message_insertions(session_id, turn_id);
    }
}

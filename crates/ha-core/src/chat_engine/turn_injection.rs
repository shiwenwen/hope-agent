//! In-memory user-message injection queue for active desktop / HTTP turns.
//!
//! The queue stores intent only. Messages are persisted and added to the
//! provider-native conversation history at a safe tool-loop boundary, after
//! assistant tool calls have received their matching tool results.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use crate::agent::Attachment;

use super::active_turn;
use super::stream_seq::ChatSource;

#[derive(Debug, Clone)]
pub struct QueuedTurnUserMessage {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub message: String,
    pub display_text: Option<String>,
    pub attachments: Vec<Attachment>,
    pub is_plan_trigger: bool,
    pub plan_comment: Option<Value>,
    pub source: ChatSource,
}

#[derive(Debug, Clone)]
pub struct QueueTurnUserMessageArgs {
    pub request_id: Option<String>,
    pub session_id: String,
    pub turn_id: String,
    pub message: String,
    pub display_text: Option<String>,
    pub attachments: Vec<Attachment>,
    pub is_plan_trigger: bool,
    pub plan_comment: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTurnUserMessageResult {
    pub queued: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQueuedTurnMessageResult {
    pub cancelled: bool,
}

type QueueKey = (String, String);

static QUEUES: OnceLock<Mutex<HashMap<QueueKey, VecDeque<QueuedTurnUserMessage>>>> =
    OnceLock::new();

fn registry() -> &'static Mutex<HashMap<QueueKey, VecDeque<QueuedTurnUserMessage>>> {
    QUEUES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(session_id: &str, turn_id: &str) -> QueueKey {
    (session_id.to_string(), turn_id.to_string())
}

pub fn enqueue(args: QueueTurnUserMessageArgs) -> QueueTurnUserMessageResult {
    let request_id = args
        .request_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let Some(active) = active_turn::current(&args.session_id) else {
        return QueueTurnUserMessageResult {
            queued: false,
            request_id,
            reason: Some("no active turn for session".to_string()),
        };
    };
    if active.turn_id != args.turn_id {
        return QueueTurnUserMessageResult {
            queued: false,
            request_id,
            reason: Some("active turn id does not match".to_string()),
        };
    }
    if active.cancel.load(Ordering::SeqCst) {
        return QueueTurnUserMessageResult {
            queued: false,
            request_id,
            reason: Some("active turn is cancelling".to_string()),
        };
    }
    if !matches!(active.source, ChatSource::Desktop | ChatSource::Http) {
        return QueueTurnUserMessageResult {
            queued: false,
            request_id,
            reason: Some("active turn source does not support injection".to_string()),
        };
    }

    let item = QueuedTurnUserMessage {
        request_id: request_id.clone(),
        session_id: args.session_id.clone(),
        turn_id: args.turn_id.clone(),
        message: args.message,
        display_text: args.display_text,
        attachments: args.attachments,
        is_plan_trigger: args.is_plan_trigger,
        plan_comment: args.plan_comment,
        source: active.source,
    };
    let mut map = registry()
        .lock()
        .expect("turn injection registry poisoned");
    let queue = map.entry(key(&args.session_id, &args.turn_id)).or_default();
    if queue.iter().any(|existing| existing.request_id == request_id) {
        return QueueTurnUserMessageResult {
            queued: true,
            request_id,
            reason: None,
        };
    }
    queue.push_back(item);

    QueueTurnUserMessageResult {
        queued: true,
        request_id,
        reason: None,
    }
}

pub fn cancel(
    session_id: &str,
    turn_id: &str,
    request_id: &str,
) -> CancelQueuedTurnMessageResult {
    let mut map = registry()
        .lock()
        .expect("turn injection registry poisoned");
    let queue_key = key(session_id, turn_id);
    let (cancelled, empty) = {
        let Some(queue) = map.get_mut(&queue_key) else {
            return CancelQueuedTurnMessageResult { cancelled: false };
        };
        let before = queue.len();
        queue.retain(|item| item.request_id != request_id);
        (queue.len() != before, queue.is_empty())
    };
    if empty {
        map.remove(&queue_key);
    }
    CancelQueuedTurnMessageResult { cancelled }
}

pub(crate) fn drain(session_id: &str, turn_id: &str) -> Vec<QueuedTurnUserMessage> {
    let mut map = registry()
        .lock()
        .expect("turn injection registry poisoned");
    map.remove(&key(session_id, turn_id))
        .unwrap_or_default()
        .into_iter()
        .collect()
}

pub(crate) fn clear_turn(session_id: &str, turn_id: &str) {
    let mut map = registry()
        .lock()
        .expect("turn injection registry poisoned");
    map.remove(&key(session_id, turn_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn cancel_missing_queue_is_false() {
        let result = cancel("missing-session", "missing-turn", "missing-request");
        assert!(!result.cancelled);
    }

    #[test]
    fn enqueue_and_drain_preserves_fifo_order() {
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let _guard = active_turn::try_acquire(
            &session_id,
            ChatSource::Desktop,
            turn_id.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("acquire active turn");

        let first = enqueue(QueueTurnUserMessageArgs {
            request_id: Some("first".to_string()),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            message: "first message".to_string(),
            display_text: None,
            attachments: Vec::new(),
            is_plan_trigger: false,
            plan_comment: None,
        });
        let second = enqueue(QueueTurnUserMessageArgs {
            request_id: Some("second".to_string()),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            message: "second message".to_string(),
            display_text: None,
            attachments: Vec::new(),
            is_plan_trigger: false,
            plan_comment: None,
        });

        assert!(first.queued);
        assert!(second.queued);
        let drained = drain(&session_id, &turn_id);
        let ids: Vec<_> = drained.iter().map(|item| item.request_id.as_str()).collect();
        assert_eq!(ids, vec!["first", "second"]);
        assert!(drain(&session_id, &turn_id).is_empty());
    }

    #[test]
    fn cancel_removes_only_matching_request() {
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let _guard = active_turn::try_acquire(
            &session_id,
            ChatSource::Http,
            turn_id.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("acquire active turn");

        for id in ["keep", "drop"] {
            assert!(
                enqueue(QueueTurnUserMessageArgs {
                    request_id: Some(id.to_string()),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message: id.to_string(),
                    display_text: None,
                    attachments: Vec::new(),
                    is_plan_trigger: false,
                    plan_comment: None,
                })
                .queued
            );
        }

        assert!(cancel(&session_id, &turn_id, "drop").cancelled);
        let drained = drain(&session_id, &turn_id);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].request_id, "keep");
    }

    #[test]
    fn enqueue_is_idempotent_by_request_id() {
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let _guard = active_turn::try_acquire(
            &session_id,
            ChatSource::Desktop,
            turn_id.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("acquire active turn");

        for message in ["first", "duplicate"] {
            assert!(
                enqueue(QueueTurnUserMessageArgs {
                    request_id: Some("same-request".to_string()),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    message: message.to_string(),
                    display_text: None,
                    attachments: Vec::new(),
                    is_plan_trigger: false,
                    plan_comment: None,
                })
                .queued
            );
        }

        let drained = drain(&session_id, &turn_id);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].request_id, "same-request");
        assert_eq!(drained[0].message, "first");
    }
}

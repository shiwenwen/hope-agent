use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use super::injection::flush_pending_injections;
use super::ACTIVE_CHAT_SESSIONS;
use super::INJECTION_CANCELS;
use super::SESSION_IDLE_NOTIFY;

// ── Steer Mailbox ───────────────────────────────────────────────

/// Per-run message queue for steering running sub-agents.
/// Parent agents push steer messages; the child agent's tool loop drains them each round.
pub struct SubagentMailbox {
    messages: Mutex<HashMap<String, Vec<SubagentMailboxMessage>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentMailboxMessage {
    pub dispatch_id: Option<String>,
    pub message: String,
}

impl SubagentMailbox {
    pub fn new() -> Self {
        Self {
            messages: Mutex::new(HashMap::new()),
        }
    }

    /// Push a steer message for the given run. Returns `false` if run_id is not registered.
    pub fn push(&self, run_id: &str, msg: String) -> bool {
        self.push_message(
            run_id,
            SubagentMailboxMessage {
                dispatch_id: None,
                message: msg,
            },
        )
    }

    /// Queue a durable steer dispatch. The dispatch remains `accepted` in
    /// SQLite until the child loop checkpoints this envelope into its session.
    pub fn push_dispatch(&self, run_id: &str, dispatch_id: String, msg: String) -> bool {
        self.push_message(
            run_id,
            SubagentMailboxMessage {
                dispatch_id: Some(dispatch_id),
                message: msg,
            },
        )
    }

    fn push_message(&self, run_id: &str, msg: SubagentMailboxMessage) -> bool {
        if let Ok(mut map) = self.messages.lock() {
            if let Some(queue) = map.get_mut(run_id) {
                queue.push(msg);
                return true;
            }
        }
        false
    }

    /// Drain all pending steer messages for a run (called by the child agent's tool loop).
    pub fn drain(&self, run_id: &str) -> Vec<SubagentMailboxMessage> {
        if let Ok(mut map) = self.messages.lock() {
            if let Some(queue) = map.get_mut(run_id) {
                return std::mem::take(queue);
            }
        }
        Vec::new()
    }

    /// Register a run_id slot (called at spawn time).
    pub fn register(&self, run_id: &str) {
        if let Ok(mut map) = self.messages.lock() {
            map.insert(run_id.to_string(), Vec::new());
        }
    }

    /// Remove a run_id slot (called when run terminates).
    pub fn remove(&self, run_id: &str) {
        if let Ok(mut map) = self.messages.lock() {
            map.remove(run_id);
        }
    }
}

/// Global steer mailbox — accessible from tools and agent providers.
pub static SUBAGENT_MAILBOX: std::sync::LazyLock<SubagentMailbox> =
    std::sync::LazyLock::new(SubagentMailbox::new);

#[cfg(test)]
mod mailbox_tests {
    use super::*;

    #[test]
    fn durable_dispatch_keeps_identity_until_child_drain() {
        let mailbox = SubagentMailbox::new();
        mailbox.register("run-1");
        assert!(mailbox.push_dispatch("run-1", "dispatch-1".into(), "continue".into()));

        assert_eq!(
            mailbox.drain("run-1"),
            vec![SubagentMailboxMessage {
                dispatch_id: Some("dispatch-1".into()),
                message: "continue".into(),
            }]
        );
        assert!(mailbox.drain("run-1").is_empty());
    }
}

// ── Chat Session Guard ──────────────────────────────────────────

/// RAII guard: marks a session as active in user chat, cancels any running injection.
/// Drop releases only this guard's reference so a stale stopped turn cannot
/// clear a newer turn that started for the same session.
pub struct ChatSessionGuard {
    session_id: String,
}

impl ChatSessionGuard {
    pub fn new(session_id: &str) -> Self {
        if let Ok(mut counts) = ACTIVE_CHAT_SESSIONS.lock() {
            *counts.entry(session_id.to_string()).or_insert(0) += 1;
        }
        // Cancel any running injection for this session
        if let Ok(map) = INJECTION_CANCELS.lock() {
            if let Some(injection) = map.get(session_id) {
                injection.cancel.store(true, Ordering::SeqCst);
            }
        }
        Self {
            session_id: session_id.to_string(),
        }
    }
}

impl Drop for ChatSessionGuard {
    fn drop(&mut self) {
        let mut became_idle = false;
        if let Ok(mut counts) = ACTIVE_CHAT_SESSIONS.lock() {
            if let Some(count) = counts.get_mut(&self.session_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&self.session_id);
                    became_idle = true;
                }
            }
        }
        if became_idle {
            // Wake up any injection waiters (replaces 2s polling)
            SESSION_IDLE_NOTIFY.notify_waiters();
            // Re-trigger any pending injections that were cancelled during this chat
            flush_pending_injections(&self.session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_chat_session_guards_release_by_reference() {
        let sid = "test-subagent-chat-session-guard-refcount";
        ACTIVE_CHAT_SESSIONS.lock().unwrap().remove(sid);

        let first = ChatSessionGuard::new(sid);
        let second = ChatSessionGuard::new(sid);
        assert_eq!(
            ACTIVE_CHAT_SESSIONS.lock().unwrap().get(sid).copied(),
            Some(2)
        );

        drop(first);
        assert_eq!(
            ACTIVE_CHAT_SESSIONS.lock().unwrap().get(sid).copied(),
            Some(1)
        );

        drop(second);
        assert!(!ACTIVE_CHAT_SESSIONS.lock().unwrap().contains_key(sid));
    }
}

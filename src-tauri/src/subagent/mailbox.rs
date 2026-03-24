use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use super::injection::flush_pending_injections;
use super::ACTIVE_CHAT_SESSIONS;
use super::INJECTION_CANCELS;
use super::SESSION_IDLE_NOTIFY;

// ── Steer Mailbox ───────────────────────────────────────────────

/// Per-run message queue for steering running sub-agents.
/// Parent agents push steer messages; the child agent's tool loop drains them each round.
pub struct SubagentMailbox {
    messages: Mutex<HashMap<String, Vec<String>>>,
}

impl SubagentMailbox {
    pub fn new() -> Self {
        Self { messages: Mutex::new(HashMap::new()) }
    }

    /// Push a steer message for the given run. Returns Err if run_id not registered.
    pub fn push(&self, run_id: &str, msg: String) -> bool {
        if let Ok(mut map) = self.messages.lock() {
            if let Some(queue) = map.get_mut(run_id) {
                queue.push(msg);
                return true;
            }
        }
        false
    }

    /// Drain all pending steer messages for a run (called by the child agent's tool loop).
    pub fn drain(&self, run_id: &str) -> Vec<String> {
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

// ── Chat Session Guard ──────────────────────────────────────────

/// RAII guard: marks a session as active in user chat, cancels any running injection.
/// Drop removes the session from the active set.
pub struct ChatSessionGuard {
    session_id: String,
}

impl ChatSessionGuard {
    pub fn new(session_id: &str) -> Self {
        if let Ok(mut set) = ACTIVE_CHAT_SESSIONS.lock() {
            set.insert(session_id.to_string());
        }
        // Cancel any running injection for this session
        if let Ok(map) = INJECTION_CANCELS.lock() {
            if let Some(cancel) = map.get(session_id) {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        Self { session_id: session_id.to_string() }
    }
}

impl Drop for ChatSessionGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = ACTIVE_CHAT_SESSIONS.lock() {
            set.remove(&self.session_id);
        }
        // Wake up any injection waiters (replaces 2s polling)
        SESSION_IDLE_NOTIFY.notify_waiters();
        // Re-trigger any pending injections that were cancelled during this chat
        flush_pending_injections(&self.session_id);
    }
}

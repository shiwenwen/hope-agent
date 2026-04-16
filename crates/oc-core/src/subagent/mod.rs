mod cancel;
mod helpers;
pub(crate) mod injection;
mod mailbox;
mod spawn;
mod types;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

// ── Constants ────────────────────────────────────────────────────

/// Default maximum nesting depth for sub-agents
const DEFAULT_MAX_DEPTH: u32 = 3;

/// Get the effective max depth, checking global config.
pub fn max_depth() -> u32 {
    // In the future, this could read from a global config.
    // For now, individual agent configs can override via max_spawn_depth.
    DEFAULT_MAX_DEPTH
}

/// Get the effective max depth for a specific agent.
pub fn max_depth_for_agent(agent_id: &str) -> u32 {
    crate::agent_loader::load_agent(agent_id)
        .ok()
        .and_then(|def| def.config.subagents.max_spawn_depth)
        .map(|d| d.clamp(1, 5))
        .unwrap_or(DEFAULT_MAX_DEPTH)
}

/// Default max tasks per batch_spawn call
const DEFAULT_MAX_BATCH_SIZE: usize = 10;

/// Get the effective max batch size for a specific agent.
pub fn max_batch_size_for_agent(agent_id: &str) -> usize {
    crate::agent_loader::load_agent(agent_id)
        .ok()
        .and_then(|def| def.config.subagents.max_batch_size)
        .map(|s| (s as usize).clamp(1, 50))
        .unwrap_or(DEFAULT_MAX_BATCH_SIZE)
}

/// Default timeout for sub-agent execution (seconds)
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Max result characters stored in DB
const MAX_RESULT_CHARS: usize = 10_000;

/// Max concurrent sub-agents per parent session
pub const MAX_CONCURRENT_PER_SESSION: usize = 5;

// ── Global statics (used by injection, mailbox, helpers) ────────

/// Global set tracking which parent sessions currently have an active backend injection.
/// Prevents concurrent double-injection for the same session.
static INJECTING_SESSIONS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Sessions currently in a user-initiated chat() call.
/// Injection must wait until the session is idle.
pub static ACTIVE_CHAT_SESSIONS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Per-session cancel flags for active injections.
/// When the user starts a new chat() on a session, the injection cancel flag is set.
pub static INJECTION_CANCELS: std::sync::LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Run IDs whose results have been read by the parent agent via check/result tool actions.
/// If a run_id is here, auto-injection is skipped.
static FETCHED_RUN_IDS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Queue of injection tasks that were cancelled (user sent new message) and need retry.
static PENDING_INJECTIONS: std::sync::LazyLock<Mutex<Vec<injection::PendingInjection>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// Notify signal: fired when a session becomes idle (ChatSessionGuard dropped).
/// Injection waiters use this instead of polling.
static SESSION_IDLE_NOTIFY: std::sync::LazyLock<tokio::sync::Notify> =
    std::sync::LazyLock::new(|| tokio::sync::Notify::new());

// ── Re-exports ──────────────────────────────────────────────────

pub use cancel::SubagentCancelRegistry;
pub use helpers::{cleanup_orphan_runs, mark_run_fetched};
pub use mailbox::{ChatSessionGuard, SUBAGENT_MAILBOX};
pub use spawn::spawn_subagent;
pub use types::{SpawnParams, SubagentRun, SubagentStatus};

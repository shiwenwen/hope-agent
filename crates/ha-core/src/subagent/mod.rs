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

/// Default max concurrent sub-agents per parent session — fallback used when
/// the parent agent's config can't be loaded. The real limit is per-agent and
/// configurable via `subagents.maxConcurrent` (see [`max_concurrent_for_agent`]);
/// this constant is only the floor when that lookup fails.
pub const DEFAULT_MAX_CONCURRENT_PER_SESSION: usize = 8;

/// Effective max concurrent sub-agents for a specific parent agent. Reads the
/// per-agent `subagents.max_concurrent` config (clamped 1..=50 so a typo can't
/// uncap it), falling back to [`DEFAULT_MAX_CONCURRENT_PER_SESSION`] when the
/// agent definition can't be loaded.
///
/// NOTE: the `subagents.max_concurrent` field used to be dead config — the spawn
/// gate hardcoded a constant and never consulted it. This is the single wiring
/// point that makes the GUI/`agent.json` value actually take effect.
pub fn max_concurrent_for_agent(agent_id: &str) -> usize {
    crate::agent_loader::load_agent(agent_id)
        .ok()
        .map(|def| clamp_max_concurrent(def.config.subagents.max_concurrent))
        .unwrap_or(DEFAULT_MAX_CONCURRENT_PER_SESSION)
}

/// Clamp a configured `max_concurrent` into a sane band (1..=50): a `0`/typo
/// must not silently block every spawn, and an absurd value must not let one
/// session fan out enough sub-agents to instantly exhaust the API rate limit.
fn clamp_max_concurrent(raw: u32) -> usize {
    (raw as usize).clamp(1, 50)
}

// ── Global statics (used by injection, mailbox, helpers) ────────

/// Global set tracking which parent sessions currently have an active backend injection.
/// Prevents concurrent double-injection for the same session.
static INJECTING_SESSIONS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Sessions currently in one or more user-initiated chat() calls.
/// Injection must wait until the session is idle.
pub static ACTIVE_CHAT_SESSIONS: std::sync::LazyLock<Mutex<HashMap<String, usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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

/// Request cancellation of a sub-agent run by `run_id` — the shared entry used
/// by the `kill` tool action, runtime-task cancel, and the R6 background-job
/// cancel routing (`async_jobs::cancel_job` for a `kind=subagent` projection).
///
/// Signals the in-process cancel flag (the running task aborts via its
/// `abort_on_cancel` chat engine and settles `Killed`, which syncs the
/// projection through `update_subagent_status`). If no flag is registered (the
/// run already settled and was removed from the registry) and the run is still
/// active, stamps it `Killed` directly so a caller is never left with an
/// un-cancellable row. Returns true if a cancel was signalled or stamped.
pub fn request_cancel_run(run_id: &str) -> bool {
    if let Some(registry) = crate::get_subagent_cancels() {
        if registry.cancel(run_id) {
            return true;
        }
    }
    // No in-process flag — fall back to stamping terminal if still active.
    if let Some(db) = crate::get_session_db() {
        if let Ok(Some(run)) = db.get_subagent_run(run_id) {
            if !run.status.is_terminal() {
                let _ = db.update_subagent_status(
                    run_id,
                    SubagentStatus::Killed,
                    None,
                    Some("Killed via background-job cancel"),
                    None,
                    None,
                );
                return true;
            }
        }
    }
    false
}

// ── Re-exports ──────────────────────────────────────────────────

pub use cancel::SubagentCancelRegistry;
pub use helpers::{cleanup_orphan_runs, mark_run_fetched, take_runs_fetched};
pub use mailbox::{ChatSessionGuard, SUBAGENT_MAILBOX};
pub use spawn::{spawn_subagent, HOOK_SPAWN_LABEL};
pub use types::{SpawnParams, SubagentRun, SubagentStatus};

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    #[test]
    fn clamp_max_concurrent_bounds_to_1_50() {
        assert_eq!(clamp_max_concurrent(0), 1, "0 must not block every spawn");
        assert_eq!(clamp_max_concurrent(1), 1);
        assert_eq!(clamp_max_concurrent(5), 5);
        assert_eq!(clamp_max_concurrent(8), 8, "new default passes through");
        assert_eq!(clamp_max_concurrent(50), 50);
        assert_eq!(
            clamp_max_concurrent(10_000),
            50,
            "absurd value clamped down"
        );
    }

    #[test]
    fn default_max_concurrent_per_session_is_eight() {
        // Contract: default bumped 5 -> 8 (PRD R7.2). The agent_config serde
        // default must stay in lockstep with this fallback.
        assert_eq!(DEFAULT_MAX_CONCURRENT_PER_SESSION, 8);
        assert_eq!(
            crate::agent_config::SubagentConfig::default().max_concurrent,
            8
        );
    }

    #[test]
    fn max_concurrent_for_agent_reads_configured_field() {
        // The whole point of wiring this fn: prove the (formerly dead)
        // subagents.maxConcurrent field on disk is actually consulted.
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let agent_id = "test-maxconc-agent";
            let dir = crate::paths::agent_dir(agent_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            // Serialize a full default AgentConfig with only max_concurrent
            // overridden, so the on-disk agent.json is guaranteed to deserialize.
            let mut cfg = crate::agent_config::AgentConfig::default();
            cfg.subagents.max_concurrent = 3;
            std::fs::write(dir.join("agent.json"), serde_json::to_string(&cfg).unwrap()).unwrap();
            assert_eq!(max_concurrent_for_agent(agent_id), 3);
        });
    }

    #[test]
    fn max_concurrent_for_agent_falls_back_when_agent_missing() {
        // load_agent() fails (no agents/<id>/ dir) -> .ok()=None -> fallback to
        // the default, NOT 0 (which would block every spawn).
        let root = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            assert_eq!(
                max_concurrent_for_agent("__nonexistent_agent__"),
                DEFAULT_MAX_CONCURRENT_PER_SESSION
            );
        });
    }
}

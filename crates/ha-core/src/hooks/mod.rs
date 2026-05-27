//! Hooks system — event → pluggable handler dispatch.
//!
//! Field-level aligned with the Claude Code hooks protocol. See
//! `docs/plans/hooks-system-design.md` for the full design; this module is the
//! `ha-core` implementation (zero Tauri deps — runs in desktop, `server`, and
//! ACP modes alike, per design §3.6).
//!
//! Business call sites only ever touch [`HookDispatcher::dispatch`] and read a
//! [`HookOutcome`]; matcher filtering, concurrent execution, timeouts,
//! dedup, and aggregation are all internal.

pub mod audit;
pub mod condition;
pub mod config;
pub mod decision;
pub mod env;
pub mod matcher;
pub mod parse;
pub mod registry;
pub mod runner;
pub mod scopes;
pub mod transcript;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use runner::HookHandler;

/// Sessions that have already fired `SessionStart` (startup/resume) in this
/// process. `SessionStart` is a once-per-session event, but the engine runs
/// per user turn — this gate prevents re-firing on turn 2+. (The `compact`
/// source is fired separately from the compaction path and is not gated here.)
static SESSION_START_SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Returns `true` the first time `session_id` becomes active in this process
/// (and records it), `false` afterwards. Used to fire `SessionStart`
/// (startup/resume) once per session rather than once per turn.
pub fn claim_session_start(session_id: &str) -> bool {
    // Hard cap so a long-running server/desktop process that touches many
    // sessions doesn't grow this set unboundedly. On overflow we clear it: the
    // worst case is an old, still-active session re-firing SessionStart once —
    // harmless and vanishingly rare versus the leak.
    const CAP: usize = 8192;
    let seen = SESSION_START_SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = seen.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= CAP {
        guard.clear();
    }
    guard.insert(session_id.to_string())
}

/// `(session_id, handler_key)` pairs that have already run a `once: true`
/// handler in this process. Same shape + CAP rationale as [`claim_session_start`].
static ONCE_PER_SESSION_SEEN: OnceLock<Mutex<HashSet<(String, String)>>> = OnceLock::new();

/// Returns `true` the first time `(session_id, handler_key)` runs a `once`
/// handler (and records it), `false` afterwards. `handler_key` is the handler's
/// `type|identity` so distinct handlers in the same session don't collide.
pub fn claim_once_per_session(session_id: &str, handler_key: &str) -> bool {
    const CAP: usize = 65536;
    let seen = ONCE_PER_SESSION_SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = seen.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= CAP {
        guard.clear();
    }
    guard.insert((session_id.to_string(), handler_key.to_string()))
}

/// Last time compaction hooks fired per session, for cross-retry de-dup.
static COMPACTION_HOOKS_FIRED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// De-dupe compaction hooks (`PostCompact` / `SessionStart(compact)`) across
/// failover retries: each retry rebuilds the agent and re-runs compaction from
/// the same un-compacted DB history, producing an identical `key` (the caller
/// passes `session:tier:tokens_after`). A genuinely distinct compaction —
/// different tier or post-compaction token count — has a different `key` and
/// fires even within the window, so an emergency second compaction (which can
/// bypass the `compact.cacheTtlSecs` throttle) is no longer suppressed. The
/// time window only bounds the map and lets the same key recur much later.
/// Returns `true` the first time `key` is seen within the window.
pub fn claim_compaction_hooks(key: &str) -> bool {
    const WINDOW: Duration = Duration::from_secs(60);
    let map = COMPACTION_HOOKS_FIRED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    guard.retain(|_, t| now.duration_since(*t) < WINDOW);
    if guard.contains_key(key) {
        return false;
    }
    guard.insert(key.to_string(), now);
    true
}

/// Consecutive `PreCompact` blocks per session. Bounds a hook that keeps
/// blocking compaction while context usage sits in the band below the
/// emergency-override ratio (where it would otherwise never reach the forced
/// threshold, so compaction is deferred forever and the hook re-fires every
/// turn). After the cap the block is overridden once and the count resets.
static PRECOMPACT_BLOCK_COUNTS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

/// Max consecutive `PreCompact` blocks honored before forcing compaction.
const MAX_PRECOMPACT_BLOCKS: u32 = 5;

/// Record a `PreCompact` block for `session_id` and return whether it should be
/// HONORED. Returns `false` (override → compact anyway, and resets the count)
/// once the consecutive-block cap is exceeded.
pub fn honor_precompact_block(session_id: &str) -> bool {
    let map = PRECOMPACT_BLOCK_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    let count = guard.entry(session_id.to_string()).or_insert(0);
    *count += 1;
    if *count > MAX_PRECOMPACT_BLOCKS {
        guard.remove(session_id);
        false
    } else {
        true
    }
}

/// Reset the consecutive `PreCompact` block counter — called whenever
/// compaction actually proceeds (block not honored, or no block at all).
pub fn reset_precompact_blocks(session_id: &str) {
    if let Some(map) = PRECOMPACT_BLOCK_COUNTS.get() {
        map.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }
}

/// Pending `UserPromptSubmit` `additionalContext` per session. The preflight
/// chokepoint sets this after the hook runs; the turn drains it once at start
/// and folds it into `extra_system_context` next to `SessionStart`. Keyed by
/// session so concurrent sessions never cross-contaminate, and preflight always
/// overwrites/clears its session's slot before the turn runs — so a turn that
/// never reaches the engine (rare persist failure between preflight and the
/// engine) cannot leak stale context into the next turn.
static PENDING_PROMPT_CONTEXT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Set (non-empty `Some`) or clear (`None` / empty) the pending
/// `UserPromptSubmit` context for `session_id`. Called by the preflight
/// chokepoint after the hook runs, exactly once per turn.
pub fn set_user_prompt_context(session_id: &str, ctx: Option<String>) {
    let map = PENDING_PROMPT_CONTEXT.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    match ctx {
        Some(c) if !c.is_empty() => {
            // Bound the map so a pathological never-drained case can't grow it
            // unboundedly (the normal path clears each entry as its turn drains
            // it). On overflow, clearing only loses pending context for a few
            // in-flight turns — harmless versus the leak.
            const CAP: usize = 4096;
            if guard.len() >= CAP {
                guard.clear();
            }
            guard.insert(session_id.to_string(), c);
        }
        _ => {
            guard.remove(session_id);
        }
    }
}

/// Take (and clear) the pending `UserPromptSubmit` context for `session_id`.
/// Called once at the start of the turn the prompt belongs to.
pub fn take_user_prompt_context(session_id: &str) -> Option<String> {
    let map = PENDING_PROMPT_CONTEXT.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard.remove(session_id)
}

pub use config::{
    AgentHookConfig, CommandHookConfig, HookHandlerConfig, HookMatcherGroup, HookShell,
    HooksConfig, HttpHookConfig, McpToolHookConfig, PromptHookConfig,
};
pub use transcript::{TranscriptLine, TranscriptMessage, TranscriptMirror};
pub use types::{
    CommonHookInput, CompactTrigger, HookDecision, HookEvent, HookInput, HookOutcome, HookOutput,
    PermissionMode, SessionStartSource,
};

/// Entry point for business code: run all hooks matching `event`/`input` and
/// return their aggregated outcome. Returns `HookOutcome::noop()` when nothing
/// matches — callers treat that as "nothing happened".
pub struct HookDispatcher;

impl HookDispatcher {
    pub async fn dispatch(event: HookEvent, input: HookInput) -> HookOutcome {
        // Resolve the effective registry for this session's working dir so
        // project/local-scope hooks merge on top of the global (user+managed)
        // scope (design §4). Falls back to the global registry for app-global
        // events / sessions without a working dir.
        let wd = session_working_dir(&input);
        let registry = scopes::resolve_for_cwd(wd.as_deref().map(std::path::Path::new));
        Self::dispatch_with(&registry, event, input).await
    }

    /// Testable core: dispatch against an explicit registry.
    async fn dispatch_with(
        registry: &registry::HookRegistry,
        event: HookEvent,
        input: HookInput,
    ) -> HookOutcome {
        // Hot-path short-circuit: skip building anything when no hook listens
        // for this event.
        if !registry.has_handlers_for(event) {
            return HookOutcome::noop();
        }
        let configs = registry.matching_handlers(event, input.matcher_target());
        if configs.is_empty() {
            return HookOutcome::noop();
        }

        // Build runnable handlers (this phase: `command` only) and dedupe by
        // (type, identity) across the merged groups (design §7.7).
        let mut seen: HashSet<(&'static str, String)> = HashSet::new();
        let mut handlers: Vec<Box<dyn HookHandler>> = Vec::new();
        for cfg in configs {
            if let Some(h) = build_handler(cfg) {
                // `if` condition: run only when the tool call matches the rule
                // (non-tool events / mismatches skip — see `condition`).
                if let Some(rule) = cfg.if_rule() {
                    if !condition::if_matches(rule, &input) {
                        continue;
                    }
                }
                // `once`: skip if this handler already ran in this session.
                if cfg.once() {
                    let key = format!("{}|{}", h.handler_type(), h.identity());
                    if !claim_once_per_session(&input.common().session_id, &key) {
                        continue;
                    }
                }
                if seen.insert((h.handler_type(), h.identity())) {
                    // `statusMessage`: this handler will run — surface its note.
                    if let Some(msg) = cfg.status_message() {
                        emit_hook_status(&input, msg, h.handler_type());
                    }
                    handlers.push(h);
                }
            }
        }
        if handlers.is_empty() {
            return HookOutcome::noop();
        }

        let env = env::HookEnv::build_for_command(input.common());
        let start = Instant::now();

        // Concurrent execution, each handler INDIVIDUALLY bounded (its own
        // timeout + 5s backstop, design §7.8). A laggard yields its own
        // non-blocking timed-out result rather than discarding every sibling's
        // already-completed output — so a fast hook's context / decision is
        // never lost to a slow neighbor.
        let runs = handlers.iter().map(|h| {
            let timeout = h.default_timeout();
            let deadline = Instant::now() + timeout;
            let backstop = timeout + Duration::from_secs(5);
            // Borrow (not move) the shared input/env so each future only holds a
            // reference and `input` stays usable after `join_all`.
            let input = &input;
            let env = &env;
            async move {
                // Isolate a panicking handler: `join_all` polls these inline, so
                // an unwrap/panic inside `run` would otherwise propagate and take
                // down the whole dispatch (and its host call site). catch_unwind
                // turns it into a non-blocking error like any other handler fault.
                use futures_util::FutureExt;
                let guarded = std::panic::AssertUnwindSafe(h.run(input, env, deadline)).catch_unwind();
                match tokio::time::timeout(backstop, guarded).await {
                    Ok(Ok(r)) => r,
                    Ok(Err(_panic)) => {
                        app_warn!(
                            "hooks",
                            "dispatch",
                            "event={} a handler panicked — treating it as non-blocking",
                            event.as_str()
                        );
                        runner::RawHookResult::non_blocking_error("hook handler panicked")
                    }
                    Err(_) => {
                        app_warn!(
                            "hooks",
                            "dispatch",
                            "event={} a handler exceeded its timeout backstop — treating it as non-blocking",
                            event.as_str()
                        );
                        runner::RawHookResult::non_blocking_error("hook exceeded dispatch backstop")
                    }
                }
            }
        });
        let raws = futures_util::future::join_all(runs).await;

        let contributions = raws.iter().map(|r| parse::parse(r, event)).collect();
        let mut outcome = decision::aggregate(contributions);
        downgrade_block_on_observation(&mut outcome, event);
        apply_overflow(&mut outcome, event, input.common());
        audit::log_dispatch(event, handlers.len(), &outcome, start.elapsed());
        outcome
    }
}

/// Build a runnable handler from config — all five handler types are wired.
/// Emit a `hook:status` event so the GUI can surface a handler's
/// `statusMessage` while it runs (design §20.7 → desktop toast). No-op when no
/// event bus is wired (e.g. unit tests).
fn emit_hook_status(input: &HookInput, message: &str, handler_type: &str) {
    if let Some(bus) = crate::globals::get_event_bus() {
        bus.emit(
            "hook:status",
            serde_json::json!({
                "sessionId": input.common().session_id,
                "hookEvent": input.common().hook_event_name,
                "handlerType": handler_type,
                "message": message,
            }),
        );
    }
}

fn build_handler(cfg: &HookHandlerConfig) -> Option<Box<dyn HookHandler>> {
    match cfg {
        HookHandlerConfig::Command(c) => {
            Some(Box::new(runner::command::CommandHandler::new(c.clone())))
        }
        HookHandlerConfig::Http(c) => Some(Box::new(runner::http::HttpHandler::new(c.clone()))),
        HookHandlerConfig::McpTool(c) => {
            Some(Box::new(runner::mcp_tool::McpToolHandler::new(c.clone())))
        }
        HookHandlerConfig::Prompt(c) => {
            Some(Box::new(runner::prompt::PromptHandler::new(c.clone())))
        }
        HookHandlerConfig::Agent(c) => Some(Box::new(runner::agent::AgentHandler::new(c.clone()))),
    }
}

/// Observation events can't gate execution (design §5.1.1): if a hook returns
/// `block`/`deny` for one, keep its `additionalContext` but neutralize the
/// decision and log — so a future caller that honors `outcome.decision` can't
/// be made to block by an observation hook.
fn downgrade_block_on_observation(outcome: &mut HookOutcome, event: HookEvent) {
    if !event.is_observation_only() {
        return;
    }
    if matches!(
        outcome.decision,
        HookDecision::Block { .. } | HookDecision::Deny { .. }
    ) {
        app_warn!(
            "hooks",
            "dispatch",
            "event={} hook returned a blocking decision on an observation event — downgraded to non-blocking",
            event.as_str()
        );
        outcome.decision = HookDecision::Allow;
        outcome.continue_execution = true;
        outcome.stop_reason = None;
    }
}

/// Enforce the 10 000-char injection cap: spill the full merged context to an
/// overflow file, but keep an inline head slice (+ a pointer to the rest)
/// rather than discarding everything — so the model still sees the beginning of
/// the smaller contributions instead of only a pointer (design §8.6).
fn apply_overflow(outcome: &mut HookOutcome, event: HookEvent, common: &CommonHookInput) {
    let Some(merged) = outcome.merged_additional_context() else {
        return;
    };
    if merged.chars().count() <= audit::MAX_INJECT_CHARS {
        return;
    }
    let pointer = match audit::write_overflow(event, &common.session_id, &merged) {
        Some(p) => format!("\n\n[…truncated; full hook output at {}]", p.display()),
        None => "\n\n[…truncated]".to_string(),
    };
    // Keep as much of the head as fits under the cap once the pointer is added.
    let budget = audit::MAX_INJECT_CHARS.saturating_sub(pointer.chars().count());
    let head: String = merged.chars().take(budget).collect();
    outcome.additional_context = vec![format!("{head}{pointer}")];
    app_warn!(
        "hooks",
        "dispatch",
        "event={} injected context exceeded {} chars; kept head + spilled rest to overflow file",
        event.as_str(),
        audit::MAX_INJECT_CHARS
    );
}

/// Process-lived runtime for fire-and-forget dispatch from synchronous call
/// sites that have no ambient tokio runtime. Built once, never dropped, so a
/// spawned dispatch — including an `async` command hook that detaches a child
/// task — runs to completion instead of being killed when a throwaway runtime
/// drops. `None` only if runtime construction fails (then fire-and-forget is a
/// no-op rather than a panic).
pub(crate) fn fire_and_forget_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static FIRE_RT: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    FIRE_RT
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("hooks-fire")
                .build()
                .ok()
        })
        .as_ref()
}

/// Fire an observation hook without waiting for or consuming its output —
/// for events that don't feed back into the current flow (`SessionEnd`,
/// `Notification`). No-op when no hook is configured for the event, so call
/// sites stay cheap. Bridges synchronous call sites onto a runtime.
pub fn fire_and_forget(event: HookEvent, input: HookInput) {
    let wd = session_working_dir(&input);
    if !scopes::any_handlers_for(event, wd.as_deref().map(std::path::Path::new)) {
        return;
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            HookDispatcher::dispatch(event, input).await;
        });
    } else if let Some(rt) = fire_and_forget_runtime() {
        // Synchronous call site with no ambient runtime: spawn onto the
        // process-lived runtime (do NOT block_on a throwaway one — that would
        // kill detached `async` hook child tasks the moment it drops).
        rt.spawn(async move {
            HookDispatcher::dispatch(event, input).await;
        });
    }
}

/// The session working dir used for project/local scope resolution — the real
/// `sessions.working_dir` (no home fallback), so a session without one (or an
/// app-global event with an empty session id) sees only the global scope and
/// never picks up a stray `~/.hope-agent/hooks.json` as if it were a project.
fn session_working_dir(input: &HookInput) -> Option<String> {
    let sid = &input.common().session_id;
    if sid.is_empty() {
        return None;
    }
    crate::session::effective_session_working_dir(Some(sid))
}

/// Common hook-input fields for app-/session-level (non-tool) observation
/// hooks. `cwd` is the session working dir (falling back to home);
/// `agent_id`/`agent_type` unknown at these sites.
fn observation_common(event: &str, session_id: &str) -> CommonHookInput {
    // Empty session_id (app-global events like logout/shutdown) → no transcript
    // path and no working dir, rather than a bogus `sessions/transcript.jsonl`.
    let sid_opt = (!session_id.is_empty()).then_some(session_id);
    let transcript_path = sid_opt
        .and_then(|sid| crate::paths::session_dir(sid).ok())
        .map(|d| d.join("transcript.jsonl"))
        .unwrap_or_default();
    let cwd = crate::session::effective_session_working_dir(sid_opt)
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    CommonHookInput {
        session_id: session_id.to_string(),
        transcript_path,
        cwd,
        permission_mode: PermissionMode::Default,
        hook_event_name: event.to_string(),
        agent_id: None,
        agent_type: None,
    }
}

/// Fire a `Notification` hook (e.g. permission prompt, auth success). Used for
/// desktop-notification bridging. `additionalContext` is not injected this
/// phase (these sites are outside a turn).
pub fn fire_notification(session_id: &str, notification_type: &str, message: &str) {
    let input = HookInput::Notification {
        common: observation_common("Notification", session_id),
        notification_type: notification_type.to_string(),
        message: message.to_string(),
        title: None,
    };
    fire_and_forget(HookEvent::Notification, input);
}

/// Fire the `UserPromptSubmit` hook (blocking) and hand back the full outcome
/// so the preflight chokepoint can map it to block / proceed and stash any
/// injected context for the turn. Returns `noop()` early when no hook is
/// configured. `prompt` is the raw user text about to be persisted; `agent_id`
/// is the agent that will run the turn (lets scripts gate per agent).
pub async fn fire_user_prompt_submit(
    session_id: &str,
    agent_id: Option<&str>,
    prompt: &str,
) -> HookOutcome {
    let wd = crate::session::effective_session_working_dir(Some(session_id));
    if !scopes::any_handlers_for(
        HookEvent::UserPromptSubmit,
        wd.as_deref().map(std::path::Path::new),
    ) {
        return HookOutcome::noop();
    }
    let mut common = observation_common("UserPromptSubmit", session_id);
    common.agent_id = agent_id.map(|s| s.to_string());
    let input = HookInput::UserPromptSubmit {
        common,
        prompt: prompt.to_string(),
    };
    HookDispatcher::dispatch(HookEvent::UserPromptSubmit, input).await
}

/// Fire the `SessionStart` observation hook (startup/resume) and return any
/// merged `additionalContext` to fold into this turn's system prompt. Fires
/// once per session per process (`claim_session_start`); later turns return
/// `None`. Shared by the chat engine and the ACP turn loop so both entry points
/// inject identical context — ACP runs `AssistantAgent::chat` directly rather
/// than `run_chat_engine`, so without this it would never see `SessionStart`.
///
/// `startup` vs `resume` is decided by the persisted message count (the user
/// message for this turn is already saved at every call site): `≤1` → first
/// turn → startup. Returns `None` early when no `SessionStart` hook is
/// configured, keeping the no-hook path free of the session lookup.
pub async fn fire_session_start_observation(
    session_id: &str,
    agent_id: &str,
    model: &str,
) -> Option<String> {
    let wd = crate::session::effective_session_working_dir(Some(session_id));
    if !scopes::any_handlers_for(
        HookEvent::SessionStart,
        wd.as_deref().map(std::path::Path::new),
    ) || !claim_session_start(session_id)
    {
        return None;
    }
    let source = if crate::session::lookup_session_meta(Some(session_id))
        .map(|m| m.message_count <= 1)
        .unwrap_or(true)
    {
        SessionStartSource::Startup
    } else {
        SessionStartSource::Resume
    };
    let mut common = observation_common("SessionStart", session_id);
    common.agent_id = Some(agent_id.to_string());
    let input = HookInput::SessionStart {
        common,
        source,
        model: model.to_string(),
        agent_type: None,
    };
    HookDispatcher::dispatch(HookEvent::SessionStart, input)
        .await
        .merged_additional_context()
}

/// Fire a `SessionEnd` hook. Pure observation (no `hookSpecificOutput`).
pub fn fire_session_end(session_id: &str, source: &str) {
    let input = HookInput::SessionEnd {
        common: observation_common("SessionEnd", session_id),
        source: source.to_string(),
    };
    fire_and_forget(HookEvent::SessionEnd, input);
}

/// Await-able `SessionEnd` dispatch — for shutdown paths that want the hook to
/// actually finish before the process exits (e.g. the server's graceful
/// shutdown). Synchronous, fire-and-forget call sites use [`fire_session_end`].
pub async fn dispatch_session_end(session_id: &str, source: &str) {
    let wd = crate::session::effective_session_working_dir(Some(session_id));
    if !scopes::any_handlers_for(
        HookEvent::SessionEnd,
        wd.as_deref().map(std::path::Path::new),
    ) {
        return;
    }
    let input = HookInput::SessionEnd {
        common: observation_common("SessionEnd", session_id),
        source: source.to_string(),
    };
    HookDispatcher::dispatch(HookEvent::SessionEnd, input).await;
}

/// Fire a `SubagentStart` observation hook (sub-agent spawned). `session_id` is
/// the parent session; `subagent_id` is the spawned agent's id (matcher target).
pub fn fire_subagent_start(session_id: &str, subagent_id: &str, run_id: &str) {
    let input = HookInput::SubagentStart {
        common: observation_common("SubagentStart", session_id),
        subagent_id: subagent_id.to_string(),
        run_id: run_id.to_string(),
    };
    fire_and_forget(HookEvent::SubagentStart, input);
}

/// Fire a `SubagentStop` observation hook (sub-agent reached a terminal state).
pub fn fire_subagent_stop(session_id: &str, subagent_id: &str, run_id: &str, status: &str) {
    let input = HookInput::SubagentStop {
        common: observation_common("SubagentStop", session_id),
        subagent_id: subagent_id.to_string(),
        run_id: run_id.to_string(),
        status: status.to_string(),
    };
    fire_and_forget(HookEvent::SubagentStop, input);
}

/// Fire the `Stop` observation hook — a turn finished responding without an
/// error (normal completion or a user-initiated stop). `status` is the terminal
/// turn status (`completed` / `interrupted`). Fire-and-forget; block-to-continue
/// is not implemented this phase.
pub fn fire_stop(session_id: &str, agent_id: Option<&str>, status: &str) {
    let mut common = observation_common("Stop", session_id);
    common.agent_id = agent_id.map(|s| s.to_string());
    let input = HookInput::Stop {
        common,
        status: status.to_string(),
        stop_hook_active: false,
    };
    fire_and_forget(HookEvent::Stop, input);
}

/// Fire the `StopFailure` observation hook — a turn ended because of an error.
/// `reason` is the failure category (matcher target, e.g. `provider_failed`);
/// `error` is the message, when there is one.
pub fn fire_stop_failure(session_id: &str, reason: &str, error: Option<&str>) {
    let input = HookInput::StopFailure {
        common: observation_common("StopFailure", session_id),
        reason: reason.to_string(),
        error: error.map(|s| s.to_string()),
    };
    fire_and_forget(HookEvent::StopFailure, input);
}

/// Fire a `TaskCreated` observation hook (one per task created in a
/// `task_create` call). `batch_id` ties tasks from the same call together.
pub fn fire_task_created(
    session_id: &str,
    content: &str,
    active_form: Option<&str>,
    batch_id: &str,
) {
    let input = HookInput::TaskCreated {
        common: observation_common("TaskCreated", session_id),
        content: content.to_string(),
        active_form: active_form.map(|s| s.to_string()),
        batch_id: batch_id.to_string(),
    };
    fire_and_forget(HookEvent::TaskCreated, input);
}

/// Fire a `TaskCompleted` observation hook (a task transitioned to completed).
pub fn fire_task_completed(session_id: &str, task_id: i64, content: &str) {
    let input = HookInput::TaskCompleted {
        common: observation_common("TaskCompleted", session_id),
        task_id,
        content: content.to_string(),
    };
    fire_and_forget(HookEvent::TaskCompleted, input);
}

/// Fire a `ConfigChange` observation hook (app config was written). App-global
/// (no session). `category` is the matcher target (the config domain that
/// changed); `source` is who triggered it.
pub fn fire_config_change(category: &str, source: &str) {
    let input = HookInput::ConfigChange {
        common: observation_common("ConfigChange", ""),
        category: category.to_string(),
        source: source.to_string(),
    };
    fire_and_forget(HookEvent::ConfigChange, input);
}

/// Fire a `CwdChanged` observation hook (a session's working dir changed).
pub fn fire_cwd_changed(session_id: &str, old_cwd: Option<&str>, new_cwd: Option<&str>) {
    let input = HookInput::CwdChanged {
        common: observation_common("CwdChanged", session_id),
        old_cwd: old_cwd.map(|s| s.to_string()),
        new_cwd: new_cwd.map(|s| s.to_string()),
    };
    fire_and_forget(HookEvent::CwdChanged, input);
}

/// Fire a `FileChanged` observation hook (a tool created / edited / patched a
/// file). `path` is the matcher target (regex-matchable file pattern);
/// `action` is `create` / `edit` / `delete` / `patch`. No-op fast path when no
/// FileChanged hook is configured, so it's cheap to call on every file write.
pub fn fire_file_changed(session_id: Option<&str>, path: &str, action: &str) {
    // `fire_and_forget` applies the cwd-aware multi-scope gate.
    let input = HookInput::FileChanged {
        common: observation_common("FileChanged", session_id.unwrap_or("")),
        path: path.to_string(),
        action: action.to_string(),
    };
    fire_and_forget(HookEvent::FileChanged, input);
}

/// Fire a `PermissionRequest` observation hook (a tool approval prompt was
/// raised). `command` is the matcher target (the command / tool being gated).
pub fn fire_permission_request(session_id: Option<&str>, command: &str) {
    let input = HookInput::PermissionRequest {
        common: observation_common("PermissionRequest", session_id.unwrap_or("")),
        command: command.to_string(),
    };
    fire_and_forget(HookEvent::PermissionRequest, input);
}

/// Fire a `PermissionDenied` observation hook (a tool was denied). `reason` is
/// `user_declined` (the user said no to a prompt) or `policy` (engine auto-deny).
pub fn fire_permission_denied(session_id: Option<&str>, command: &str, reason: &str) {
    let input = HookInput::PermissionDenied {
        common: observation_common("PermissionDenied", session_id.unwrap_or("")),
        command: command.to_string(),
        reason: reason.to_string(),
    };
    fire_and_forget(HookEvent::PermissionDenied, input);
}

/// Fire a `UserPromptExpansion` observation hook (a slash command ran).
/// `command` is the matcher target (the command name); `command_text` is the
/// full raw text including args. No-op fast path when nothing is configured.
pub fn fire_user_prompt_expansion(
    session_id: Option<&str>,
    agent_id: &str,
    command: &str,
    command_text: &str,
) {
    let mut common = observation_common("UserPromptExpansion", session_id.unwrap_or(""));
    common.agent_id = Some(agent_id.to_string());
    let input = HookInput::UserPromptExpansion {
        common,
        command: command.to_string(),
        command_text: command_text.to_string(),
    };
    fire_and_forget(HookEvent::UserPromptExpansion, input);
}

/// Fire an `Elicitation` observation hook (`ask_user_question` raised a prompt).
pub fn fire_elicitation(session_id: &str, request_id: &str, question_count: usize) {
    let input = HookInput::Elicitation {
        common: observation_common("Elicitation", session_id),
        request_id: request_id.to_string(),
        question_count,
    };
    fire_and_forget(HookEvent::Elicitation, input);
}

/// Fire an `ElicitationResult` observation hook (an `ask_user_question` group
/// reached a terminal state). `status` is `answered` / `cancelled` / `timeout`.
pub fn fire_elicitation_result(session_id: &str, request_id: &str, status: &str) {
    let input = HookInput::ElicitationResult {
        common: observation_common("ElicitationResult", session_id),
        request_id: request_id.to_string(),
        status: status.to_string(),
    };
    fire_and_forget(HookEvent::ElicitationResult, input);
}

/// Initialize the hooks subsystem during `ha-core` startup. Best-effort: never
/// panics — hooks are an additive capability.
pub fn init() {
    // Load the registry now so dispatch works during the startup window before
    // `spawn_hooks_config_listener` runs — and even if the EventBus is never
    // available (the listener skips its initial reload in that case). The
    // listener reloads again on each `config:changed`.
    registry::reload_from_config();
    app_info!("hooks", "init", "hooks subsystem initialized");
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn session_start_claimed_once_per_session() {
        // Unique ids so the process-global set isn't shared with other tests.
        assert!(claim_session_start("guard-test-sess-A"));
        assert!(!claim_session_start("guard-test-sess-A"));
        // A different session is independent.
        assert!(claim_session_start("guard-test-sess-B"));
    }

    #[test]
    fn compaction_hooks_deduped_within_window() {
        assert!(claim_compaction_hooks("guard-test-compact-A"));
        // Immediate retry (failover) within the window is suppressed.
        assert!(!claim_compaction_hooks("guard-test-compact-A"));
        // A different session is independent.
        assert!(claim_compaction_hooks("guard-test-compact-B"));
    }

    #[test]
    fn pending_prompt_context_set_take_and_clear() {
        // Set then take → returns once, cleared after.
        set_user_prompt_context("guard-test-ups-A", Some("CTX".into()));
        assert_eq!(
            take_user_prompt_context("guard-test-ups-A").as_deref(),
            Some("CTX")
        );
        assert!(take_user_prompt_context("guard-test-ups-A").is_none());
        // Explicit None clears a previously-set slot.
        set_user_prompt_context("guard-test-ups-B", Some("X".into()));
        set_user_prompt_context("guard-test-ups-B", None);
        assert!(take_user_prompt_context("guard-test-ups-B").is_none());
        // Empty string is treated as "no context" → clears.
        set_user_prompt_context("guard-test-ups-C", Some(String::new()));
        assert!(take_user_prompt_context("guard-test-ups-C").is_none());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::hooks::registry::HookRegistry;
    use std::path::PathBuf;

    fn common(event: &str) -> CommonHookInput {
        CommonHookInput {
            session_id: "s1".into(),
            transcript_path: PathBuf::from("/tmp/t.jsonl"),
            cwd: PathBuf::from("/tmp"),
            permission_mode: PermissionMode::Default,
            hook_event_name: event.into(),
            agent_id: None,
            agent_type: None,
        }
    }

    fn post_tool_use(tool: &str) -> HookInput {
        HookInput::PostToolUse {
            common: common("PostToolUse"),
            tool_name: tool.into(),
            tool_input: serde_json::json!({}),
            tool_response: serde_json::json!("ok"),
            tool_use_id: "c1".into(),
        }
    }

    fn pre_tool_use(tool: &str) -> HookInput {
        HookInput::PreToolUse {
            common: common("PreToolUse"),
            tool_name: tool.into(),
            tool_input: serde_json::json!({}),
            tool_use_id: "c1".into(),
        }
    }

    fn user_prompt_submit(prompt: &str) -> HookInput {
        HookInput::UserPromptSubmit {
            common: common("UserPromptSubmit"),
            prompt: prompt.into(),
        }
    }

    fn registry_from(json: &str) -> HookRegistry {
        HookRegistry::from_config(&serde_json::from_str(json).unwrap())
    }

    #[tokio::test]
    async fn matching_command_injects_additional_context() {
        let reg = registry_from(
            r#"{"PostToolUse":[{"matcher":"Write","hooks":[
                {"type":"command","shell":"bash","command":"printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"INJECTED\"}}'"}
            ]}]}"#,
        );
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PostToolUse, post_tool_use("Write"))
                .await;
        assert_eq!(out.merged_additional_context().as_deref(), Some("INJECTED"));
        assert_eq!(out.decision, HookDecision::Allow);
    }

    #[tokio::test]
    async fn non_matching_tool_is_noop() {
        let reg = registry_from(
            r#"{"PostToolUse":[{"matcher":"Write","hooks":[{"type":"command","command":"echo x"}]}]}"#,
        );
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PostToolUse, post_tool_use("Read"))
                .await;
        assert!(out.merged_additional_context().is_none());
    }

    #[tokio::test]
    async fn empty_registry_is_noop() {
        let reg = HookRegistry::empty();
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PostToolUse, post_tool_use("Write"))
                .await;
        assert_eq!(out.decision, HookDecision::Allow);
        assert!(out.merged_additional_context().is_none());
    }

    #[tokio::test]
    async fn exit_two_produces_block_decision() {
        let reg = registry_from(
            r#"{"PreToolUse":[{"matcher":"Bash","hooks":[
                {"type":"command","shell":"bash","command":"echo blocked 1>&2; exit 2"}
            ]}]}"#,
        );
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PreToolUse, pre_tool_use("Bash")).await;
        assert!(matches!(out.decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn block_on_observation_event_is_downgraded() {
        // A PostToolUse hook (observation) exiting 2 would aggregate to Block;
        // dispatch must downgrade it to a non-blocking Allow.
        let reg = registry_from(
            r#"{"PostToolUse":[{"matcher":"Bash","hooks":[
                {"type":"command","shell":"bash","command":"echo nope 1>&2; exit 2"}
            ]}]}"#,
        );
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PostToolUse, post_tool_use("Bash"))
                .await;
        assert_eq!(out.decision, HookDecision::Allow);
        assert!(out.continue_execution);
    }

    #[tokio::test]
    async fn user_prompt_submit_block_is_not_downgraded() {
        // UserPromptSubmit is a blocking event (NOT observation-only), so a
        // `decision:block` must survive aggregation rather than be neutralized.
        let reg = registry_from(
            r#"{"UserPromptSubmit":[{"hooks":[
                {"type":"command","shell":"bash","command":"printf '%s' '{\"decision\":\"block\",\"reason\":\"nope\"}'"}
            ]}]}"#,
        );
        let out = HookDispatcher::dispatch_with(
            &reg,
            HookEvent::UserPromptSubmit,
            user_prompt_submit("hi"),
        )
        .await;
        assert_eq!(
            out.decision,
            HookDecision::Block {
                reason: "nope".into()
            }
        );
    }

    #[tokio::test]
    async fn user_prompt_submit_injects_context() {
        // A non-blocking UserPromptSubmit hook contributes additionalContext.
        let reg = registry_from(
            r#"{"UserPromptSubmit":[{"hooks":[
                {"type":"command","shell":"bash","command":"printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"CTX\"}}'"}
            ]}]}"#,
        );
        let out = HookDispatcher::dispatch_with(
            &reg,
            HookEvent::UserPromptSubmit,
            user_prompt_submit("hi"),
        )
        .await;
        assert_eq!(out.merged_additional_context().as_deref(), Some("CTX"));
        assert_eq!(out.decision, HookDecision::Allow);
    }

    #[tokio::test]
    async fn duplicate_command_deduped() {
        // Two identical commands in two groups → run once.
        let reg = registry_from(
            r#"{"PostToolUse":[
                {"hooks":[{"type":"command","shell":"bash","command":"printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"X\"}}'"}]},
                {"matcher":"Write","hooks":[{"type":"command","shell":"bash","command":"printf '%s' '{\"hookSpecificOutput\":{\"additionalContext\":\"X\"}}'"}]}
            ]}"#,
        );
        let out =
            HookDispatcher::dispatch_with(&reg, HookEvent::PostToolUse, post_tool_use("Write"))
                .await;
        // Deduped: only one "X" injected, not two.
        assert_eq!(out.merged_additional_context().as_deref(), Some("X"));
    }
}

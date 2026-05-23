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
pub mod config;
pub mod decision;
pub mod env;
pub mod matcher;
pub mod parse;
pub mod registry;
pub mod runner;
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
    let seen = SESSION_START_SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = seen.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(session_id.to_string())
}

/// Last time compaction hooks fired per session, for cross-retry de-dup.
static COMPACTION_HOOKS_FIRED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// De-dupe compaction hooks (`PostCompact` / `SessionStart(compact)`) across
/// failover retries: each retry rebuilds the agent and re-runs compaction from
/// the un-compacted DB history, but the user-facing hooks should fire once per
/// actual compaction. Returns `true` if no compaction-hook fire happened for
/// this session within the dedup window (and records it). The window is far
/// below the compaction throttle (`compact.cacheTtlSecs`, default 300s) so a
/// legitimate second compaction is never suppressed, and far above failover's
/// sub-second retries.
pub fn claim_compaction_hooks(session_id: &str) -> bool {
    const WINDOW: Duration = Duration::from_secs(60);
    let map = COMPACTION_HOOKS_FIRED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    guard.retain(|_, t| now.duration_since(*t) < WINDOW);
    if guard.contains_key(session_id) {
        return false;
    }
    guard.insert(session_id.to_string(), now);
    true
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
        Self::dispatch_with(&registry::global(), event, input).await
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
                if seen.insert((h.handler_type(), h.identity())) {
                    handlers.push(h);
                }
            }
        }
        if handlers.is_empty() {
            return HookOutcome::noop();
        }

        let env = env::HookEnv::build_for_command(input.common());
        let start = Instant::now();

        // Concurrent execution with a total circuit-breaker: the longest
        // per-handler timeout + 5s (design §7.8). If it trips, treat the whole
        // dispatch as a non-blocking no-op.
        let breaker = handlers
            .iter()
            .map(|h| h.default_timeout())
            .max()
            .unwrap_or_else(|| Duration::from_secs(30))
            + Duration::from_secs(5);
        let runs = handlers.iter().map(|h| {
            let deadline = Instant::now() + h.default_timeout();
            h.run(&input, &env, deadline)
        });
        let raws = match tokio::time::timeout(breaker, futures_util::future::join_all(runs)).await {
            Ok(r) => r,
            Err(_) => {
                app_warn!(
                    "hooks",
                    "dispatch",
                    "event={} hit the total circuit-breaker — treating as no-op",
                    event.as_str()
                );
                return HookOutcome::noop();
            }
        };

        let contributions = raws.iter().map(|r| parse::parse(r, event)).collect();
        let mut outcome = decision::aggregate(contributions);
        apply_overflow(&mut outcome, event, input.common());
        audit::log_dispatch(event, handlers.len(), &outcome, start.elapsed());
        outcome
    }
}

/// Build a runnable handler from config. This phase executes `command` only;
/// other types are recognized but skipped (http/mcp/prompt/agent land later).
fn build_handler(cfg: &HookHandlerConfig) -> Option<Box<dyn HookHandler>> {
    match cfg {
        HookHandlerConfig::Command(c) => {
            Some(Box::new(runner::command::CommandHandler::new(c.clone())))
        }
        HookHandlerConfig::Http(_)
        | HookHandlerConfig::McpTool(_)
        | HookHandlerConfig::Prompt(_)
        | HookHandlerConfig::Agent(_) => {
            app_debug!(
                "hooks",
                "dispatch",
                "skipping non-command handler (not supported this phase)"
            );
            None
        }
    }
}

/// Enforce the 10 000-char injection cap: spill oversized merged context to an
/// overflow file and replace it inline with a pointer (design §8.6).
fn apply_overflow(outcome: &mut HookOutcome, event: HookEvent, common: &CommonHookInput) {
    let Some(merged) = outcome.merged_additional_context() else {
        return;
    };
    if merged.chars().count() <= audit::MAX_INJECT_CHARS {
        return;
    }
    let pointer = match audit::write_overflow(event, &common.session_id, &merged) {
        Some(p) => format!("<hook output truncated; full content at {}>", p.display()),
        None => "<hook output truncated>".to_string(),
    };
    outcome.additional_context = vec![pointer];
    app_warn!(
        "hooks",
        "dispatch",
        "event={} injected context exceeded {} chars; spilled to overflow file",
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
fn fire_and_forget_runtime() -> Option<&'static tokio::runtime::Runtime> {
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
    if !registry::global().has_handlers_for(event) {
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
    if !registry::global().has_handlers_for(HookEvent::SessionStart)
        || !claim_session_start(session_id)
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

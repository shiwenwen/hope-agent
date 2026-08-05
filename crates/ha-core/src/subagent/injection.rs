use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::helpers::{emit_parent_stream_event, truncate_str, CleanupGuard};
use super::types::{ParentAgentStreamEvent, SubagentStatus};
use super::{
    ActiveInjection, ACTIVE_CHAT_SESSIONS, FETCHED_RUN_IDS, INJECTING_SESSIONS, INJECTION_CANCELS,
    PENDING_INJECTIONS, SESSION_IDLE_NOTIFY,
};

type InjectionReceiptStep = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;

/// Durable source receipt carried with an injection and all of its in-process
/// retries.
///
/// An attached IM mirror can perform a persistent provider mutation before the
/// parent turn settles. It therefore calls `arm_no_replay` before starting the
/// engine. That write must make startup recovery skip this source; a confirmed
/// cancellation may still retry from [`PENDING_INJECTIONS`] in this process,
/// but a crash deliberately loses that automatic retry rather than duplicate an
/// IM reply. `settle` records an ordinary terminal landing and must be
/// idempotent because fetched/cancel races can converge on the same source.
#[derive(Clone)]
pub struct OnInjected {
    arm_no_replay: InjectionReceiptStep,
    settle: InjectionReceiptStep,
    no_replay_armed: Arc<AtomicBool>,
}

impl OnInjected {
    pub(crate) fn new(
        arm_no_replay: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
        settle: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            arm_no_replay: Arc::new(arm_no_replay),
            settle: Arc::new(settle),
            no_replay_armed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Use one idempotent durable write for both phases (for example,
    /// `background_jobs.injected = 1`).
    pub(crate) fn idempotent(
        step: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        let step: InjectionReceiptStep = Arc::new(step);
        Self {
            arm_no_replay: step.clone(),
            settle: step,
            no_replay_armed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn arm_no_replay(&self) -> anyhow::Result<()> {
        if self.no_replay_armed.load(Ordering::Acquire) {
            return Ok(());
        }
        (self.arm_no_replay)()?;
        self.no_replay_armed.store(true, Ordering::Release);
        Ok(())
    }

    fn is_no_replay_armed(&self) -> bool {
        self.no_replay_armed.load(Ordering::Acquire)
    }

    fn settle(&self) -> anyhow::Result<()> {
        (self.settle)()
    }
}

fn settle_injection_source(receipt: Option<&OnInjected>, run_id: &str) {
    let Some(receipt) = receipt else { return };
    if let Err(error) = receipt.settle() {
        app_error!(
            "subagent",
            "inject",
            "Failed to persist terminal source receipt for run {}: {}",
            run_id,
            crate::logging::redact_sensitive(&error.to_string())
        );
    }
}

/// Establish the durable replay owner, persist the parent injection row, then
/// prepare the engine call in that strict order.
///
/// `persist` must contain every durable parent-session mutation that identifies
/// this attempt. A cross-process CAS loser (or any arm error) never invokes it,
/// and `start_engine` is invoked only after both arm and persistence succeed.
/// The receipt deliberately remains armed when persistence fails: callers must
/// treat that as a safe terminal failure instead of reviving startup replay.
fn arm_source_persist_then<T>(
    mirror_attached: bool,
    receipt: Option<&OnInjected>,
    persist: impl FnOnce() -> anyhow::Result<()>,
    start_engine: impl FnOnce(bool) -> T,
) -> anyhow::Result<T> {
    let mut no_replay_armed = receipt.is_some_and(OnInjected::is_no_replay_armed);
    if mirror_attached {
        if let Some(receipt) = receipt {
            receipt.arm_no_replay()?;
            no_replay_armed = true;
        }
    }
    persist()?;
    Ok(start_engine(no_replay_armed))
}

/// Preserve retry idempotency without treating a failed dedup lookup as
/// "missing". In particular, a read error must not fall through to append a
/// second parent row.
fn persist_parent_injection_row_if_missing(
    already_written: impl FnOnce() -> anyhow::Result<bool>,
    append: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if already_written()? {
        Ok(())
    } else {
        append()
    }
}

/// Result of one `inject_and_run_parent` attempt. Lets the caller decide
/// whether the source record is done (`Injected`), owned by the retry queue
/// (`Queued`), or must stay pending for restart replay (`Abandoned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionOutcome {
    /// Parent turn ran (or the result was already fetched / all models failed
    /// terminally), or a pre-engine persistence failure safely terminated an
    /// already-armed source. The source is settled, or its durable no-replay
    /// fence remains armed — nothing more should be replayed.
    Injected,
    /// Deferred: another injection holds the session, or the user pre-empted
    /// this turn. The task was pushed to `PENDING_INJECTIONS` (carrying its
    /// `on_injected`); the next flush owns the retry. Caller must NOT mark the
    /// source injected.
    Queued,
    /// Could not persist or re-queue the attempt (a poisoned `PENDING_INJECTIONS`
    /// lock — the only remaining path here now that the idle-timeout re-queues as
    /// `Queued`), or a pre-engine durable arm failure. Unless another process
    /// already owns the source, its replay marker remains pending for restart
    /// recovery (MISC-15: an abandoned injection must not look delivered). An
    /// unarmed parent-row read/write failure follows this path as well.
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionImTerminal {
    Failed,
    InterruptedWillRetry,
    InterruptedConsumed,
}

impl InjectionImTerminal {
    /// Static, user-facing IM copy only. Raw provider / model errors must never
    /// cross this boundary because they may contain credentials or request
    /// details. Detailed diagnostics remain in the local session event below.
    fn body(self) -> &'static str {
        match self {
            Self::Failed => {
                "⚠️ **Background follow-up failed** — this reply has stopped. Please try again later."
            }
            Self::InterruptedWillRetry => {
                "⏸️ **Background follow-up interrupted** — a new message took priority. It will retry automatically when the conversation is idle."
            }
            Self::InterruptedConsumed => {
                "⏹️ **Background follow-up stopped** — the result was already retrieved in this conversation, so it will not retry."
            }
        }
    }
}

struct ParentInjectionSink {
    parent_session_id: String,
    run_id: String,
}

impl crate::chat_engine::EventSink for ParentInjectionSink {
    fn send(&self, event: &str) {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "delta".into(),
            parent_session_id: self.parent_session_id.clone(),
            run_id: self.run_id.clone(),
            push_message: None,
            delta: Some(event.to_string()),
            error: None,
        });
    }
}

/// A deferred injection task that was cancelled and needs to be retried.
#[derive(Clone)]
pub(super) struct PendingInjection {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub run_id: String,
    pub push_message: String,
    pub session_db: Arc<crate::session::SessionDB>,
    /// Carried so a deferred injection still marks its source done when the
    /// queued attempt eventually lands. `None` for subagent runs.
    pub on_injected: Option<OnInjected>,
    /// Keeps a verified first-party HTTP UI approval path alive while this
    /// parent follow-up waits behind a foreground turn.
    pub reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
}

fn claim_next_pending_injection(
    queue: &mut Vec<PendingInjection>,
    injecting: &mut std::collections::HashSet<String>,
    session_id: &str,
) -> Option<PendingInjection> {
    if injecting.contains(session_id) {
        return None;
    }
    let task = queue
        .iter()
        .position(|task| task.parent_session_id == session_id)
        .map(|index| queue.remove(index));
    if task.is_some() {
        injecting.insert(session_id.to_string());
    }
    task
}

/// Claim and re-trigger the next pending injection for a session.
/// Called from ChatSessionGuard::drop when a user chat completes.
pub(crate) fn flush_pending_injections(session_id: &str) {
    loop {
        // Atomically (under the established INJECTING -> PENDING lock order)
        // remove one matching task and reserve the session for it. Without the
        // preclaim, two concurrent CleanupGuard / ChatSessionGuard drops could
        // dequeue A and B before either spawned runtime registered itself,
        // rotating the same-session FIFO suffix when B re-queued.
        let task = {
            let mut injecting = INJECTING_SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
            let mut queue = match PENDING_INJECTIONS.lock() {
                Ok(q) => q,
                Err(p) => p.into_inner(),
            };
            claim_next_pending_injection(&mut queue, &mut injecting, session_id)
        };
        let Some(task) = task else { return };

        // Skip if already fetched, and clean up the entry
        let already_fetched = {
            let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
            set.remove(&task.run_id)
        };
        if already_fetched {
            INJECTING_SESSIONS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(session_id);
            continue;
        }
        let t = task.clone();
        let preclaimed_cleanup = CleanupGuard {
            session_id: t.parent_session_id.clone(),
        };
        std::thread::spawn(move || {
            // Own the dequeue-time session reservation until the async
            // injection returns. If thread/runtime construction fails, dropping
            // the captured guard still releases the claim and advances FIFO.
            let mut preclaimed_cleanup = Some(preclaimed_cleanup);
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => {
                    // Outcome is ignored here: a successful run fires the carried
                    // `on_injected` internally, and a re-cancel re-queues itself.
                    let _ = rt.block_on(inject_and_run_parent_with_ui_guard(
                        t.parent_session_id,
                        t.parent_agent_id,
                        t.child_agent_id,
                        t.run_id,
                        t.push_message,
                        t.session_db,
                        t.on_injected,
                        t.reattachable_ui_guard,
                        preclaimed_cleanup.take(),
                    ));
                }
                Err(e) => app_error!(
                    "subagent",
                    "inject",
                    "Failed to build runtime for retry: {}",
                    e
                ),
            }
        });
        return; // Next task stays queued until this one's CleanupGuard fires.
    }
}

/// Build the push message text injected into the parent session.
pub(crate) fn build_subagent_push_message(
    thread_id: &str,
    run_id: &str,
    agent_id: &str,
    task: &str,
    status: &SubagentStatus,
    duration_ms: u64,
    result: Option<&str>,
    error: Option<&str>,
    terminal_reason: Option<crate::subagent::SubagentTerminalReason>,
) -> String {
    let duration = format!("{:.1}s", duration_ms as f64 / 1000.0);
    let result_block = result
        .filter(|text| !text.trim().is_empty())
        .map(|text| format!("<result>\n{}\n</result>\n", escape_xml_text(text.trim())))
        .unwrap_or_default();
    let error_block = error
        .filter(|text| !text.trim().is_empty())
        .map(|text| format!("<error>\n{}\n</error>\n", escape_xml_text(text.trim())))
        .unwrap_or_default();
    let output_block = if result_block.is_empty() && error_block.is_empty() {
        "<result>(no output)</result>\n".to_string()
    } else {
        format!("{}{}", result_block, error_block)
    };
    let summary = format!(
        "Sub-agent \"{}\" finished with status \"{}\" in {}.",
        agent_id,
        status.as_str(),
        duration
    );
    let terminal_reason =
        terminal_reason.unwrap_or(crate::subagent::SubagentTerminalReason::Unknown);
    format!(
        "<subagent-result>\n\
         <thread-id>{}</thread-id>\n\
         <run-id>{}</run-id>\n\
         <agent>{}</agent>\n\
         <status>{}</status>\n\
         <terminal-reason>{}</terminal-reason>\n\
         <resume-allowed>{}</resume-allowed>\n\
         <resume-recommended>{}</resume-recommended>\n\
         <duration-ms>{}</duration-ms>\n\
         <duration>{}</duration>\n\
         <task>{}</task>\n\
         {}\
         <summary>{}</summary>\n\
         </subagent-result>",
        escape_xml_text(thread_id),
        escape_xml_text(run_id),
        escape_xml_text(agent_id),
        escape_xml_text(status.as_str()),
        escape_xml_text(terminal_reason.as_str()),
        terminal_reason.resume_allowed(),
        terminal_reason.resume_recommended(),
        duration_ms,
        escape_xml_text(&duration),
        escape_xml_text(&truncate_str(task, 50)),
        output_block,
        escape_xml_text(&summary)
    )
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// E2 / DELETE-3 / INCOG-3 backstop: is the parent session still present?
/// An absent row (deleted or incognito-burned) must abort the injection before
/// it resurrects a ghost turn (a billed LLM round + persisted rows against a
/// session that no longer exists). A transient lookup error is treated as
/// *alive* so a momentary glitch doesn't drop a real injection —
/// `dispatch_injection` already fired the primary gate, and the idle-timeout
/// path leaves the source row for restart replay.
fn parent_session_present(db: &crate::session::SessionDB, session_id: &str) -> bool {
    !matches!(db.get_session(session_id), Ok(None))
}

/// `child_agent_id` label used by `crate::wakeup` when reusing this injection
/// pipeline for a self-scheduled wakeup (R10). `inject_and_run_parent` branches
/// on it to write a `wakeup_trigger` marker instead of `subagent_result`.
pub(crate) const WAKEUP_CHILD_AGENT_ID: &str = "wakeup";
pub(crate) const PROCESS_NOTIFICATION_CHILD_AGENT_ID: &str = "process_notification";
pub const LOOP_CHILD_AGENT_ID: &str = "loop";
pub(crate) const WORKFLOW_CHILD_AGENT_ID: &str = "workflow";

/// Outcome of waiting for a parent session to become idle before injecting.
enum IdleWait {
    /// No foreground turn is active — safe to inject now.
    Idle,
    /// `should_abort` fired (e.g. the agent already fetched the result via a
    /// `check`/`result` tool action) — caller treats the injection as handled.
    Aborted,
    /// Timed out waiting for the session to go idle — caller abandons the
    /// attempt (the source row stays for restart replay).
    TimedOut,
}

/// Wait until `session_id` has no active foreground chat turn, or until
/// `should_abort` fires, or `max_wait` elapses.
///
/// Foreground turns are tracked in `ACTIVE_CHAT_SESSIONS` by
/// [`ChatSessionGuard`](super::ChatSessionGuard), created at the shared
/// `run_chat_engine` entry (R2) so this gate holds across desktop / HTTP / IM /
/// cron — and at the ACP turn boundary for ACP. The wait is event-driven on
/// `SESSION_IDLE_NOTIFY` (fired when a guard releases) with a bounded fallback
/// poll so a missed notification can't park forever. The fallback is clamped to
/// the time remaining before `max_wait` so the timeout is honored promptly
/// regardless of the 5s poll cadence.
async fn wait_for_session_idle(
    session_id: &str,
    max_wait: std::time::Duration,
    should_abort: impl Fn() -> bool,
) -> IdleWait {
    let fallback_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let is_busy = ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .copied()
            .unwrap_or(0)
            > 0;
        if !is_busy {
            return IdleWait::Idle;
        }
        if start.elapsed() >= max_wait {
            return IdleWait::TimedOut;
        }
        if should_abort() {
            return IdleWait::Aborted;
        }
        // Wait for notify (instant wake) or the fallback poll (in case notify is
        // missed). Cap the poll at the remaining budget so timeout is honored
        // without overshooting by up to a full poll interval.
        let remaining = max_wait.saturating_sub(start.elapsed());
        let sleep_dur = fallback_interval.min(remaining.max(std::time::Duration::from_millis(1)));
        tokio::select! {
            _ = SESSION_IDLE_NOTIFY.notified() => {}
            _ = tokio::time::sleep(sleep_dur) => {}
        }
    }
}

/// Backend-driven result injection: wait for idle, then run the parent agent with the push message.
/// Respects user chat priority: waits if busy, cancels if user sends a new message, skips if
/// the agent already fetched the result via check/result tool actions.
pub async fn inject_and_run_parent(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
    on_injected: Option<OnInjected>,
) -> InjectionOutcome {
    inject_and_run_parent_with_ui_guard(
        parent_session_id,
        parent_agent_id,
        child_agent_id,
        run_id,
        push_message,
        session_db,
        on_injected,
        None,
        None,
    )
    .await
}

/// Variant used by first-party UI descendant work. The lease is moved into
/// `PENDING_INJECTIONS` whenever delivery is deferred, so closing/reloading the
/// browser never converts a later parent follow-up approval into unattended.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inject_and_run_parent_with_ui_guard(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
    on_injected: Option<OnInjected>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    preclaimed_cleanup: Option<CleanupGuard>,
) -> InjectionOutcome {
    use crate::provider;

    let session_preclaimed = preclaimed_cleanup.is_some();
    let mut _cleanup = preclaimed_cleanup;

    // 0. Skip if the parent agent already fetched this result via check/result tool
    {
        let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
        if set.contains(&run_id) {
            app_info!(
                "subagent",
                "inject",
                "Run {} already fetched by parent, skipping injection",
                &run_id
            );
            set.remove(&run_id); // Clean up — no longer needed
            settle_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Injected;
        }
    }

    // E2 / DELETE-3 / INCOG-3 backstop (entry): mirror dispatch_injection's gate
    // in case the session was already gone by the time this attempt starts. Fire
    // `on_injected` (consume the source so replay won't retry a dead session)
    // and bail — this is `Injected`, not `Abandoned`.
    if !parent_session_present(&session_db, &parent_session_id) {
        app_info!(
            "subagent",
            "inject",
            "Parent session {} gone; skipping injection for run {}",
            &parent_session_id,
            &run_id
        );
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    // Guard: if another injection is active for this session, queue for later
    if !session_preclaimed {
        let mut guard = INJECTING_SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        if guard.contains(&parent_session_id) {
            app_info!(
                "subagent",
                "inject",
                "Session {} already has active injection, queuing for later",
                &parent_session_id
            );
            match PENDING_INJECTIONS.lock() {
                Ok(mut queue) => {
                    queue.push(PendingInjection {
                        parent_session_id,
                        parent_agent_id,
                        child_agent_id,
                        run_id,
                        push_message,
                        session_db,
                        on_injected,
                        reattachable_ui_guard,
                    });
                    return InjectionOutcome::Queued;
                }
                // Couldn't enqueue (poisoned): leave the source pending for
                // replay rather than firing on_injected on a dropped task.
                Err(_) => return InjectionOutcome::Abandoned,
            }
        }
        guard.insert(parent_session_id.clone());
        _cleanup = Some(CleanupGuard {
            session_id: parent_session_id.clone(),
        });
    }

    // 1. Wait for parent session to become idle (event-driven with timeout
    // fallback). The idle gate (`ACTIVE_CHAT_SESSIONS`) is now populated by
    // `ChatSessionGuard` at the shared `run_chat_engine` entry (R2), so this
    // wait correctly parks behind live turns on every entry point, not just
    // desktop.
    let announce_timeout = crate::agent_loader::load_agent(&parent_agent_id)
        .ok()
        .and_then(|def| def.config.subagents.announce_timeout_secs)
        .unwrap_or(120)
        .clamp(10, 600);
    let max_wait = std::time::Duration::from_secs(announce_timeout);
    match wait_for_session_idle(&parent_session_id, max_wait, || {
        // Re-check if the result was fetched while we were waiting.
        FETCHED_RUN_IDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&run_id)
    })
    .await
    {
        IdleWait::Idle => {}
        IdleWait::TimedOut => {
            // G3/G5: the parent session stayed busy past `announce_timeout`.
            // Re-queue (carrying `on_injected`) instead of abandoning to
            // restart-replay — `PENDING_INJECTIONS` flushes when the long
            // foreground turn ends (`ChatSessionGuard::drop`), so the completion
            // surfaces this run instead of waiting for the next process start.
            // Critical for subagent / Group injections (`on_injected = None`),
            // which have no `injected=0` restart-replay backstop — a Group's
            // merged injection (row `injected=true`, out of replay) would
            // otherwise be lost permanently. `on_injected` is carried but NOT
            // fired, so a tool job's row stays un-injected (MISC-15: an
            // undelivered injection must not look delivered) and the restart
            // backstop is preserved.
            app_warn!(
                "subagent",
                "inject",
                "Session {} still busy after idle wait; re-queuing injection for run {}",
                &parent_session_id,
                &run_id
            );
            return match PENDING_INJECTIONS.lock() {
                Ok(mut queue) => {
                    queue.push(PendingInjection {
                        parent_session_id,
                        parent_agent_id,
                        child_agent_id,
                        run_id,
                        push_message,
                        session_db,
                        on_injected,
                        reattachable_ui_guard,
                    });
                    InjectionOutcome::Queued
                }
                // Couldn't re-queue (poisoned): leave the source pending for
                // restart replay rather than firing on_injected on a dropped task.
                Err(_) => InjectionOutcome::Abandoned,
            };
        }
        IdleWait::Aborted => {
            app_info!(
                "subagent",
                "inject",
                "Run {} fetched while waiting, skipping",
                &run_id
            );
            settle_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Injected;
        }
    }

    // Final check before proceeding
    if FETCHED_RUN_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&run_id)
    {
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    // 2. Register cancel flag — user's chat() will set this to abort the injection
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = INJECTION_CANCELS.lock() {
        map.insert(
            parent_session_id.clone(),
            ActiveInjection {
                run_id: run_id.clone(),
                cancel: cancel.clone(),
            },
        );
    }
    // Ensure cancel flag is cleaned up on all exit paths
    let cancel_cleanup_sid = parent_session_id.clone();
    struct CancelCleanup {
        sid: String,
    }
    impl Drop for CancelCleanup {
        fn drop(&mut self) {
            if let Ok(mut map) = INJECTION_CANCELS.lock() {
                map.remove(&self.sid);
            }
        }
    }
    let _cancel_cleanup = CancelCleanup {
        sid: cancel_cleanup_sid,
    };

    // 3. Emit "started" so frontend can show loading state
    emit_parent_stream_event(&ParentAgentStreamEvent {
        event_type: "started".into(),
        parent_session_id: parent_session_id.clone(),
        run_id: run_id.clone(),
        push_message: Some(push_message.clone()),
        delta: None,
        error: None,
    });

    // 4. Build model chain
    let store = crate::config::cached_config();
    let agent_model_config = crate::agent_loader::load_agent(&parent_agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();
    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);
    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain.iter().any(|m: &crate::provider::ActiveModel| {
            m.provider_id == fb.provider_id && m.model_id == fb.model_id
        }) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        app_error!(
            "subagent",
            "inject",
            "No model configured for parent agent {}",
            &parent_agent_id
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id: parent_session_id.clone(),
            run_id: run_id.clone(),
            push_message: None,
            delta: None,
            error: Some("No model configured for parent agent".into()),
        });
        // Persistent misconfiguration: mark injected so a restart doesn't
        // re-inject in a loop. The tool output is still saved to disk; only
        // the notification is dropped, and the parent can't run without a model.
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    let mut last_error = String::new();
    let mut succeeded = false;
    // Captured at the engine-error boundary so the IM terminal copy and the
    // later queue decision use the same fetched/not-fetched observation.
    let mut cancelled_while_running_fetched: Option<bool> = None;
    let mut engine_failed_without_cancel = false;
    let mut im_terminal_safe_for_retry = true;
    let mut source_no_replay_armed = on_injected
        .as_ref()
        .is_some_and(OnInjected::is_no_replay_armed);

    // E2 / DELETE-3 / INCOG-3 backstop (post-idle): the most dangerous window —
    // the session can be deleted or burned *during* the idle wait above. Re-check
    // before writing the push row or running a billed turn against a dead session.
    if !parent_session_present(&session_db, &parent_session_id) {
        app_info!(
            "subagent",
            "inject",
            "Parent session {} gone during idle wait; skipping injection for run {}",
            &parent_session_id,
            &run_id
        );
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    // Acquire after the potentially long idle wait but before writing the push
    // row. This closes the terminal-subagent/delete race without pinning the
    // Agent for the entire wait; the engine keeps its own admission backstop.
    let _agent_admission = match crate::agent_lifecycle::begin_agent_run(&parent_agent_id) {
        Ok(guard) => guard,
        Err(error) => {
            app_warn!(
                "subagent",
                "inject",
                "Parent agent {} became unavailable before injection {}: {}",
                &parent_agent_id,
                &run_id,
                error
            );
            return InjectionOutcome::Abandoned;
        }
    };

    // The foreground HTTP turn may already have returned. Keep the dormant
    // eval root identity alive while this real parent-injection turn runs so
    // its model/tool calls remain in the originating trial rather than
    // becoming unattributed background usage.
    let _eval_injection_guard = crate::eval_context::retain_session(&parent_session_id);

    if cancel.load(Ordering::SeqCst) {
        app_info!(
            "subagent",
            "inject",
            "Injection cancelled before attempt for session {}",
            &parent_session_id
        );
    } else {
        let parent_agent_def = crate::agent_loader::load_agent(&parent_agent_id).ok();

        // G1: if the parent session is attached to an IM chat, mirror this
        // injection turn into it so an IM-origin background task's completion
        // reaches the IM user (per the account's `imReplyMode`). Reuses the
        // GUI↔IM live mirror; the engine's own attach gates `ParentInjection`
        // out, so we drive it here and AWAIT finalize/abort below — this runs on
        // a short-lived current-thread runtime whose drop would cancel a spawned
        // finalize. `None` when there's no IM attach (desktop-only / no channel).
        let injection_mirror =
            crate::channel_hooks::attach_injection_mirror(&parent_session_id).await;

        // Attach first to determine whether this attempt has an external IM
        // mutation surface. If so, claim its durable replay source before
        // writing *any* parent user row; a cross-process CAS loser must leave
        // the session untouched. With no mirror, this deliberately skips the
        // arm and retains the existing at-least-once restart contract.
        let resolved_reasoning_effort = parent_agent_def
            .as_ref()
            .and_then(|def| def.config.model.reasoning_effort.clone())
            .or(crate::agent::live_reasoning_effort(None).await);
        let engine_params = crate::chat_engine::ChatEngineParams {
            session_id: parent_session_id.clone(),
            agent_id: parent_agent_id.clone(),
            turn_id: None,
            message: push_message.clone(),
            display_text: None,
            attachments: Vec::new(),
            session_db: session_db.clone(),
            model_chain,
            providers: store.providers.clone(),
            codex_token: None,
            resolved_temperature: parent_agent_def
                .as_ref()
                .and_then(|def| def.config.model.temperature)
                .or(store.temperature),
            compact_config: store.compact.clone(),
            extra_system_context: None,
            reasoning_effort: resolved_reasoning_effort,
            cancel: cancel.clone(),
            plan_context_override: None,
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            tool_scope: None,
            subagent_depth: 0,
            steer_run_id: None,
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: true,
            persist_final_error_event: false,
            source: crate::chat_engine::stream_seq::ChatSource::ParentInjection,
            ui_surface: None,
            origin_source: None,
            // Parent-injection turns are owner-internal, never IM. No opt-in gate.
            channel_kb_context: None,
            event_sink: Arc::new(ParentInjectionSink {
                parent_session_id: parent_session_id.clone(),
                run_id: run_id.clone(),
            }),
        };
        let engine_result = arm_source_persist_then(
            injection_mirror.is_some(),
            on_injected.as_ref(),
            || {
                // Write the push user row BEFORE agent.chat() so intermediate
                // rows streamed from the callback land between it and the final
                // assistant row. Re-queued attempts retain the run_id and reuse
                // this idempotency guard.
                persist_parent_injection_row_if_missing(
                    || session_db.has_injection_user_msg(&parent_session_id, &run_id),
                    || {
                        let mut user_msg = crate::session::NewMessage::user(&push_message)
                            .with_source(crate::chat_engine::ChatSource::ParentInjection);
                        // A wakeup is a trigger rather than a subagent result.
                        // Every shape retains run_id because the dedup lookup
                        // above uses it to recognize confirmed in-process retries.
                        let meta = if child_agent_id == WAKEUP_CHILD_AGENT_ID {
                            serde_json::json!({ "wakeup_trigger": { "run_id": &run_id } })
                        } else if child_agent_id == LOOP_CHILD_AGENT_ID {
                            serde_json::json!({ "loop_trigger": { "run_id": &run_id } })
                        } else if child_agent_id == PROCESS_NOTIFICATION_CHILD_AGENT_ID {
                            serde_json::json!({ "process_notification": { "run_id": &run_id } })
                        } else if child_agent_id == WORKFLOW_CHILD_AGENT_ID {
                            serde_json::json!({ "workflow_result": { "run_id": &run_id } })
                        } else {
                            serde_json::json!({
                                "subagent_result": {
                                    "run_id": &run_id,
                                    "agent_id": &child_agent_id,
                                }
                            })
                        };
                        user_msg.attachments_meta = Some(meta.to_string());
                        session_db
                            .append_injection_user_msg_if_missing(
                                &parent_session_id,
                                &run_id,
                                &user_msg,
                            )
                            .map(|_| ())
                    },
                )
            },
            |armed| {
                source_no_replay_armed = armed;
                crate::chat_engine::run_chat_engine(engine_params)
            },
        );
        let engine = match engine_result {
            Ok(engine) => engine,
            Err(error) => {
                let durable_no_replay_armed = on_injected
                    .as_ref()
                    .is_some_and(OnInjected::is_no_replay_armed);
                app_error!(
                    "subagent",
                    "inject",
                    "Failed to prepare parent injection for run {} (no_replay_armed={}): {}",
                    &run_id,
                    durable_no_replay_armed,
                    crate::logging::redact_sensitive(&error.to_string())
                );
                if let Some(state) = injection_mirror {
                    let _ = state.abort(None).await;
                }
                emit_parent_stream_event(&ParentAgentStreamEvent {
                    event_type: "error".into(),
                    parent_session_id,
                    run_id,
                    push_message: None,
                    delta: None,
                    error: Some(
                        "Background follow-up was not started because its delivery state could not be saved"
                            .into(),
                    ),
                });
                // Never settle here. If arm succeeded, its durable no-replay
                // fence is the terminal safety decision; reviving the source
                // could duplicate a provider-side reply after a crash. Without
                // such a fence (including the no-mirror path), leave the source
                // pending for the existing at-least-once restart replay.
                return if durable_no_replay_armed {
                    InjectionOutcome::Injected
                } else {
                    InjectionOutcome::Abandoned
                };
            }
        };

        match engine.await {
            Ok(result) => {
                // run_chat_engine returning Ok means the reply was persisted.
                // Mark succeeded unconditionally — even if cancel flipped to
                // true after Ok was produced (user started new chat in the
                // narrow post-return window), re-queueing would write a
                // duplicate sub-agent completion to the parent conversation.
                let model_label = result
                    .model_used
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "(unknown model)".to_string());
                app_info!(
                    "subagent",
                    "inject",
                    "Parent agent {} responded via model {}",
                    &parent_agent_id,
                    model_label
                );
                succeeded = true;
                crate::eval_context::record_lifecycle_event(
                    Some(&parent_session_id),
                    "handoff",
                    "agent.result_injected",
                    Some(&run_id),
                    "completed",
                    0,
                );
                // G1: deliver the mirrored injection turn to IM (per imReplyMode).
                // Awaited so it completes before this current-thread runtime drops.
                if let Some(state) = injection_mirror {
                    state.finalize(&result.response).await;
                }
                // G2: if this is a cron run session, fan the injected result out to
                // the cron job's delivery_targets (the inline run delivered its own
                // response; a background job spawned during the run completes later
                // and would otherwise reach nobody). No-op for non-cron sessions.
                crate::cron_hooks::deliver_injection_for_session(
                    &parent_session_id,
                    &result.response,
                )
                .await;
            }
            Err(e) => {
                let was_cancelled = cancel.load(Ordering::SeqCst);
                let fetched_while_active = was_cancelled
                    && FETCHED_RUN_IDS
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(&run_id);
                let terminal = if was_cancelled {
                    cancelled_while_running_fetched = Some(fetched_while_active);
                    app_info!(
                        "subagent",
                        "inject",
                        "Injection cancelled (error path) for session {} (result_fetched={})",
                        &parent_session_id,
                        fetched_while_active
                    );
                    if fetched_while_active {
                        InjectionImTerminal::InterruptedConsumed
                    } else {
                        InjectionImTerminal::InterruptedWillRetry
                    }
                } else {
                    engine_failed_without_cancel = true;
                    last_error = e;
                    InjectionImTerminal::Failed
                };
                // G1: a ParentInjection has no user-quote, but its Message/Card
                // preview can already be visible. Terminate that same preview
                // identity with bounded static copy before a retry can create a
                // second reply. Native mirrors use their provider abort path.
                if let Some(state) = injection_mirror {
                    im_terminal_safe_for_retry = state
                        .abort(Some(terminal.body().to_string()))
                        .await
                        .is_confirmed();
                }
            }
        }
    }

    // All models failed (not cancelled): surface a terminal event row so
    // the log doesn't show a silent user push without a response.
    if engine_failed_without_cancel {
        let _ = session_db.append_message(
            &parent_session_id,
            &crate::session::NewMessage::error_event(&format!("[injection failed] {}", last_error))
                .with_source(crate::chat_engine::ChatSource::ParentInjection),
        );
    }

    // 6. Emit final event. Order matters: a successful Ok already persisted
    // the reply, so even if cancel was set after the run completed, we must
    // not re-queue (would duplicate the sub-agent completion in the parent
    // conversation).
    let was_cancelled =
        !succeeded && !engine_failed_without_cancel && cancel.load(Ordering::SeqCst);
    let fetched_while_active = cancelled_while_running_fetched.unwrap_or_else(|| {
        FETCHED_RUN_IDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&run_id)
    });
    if was_cancelled && !im_terminal_safe_for_retry {
        // A persistent provider mutation may still be visible. Starting a new
        // mirror for this result would risk partial + full double delivery, so
        // keep the write-ahead source fence armed. The durable child/job result
        // remains inspectable, but neither the current process nor startup
        // recovery may send it again automatically.
        app_warn!(
            "subagent",
            "inject",
            "Injection for run {} was cancelled but its IM mirror terminal is unconfirmed; automatic retry suppressed",
            &run_id
        );
        crate::eval_context::record_lifecycle_event(
            Some(&parent_session_id),
            "handoff",
            "agent.result_injected",
            Some(&run_id),
            "terminal_ambiguous_no_replay",
            0,
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(
                "Cancelled: previous IM reply could not be closed safely; automatic retry was suppressed"
                    .into(),
            ),
        });
        InjectionOutcome::Injected
    } else if was_cancelled && fetched_while_active {
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
        InjectionOutcome::Injected
    } else if was_cancelled {
        // Re-queue for retry after the user's chat completes, carrying
        // on_injected so the eventual landing still marks the source done.
        let requeued = match PENDING_INJECTIONS.lock() {
            Ok(mut queue) => {
                queue.push(PendingInjection {
                    parent_session_id: parent_session_id.clone(),
                    parent_agent_id: parent_agent_id.clone(),
                    child_agent_id,
                    run_id: run_id.clone(),
                    push_message,
                    session_db,
                    on_injected,
                    reattachable_ui_guard,
                });
                true
            }
            Err(_) => false,
        };
        app_info!(
            "subagent",
            "inject",
            "Injection for run {} cancelled, re-queued for next idle",
            &run_id
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some("Cancelled: user started new chat, will retry when idle".into()),
        });
        if requeued {
            InjectionOutcome::Queued
        } else if source_no_replay_armed {
            // The provider terminal is confirmed, but the in-memory queue could
            // not accept ownership. Do not revive the already-armed durable
            // replay source; at-most-once wins over an automatic duplicate.
            InjectionOutcome::Injected
        } else {
            // Couldn't re-queue (poisoned): leave the source pending for replay.
            InjectionOutcome::Abandoned
        }
    } else if succeeded {
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
        InjectionOutcome::Injected
    } else {
        // All models failed: a terminal error row was persisted above. Mark
        // injected so the failure isn't re-injected on every restart.
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(format!("All models failed: {}", last_error)),
        });
        InjectionOutcome::Injected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_push_message_uses_xmlish_payload_and_escapes_text() {
        let msg = build_subagent_push_message(
            "thread<&",
            "run<&",
            "agent>&",
            "read <file> & report",
            &SubagentStatus::Completed,
            1234,
            Some("ok <done> & safe"),
            None,
            Some(crate::subagent::SubagentTerminalReason::Success),
        );

        assert!(msg.starts_with("<subagent-result>"));
        assert!(msg.contains("<thread-id>thread&lt;&amp;</thread-id>"));
        assert!(msg.contains("<run-id>run&lt;&amp;</run-id>"));
        assert!(msg.contains("<agent>agent&gt;&amp;</agent>"));
        assert!(msg.contains("<task>read &lt;file&gt; &amp; report</task>"));
        assert!(msg.contains("<result>\nok &lt;done&gt; &amp; safe\n</result>"));
        assert!(!msg.contains("BEGIN_SUBAGENT_RESULT"));
    }

    #[test]
    fn injection_im_terminal_copy_is_static_and_describes_retry_semantics() {
        let failed = InjectionImTerminal::Failed.body();
        let retry = InjectionImTerminal::InterruptedWillRetry.body();
        let consumed = InjectionImTerminal::InterruptedConsumed.body();

        assert!(failed.contains("failed"));
        assert!(retry.contains("retry automatically"));
        assert!(consumed.contains("will not retry"));

        // The IM copy is selected from a closed enum and never interpolates the
        // raw engine error, provider response, token, or request URL.
        for body in [failed, retry, consumed] {
            assert!(!body.contains("sk-test-secret"));
            assert!(!body.contains("provider.example"));
        }
    }

    #[test]
    fn durable_arm_failure_never_persists_parent_row_or_starts_engine() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                anyhow::bail!("durable CAS lost")
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            move || {
                persist_steps.lock().unwrap().push("persist-parent-row");
                Ok(())
            },
            move |_| engine_steps.lock().unwrap().push("start-engine"),
        );

        assert!(result.is_err());
        assert_eq!(*steps.lock().unwrap(), ["arm"]);
        assert!(!receipt.is_no_replay_armed());
    }

    #[test]
    fn armed_parent_dedup_read_failure_skips_append_and_engine() {
        let receipt = OnInjected::new(|| Ok(()), || Ok(()));
        let append_called = Arc::new(AtomicBool::new(false));
        let append_flag = append_called.clone();
        let engine_started = Arc::new(AtomicBool::new(false));
        let engine_flag = engine_started.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            || {
                persist_parent_injection_row_if_missing(
                    || anyhow::bail!("dedup read failed"),
                    move || {
                        append_flag.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            },
            move |_| engine_flag.store(true, Ordering::SeqCst),
        );

        assert!(result.is_err());
        assert!(!append_called.load(Ordering::SeqCst));
        assert!(!engine_started.load(Ordering::SeqCst));
        assert!(receipt.is_no_replay_armed());
    }

    #[test]
    fn armed_parent_write_failure_keeps_fence_and_never_starts_engine() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                Ok(())
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            move || {
                persist_parent_injection_row_if_missing(
                    || {
                        persist_steps.lock().unwrap().push("read-parent-row");
                        Ok(false)
                    },
                    || {
                        persist_steps.lock().unwrap().push("persist-parent-row");
                        anyhow::bail!("parent row write failed")
                    },
                )
            },
            move |_| engine_steps.lock().unwrap().push("start-engine"),
        );

        assert!(result.is_err());
        assert_eq!(
            *steps.lock().unwrap(),
            ["arm", "read-parent-row", "persist-parent-row"]
        );
        assert!(receipt.is_no_replay_armed());
    }

    #[test]
    fn no_mirror_keeps_source_replayable_until_parent_turn_settles() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                Ok(())
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let armed = arm_source_persist_then(
            false,
            Some(&receipt),
            move || {
                persist_steps.lock().unwrap().push("persist-parent-row");
                Ok(())
            },
            move |armed| {
                engine_steps.lock().unwrap().push("start-engine");
                armed
            },
        )
        .unwrap();

        assert!(!armed);
        assert_eq!(
            *steps.lock().unwrap(),
            ["persist-parent-row", "start-engine"]
        );
        assert!(!receipt.is_no_replay_armed());
    }

    #[test]
    fn pending_flush_claims_one_and_preserves_same_session_fifo() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let pending = |session: &str, run: &str| PendingInjection {
            parent_session_id: session.to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
        };
        let mut queue = vec![
            pending("s1", "run-1"),
            pending("s2", "run-2"),
            pending("s1", "run-3"),
        ];
        let mut injecting = std::collections::HashSet::new();

        assert_eq!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1")
                .unwrap()
                .run_id,
            "run-1"
        );
        assert_eq!(
            queue
                .iter()
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-2", "run-3"]
        );
        assert!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1").is_none(),
            "a concurrent flush must not dequeue the same-session suffix"
        );
        assert_eq!(
            queue
                .iter()
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-2", "run-3"]
        );
        injecting.remove("s1");
        assert_eq!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1")
                .unwrap()
                .run_id,
            "run-3"
        );
        assert_eq!(queue[0].run_id, "run-2");
    }

    // R2 (§5.4): the idle gate must park completion injection behind a live
    // foreground turn on *every* entry point. These exercise the shared wait
    // helper against `ChatSessionGuard` (the same guard `run_chat_engine` now
    // creates for HTTP / IM / cron, and ACP creates at its turn boundary).

    #[tokio::test]
    async fn wait_for_session_idle_parks_until_guard_released() {
        let sid = "test-r2-wait-idle-parks";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);

        // A live foreground turn holds the guard → busy → a bounded wait times
        // out rather than firing (injection would NOT splice into a live turn).
        let guard = crate::subagent::ChatSessionGuard::new(sid);
        let outcome =
            wait_for_session_idle(sid, std::time::Duration::from_millis(120), || false).await;
        assert!(matches!(outcome, IdleWait::TimedOut));

        // Releasing the turn makes the session idle → the next wait returns Idle.
        drop(guard);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || false).await;
        assert!(matches!(outcome, IdleWait::Idle));
    }

    #[tokio::test]
    async fn wait_for_session_idle_aborts_when_should_abort_fires() {
        let sid = "test-r2-wait-idle-abort";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);

        // Busy, but the agent already fetched the result → Aborted (caller
        // fires on_injected and returns Injected without running a turn).
        let _guard = crate::subagent::ChatSessionGuard::new(sid);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || true).await;
        assert!(matches!(outcome, IdleWait::Aborted));
    }

    #[tokio::test]
    async fn wait_for_session_idle_idle_when_no_turn_active() {
        let sid = "test-r2-wait-idle-noturn";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || false).await;
        assert!(matches!(outcome, IdleWait::Idle));
    }
}

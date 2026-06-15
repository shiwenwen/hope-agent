use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::helpers::{emit_parent_stream_event, truncate_str, CleanupGuard};
use super::types::{ParentAgentStreamEvent, SubagentStatus};
use super::{
    ACTIVE_CHAT_SESSIONS, FETCHED_RUN_IDS, INJECTING_SESSIONS, INJECTION_CANCELS,
    PENDING_INJECTIONS, SESSION_IDLE_NOTIFY,
};

/// Callback fired exactly once, on the dedicated injection OS-thread, when an
/// injection reaches its terminal **Injected** state (the parent turn ran and
/// persisted, the result was already consumed, or it failed terminally). Tool
/// jobs pass a closure that marks the `async_tool_jobs` row injected; subagent
/// runs pass `None`. Carried through the re-queue so a deferred injection still
/// marks its source done when the queued attempt eventually lands.
pub(crate) type OnInjected = Arc<dyn Fn() + Send + Sync>;

/// Result of one `inject_and_run_parent` attempt. Lets the caller decide
/// whether the source record is done (`Injected`), owned by the retry queue
/// (`Queued`), or must stay pending for restart replay (`Abandoned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InjectionOutcome {
    /// Parent turn ran (or the result was already fetched / all models failed
    /// terminally). `on_injected` has fired — nothing more to do.
    Injected,
    /// Deferred: another injection holds the session, or the user pre-empted
    /// this turn. The task was pushed to `PENDING_INJECTIONS` (carrying its
    /// `on_injected`); the next flush owns the retry. Caller must NOT mark the
    /// source injected.
    Queued,
    /// Gave up after waiting for the session to go idle (announce_timeout).
    /// Nothing persisted, nothing re-queued, `on_injected` NOT fired — the
    /// source row stays un-injected so `replay_pending_jobs()` retries it on
    /// the next restart (MISC-15: abandoned must not look delivered).
    Abandoned,
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
}

/// Drain and re-trigger pending injections for a session.
/// Called from ChatSessionGuard::drop when a user chat completes.
pub(crate) fn flush_pending_injections(session_id: &str) {
    let tasks: Vec<PendingInjection> = {
        let mut queue = match PENDING_INJECTIONS.lock() {
            Ok(q) => q,
            Err(p) => p.into_inner(),
        };
        let mut remaining = Vec::new();
        let mut to_run = Vec::new();
        for task in queue.drain(..) {
            if task.parent_session_id == session_id {
                to_run.push(task);
            } else {
                remaining.push(task);
            }
        }
        *queue = remaining;
        to_run
    };

    for task in tasks {
        // Skip if already fetched, and clean up the entry
        {
            let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
            if set.remove(&task.run_id) {
                continue;
            }
        }
        let t = task.clone();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => {
                    // Outcome is ignored here: a successful run fires the carried
                    // `on_injected` internally, and a re-cancel re-queues itself.
                    let _ = rt.block_on(inject_and_run_parent(
                        t.parent_session_id,
                        t.parent_agent_id,
                        t.child_agent_id,
                        t.run_id,
                        t.push_message,
                        t.session_db,
                        t.on_injected,
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
        break; // Only re-trigger one at a time; next one queues on completion
    }
}

/// Build the push message text injected into the parent session.
pub(crate) fn build_subagent_push_message(
    run_id: &str,
    agent_id: &str,
    task: &str,
    status: &SubagentStatus,
    duration_ms: u64,
    result: Option<&str>,
    error: Option<&str>,
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
    format!(
        "<subagent-result>\n\
         <run-id>{}</run-id>\n\
         <agent>{}</agent>\n\
         <status>{}</status>\n\
         <duration-ms>{}</duration-ms>\n\
         <duration>{}</duration>\n\
         <task>{}</task>\n\
         {}\
         <summary>{}</summary>\n\
         </subagent-result>",
        escape_xml_text(run_id),
        escape_xml_text(agent_id),
        escape_xml_text(status.as_str()),
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
pub(crate) async fn inject_and_run_parent(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
    on_injected: Option<OnInjected>,
) -> InjectionOutcome {
    use crate::provider;

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
            if let Some(cb) = on_injected.as_ref() {
                cb();
            }
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
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
        return InjectionOutcome::Injected;
    }

    // Guard: if another injection is active for this session, queue for later
    {
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
                    });
                    return InjectionOutcome::Queued;
                }
                // Couldn't enqueue (poisoned): leave the source pending for
                // replay rather than firing on_injected on a dropped task.
                Err(_) => return InjectionOutcome::Abandoned,
            }
        }
        guard.insert(parent_session_id.clone());
    }
    let _cleanup = CleanupGuard {
        session_id: parent_session_id.clone(),
    };

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
            app_warn!(
                "subagent",
                "inject",
                "Timed out waiting for session {} to become idle, skipping",
                &parent_session_id
            );
            return InjectionOutcome::Abandoned;
        }
        IdleWait::Aborted => {
            app_info!(
                "subagent",
                "inject",
                "Run {} fetched while waiting, skipping",
                &run_id
            );
            if let Some(cb) = on_injected.as_ref() {
                cb();
            }
            return InjectionOutcome::Injected;
        }
    }

    // Final check before proceeding
    if FETCHED_RUN_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&run_id)
    {
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
        return InjectionOutcome::Injected;
    }

    // 2. Register cancel flag — user's chat() will set this to abort the injection
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut map) = INJECTION_CANCELS.lock() {
        map.insert(parent_session_id.clone(), cancel.clone());
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
            run_id,
            push_message: None,
            delta: None,
            error: Some("No model configured for parent agent".into()),
        });
        // Persistent misconfiguration: mark injected so a restart doesn't
        // re-inject in a loop. The tool output is still saved to disk; only
        // the notification is dropped, and the parent can't run without a model.
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
        return InjectionOutcome::Injected;
    }

    let mut last_error = String::new();
    let mut succeeded = false;

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
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
        return InjectionOutcome::Injected;
    }

    // Write the push user row BEFORE agent.chat() so intermediate rows
    // streamed from the callback land between it and the final assistant
    // row in id order — `parseSessionMessages` on the frontend groups
    // pending tool/text blocks under the next assistant, so user → tool*
    // → assistant ordering is load-bearing. Idempotent across re-queued
    // attempts (cancelled injections are retried via PENDING_INJECTIONS).
    let user_msg_already_written = session_db
        .has_injection_user_msg(&parent_session_id, &run_id)
        .unwrap_or(false);
    if !user_msg_already_written {
        let mut user_msg = crate::session::NewMessage::user(&push_message)
            .with_source(crate::chat_engine::ChatSource::ParentInjection);
        // Tag the injected row so the frontend renders it as the right kind of
        // system chip. A self-scheduled wakeup (R10) is a *trigger*, not a
        // sub-agent *result* — stamping `subagent_result` made it render as a
        // misleading green "completed" pill with the note dropped, so wakeups
        // get their own `wakeup_trigger` marker (mirrors cron's `cron_trigger`).
        let meta = if child_agent_id == WAKEUP_CHILD_AGENT_ID {
            serde_json::json!({ "wakeup_trigger": {} })
        } else {
            serde_json::json!({
                "subagent_result": {
                    "run_id": &run_id,
                    "agent_id": &child_agent_id,
                }
            })
        };
        user_msg.attachments_meta = Some(meta.to_string());
        let _ = session_db.append_message(&parent_session_id, &user_msg);
    }

    if cancel.load(Ordering::SeqCst) {
        app_info!(
            "subagent",
            "inject",
            "Injection cancelled before attempt for session {}",
            &parent_session_id
        );
    } else {
        let parent_agent_def = crate::agent_loader::load_agent(&parent_agent_id).ok();

        match crate::chat_engine::run_chat_engine(crate::chat_engine::ChatEngineParams {
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
            reasoning_effort: parent_agent_def
                .as_ref()
                .and_then(|def| def.config.model.reasoning_effort.clone())
                .or(crate::agent::live_reasoning_effort(None).await),
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
            origin_source: None,
            // Parent-injection turns are owner-internal, never IM. No opt-in gate.
            channel_kb_context: None,
            event_sink: Arc::new(ParentInjectionSink {
                parent_session_id: parent_session_id.clone(),
                run_id: run_id.clone(),
            }),
        })
        .await
        {
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
            }
            Err(e) => {
                if cancel.load(Ordering::SeqCst) {
                    app_info!(
                        "subagent",
                        "inject",
                        "Injection cancelled (error path) for session {}",
                        &parent_session_id
                    );
                } else {
                    last_error = e;
                }
            }
        }
    }

    // All models failed (not cancelled): surface a terminal event row so
    // the log doesn't show a silent user push without a response.
    if !succeeded && !cancel.load(Ordering::SeqCst) {
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
    let was_cancelled = !succeeded && cancel.load(Ordering::SeqCst);
    if was_cancelled {
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
        } else {
            // Couldn't re-queue (poisoned): leave the source pending for replay.
            InjectionOutcome::Abandoned
        }
    } else if succeeded {
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
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
        if let Some(cb) = on_injected.as_ref() {
            cb();
        }
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
            "run<&",
            "agent>&",
            "read <file> & report",
            &SubagentStatus::Completed,
            1234,
            Some("ok <done> & safe"),
            None,
        );

        assert!(msg.starts_with("<subagent-result>"));
        assert!(msg.contains("<run-id>run&lt;&amp;</run-id>"));
        assert!(msg.contains("<agent>agent&gt;&amp;</agent>"));
        assert!(msg.contains("<task>read &lt;file&gt; &amp; report</task>"));
        assert!(msg.contains("<result>\nok &lt;done&gt; &amp; safe\n</result>"));
        assert!(!msg.contains("BEGIN_SUBAGENT_RESULT"));
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

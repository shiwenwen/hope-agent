use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::helpers::{emit_parent_stream_event, truncate_str, CleanupGuard};
use super::types::{ParentAgentStreamEvent, SubagentStatus};
use super::{
    ACTIVE_CHAT_SESSIONS, FETCHED_RUN_IDS, INJECTING_SESSIONS, INJECTION_CANCELS,
    PENDING_INJECTIONS, SESSION_IDLE_NOTIFY,
};

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
                Ok(rt) => rt.block_on(inject_and_run_parent(
                    t.parent_session_id,
                    t.parent_agent_id,
                    t.child_agent_id,
                    t.run_id,
                    t.push_message,
                    t.session_db,
                )),
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
    let content = result.or(error).unwrap_or("(no output)");
    format!(
        "[Sub-Agent Completion — auto-delivered]\nRun ID: {}\nAgent: {}\nTask: {}\nStatus: {}\nDuration: {}\n<<<BEGIN_SUBAGENT_RESULT>>>\n{}\n<<<END_SUBAGENT_RESULT>>>",
        run_id, agent_id, truncate_str(task, 50), status.as_str(), duration, content
    )
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
) {
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
            return;
        }
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
            if let Ok(mut queue) = PENDING_INJECTIONS.lock() {
                queue.push(PendingInjection {
                    parent_session_id,
                    parent_agent_id,
                    child_agent_id,
                    run_id,
                    push_message,
                    session_db,
                });
            }
            return;
        }
        guard.insert(parent_session_id.clone());
    }
    let _cleanup = CleanupGuard {
        session_id: parent_session_id.clone(),
    };

    // 1. Wait for parent session to become idle (event-driven with timeout fallback)
    let announce_timeout = crate::agent_loader::load_agent(&parent_agent_id)
        .ok()
        .and_then(|def| def.config.subagents.announce_timeout_secs)
        .unwrap_or(120)
        .clamp(10, 600);
    let max_wait = std::time::Duration::from_secs(announce_timeout);
    let fallback_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let is_busy = ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&parent_session_id);
        if !is_busy {
            break;
        }

        if start.elapsed() > max_wait {
            app_warn!(
                "subagent",
                "inject",
                "Timed out waiting for session {} to become idle, skipping",
                &parent_session_id
            );
            return;
        }
        // Re-check if result was fetched while we were waiting
        if FETCHED_RUN_IDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&run_id)
        {
            app_info!(
                "subagent",
                "inject",
                "Run {} fetched while waiting, skipping",
                &run_id
            );
            return;
        }
        // Wait for notify (instant wake) or fallback timeout (in case notify is missed)
        tokio::select! {
            _ = SESSION_IDLE_NOTIFY.notified() => {}
            _ = tokio::time::sleep(fallback_interval) => {}
        }
    }

    // Final check before proceeding
    if FETCHED_RUN_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&run_id)
    {
        return;
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
        return;
    }

    let mut last_error = String::new();
    let mut succeeded = false;

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
        let mut user_msg = crate::session::NewMessage::user(&push_message);
        user_msg.attachments_meta = Some(
            serde_json::json!({
                "subagent_result": {
                    "run_id": &run_id,
                    "agent_id": &child_agent_id,
                }
            })
            .to_string(),
        );
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
            message: push_message.clone(),
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
            reasoning_effort: crate::agent::live_reasoning_effort(None).await,
            cancel: cancel.clone(),
            plan_context_override: None,
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            subagent_depth: 0,
            steer_run_id: None,
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: true,
            persist_final_error_event: false,
            source: crate::chat_engine::stream_seq::ChatSource::ParentInjection,
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
            &crate::session::NewMessage::error_event(&format!("[injection failed] {}", last_error)),
        );
    }

    // 6. Emit final event. Order matters: a successful Ok already persisted
    // the reply, so even if cancel was set after the run completed, we must
    // not re-queue (would duplicate the sub-agent completion in the parent
    // conversation).
    let was_cancelled = !succeeded && cancel.load(Ordering::SeqCst);
    if was_cancelled {
        // Re-queue for retry after the user's chat completes
        if let Ok(mut queue) = PENDING_INJECTIONS.lock() {
            queue.push(PendingInjection {
                parent_session_id: parent_session_id.clone(),
                parent_agent_id: parent_agent_id.clone(),
                child_agent_id,
                run_id: run_id.clone(),
                push_message,
                session_db,
            });
        }
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
    } else if succeeded {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
    } else {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(format!("All models failed: {}", last_error)),
        });
    }
}

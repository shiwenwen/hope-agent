//! Core ACP Agent implementation.
//!
//! Implements all ACP Agent interface methods by translating between
//! ACP protocol and the existing Hope Agent AssistantAgent + SessionDB.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use anyhow::Result;
use serde_json::Value;

use crate::acp::event_mapper;
use crate::acp::protocol::NdJsonTransport;
use crate::acp::session::{now_epoch_secs, AcpSession, AcpSessionStore};
use crate::acp::types::*;
use ha_core::agent::{AssistantAgent, CurrentUserMessageState};
use ha_core::chat_engine::EventSink;
use ha_core::failover;
use ha_core::provider;
use ha_core::session::{self, SessionDB, SessionIdeContext};
use ha_core::turn_durability::{FlushReason, TurnDurabilitySink};

const ACP_CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
const ACP_CANCEL_COOPERATIVE_GRACE: std::time::Duration = std::time::Duration::from_secs(6);
const ACP_INBOUND_QUEUE_CAPACITY: usize = 256;
const ACP_PERMISSION_REQUEST_ID_PREFIX: &str = "acp-permission:";

type AcpCancelStates = Arc<Mutex<HashMap<String, Arc<AcpCancelState>>>>;

fn current_user_message_state_for_retry_base(
    contains_current_user: bool,
) -> CurrentUserMessageState {
    if contains_current_user {
        CurrentUserMessageState::AlreadyInHistory
    } else {
        CurrentUserMessageState::MissingFromHistory
    }
}

struct AcpCancelState {
    inner: Mutex<AcpCancelStateInner>,
}

struct AcpCancelStateInner {
    internal_session_id: Option<String>,
    cancel: Arc<AtomicBool>,
    armed: bool,
    cleanup_started: bool,
}

impl AcpCancelState {
    fn new(internal_session_id: Option<String>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            inner: Mutex::new(AcpCancelStateInner {
                internal_session_id,
                cancel,
                armed: false,
                cleanup_started: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, AcpCancelStateInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Arm a fresh, turn-owned token before the prompt is handed to the
    /// synchronous request dispatcher. Keeping tokens per turn prevents a
    /// late old future from being revived when the next prompt starts.
    fn prepare_prompt(&self) -> Arc<AtomicBool> {
        let mut inner = self.lock();
        if !inner.armed {
            Self::arm_fresh_prompt(&mut inner);
        }
        inner.cancel.clone()
    }

    fn prepare_prompt_for_enqueue(&self) -> Option<Arc<AtomicBool>> {
        let mut inner = self.lock();
        if inner.armed {
            return None;
        }
        Self::arm_fresh_prompt(&mut inner);
        Some(inner.cancel.clone())
    }

    fn arm_fresh_prompt(inner: &mut AcpCancelStateInner) {
        inner.cancel = Arc::new(AtomicBool::new(false));
        inner.armed = true;
        inner.cleanup_started = false;
    }

    fn rollback_prompt_enqueue(&self, prepared_cancel: &Arc<AtomicBool>) {
        let mut inner = self.lock();
        if inner.armed && Arc::ptr_eq(&inner.cancel, prepared_cancel) {
            inner.armed = false;
            inner.cleanup_started = false;
        }
    }

    fn attach(&self, internal_session_id: String) -> Option<String> {
        let mut inner = self.lock();
        inner.internal_session_id = Some(internal_session_id.clone());
        let should_start_cleanup =
            inner.armed && inner.cancel.load(Ordering::Acquire) && !inner.cleanup_started;
        if should_start_cleanup {
            inner.cleanup_started = true;
            Some(internal_session_id)
        } else {
            None
        }
    }

    fn request_cancel(&self) -> Option<String> {
        let mut inner = self.lock();
        if !inner.armed {
            return None;
        }
        inner.cancel.store(true, Ordering::SeqCst);
        if inner.cleanup_started {
            return None;
        }
        let internal_session_id = inner.internal_session_id.clone()?;
        inner.cleanup_started = true;
        Some(internal_session_id)
    }

    fn finish_prompt(&self) {
        let mut inner = self.lock();
        inner.armed = false;
        inner.cleanup_started = false;
        // Deliberately retain the old token and its value. A cancellation-
        // unaware future may still be unwinding after the ACP response; only
        // prepare_prompt installs a distinct token for the next turn.
    }

    /// Linearization point between a natural/error completion and a Stop.
    /// Once completion claims the state, a later notification belongs after
    /// this prompt and must not rewrite its terminal result.
    fn claim_non_cancelled_completion(&self) -> bool {
        let mut inner = self.lock();
        if inner.cancel.load(Ordering::Acquire) {
            return false;
        }
        inner.armed = false;
        inner.cleanup_started = false;
        true
    }

    /// Ordering point between `session/cancel` and durable prompt submission.
    /// The claim is short and contains no SQLite I/O: cancellation before it
    /// suppresses persistence, while cancellation after it belongs to a prompt
    /// that has already been accepted for persistence.
    fn claim_non_cancelled_persistence(&self) -> bool {
        let inner = self.lock();
        if inner.cancel.load(Ordering::Acquire) || !inner.armed {
            return false;
        }
        true
    }

    fn force_cancel(&self) -> Option<String> {
        let mut inner = self.lock();
        inner.cancel.store(true, Ordering::SeqCst);
        if inner.cleanup_started {
            return None;
        }
        let internal_session_id = inner.internal_session_id.clone()?;
        inner.cleanup_started = true;
        Some(internal_session_id)
    }
}

struct AcpPromptStateGuard {
    state: Arc<AcpCancelState>,
}

impl Drop for AcpPromptStateGuard {
    fn drop(&mut self) {
        self.state.finish_prompt();
    }
}

struct AcpPromptEnqueueGuard {
    state: Arc<AcpCancelState>,
    prepared_cancel: Option<Arc<AtomicBool>>,
}

enum AcpPromptEnqueueDecision {
    NotPrompt,
    Prepared(AcpPromptEnqueueGuard),
    RejectActive,
}

impl AcpPromptEnqueueGuard {
    fn prepare(msg: &JsonRpcMessage, states: &AcpCancelStates) -> AcpPromptEnqueueDecision {
        if msg.method.as_deref() != Some("session/prompt") || msg.id.is_none() {
            return AcpPromptEnqueueDecision::NotPrompt;
        }
        let Some(session_id) = msg
            .params
            .as_ref()
            .and_then(|params| params.get("sessionId"))
            .and_then(Value::as_str)
        else {
            return AcpPromptEnqueueDecision::NotPrompt;
        };
        let Some(state) = lock_cancel_states(states).get(session_id).cloned() else {
            return AcpPromptEnqueueDecision::NotPrompt;
        };
        match state.prepare_prompt_for_enqueue() {
            Some(prepared_cancel) => AcpPromptEnqueueDecision::Prepared(AcpPromptEnqueueGuard {
                state,
                prepared_cancel: Some(prepared_cancel),
            }),
            None => AcpPromptEnqueueDecision::RejectActive,
        }
    }

    fn commit(mut self) {
        self.prepared_cancel = None;
    }
}

impl Drop for AcpPromptEnqueueGuard {
    fn drop(&mut self) {
        if let Some(prepared_cancel) = self.prepared_cancel.as_ref() {
            self.state.rollback_prompt_enqueue(prepared_cancel);
        }
    }
}

enum AcpInbound {
    Message(JsonRpcMessage),
    Response(JsonRpcResponse),
    ParseError(String),
    IoError(String),
    Eof,
}

fn lock_cancel_states(
    states: &AcpCancelStates,
) -> MutexGuard<'_, HashMap<String, Arc<AcpCancelState>>> {
    states
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn observe_acp_control_message(msg: &JsonRpcMessage, states: &AcpCancelStates) -> Option<String> {
    let method = msg.method.as_deref()?;
    let session_id = msg
        .params
        .as_ref()
        .and_then(|params| params.get("sessionId"))
        .and_then(Value::as_str)?;
    match method {
        "session/prompt" => {
            if let Some(state) = lock_cancel_states(states).get(session_id).cloned() {
                state.prepare_prompt();
            }
            None
        }
        "session/cancel" => lock_cancel_states(states)
            .get(session_id)
            .cloned()
            .and_then(|state| state.request_cancel()),
        _ => None,
    }
}

fn install_acp_permission_forwarder() {
    ha_core::tools::set_approval_request_forwarder(Some(Arc::new(|request| {
        let Some(session_id) = request.session_id.as_deref() else {
            return false;
        };
        let strict = request
            .reason
            .as_ref()
            .is_some_and(|reason| reason.kind.is_strict());
        let mut options = vec![serde_json::json!({
            "optionId": "allow-once",
            "name": "Allow once",
            "kind": "allow_once"
        })];
        if !strict && !request.incognito {
            options.push(serde_json::json!({
                "optionId": "allow-always",
                "name": "Always allow",
                "kind": "allow_always"
            }));
        }
        options.push(serde_json::json!({
            "optionId": "reject-once",
            "name": "Reject",
            "kind": "reject_once"
        }));
        let outbound = serde_json::json!({
            "jsonrpc": "2.0",
            "id": format!("{ACP_PERMISSION_REQUEST_ID_PREFIX}{}", request.request_id),
            "method": "session/request_permission",
            "params": {
                "sessionId": session_id,
                "toolCall": {
                    "toolCallId": request.request_id,
                    "title": request.command,
                    "status": "pending",
                    "rawInput": {
                        "cwd": request.cwd,
                        "reason": request.reason,
                    }
                },
                "options": options
            }
        });
        let Ok(line) = serde_json::to_string(&outbound) else {
            return false;
        };
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        writeln!(handle, "{line}")
            .and_then(|_| handle.flush())
            .is_ok()
    })));
}

fn handle_acp_permission_response(msg: &JsonRpcMessage) -> bool {
    let Some(request_id) = msg
        .id
        .as_ref()
        .and_then(Value::as_str)
        .and_then(|id| id.strip_prefix(ACP_PERMISSION_REQUEST_ID_PREFIX))
    else {
        return false;
    };
    let option_id = msg
        .result
        .as_ref()
        .and_then(|result| result.get("outcome"))
        .and_then(|outcome| {
            (outcome.get("outcome").and_then(Value::as_str) == Some("selected"))
                .then(|| outcome.get("optionId").and_then(Value::as_str))
                .flatten()
        });
    let request_id = request_id.to_string();
    let option_id = option_id.map(str::to_string);
    let _ = std::thread::Builder::new()
        .name("ha-acp-permission-response".to_string())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                let response = match option_id.as_deref() {
                    Some("allow-once") => ha_core::tools::ApprovalResponse::AllowOnce,
                    Some("allow-always")
                        if ha_core::tools::pending_approval_allows_always(&request_id).await =>
                    {
                        ha_core::tools::ApprovalResponse::AllowAlways
                    }
                    _ => ha_core::tools::ApprovalResponse::Deny,
                };
                let _ = ha_core::tools::submit_approval_response(
                    &request_id,
                    response,
                    ha_core::tools::ApprovalResolutionSource::Acp,
                )
                .await;
            });
        });
    true
}

fn spawn_acp_stop_cleanup(db: Arc<SessionDB>, internal_session_id: String) {
    let session_for_log = internal_session_id.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("ha-acp-stop".to_string())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                app_warn!(
                    "acp",
                    "stop",
                    "failed to create ACP Stop cleanup runtime for session {}",
                    internal_session_id
                );
                return;
            };
            let _ = runtime.block_on(ha_core::chat_engine::stop::stop_session(
                db,
                &internal_session_id,
                None,
                true,
            ));
        })
    {
        app_warn!(
            "acp",
            "stop",
            "failed to start ACP Stop cleanup thread for session {}: {}",
            session_for_log,
            error
        );
    }
}

async fn wait_for_acp_cancel(cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(ACP_CANCEL_POLL_INTERVAL).await;
    }
}

fn finalize_acp_user_stop(
    runtime: &tokio::runtime::Runtime,
    db: &Arc<SessionDB>,
    durability: &Arc<ha_core::chat_engine::durability::StreamCoordinator>,
    session_id: &str,
) -> Result<()> {
    let durable_seq = match runtime.block_on(durability.flush(FlushReason::Stop)) {
        Ok(seq) => seq,
        Err(error) => {
            app_warn!(
                "acp",
                "stop",
                "failed to flush stopped ACP turn for session {}: {}",
                session_id,
                error
            );
            durability.snapshot().durable_seq
        }
    };
    if let Err(error) = runtime.block_on(durability.reconcile_spool_to_sqlite()) {
        app_warn!(
            "acp",
            "stop",
            "failed to reconcile stopped ACP turn for session {}: {}",
            session_id,
            error
        );
    }

    let (attempt_no, final_seq, visible_events, provider_kind) = if durability.is_persistent() {
        let snapshot = db
            .stream_run_snapshot(durability.persistence_run_id())?
            .ok_or_else(|| anyhow::anyhow!("ACP persistence run disappeared during Stop"))?;
        let (attempt_no, final_seq, events, _) =
            session::select_recoverable_attempt_prefix(&snapshot);
        let provider_kind = snapshot
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_no == attempt_no)
            .and_then(|attempt| attempt.provider_shape.as_deref())
            .or(snapshot.run.provider_shape.as_deref())
            .and_then(ha_core::chat_engine::finalize::ProviderApiKind::from_shape);
        (attempt_no, final_seq, events, provider_kind)
    } else {
        let snapshot = durability.snapshot();
        (
            durability.current_attempt_no(),
            durable_seq,
            snapshot.events,
            durability
                .current_provider_shape()
                .as_deref()
                .and_then(ha_core::chat_engine::finalize::ProviderApiKind::from_shape),
        )
    };
    let trailing = session::trailing_text_from_journal_events(&visible_events);
    let (stored_context, context_checkpoint_seq, context_revision) = if durability.is_persistent() {
        db.recovery_context_for_prefix(durability.persistence_run_id(), attempt_no, final_seq)?
    } else {
        let (context, revision) = db
            .load_context_with_revision(session_id)
            .unwrap_or((None, durability.context_revision()));
        (context, 0, revision)
    };
    let mut history: Vec<serde_json::Value> = stored_context
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    ha_core::chat_engine::finalize::rebuild::append_journal_suffix_to_history(
        &mut history,
        &visible_events,
        context_checkpoint_seq,
        provider_kind,
    )?;
    history.push(serde_json::json!({
        "role": "assistant",
        "content": ha_core::chat_engine::finalize::copy::model_marker(
            &ha_core::chat_engine::finalize::TerminationReason::UserStop,
        ),
    }));
    let assistant = session::journal_events_have_assistant_output(&visible_events).then(|| {
        session::NewMessage::assistant(&trailing).with_source(ha_core::chat_engine::ChatSource::Acp)
    });
    let commit = session::CommitInterruptedTurn {
        run_id: durability
            .is_persistent()
            .then(|| durability.persistence_run_id().to_string()),
        attempt_no,
        session_id: session_id.to_string(),
        assistant,
        context_json: serde_json::to_string(&history)?,
        expected_context_revision: context_revision,
        turn_id: None,
        final_seq,
        status: session::ChatTurnStatus::Interrupted,
        interrupt_reason: Some("user_stop".to_string()),
        error: None,
        recovery_event: None,
        request_plan: durability.interrupted_request_plan_commit(
            session::RequestPlanResponseOutcome::CancelledAfterResponse,
        ),
    };
    db.commit_interrupted_turn(&commit)?;
    runtime.block_on(durability.finalize_interrupted_request_after_turn_commit(
        session::RequestPlanResponseOutcome::CancelledAfterResponse,
    ))?;
    durability.mark_interrupted("interrupted");
    Ok(())
}

/// ACP's live transport is fed by the coordinator's post-durability output
/// hook. Writing from the model callback would expose bytes before the journal
/// or emergency spool had acknowledged them.
struct AcpDurableEventSink {
    session_id: String,
    message_id: String,
}

impl EventSink for AcpDurableEventSink {
    fn send(&self, event: &str) {
        let Some(notification) =
            event_mapper::map_agent_event(&self.session_id, &self.message_id, event)
        else {
            return;
        };
        let Ok(json) = serde_json::to_string(&notification) else {
            return;
        };
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        let _ = writeln!(handle, "{}", json);
        let _ = handle.flush();
    }
}

/// ACP Agent server — handles incoming JSON-RPC requests/notifications
pub struct AcpAgent {
    pub transport: NdJsonTransport,
    pub sessions: AcpSessionStore,
    pub session_db: Arc<SessionDB>,
    pub initialized: bool,
    pub protocol_version: AcpProtocolVersion,
    pub verbose: bool,
    /// Default agent ID (from CLI flag)
    pub default_agent_id: String,
    cancel_states: AcpCancelStates,
}

fn persist_acp_ide_context(db: &Arc<SessionDB>, session_id: &str, meta: &Option<Value>) {
    let Some(value) = meta
        .as_ref()
        .and_then(|meta| meta.get("ideContext").or_else(|| meta.get("ide_context")))
        .cloned()
    else {
        return;
    };
    match serde_json::from_value::<SessionIdeContext>(value) {
        Ok(context) => {
            if let Err(err) = db.save_session_ide_context(session_id, context) {
                ha_core::app_warn!(
                    "acp",
                    "ide_context",
                    "failed to persist ACP IDE context for session {}: {}",
                    session_id,
                    err
                );
            }
        }
        Err(err) => {
            ha_core::app_warn!(
                "acp",
                "ide_context",
                "invalid ACP ideContext for session {}: {}",
                session_id,
                err
            );
        }
    }
}

fn acp_prompt_blocked_by_stop_cleanup(internal_session_id: &str) -> bool {
    ha_core::chat_engine::active_turn::stop_cleanup_active(internal_session_id)
}

impl AcpAgent {
    pub fn new(session_db: Arc<SessionDB>, default_agent_id: String, verbose: bool) -> Self {
        Self {
            transport: NdJsonTransport::new(),
            sessions: AcpSessionStore::new(32),
            session_db,
            initialized: false,
            protocol_version: AcpProtocolVersion::V1(ACP_PROTOCOL_VERSION_V1),
            verbose,
            default_agent_id,
            cancel_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn reconcile_cancel_states(&self) {
        let live: Vec<(String, String, Arc<AtomicBool>)> = self
            .sessions
            .list()
            .into_iter()
            .map(|session| {
                (
                    session.session_id.clone(),
                    session.internal_session_id.clone(),
                    session.cancel.clone(),
                )
            })
            .collect();
        let live_ids: HashSet<String> = live
            .iter()
            .map(|(session_id, _, _)| session_id.clone())
            .collect();
        let mut cleanup_sessions = Vec::new();
        {
            let mut states = lock_cancel_states(&self.cancel_states);
            states.retain(|session_id, _| live_ids.contains(session_id));
            for (session_id, internal_session_id, cancel) in live {
                let state = states
                    .entry(session_id)
                    .or_insert_with(|| {
                        Arc::new(AcpCancelState::new(
                            Some(internal_session_id.clone()),
                            cancel,
                        ))
                    })
                    .clone();
                if let Some(session_id) = state.attach(internal_session_id) {
                    cleanup_sessions.push(session_id);
                }
            }
        }
        for session_id in cleanup_sessions {
            spawn_acp_stop_cleanup(self.session_db.clone(), session_id);
        }
    }

    fn cancel_state(&self, session_id: &str) -> Option<Arc<AcpCancelState>> {
        lock_cancel_states(&self.cancel_states)
            .get(session_id)
            .cloned()
    }

    /// Main loop: read messages from stdin, dispatch, respond
    pub fn run(&mut self) -> Result<()> {
        install_acp_permission_forwarder();
        // Prompt handling is intentionally synchronous, but cancellation is a
        // notification on the same stdin stream. A dedicated reader must arm
        // and flip the turn token before queueing messages, otherwise the main
        // loop cannot even observe `session/cancel` until the prompt finishes.
        // Ordinary JSON-RPC work is bounded so a client cannot buffer an
        // unbounded number of requests behind one synchronous model prompt.
        // `session/cancel` never enters this queue: the reader handles it on
        // the control path below, so backpressure cannot starve Stop.
        let (inbound_tx, inbound_rx) = mpsc::sync_channel(ACP_INBOUND_QUEUE_CAPACITY);
        let inbound_overloaded = Arc::new(AtomicBool::new(false));
        let overloaded_for_reader = inbound_overloaded.clone();
        let cancel_states = self.cancel_states.clone();
        let db = self.session_db.clone();
        std::thread::Builder::new()
            .name("ha-acp-input".to_string())
            .spawn(move || {
                let stdin = std::io::stdin();
                let mut reader = std::io::BufReader::new(stdin);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            let _ = inbound_tx.try_send(AcpInbound::Eof);
                            return;
                        }
                        Ok(_) if line.trim().is_empty() => continue,
                        Ok(_) => match serde_json::from_str::<JsonRpcMessage>(line.trim()) {
                            Ok(msg) => {
                                if handle_acp_permission_response(&msg) {
                                    continue;
                                }
                                if msg.method.as_deref() == Some("session/cancel") {
                                    if let Some(session_id) =
                                        observe_acp_control_message(&msg, &cancel_states)
                                    {
                                        spawn_acp_stop_cleanup(db.clone(), session_id);
                                    }
                                    // Cancellation is a notification and has
                                    // no response; its complete behavior ran on
                                    // the out-of-band path above.
                                    continue;
                                }
                                if overloaded_for_reader.load(Ordering::Acquire) {
                                    continue;
                                }
                                // Arm the prompt before publishing it. The
                                // dispatcher may receive and finish a fast
                                // prompt before `try_send` returns, so doing
                                // this after the send can revive a completed
                                // generation. A failed send drops the guard
                                // and rolls back only the generation armed by
                                // this enqueue attempt.
                                let prompt_enqueue =
                                    match AcpPromptEnqueueGuard::prepare(&msg, &cancel_states) {
                                        AcpPromptEnqueueDecision::NotPrompt => None,
                                        AcpPromptEnqueueDecision::Prepared(guard) => Some(guard),
                                        AcpPromptEnqueueDecision::RejectActive => {
                                            let response = JsonRpcResponse::error(
                                                msg.id.clone().unwrap_or(Value::Null),
                                                ERROR_INVALID_REQUEST,
                                                "A prompt is already active",
                                            );
                                            match inbound_tx
                                                .try_send(AcpInbound::Response(response))
                                            {
                                                Ok(()) => {}
                                                Err(mpsc::TrySendError::Full(_)) => {
                                                    overloaded_for_reader
                                                        .store(true, Ordering::Release);
                                                }
                                                Err(mpsc::TrySendError::Disconnected(_)) => return,
                                            }
                                            continue;
                                        }
                                    };
                                match inbound_tx.try_send(AcpInbound::Message(msg)) {
                                    Ok(()) => {
                                        if let Some(prompt_enqueue) = prompt_enqueue {
                                            prompt_enqueue.commit();
                                        }
                                    }
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        overloaded_for_reader.store(true, Ordering::Release);
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => return,
                                }
                            }
                            Err(error) => {
                                if overloaded_for_reader.load(Ordering::Acquire) {
                                    continue;
                                }
                                match inbound_tx.try_send(AcpInbound::ParseError(error.to_string()))
                                {
                                    Ok(()) => {}
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        overloaded_for_reader.store(true, Ordering::Release);
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => return,
                                }
                            }
                        },
                        Err(error) => {
                            let _ = inbound_tx.try_send(AcpInbound::IoError(error.to_string()));
                            return;
                        }
                    }
                }
            })?;

        loop {
            if inbound_overloaded.load(Ordering::Acquire) {
                anyhow::bail!(
                    "ACP inbound queue exceeded {} messages; closing connection",
                    ACP_INBOUND_QUEUE_CAPACITY
                );
            }
            let msg = match inbound_rx.recv() {
                Ok(AcpInbound::Message(message)) => message,
                Ok(AcpInbound::Response(response)) => {
                    self.transport.write_response(&response)?;
                    continue;
                }
                Ok(AcpInbound::ParseError(error)) => {
                    let response = JsonRpcResponse::error(
                        Value::Null,
                        ERROR_PARSE,
                        format!("Parse error: {error}"),
                    );
                    self.transport.write_response(&response)?;
                    continue;
                }
                Ok(AcpInbound::IoError(error)) => anyhow::bail!("ACP stdin read failed: {error}"),
                Ok(AcpInbound::Eof) | Err(_) => {
                    if self.verbose {
                        eprintln!("[acp] stdin closed, shutting down");
                    }
                    return Ok(());
                }
            };

            if self.verbose {
                eprintln!("[acp] <- method={:?} id={:?}", msg.method, msg.id);
            }

            let is_notification = msg.id.is_none();
            let method = msg.method.clone().unwrap_or_default();
            let params = msg.params.clone().unwrap_or(Value::Null);

            if is_notification {
                self.handle_notification(&method, &params);
            } else {
                let id = msg.id.clone().unwrap_or(Value::Null);
                let response = self.handle_request(&method, &params, &id);
                self.transport.write_response(&response)?;
            }
        }
    }

    fn handle_notification(&mut self, method: &str, params: &Value) {
        match method {
            "session/cancel" => {
                if let Ok(cancel) = serde_json::from_value::<CancelNotification>(params.clone()) {
                    self.handle_cancel(&cancel);
                }
            }
            _ => {
                if self.verbose {
                    eprintln!("[acp] ignoring unknown notification: {}", method);
                }
            }
        }
    }

    fn handle_request(&mut self, method: &str, params: &Value, id: &Value) -> JsonRpcResponse {
        if !self.initialized && method != "initialize" {
            return JsonRpcResponse::error(
                id.clone(),
                ERROR_INVALID_REQUEST,
                "Server not initialized. Call 'initialize' first.",
            );
        }

        match method {
            "initialize" => self.do_initialize(params, id),
            "session/new" => self.do_new_session(params, id),
            "session/load" => self.do_load_session(params, id),
            "session/resume" => self.do_resume_session(params, id),
            "session/prompt" => self.do_prompt(params, id),
            "session/setMode" => self.do_set_session_mode(params, id),
            "session/setConfigOption" => self.do_set_config_option(params, id),
            "session/list" => self.do_list_sessions(params, id),
            "session/close" => self.do_close_session(params, id),
            "authenticate" => self.do_authenticate(id),
            _ => JsonRpcResponse::error(
                id.clone(),
                ERROR_METHOD_NOT_FOUND,
                format!("Method not found: {}", method),
            ),
        }
    }

    // ── initialize ──────────────────────────────────────────────

    fn do_initialize(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: InitializeRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        if let Some(caps) = req.client_capabilities {
            self.sessions.set_client_capabilities(caps);
        }

        self.protocol_version = AcpProtocolVersion::negotiated(&req.protocol_version);
        self.initialized = true;
        ha_core::permission::approval_surface::set_acp_permission_capable(
            self.protocol_version.is_v1(),
        );

        let response = InitializeResponse {
            protocol_version: self.protocol_version.clone(),
            agent_capabilities: AgentCapabilities {
                load_session: true,
                prompt_capabilities: PromptCapabilities {
                    image: true,
                    audio: false,
                    embedded_context: true,
                },
                // Hope currently does not connect the per-session mcpServers
                // supplied by an ACP client, so it must not advertise either
                // transport until that execution path exists.
                mcp_capabilities: None,
                session_capabilities: Some(SessionCapabilities {
                    list: Some(Value::Object(serde_json::Map::new())),
                    resume: Some(Value::Object(serde_json::Map::new())),
                    close: Some(Value::Object(serde_json::Map::new())),
                }),
            },
            agent_info: AgentInfo {
                name: "hope-agent-acp".to_string(),
                title: "Hope Agent ACP Agent".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            auth_methods: vec![],
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── authenticate ────────────────────────────────────────────

    fn do_authenticate(&self, id: &Value) -> JsonRpcResponse {
        JsonRpcResponse::success(
            id.clone(),
            serde_json::to_value(&AuthenticateResponse {}).unwrap(),
        )
    }

    // ── session/new ─────────────────────────────────────────────

    fn do_new_session(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: NewSessionRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let meta = parse_session_meta(&req.meta);
        let agent_id = meta
            .agent_id
            .unwrap_or_else(|| self.default_agent_id.clone());

        // ACP does not use the shared chat engine. Reserve the Agent before
        // creating the durable session so deletion cannot slip between
        // validation and session construction.
        let _agent_admission = match ha_core::agent_lifecycle::begin_agent_run(&agent_id) {
            Ok(guard) => guard,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let session_meta = match self.session_db.create_session(&agent_id) {
            Ok(m) => m,
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        let acp_session_id = session_meta.id.clone();
        persist_acp_ide_context(&self.session_db, &acp_session_id, &req.meta);

        let agent = match self.build_agent(&agent_id, &acp_session_id) {
            Ok(a) => a,
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        let acp_session = AcpSession {
            session_id: acp_session_id.clone(),
            internal_session_id: session_meta.id.clone(),
            agent_id: agent_id.clone(),
            cwd: req.cwd.clone(),
            agent,
            cancel: Arc::new(AtomicBool::new(false)),
            active_prompt: false,
            created_at: now_epoch_secs(),
            last_activity_at: now_epoch_secs(),
        };

        if let Err(e) = self.sessions.insert(acp_session) {
            return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string());
        }
        self.reconcile_cancel_states();

        let modes = self.build_modes(&agent_id);
        let config_options = self.build_config_options(&agent_id);

        let response = NewSessionResponse {
            session_id: acp_session_id,
            config_options: Some(config_options),
            modes: Some(modes),
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── session/load ────────────────────────────────────────────

    fn do_load_session(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: LoadSessionRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let session_meta = match self.session_db.get_session(&req.session_id) {
            Ok(Some(m)) => m,
            Ok(None) => {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_PARAMS,
                    "Session not found",
                )
            }
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        let agent_id = session_meta.agent_id.clone();
        persist_acp_ide_context(&self.session_db, &req.session_id, &req.meta);

        let agent = match self.build_agent(&agent_id, &req.session_id) {
            Ok(a) => a,
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        // Restore conversation context
        restore_agent_context(&self.session_db, &req.session_id, &agent);

        let acp_session = AcpSession {
            session_id: req.session_id.clone(),
            internal_session_id: req.session_id.clone(),
            agent_id: agent_id.clone(),
            cwd: req.cwd.clone(),
            agent,
            cancel: Arc::new(AtomicBool::new(false)),
            active_prompt: false,
            created_at: now_epoch_secs(),
            last_activity_at: now_epoch_secs(),
        };

        if let Err(e) = self.sessions.insert(acp_session) {
            return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string());
        }
        self.reconcile_cancel_states();

        // Replay session history
        self.replay_session_history(&req.session_id);

        let modes = self.build_modes(&agent_id);
        let config_options = self.build_config_options(&agent_id);

        let response = LoadSessionResponse {
            config_options: Some(config_options),
            modes: Some(modes),
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── session/resume ──────────────────────────────────────────

    fn do_resume_session(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: ResumeSessionRequest = match serde_json::from_value(params.clone()) {
            Ok(request) => request,
            Err(error) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, error.to_string())
            }
        };
        let session_meta = match self.session_db.get_session(&req.session_id) {
            Ok(Some(meta)) => meta,
            Ok(None) => {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_PARAMS,
                    "Session not found",
                )
            }
            Err(error) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, error.to_string())
            }
        };
        let agent_id = session_meta.agent_id.clone();
        persist_acp_ide_context(&self.session_db, &req.session_id, &req.meta);
        let agent = match self.build_agent(&agent_id, &req.session_id) {
            Ok(agent) => agent,
            Err(error) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, error.to_string())
            }
        };
        restore_agent_context(&self.session_db, &req.session_id, &agent);
        let acp_session = AcpSession {
            session_id: req.session_id.clone(),
            internal_session_id: req.session_id.clone(),
            agent_id: agent_id.clone(),
            cwd: req.cwd,
            agent,
            cancel: Arc::new(AtomicBool::new(false)),
            active_prompt: false,
            created_at: now_epoch_secs(),
            last_activity_at: now_epoch_secs(),
        };
        if let Err(error) = self.sessions.insert(acp_session) {
            return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, error.to_string());
        }
        self.reconcile_cancel_states();
        let response = ResumeSessionResponse {
            config_options: Some(self.build_config_options(&agent_id)),
            modes: Some(self.build_modes(&agent_id)),
        };
        JsonRpcResponse::success(id.clone(), serde_json::to_value(response).unwrap())
    }

    // ── session/prompt ──────────────────────────────────────────

    fn do_prompt(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: PromptRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                if let Some(session_id) = params.get("sessionId").and_then(Value::as_str) {
                    if let Some(state) = self.cancel_state(session_id) {
                        state.finish_prompt();
                    }
                }
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string());
            }
        };

        let session_id = req.session_id.clone();

        let (internal_session_id, previous_cancel) = match self.sessions.get(&session_id) {
            Some(session) if session.active_prompt => {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_REQUEST,
                    "A prompt is already active",
                )
            }
            Some(session) => (session.internal_session_id.clone(), session.cancel.clone()),
            None => {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_PARAMS,
                    "Session not found",
                )
            }
        };
        if acp_prompt_blocked_by_stop_cleanup(&internal_session_id) {
            // The stdin reader armed this generation before enqueueing it.
            // Disarm it here so a prompt rejected by a late runtime snapshot
            // cannot make the next legitimate prompt look concurrently active.
            if let Some(state) = self.cancel_state(&session_id) {
                state.finish_prompt();
            }
            app_info!(
                "acp",
                "stop_gate",
                "Rejected replacement ACP prompt while Stop cleanup is active: session={}",
                internal_session_id
            );
            let response = PromptResponse {
                stop_reason: "cancelled".to_string(),
            };
            return JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap());
        }
        let cancel_state = {
            let mut states = lock_cancel_states(&self.cancel_states);
            states
                .entry(session_id.clone())
                .or_insert_with(|| {
                    Arc::new(AcpCancelState::new(
                        Some(internal_session_id),
                        previous_cancel,
                    ))
                })
                .clone()
        };
        let turn_cancel = cancel_state.prepare_prompt();
        let _prompt_state_guard = AcpPromptStateGuard {
            state: cancel_state.clone(),
        };

        {
            let session = self
                .sessions
                .get_mut(&session_id)
                .expect("ACP session was validated before prompt setup");
            session.active_prompt = true;
            session.cancel = turn_cancel.clone();
        }
        self.sessions.touch(&session_id);
        persist_acp_ide_context(&self.session_db, &session_id, &req.meta);

        // Extract text and images
        let text = match extract_text_from_prompt(&req.prompt) {
            Ok(t) => t,
            Err(e) => {
                if let Some(s) = self.sessions.get_mut(&session_id) {
                    s.active_prompt = false;
                }
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string());
            }
        };
        let attachments = extract_images_from_prompt(&req.prompt);

        // Preflight chokepoint: pass-through in Phase 0.1; PR 1.2 runs the
        // `UserPromptSubmit` hook here. `do_prompt` is synchronous, so bridge to
        // the async helper on a short-lived runtime — the same pattern
        // `run_agent_chat` uses below.
        //
        // ACP registers no `active_turn` entry (that would pull in the cancel
        // walks / crash-flush / stream-acceptance semantics the `chat:*` bus
        // owns), so this id is minted purely to give `UserPromptSubmit` a
        // `prompt_id`. ACP's *in-turn* hooks still report `None` — closing that
        // needs ACP to hold a real active turn, tracked as a follow-up in
        // docs/architecture/agent/hooks.md §2.4.
        let turn_id = uuid::Uuid::new_v4().to_string();
        let effective_prompt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => match rt.block_on(
                ha_core::agent::preflight::user_prompt_preflight_cancellable(
                    ha_core::agent::preflight::PreflightArgs {
                        session_id: &session_id,
                        agent_id: None,
                        raw_prompt: &text,
                        turn_id: &turn_id,
                    },
                    turn_cancel.as_ref(),
                ),
            ) {
                Some(ha_core::agent::preflight::PreflightOutcome::Proceed { effective_prompt }) => {
                    effective_prompt
                }
                Some(ha_core::agent::preflight::PreflightOutcome::Block { reason }) => {
                    // A UserPromptSubmit hook blocked the prompt: record a
                    // UI-only event marker (excluded from LLM context), surface
                    // the reason as an agent message, and return without
                    // running a turn.
                    let notice = format!("🚫 {reason}");
                    let _ = self
                        .session_db
                        .append_message(&session_id, &session::NewMessage::event(&notice));
                    let update = serde_json::json!({
                        "sessionId": session_id,
                        "sessionUpdate": {
                            "sessionUpdate": "agent_message_chunk",
                            "messageId": format!("msg-agent-{turn_id}"),
                            "content": { "type": "text", "text": notice }
                        },
                        "final": true,
                    });
                    let _ = self
                        .transport
                        .write_notification(&JsonRpcNotification::new("session/update", update));
                    if let Some(s) = self.sessions.get_mut(&session_id) {
                        s.active_prompt = false;
                    }
                    let response = PromptResponse {
                        stop_reason: "refusal".to_string(),
                    };
                    return JsonRpcResponse::success(
                        id.clone(),
                        serde_json::to_value(&response).unwrap(),
                    );
                }
                None => {
                    if let Some(s) = self.sessions.get_mut(&session_id) {
                        s.active_prompt = false;
                    }
                    let response = PromptResponse {
                        stop_reason: "cancelled".to_string(),
                    };
                    return JsonRpcResponse::success(
                        id.clone(),
                        serde_json::to_value(&response).unwrap(),
                    );
                }
            },
            Err(_) if turn_cancel.load(Ordering::Acquire) => {
                if let Some(s) = self.sessions.get_mut(&session_id) {
                    s.active_prompt = false;
                }
                let response = PromptResponse {
                    stop_reason: "cancelled".to_string(),
                };
                return JsonRpcResponse::success(
                    id.clone(),
                    serde_json::to_value(&response).unwrap(),
                );
            }
            Err(_) => text.clone(),
        };

        if !cancel_state.claim_non_cancelled_persistence() {
            ha_core::hooks::set_user_prompt_context(&session_id, None);
            if let Some(s) = self.sessions.get_mut(&session_id) {
                s.active_prompt = false;
            }
            let response = PromptResponse {
                stop_reason: "cancelled".to_string(),
            };
            return JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap());
        }

        // A model turn must never start without its triggering user message
        // being durable. Otherwise a successful assistant commit could leave
        // an irrecoverable one-sided transcript.
        if let Err(error) = self.session_db.append_message(
            &session_id,
            &session::NewMessage::user(&effective_prompt)
                .with_source(ha_core::chat_engine::ChatSource::Acp),
        ) {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                session.active_prompt = false;
            }
            return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, error.to_string());
        }

        // Auto-generate fallback title
        if let Ok(Some(title)) = session::ensure_first_message_title(
            &self.session_db,
            &session_id,
            &effective_prompt,
            None,
        ) {
            // Emit session_info_update
            let notif = serde_json::json!({
                "sessionId": session_id,
                "sessionUpdate": {
                    "sessionUpdate": "session_info_update",
                    "title": title,
                    "updatedAt": chrono::Utc::now().to_rfc3339(),
                }
            });
            let _ = self
                .transport
                .write_notification(&JsonRpcNotification::new("session/update", notif));
        }

        // Run agent chat
        // Run the turn with the preflight-resolved prompt (same value the
        // other three entry points feed their engine), not the raw `text`, so
        // a future hook rewrite is honored consistently across all entries.
        let stop_reason = self.run_agent_chat(&session_id, &effective_prompt, &attachments);

        // Mark done
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.active_prompt = false;
        }

        let response = PromptResponse {
            stop_reason: stop_reason.unwrap_or_else(|e| {
                eprintln!("[acp] prompt error: {}", e);
                "error".to_string()
            }),
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── session/cancel ──────────────────────────────────────────

    fn handle_cancel(&mut self, cancel: &CancelNotification) {
        if let Some(session_id) = self
            .cancel_state(&cancel.session_id)
            .and_then(|state| state.request_cancel())
        {
            spawn_acp_stop_cleanup(self.session_db.clone(), session_id);
        }
    }

    // ── session/list ────────────────────────────────────────────

    fn do_list_sessions(&self, params: &Value, id: &Value) -> JsonRpcResponse {
        let _req: ListSessionsRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let sessions = match self.session_db.list_sessions(None) {
            Ok(s) => s,
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        let summaries: Vec<SessionSummary> = sessions
            .into_iter()
            .filter(|s| !s.is_cron && s.parent_session_id.is_none())
            .take(100)
            .map(|s| SessionSummary {
                session_id: s.id,
                cwd: None,
                title: s.title,
                updated_at: Some(s.updated_at),
            })
            .collect();

        let response = ListSessionsResponse {
            sessions: summaries,
            next_cursor: None,
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── session/setMode ─────────────────────────────────────────

    fn do_set_session_mode(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: SetSessionModeRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let session = match self.sessions.get_mut(&req.session_id) {
            Some(s) => s,
            None => {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_PARAMS,
                    "Session not found",
                )
            }
        };

        if let Some(mode_id) = &req.mode_id {
            let session_id = session.session_id.clone();
            let new_agent = match self.build_agent(mode_id, &session_id) {
                Ok(a) => a,
                Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
            };
            // Re-borrow after build_agent
            if let Some(session) = self.sessions.get_mut(&req.session_id) {
                session.agent = new_agent;
                session.agent_id = mode_id.clone();
            }
        }

        JsonRpcResponse::success(
            id.clone(),
            serde_json::to_value(&SetSessionModeResponse {}).unwrap(),
        )
    }

    // ── session/setConfigOption ─────────────────────────────────

    fn do_set_config_option(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: SetSessionConfigOptionRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        if self.sessions.get(&req.session_id).is_none() {
            return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, "Session not found");
        }

        if self.verbose {
            eprintln!("[acp] set config: {}={}", req.config_id, req.value);
        }

        let agent_id = self
            .sessions
            .get(&req.session_id)
            .map(|s| s.agent_id.clone())
            .unwrap_or_else(|| self.default_agent_id.clone());

        let config_options = self.build_config_options(&agent_id);

        let response = SetSessionConfigOptionResponse {
            config_options: Some(config_options),
        };

        JsonRpcResponse::success(id.clone(), serde_json::to_value(&response).unwrap())
    }

    // ── session/close ───────────────────────────────────────────

    fn do_close_session(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: CloseSessionRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        if let Some(session_id) = self
            .cancel_state(&req.session_id)
            .and_then(|state| state.force_cancel())
        {
            spawn_acp_stop_cleanup(self.session_db.clone(), session_id);
        }
        self.sessions.remove(&req.session_id);
        self.reconcile_cancel_states();

        JsonRpcResponse::success(
            id.clone(),
            serde_json::to_value(&CloseSessionResponse {}).unwrap(),
        )
    }

    // ── Internal helpers ────────────────────────────────────────

    /// Build an AssistantAgent from provider config (mirrors cron::build_and_run_agent)
    fn build_agent(&self, agent_id: &str, session_id: &str) -> Result<AssistantAgent> {
        let _agent_admission = ha_core::agent_lifecycle::begin_agent_run(agent_id)?;
        let store = ha_core::config::cached_config();
        let agent_model_config = ha_core::agent_loader::load_agent(agent_id)
            .map(|def| def.config.model)
            .unwrap_or_default();

        let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

        let mut model_chain = Vec::new();
        if let Some(p) = primary {
            model_chain.push(p);
        }
        for fb in fallbacks {
            if !model_chain
                .iter()
                .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
            {
                model_chain.push(fb);
            }
        }

        if model_chain.is_empty() {
            return Err(anyhow::anyhow!(
                "No model configured for agent '{}'",
                agent_id
            ));
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        // Iterate the chain and pick the first model that actually constructs.
        // The session-shell agent built here only needs *some* working model;
        // run_agent_chat re-builds per-attempt at chat time.
        let mut agent = None;
        let mut last_error = String::new();
        for candidate in &model_chain {
            let Some(prov) = provider::find_provider(&store.providers, &candidate.provider_id)
            else {
                continue;
            };
            match rt.block_on(AssistantAgent::try_new_from_provider(
                prov,
                &candidate.model_id,
            )) {
                Ok(a) => {
                    agent = Some(a.with_failover_context(prov));
                    break;
                }
                Err(e) => {
                    last_error = e.to_string();
                    app_warn!(
                        "acp",
                        "build_agent",
                        "Build agent failed for {}::{}, trying next model: {}",
                        candidate.provider_id,
                        candidate.model_id,
                        last_error
                    );
                }
            }
        }
        let mut agent = agent.ok_or_else(|| {
            anyhow::anyhow!(
                "All models failed to build for agent '{}': {}",
                agent_id,
                last_error
            )
        })?;
        agent.set_agent_id(agent_id);
        agent.set_session_id(session_id);
        agent.set_compact_config(store.compact.clone());

        if let Some(model_ref) = store.compact.effective_summarization_model_ref() {
            if let Some(cp) =
                ha_core::agent::build_compaction_provider(&model_ref, &store.providers, session_id)
            {
                agent.set_compaction_provider(Some(std::sync::Arc::new(cp)));
            }
        }

        // Resolve temperature: agent > global
        let agent_temp = ha_core::agent_loader::load_agent(agent_id)
            .ok()
            .and_then(|def| def.config.model.temperature);
        agent.set_temperature(agent_temp.or(store.temperature));

        Ok(agent)
    }

    /// Run agent chat synchronously, streaming ACP events to stdout.
    fn run_agent_chat(
        &mut self,
        session_id: &str,
        text: &str,
        attachments: &[ha_core::agent::Attachment],
    ) -> Result<String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let cancel = match self.sessions.get(session_id) {
            Some(s) => s.cancel.clone(),
            None => return Err(anyhow::anyhow!("Session not found")),
        };
        let cancel_state = self.cancel_state(session_id);

        let session_id_owned = session_id.to_string();
        let db_clone = self.session_db.clone();
        let text_owned = text.to_string();
        let attachments_owned = attachments.to_vec();

        // Idle/busy tracking (R2 — §5.4 fix). ACP runs `AssistantAgent::chat`
        // directly rather than `run_chat_engine`, so it doesn't inherit the
        // engine's idle guard — create one here for the turn's duration so
        // background-job / sub-agent completion injection (which always runs
        // through `run_chat_engine`) yields to a live ACP turn instead of
        // splicing into it. Dropped at function exit, after the failover loop's
        // `rt.block_on` turns complete; the guard's `Drop` (idle notify +
        // pending-injection flush) uses `std::thread::spawn`, so dropping it
        // outside the local runtime is safe.
        let _idle_guard = ha_core::subagent::ChatSessionGuard::new(&session_id_owned);

        // Build model chain for failover
        let store = ha_core::config::cached_config();
        let agent_id = self
            .sessions
            .get(session_id)
            .map(|s| s.agent_id.clone())
            .unwrap_or_else(|| self.default_agent_id.clone());
        let _agent_run_guard = ha_core::agent_lifecycle::begin_agent_run(&agent_id)?;

        let agent_model_config = ha_core::agent_loader::load_agent(&agent_id)
            .map(|def| def.config.model)
            .unwrap_or_default();
        let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);

        let mut model_chain = Vec::new();
        if let Some(p) = primary {
            model_chain.push(p);
        }
        for fb in fallbacks {
            if !model_chain
                .iter()
                .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
            {
                model_chain.push(fb);
            }
        }

        if model_chain.is_empty() {
            return Err(anyhow::anyhow!("No model configured"));
        }

        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10_000;

        let mut last_error = String::new();
        let mut last_reason = failover::FailoverReason::Unknown;
        let mut stop_all_models = false;
        let mut cancelled = false;

        // Build CompactionProvider once, reuse across retries
        let compaction_provider: Option<
            std::sync::Arc<dyn ha_core::context_compact::CompactionProvider>,
        > = store
            .compact
            .effective_summarization_model_ref()
            .and_then(|mr| {
                ha_core::agent::build_compaction_provider(&mr, &store.providers, &session_id_owned)
                    .map(|cp| std::sync::Arc::new(cp) as _)
            });

        let durability =
            rt.block_on(ha_core::chat_engine::durability::StreamCoordinator::create(
                db_clone.clone(),
                session_id_owned.clone(),
                ha_core::chat_engine::ChatSource::Acp,
                None,
                None,
                Arc::new(AcpDurableEventSink {
                    session_id: session_id_owned.clone(),
                    message_id: format!("msg-agent-{}", uuid::Uuid::new_v4()),
                }),
                cancel.clone(),
                None,
            ))?;

        // SessionStart hook (startup/resume). ACP runs `AssistantAgent::chat`
        // directly rather than `run_chat_engine`, so the engine's SessionStart
        // embed never fires here — we invoke the shared observation helper
        // ourselves. Fired once before the failover loop (`claim_session_start`
        // only releases once); the resulting additionalContext is re-applied to
        // each rebuilt agent so it survives retries, mirroring how the engine
        // carries it in the dynamic user-data lane.
        let session_start_ctx = rt.block_on(async {
            tokio::select! {
                biased;
                _ = wait_for_acp_cancel(cancel.clone()) => None,
                context = ha_core::hooks::fire_session_start_observation(
                    &session_id_owned,
                    &agent_id,
                    model_chain
                        .first()
                        .map(|m| m.model_id.as_str())
                        .unwrap_or_default(),
                ) => Some(context),
            }
        });
        let mut session_start_ctx = match session_start_ctx {
            Some(context) => context,
            None => {
                if let Err(error) =
                    finalize_acp_user_stop(&rt, &db_clone, &durability, &session_id_owned)
                {
                    app_error!(
                        "acp",
                        "stop",
                        "failed to finalize ACP Stop during session-start hook for {}: {}",
                        session_id_owned,
                        error
                    );
                }
                return Ok("cancelled".to_string());
            }
        };
        // Fold in any UserPromptSubmit hook context the preflight chokepoint
        // stashed for this turn, so the ACP entry injects it identically to
        // `run_chat_engine`. Drained once; re-applied to each rebuilt agent
        // below alongside the SessionStart context.
        if let Some(extra) = ha_core::hooks::take_user_prompt_context(&session_id_owned) {
            session_start_ctx = Some(match session_start_ctx.take() {
                Some(e) => format!("{e}\n\n{extra}"),
                None => extra,
            });
        }

        for model_ref in &model_chain {
            if cancel.load(Ordering::Acquire) {
                cancelled = true;
                break;
            }
            let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
                Some(p) => p,
                None => continue,
            };

            let mut retry_count: u32 = 0;
            loop {
                if cancel.load(Ordering::Acquire) {
                    cancelled = true;
                    break;
                }
                let provider_shape = match &prov.api_type {
                    ha_core::provider::ApiType::Anthropic => "anthropic",
                    ha_core::provider::ApiType::OpenaiChat => "openai_chat",
                    ha_core::provider::ApiType::OpenaiResponses => "openai_responses",
                    ha_core::provider::ApiType::Codex => "codex",
                };
                rt.block_on(durability.begin_attempt(
                    Some(&model_ref.provider_id),
                    Some(&model_ref.model_id),
                    Some(provider_shape),
                ))?;
                let build_result = rt.block_on(async {
                    tokio::select! {
                        biased;
                        _ = wait_for_acp_cancel(cancel.clone()) => {
                            Err(anyhow::anyhow!("ACP prompt cancelled"))
                        }
                        result = AssistantAgent::try_new_from_provider(
                            prov,
                            &model_ref.model_id,
                        ) => result,
                    }
                });
                let mut agent = match build_result {
                    Ok(a) => a.with_failover_context(prov),
                    Err(e) => {
                        let (reason, _) = failover::classify_error_with_evidence(&e);
                        last_reason = reason;
                        last_error = e.to_string();
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        if reason.is_retryable() && retry_count < MAX_RETRIES {
                            retry_count += 1;
                            let delay = failover::retry_delay_ms(
                                retry_count - 1,
                                RETRY_BASE_MS,
                                RETRY_MAX_MS,
                            );
                            let completed_delay = rt.block_on(async {
                                tokio::select! {
                                    biased;
                                    _ = wait_for_acp_cancel(cancel.clone()) => false,
                                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => true,
                                }
                            });
                            if !completed_delay {
                                cancelled = true;
                                break;
                            }
                            continue;
                        }
                        app_warn!(
                            "acp",
                            "build_agent",
                            "Build agent failed for {}::{}, trying next model: {}",
                            model_ref.provider_id,
                            model_ref.model_id,
                            last_error
                        );
                        break;
                    }
                };
                agent.set_agent_id(&agent_id);
                agent.set_session_id(&session_id_owned);
                agent.set_turn_provenance(ha_core::tool_defs::ToolTurnProvenance::ForegroundUser);
                let (lineage_epoch, global_stop_epoch, global_stop_receipt_count) =
                    durability.stop_admission();
                agent.set_turn_stop_admission(
                    lineage_epoch,
                    global_stop_epoch,
                    global_stop_receipt_count,
                );
                agent.set_session_db(db_clone.clone());
                agent.set_turn_durability(durability.clone());
                agent.set_compact_config(store.compact.clone());
                if let Some(ref cp) = compaction_provider {
                    agent.set_compaction_provider(Some(cp.clone()));
                }

                // Restore context
                restore_agent_context(&db_clone, &session_id_owned, &agent);

                // SessionStart/UserPromptSubmit hook output survives failover,
                // but remains untrusted data rather than being promoted into
                // the ACP run instruction lane.
                if let Some(ref ctx) = session_start_ctx {
                    agent.set_run_context(
                        ha_core::prompt_context::RunInstructionContext::data_only(
                            ha_core::prompt_context::RunInstructionSource::Acp,
                            ctx.clone(),
                        )
                        .map_err(|error| anyhow::anyhow!(error))?,
                    );
                }

                let cancel_clone = cancel.clone();
                let durability_for_cb = durability.clone();

                let result = rt.block_on(async {
                    let current_user_message_state = current_user_message_state_for_retry_base(
                        durability.attempt_base_contains_current_user(),
                    );
                    let chat = agent.chat_with_user_message_state(
                        &text_owned,
                        &attachments_owned,
                        current_user_message_state,
                        None,
                        cancel_clone.clone(),
                        move |delta| {
                            if let Err(error) = durability_for_cb.accept_event(delta) {
                                app_error!(
                                    "acp",
                                    "stream_durability",
                                    "failed to accept ACP stream event: {}",
                                    error
                                );
                            }
                        },
                    );
                    tokio::pin!(chat);
                    tokio::select! {
                        biased;
                        _ = wait_for_acp_cancel(cancel_clone) => {
                            match tokio::time::timeout(
                                ACP_CANCEL_COOPERATIVE_GRACE,
                                &mut chat,
                            ).await {
                                Ok(result) => result,
                                Err(_) => Err(anyhow::anyhow!(
                                    "ACP chat cancellation grace timed out"
                                )),
                            }
                        }
                        result = &mut chat => result,
                    }
                });

                match result {
                    Ok((response, _thinking)) => {
                        if cancel.load(Ordering::Acquire) {
                            if let Err(error) = finalize_acp_user_stop(
                                &rt,
                                &db_clone,
                                &durability,
                                &session_id_owned,
                            ) {
                                app_error!(
                                    "acp",
                                    "stop",
                                    "failed to finalize stopped ACP turn for {}: {}",
                                    session_id_owned,
                                    error
                                );
                            }
                            return Ok("cancelled".to_string());
                        }
                        let final_seq = rt.block_on(durability.flush(FlushReason::FinalEnd))?;
                        rt.block_on(durability.reconcile_spool_to_sqlite())?;
                        let trailing = {
                            let text = durability.trailing_text();
                            if text.is_empty() && !durability.had_text_output() {
                                response.clone()
                            } else {
                                text
                            }
                        };
                        let mut assistant_msg = session::NewMessage::assistant(&trailing)
                            .with_source(ha_core::chat_engine::ChatSource::Acp);
                        let usage = durability.usage();
                        assistant_msg.tokens_in = usage.input_tokens;
                        assistant_msg.tokens_out = usage.output_tokens;
                        assistant_msg.tokens_in_last =
                            usage.last_context_input_tokens.or(usage.last_input_tokens);
                        assistant_msg.model = usage.model.clone();
                        assistant_msg.ttft_ms = usage.ttft_ms;
                        assistant_msg.tokens_cache_creation = usage
                            .last_cache_creation_input_tokens
                            .or(usage.cache_creation_input_tokens);
                        assistant_msg.tokens_cache_read = usage
                            .last_cache_read_input_tokens
                            .or(usage.cache_read_input_tokens);
                        let mut usage_event = ha_core::model_usage::ModelUsageEvent::new(
                            ha_core::model_usage::KIND_CHAT,
                        );
                        usage_event.input_tokens =
                            usage.input_tokens.map(|value| value.max(0) as u64);
                        usage_event.output_tokens =
                            usage.output_tokens.map(|value| value.max(0) as u64);
                        usage_event.cache_creation_input_tokens = usage
                            .cache_creation_input_tokens
                            .map(|value| value.max(0) as u64);
                        usage_event.cache_read_input_tokens = usage
                            .cache_read_input_tokens
                            .map(|value| value.max(0) as u64);
                        usage_event.context_input_tokens = usage
                            .context_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                        usage_event.fresh_input_tokens = usage
                            .fresh_input_tokens
                            .or(usage.input_tokens)
                            .map(|value| value.max(0) as u64);
                        usage_event.metadata = Some(serde_json::json!({
                            "tokenAccounting": {
                                "inputCoverage": usage.input_coverage,
                                "outputCoverage": usage.output_coverage,
                                "observations": usage.token_accounting_observations,
                            }
                        }));
                        usage_event.model_id = Some(
                            usage
                                .model
                                .clone()
                                .unwrap_or_else(|| model_ref.model_id.clone()),
                        );
                        usage_event.ttft_ms = usage.ttft_ms.map(|value| value.max(0) as u64);
                        usage_event.operation = Some("chat.acp".to_string());
                        usage_event.source = Some("acp".to_string());
                        usage_event.provider_id = Some(model_ref.provider_id.clone());
                        usage_event.provider_name = Some(prov.name.clone());
                        usage_event.session_id = Some(session_id_owned.clone());
                        usage_event.agent_id = Some(agent_id.clone());
                        let context_json =
                            serde_json::to_string(&agent.get_conversation_history())?;
                        let completion_claimed = cancel_state
                            .as_ref()
                            .map(|state| state.claim_non_cancelled_completion())
                            .unwrap_or_else(|| !cancel.load(Ordering::Acquire));
                        if !completion_claimed {
                            if let Err(error) = finalize_acp_user_stop(
                                &rt,
                                &db_clone,
                                &durability,
                                &session_id_owned,
                            ) {
                                app_error!(
                                    "acp",
                                    "stop",
                                    "failed to finalize late ACP Stop for {}: {}",
                                    session_id_owned,
                                    error
                                );
                            }
                            return Ok("cancelled".to_string());
                        }
                        let commit = ha_core::session::CommitAssistantTurn {
                            run_id: durability
                                .is_persistent()
                                .then(|| durability.persistence_run_id().to_string()),
                            attempt_no: durability.current_attempt_no(),
                            session_id: session_id_owned.clone(),
                            assistant: assistant_msg,
                            trailing_placeholder_id: None,
                            context_json,
                            expected_context_revision: durability.context_revision(),
                            turn_id: None,
                            usage: Some(usage_event),
                            final_seq,
                            tier3_recovery: ha_core::session::Tier3RecoveryCommit::Unchanged,
                            request_plan: durability.successful_request_plan_commit()?,
                        };
                        let committed = db_clone.commit_assistant_turn(&commit)?;
                        rt.block_on(durability.finalize_successful_request_after_turn_commit())?;
                        durability.mark_committed(committed.committed_seq);
                        ha_core::session_title::maybe_schedule_after_success(
                            db_clone.clone(),
                            session_id_owned.clone(),
                            agent_id.clone(),
                            model_ref.clone(),
                        );

                        return Ok("end_turn".to_string());
                    }
                    Err(e) => {
                        let (reason, _) = failover::classify_error_with_evidence(&e);
                        last_reason = reason;
                        last_error = e.to_string();
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }

                        if reason.is_terminal() {
                            stop_all_models = true;
                            break;
                        }

                        if reason.is_retryable() && retry_count < MAX_RETRIES {
                            retry_count += 1;
                            let delay = failover::retry_delay_ms(
                                retry_count - 1,
                                RETRY_BASE_MS,
                                RETRY_MAX_MS,
                            );
                            let completed_delay = rt.block_on(async {
                                tokio::select! {
                                    biased;
                                    _ = wait_for_acp_cancel(cancel.clone()) => false,
                                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => true,
                                }
                            });
                            if !completed_delay {
                                cancelled = true;
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }
            }
            if stop_all_models || cancelled {
                break;
            }
        }

        if cancelled || cancel.load(Ordering::Acquire) {
            if let Err(error) =
                finalize_acp_user_stop(&rt, &db_clone, &durability, &session_id_owned)
            {
                app_error!(
                    "acp",
                    "stop",
                    "failed to finalize stopped ACP turn for {}: {}",
                    session_id_owned,
                    error
                );
            }
            return Ok("cancelled".to_string());
        }

        // Converge provider/build failures through the same durable protocol;
        // do not leave a `running` run waiting for the next process restart.
        let final_error = if last_reason == failover::FailoverReason::DispatchUnknown {
            format!(
                "Provider dispatch outcome is unknown. Automatic retry was stopped to avoid a duplicate request. Check Provider activity before retrying manually. Last error: {}",
                last_error
            )
        } else {
            format!("All models failed. Last error: {}", last_error)
        };
        let durable_seq = rt.block_on(durability.flush(FlushReason::Failure))?;
        // Never terminalize a run while some already-emitted bytes exist only
        // in the spool. Keeping it `running` lets startup import and recover
        // that complete displayed prefix.
        rt.block_on(durability.reconcile_spool_to_sqlite())?;
        let (attempt_no, final_seq, visible_events, integrity_error, provider_kind) =
            if durability.is_persistent() {
                let snapshot = db_clone
                    .stream_run_snapshot(durability.persistence_run_id())?
                    .ok_or_else(|| anyhow::anyhow!("ACP persistence run disappeared"))?;
                let (attempt_no, final_seq, events, integrity_error) =
                    session::select_recoverable_attempt_prefix(&snapshot);
                let provider_kind = snapshot
                    .attempts
                    .iter()
                    .find(|attempt| attempt.attempt_no == attempt_no)
                    .and_then(|attempt| attempt.provider_shape.as_deref())
                    .or(snapshot.run.provider_shape.as_deref())
                    .and_then(ha_core::chat_engine::finalize::ProviderApiKind::from_shape);
                (
                    attempt_no,
                    final_seq,
                    events,
                    integrity_error,
                    provider_kind,
                )
            } else {
                let snapshot = durability.snapshot();
                (
                    durability.current_attempt_no(),
                    durable_seq,
                    snapshot.events,
                    None,
                    durability
                        .current_provider_shape()
                        .as_deref()
                        .and_then(ha_core::chat_engine::finalize::ProviderApiKind::from_shape),
                )
            };
        let trailing = session::trailing_text_from_journal_events(&visible_events);
        let (stored_context, context_checkpoint_seq, context_revision) =
            if durability.is_persistent() {
                db_clone.recovery_context_for_prefix(
                    durability.persistence_run_id(),
                    attempt_no,
                    final_seq,
                )?
            } else {
                let (context, revision) = db_clone
                    .load_context_with_revision(&session_id_owned)
                    .unwrap_or((None, durability.context_revision()));
                (context, 0, revision)
            };
        let mut history: Vec<serde_json::Value> = stored_context
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        ha_core::chat_engine::finalize::rebuild::append_journal_suffix_to_history(
            &mut history,
            &visible_events,
            context_checkpoint_seq,
            provider_kind,
        )?;
        let terminal_reason = ha_core::chat_engine::finalize::TerminationReason::ProviderFailed {
            last_kind: last_reason,
            last_message: final_error.clone(),
            is_codex_auth: false,
        };
        history.push(serde_json::json!({
            "role": "assistant",
            "content": ha_core::chat_engine::finalize::copy::model_marker(&terminal_reason),
        }));
        let context_json = serde_json::to_string(&history)?;
        let assistant = session::journal_events_have_assistant_output(&visible_events).then(|| {
            session::NewMessage::assistant(&trailing)
                .with_source(ha_core::chat_engine::ChatSource::Acp)
        });
        let commit = session::CommitInterruptedTurn {
            run_id: durability
                .is_persistent()
                .then(|| durability.persistence_run_id().to_string()),
            attempt_no,
            session_id: session_id_owned.clone(),
            assistant,
            context_json,
            expected_context_revision: context_revision,
            turn_id: None,
            final_seq,
            status: session::ChatTurnStatus::Failed,
            interrupt_reason: Some(
                terminal_reason
                    .to_chat_turn_interrupt_reason()
                    .as_str()
                    .to_string(),
            ),
            error: Some(
                integrity_error
                    .map(|integrity| format!("{final_error}; {integrity}"))
                    .unwrap_or_else(|| final_error.clone()),
            ),
            recovery_event: Some(
                session::NewMessage::error_event(&final_error)
                    .with_source(ha_core::chat_engine::ChatSource::Acp),
            ),
            request_plan: durability.interrupted_request_plan_commit(
                session::RequestPlanResponseOutcome::ResponseIncomplete,
            ),
        };
        let completion_claimed = cancel_state
            .as_ref()
            .map(|state| state.claim_non_cancelled_completion())
            .unwrap_or_else(|| !cancel.load(Ordering::Acquire));
        if !completion_claimed {
            if let Err(error) =
                finalize_acp_user_stop(&rt, &db_clone, &durability, &session_id_owned)
            {
                app_error!(
                    "acp",
                    "stop",
                    "failed to finalize ACP Stop racing terminal failure for {}: {}",
                    session_id_owned,
                    error
                );
            }
            return Ok("cancelled".to_string());
        }
        if let Err(error) = db_clone.commit_interrupted_turn(&commit) {
            app_error!(
                "acp",
                "stream_durability",
                "failed to converge terminal ACP run {}: {}",
                durability.persistence_run_id(),
                error
            );
            // Keep the DB run recoverable; its journal/spool is still the
            // durable source for every ACP chunk already emitted.
        } else if let Err(error) =
            rt.block_on(durability.finalize_interrupted_request_after_turn_commit(
                session::RequestPlanResponseOutcome::ResponseIncomplete,
            ))
        {
            app_error!(
                "acp",
                "stream_durability",
                "failed to converge incognito request plan {}: {}",
                durability.persistence_run_id(),
                error
            );
        }
        durability.mark_interrupted("failed");
        Err(anyhow::anyhow!(final_error))
    }

    /// Build session modes from available agents
    fn build_modes(&self, current_agent_id: &str) -> SessionModeState {
        let agents = ha_core::agent_loader::list_agents().unwrap_or_default();
        let modes: Vec<SessionMode> = agents
            .iter()
            .map(|a| SessionMode {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone(),
            })
            .collect();

        SessionModeState {
            current_mode_id: current_agent_id.to_string(),
            available_modes: modes,
        }
    }

    /// Build config options
    fn build_config_options(&self, _agent_id: &str) -> Vec<SessionConfigOption> {
        vec![SessionConfigOption {
            option_type: "select".to_string(),
            id: "reasoning_effort".to_string(),
            name: "Reasoning Effort".to_string(),
            category: Some("Model".to_string()),
            description: "Control how much effort the model puts into reasoning".to_string(),
            current_value: "medium".to_string(),
            options: vec![
                ConfigOptionValue {
                    value: "low".to_string(),
                    name: "Low".to_string(),
                },
                ConfigOptionValue {
                    value: "medium".to_string(),
                    name: "Medium".to_string(),
                },
                ConfigOptionValue {
                    value: "high".to_string(),
                    name: "High".to_string(),
                },
            ],
        }]
    }

    /// Replay session history as ACP notifications
    fn replay_session_history(&mut self, session_id: &str) {
        let messages = match self.session_db.load_session_messages(session_id) {
            Ok(m) => m,
            Err(_) => return,
        };

        for msg in &messages {
            let notif = match msg.role {
                session::MessageRole::User => {
                    let update = serde_json::json!({
                        "sessionId": session_id,
                        "sessionUpdate": {
                            "sessionUpdate": "user_message_chunk",
                            "messageId": format!("msg-user-{}", msg.id),
                            "content": { "type": "text", "text": msg.content }
                        },
                        "final": true,
                    });
                    Some(JsonRpcNotification::new("session/update", update))
                }
                session::MessageRole::Assistant | session::MessageRole::TextBlock => {
                    let update = serde_json::json!({
                        "sessionId": session_id,
                        "sessionUpdate": {
                            "sessionUpdate": "agent_message_chunk",
                            "messageId": format!("msg-agent-{}", msg.id),
                            "content": { "type": "text", "text": msg.content }
                        },
                        "final": true,
                    });
                    Some(JsonRpcNotification::new("session/update", update))
                }
                session::MessageRole::Tool => {
                    let tool_name = msg.tool_name.as_deref().unwrap_or("unknown");
                    let call_id = msg.tool_call_id.as_deref().unwrap_or("");
                    let status = if msg.is_error.unwrap_or(false) {
                        "failed"
                    } else {
                        "completed"
                    };

                    let start_update = serde_json::json!({
                        "sessionId": session_id,
                        "sessionUpdate": {
                            "sessionUpdate": "tool_call",
                            "toolCallId": call_id,
                            "title": tool_name,
                            "status": status,
                        },
                        "final": true,
                    });
                    let _ = self.transport.write_notification(&JsonRpcNotification::new(
                        "session/update",
                        start_update,
                    ));

                    if let Some(ref result) = msg.tool_result {
                        let truncated = if result.len() > 8192 {
                            format!("{}...(truncated)", ha_core::truncate_utf8(result, 8192))
                        } else {
                            result.clone()
                        };
                        let result_update = serde_json::json!({
                            "sessionId": session_id,
                            "sessionUpdate": {
                                "sessionUpdate": "tool_call_update",
                                "toolCallId": call_id,
                                "status": status,
                                "content": [{"type": "text", "content": {"type": "text", "text": truncated}}]
                            },
                            "final": true,
                        });
                        Some(JsonRpcNotification::new("session/update", result_update))
                    } else {
                        None
                    }
                }
                session::MessageRole::Event | session::MessageRole::ThinkingBlock => None,
            };

            if let Some(n) = notif {
                let _ = self.transport.write_notification(&n);
            }
        }
    }
}

// ── Standalone helper functions (no Tauri dependency) ────────────

/// Restore conversation history from DB into the agent
fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    if let Ok(Some(json_str)) = db.load_context(session_id) {
        if let Ok(history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            if !history.is_empty() {
                agent.set_conversation_history(history);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarized_retry_base_does_not_reappend_current_user() {
        assert_eq!(
            current_user_message_state_for_retry_base(true),
            CurrentUserMessageState::AlreadyInHistory
        );
        assert_eq!(
            current_user_message_state_for_retry_base(false),
            CurrentUserMessageState::MissingFromHistory
        );
    }

    fn inbound(method: &str, session_id: &str) -> JsonRpcMessage {
        JsonRpcMessage {
            jsonrpc: "2.0".to_string(),
            id: (method == "session/prompt").then(|| serde_json::json!(1)),
            method: Some(method.to_string()),
            params: Some(serde_json::json!({ "sessionId": session_id })),
            result: None,
            error: None,
        }
    }

    #[test]
    fn queued_acp_prompt_observes_cancel_before_dispatch() {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let initial = Arc::new(AtomicBool::new(false));
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            initial,
        ));
        lock_cancel_states(&states).insert("acp-session".to_string(), state.clone());

        assert_eq!(
            observe_acp_control_message(&inbound("session/prompt", "acp-session"), &states),
            None
        );
        let turn_cancel = state.prepare_prompt();
        assert!(!turn_cancel.load(Ordering::Acquire));
        assert_eq!(
            observe_acp_control_message(&inbound("session/cancel", "acp-session"), &states),
            Some("internal-session".to_string())
        );
        assert!(turn_cancel.load(Ordering::Acquire));
        assert_eq!(
            observe_acp_control_message(&inbound("session/cancel", "acp-session"), &states),
            None,
            "repeated Stop must not start duplicate cleanup"
        );
        assert!(!state.claim_non_cancelled_completion());
        assert!(!state.claim_non_cancelled_persistence());
    }

    #[test]
    fn acp_persistence_claim_orders_prompt_before_later_cancel() {
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            Arc::new(AtomicBool::new(false)),
        ));
        let turn_cancel = state.prepare_prompt();

        assert!(state.claim_non_cancelled_persistence());
        assert_eq!(state.request_cancel(), Some("internal-session".to_string()));
        assert!(turn_cancel.load(Ordering::Acquire));
    }

    #[test]
    fn replacement_acp_prompt_observes_deferred_stop_gate() {
        let session_id = format!("acp-deferred-stop-{}", uuid::Uuid::new_v4());
        let gate = ha_core::chat_engine::active_turn::begin_stop_cleanup(&session_id);

        assert!(acp_prompt_blocked_by_stop_cleanup(&session_id));
        drop(gate);
    }

    #[test]
    fn completed_acp_prompt_ignores_late_cancel_and_next_turn_gets_fresh_token() {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            Arc::new(AtomicBool::new(false)),
        ));
        lock_cancel_states(&states).insert("acp-session".to_string(), state.clone());

        observe_acp_control_message(&inbound("session/prompt", "acp-session"), &states);
        let first = state.prepare_prompt();
        assert!(state.claim_non_cancelled_completion());
        assert_eq!(
            observe_acp_control_message(&inbound("session/cancel", "acp-session"), &states),
            None
        );
        assert!(!first.load(Ordering::Acquire));

        observe_acp_control_message(&inbound("session/prompt", "acp-session"), &states);
        let second = state.prepare_prompt();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!second.load(Ordering::Acquire));
    }

    #[test]
    fn prompt_enqueue_commit_cannot_rearm_a_generation_completed_by_dispatch() {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            Arc::new(AtomicBool::new(false)),
        ));
        lock_cancel_states(&states).insert("acp-session".to_string(), state.clone());

        let enqueue = match AcpPromptEnqueueGuard::prepare(
            &inbound("session/prompt", "acp-session"),
            &states,
        ) {
            AcpPromptEnqueueDecision::Prepared(guard) => guard,
            _ => panic!("prompt must be armed before it is published"),
        };
        let completed = state.prepare_prompt();

        // Model the dispatcher receiving and completing the prompt before
        // try_send returns to the reader and commits its enqueue guard.
        assert!(state.claim_non_cancelled_completion());
        enqueue.commit();
        assert_eq!(
            observe_acp_control_message(&inbound("session/cancel", "acp-session"), &states),
            None,
            "a late cancel must not target the completed generation"
        );

        let next = state.prepare_prompt();
        assert!(!Arc::ptr_eq(&completed, &next));
        assert!(!next.load(Ordering::Acquire));
    }

    #[test]
    fn failed_prompt_enqueue_rolls_back_only_its_new_generation() {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            Arc::new(AtomicBool::new(false)),
        ));
        lock_cancel_states(&states).insert("acp-session".to_string(), state.clone());

        let failed_enqueue = match AcpPromptEnqueueGuard::prepare(
            &inbound("session/prompt", "acp-session"),
            &states,
        ) {
            AcpPromptEnqueueDecision::Prepared(guard) => guard,
            _ => panic!("prompt must be armed before enqueue"),
        };
        let rejected = state.prepare_prompt();
        drop(failed_enqueue);
        assert_eq!(
            observe_acp_control_message(&inbound("session/cancel", "acp-session"), &states),
            None,
            "a prompt rejected by the inbound queue must not remain cancellable"
        );

        let next = state.prepare_prompt();
        assert!(!Arc::ptr_eq(&rejected, &next));
        assert!(!next.load(Ordering::Acquire));
    }

    #[test]
    fn concurrent_prompt_is_rejected_before_inbound_queueing() {
        let states = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(AcpCancelState::new(
            Some("internal-session".to_string()),
            Arc::new(AtomicBool::new(false)),
        ));
        lock_cancel_states(&states).insert("acp-session".to_string(), state.clone());

        let first = match AcpPromptEnqueueGuard::prepare(
            &inbound("session/prompt", "acp-session"),
            &states,
        ) {
            AcpPromptEnqueueDecision::Prepared(guard) => guard,
            _ => panic!("first prompt must be accepted"),
        };
        first.commit();

        assert!(matches!(
            AcpPromptEnqueueGuard::prepare(&inbound("session/prompt", "acp-session"), &states),
            AcpPromptEnqueueDecision::RejectActive
        ));
        let first_cancel = state.prepare_prompt();
        assert_eq!(state.request_cancel(), Some("internal-session".to_string()));
        assert!(first_cancel.load(Ordering::Acquire));

        state.finish_prompt();
        assert!(matches!(
            AcpPromptEnqueueGuard::prepare(&inbound("session/prompt", "acp-session"), &states),
            AcpPromptEnqueueDecision::Prepared(_)
        ));
    }
}

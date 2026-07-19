//! Core ACP Agent implementation.
//!
//! Implements all ACP Agent interface methods by translating between
//! ACP protocol and the existing Hope Agent AssistantAgent + SessionDB.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::acp::event_mapper;
use crate::acp::protocol::NdJsonTransport;
use crate::acp::session::{now_epoch_secs, AcpSession, AcpSessionStore};
use crate::acp::types::*;
use crate::agent::AssistantAgent;
use crate::chat_engine::EventSink;
use crate::failover;
use crate::provider;
use crate::session::{self, SessionDB, SessionIdeContext};
use crate::turn_durability::{FlushReason, TurnDurabilitySink};

/// ACP protocol version we advertise
const ACP_PROTOCOL_VERSION: &str = "0.2";

/// ACP's live transport is fed by the coordinator's post-durability output
/// hook. Writing from the model callback would expose bytes before the journal
/// or emergency spool had acknowledged them.
struct AcpDurableEventSink {
    session_id: String,
}

impl EventSink for AcpDurableEventSink {
    fn send(&self, event: &str) {
        let Some(notification) = event_mapper::map_agent_event(&self.session_id, event) else {
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
    pub verbose: bool,
    /// Default agent ID (from CLI flag)
    pub default_agent_id: String,
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
                crate::app_warn!(
                    "acp",
                    "ide_context",
                    "failed to persist ACP IDE context for session {}: {}",
                    session_id,
                    err
                );
            }
        }
        Err(err) => {
            crate::app_warn!(
                "acp",
                "ide_context",
                "invalid ACP ideContext for session {}: {}",
                session_id,
                err
            );
        }
    }
}

impl AcpAgent {
    pub fn new(session_db: Arc<SessionDB>, default_agent_id: String, verbose: bool) -> Self {
        Self {
            transport: NdJsonTransport::new(),
            sessions: AcpSessionStore::new(32),
            session_db,
            initialized: false,
            verbose,
            default_agent_id,
        }
    }

    /// Main loop: read messages from stdin, dispatch, respond
    pub fn run(&mut self) -> Result<()> {
        loop {
            let msg = match self.transport.read_message()? {
                Some(m) => m,
                None => {
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

        self.initialized = true;

        // Epic D / DEADLOCK-1: ACP has no outbound `session/request_permission`
        // forwarding yet, so a tool that needs approval can't reach the editor.
        // The unattended-surface check (permission::approval_surface) fail-closes
        // those to a clear deny instead of hanging the prompt forever — surface
        // it up front so the operator knows interactive approvals won't appear in
        // the editor and can switch that agent to YOLO / auto-approve if it needs
        // to edit. (`set_acp_permission_capable` stays false until real
        // forwarding lands.)
        app_warn!(
            "acp",
            "initialize",
            "ACP mode has no approval-forwarding channel; tools that need approval are auto-denied (fail-closed). Use YOLO / per-agent auto-approve for unattended ACP editing."
        );

        let response = InitializeResponse {
            protocol_version: ACP_PROTOCOL_VERSION.to_string(),
            agent_capabilities: AgentCapabilities {
                load_session: true,
                prompt_capabilities: PromptCapabilities {
                    image: true,
                    audio: false,
                    embedded_context: true,
                },
                session_capabilities: Some(SessionCapabilities {
                    list: Value::Object(serde_json::Map::new()),
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
        let _agent_admission = match crate::agent_lifecycle::begin_agent_run(&agent_id) {
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

    // ── session/prompt ──────────────────────────────────────────

    fn do_prompt(&mut self, params: &Value, id: &Value) -> JsonRpcResponse {
        let req: PromptRequest = match serde_json::from_value(params.clone()) {
            Ok(r) => r,
            Err(e) => {
                return JsonRpcResponse::error(id.clone(), ERROR_INVALID_PARAMS, e.to_string())
            }
        };

        let session_id = req.session_id.clone();

        // Validate session
        {
            let session = match self.sessions.get_mut(&session_id) {
                Some(s) => s,
                None => {
                    return JsonRpcResponse::error(
                        id.clone(),
                        ERROR_INVALID_PARAMS,
                        "Session not found",
                    )
                }
            };
            if session.active_prompt {
                return JsonRpcResponse::error(
                    id.clone(),
                    ERROR_INVALID_REQUEST,
                    "A prompt is already active",
                );
            }
            session.active_prompt = true;
            session.cancel.store(false, Ordering::SeqCst);
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
        let effective_prompt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => match rt.block_on(crate::agent::preflight::user_prompt_preflight(
                crate::agent::preflight::PreflightArgs {
                    session_id: &session_id,
                    agent_id: None,
                    raw_prompt: &text,
                },
            )) {
                crate::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
                    effective_prompt
                }
                crate::agent::preflight::PreflightOutcome::Block { reason } => {
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
            },
            Err(_) => text.clone(),
        };

        // A model turn must never start without its triggering user message
        // being durable. Otherwise a successful assistant commit could leave
        // an irrecoverable one-sided transcript.
        if let Err(error) = self.session_db.append_message(
            &session_id,
            &session::NewMessage::user(&effective_prompt)
                .with_source(crate::chat_engine::ChatSource::Acp),
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
        if let Some(session) = self.sessions.get_mut(&cancel.session_id) {
            session.cancel.store(true, Ordering::SeqCst);
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

        if let Some(session) = self.sessions.get(&req.session_id) {
            session.cancel.store(true, Ordering::SeqCst);
        }
        self.sessions.remove(&req.session_id);

        JsonRpcResponse::success(
            id.clone(),
            serde_json::to_value(&CloseSessionResponse {}).unwrap(),
        )
    }

    // ── Internal helpers ────────────────────────────────────────

    /// Build an AssistantAgent from provider config (mirrors cron::build_and_run_agent)
    fn build_agent(&self, agent_id: &str, session_id: &str) -> Result<AssistantAgent> {
        let _agent_admission = crate::agent_lifecycle::begin_agent_run(agent_id)?;
        let store = crate::config::cached_config();
        let agent_model_config = crate::agent_loader::load_agent(agent_id)
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
                crate::agent::build_compaction_provider(&model_ref, &store.providers, session_id)
            {
                agent.set_compaction_provider(Some(std::sync::Arc::new(cp)));
            }
        }

        // Resolve temperature: agent > global
        let agent_temp = crate::agent_loader::load_agent(agent_id)
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
        attachments: &[crate::agent::Attachment],
    ) -> Result<String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let cancel = match self.sessions.get(session_id) {
            Some(s) => s.cancel.clone(),
            None => return Err(anyhow::anyhow!("Session not found")),
        };

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
        let _idle_guard = crate::subagent::ChatSessionGuard::new(&session_id_owned);

        // Build model chain for failover
        let store = crate::config::cached_config();
        let agent_id = self
            .sessions
            .get(session_id)
            .map(|s| s.agent_id.clone())
            .unwrap_or_else(|| self.default_agent_id.clone());
        let _agent_run_guard = crate::agent_lifecycle::begin_agent_run(&agent_id)?;

        let agent_model_config = crate::agent_loader::load_agent(&agent_id)
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
        let mut stop_all_models = false;

        // Build CompactionProvider once, reuse across retries
        let compaction_provider: Option<
            std::sync::Arc<dyn crate::context_compact::CompactionProvider>,
        > = store
            .compact
            .effective_summarization_model_ref()
            .and_then(|mr| {
                crate::agent::build_compaction_provider(&mr, &store.providers, &session_id_owned)
                    .map(|cp| std::sync::Arc::new(cp) as _)
            });

        // SessionStart hook (startup/resume). ACP runs `AssistantAgent::chat`
        // directly rather than `run_chat_engine`, so the engine's SessionStart
        // embed never fires here — we invoke the shared observation helper
        // ourselves. Fired once before the failover loop (`claim_session_start`
        // only releases once); the resulting additionalContext is re-applied to
        // each rebuilt agent so it survives retries, mirroring how the engine
        // threads it through `extra_system_context`.
        let mut session_start_ctx = rt.block_on(crate::hooks::fire_session_start_observation(
            &session_id_owned,
            &agent_id,
            model_chain
                .first()
                .map(|m| m.model_id.as_str())
                .unwrap_or_default(),
        ));
        // Fold in any UserPromptSubmit hook context the preflight chokepoint
        // stashed for this turn, so the ACP entry injects it identically to
        // `run_chat_engine`. Drained once; re-applied to each rebuilt agent
        // below alongside the SessionStart context.
        if let Some(extra) = crate::hooks::take_user_prompt_context(&session_id_owned) {
            session_start_ctx = Some(match session_start_ctx.take() {
                Some(e) => format!("{e}\n\n{extra}"),
                None => extra,
            });
        }

        let durability = rt.block_on(crate::chat_engine::durability::StreamCoordinator::create(
            db_clone.clone(),
            session_id_owned.clone(),
            crate::chat_engine::ChatSource::Acp,
            None,
            None,
            Arc::new(AcpDurableEventSink {
                session_id: session_id_owned.clone(),
            }),
            cancel.clone(),
        ))?;

        for model_ref in &model_chain {
            let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
                Some(p) => p,
                None => continue,
            };

            let mut retry_count: u32 = 0;
            loop {
                let provider_shape = match &prov.api_type {
                    crate::provider::ApiType::Anthropic => "anthropic",
                    crate::provider::ApiType::OpenaiChat => "openai_chat",
                    crate::provider::ApiType::OpenaiResponses => "openai_responses",
                    crate::provider::ApiType::Codex => "codex",
                };
                rt.block_on(durability.begin_attempt(
                    Some(&model_ref.provider_id),
                    Some(&model_ref.model_id),
                    Some(provider_shape),
                ))?;
                let build_result = rt.block_on(AssistantAgent::try_new_from_provider(
                    prov,
                    &model_ref.model_id,
                ));
                let mut agent = match build_result {
                    Ok(a) => a.with_failover_context(prov),
                    Err(e) => {
                        last_error = e.to_string();
                        let reason = failover::classify_error(&last_error);
                        if reason.is_retryable() && retry_count < MAX_RETRIES {
                            retry_count += 1;
                            let delay = failover::retry_delay_ms(
                                retry_count - 1,
                                RETRY_BASE_MS,
                                RETRY_MAX_MS,
                            );
                            rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(
                                delay,
                            )));
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
                agent.set_session_db(db_clone.clone());
                agent.set_turn_durability(durability.clone());
                agent.set_compact_config(store.compact.clone());
                if let Some(ref cp) = compaction_provider {
                    agent.set_compaction_provider(Some(cp.clone()));
                }

                // Restore context
                restore_agent_context(&db_clone, &session_id_owned, &agent);

                // Fold any SessionStart additionalContext into this turn's
                // system prompt. Set after restore (and re-applied on every
                // retry since the agent is rebuilt above) so it isn't clobbered.
                if let Some(ref ctx) = session_start_ctx {
                    agent.set_extra_system_context(ctx.clone());
                }

                let cancel_clone = cancel.clone();
                let durability_for_cb = durability.clone();

                let result = rt.block_on(async {
                    agent
                        .chat(
                            &text_owned,
                            &attachments_owned,
                            None,
                            cancel_clone,
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
                        )
                        .await
                });

                match result {
                    Ok((response, _thinking)) => {
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
                            .with_source(crate::chat_engine::ChatSource::Acp);
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
                        let mut usage_event =
                            crate::model_usage::ModelUsageEvent::new(crate::model_usage::KIND_CHAT);
                        usage_event = usage_event.with_usage(
                            usage.input_tokens.unwrap_or(0) as u64,
                            usage.output_tokens.unwrap_or(0) as u64,
                            usage.cache_creation_input_tokens.unwrap_or(0) as u64,
                            usage.cache_read_input_tokens.unwrap_or(0) as u64,
                        );
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
                        if cancel.load(Ordering::SeqCst) {
                            let commit = session::CommitInterruptedTurn {
                                run_id: durability
                                    .is_persistent()
                                    .then(|| durability.persistence_run_id().to_string()),
                                attempt_no: durability.current_attempt_no(),
                                session_id: session_id_owned.clone(),
                                assistant: Some(assistant_msg),
                                context_json,
                                expected_context_revision: durability.context_revision(),
                                turn_id: None,
                                final_seq,
                                status: session::ChatTurnStatus::Interrupted,
                                interrupt_reason: Some("user_stop".to_string()),
                                error: None,
                                recovery_event: None,
                            };
                            db_clone.commit_interrupted_turn(&commit)?;
                            durability.mark_interrupted("interrupted");
                        } else {
                            let commit = session::CommitAssistantTurn {
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
                            };
                            let committed = db_clone.commit_assistant_turn(&commit)?;
                            durability.mark_committed(committed.committed_seq);
                        }
                        crate::session_title::maybe_schedule_after_success(
                            db_clone.clone(),
                            session_id_owned.clone(),
                            agent_id.clone(),
                            model_ref.clone(),
                        );

                        let stop = if cancel.load(Ordering::SeqCst) {
                            "cancelled"
                        } else {
                            "end_turn"
                        };
                        return Ok(stop.to_string());
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        let reason = failover::classify_error(&last_error);

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
                            rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(
                                delay,
                            )));
                            continue;
                        }
                        break;
                    }
                }
            }
            if stop_all_models {
                break;
            }
        }

        // Converge provider/build failures through the same durable protocol;
        // do not leave a `running` run waiting for the next process restart.
        let final_error = format!("All models failed. Last error: {}", last_error);
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
                    .and_then(crate::chat_engine::finalize::ProviderApiKind::from_shape);
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
                        .and_then(crate::chat_engine::finalize::ProviderApiKind::from_shape),
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
        crate::chat_engine::finalize::rebuild::append_journal_suffix_to_history(
            &mut history,
            &visible_events,
            context_checkpoint_seq,
            provider_kind,
        )?;
        history.push(serde_json::json!({
            "role": "assistant",
            "content": crate::chat_engine::finalize::copy::model_marker(
                &crate::chat_engine::finalize::TerminationReason::ProviderFailed {
                    last_kind: failover::classify_error(&final_error),
                    last_message: final_error.clone(),
                    is_codex_auth: false,
                },
            ),
        }));
        let context_json = serde_json::to_string(&history)?;
        let assistant = session::journal_events_have_assistant_output(&visible_events).then(|| {
            session::NewMessage::assistant(&trailing)
                .with_source(crate::chat_engine::ChatSource::Acp)
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
            interrupt_reason: Some("provider_failed".to_string()),
            error: Some(
                integrity_error
                    .map(|integrity| format!("{final_error}; {integrity}"))
                    .unwrap_or_else(|| final_error.clone()),
            ),
            recovery_event: Some(
                session::NewMessage::error_event(&final_error)
                    .with_source(crate::chat_engine::ChatSource::Acp),
            ),
        };
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
        }
        durability.mark_interrupted("failed");
        Err(anyhow::anyhow!(final_error))
    }

    /// Build session modes from available agents
    fn build_modes(&self, current_agent_id: &str) -> SessionModeState {
        let agents = crate::agent_loader::list_agents().unwrap_or_default();
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
                            format!("{}...(truncated)", crate::truncate_utf8(result, 8192))
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

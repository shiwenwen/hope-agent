//! Core ACP Agent implementation.
//!
//! Implements all ACP Agent interface methods by translating between
//! ACP protocol and the existing OpenComputer AssistantAgent + SessionDB.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::acp::event_mapper;
use crate::acp::protocol::NdJsonTransport;
use crate::acp::session::{now_epoch_secs, AcpSession, AcpSessionStore};
use crate::acp::types::*;
use crate::agent::AssistantAgent;
use crate::failover;
use crate::provider;
use crate::session::{self, SessionDB};

/// ACP protocol version we advertise
const ACP_PROTOCOL_VERSION: &str = "0.2";

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
                name: "opencomputer-acp".to_string(),
                title: "OpenComputer ACP Agent".to_string(),
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

        let session_meta = match self.session_db.create_session(&agent_id) {
            Ok(m) => m,
            Err(e) => return JsonRpcResponse::error(id.clone(), ERROR_INTERNAL, e.to_string()),
        };

        let acp_session_id = session_meta.id.clone();

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

        // Save user message
        let _ = self
            .session_db
            .append_message(&session_id, &session::NewMessage::user(&text));

        // Auto-generate title
        if let Ok(Some(meta)) = self.session_db.get_session(&session_id) {
            if meta.title.is_none() && meta.message_count <= 1 {
                let title = session::auto_title(&text);
                let _ = self.session_db.update_session_title(&session_id, &title);
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
        }

        // Run agent chat
        let stop_reason = self.run_agent_chat(&session_id, &text, &attachments);

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

        let first = &model_chain[0];
        let prov = provider::find_provider(&store.providers, &first.provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", first.provider_id))?;

        let mut agent =
            AssistantAgent::new_from_provider(prov, &first.model_id).with_failover_context(prov);
        agent.set_agent_id(agent_id);
        agent.set_session_id(session_id);
        agent.set_compact_config(store.compact.clone());

        if let Some(ref model_ref) = store.compact.summarization_model {
            if let Some(cp) =
                crate::agent::build_compaction_provider(model_ref, &store.providers, session_id)
            {
                agent.set_compaction_provider(Some(std::sync::Arc::new(cp)));
            }
        }

        agent.set_web_search_enabled(crate::tools::web_search::has_enabled_provider(
            &store.web_search,
        ));
        agent.set_notification_enabled(store.notification.enabled);
        let image_gen_config = if crate::tools::image_generate::has_configured_provider_from_config(
            &store.image_generate,
        ) {
            let mut cfg = store.image_generate.clone();
            crate::tools::image_generate::backfill_providers(&mut cfg);
            Some(cfg)
        } else {
            None
        };
        agent.set_image_generate_config(image_gen_config);
        agent.set_canvas_enabled(store.canvas.enabled);

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

        // Build model chain for failover
        let store = crate::config::cached_config();
        let agent_id = self
            .sessions
            .get(session_id)
            .map(|s| s.agent_id.clone())
            .unwrap_or_else(|| self.default_agent_id.clone());

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

        // Build CompactionProvider once, reuse across retries
        let compaction_provider: Option<
            std::sync::Arc<dyn crate::context_compact::CompactionProvider>,
        > = store.compact.summarization_model.as_ref().and_then(|mr| {
            crate::agent::build_compaction_provider(mr, &store.providers, &session_id_owned)
                .map(|cp| std::sync::Arc::new(cp) as _)
        });

        for model_ref in &model_chain {
            let prov = match provider::find_provider(&store.providers, &model_ref.provider_id) {
                Some(p) => p,
                None => continue,
            };

            let mut retry_count: u32 = 0;
            loop {
                let mut agent = AssistantAgent::new_from_provider(prov, &model_ref.model_id)
                    .with_failover_context(prov);
                agent.set_agent_id(&agent_id);
                agent.set_session_id(&session_id_owned);
                agent.set_compact_config(store.compact.clone());
                if let Some(ref cp) = compaction_provider {
                    agent.set_compaction_provider(Some(cp.clone()));
                }

                // Restore context
                restore_agent_context(&db_clone, &session_id_owned, &agent);

                let cancel_clone = cancel.clone();
                let sid_for_cb = session_id_owned.clone();
                let db_for_cb = db_clone.clone();

                // Use a channel to send events from async callback to sync transport
                let (tx, rx) = std::sync::mpsc::channel::<String>();

                let result = rt.block_on(async {
                    agent
                        .chat(
                            &text_owned,
                            &attachments_owned,
                            None,
                            cancel_clone,
                            move |delta| {
                                // Map to ACP event
                                if let Some(notif) =
                                    event_mapper::map_agent_event(&sid_for_cb, delta)
                                {
                                    if let Ok(json) = serde_json::to_string(&notif) {
                                        let _ = tx.send(json);
                                    }
                                }
                                // Persist tool events
                                persist_tool_event_inline(&db_for_cb, &sid_for_cb, delta);
                            },
                        )
                        .await
                });

                // Flush all queued events to stdout
                while let Ok(event_json) = rx.try_recv() {
                    use std::io::Write;
                    let stdout = std::io::stdout();
                    let mut handle = stdout.lock();
                    let _ = writeln!(handle, "{}", event_json);
                    let _ = handle.flush();
                }

                match result {
                    Ok((response, thinking)) => {
                        let mut assistant_msg = session::NewMessage::assistant(&response);
                        assistant_msg.thinking = thinking;
                        let _ = self
                            .session_db
                            .append_message(&session_id_owned, &assistant_msg);
                        save_agent_context(&db_clone, &session_id_owned, &agent);

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
                            return Err(anyhow::anyhow!("{}", last_error));
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
        }

        Err(anyhow::anyhow!(
            "All models failed. Last error: {}",
            last_error
        ))
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

/// Save agent conversation history to DB
fn save_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &AssistantAgent) {
    let history = agent.get_conversation_history();
    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

/// Persist tool events (call + result) to session DB (inline version)
fn persist_tool_event_inline(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("tool_result") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                let duration_ms = event.get("duration_ms").and_then(|v| v.as_i64());
                let is_error = event
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let _ = db.update_tool_result(session_id, call_id, result, duration_ms, is_error);
            }
            Some("tool_call") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = event
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_msg = session::NewMessage::tool(call_id, name, arguments, "", None, false);
                let _ = db.append_message(session_id, &tool_msg);
            }
            _ => {}
        }
    }
}

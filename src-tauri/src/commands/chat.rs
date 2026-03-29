use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::State;
use crate::AppState;
use crate::agent::{AssistantAgent, Attachment};
use crate::provider::{self, ActiveModel, ApiType};
use crate::session::{self, SessionDB};
use crate::tools;
use crate::{context_compact, failover, memory, memory_extract, agent_loader};
use crate::truncate_utf8;

/// Build an AssistantAgent for a given ActiveModel.
/// Handles Codex (OAuth) vs regular API key providers.
/// Sets context_window from ModelConfig and compact_config from ProviderStore.
pub(crate) async fn build_agent_for_model(
    model: &ActiveModel,
    state: &State<'_, AppState>,
) -> Option<AssistantAgent> {
    let store = state.provider_store.lock().await;
    let prov = provider::find_provider(&store.providers, &model.provider_id)?;
    let compact_config = store.compact.clone();

    let mut agent = if prov.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        let (access_token, account_id) = token_info?;
        AssistantAgent::new_openai(&access_token, &account_id, &model.model_id)
    } else {
        AssistantAgent::new_from_provider(prov, &model.model_id)
    };
    agent.set_compact_config(compact_config);
    Some(agent)
}

/// Find the provider name + model name for display in fallback notifications.
pub(crate) async fn model_display_name(
    model: &ActiveModel,
    state: &State<'_, AppState>,
) -> String {
    let store = state.provider_store.lock().await;
    if let Some(prov) = store.providers.iter().find(|p| p.id == model.provider_id) {
        let model_name = prov.models.iter()
            .find(|m| m.id == model.model_id)
            .map(|m| m.name.as_str())
            .unwrap_or(&model.model_id);
        format!("{} / {}", prov.name, model_name)
    } else {
        format!("{}::{}", model.provider_id, model.model_id)
    }
}

/// Save an attachment file to disk. Uses a temp directory when session_id is empty.
/// Returns the absolute path to the saved file.
#[tauri::command]
pub async fn save_attachment(
    session_id: Option<String>,
    file_name: String,
    _mime_type: String,
    data: Vec<u8>,
) -> Result<String, String> {
    // Use temp directory if no session ID yet (new chat)
    let att_dir = match &session_id {
        Some(sid) if !sid.is_empty() => {
            crate::paths::attachments_dir(sid).map_err(|e| e.to_string())?
        }
        _ => {
            let root = crate::paths::root_dir().map_err(|e| e.to_string())?;
            root.join("attachments").join("_temp")
        }
    };
    std::fs::create_dir_all(&att_dir)
        .map_err(|e| format!("Failed to create attachments dir: {}", e))?;

    // Generate unique filename with timestamp to avoid collisions
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(['/', '\\', ':'], "_");
    let filename = format!("{}_{}", ts, safe_name);
    let file_path = att_dir.join(&filename);

    std::fs::write(&file_path, &data)
        .map_err(|e| format!("Failed to write attachment {}: {}", file_name, e))?;

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn chat(
    message: String,
    mut attachments: Vec<Attachment>,
    session_id: Option<String>,
    model_override: Option<String>,
    agent_id: Option<String>,
    tool_permission_mode: Option<String>,
    plan_mode: Option<String>,
    temperature_override: Option<f64>,
    on_event: tauri::ipc::Channel<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    // Set session-level tool permission mode
    if let Some(ref mode_str) = tool_permission_mode {
        let mode = match mode_str.as_str() {
            "ask_every_time" => crate::tools::ToolPermissionMode::AskEveryTime,
            "full_approve" => crate::tools::ToolPermissionMode::FullApprove,
            _ => crate::tools::ToolPermissionMode::Auto,
        };
        crate::tools::set_tool_permission_mode(mode).await;
    }

    let effort = state.reasoning_effort.lock().await.clone();
    let effort_ref_str = effort.clone();
    let db = state.session_db.clone();
    let cancel = state.chat_cancel.clone();
    cancel.store(false, Ordering::SeqCst); // Reset cancel flag
    let logger = state.logger.clone();
    // NOTE: _chat_session_guard is set later after session_id is resolved

    // Resolve or create session — prefer explicit agent_id from frontend
    let current_agent_id = match agent_id {
        Some(id) => {
            // Sync backend state so other code paths see the correct agent
            *state.current_agent_id.lock().await = id.clone();
            id
        }
        None => state.current_agent_id.lock().await.clone(),
    };
    let sid = match session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // Auto-create a new session
            let meta = db.create_session(&current_agent_id).map_err(|e| e.to_string())?;
            // Emit session_created event so frontend knows
            let event = serde_json::json!({
                "type": "session_created",
                "session_id": &meta.id,
            });
            if let Ok(json_str) = serde_json::to_string(&event) {
                let _ = on_event.send(json_str);
            }
            meta.id
        }
    };

    // Mark this session as active — cancels any running subagent injection and blocks new ones
    let _chat_session_guard = crate::subagent::ChatSessionGuard::new(&sid);

    // Build attachments metadata from file paths (files already saved by save_attachment)
    let attachments_meta = if !attachments.is_empty() {
        // Ensure session attachments directory exists and move temp files if needed
        let att_dir = crate::paths::attachments_dir(&sid).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&att_dir).map_err(|e| format!("Failed to create attachments dir: {}", e))?;

        let temp_dir = crate::paths::root_dir()
            .map(|r| r.join("attachments").join("_temp"))
            .unwrap_or_default();

        let mut meta_list = Vec::new();
        for att in attachments.iter_mut() {
            // Images: have base64 data directly, save to disk for persistence
            if let Some(ref b64_data) = att.data {
                use base64::Engine;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(b64_data)
                    .unwrap_or_default();
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let safe_name = att.name.replace(['/', '\\', ':'], "_");
                let filename = format!("{}_{}", ts, safe_name);
                let file_path = att_dir.join(&filename);
                if let Err(e) = std::fs::write(&file_path, &decoded) {
                    app_warn!("app", "chat", "Failed to save image attachment {}: {}", att.name, e);
                    continue;
                }
                meta_list.push(serde_json::json!({
                    "name": att.name,
                    "mime_type": att.mime_type,
                    "size": decoded.len(),
                    "path": file_path.to_string_lossy(),
                }));
                continue;
            }

            // Non-image files: have file_path, move from temp dir if needed
            if let Some(ref fp) = att.file_path {
                let src_path = std::path::Path::new(fp);

                let final_path = if src_path.starts_with(&temp_dir) {
                    if let Some(fname) = src_path.file_name() {
                        let dest = att_dir.join(fname);
                        if let Err(e) = std::fs::rename(src_path, &dest) {
                            if let Err(e2) = std::fs::copy(src_path, &dest) {
                                app_warn!("app", "chat", "Failed to move attachment {}: rename={}, copy={}", att.name, e, e2);
                                continue;
                            }
                            let _ = std::fs::remove_file(src_path);
                        }
                        dest
                    } else {
                        src_path.to_path_buf()
                    }
                } else {
                    src_path.to_path_buf()
                };

                // Update the attachment's file_path to the final location
                att.file_path = Some(final_path.to_string_lossy().to_string());

                let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                meta_list.push(serde_json::json!({
                    "name": att.name,
                    "mime_type": att.mime_type,
                    "size": size,
                    "path": final_path.to_string_lossy(),
                }));
            }
        }
        Some(serde_json::to_string(&meta_list).unwrap_or_default())
    } else {
        None
    };

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(&message);
    user_msg.attachments_meta = attachments_meta;
    let _ = db.append_message(&sid, &user_msg);

    // Log chat start
    let msg_preview = if message.len() > 100 { format!("{}...", truncate_utf8(&message, 100)) } else { message.clone() };
    logger.log("info", "session", "lib::chat", &format!("Chat started: {}", msg_preview),
        Some(serde_json::json!({"session_id": &sid, "attachments": attachments.len()}).to_string()),
        Some(sid.clone()), Some(current_agent_id.clone()));

    // Auto-generate title from first user message if session has no title
    if let Ok(Some(meta)) = db.get_session(&sid) {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = session::auto_title(&message);
            let _ = db.update_session_title(&sid, &title);
        }
    }

    // Resolve model chain and notification config from current agent config
    let agent_def = agent_loader::load_agent(&current_agent_id).ok();
    let agent_model_config = agent_def.as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();
    let agent_notify_on_complete = agent_def.as_ref()
        .and_then(|def| def.config.notify_on_complete);

    // Determine if notification tool should be available for this agent
    let notification_enabled = {
        let store = state.provider_store.lock().await;
        let global_enabled = store.notification.enabled;
        global_enabled && agent_notify_on_complete != Some(false)
    };

    let image_gen_config = {
        let store = state.provider_store.lock().await;
        if crate::tools::image_generate::has_configured_provider_from_config(&store.image_generate) {
            let mut cfg = store.image_generate.clone();
            crate::tools::image_generate::backfill_providers(&mut cfg);
            Some(cfg)
        } else {
            None
        }
    };

    let canvas_enabled = {
        let store = state.provider_store.lock().await;
        store.canvas.enabled
    };

    let web_search_enabled = {
        let store = state.provider_store.lock().await;
        crate::tools::web_search::has_enabled_provider(&store.web_search)
    };

    // Resolve temperature: session > agent > global
    let resolved_temperature: Option<f64> = {
        let global_temp = state.provider_store.lock().await.temperature;
        let agent_temp = agent_def.as_ref()
            .and_then(|def| def.config.model.temperature);
        // Priority: session (frontend override) > agent > global
        temperature_override.or(agent_temp).or(global_temp)
    };

    // Resolve plan state early so we can use plan_model override for model chain
    let early_plan_state = if let Some(ref pm) = plan_mode {
        let ps = crate::plan::PlanModeState::from_str(pm);
        if ps != crate::plan::PlanModeState::Off {
            crate::plan::set_plan_state(&sid, ps.clone()).await;
            let _ = db.update_session_plan_mode(&sid, pm);
        }
        ps
    } else {
        crate::plan::get_plan_state(&sid).await
    };

    // ── Plan Sub-Agent: optionally dispatch Planning to an isolated sub-agent ──
    // When plan_subagent=true, keeps the main agent's context clean for execution.
    // When plan_subagent=false (default), planning runs inline in the main agent.
    if early_plan_state == crate::plan::PlanModeState::Planning {
        let use_subagent = {
            let store = state.provider_store.lock().await;
            store.plan_subagent
        };

        if use_subagent {
            // Check if a plan sub-agent is already active for this session
            if let Some(run_id) = crate::plan::get_active_plan_run_id(&sid).await {
                // User sent a message while planning → route as steer to the sub-agent
                crate::subagent::SUBAGENT_MAILBOX.push(&run_id, message.clone());
                let _ = on_event.send(serde_json::json!({
                    "type": "text",
                    "text": "💬 Message forwarded to planning agent."
                }).to_string());
                return Ok("Message forwarded to planning agent.".to_string());
            }

            // First message in Planning state → spawn plan sub-agent
            let recent_summary = build_recent_context_summary(&db, &sid).await;
            let cancel_registry = crate::get_subagent_cancels()
                .cloned()
                .ok_or_else(|| "Sub-agent cancel registry not initialized".to_string())?;
            match crate::plan::spawn_plan_subagent(
                &sid, &current_agent_id, &message, &recent_summary,
                db.clone(), cancel_registry,
            ).await {
                Ok(run_id) => {
                    app_info!("plan", "chat", "Plan sub-agent spawned: run_id={}", run_id);
                    let _ = on_event.send(serde_json::json!({
                        "type": "text",
                        "text": "🗂️ Plan creation started..."
                    }).to_string());
                    return Ok(format!("Plan sub-agent spawned: {}", run_id));
                }
                Err(e) => {
                    app_error!("plan", "chat", "Failed to spawn plan sub-agent: {}", e);
                    // Fall through to inline planning as fallback
                }
            }
        }
        // else: use_subagent=false, fall through to inline PlanAgent mode below
    }

    let (primary, fallbacks) = {
        let store = state.provider_store.lock().await;
        // Plan Mode model override: use cheaper/faster model during Planning phase
        let plan_model_override = if early_plan_state == crate::plan::PlanModeState::Planning {
            agent_model_config.plan_model.clone()
        } else {
            None
        };

        if let Some(ref plan_model_str) = plan_model_override {
            // Planning phase: use plan_model as primary, keep fallbacks
            let mut cfg = agent_model_config.clone();
            cfg.primary = Some(plan_model_str.clone());
            provider::resolve_model_chain(&cfg, &store)
        } else if let Some(ref override_str) = model_override {
            // User explicitly selected a model in the input box
            let override_model = provider::parse_model_ref(override_str);
            let mut cfg = agent_model_config.clone();
            if override_model.is_some() {
                cfg.primary = Some(override_str.clone());
            }
            provider::resolve_model_chain(&cfg, &store)
        } else {
            provider::resolve_model_chain(&agent_model_config, &store)
        }
    };

    // Build ordered model chain: [primary, ...fallbacks]
    let mut model_chain: Vec<ActiveModel> = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        // Avoid duplicates
        if !model_chain.iter().any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id) {
            model_chain.push(fb);
        }
    }

    // Log model chain resolution
    logger.log("info", "agent", "lib::chat::model_chain",
        &format!("Model chain resolved: {} models", model_chain.len()),
        Some(serde_json::json!({
            "chain": model_chain.iter().map(|m| format!("{}::{}", m.provider_id, m.model_id)).collect::<Vec<_>>(),
            "total": model_chain.len(),
        }).to_string()),
        Some(sid.clone()), Some(current_agent_id.clone()));

    if model_chain.is_empty() {
        // No model chain resolved — fall back to existing agent instance
        let agent_lock = state.agent.lock().await;
        return match agent_lock.as_ref() {
            Some(agent) => {
                // Restore conversation history from DB for this session
                restore_agent_context(&db, &sid, agent);

                let effort_ref = Some(effort_ref_str.as_str());
                let db_for_cb = db.clone();
                let sid_for_cb = sid.clone();
                let cancel_clone = cancel.clone();
                let chat_start = std::time::Instant::now();
                let on_event_clone = on_event.clone();
                // Shared state to capture token usage, model, and TTFT from on_delta callback
                let captured_usage: Arc<std::sync::Mutex<(Option<i64>, Option<i64>, Option<String>, Option<i64>)>> = Arc::new(std::sync::Mutex::new((None, None, None, None)));
                let captured_usage_clone = captured_usage.clone();
                let (result, thinking) = match agent.chat(&message, &attachments, effort_ref, cancel_clone, move |delta| {
                    // Intercept usage events to capture token counts, model, and TTFT
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                        if event.get("type").and_then(|t| t.as_str()) == Some("usage") {
                            if let Ok(mut usage) = captured_usage_clone.lock() {
                                if let Some(it) = event.get("input_tokens").and_then(|v| v.as_i64()) {
                                    usage.0 = Some(it);
                                }
                                if let Some(ot) = event.get("output_tokens").and_then(|v| v.as_i64()) {
                                    usage.1 = Some(ot);
                                }
                                if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                                    usage.2 = Some(m.to_string());
                                }
                                if let Some(ttft) = event.get("ttft_ms").and_then(|v| v.as_i64()) {
                                    usage.3 = Some(ttft);
                                }
                            }
                        }
                    }
                    persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                    let _ = on_event_clone.send(delta.to_string());
                }).await {
                    Ok((text, thinking)) => (text, thinking),
                    Err(e) => {
                        let err = e.to_string();
                        let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                        return Err(err);
                    }
                };
                let duration_ms = chat_start.elapsed().as_millis() as u64;
                // Emit usage event with duration
                emit_usage_event(&on_event, duration_ms);
                // Save assistant reply with duration, tokens, and TTFT
                let mut assistant_msg = session::NewMessage::assistant(&result);
                assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                assistant_msg.thinking = thinking;
                if let Ok(usage) = captured_usage.lock() {
                    assistant_msg.tokens_in = usage.0;
                    assistant_msg.tokens_out = usage.1;
                    assistant_msg.model = usage.2.clone();
                    assistant_msg.ttft_ms = usage.3;
                }
                let _ = db.append_message(&sid, &assistant_msg);
                // Persist conversation context for future restoration
                save_agent_context(&db, &sid, agent);
                Ok(result)
            }
            None => {
                let err = "Agent not initialized. Please sign in first.".to_string();
                let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                Err(err)
            }
        };
    }

    let mut last_error: Option<String> = None;
    let total_models = model_chain.len();
    // Track first model for "from_model" in fallback events
    let primary_display = {
        let first = &model_chain[0];
        model_display_name(first, &state).await
    };

    for (idx, model_ref) in model_chain.iter().enumerate() {
        // Log model attempt
        logger.log("info", "agent", "lib::chat::model_attempt",
            &format!("Trying model {}/{}: {}::{}", idx + 1, total_models, model_ref.provider_id, model_ref.model_id),
            Some(serde_json::json!({
                "model_index": idx,
                "total_models": total_models,
                "provider_id": &model_ref.provider_id,
                "model_id": &model_ref.model_id,
            }).to_string()),
            Some(sid.clone()), Some(current_agent_id.clone()));

        let mut agent = match build_agent_for_model(model_ref, &state).await {
            Some(a) => a,
            None => {
                last_error = Some(format!("Cannot build agent for {}::{}", model_ref.provider_id, model_ref.model_id));
                continue;
            }
        };
        agent.set_agent_id(&current_agent_id);
        agent.set_session_id(&sid);
        agent.set_web_search_enabled(web_search_enabled);
        agent.set_notification_enabled(notification_enabled);
        agent.set_image_generate_config(image_gen_config.clone());
        agent.set_canvas_enabled(canvas_enabled);
        agent.set_temperature(resolved_temperature);

        // ── Plan Mode: dual-agent architecture ──
        // Planning state is handled by sub-agent above (early return).
        // This section handles Review (inline), Build (executing), and fallback Planning.
        let plan_state = crate::plan::get_plan_state(&sid).await;
        match plan_state {
            crate::plan::PlanModeState::Planning | crate::plan::PlanModeState::Review => {
                // Planning fallback (sub-agent spawn failed) or Review state
                // Review: main agent handles change requests with plan content
                let config = crate::plan::PlanAgentConfig::default_config();
                agent.set_plan_agent_mode(crate::agent::PlanAgentMode::PlanAgent {
                    allowed_tools: config.allowed_tools,
                    ask_tools: config.ask_tools,
                });
                agent.set_plan_mode_allow_paths(config.plan_mode_allow_paths);
                let prompt = if plan_state == crate::plan::PlanModeState::Review {
                    if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                        format!("# Plan Review\n\nThe following plan has been submitted and is awaiting user approval:\n\n{}", plan_content)
                    } else {
                        crate::plan::PLAN_MODE_SYSTEM_PROMPT.to_string()
                    }
                } else {
                    crate::plan::PLAN_MODE_SYSTEM_PROMPT.to_string()
                };
                agent.set_extra_system_context(prompt);
            }
            crate::plan::PlanModeState::Executing | crate::plan::PlanModeState::Paused => {
                // ── Build Agent ──
                agent.set_plan_agent_mode(crate::agent::PlanAgentMode::BuildAgent {
                    extra_tools: crate::plan::BUILD_AGENT_EXTRA_TOOLS.iter().map(|s| s.to_string()).collect(),
                });
                if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                    let prefix = if plan_state == crate::plan::PlanModeState::Paused {
                        let paused_step = crate::plan::get_plan_meta(&sid).await
                            .and_then(|m| m.paused_at_step)
                            .unwrap_or(0);
                        format!(
                            "# Plan Paused\n\nPlan execution is currently **paused** at step {}. \
                             The user may ask to resume, modify the plan, or discuss progress.\n\n\
                             ## Plan Content\n\n",
                            paused_step
                        )
                    } else {
                        crate::plan::PLAN_EXECUTING_SYSTEM_PROMPT_PREFIX.to_string()
                    };
                    agent.set_extra_system_context(format!("{}{}", prefix, plan_content));
                }
            }
            crate::plan::PlanModeState::Completed => {
                // No special tool mode — just inject summary context
                if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                    let step_summary = if let Some(meta) = crate::plan::get_plan_meta(&sid).await {
                        let completed = meta.steps.iter().filter(|s| s.status == crate::plan::PlanStepStatus::Completed).count();
                        let failed = meta.steps.iter().filter(|s| s.status == crate::plan::PlanStepStatus::Failed).count();
                        let skipped = meta.steps.iter().filter(|s| s.status == crate::plan::PlanStepStatus::Skipped).count();
                        format!("\n\n## Statistics\n- Completed: {}\n- Failed: {}\n- Skipped: {}\n- Total: {}\n",
                            completed, failed, skipped, meta.steps.len())
                    } else {
                        String::new()
                    };
                    agent.set_extra_system_context(format!(
                        "{}{}{}",
                        crate::plan::PLAN_COMPLETED_SYSTEM_PROMPT,
                        plan_content,
                        step_summary
                    ));
                }
            }
            crate::plan::PlanModeState::Off => {}
        }

        // Restore conversation history from DB for this session
        restore_agent_context(&db, &sid, &agent);

        // Determine max retries for this model
        const MAX_RETRIES: u32 = 2;
        const RETRY_BASE_MS: u64 = 1000;
        const RETRY_MAX_MS: u64 = 10000;

        let mut retry_count: u32 = 0;

        loop {
            // If this is a fallback (not the first model) and first attempt, notify frontend
            if idx > 0 && retry_count == 0 {
                let display = model_display_name(model_ref, &state).await;
                let reason_str = last_error.as_deref()
                    .map(|e| failover::classify_error(e))
                    .unwrap_or(failover::FailoverReason::Unknown);
                let event = serde_json::json!({
                    "type": "model_fallback",
                    "model": display,
                    "from_model": primary_display,
                    "provider_id": model_ref.provider_id,
                    "model_id": model_ref.model_id,
                    "reason": reason_str,
                    "attempt": idx + 1,
                    "total": total_models,
                    "error": last_error.as_deref().unwrap_or(""),
                });
                if let Ok(json_str) = serde_json::to_string(&event) {
                    let _ = on_event.send(json_str.clone());
                    // Persist fallback event to session DB
                    let _ = db.append_message(&sid, &session::NewMessage::event(&json_str));
                }
            }

            // Update session with current model info
            if retry_count == 0 {
                let store = state.provider_store.lock().await;
                let provider_name = store.providers.iter()
                    .find(|p| p.id == model_ref.provider_id)
                    .map(|p| p.name.as_str());
                let _ = db.update_session_model(&sid, Some(&model_ref.provider_id), provider_name, Some(&model_ref.model_id));
            }

            let effort_ref = Some(effort_ref_str.as_str());
            let on_event_clone = on_event.clone();
            let db_for_cb = db.clone();
            let sid_for_cb = sid.clone();
            let cancel_clone = cancel.clone();

            // Shared state to capture token usage, model, and TTFT from on_delta callback
            let captured_usage: Arc<std::sync::Mutex<(Option<i64>, Option<i64>, Option<String>, Option<i64>)>> = Arc::new(std::sync::Mutex::new((None, None, None, None)));
            let captured_usage_clone = captured_usage.clone();

            // Accumulate text_delta content; flush as text_block before tool_call to preserve ordering
            let pending_text: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
            let pending_text_clone = pending_text.clone();

            // Accumulate thinking_delta content; flush as thinking_block before tool_call to preserve multi-round thinking
            let pending_thinking: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
            let pending_thinking_clone = pending_thinking.clone();
            // Track whether any thinking_block was persisted (to avoid duplicating on assistant message)
            let had_thinking_blocks: Arc<std::sync::atomic::AtomicBool> = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let had_thinking_blocks_clone = had_thinking_blocks.clone();

            let chat_start = std::time::Instant::now();
            match agent.chat(&message, &attachments, effort_ref, cancel_clone, move |delta| {
                // Intercept usage events to capture token counts, model, and TTFT
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                    match event.get("type").and_then(|t| t.as_str()) {
                        Some("usage") => {
                            if let Ok(mut usage) = captured_usage_clone.lock() {
                                if let Some(it) = event.get("input_tokens").and_then(|v| v.as_i64()) {
                                    usage.0 = Some(it);
                                }
                                if let Some(ot) = event.get("output_tokens").and_then(|v| v.as_i64()) {
                                    usage.1 = Some(ot);
                                }
                                if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                                    usage.2 = Some(m.to_string());
                                }
                                if let Some(ttft) = event.get("ttft_ms").and_then(|v| v.as_i64()) {
                                    usage.3 = Some(ttft);
                                }
                            }
                        }
                        Some("thinking_delta") => {
                            // Accumulate thinking content for ordering preservation
                            if let Some(text) = event.get("content").and_then(|t| t.as_str()) {
                                if let Ok(mut pk) = pending_thinking_clone.lock() {
                                    pk.push_str(text);
                                }
                            }
                        }
                        Some("text_delta") => {
                            // Accumulate text content for ordering preservation
                            if let Some(text) = event.get("text").and_then(|t| t.as_str()) {
                                if let Ok(mut pt) = pending_text_clone.lock() {
                                    pt.push_str(text);
                                }
                            }
                        }
                        Some("tool_call") => {
                            // Flush accumulated thinking as thinking_block before tool_call
                            if let Ok(mut pk) = pending_thinking_clone.lock() {
                                if !pk.is_empty() {
                                    let thinking_msg = session::NewMessage::thinking_block(&pk);
                                    let _ = db_for_cb.append_message(&sid_for_cb, &thinking_msg);
                                    pk.clear();
                                    had_thinking_blocks_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                            // Flush accumulated text as text_block before tool_call
                            if let Ok(mut pt) = pending_text_clone.lock() {
                                if !pt.is_empty() {
                                    let text_msg = session::NewMessage::text_block(&pt);
                                    let _ = db_for_cb.append_message(&sid_for_cb, &text_msg);
                                    pt.clear();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                let _ = on_event_clone.send(delta.to_string());
            }).await {
                Ok((result, thinking)) => {
                    let duration_ms = chat_start.elapsed().as_millis() as u64;
                    // Emit usage event with duration
                    emit_usage_event(&on_event, duration_ms);
                    // Flush any remaining pending thinking as thinking_block
                    if let Ok(mut pk) = pending_thinking.lock() {
                        if !pk.is_empty() {
                            let thinking_msg = session::NewMessage::thinking_block(&pk);
                            let _ = db.append_message(&sid, &thinking_msg);
                            pk.clear();
                            had_thinking_blocks.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                    let has_thinking_blocks = had_thinking_blocks.load(std::sync::atomic::Ordering::SeqCst);
                    // Save assistant reply to DB with duration, tokens, and TTFT
                    let mut assistant_msg = session::NewMessage::assistant(&result);
                    assistant_msg.tool_duration_ms = Some(duration_ms as i64);
                    // Only store thinking on assistant message if no thinking_blocks were persisted
                    // (thinking_blocks preserve multi-round ordering; assistant.thinking is a flat concatenation)
                    if !has_thinking_blocks {
                        assistant_msg.thinking = thinking;
                    }
                    if let Ok(usage) = captured_usage.lock() {
                        assistant_msg.tokens_in = usage.0;
                        assistant_msg.tokens_out = usage.1;
                        assistant_msg.model = usage.2.clone();
                        assistant_msg.ttft_ms = usage.3;
                    }
                    let _ = db.append_message(&sid, &assistant_msg);
                    // Persist conversation context for future restoration
                    save_agent_context(&db, &sid, &agent);
                    // Log chat success
                    logger.log("info", "session", "lib::chat::done",
                        &format!("Chat completed in {}ms, model {}::{}", duration_ms, model_ref.provider_id, model_ref.model_id),
                        Some(serde_json::json!({
                            "duration_ms": duration_ms,
                            "provider_id": &model_ref.provider_id,
                            "model_id": &model_ref.model_id,
                            "model_index": idx,
                            "response_length": result.len(),
                            "tokens_in": assistant_msg.tokens_in,
                            "tokens_out": assistant_msg.tokens_out,
                        }).to_string()),
                        Some(sid.clone()), Some(current_agent_id.clone()));
                    // ── Plan Mode: auto-detect plan content from LLM output ──
                    if plan_state == crate::plan::PlanModeState::Planning && !result.is_empty() {
                        let steps = crate::plan::parse_plan_steps(&result);
                        if steps.len() >= 2 {
                            // Save plan file + update in-memory steps
                            let _ = crate::plan::save_plan_file(&sid, &result);
                            crate::plan::update_plan_steps(&sid, steps.clone()).await;
                            // Emit Tauri global event so PlanPanel updates in real-time
                            if let Some(app_handle) = crate::get_app_handle() {
                                use tauri::Emitter;
                                let _ = app_handle.emit("plan_content_updated", serde_json::json!({
                                    "sessionId": &sid,
                                    "stepCount": steps.len(),
                                    "content": &result,
                                }));
                            }
                        }
                    }

                    // Spawn async memory extraction if enabled
                    // Resolve effective config: agent override → global fallback
                    {
                        let global_extract = memory::load_extract_config();
                        let agent_def = crate::agent_loader::load_agent(&current_agent_id);
                        let agent_mem = agent_def.as_ref().ok().map(|d| &d.config.memory);

                        let auto_extract = agent_mem
                            .and_then(|m| m.auto_extract)
                            .unwrap_or(global_extract.auto_extract);
                        let min_turns = agent_mem
                            .and_then(|m| m.extract_min_turns)
                            .unwrap_or(global_extract.extract_min_turns);
                        let history = agent.get_conversation_history();

                        if auto_extract && history.len() >= min_turns * 2 {
                            // Resolve extraction model: agent override → global → chat model
                            let extract_agent_id = current_agent_id.clone();
                            let extract_session_id = sid.clone();
                            let extract_provider_id = agent_mem
                                .and_then(|m| m.extract_provider_id.clone())
                                .or_else(|| global_extract.extract_provider_id.clone())
                                .unwrap_or_else(|| model_ref.provider_id.clone());
                            let extract_model_id = agent_mem
                                .and_then(|m| m.extract_model_id.clone())
                                .or_else(|| global_extract.extract_model_id.clone())
                                .unwrap_or_else(|| model_ref.model_id.clone());

                            tokio::spawn(async move {
                                // Load provider config for extraction
                                let store = provider::load_store().unwrap_or_default();
                                if let Some(prov) = provider::find_provider(&store.providers, &extract_provider_id) {
                                    memory_extract::run_extraction(
                                        &history,
                                        &extract_agent_id,
                                        &extract_session_id,
                                        prov,
                                        &extract_model_id,
                                    ).await;
                                }
                            });
                        }
                    }

                    // Update the active agent instance for conversation continuity
                    *state.agent.lock().await = Some(agent);
                    return Ok(result);
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    let reason = failover::classify_error(&error_msg);

                    app_warn!("provider", "failover",
                        "Model {}::{} failed (attempt {}/{}, retry {}, reason {:?}): {}",
                        model_ref.provider_id, model_ref.model_id,
                        idx + 1, total_models, retry_count, reason, error_msg
                    );
                    logger.log("warn", "provider", "lib::chat::failover",
                        &format!("Model {}::{} failed: {:?}", model_ref.provider_id, model_ref.model_id, reason),
                        Some(serde_json::json!({
                            "provider_id": model_ref.provider_id,
                            "model_id": model_ref.model_id,
                            "attempt": idx + 1,
                            "retry": retry_count,
                            "reason": format!("{:?}", reason),
                            "error": error_msg,
                        }).to_string()),
                        Some(sid.clone()), Some(current_agent_id.clone()));

                    // Context overflow — try emergency compaction, then retry once
                    if reason.needs_compaction() && retry_count == 0 {
                        app_info!("context", "compact",
                            "Context overflow on {}::{}, attempting emergency compaction",
                            model_ref.provider_id, model_ref.model_id
                        );
                        let compact_config = {
                            let store = state.provider_store.lock().await;
                            store.compact.clone()
                        };
                        let mut history = agent.get_conversation_history();
                        let result = context_compact::emergency_compact(&mut history, &compact_config);
                        agent.set_conversation_history(history);
                        // Persist compacted context immediately to prevent data loss
                        save_agent_context(&db, &sid, &agent);

                        // Notify frontend
                        if let Ok(event_str) = serde_json::to_string(&serde_json::json!({
                            "type": "context_compacted",
                            "data": result,
                        })) {
                            let _ = on_event.send(event_str);
                        }

                        retry_count += 1;
                        continue; // Retry with compacted context
                    }

                    // Terminal errors — surface immediately, no fallback
                    if reason.is_terminal() || reason.needs_compaction() {
                        // Still persist any in-memory compaction that happened during this attempt
                        save_agent_context(&db, &sid, &agent);
                        let _ = db.append_message(&sid, &session::NewMessage::event(&error_msg));
                        return Err(error_msg);
                    }

                    // Retryable errors — retry on same model with backoff
                    if reason.is_retryable() && retry_count < MAX_RETRIES {
                        retry_count += 1;
                        let delay = failover::retry_delay_ms(retry_count - 1, RETRY_BASE_MS, RETRY_MAX_MS);
                        app_info!("provider", "failover",
                            "Retrying {}::{} in {}ms (retry {}/{}, reason {:?})",
                            model_ref.provider_id, model_ref.model_id,
                            delay, retry_count, MAX_RETRIES, reason
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        continue; // Retry same model
                    }

                    // Emit codex_auth_expired event when a Codex provider gets an Auth error
                    if matches!(reason, failover::FailoverReason::Auth) {
                        let is_codex = {
                            let store = state.provider_store.lock().await;
                            store.providers.iter()
                                .find(|p| p.id == model_ref.provider_id)
                                .map(|p| p.api_type == ApiType::Codex)
                                .unwrap_or(false)
                        };
                        if is_codex {
                            let event = serde_json::json!({
                                "type": "codex_auth_expired",
                                "error": &error_msg,
                            });
                            if let Ok(json_str) = serde_json::to_string(&event) {
                                let _ = on_event.send(json_str);
                            }
                        }
                    }

                    // Non-retryable or retries exhausted — move to next model
                    last_error = Some(error_msg);
                    break; // Break inner retry loop, continue outer model loop
                }
            }
        }
    }

    // Persist any in-memory compaction before returning error
    if let Some(ref agent) = *state.agent.lock().await {
        save_agent_context(&db, &sid, agent);
    }
    let final_error = last_error.unwrap_or_else(|| "All models in the fallback chain failed.".to_string());
    let _ = db.append_message(&sid, &session::NewMessage::event(&final_error));
    Err(final_error)
}

#[tauri::command]
pub async fn stop_chat(state: State<'_, AppState>) -> Result<(), String> {
    state.chat_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// Restore conversation history from DB into the agent (if the session has saved context).
pub(crate) fn restore_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &crate::agent::AssistantAgent) {
    if let Ok(Some(json_str)) = db.load_context(session_id) {
        if let Ok(history) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            if !history.is_empty() {
                if let Some(logger) = crate::get_logger() {
                    logger.log("debug", "session", "lib::restore_context",
                        &format!("Restored {} messages for session {} ({}B JSON)", history.len(), session_id, json_str.len()),
                        Some(serde_json::json!({
                            "message_count": history.len(),
                            "json_size_bytes": json_str.len(),
                        }).to_string()),
                        Some(session_id.to_string()), None);
                }
                agent.set_conversation_history(history);
            }
        }
    }
}

/// Save the agent's conversation history to DB for future restoration.
pub(crate) fn save_agent_context(db: &Arc<SessionDB>, session_id: &str, agent: &crate::agent::AssistantAgent) {
    let history = agent.get_conversation_history();
    if let Ok(json_str) = serde_json::to_string(&history) {
        let _ = db.save_context(session_id, &json_str);
    }
}

/// Emit a usage event (with duration) to the frontend via the Tauri Channel.
/// Build a compact summary of recent conversation for passing to a plan sub-agent.
/// Returns up to the last N messages as a condensed text summary.
async fn build_recent_context_summary(db: &Arc<SessionDB>, session_id: &str) -> String {
    const MAX_MESSAGES: u32 = 10;
    const MAX_CHARS: usize = 4000;

    // Load the latest messages (excluding the just-appended user message which is the task)
    let (messages, _total) = match db.load_session_messages_latest(session_id, MAX_MESSAGES + 1) {
        Ok(result) => result,
        Err(_) => return String::new(),
    };

    if messages.len() <= 1 {
        return String::new();
    }

    // Skip the last message (it's the task itself, just appended)
    let relevant = &messages[..messages.len() - 1];

    let mut summary = String::new();
    for msg in relevant {
        let role = &msg.role;
        let content = &msg.content;
        let line = format!("[{:?}]: {}\n", role, truncate_utf8(content, 500));
        if summary.len() + line.len() > MAX_CHARS {
            summary.push_str("...(earlier messages omitted)\n");
            break;
        }
        summary.push_str(&line);
    }

    summary
}

fn emit_usage_event(on_event: &tauri::ipc::Channel<String>, duration_ms: u64) {
    let event = serde_json::json!({
        "type": "usage",
        "duration_ms": duration_ms,
    });
    if let Ok(json_str) = serde_json::to_string(&event) {
        let _ = on_event.send(json_str);
    }
}

/// Parse tool_call and tool_result events from the streaming callback and persist to DB.
fn persist_tool_event(db: &Arc<SessionDB>, session_id: &str, delta: &str) {
    if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("tool_result") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let result = event.get("result").and_then(|v| v.as_str()).unwrap_or("");
                let duration_ms = event.get("duration_ms").and_then(|v| v.as_i64());
                let is_error = event.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                // Update the existing tool_call record with result/duration/is_error
                let _ = db.update_tool_result(session_id, call_id, result, duration_ms, is_error);
            }
            Some("tool_call") => {
                let call_id = event.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let name = event.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = event.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                let tool_msg = session::NewMessage::tool(
                    call_id,
                    name,
                    arguments,
                    "", // result will be filled in tool_result event
                    None,
                    false,
                );
                let _ = db.append_message(session_id, &tool_msg);
            }
            _ => {
                // text_delta events are not persisted as separate messages.
                // text_delta is accumulated into the final assistant message.
            }
        }
    }
}

// ── Command Approval ──────────────────────────────────────────────

#[tauri::command]
pub async fn respond_to_approval(
    request_id: String,
    response: String,
) -> Result<(), String> {
    let approval_response = match response.as_str() {
        "allow_once" => tools::ApprovalResponse::AllowOnce,
        "allow_always" => tools::ApprovalResponse::AllowAlways,
        "deny" => tools::ApprovalResponse::Deny,
        _ => return Err(format!("Invalid approval response: {}", response)),
    };
    tools::submit_approval_response(&request_id, approval_response)
        .await
        .map_err(|e| e.to_string())
}

// ── Tools Info Commands ───────────────────────────────────────────

#[tauri::command]
pub async fn list_builtin_tools() -> Result<Vec<serde_json::Value>, String> {
    let mut all = tools::get_available_tools();
    // Include conditionally-injected tools so they appear in settings
    all.push(tools::get_notification_tool());
    Ok(all
        .into_iter()
        .map(|t| serde_json::json!({ "name": t.name, "description": t.description, "internal": t.internal }))
        .collect())
}

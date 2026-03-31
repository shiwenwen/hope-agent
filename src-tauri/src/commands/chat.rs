use crate::agent::Attachment;
use crate::agent_loader;
use crate::provider::{self, ActiveModel};
use crate::session::{self, SessionDB};
use crate::tools;
use crate::truncate_utf8;
use crate::AppState;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::State;

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
    let mut new_session_created: Option<String> = None;
    let sid = match session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // Auto-create a new session; emit session_created after auto_title is set
            let meta = db
                .create_session(&current_agent_id)
                .map_err(|e| e.to_string())?;
            new_session_created = Some(meta.id.clone());
            meta.id
        }
    };

    // Mark this session as active — cancels any running subagent injection and blocks new ones
    let _chat_session_guard = crate::subagent::ChatSessionGuard::new(&sid);

    // Build attachments metadata from file paths (files already saved by save_attachment)
    let attachments_meta = if !attachments.is_empty() {
        // Ensure session attachments directory exists and move temp files if needed
        let att_dir = crate::paths::attachments_dir(&sid).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&att_dir)
            .map_err(|e| format!("Failed to create attachments dir: {}", e))?;

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
                    app_warn!(
                        "app",
                        "chat",
                        "Failed to save image attachment {}: {}",
                        att.name,
                        e
                    );
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
                                app_warn!(
                                    "app",
                                    "chat",
                                    "Failed to move attachment {}: rename={}, copy={}",
                                    att.name,
                                    e,
                                    e2
                                );
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
    let msg_preview = if message.len() > 100 {
        format!("{}...", truncate_utf8(&message, 100))
    } else {
        message.clone()
    };
    logger.log(
        "info",
        "session",
        "lib::chat",
        &format!("Chat started: {}", msg_preview),
        Some(serde_json::json!({"session_id": &sid, "attachments": attachments.len()}).to_string()),
        Some(sid.clone()),
        Some(current_agent_id.clone()),
    );

    // Auto-generate title from first user message if session has no title
    if let Ok(Some(meta)) = db.get_session(&sid) {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = session::auto_title(&message);
            let _ = db.update_session_title(&sid, &title);
        }
    }

    // Emit session_created now that title is set, so frontend's reloadSessions() gets the title
    if let Some(ref new_sid) = new_session_created {
        let event = serde_json::json!({
            "type": "session_created",
            "session_id": new_sid,
        });
        if let Ok(json_str) = serde_json::to_string(&event) {
            let _ = on_event.send(json_str);
        }
    }

    // Resolve model chain and notification config from current agent config
    let agent_def = agent_loader::load_agent(&current_agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();
    let agent_notify_on_complete = agent_def
        .as_ref()
        .and_then(|def| def.config.notify_on_complete);

    // Determine if notification tool should be available for this agent
    let notification_enabled = {
        let store = state.provider_store.lock().await;
        let global_enabled = store.notification.enabled;
        global_enabled && agent_notify_on_complete != Some(false)
    };

    let image_gen_config = {
        let store = state.provider_store.lock().await;
        if crate::tools::image_generate::has_configured_provider_from_config(&store.image_generate)
        {
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
        let agent_temp = agent_def
            .as_ref()
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
                let _ = on_event.send(
                    serde_json::json!({
                        "type": "text",
                        "text": "💬 Message forwarded to planning agent."
                    })
                    .to_string(),
                );
                return Ok("Message forwarded to planning agent.".to_string());
            }

            // First message in Planning state → spawn plan sub-agent
            let recent_summary = build_recent_context_summary(&db, &sid).await;
            let cancel_registry = crate::get_subagent_cancels()
                .cloned()
                .ok_or_else(|| "Sub-agent cancel registry not initialized".to_string())?;
            match crate::plan::spawn_plan_subagent(
                &sid,
                &current_agent_id,
                &message,
                &recent_summary,
                db.clone(),
                cancel_registry,
            )
            .await
            {
                Ok(run_id) => {
                    app_info!("plan", "chat", "Plan sub-agent spawned: run_id={}", run_id);
                    let _ = on_event.send(
                        serde_json::json!({
                            "type": "text",
                            "text": "🗂️ Plan creation started..."
                        })
                        .to_string(),
                    );
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
        if !model_chain
            .iter()
            .any(|m| m.provider_id == fb.provider_id && m.model_id == fb.model_id)
        {
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
                crate::chat_engine::restore_agent_context(&db, &sid, agent);

                let effort_ref = Some(effort_ref_str.as_str());
                let db_for_cb = db.clone();
                let sid_for_cb = sid.clone();
                let cancel_clone = cancel.clone();
                let chat_start = std::time::Instant::now();
                let on_event_clone = on_event.clone();
                let captured_usage: Arc<
                    std::sync::Mutex<(Option<i64>, Option<i64>, Option<String>, Option<i64>)>,
                > = Arc::new(std::sync::Mutex::new((None, None, None, None)));
                let captured_usage_clone = captured_usage.clone();
                let (result, thinking) = match agent
                    .chat(
                        &message,
                        &attachments,
                        effort_ref,
                        cancel_clone,
                        move |delta| {
                            if let Ok(event) = serde_json::from_str::<serde_json::Value>(delta) {
                                if event.get("type").and_then(|t| t.as_str()) == Some("usage") {
                                    if let Ok(mut usage) = captured_usage_clone.lock() {
                                        if let Some(it) =
                                            event.get("input_tokens").and_then(|v| v.as_i64())
                                        {
                                            usage.0 = Some(it);
                                        }
                                        if let Some(ot) =
                                            event.get("output_tokens").and_then(|v| v.as_i64())
                                        {
                                            usage.1 = Some(ot);
                                        }
                                        if let Some(m) = event.get("model").and_then(|v| v.as_str())
                                        {
                                            usage.2 = Some(m.to_string());
                                        }
                                        if let Some(ttft) =
                                            event.get("ttft_ms").and_then(|v| v.as_i64())
                                        {
                                            usage.3 = Some(ttft);
                                        }
                                    }
                                }
                            }
                            crate::chat_engine::persist_tool_event(&db_for_cb, &sid_for_cb, delta);
                            let _ = on_event_clone.send(delta.to_string());
                        },
                    )
                    .await
                {
                    Ok((text, thinking)) => (text, thinking),
                    Err(e) => {
                        let err = e.to_string();
                        let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                        return Err(err);
                    }
                };
                let duration_ms = chat_start.elapsed().as_millis() as u64;
                let usage_event = serde_json::json!({"type": "usage", "duration_ms": duration_ms});
                if let Ok(json_str) = serde_json::to_string(&usage_event) {
                    let _ = on_event.send(json_str);
                }
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
                crate::chat_engine::save_agent_context(&db, &sid, agent);
                Ok(result)
            }
            None => {
                let err = "Agent not initialized. Please sign in first.".to_string();
                let _ = db.append_message(&sid, &session::NewMessage::event(&err));
                Err(err)
            }
        };
    }

    // ── Resolve Plan Mode agent configuration ──
    let plan_state = crate::plan::get_plan_state(&sid).await;
    let (plan_extra_context, plan_agent_mode, plan_allow_paths) = match plan_state {
        crate::plan::PlanModeState::Planning | crate::plan::PlanModeState::Review => {
            let config = crate::plan::PlanAgentConfig::default_config();
            let prompt = if plan_state == crate::plan::PlanModeState::Review {
                if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                    format!("# Plan Review\n\nThe following plan has been submitted and is awaiting user approval:\n\n{}", plan_content)
                } else {
                    crate::plan::PLAN_MODE_SYSTEM_PROMPT.to_string()
                }
            } else {
                crate::plan::PLAN_MODE_SYSTEM_PROMPT.to_string()
            };
            (
                Some(prompt),
                Some(crate::agent::PlanAgentMode::PlanAgent {
                    allowed_tools: config.allowed_tools,
                    ask_tools: config.ask_tools,
                }),
                Some(config.plan_mode_allow_paths),
            )
        }
        crate::plan::PlanModeState::Executing | crate::plan::PlanModeState::Paused => {
            let mode = crate::agent::PlanAgentMode::BuildAgent {
                extra_tools: crate::plan::BUILD_AGENT_EXTRA_TOOLS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            };
            let ctx = if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                let prefix = if plan_state == crate::plan::PlanModeState::Paused {
                    let paused_step = crate::plan::get_plan_meta(&sid)
                        .await
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
                Some(format!("{}{}", prefix, plan_content))
            } else {
                None
            };
            (ctx, Some(mode), None)
        }
        crate::plan::PlanModeState::Completed => {
            let ctx = if let Ok(Some(plan_content)) = crate::plan::load_plan_file(&sid) {
                let step_summary = if let Some(meta) = crate::plan::get_plan_meta(&sid).await {
                    let completed = meta
                        .steps
                        .iter()
                        .filter(|s| s.status == crate::plan::PlanStepStatus::Completed)
                        .count();
                    let failed = meta
                        .steps
                        .iter()
                        .filter(|s| s.status == crate::plan::PlanStepStatus::Failed)
                        .count();
                    let skipped = meta
                        .steps
                        .iter()
                        .filter(|s| s.status == crate::plan::PlanStepStatus::Skipped)
                        .count();
                    format!("\n\n## Statistics\n- Completed: {}\n- Failed: {}\n- Skipped: {}\n- Total: {}\n",
                        completed, failed, skipped, meta.steps.len())
                } else {
                    String::new()
                };
                Some(format!(
                    "{}{}{}",
                    crate::plan::PLAN_COMPLETED_SYSTEM_PROMPT,
                    plan_content,
                    step_summary
                ))
            } else {
                None
            };
            (ctx, None, None)
        }
        crate::plan::PlanModeState::Off => (None, None, None),
    };

    // ── Build ChatEngineParams and delegate to shared engine ──
    let (providers_snapshot, compact_config) = {
        let store = state.provider_store.lock().await;
        (store.providers.clone(), store.compact.clone())
    };
    let codex_token_snapshot = state.codex_token.lock().await.clone();

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: sid.clone(),
        agent_id: current_agent_id.clone(),
        message: message.clone(),
        session_db: db.clone(),
        model_chain,
        providers: providers_snapshot,
        codex_token: codex_token_snapshot,
        resolved_temperature,
        web_search_enabled,
        notification_enabled,
        image_gen_config,
        canvas_enabled,
        compact_config,
        extra_system_context: plan_extra_context,
        reasoning_effort: Some(effort.clone()),
        cancel: cancel.clone(),
        plan_agent_mode,
        plan_mode_allow_paths: plan_allow_paths,
        event_sink: Arc::new(crate::chat_engine::ChannelSink {
            channel: on_event.clone(),
        }),
    };

    match crate::chat_engine::run_chat_engine(engine_params).await {
        Ok(result) => {
            // Relay to IM channel if this session is linked to one
            crate::chat_engine::relay_to_channel(&sid, &result.response).await;

            // Plan Mode: auto-detect plan content from LLM output
            if plan_state == crate::plan::PlanModeState::Planning && !result.response.is_empty() {
                let steps = crate::plan::parse_plan_steps(&result.response);
                if steps.len() >= 2 {
                    let _ = crate::plan::save_plan_file(&sid, &result.response);
                    crate::plan::update_plan_steps(&sid, steps.clone()).await;
                    if let Some(app_handle) = crate::get_app_handle() {
                        use tauri::Emitter;
                        let _ = app_handle.emit(
                            "plan_content_updated",
                            serde_json::json!({
                                "sessionId": &sid,
                                "stepCount": steps.len(),
                                "content": &result.response,
                            }),
                        );
                    }
                }
            }

            // Update the active agent instance for conversation continuity (UI chat only)
            if let Some(agent) = result.agent {
                *state.agent.lock().await = Some(agent);
            }

            Ok(result.response)
        }
        Err(e) => {
            // Persist any in-memory compaction before returning error
            if let Some(ref agent) = *state.agent.lock().await {
                crate::chat_engine::save_agent_context(&db, &sid, agent);
            }
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn stop_chat(state: State<'_, AppState>) -> Result<(), String> {
    state.chat_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

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

// ── Command Approval ──────────────────────────────────────────────

#[tauri::command]
pub async fn respond_to_approval(request_id: String, response: String) -> Result<(), String> {
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

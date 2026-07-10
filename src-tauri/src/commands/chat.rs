use crate::agent::Attachment;
use crate::agent_loader;
use crate::chat_engine::EventSink;
use crate::commands::CmdError;
use crate::provider::{self, ActiveModel};
use crate::session::{self, SessionDB};
use crate::tools;
use crate::truncate_utf8;
use crate::AppState;
use ha_core::{app_error, app_info, app_warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;

/// Tauri-specific EventSink — wraps `tauri::ipc::Channel<String>`.
pub(crate) struct ChannelSink {
    pub channel: tauri::ipc::Channel<String>,
}

impl EventSink for ChannelSink {
    fn send(&self, event: &str) {
        let _ = self.channel.send(event.to_string());
    }
}

fn broadcast_turn_end(
    session_id: &str,
    turn_id: &str,
    status: session::ChatTurnStatus,
    interrupt_reason: Option<session::ChatTurnInterruptReason>,
    error: Option<&str>,
) {
    ha_core::chat_engine::stream_broadcast::broadcast_stream_end(
        session_id,
        None,
        Some(turn_id),
        Some(status),
        interrupt_reason,
        error,
    );
}

fn finish_turn_once_and_broadcast(
    db: &SessionDB,
    session_id: &str,
    turn_id: &str,
    status: session::ChatTurnStatus,
    interrupt_reason: Option<session::ChatTurnInterruptReason>,
    error: Option<&str>,
    assistant_message_id: Option<i64>,
) {
    let _ = db.finish_chat_turn_once(
        turn_id,
        status,
        interrupt_reason,
        error,
        assistant_message_id,
    );
    broadcast_turn_end(session_id, turn_id, status, interrupt_reason, error);
}

/// Save an attachment file to disk. Uses a temp directory when session_id is empty.
/// Returns the absolute path to the saved file.
#[tauri::command]
pub async fn save_attachment(
    session_id: Option<String>,
    file_name: String,
    _mime_type: String,
    data: Vec<u8>,
) -> Result<String, CmdError> {
    ha_core::attachments::save_attachment_bytes(session_id.as_deref(), &file_name, &data)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn queue_turn_user_message(
    request_id: Option<String>,
    message: String,
    attachments: Vec<Attachment>,
    session_id: String,
    turn_id: String,
    display_text: Option<String>,
    is_plan_trigger: Option<bool>,
    plan_comment: Option<serde_json::Value>,
) -> Result<ha_core::chat_engine::turn_injection::QueueTurnUserMessageResult, CmdError> {
    Ok(ha_core::chat_engine::turn_injection::enqueue(
        ha_core::chat_engine::turn_injection::QueueTurnUserMessageArgs {
            request_id,
            session_id,
            turn_id,
            message,
            display_text,
            attachments,
            is_plan_trigger: is_plan_trigger.unwrap_or(false),
            plan_comment,
        },
    ))
}

#[tauri::command]
pub async fn cancel_queued_turn_user_message(
    session_id: String,
    turn_id: String,
    request_id: String,
) -> Result<ha_core::chat_engine::turn_injection::CancelQueuedTurnMessageResult, CmdError> {
    Ok(ha_core::chat_engine::turn_injection::cancel(
        &session_id,
        &turn_id,
        &request_id,
    ))
}

#[tauri::command]
pub async fn chat(
    message: String,
    mut attachments: Vec<Attachment>,
    session_id: Option<String>,
    incognito: Option<bool>,
    model_override: Option<String>,
    agent_id: Option<String>,
    permission_mode: Option<ha_core::permission::SessionMode>,
    sandbox_mode: Option<ha_core::permission::SandboxMode>,
    plan_mode: Option<String>,
    temperature_override: Option<f64>,
    reasoning_effort: Option<String>,
    // When set, DB stores `display_text` as the user message while `message` is still
    // fed to the LLM (slash-skill passThrough uses this).
    display_text: Option<String>,
    // When true, the persisted user row is tagged with
    // `attachments_meta = {"plan_trigger": true}` so the UI can render it as a
    // system chip instead of a regular user bubble (Plan Mode approve/resume).
    is_plan_trigger: Option<bool>,
    // Structured payload for plan inline-comment messages — stamped into
    // `attachments_meta = {"plan_comment": {selectedText, comment}}`. The
    // desktop GUI reads this back to render PlanCommentBubble; IM channels
    // ignore it (they consume `display_text` instead). Mutually exclusive
    // with `is_plan_trigger` (a comment is not a trigger), `is_plan_trigger`
    // wins if both are set.
    plan_comment: Option<serde_json::Value>,
    // Draft working dir picked before the session was materialized. Only honored
    // when this call also creates the session — applies via the same
    // `update_session_working_dir` validation as the explicit setter command.
    working_dir: Option<String>,
    // Composer-staged KB attaches. Only honored when this call also creates the
    // session (mirrors `working_dir`); applied before the engine runs so the
    // first turn already sees the access. No-op for incognito.
    kb_attachments: Option<Vec<ha_core::knowledge::types::KbAttachInput>>,
    // Tool-visibility scope (`"knowledge"`). Set by the knowledge-space sidebar
    // chat to trim the injected tool set; `None` for normal chats.
    tool_scope: Option<String>,
    // Knowledge-space sidebar chat: the note open when the conversation started.
    // Only honored on the auto-create branch (mirrors `working_dir` /
    // `kb_attachments`) — promotes the new session into a KB chat thread.
    kb_anchor_note: Option<String>,
    // Lazy project binding: when the frontend opens a project draft (no session
    // yet), the first message carries the project id here so the auto-create
    // branch materializes the session inside the project. Ignored when
    // `session_id` is set (existing sessions keep their project). Mutually
    // exclusive with incognito (coerced in `create_session_with_project`).
    project_id: Option<String>,
    on_event: tauri::ipc::Channel<String>,
    state: State<'_, AppState>,
) -> Result<String, CmdError> {
    // Capture optional per-session modes — applied below once we have a session id.
    let permission_mode_pending = permission_mode;
    let sandbox_mode_pending = sandbox_mode;

    let db = state.session_db.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let logger = state.logger.clone();
    // NOTE: _chat_session_guard is set later after session_id is resolved

    // Normalize the lazy project binding once: trim and treat empty/whitespace as
    // "no project" so a blank `project_id` neither resolves a bogus project agent
    // nor persists a non-matching `project_id` (which would orphan the session and
    // wrongly coerce incognito off). Used for both agent resolution and create.
    let project_id = project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Resolve or create session — prefer explicit agent_id from frontend
    let current_agent_id = match agent_id {
        Some(id) => {
            // Sync backend state so other code paths see the correct agent
            *state.current_agent_id.lock().await = id.clone();
            id
        }
        // No explicit agent. For a lazy project draft (no session yet) resolve
        // via the project's default-agent chain so the materialized session
        // matches what `create_session_cmd` / the resolver would pick;
        // otherwise fall back to the last-used agent in global state.
        None => match project_id.as_deref() {
            Some(pid) => {
                let project_db = state.project_db.clone();
                let pid = pid.to_string();
                let project =
                    ha_core::blocking::run_blocking(move || project_db.get(&pid).ok().flatten())
                        .await;
                ha_core::agent::resolver::resolve_default_agent_id(project.as_ref(), None)
            }
            None => state.current_agent_id.lock().await.clone(),
        },
    };
    let mut new_session_created: Option<String> = None;
    let sid = match session_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            // Auto-create a new session; emit session_created after auto_title is set.
            // `project_id` binds the session to a project on this lazy-create
            // branch (None for plain chats); incognito is coerced off when set.
            let meta = {
                let agent_id = current_agent_id.clone();
                let project_id = project_id.clone();
                db.run(move |db| {
                    db.create_session_with_project(&agent_id, project_id.as_deref(), incognito)
                })
                .await?
            };
            new_session_created = Some(meta.id.clone());
            meta.id
        }
    };
    let agent_def = agent_loader::load_agent(&current_agent_id).ok();
    let agent_default_effort = agent_def
        .as_ref()
        .and_then(|def| def.config.model.reasoning_effort.clone());

    let requested_effort = reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_string);
    let session_effort = {
        let sid = sid.clone();
        db.run(move |db| db.get_session(&sid)).await?
    }
    .and_then(|meta| meta.reasoning_effort);
    let global_effort = state.reasoning_effort.lock().await.clone();
    let effort = requested_effort
        .or(session_effort)
        .or(agent_default_effort)
        .unwrap_or(global_effort);
    if !ha_core::agent::is_valid_reasoning_effort(&effort) {
        return Err(CmdError::msg(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort,
            ha_core::agent::VALID_REASONING_EFFORTS
        )));
    }
    *state.reasoning_effort.lock().await = effort.clone();
    {
        let sid = sid.clone();
        let effort = effort.clone();
        db.run(move |db| db.update_session_reasoning_effort(&sid, Some(&effort)))
            .await?;
    }
    // Apply draft working dir picked before the session existed. Only honored on
    // the auto-create branch — explicit-session callers must use
    // `set_session_working_dir` to change it. Validation errors are surfaced so
    // an invalid path doesn't silently get dropped.
    // Persist per-session permission mode if the caller supplied one.
    if permission_mode_pending.is_some() || sandbox_mode_pending.is_some() {
        let sid = sid.clone();
        db.run(move |db| -> anyhow::Result<()> {
            if let Some(mode) = permission_mode_pending {
                db.update_session_permission_mode(&sid, mode)?;
            }
            if let Some(mode) = sandbox_mode_pending {
                db.update_session_sandbox_mode(&sid, mode)?;
            }
            Ok(())
        })
        .await?;
    }

    if new_session_created.is_some() {
        if let Some(wd) = working_dir.as_ref().filter(|s| !s.trim().is_empty()) {
            {
                let sid = sid.clone();
                let wd = wd.clone();
                db.run(move |db| db.update_session_working_dir(&sid, Some(wd)))
                    .await?;
            }
            app_info!(
                "session",
                "chat",
                "Applied draft working_dir on new session: session={} dir={}",
                sid,
                wd
            );
        }
        if let Some(attaches) = kb_attachments.as_ref() {
            ha_core::knowledge::service::apply_draft_attachments(
                &sid,
                incognito.unwrap_or(false),
                attaches,
            );
        }
    }

    let turn_id = uuid::Uuid::new_v4().to_string();
    let _active_turn_guard = crate::chat_engine::active_turn::try_acquire(
        &sid,
        crate::chat_engine::stream_seq::ChatSource::Desktop,
        turn_id.clone(),
        cancel.clone(),
    )?;

    // Mark this session as active — cancels any running subagent injection and blocks new ones
    let _chat_session_guard = crate::subagent::ChatSessionGuard::new(&sid);

    // Prefer display_text for DB/title, fall back to the LLM-bound message.
    let raw_prompt = ha_core::non_empty_trim_or(display_text.as_deref(), &message);

    // Preflight chokepoint runs BEFORE any side effects (attachments dir creation,
    // base64 image write, temp file move) so a `UserPromptSubmit` block doesn't
    // leave orphan attachment files on disk. Reordered after the adversarial
    // review caught this leak: blocked-prompt semantics were "no user message
    // persisted", but the attachment IO had already touched disk. The preflight
    // only consumes `raw_prompt` / `session_id` / `agent_id`, so it doesn't need
    // any attachment metadata — moving it up is purely a side-effect deferral.
    let effective_prompt = match ha_core::agent::preflight::user_prompt_preflight(
        ha_core::agent::preflight::PreflightArgs {
            session_id: &sid,
            agent_id: Some(current_agent_id.as_str()),
            raw_prompt,
        },
    )
    .await
    {
        ha_core::agent::preflight::PreflightOutcome::Proceed { effective_prompt } => {
            effective_prompt
        }
        ha_core::agent::preflight::PreflightOutcome::Block { reason } => {
            // A UserPromptSubmit hook blocked the prompt: record a UI-only event
            // marker (visible in history but excluded from LLM context) and
            // surface it. The prompt is neither persisted as a user message nor
            // run as a turn — and crucially, no attachment file has been
            // written yet (we're upstream of all attachment IO).
            let notice = format!("🚫 {reason}");
            // KB sidebar lazy-create: a blocked first message must leave NO
            // session behind — neither a hidden `kind=Knowledge` zombie nor a
            // stray `kind=regular` row polluting the main list / picker / FTS.
            // Drop the freshly auto-created session; the notice still reaches the
            // panel via the transient event channel (no `session_created`, so the
            // frontend never registers it).
            if new_session_created.is_some() && tool_scope.as_deref() == Some("knowledge") {
                let _ = {
                    let sid = sid.clone();
                    db.run(move |db| db.delete_session(&sid)).await
                };
                let _ = on_event
                    .send(serde_json::json!({ "type": "text", "text": notice }).to_string());
                return Ok(notice);
            }
            // If this preflight ran against a freshly-auto-created session,
            // emit `session_created` BEFORE the block notice so the frontend
            // can register the new session and route the notice to it.
            // Without this the empty session stays orphaned in the DB (no
            // user message, no title, no sidebar entry) and the block text
            // event has nowhere to dock — symmetric with the HTTP path,
            // which returns `session_id` via `ChatResponse.blocked_reason`.
            // Title is derived from the raw prompt so the user can find the
            // session in the sidebar; ensure_first_message_title is the same
            // helper the post-stream path uses, so the title shape stays
            // consistent across blocked-first-message and normal flows.
            if let Some(ref new_sid) = new_session_created {
                let _ = {
                    let new_sid = new_sid.clone();
                    let prompt = raw_prompt.to_string();
                    db.run(move |db| {
                        ha_core::session::ensure_first_message_title(db, &new_sid, &prompt)
                    })
                    .await
                };
                let event = serde_json::json!({
                    "type": "session_created",
                    "session_id": new_sid,
                });
                if let Ok(json_str) = serde_json::to_string(&event) {
                    let _ = on_event.send(json_str);
                }
            }
            let _ = {
                let sid = sid.clone();
                let notice = notice.clone();
                db.run(move |db| db.append_message(&sid, &session::NewMessage::event(&notice)))
                    .await
            };
            let _ =
                on_event.send(serde_json::json!({ "type": "text", "text": notice }).to_string());
            return Ok(notice);
        }
    };

    // KB sidebar chat: promote the freshly-created session into a knowledge
    // thread (hidden from the main list; bound to the KB + anchor note) now that
    // preflight has passed. Doing this in the auto-create block above left a
    // hidden `kind=Knowledge` zombie + thread row whenever a UserPromptSubmit
    // hook blocked the very first message.
    if new_session_created.is_some() && tool_scope.as_deref() == Some("knowledge") {
        if let Some(kb_id) = kb_attachments
            .as_ref()
            .and_then(|a| a.first())
            .map(|a| a.kb_id.clone())
        {
            ha_core::knowledge::service::mark_session_as_kb_thread(
                &sid,
                &kb_id,
                kb_anchor_note.as_deref(),
            );
        }
    }

    let attachments_meta = {
        let sid = sid.clone();
        let mut moved = std::mem::take(&mut attachments);
        let (meta, persisted) = ha_core::blocking::run_blocking(move || {
            let meta = ha_core::attachments::persist_chat_user_attachments_meta(&sid, &mut moved)?;
            anyhow::Ok((meta, moved))
        })
        .await?;
        attachments = persisted;
        meta
    };

    // Save user message to DB
    let mut user_msg = session::NewMessage::user(&effective_prompt)
        .with_source(ha_core::chat_engine::ChatSource::Desktop);
    user_msg.attachments_meta = session::build_chat_user_attachments_meta(
        is_plan_trigger.unwrap_or(false),
        plan_comment.as_ref(),
        attachments_meta,
    );
    let _user_message_id = {
        let sid = sid.clone();
        let turn_id = turn_id.clone();
        db.run(move |db| -> anyhow::Result<Option<i64>> {
            let user_message_id = db.append_message(&sid, &user_msg).ok();
            db.create_chat_turn_with_id(
                &turn_id,
                &sid,
                ha_core::chat_engine::ChatSource::Desktop.as_str(),
                None,
                user_message_id,
            )?;
            Ok(user_message_id)
        })
        .await?
    };

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

    // Auto-generate fallback title from first user message if session has no title.
    // Prefer the displayed text so titles read naturally ("/drawio ..." rather than the expanded form).
    let _ = {
        let sid = sid.clone();
        let prompt = effective_prompt.clone();
        db.run(move |db| session::ensure_first_message_title(db, &sid, &prompt))
            .await
    };

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
    let turn_event = serde_json::json!({
        "type": "turn_started",
        "session_id": &sid,
        "turn_id": &turn_id,
    });
    if let Ok(json_str) = serde_json::to_string(&turn_event) {
        let _ = on_event.send(json_str);
    }

    // Resolve model chain from current agent config. The legacy
    // `notify_on_complete` per-agent override is consumed inside ha-core
    // (`AssistantAgent::agent_caps`), where it folds into
    // `capability_toggles.send_notification` so the dispatcher gates the
    // tool consistently — no need to thread it through here.
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    // One lock-free config snapshot for the whole request.
    let cfg = ha_core::config::cached_config();

    // Resolve temperature: session > agent > global
    let resolved_temperature: Option<f64> = {
        let global_temp = cfg.temperature;
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
            let applied = crate::plan::set_plan_state(&sid, ps).await;
            if applied {
                let _ = {
                    let sid = sid.clone();
                    db.run(move |db| db.update_session_plan_mode(&sid, ps))
                        .await
                };
                ps
            } else {
                let current = crate::plan::get_plan_state(&sid).await;
                if current != crate::plan::PlanModeState::Off {
                    let _ = {
                        let sid = sid.clone();
                        db.run(move |db| db.update_session_plan_mode(&sid, current))
                            .await
                    };
                }
                current
            }
        } else {
            crate::plan::get_plan_state(&sid).await
        }
    } else {
        crate::plan::get_plan_state(&sid).await
    };

    // ── Plan Sub-Agent: optionally dispatch Planning to an isolated sub-agent ──
    // When plan_subagent=true, keeps the main agent's context clean for execution.
    // When plan_subagent=false (default), planning runs inline in the main agent.
    if early_plan_state == crate::plan::PlanModeState::Planning {
        let use_subagent = cfg.plan_subagent;

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
                finish_turn_once_and_broadcast(
                    &db,
                    &sid,
                    &turn_id,
                    session::ChatTurnStatus::Completed,
                    None,
                    None,
                    None,
                );
                return Ok("Message forwarded to planning agent.".to_string());
            }

            // First message in Planning state → spawn plan sub-agent
            let recent_summary = build_recent_context_summary(&db, &sid).await;
            let cancel_registry = crate::get_subagent_cancels()
                .cloned()
                .ok_or_else(|| CmdError::msg("Sub-agent cancel registry not initialized"))?;
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
                    finish_turn_once_and_broadcast(
                        &db,
                        &sid,
                        &turn_id,
                        session::ChatTurnStatus::Completed,
                        None,
                        None,
                        None,
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

    // Plan Mode's persisted model preference remains the highest-priority
    // candidate during Planning. Unlike a per-turn override, a stale Plan or
    // Session preference is allowed to fall through to Agent/global defaults.
    let plan_model_preference = if early_plan_state == crate::plan::PlanModeState::Planning {
        agent_model_config.plan_model.as_deref()
    } else {
        None
    };

    // Session-scoped model pin trumps both agent.primary and config.active_model
    // when neither Plan Mode nor an explicit per-turn override won. This is how
    // set_session_model surfaces its effect on subsequent turns.
    let session_pinned_model: Option<String> = if plan_model_preference.is_none()
        && model_override.is_none()
    {
        let sid2 = sid.clone();
        db.run(move |db| db.get_session(&sid2))
            .await
            .ok()
            .flatten()
            .and_then(|meta| match (meta.provider_id, meta.model_id) {
                (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => {
                    Some(format!("{}::{}", p, m))
                }
                _ => None,
            })
    } else {
        None
    };

    // Explicit current-turn overrides are strict: if the requested model was
    // removed or its Provider is disabled, surface the error instead of
    // silently switching Provider. Plan Mode still wins when configured.
    if plan_model_preference.is_none() {
        if let Some(override_str) = model_override.as_deref() {
            let override_is_available = provider::parse_model_ref(override_str)
                .is_some_and(|model| provider::model_ref_is_available(&cfg.providers, &model));
            if !override_is_available {
                let err = format!(
                    "Selected model override is unavailable: {override_str}. Please choose an enabled provider and model."
                );
                let partial = ha_core::chat_engine::finalize::PartialMeta {
                    user_message: Some(message.clone()),
                    turn_id: Some(turn_id.clone()),
                    ..Default::default()
                };
                let outcome = ha_core::chat_engine::finalize::finalize_turn_context_blocking(
                    &db,
                    &sid,
                    ha_core::chat_engine::finalize::TerminationReason::Other {
                        message: err.clone(),
                    },
                    partial,
                    ha_core::chat_engine::ChatSource::Desktop,
                );
                broadcast_turn_end(
                    &sid,
                    &turn_id,
                    outcome
                        .turn_status
                        .unwrap_or(session::ChatTurnStatus::Failed),
                    outcome.interrupt_reason,
                    Some(&err),
                );
                return Err(CmdError::msg(err));
            }
        }
    }

    let preferred_model = plan_model_preference
        .or(model_override.as_deref())
        .or(session_pinned_model.as_deref());
    let (primary, fallbacks) = provider::resolve_model_chain_with_preferred(
        preferred_model,
        &agent_model_config,
        &cfg,
    );

    // Build ordered model chain: [primary, ...fallbacks]
    let model_chain: Vec<ActiveModel> = primary.into_iter().chain(fallbacks).collect();

    // Log model chain resolution
    logger.log("info", "agent", "lib::chat::model_chain",
        &format!("Model chain resolved: {} models", model_chain.len()),
        Some(serde_json::json!({
            "chain": model_chain.iter().map(|m| format!("{}::{}", m.provider_id, m.model_id)).collect::<Vec<_>>(),
            "total": model_chain.len(),
        }).to_string()),
        Some(sid.clone()), Some(current_agent_id.clone()));

    if model_chain.is_empty() {
        let err = "No model configured. Please add a provider and set an active model.".to_string();
        let partial = ha_core::chat_engine::finalize::PartialMeta {
            user_message: Some(message.clone()),
            turn_id: Some(turn_id.clone()),
            ..Default::default()
        };
        let outcome = ha_core::chat_engine::finalize::finalize_turn_context_blocking(
            &db,
            &sid,
            ha_core::chat_engine::finalize::TerminationReason::NoProfileAvailable,
            partial,
            ha_core::chat_engine::ChatSource::Desktop,
        );
        broadcast_turn_end(
            &sid,
            &turn_id,
            outcome
                .turn_status
                .unwrap_or(session::ChatTurnStatus::Failed),
            outcome.interrupt_reason,
            Some(&err),
        );
        return Err(CmdError::msg(err));
    }

    // ── Build ChatEngineParams and delegate to shared engine ──
    // Plan-mode resolution (mode + allow paths + system-prompt segment)
    // happens inside chat_engine via `resolve_plan_context_for_session`,
    // unified across Tauri / HTTP / channel / cron entry points. The
    // streaming loop's mid-turn probe handles `enter_plan_mode` flips.
    let (providers_snapshot, compact_config) = (cfg.providers.clone(), cfg.compact.clone());
    let codex_token_snapshot = state.codex_token.lock().await.clone();

    let engine_params = crate::chat_engine::ChatEngineParams {
        session_id: sid.clone(),
        agent_id: current_agent_id.clone(),
        turn_id: Some(turn_id.clone()),
        message: message.clone(),
        display_text: display_text.clone(),
        attachments,
        session_db: db.clone(),
        model_chain,
        providers: providers_snapshot,
        codex_token: codex_token_snapshot,
        resolved_temperature,
        compact_config,
        extra_system_context: None,
        reasoning_effort: Some(effort.clone()),
        cancel: cancel.clone(),
        plan_context_override: None,
        skill_allowed_tools: Vec::new(),
        denied_tools: Vec::new(),
        tool_scope: ha_core::tools::ToolScope::from_str_opt(tool_scope.as_deref()),
        subagent_depth: 0,
        steer_run_id: None,
        auto_approve_tools: false,
        follow_global_reasoning_effort: true,
        post_turn_effects: true,
        abort_on_cancel: false,
        persist_final_error_event: true,
        source: crate::chat_engine::stream_seq::ChatSource::Desktop,
        origin_source: None,
        // Desktop owner turn — KB access via attach, not the IM opt-in gate.
        channel_kb_context: None,
        event_sink: Arc::new(ChannelSink {
            channel: on_event.clone(),
        }),
    };

    match crate::chat_engine::run_chat_engine(engine_params).await {
        Ok(result) => {
            if let Some(agent) = result.agent {
                *state.agent.lock().await = Some(agent);
            }

            Ok(result.response)
        }
        Err(e) => Err(CmdError::msg(e)),
    }
}

#[tauri::command]
pub async fn stop_chat(
    session_id: Option<String>,
    turn_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    let mut stopped = false;
    let mut watchdog_turns = Vec::new();
    if let Some(sid) = session_id.as_deref() {
        if let Some(active) = crate::chat_engine::active_turn::current(sid) {
            let matches_turn = turn_id
                .as_deref()
                .map(|id| id == active.turn_id)
                .unwrap_or(true);
            if matches_turn {
                active.cancel.store(true, Ordering::SeqCst);
                let _ = {
                    let turn_id = active.turn_id.clone();
                    state
                        .session_db
                        .run(move |db| {
                            db.mark_chat_turn_cancelling(
                                &turn_id,
                                session::ChatTurnInterruptReason::UserStop,
                            )
                        })
                        .await
                };
                ha_core::chat_engine::stream_broadcast::broadcast_turn_status(
                    sid,
                    &active.turn_id,
                    session::ChatTurnStatus::Cancelling,
                    Some(session::ChatTurnInterruptReason::UserStop),
                );
                watchdog_turns.push((sid.to_string(), active.turn_id.clone(), active.source));
                stopped = true;
            } else {
                app_info!(
                    "chat",
                    "stop_chat",
                    "Ignoring stale stop for session {} turn {:?}; active turn is {}",
                    sid,
                    turn_id,
                    active.turn_id
                );
            }
        }
    } else {
        // Legacy fallback for callers that cannot target a session. Keep the
        // old global flag, but all new UI paths pass a session id.
        state.chat_cancel.store(true, Ordering::SeqCst);
        let mut cancelling_turn_ids = Vec::new();
        for active in crate::chat_engine::active_turn::all_current() {
            active.cancel.store(true, Ordering::SeqCst);
            ha_core::chat_engine::stream_broadcast::broadcast_turn_status(
                &active.session_id,
                &active.turn_id,
                session::ChatTurnStatus::Cancelling,
                Some(session::ChatTurnInterruptReason::UserStop),
            );
            cancelling_turn_ids.push(active.turn_id.clone());
            watchdog_turns.push((
                active.session_id.clone(),
                active.turn_id.clone(),
                active.source,
            ));
        }
        // One blocking-pool hop for all the DB marks (a stalled DB otherwise
        // multiplies into one queued task per turn).
        if !cancelling_turn_ids.is_empty() {
            let _ = state
                .session_db
                .run(move |db| {
                    for turn_id in &cancelling_turn_ids {
                        let _ = db.mark_chat_turn_cancelling(
                            turn_id,
                            session::ChatTurnInterruptReason::UserStop,
                        );
                    }
                })
                .await;
        }
        stopped = true;
    }
    let runtime_scope = stopped.then_some(session_id.as_deref()).flatten();
    let runtime_cancellations = if stopped || session_id.is_none() {
        ha_core::runtime_tasks::cancel_runtime_tasks_for_session(runtime_scope).await
    } else {
        Ok(Vec::new())
    };
    match runtime_cancellations {
        Ok(results) => {
            app_info!(
                "chat",
                "stop_chat",
                "Stop chat requested; stopped={} runtime cancellations attempted: {}",
                stopped,
                results.len()
            );
        }
        Err(e) => {
            app_warn!(
                "chat",
                "stop_chat",
                "Stop chat runtime cancellation failed: {}",
                e
            );
        }
    }
    for (sid, turn_id, source) in watchdog_turns {
        crate::chat_engine::spawn_user_stop_watchdog(
            state.session_db.clone(),
            sid,
            turn_id,
            source,
        );
    }
    Ok(())
}

/// Persist the per-session permission mode (`default` / `smart` / `yolo`)
/// to the session row so the chat title bar's switcher is restored on revisit.
#[tauri::command]
pub async fn set_permission_mode(
    session_id: String,
    mode: ha_core::permission::SessionMode,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    if session_id.is_empty() {
        return Err(CmdError::from(anyhow::anyhow!("session_id required")));
    }
    state
        .session_db
        .update_session_permission_mode(&session_id, mode)?;
    Ok(())
}

/// Persist the per-session sandbox mode (`off` / `standard` / `isolated` /
/// `workspace` / `trusted`) to the session row.
#[tauri::command]
pub async fn set_sandbox_mode(
    session_id: String,
    mode: ha_core::permission::SandboxMode,
    state: State<'_, AppState>,
) -> Result<(), CmdError> {
    if session_id.is_empty() {
        return Err(CmdError::from(anyhow::anyhow!("session_id required")));
    }
    state
        .session_db
        .update_session_sandbox_mode(&session_id, mode)?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit(
            "sandbox:mode_changed",
            serde_json::json!({
                "sessionId": session_id,
                "mode": mode.as_str(),
            }),
        );
    }
    Ok(())
}

/// Build a compact summary of recent conversation for passing to a plan sub-agent.
/// Returns up to the last N messages as a condensed text summary.
async fn build_recent_context_summary(db: &Arc<SessionDB>, session_id: &str) -> String {
    const MAX_MESSAGES: u32 = 10;
    const MAX_CHARS: usize = 4000;

    // Load the latest messages (excluding the just-appended user message which is the task)
    let (messages, _total, _has_more) =
        match db.load_session_messages_latest(session_id, MAX_MESSAGES + 1) {
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
pub async fn respond_to_approval(request_id: String, response: String) -> Result<(), CmdError> {
    let approval_response = match response.as_str() {
        "allow_once" => tools::ApprovalResponse::AllowOnce,
        "allow_always" => tools::ApprovalResponse::AllowAlways,
        "deny" => tools::ApprovalResponse::Deny,
        _ => {
            return Err(CmdError::msg(format!(
                "Invalid approval response: {}",
                response
            )))
        }
    };
    tools::submit_approval_response(
        &request_id,
        approval_response,
        tools::ApprovalResolutionSource::Gui,
    )
    .await
    .map_err(|e| CmdError::msg(e.to_string()))
}

// ── System Prompt ────────────────────────────────────────────────

/// Return the assembled system prompt for the current agent + model.
///
/// When `session_id` is provided and the session is attached to a project,
/// the returned prompt includes the "# Current Project" + "# Project Files"
/// sections and project-scoped memories — matching what the chat loop
/// actually ships on the next turn.
#[tauri::command]
pub async fn get_system_prompt(
    agent_id: Option<String>,
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, CmdError> {
    let aid = match agent_id {
        Some(id) => id,
        None => state.current_agent_id.lock().await.clone(),
    };

    // Resolve model and provider name from active model
    let (model, provider) = {
        let store = ha_core::config::cached_config();
        if let Some(ref active) = store.active_model {
            let prov = store.providers.iter().find(|p| p.id == active.provider_id);
            let model_id = active.model_id.clone();
            let provider_name = prov
                .map(|p| p.api_type.display_name().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            (model_id, provider_name)
        } else {
            ("unknown".to_string(), "Unknown".to_string())
        }
    };

    Ok(crate::agent::build_system_prompt_with_session(
        &aid,
        &model,
        &provider,
        session_id.as_deref(),
    ))
}

// ── Tools Info Commands ───────────────────────────────────────────

#[tauri::command]
pub async fn list_builtin_tools() -> Result<Vec<serde_json::Value>, CmdError> {
    let cfg = ha_core::config::cached_config();
    Ok(tools::dispatch::all_dispatchable_tools()
        .iter()
        .map(|t| t.to_api_metadata(&cfg))
        .collect())
}

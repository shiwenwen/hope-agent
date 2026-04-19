use crate::channel::db::ChannelDB;

/// Outcome of dispatching a slash command from an IM channel message.
pub(super) enum ChannelSlashOutcome {
    /// Send `content` as a direct reply; no LLM call needed.
    /// `new_session_id` is set when the command created a fresh session that should
    /// replace the current channel → session mapping.
    /// `buttons` provides optional inline keyboard buttons for IM channels that support them.
    Reply {
        content: String,
        new_session_id: Option<String>,
        buttons: Vec<Vec<crate::channel::types::InlineButton>>,
    },
    /// The command (e.g. a skill invocation) asks to pass a transformed message
    /// through to the LLM instead of the original "/" text.
    PassThrough(String),
}

/// Dispatch a slash command received via an IM channel.
///
/// Returns a `ChannelSlashOutcome` describing what to do next:
///   - `Reply`       → send the content as a direct reply and skip the LLM.
///   - `PassThrough` → forward the (possibly rewritten) message to the LLM.
pub(super) async fn dispatch_slash_for_channel(
    channel_db: &ChannelDB,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    session_id: &str,
    agent_id: &str,
    text: &str,
) -> Result<ChannelSlashOutcome, anyhow::Error> {
    use crate::slash_commands::{handlers, parser};

    let (name, args) = parser::parse(text).map_err(|e| anyhow::anyhow!(e))?;

    // Obtain a reference to the global AppState so we can reuse the shared handlers.
    let app_state_arc = crate::globals::get_app_state()
        .ok_or_else(|| anyhow::anyhow!("AppState not initialized"))?;
    let app_state: &crate::globals::AppState = app_state_arc;

    // For commands with fixed arg_options and no args provided, return inline buttons
    // so IM channel users (e.g. Telegram) can tap to select an option.
    // Checks both built-in commands AND dynamic skill commands.
    if args.trim().is_empty() {
        use crate::slash_commands::registry;

        // First check built-in commands
        let commands = registry::all_commands();
        let mut options_found: Option<Vec<String>> = commands
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.arg_options.clone());

        // If not found in built-in, check dynamic skill commands
        if options_found.is_none() {
            let store = app_state.config.lock().await;
            let skills = crate::skills::get_invocable_skills(
                &store.extra_skills_dirs,
                &store.disabled_skills,
            );
            drop(store);
            options_found = skills
                .into_iter()
                .find(|s| crate::skills::normalize_skill_command_name(&s.name) == name)
                .and_then(|s| s.command_arg_options);
        }

        if let Some(options) = options_found {
            let buttons: Vec<Vec<crate::channel::types::InlineButton>> = options
                .iter()
                .map(|opt| {
                    vec![crate::channel::types::InlineButton {
                        text: opt.clone(),
                        callback_data: Some(format!("slash:{} {}", name, opt)),
                        url: None,
                    }]
                })
                .collect();
            return Ok(ChannelSlashOutcome::Reply {
                content: format!("Select an option for /{}:", name),
                new_session_id: None,
                buttons,
            });
        }
    }

    let result = handlers::dispatch(app_state, Some(session_id), agent_id, &name, &args)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    use crate::slash_commands::types::CommandAction;
    match result.action {
        // Pass transformed message to the LLM (skill commands, /search, etc.)
        Some(CommandAction::PassThrough { message }) => {
            Ok(ChannelSlashOutcome::PassThrough(message))
        }

        // A new session was created — remap the channel conversation to it.
        Some(CommandAction::NewSession {
            session_id: new_sid,
        }) => {
            if let Err(e) =
                channel_db.update_session(channel_id, account_id, chat_id, thread_id, &new_sid)
            {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to remap channel session after /new: {}",
                    e
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: Some(new_sid),
                buttons: vec![],
            })
        }

        // Agent switch also creates a new session.
        Some(CommandAction::SwitchAgent {
            session_id: new_sid,
            ..
        }) => {
            if let Err(e) =
                channel_db.update_session(channel_id, account_id, chat_id, thread_id, &new_sid)
            {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to remap channel session after /agent: {}",
                    e
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: Some(new_sid),
                buttons: vec![],
            })
        }

        // ViewSystemPrompt — build and return the system prompt text directly.
        Some(CommandAction::ViewSystemPrompt) => {
            let (model, provider) = {
                let store = app_state.config.lock().await;
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
            let prompt = crate::agent::build_system_prompt_with_session(
                agent_id,
                &model,
                &provider,
                Some(session_id),
            );
            Ok(ChannelSlashOutcome::Reply {
                content: format!("**System Prompt**\n\n```\n{}\n```", prompt),
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Model switch — persist + notify frontend ──
        Some(CommandAction::SwitchModel {
            provider_id,
            model_id,
        }) => {
            if let Err(e) = set_active_model_core(&provider_id, &model_id, app_state).await {
                app_warn!("channel", "worker", "Failed to switch model: {}", e);
            } else if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "slash:model_switched",
                    serde_json::json!({
                        "providerId": provider_id,
                        "modelId": model_id,
                    }),
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Reasoning effort — persist + notify frontend ──
        Some(CommandAction::SetEffort { effort }) => {
            if let Err(e) = set_reasoning_effort_core(&effort, app_state).await {
                app_warn!("channel", "worker", "Failed to set effort: {}", e);
            } else if let Some(bus) = crate::get_event_bus() {
                bus.emit("slash:effort_changed", serde_json::json!(effort));
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Stop stream — cancel via registry ──
        Some(CommandAction::StopStream) => {
            let cancelled = app_state.channel_cancels.cancel(session_id);
            let msg = if cancelled {
                "Stopping current stream...".to_string()
            } else {
                "No active stream to stop.".to_string()
            };
            Ok(ChannelSlashOutcome::Reply {
                content: msg,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Compact — run compaction ──
        Some(CommandAction::Compact) => {
            match compact_context_now_core(session_id, app_state).await {
                Ok(r) => {
                    let msg = format!(
                        "Compacted: {} → {} tokens ({} messages affected)",
                        r.tokens_before, r.tokens_after, r.messages_affected
                    );
                    Ok(ChannelSlashOutcome::Reply {
                        content: msg,
                        new_session_id: None,
                        buttons: vec![],
                    })
                }
                Err(e) => Ok(ChannelSlashOutcome::Reply {
                    content: format!("Compaction failed: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                }),
            }
        }

        // ── Session cleared — notify frontend ──
        Some(CommandAction::SessionCleared) => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit("slash:session_cleared", serde_json::json!(session_id));
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Export — write to file ──
        Some(CommandAction::ExportFile { content, filename }) => {
            let msg = match crate::paths::root_dir() {
                Ok(root) => {
                    let export_dir = root.join("exports");
                    let _ = std::fs::create_dir_all(&export_dir);
                    let path = export_dir.join(&filename);
                    match std::fs::write(&path, &content) {
                        Ok(_) => format!("Exported to `{}`", path.display()),
                        Err(e) => format!("Export failed: {}", e),
                    }
                }
                Err(e) => format!("Export failed: {}", e),
            };
            Ok(ChannelSlashOutcome::Reply {
                content: msg,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Tool permission — not applicable in channel context ──
        Some(CommandAction::SetToolPermission { mode }) => Ok(ChannelSlashOutcome::Reply {
            content: format!(
                "Tool permission `{}` is not applicable in channel context (auto-approve).",
                mode
            ),
            new_session_id: None,
            buttons: vec![],
        }),

        // ── Plan: show plan content ──
        Some(CommandAction::ShowPlan { plan_content }) => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit("slash:plan_changed", serde_json::json!(session_id));
            }
            Ok(ChannelSlashOutcome::Reply {
                content: format!("**Current Plan**\n\n{}", plan_content),
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Plan: state transitions (DB already persisted by handler) ──
        Some(CommandAction::EnterPlanMode)
        | Some(CommandAction::ExitPlanMode { .. })
        | Some(CommandAction::ApprovePlan { .. })
        | Some(CommandAction::PausePlan)
        | Some(CommandAction::ResumePlan) => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit("slash:plan_changed", serde_json::json!(session_id));
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── ShowModelPicker: render model list as inline buttons for IM channels ──
        Some(CommandAction::ShowModelPicker {
            models,
            active_provider_id,
            active_model_id,
        }) => {
            let buttons =
                build_model_buttons_from_items(&models, &active_provider_id, &active_model_id);
            Ok(ChannelSlashOutcome::Reply {
                content: "Select a model:".into(),
                new_session_id: None,
                buttons,
            })
        }

        // ── DisplayOnly and any unhandled actions — just return text ──
        _ => Ok(ChannelSlashOutcome::Reply {
            content: result.content,
            new_session_id: None,
            buttons: vec![],
        }),
    }
}

// ── Core helpers (migrated from src-tauri/src/commands/) ──────────

/// Switch the active model. Equivalent to the old `commands::provider::set_active_model_core`.
async fn set_active_model_core(
    provider_id: &str,
    model_id: &str,
    state: &crate::globals::AppState,
) -> Result<(), String> {
    use crate::agent::AssistantAgent;
    use crate::provider::{ActiveModel, ApiType};

    let mut store = state.config.lock().await;

    let provider = store
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider not found: {}", provider_id))?;

    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(format!("Model not found: {}", model_id));
    }

    if provider.api_type == ApiType::Codex {
        let token_info = state.codex_token.lock().await.clone();
        if let Some((access_token, account_id)) = token_info {
            let agent = AssistantAgent::new_openai(&access_token, &account_id, model_id);
            *state.agent.lock().await = Some(agent);
        }
    } else {
        let agent = AssistantAgent::new_from_provider(provider, model_id)
            .with_failover_context(provider);
        *state.agent.lock().await = Some(agent);
    }

    store.active_model = Some(ActiveModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    });
    crate::config::save_config(&store).map_err(|e| e.to_string())?;
    Ok(())
}

/// Set reasoning effort. Equivalent to the old `commands::auth::set_reasoning_effort_core`.
async fn set_reasoning_effort_core(
    effort: &str,
    state: &crate::globals::AppState,
) -> Result<(), String> {
    let valid = ["none", "low", "medium", "high", "xhigh"];
    if !valid.contains(&effort) {
        return Err(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort, valid
        ));
    }
    *state.reasoning_effort.lock().await = effort.to_string();
    Ok(())
}

/// Manual context compaction. Equivalent to the old `commands::config::compact_context_now_core`.
async fn compact_context_now_core(
    session_id: &str,
    state: &crate::globals::AppState,
) -> Result<crate::context_compact::CompactResult, String> {
    use crate::chat_engine::save_agent_context;
    use crate::context_compact;

    let agent = state.agent.lock().await;
    let agent = agent.as_ref().ok_or("No active agent")?;

    let mut history = agent.get_conversation_history();
    if history.is_empty() {
        return Ok(context_compact::CompactResult {
            tier_applied: 0,
            tokens_before: 0,
            tokens_after: 0,
            messages_affected: 0,
            description: "no_messages".to_string(),
            details: None,
        });
    }

    let compact_config = crate::config::cached_config().compact.clone();

    let system_prompt_estimate = "system";
    let max_tokens: u32 = 16384;

    let result = context_compact::compact_if_needed(
        &mut history,
        system_prompt_estimate,
        agent.get_context_window(),
        max_tokens,
        &compact_config,
    );

    if result.tier_applied == 0 {
        let mut forced_config = compact_config;
        forced_config.soft_trim_ratio = 0.0;
        forced_config.hard_clear_ratio = 0.0;

        let forced_result = context_compact::compact_if_needed(
            &mut history,
            system_prompt_estimate,
            agent.get_context_window(),
            max_tokens,
            &forced_config,
        );

        if forced_result.messages_affected > 0 {
            agent.set_conversation_history(history);
            save_agent_context(&state.session_db, session_id, agent);
            app_info!(
                "context",
                "compact::manual",
                "Manual compaction: {} → {} tokens, {} affected",
                forced_result.tokens_before,
                forced_result.tokens_after,
                forced_result.messages_affected
            );
        }
        return Ok(forced_result);
    }

    agent.set_conversation_history(history);
    save_agent_context(&state.session_db, session_id, agent);
    app_info!(
        "context",
        "compact::manual",
        "Manual compaction: tier={}, {} → {} tokens, {} affected",
        result.tier_applied,
        result.tokens_before,
        result.tokens_after,
        result.messages_affected
    );

    Ok(result)
}

/// Build inline keyboard buttons from model picker items.
/// Each model gets a button with callback_data `slash:model <model_name>`.
/// Telegram limits callback_data to 64 bytes, so we use model_name
/// (the display name the fuzzy matcher accepts) rather than model_id.
pub(super) fn build_model_buttons_from_items(
    models: &[crate::slash_commands::types::ModelPickerItem],
    active_provider_id: &Option<String>,
    active_model_id: &Option<String>,
) -> Vec<Vec<crate::channel::types::InlineButton>> {
    let mut rows: Vec<Vec<crate::channel::types::InlineButton>> = Vec::new();
    let mut row: Vec<crate::channel::types::InlineButton> = Vec::new();

    for m in models.iter().take(20) {
        let is_active = active_provider_id
            .as_ref()
            .zip(active_model_id.as_ref())
            .map(|(pid, mid)| pid == &m.provider_id && mid == &m.model_id)
            .unwrap_or(false);
        let label = if is_active {
            format!("✓ {}", m.model_name)
        } else {
            m.model_name.clone()
        };
        let cb = format!("slash:model {}", m.model_name);
        let cb = if cb.len() > 64 {
            format!("slash:model {}", &m.model_id)
        } else {
            cb
        };
        row.push(crate::channel::types::InlineButton {
            text: label,
            callback_data: Some(cb),
            url: None,
        });
        if row.len() >= 2 {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

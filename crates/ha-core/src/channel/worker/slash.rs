use crate::channel::db::{ChannelDB, ATTACH_SOURCE_ATTACH};
use crate::channel::types::{ChatType, InlineButton};

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
///
/// `supports_buttons` gates the "no-arg + arg_options ⇒ inline-keyboard
/// picker" shortcut — channels without inline buttons (WeChat / iMessage /
/// IRC / Signal / WhatsApp) would otherwise show a useless `Select an
/// option for /xxx:` line with the buttons silently dropped, hiding the
/// handler's actual help text. On those channels we skip the shortcut and
/// let the handler render its normal no-arg response.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_slash_for_channel(
    channel_db: &ChannelDB,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    chat_type: &ChatType,
    session_id: &str,
    agent_id: &str,
    text: &str,
    supports_buttons: bool,
) -> Result<ChannelSlashOutcome, anyhow::Error> {
    use crate::slash_commands::{handlers, parser};

    let (name, args) = parser::parse(text).map_err(|e| anyhow::anyhow!(e))?;

    // For commands with fixed arg_options and no args provided, return inline
    // buttons so IM channel users (e.g. Telegram) can tap to select an option.
    // Checks both built-in commands AND dynamic skill commands. Skipped on
    // channels without inline-button support — see fn-level doc.
    if supports_buttons && args.trim().is_empty() {
        use crate::slash_commands::registry;

        // First check built-in commands
        let commands = registry::all_commands();
        let mut options_found: Option<Vec<String>> = commands
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.arg_options.clone());

        // If not found in built-in, check dynamic skill commands
        if options_found.is_none() {
            let store = crate::config::cached_config();
            let skills = crate::skills::get_invocable_skills(
                &store.extra_skills_dirs,
                &store.disabled_skills,
            );
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

    let result = handlers::dispatch(Some(session_id), agent_id, &name, &args)
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
        // NOTE: `/agent` is in `IM_DISABLED_COMMANDS` and the handler self-checks
        // `session.channel_info`, so this branch is currently unreachable from IM
        // channels. Kept as defense-in-depth in case future config opens a
        // controlled IM agent-switch path.
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
                let store = crate::config::cached_config();
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
            if let Err(e) = set_active_model_core(&provider_id, &model_id).await {
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
            if let Err(e) = set_reasoning_effort_core(&effort).await {
                app_warn!("channel", "worker", "Failed to set effort: {}", e);
            } else {
                if let Some(db) = crate::get_session_db() {
                    let _ = db.update_session_reasoning_effort(session_id, Some(&effort));
                }
                if let Some(bus) = crate::get_event_bus() {
                    bus.emit(
                        "slash:effort_changed",
                        serde_json::json!({
                            "sessionId": session_id,
                            "effort": effort,
                        }),
                    );
                }
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Stop stream — cancel via registry ──
        Some(CommandAction::StopStream) => {
            let cancelled = crate::globals::get_channel_cancels()
                .map(|reg| reg.cancel(session_id))
                .unwrap_or(false);
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
        Some(CommandAction::Compact) => match compact_context_now_core(session_id).await {
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
        },

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

        // ── Permission mode — write SessionMeta + notify frontend ──
        // SessionDB is guaranteed available here: `handlers::dispatch` above
        // already short-circuits with `session_db()?` on the same crate-level
        // global, so reaching this arm implies the global is initialized.
        Some(CommandAction::SetToolPermission { mode }) => {
            let resolved = crate::permission::SessionMode::parse_or_default(&mode);
            let session_db = crate::require_session_db()?;
            if let Err(e) = session_db.update_session_permission_mode(session_id, resolved) {
                app_warn!(
                    "channel",
                    "worker",
                    "Failed to update session permission mode: {}",
                    e
                );
                return Ok(ChannelSlashOutcome::Reply {
                    content: format!("Failed to set permission mode: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                });
            }
            app_info!(
                "channel",
                "worker",
                "Permission mode set to {} for session {}",
                resolved.as_str(),
                session_id
            );
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "permission:mode_changed",
                    serde_json::json!({
                        "sessionId": session_id,
                        "mode": resolved.as_str(),
                    }),
                );
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

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
        | Some(CommandAction::ApprovePlan { .. }) => {
            if let Some(bus) = crate::get_event_bus() {
                bus.emit("slash:plan_changed", serde_json::json!(session_id));
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── ShowModelPicker: inline-button picker for channels that
        //    support it; on others (WeChat / iMessage / IRC / Signal /
        //    WhatsApp) render the list as text + usage hint so the user
        //    can pick by typing `/model <name>`.
        Some(CommandAction::ShowModelPicker {
            models,
            active_provider_id,
            active_model_id,
        }) => {
            if supports_buttons {
                let buttons =
                    build_model_buttons_from_items(&models, &active_provider_id, &active_model_id);
                Ok(ChannelSlashOutcome::Reply {
                    content: "Select a model:".into(),
                    new_session_id: None,
                    buttons,
                })
            } else {
                Ok(ChannelSlashOutcome::Reply {
                    content: render_model_picker_text(
                        &models,
                        &active_provider_id,
                        &active_model_id,
                    ),
                    new_session_id: None,
                    buttons: vec![],
                })
            }
        }

        // ── Session picker (`/sessions`) — render rows as inline buttons. ──
        Some(CommandAction::ShowSessionPicker { sessions }) => {
            let buttons = build_picker_buttons("session", sessions.iter().map(|s| {
                let id_short: String = s.id.chars().take(8).collect();
                let chip = s
                    .channel_label
                    .as_deref()
                    .map(|c| format!(" · {}", c))
                    .unwrap_or_default();
                let label = format!("{} · {}{}", id_short, s.title, chip);
                (s.id.clone(), id_short, label)
            }));
            let text = if sessions.is_empty() {
                "No active sessions.".to_string()
            } else {
                format!("Pick a session ({}):", sessions.len())
            };
            Ok(ChannelSlashOutcome::Reply {
                content: text,
                new_session_id: None,
                buttons,
            })
        }

        // ── /session <id> — attach this chat to the target session. ──
        Some(CommandAction::AttachToSession {
            session_id: target_sid,
        }) => {
            if let Err(e) = channel_db.attach_session(
                channel_id,
                account_id,
                chat_id,
                thread_id,
                &target_sid,
                ATTACH_SOURCE_ATTACH,
                None,
                None,
                chat_type,
            ) {
                return Ok(ChannelSlashOutcome::Reply {
                    content: format!("Attach failed: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                });
            }
            // Future inbound from this chat now resolves to `target_sid`;
            // surface the swap to the caller so it can adopt the new id.
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: Some(target_sid),
                buttons: vec![],
            })
        }

        // ── /session exit — detach this chat from its session. ──
        Some(CommandAction::DetachFromSession) => {
            match channel_db.detach_session(channel_id, account_id, chat_id, thread_id) {
                Ok(Some(_)) => Ok(ChannelSlashOutcome::Reply {
                    content: "Detached. Send another message to start a new session.".into(),
                    new_session_id: None,
                    buttons: vec![],
                }),
                Ok(None) => Ok(ChannelSlashOutcome::Reply {
                    content: "No session attached to this chat.".into(),
                    new_session_id: None,
                    buttons: vec![],
                }),
                Err(e) => Ok(ChannelSlashOutcome::Reply {
                    content: format!("Detach failed: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                }),
            }
        }

        // ── /project <id> from IM — re-point the chat's session to a project. ──
        Some(CommandAction::AssignProject { project_id }) => {
            let session_db = crate::require_session_db()?;
            if let Err(e) = session_db.set_session_project(session_id, Some(&project_id)) {
                return Ok(ChannelSlashOutcome::Reply {
                    content: format!("Failed to link project: {}", e),
                    new_session_id: None,
                    buttons: vec![],
                });
            }
            Ok(ChannelSlashOutcome::Reply {
                content: result.content,
                new_session_id: None,
                buttons: vec![],
            })
        }

        // ── Project picker (`/project` / `/projects` no args). ──
        Some(CommandAction::ShowProjectPicker { projects }) => {
            let buttons = build_picker_buttons("project", projects.iter().map(|p| {
                let id_short: String = p.id.chars().take(8).collect();
                let label = match p.emoji.as_deref() {
                    Some(e) if !e.is_empty() => format!("{} {}", e, p.name),
                    _ => p.name.clone(),
                };
                (p.id.clone(), id_short, label)
            }));
            let text = if projects.is_empty() {
                "No projects yet.".to_string()
            } else {
                format!("Pick a project ({}):", projects.len())
            };
            Ok(ChannelSlashOutcome::Reply {
                content: text,
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
async fn set_active_model_core(provider_id: &str, model_id: &str) -> Result<(), String> {
    use crate::agent::AssistantAgent;
    use crate::provider::ApiType;

    // Clone the provider before awaiting on agent/codex_token locks —
    // the Arc from `cached_config()` must be dropped first to avoid deadlock.
    let provider = {
        let store = crate::config::cached_config();
        let found = store
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .cloned()
            .ok_or_else(|| format!("Provider not found: {}", provider_id))?;
        if !found.models.iter().any(|m| m.id == model_id) {
            return Err(format!("Model not found: {}", model_id));
        }
        found
    };

    let cached_agent = crate::require_cached_agent().map_err(|e| e.to_string())?;

    if provider.api_type == ApiType::Codex {
        let token_info = match crate::get_codex_token_cache() {
            Some(cell) => cell.lock().await.clone(),
            None => None,
        };
        if let Some((access_token, account_id)) = token_info {
            let agent = AssistantAgent::new_openai(&access_token, &account_id, model_id);
            *cached_agent.lock().await = Some(agent);
        }
    } else {
        let agent = AssistantAgent::try_new_from_provider(&provider, model_id)
            .await
            .map_err(|e| e.to_string())?
            .with_failover_context(&provider);
        *cached_agent.lock().await = Some(agent);
    }

    crate::provider::set_active_model(
        provider_id.to_string(),
        model_id.to_string(),
        "slash-channel",
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Set reasoning effort. Equivalent to the old `commands::auth::set_reasoning_effort_core`.
async fn set_reasoning_effort_core(effort: &str) -> Result<(), String> {
    if !crate::agent::is_valid_reasoning_effort(effort) {
        return Err(format!(
            "Invalid reasoning effort: {}. Valid: {:?}",
            effort,
            crate::agent::VALID_REASONING_EFFORTS
        ));
    }
    let cell = crate::require_reasoning_effort_cell().map_err(|e| e.to_string())?;
    *cell.lock().await = effort.to_string();
    Ok(())
}

/// Manual context compaction. Equivalent to the old `commands::config::compact_context_now_core`.
async fn compact_context_now_core(
    session_id: &str,
) -> Result<crate::context_compact::CompactResult, String> {
    use crate::chat_engine::save_agent_context;
    use crate::context_compact;

    let session_db = crate::require_session_db().map_err(|e| e.to_string())?;
    let cached_agent = crate::require_cached_agent().map_err(|e| e.to_string())?;
    let agent = cached_agent.lock().await;
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
            save_agent_context(session_db, session_id, agent);
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
    save_agent_context(session_db, session_id, agent);
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

fn is_model_active(
    item: &crate::slash_commands::types::ModelPickerItem,
    active_provider_id: &Option<String>,
    active_model_id: &Option<String>,
) -> bool {
    active_provider_id
        .as_ref()
        .zip(active_model_id.as_ref())
        .map(|(pid, mid)| pid == &item.provider_id && mid == &item.model_id)
        .unwrap_or(false)
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
        let label = if is_model_active(m, active_provider_id, active_model_id) {
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

/// Build a vertical inline-button list for a slash picker. Each row is
/// `slash:<command> <id>`; if the resulting callback_data exceeds the
/// 64-byte limit (Telegram), the truncated `id_short` is used instead.
/// Items beyond 20 are dropped to keep the keyboard rendering tractable.
fn build_picker_buttons(
    command: &str,
    items: impl Iterator<Item = (String, String, String)>,
) -> Vec<Vec<InlineButton>> {
    items
        .take(20)
        .map(|(id, id_short, label)| {
            let cb = format!("slash:{} {}", command, id);
            let cb = if cb.len() > 64 {
                format!("slash:{} {}", command, id_short)
            } else {
                cb
            };
            vec![InlineButton {
                text: label,
                callback_data: Some(cb),
                url: None,
            }]
        })
        .collect()
}

/// Text fallback for `ShowModelPicker` on channels without inline buttons.
/// Lists up to 20 models with the active one marked, then a one-line
/// instruction so the user can pick by typing `/model <name>`. Same 20-cap
/// + same model_name preference as `build_model_buttons_from_items` so
/// the button and text paths look identical.
pub(super) fn render_model_picker_text(
    models: &[crate::slash_commands::types::ModelPickerItem],
    active_provider_id: &Option<String>,
    active_model_id: &Option<String>,
) -> String {
    let mut lines = Vec::with_capacity(models.len().min(20) + 2);
    lines.push("**Available models** (use `/model <name>` to switch):".to_string());
    for m in models.iter().take(20) {
        let prefix = if is_model_active(m, active_provider_id, active_model_id) {
            "✓"
        } else {
            "-"
        };
        lines.push(format!(
            "{} `{}` ({})",
            prefix, m.model_name, m.provider_name
        ));
    }
    if models.len() > 20 {
        lines.push(format!("… +{} more", models.len() - 20));
    }
    lines.join("\n")
}

use std::sync::Arc;

use crate::channel::db::{ChannelDB, ATTACH_SOURCE_ATTACH};
use crate::channel::traits::ChannelPlugin;
use crate::channel::types::{ChannelAccountConfig, ChatType, InlineButton};

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
/// **No-arg shortcut for commands with fixed `arg_options`**:
///   - `supports_buttons` → render `arg_options` as an inline-keyboard picker.
///   - `!supports_buttons` + `args_optional=false` → render a `Usage / Options`
///     text hint (WeChat / iMessage / IRC / Signal / WhatsApp), so the user
///     sees the legal values up front instead of the handler's bare
///     "Invalid X: \`\`" error.
///   - `!supports_buttons` + `args_optional=true` → fall through to the
///     handler so commands like `/imreply` / `/sessions` / `/recap` get to
///     show their custom no-arg response (current state / picker / etc.).
///   Skill commands have no `args_optional` field and are treated as
///   optional-args (skills typically run no-arg by default).
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_slash_for_channel(
    channel_db: &ChannelDB,
    plugin: &Arc<dyn ChannelPlugin>,
    account: &ChannelAccountConfig,
    channel_id: &str,
    account_id: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    chat_type: &ChatType,
    session_id: &str,
    agent_id: &str,
    text: &str,
    sender_id: &str,
    supports_buttons: bool,
) -> Result<ChannelSlashOutcome, anyhow::Error> {
    use crate::slash_commands::{handlers, parser};

    let (name, args) = parser::parse(text).map_err(|e| anyhow::anyhow!(e))?;

    // WS8: `/kb on|off` writes a per-group KB-access confirmation — a security
    // consent step. When the account has admins configured, restrict the write to
    // them so a random group participant can't self-confirm their chat. (Status —
    // no-arg / `status` — stays open to anyone who may use the bot.) The
    // account-level opt-in remains owner-GUI-only regardless; this only guards the
    // in-chat per-group toggle.
    if name == "kb" && kb_write_denied_for_sender(&args, &account.security.admin_ids, sender_id) {
        return Ok(ChannelSlashOutcome::Reply {
            content: "Only a channel admin can change this chat's knowledge-base access.".into(),
            new_session_id: None,
            buttons: vec![],
        });
    }

    // No-arg shortcut for commands that advertise a fixed `arg_options` set —
    // see fn-level doc for the supports_buttons / args_optional matrix.
    if args.trim().is_empty() {
        if let Some(help) = lookup_command_help(&name) {
            if let Some(options) = help.arg_options.as_ref() {
                if supports_buttons {
                    let buttons: Vec<Vec<InlineButton>> = options
                        .iter()
                        .map(|opt| {
                            vec![InlineButton {
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
                } else if !help.args_optional {
                    return Ok(ChannelSlashOutcome::Reply {
                        content: render_options_help_text(
                            &name,
                            help.arg_placeholder.as_deref(),
                            options,
                        ),
                        new_session_id: None,
                        buttons: vec![],
                    });
                }
                // !supports_buttons + args_optional → fall through to the
                // handler so its custom no-arg branch can render.
            }
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

        // ── Model switch — pin to current session, notify GUI ──
        Some(CommandAction::SwitchModel {
            provider_id,
            model_id,
        }) => {
            if let Err(e) = set_session_model_core(session_id, &provider_id, &model_id).await {
                app_warn!("channel", "worker", "Failed to switch model: {}", e);
            } else if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    "session:model_updated",
                    serde_json::json!({
                        "sessionId": session_id,
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
        Some(CommandAction::Compact) => {
            match compact_context_now_core(session_id, agent_id).await {
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

        // ── Session picker (`/sessions`) — buttons on supporting channels,
        //    text list w/ short ids on the rest. `handle_session` accepts a
        //    unique prefix so the text path stays usable.
        Some(CommandAction::ShowSessionPicker { sessions }) => {
            // Empty-picker text comes from the handler so the no-query
            // case ("No active sessions.") and the no-match case ("No
            // sessions match `foo`.") stay distinct on IM surfaces.
            if sessions.is_empty() {
                return Ok(ChannelSlashOutcome::Reply {
                    content: result.content,
                    new_session_id: None,
                    buttons: vec![],
                });
            }
            if supports_buttons {
                // Body shows the first SESSION_PICKER_BODY_LIMIT rows with
                // chips; the rest stay reachable through inline buttons.
                // Telegram caps single messages at 4096 chars and Discord
                // at 2000, so 30 rows × ~200 bytes/row easily overflows.
                let buttons = build_picker_buttons(
                    "session",
                    sessions.iter().map(|s| {
                        let id_short: String = s.id.chars().take(8).collect();
                        let agent_chip = if s.agent_label.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", s.agent_label)
                        };
                        let label = format!("{} · {}{}", id_short, s.title, agent_chip);
                        (s.id.clone(), id_short, label)
                    }),
                );
                Ok(ChannelSlashOutcome::Reply {
                    content: render_session_picker_buttons_body(&sessions),
                    new_session_id: None,
                    buttons,
                })
            } else {
                Ok(ChannelSlashOutcome::Reply {
                    content: render_session_picker_text(&sessions),
                    new_session_id: None,
                    buttons: vec![],
                })
            }
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
            // Replay the latest completed turn (assistant text + media) to
            // this chat so the user attaching mid-conversation isn't
            // dropped into a session with zero visible context. Best-effort
            // — failures are logged inside the helper and don't fail the
            // attach itself.
            crate::channel::attach_sync::deliver_attach_catchup(
                plugin,
                account,
                &target_sid,
                chat_id,
                thread_id,
            )
            .await;
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

        // ── Project picker (`/project` / `/projects` no args). Same
        //    button-vs-text split as the session picker; text path tells
        //    the user to type `/project <name>` since `handle_project`
        //    fuzzy-matches the name.
        Some(CommandAction::ShowProjectPicker { projects }) => {
            if projects.is_empty() {
                return Ok(ChannelSlashOutcome::Reply {
                    content: "No projects yet.".into(),
                    new_session_id: None,
                    buttons: vec![],
                });
            }
            if supports_buttons {
                let buttons = build_picker_buttons(
                    "project",
                    projects.iter().map(|p| {
                        let id_short: String = p.id.chars().take(8).collect();
                        (p.id.clone(), id_short, p.name.clone())
                    }),
                );
                Ok(ChannelSlashOutcome::Reply {
                    content: format!("Pick a project ({}):", projects.len()),
                    new_session_id: None,
                    buttons,
                })
            } else {
                Ok(ChannelSlashOutcome::Reply {
                    content: render_project_picker_text(&projects),
                    new_session_id: None,
                    buttons: vec![],
                })
            }
        }

        // ── DisplayOnly and any unhandled actions — just return text ──
        _ => Ok(ChannelSlashOutcome::Reply {
            content: result.content,
            new_session_id: None,
            buttons: vec![],
        }),
    }
}

/// WS8: whether a `/kb` invocation is a *write* the sender is not authorized to
/// perform. A write (`on`/`off`/…) requires the sender to be an account admin
/// when `admin_ids` is configured; status (no-arg / `status`) is always allowed.
/// With no admins configured the per-group toggle stays open (still bounded by
/// the owner-only account-level opt-in). Pure so it is unit-tested directly.
fn kb_write_denied_for_sender(arg: &str, admin_ids: &[String], sender_id: &str) -> bool {
    let is_write = matches!(
        arg.trim().to_lowercase().as_str(),
        "on" | "off" | "enable" | "disable" | "yes" | "no"
    );
    is_write && !admin_ids.is_empty() && !admin_ids.iter().any(|a| a == sender_id)
}

// ── Core helpers (migrated from src-tauri/src/commands/) ──────────

/// Pin a provider/model to the IM-bound session. Validates that the provider /
/// model still exist + are enabled, then writes `sessions.provider_id /
/// provider_name / model_id`. The next chat turn picks it up via the
/// `session_pinned_model` branch in commands::chat / routes::chat. **Does not**
/// touch `config.active_model` — per-session selection no longer leaks into
/// the application-wide default.
async fn set_session_model_core(
    session_id: &str,
    provider_id: &str,
    model_id: &str,
) -> Result<(), String> {
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

    let session_db = crate::require_session_db().map_err(|e| e.to_string())?;
    session_db
        .update_session_model(
            session_id,
            Some(provider_id),
            Some(provider.name.as_str()),
            Some(model_id),
        )
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
    Ok(())
}

/// Manual context compaction for IM/channel slash commands.
async fn compact_context_now_core(
    session_id: &str,
    agent_id: &str,
) -> Result<crate::context_compact::CompactResult, String> {
    let session_db = crate::require_session_db().map_err(|e| e.to_string())?;
    let store = crate::config::cached_config();
    let agent_def = crate::agent_loader::load_agent(agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    let pinned = session_db
        .get_session(session_id)
        .map_err(|e| e.to_string())?
        .and_then(|meta| match (meta.provider_id, meta.model_id) {
            (Some(provider_id), Some(model_id))
                if !provider_id.is_empty() && !model_id.is_empty() =>
            {
                Some(format!("{provider_id}::{model_id}"))
            }
            _ => None,
        });

    let (primary, fallbacks) = if let Some(pinned) = pinned {
        let mut cfg = agent_model_config.clone();
        cfg.primary = Some(pinned);
        crate::provider::resolve_model_chain(&cfg, &store)
    } else {
        crate::provider::resolve_model_chain(&agent_model_config, &store)
    };

    let mut model_chain = Vec::new();
    if let Some(model) = primary {
        model_chain.push(model);
    }
    for model in fallbacks {
        if !model_chain
            .iter()
            .any(|m| m.provider_id == model.provider_id && m.model_id == model.model_id)
        {
            model_chain.push(model);
        }
    }
    let model = model_chain
        .into_iter()
        .next()
        .ok_or_else(|| "No model configured for manual compaction".to_string())?;

    let resolved_temperature = agent_def
        .as_ref()
        .and_then(|def| def.config.model.temperature)
        .or(store.temperature);

    let result =
        crate::chat_engine::compact_session_now(crate::chat_engine::CompactSessionParams {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            session_db: session_db.clone(),
            model,
            providers: store.providers.clone(),
            codex_token: None,
            resolved_temperature,
            compact_config: store.compact.clone(),
            source: crate::chat_engine::ChatSource::Channel,
            event_sink: Arc::new(crate::chat_engine::NoopEventSink),
        })
        .await?;

    Ok(result.compact_result)
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

/// Body row caps for `ShowSessionPicker` on IM channels. Picker rows can
/// reach ~280 bytes (chip line + 160-byte FTS snippet), and Discord caps
/// single messages at 2000 chars / Telegram at 4096; staying well below
/// both limits avoids silent send failures while keeping the picker
/// useful. The buttons branch shows fewer rows because the inline buttons
/// already let users pick the truncated tail.
const SESSION_PICKER_BUTTONS_BODY_LIMIT: usize = 8;
const SESSION_PICKER_TEXT_BODY_LIMIT: usize = 12;

/// Body for the buttons branch of `ShowSessionPicker` — header plus the
/// first `SESSION_PICKER_BUTTONS_BODY_LIMIT` rows with chips. Sessions
/// past the cap stay reachable via the inline buttons rendered alongside.
fn render_session_picker_buttons_body(
    sessions: &[crate::slash_commands::types::SessionPickerItem],
) -> String {
    let total = sessions.len();
    let mut lines: Vec<String> = vec![format!("Pick a session ({}):", total)];
    for s in sessions.iter().take(SESSION_PICKER_BUTTONS_BODY_LIMIT) {
        lines.push(crate::slash_commands::handlers::session::format_session_picker_line(s));
    }
    if total > SESSION_PICKER_BUTTONS_BODY_LIMIT {
        lines.push(format!(
            "… +{} more (use the buttons below)",
            total - SESSION_PICKER_BUTTONS_BODY_LIMIT
        ));
    }
    lines.join("\n")
}

/// Text fallback for `ShowSessionPicker` on channels without inline
/// buttons. `handle_session` accepts either a full id or a unique prefix,
/// so users on WeChat / iMessage / IRC / Signal / WhatsApp copy the short
/// id and type `/session <short>` to attach.
fn render_session_picker_text(
    sessions: &[crate::slash_commands::types::SessionPickerItem],
) -> String {
    let total = sessions.len();
    let mut lines = Vec::with_capacity(total.min(SESSION_PICKER_TEXT_BODY_LIMIT) + 2);
    lines.push(
        "**Sessions** (use `/session <id>` to attach; 8-char prefix works; \
         filter via `/sessions <query>`):"
            .to_string(),
    );
    for s in sessions.iter().take(SESSION_PICKER_TEXT_BODY_LIMIT) {
        lines.push(crate::slash_commands::handlers::session::format_session_picker_line(s));
    }
    if total > SESSION_PICKER_TEXT_BODY_LIMIT {
        lines.push(format!(
            "… +{} more (refine via `/sessions <query>`)",
            total - SESSION_PICKER_TEXT_BODY_LIMIT
        ));
    }
    lines.join("\n")
}

/// Resolved arg-help metadata for a slash command, gathered from either the
/// built-in registry or the dynamic skill catalog. Drives the no-arg shortcut
/// in `dispatch_slash_for_channel`.
struct CommandHelp {
    arg_options: Option<Vec<String>>,
    arg_placeholder: Option<String>,
    /// Built-in commands honour `SlashCommandDef.args_optional`. Skill commands
    /// have no equivalent field — skills almost always run no-arg by default,
    /// so we treat them as `args_optional = true` to let the handler's normal
    /// dispatch path (PassThrough / template expansion) run unimpeded.
    args_optional: bool,
}

/// Look up arg-help metadata for `name` across built-in and skill commands.
/// Returns `None` only when `name` matches neither — in that case the handler
/// will reject it and we don't want to short-circuit.
fn lookup_command_help(name: &str) -> Option<CommandHelp> {
    use crate::slash_commands::{canonical_builtin_command_name, registry};

    let lookup_name = canonical_builtin_command_name(name);
    if let Some(def) = registry::all_commands()
        .into_iter()
        .find(|c| c.name == lookup_name)
    {
        return Some(CommandHelp {
            arg_options: def.arg_options,
            arg_placeholder: def.arg_placeholder,
            args_optional: def.args_optional,
        });
    }

    let store = crate::config::cached_config();
    let skill =
        crate::skills::get_invocable_skills(&store.extra_skills_dirs, &store.disabled_skills)
            .into_iter()
            .find(|s| crate::skills::normalize_skill_command_name(&s.name) == name)?;
    Some(CommandHelp {
        arg_options: skill.command_arg_options,
        arg_placeholder: skill.command_arg_placeholder,
        args_optional: true,
    })
}

/// Text-only "Usage + Options" hint for IM channels without inline buttons
/// (WeChat / iMessage / IRC / Signal / WhatsApp). Replaces the handler's
/// otherwise-cryptic `Invalid X: \`\`` error for required-arg commands like
/// `/thinking`, `/permission`, `/plan`. Mirrors the option set the buttons
/// branch would have shown so users can copy-paste an option as the next
/// message.
pub(super) fn render_options_help_text(
    name: &str,
    placeholder: Option<&str>,
    options: &[String],
) -> String {
    let usage_arg = placeholder.unwrap_or("<option>");
    let mut lines = Vec::with_capacity(options.len() + 3);
    lines.push(format!("Usage: `/{} {}`", name, usage_arg));
    lines.push(String::new());
    lines.push("Options:".to_string());
    for opt in options {
        lines.push(format!("- `{}`", opt));
    }
    lines.join("\n")
}

/// Text fallback for `ShowProjectPicker` on channels without inline buttons.
/// Lists up to 20 projects with their name + session count.
/// `handle_project` already does fuzzy match on name, so the prompt
/// instructs users to type `/project <name>`.
fn render_project_picker_text(
    projects: &[crate::slash_commands::types::ProjectPickerItem],
) -> String {
    let mut lines = Vec::with_capacity(projects.len().min(20) + 2);
    lines.push("**Projects** (use `/project <name>` to switch):".to_string());
    for p in projects.iter().take(20) {
        lines.push(format!("- **{}** — {} session(s)", p.name, p.session_count));
    }
    if projects.len() > 20 {
        lines.push(format!("… +{} more", projects.len() - 20));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kb_status_always_allowed() {
        // No-arg / status is a read — never gated, even with admins configured.
        let admins = vec!["alice".to_string()];
        assert!(!kb_write_denied_for_sender("", &admins, "bob"));
        assert!(!kb_write_denied_for_sender("status", &admins, "bob"));
    }

    #[test]
    fn kb_write_open_without_admins() {
        // No admins configured → per-group toggle stays open (bounded by the
        // owner-only account opt-in elsewhere).
        let admins: Vec<String> = vec![];
        assert!(!kb_write_denied_for_sender("on", &admins, "bob"));
        assert!(!kb_write_denied_for_sender("off", &admins, "anyone"));
    }

    #[test]
    fn kb_write_gated_to_admins() {
        let admins = vec!["alice".to_string(), "carol".to_string()];
        // Non-admin write → denied.
        assert!(kb_write_denied_for_sender("on", &admins, "bob"));
        assert!(kb_write_denied_for_sender("OFF", &admins, "bob")); // case-insensitive
        assert!(kb_write_denied_for_sender(" enable ", &admins, "bob")); // trimmed
                                                                         // Admin write → allowed.
        assert!(!kb_write_denied_for_sender("on", &admins, "alice"));
        assert!(!kb_write_denied_for_sender("off", &admins, "carol"));
        // Unknown arg is not a write → not gated (handler will reject it).
        assert!(!kb_write_denied_for_sender("garbage", &admins, "bob"));
    }
}

use crate::agent_loader;
use crate::commands::CmdError;
use crate::context_compact;
use crate::paths;
use crate::provider;
use crate::tools;
use crate::user_config;
use crate::AppState;
use anyhow::Context;

#[tauri::command]
pub async fn get_default_agent_id() -> Result<Option<String>, CmdError> {
    Ok(ha_core::config::cached_config().default_agent_id.clone())
}

#[tauri::command]
pub async fn set_default_agent_id(agent_id: Option<String>) -> Result<(), CmdError> {
    let normalized = ha_core::agent::resolver::normalize_default_agent_id(agent_id.as_deref());
    if let Some(id) = normalized.as_deref() {
        ha_core::agent_lifecycle::ensure_agent_runnable(id)?;
    }
    ha_core::config::mutate_config_async(("default_agent", "settings-ui"), move |store| {
        store.default_agent_id = normalized;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_web_search_config() -> Result<tools::web_search::WebSearchConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    let mut config = store.web_search;
    tools::web_search::backfill_providers(&mut config);
    Ok(config)
}

#[tauri::command]
pub async fn save_web_search_config(
    config: tools::web_search::WebSearchConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("web_search", "settings-ui"), move |store| {
        store.web_search = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_web_fetch_config() -> Result<tools::web_fetch::WebFetchConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.web_fetch)
}

#[tauri::command]
pub async fn save_web_fetch_config(
    config: tools::web_fetch::WebFetchConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("web_fetch", "settings-ui"), move |store| {
        store.web_fetch = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_issue_reporting_config(
) -> Result<ha_core::issue_reporting::IssueReportingConfigStatus, CmdError> {
    Ok(ha_core::issue_reporting::get_config_status())
}

#[tauri::command]
pub async fn save_issue_reporting_config(
    config: ha_core::issue_reporting::IssueReportingConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("issue_reporting", "settings-ui"), move |store| {
        store.issue_reporting = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn save_issue_reporting_token(token: Option<String>) -> Result<(), CmdError> {
    ha_core::issue_reporting::save_token(token).map_err(Into::into)
}

#[tauri::command]
pub async fn test_issue_reporting_connection(
) -> Result<ha_core::issue_reporting::IssueReportingTestResult, CmdError> {
    let cfg = ha_core::config::cached_config().issue_reporting.clone();
    ha_core::issue_reporting::test_connection(&cfg)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ssrf_config() -> Result<ha_core::security::ssrf::SsrfConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ssrf)
}

#[tauri::command]
pub async fn save_ssrf_config(config: ha_core::security::ssrf::SsrfConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("security.ssrf", "settings-ui"), move |store| {
        store.ssrf = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_filesystem_config() -> Result<ha_core::config::FilesystemConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.filesystem)
}

#[tauri::command]
pub async fn save_filesystem_config(
    config: ha_core::config::FilesystemConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("filesystem", "settings-ui"), move |store| {
        store.filesystem = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_compact_config() -> Result<context_compact::CompactConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.compact)
}

#[tauri::command]
pub async fn save_compact_config(config: context_compact::CompactConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("compact", "settings-ui"), move |store| {
        store.compact = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_title_config(
) -> Result<ha_core::session_title::SessionTitleConfig, CmdError> {
    Ok(ha_core::config::cached_config().session_title.clone())
}

#[tauri::command]
pub async fn save_session_title_config(
    config: ha_core::session_title::SessionTitleConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("session_title", "settings-ui"), move |store| {
        store.session_title = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_notification_config() -> Result<ha_core::config::NotificationConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.notification)
}

#[tauri::command]
pub async fn save_notification_config(
    config: ha_core::config::NotificationConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("notification", "settings-ui"), move |store| {
        store.notification = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_auto_update_config() -> Result<ha_core::updater::AutoUpdateConfig, CmdError> {
    let store = ha_core::config::cached_config();
    Ok(store.auto_update.clone())
}

#[tauri::command]
pub async fn set_auto_update_config(
    config: ha_core::updater::AutoUpdateConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("auto_update", "settings-ui"), move |store| {
        store.auto_update = config;
        // Clamp the interval to the supported range on write so the stored
        // value matches what the loops actually use.
        store.auto_update.check_interval_hours = store.auto_update.clamped_interval_hours();
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_startup_notification_config(
) -> Result<ha_core::config::StartupNotificationConfig, CmdError> {
    let store = ha_core::config::cached_config();
    Ok(store.startup_notification.clone())
}

#[tauri::command]
pub async fn save_startup_notification_config(
    config: ha_core::config::StartupNotificationConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("startup_notification", "settings-ui"), move |store| {
        store.startup_notification = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_image_generate_config() -> Result<tools::image_generate::ImageGenConfig, CmdError>
{
    let store = ha_core::config::load_config()?;
    let mut config = store.image_generate;
    tools::image_generate::backfill_providers(&mut config);
    Ok(config)
}

#[tauri::command]
pub async fn save_image_generate_config(
    config: tools::image_generate::ImageGenConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("image_generate", "settings-ui"), move |store| {
        store.image_generate = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_audio_generate_config() -> Result<tools::audio_generate::AudioGenConfig, CmdError>
{
    let store = ha_core::config::load_config()?;
    let mut config = store.audio_generate;
    tools::audio_generate::backfill_providers(&mut config);
    Ok(config)
}

#[tauri::command]
pub async fn save_audio_generate_config(
    config: tools::audio_generate::AudioGenConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("audio_generate", "settings-ui"), |store| {
        store.audio_generate = config;
        Ok(())
    })
    .map_err(Into::into)
}

/// 策展音频模型目录（B8-1，GUI picker 预设，只读）。
#[tauri::command]
pub async fn get_audio_model_catalog_cmd(
) -> Result<Vec<tools::audio_generate::AudioModelInfo>, CmdError> {
    Ok(tools::audio_generate::audio_model_catalog())
}

/// 实时拉 ElevenLabs 语音列表（B8-1，语音 picker）。
#[tauri::command]
pub async fn list_elevenlabs_voices_cmd(
    limit: Option<u32>,
) -> Result<Vec<tools::audio_generate::VoiceOption>, CmdError> {
    tools::audio_generate::list_elevenlabs_voices(limit.unwrap_or(100))
        .await
        .map_err(Into::into)
}

/// Core logic for desktop manual context compaction.
pub(crate) async fn compact_context_now_core(
    session_id: &str,
    state: &AppState,
) -> Result<context_compact::CompactResult, CmdError> {
    let store = ha_core::config::load_config()?;
    let meta = {
        let session_id = session_id.to_string();
        state
            .session_db
            .run(move |db| db.get_session(&session_id))
            .await?
    }
    .ok_or_else(|| CmdError::msg("session not found"))?;
    let agent_id = meta.agent_id.clone();
    let agent_def = agent_loader::load_agent(&agent_id).ok();
    let agent_model_config = agent_def
        .as_ref()
        .map(|def| def.config.model.clone())
        .unwrap_or_default();

    let pinned = match (meta.provider_id.as_deref(), meta.model_id.as_deref()) {
        (Some(provider_id), Some(model_id)) if !provider_id.is_empty() && !model_id.is_empty() => {
            Some(format!("{provider_id}::{model_id}"))
        }
        _ => None,
    };

    let (primary, fallbacks) = if let Some(pinned) = pinned {
        let mut cfg = agent_model_config.clone();
        cfg.primary = Some(pinned);
        provider::resolve_model_chain(&cfg, &store)
    } else {
        provider::resolve_model_chain(&agent_model_config, &store)
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
        .ok_or_else(|| CmdError::msg("No model configured for manual compaction"))?;

    let resolved_temperature = if meta.runtime_defaults_initialized {
        meta.temperature
    } else {
        agent_def
            .as_ref()
            .and_then(|def| def.config.model.temperature)
            .or(store.temperature)
    };
    let codex_token = state.codex_token.lock().await.clone();

    let result =
        crate::chat_engine::compact_session_now(crate::chat_engine::CompactSessionParams {
            session_id: session_id.to_string(),
            agent_id,
            session_db: state.session_db.clone(),
            model,
            providers: store.providers.clone(),
            codex_token,
            resolved_temperature,
            compact_config: store.compact.clone(),
            source: crate::chat_engine::ChatSource::Desktop,
            event_sink: std::sync::Arc::new(crate::chat_engine::NoopEventSink),
        })
        .await
        .map_err(CmdError::msg)?;

    let compact_result = result.compact_result;
    *state.agent.lock().await = Some(result.agent);
    Ok(compact_result)
}

/// Manually trigger context compaction on the current session.
/// Returns the compaction result for frontend display.
#[tauri::command]
pub async fn compact_context_now(
    session_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<context_compact::CompactResult, CmdError> {
    compact_context_now_core(&session_id, &state).await
}

// ── Shortcuts ────────────────────────────────────────────────────

/// Temporarily unregister all global shortcuts (for recording mode)
/// or re-register them from config.
#[tauri::command]
pub async fn set_shortcuts_paused(app: tauri::AppHandle, paused: bool) -> Result<(), CmdError> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let manager = app.global_shortcut();

    if paused {
        // Clear pending chord state and unregister all
        crate::shortcuts::clear_chord_state();
        let _ = manager.unregister_all();
    } else {
        // Re-register from saved config
        let store = ha_core::config::load_config()?;
        let _ = manager.unregister_all();
        for binding in &store.shortcuts.bindings {
            if !binding.enabled || binding.keys.is_empty() {
                continue;
            }
            let key_to_register = if binding.is_chord() {
                binding.chord_parts()[0].to_string()
            } else {
                binding.keys.clone()
            };
            if let Ok(shortcut) = key_to_register.parse::<tauri_plugin_global_shortcut::Shortcut>()
            {
                let _ = manager.register(shortcut);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_shortcut_config() -> Result<ha_core::config::ShortcutConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.shortcuts)
}

#[tauri::command]
pub async fn save_shortcut_config(
    app: tauri::AppHandle,
    config: ha_core::config::ShortcutConfig,
) -> Result<(), CmdError> {
    // Validate all key combinations first
    for binding in &config.bindings {
        if binding.keys.is_empty() {
            continue;
        }
        for part in binding.chord_parts() {
            if part
                .parse::<tauri_plugin_global_shortcut::Shortcut>()
                .is_err()
            {
                return Err(CmdError::msg(format!(
                    "Invalid shortcut key combination: {}",
                    part
                )));
            }
        }
    }

    let config_for_store = config.clone();
    ha_core::config::mutate_config_async(("shortcuts", "settings-ui"), move |store| {
        store.shortcuts = config_for_store;
        Ok(())
    })
    .await?;

    // Clear any pending chord state
    crate::shortcuts::clear_chord_state();

    // Re-register global shortcuts (chord-aware)
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let manager = app.global_shortcut();
    let _ = manager.unregister_all();

    for binding in &config.bindings {
        if !binding.enabled || binding.keys.is_empty() {
            continue;
        }
        // For chord bindings, only register the first part;
        // second part is registered temporarily when first part is pressed.
        let key_to_register = if binding.is_chord() {
            binding.chord_parts()[0].to_string()
        } else {
            binding.keys.clone()
        };
        if let Ok(shortcut) = key_to_register.parse::<tauri_plugin_global_shortcut::Shortcut>() {
            if let Err(e) = manager.register(shortcut) {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "shortcut",
                        "save_shortcut_config",
                        &format!(
                            "Failed to register shortcut '{}' ({}): {}",
                            binding.id, key_to_register, e
                        ),
                        None,
                        None,
                        None,
                    );
                }
            }
        }
    }

    Ok(())
}

// ── Quick Prompts ───────────────────────────────────────────────

#[tauri::command]
pub async fn get_quick_prompt_config() -> Result<ha_core::config::QuickPromptConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.quick_prompts)
}

#[tauri::command]
pub async fn add_quick_prompt(
    content: String,
) -> Result<ha_core::config::QuickPromptAddResult, CmdError> {
    ha_core::config::mutate_config_async(("quick_prompts", "settings-ui"), move |store| {
        Ok(store.quick_prompts.add_prompt(&content)?)
    })
    .await
    .map_err(Into::into)
}

// ── Server Config ───────────────────────────────────────────────

#[tauri::command]
pub async fn get_server_config() -> Result<serde_json::Value, CmdError> {
    let store = ha_core::config::load_config()?;
    let server = &store.server;
    // Mask api_key for security
    let masked_key = server.api_key.as_ref().map(|k| {
        if k.is_empty() {
            "****".to_string()
        } else {
            ha_core::mask_secret_middle(k, 2, 2)
        }
    });
    let masked_knowledge_agent_read_token = server.knowledge_agent_read_token.as_ref().map(|k| {
        if k.is_empty() {
            "****".to_string()
        } else {
            ha_core::mask_secret_middle(k, 2, 2)
        }
    });
    Ok(serde_json::json!({
        "bindAddr": server.bind_addr,
        "apiKey": masked_key,
        "hasApiKey": server.api_key.is_some(),
        "knowledgeAgentReadToken": masked_knowledge_agent_read_token,
        "hasKnowledgeAgentReadToken": server.knowledge_agent_read_token.is_some(),
    }))
}

#[tauri::command]
pub async fn save_server_config(
    config: ha_core::config::EmbeddedServerConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("server", "settings-ui"), move |store| {
        let next = config.merge_over_existing(&store.server);
        store.server = next;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

/// Runtime status of the embedded HTTP/WS server. Shape mirrors
/// `GET /api/server/status` so frontend Transport calls route identically
/// in either mode.
#[tauri::command]
pub async fn get_server_runtime_status() -> Result<serde_json::Value, CmdError> {
    Ok(ha_core::server_status::runtime_status_json(true))
}

// ── Proxy ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_proxy_config() -> Result<provider::ProxyConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.proxy)
}

#[tauri::command]
pub async fn save_proxy_config(config: provider::ProxyConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("proxy", "settings-ui"), move |store| {
        store.proxy = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

/// Outbound proxy probe used by Settings → Proxy → "Test". Body lives in
/// [`ha_core::provider::test::test_proxy`] so the Tauri shell and HTTP route
/// share one source of truth.
#[tauri::command]
pub async fn test_proxy(config: provider::ProxyConfig) -> Result<String, CmdError> {
    ha_core::provider::test::test_proxy(config)
        .await
        .map_err(CmdError::msg)
}

// ── Theme & Language ─────────────────────────────────────────────

#[tauri::command]
pub async fn get_theme() -> Result<String, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.theme)
}

#[tauri::command]
pub async fn set_theme(theme: String) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("theme", "settings-ui"), move |store| {
        store.theme = theme;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_enhanced_focus_indicators() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.enhanced_focus_indicators)
}

#[tauri::command]
pub async fn set_enhanced_focus_indicators(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("focus_indicator", "settings-ui"), move |store| {
        store.enhanced_focus_indicators = enabled;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_language() -> Result<String, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.language)
}

#[tauri::command]
pub async fn set_language(language: String) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("language", "settings-ui"), move |store| {
        store.language = language;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ui_effects_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ui_effects_enabled)
}

#[tauri::command]
pub async fn set_ui_effects_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("ui_effects", "settings-ui"), move |store| {
        store.ui_effects_enabled = enabled;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_prevent_sleep_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.prevent_sleep)
}

#[tauri::command]
pub async fn set_prevent_sleep_enabled(enabled: bool) -> Result<(), CmdError> {
    // The OS sleep assertion is driven by the `config:changed` listener in
    // ha-core (`spawn_keep_awake_listener`); this only persists the toggle.
    ha_core::config::mutate_config_async(("prevent_sleep", "settings-ui"), move |store| {
        store.prevent_sleep = enabled;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_sidebar_display_mode() -> Result<String, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(ha_core::config::normalize_sidebar_ui_mode(
        &store.sidebar_ui_mode,
    ))
}

#[tauri::command]
pub async fn set_sidebar_display_mode(mode: String) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("sidebar_ui_mode", "settings-ui"), move |store| {
        store.sidebar_ui_mode = ha_core::config::normalize_sidebar_ui_mode(&mode);
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_tool_call_narration_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.tool_call_narration_enabled)
}

#[tauri::command]
pub async fn set_tool_call_narration_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("tool_call_narration", "settings-ui"), move |store| {
        store.tool_call_narration_enabled = enabled;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── User Config Commands ─────────────────────────────────────────

#[tauri::command]
pub async fn get_user_config() -> Result<user_config::UserConfig, CmdError> {
    user_config::load_user_config().map_err(Into::into)
}

#[tauri::command]
pub async fn save_user_config(config: user_config::UserConfig) -> Result<(), CmdError> {
    user_config::save_user_config_to_disk(&config).map_err(Into::into)
}

// ── Autostart ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, CmdError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(Into::into)
}

#[tauri::command]
pub async fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), CmdError> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(Into::into)
    } else {
        manager.disable().map_err(Into::into)
    }
}

/// Save a cropped avatar image to `~/.hope-agent/avatars/` and return
/// the absolute path. Bytes come from `transport.prepareFileData()`
/// (serialized as `number[]` in the Tauri IPC path, the `data` field of a
/// multipart form in the HTTP path — see `ha-server/routes/avatars::upload`).
#[tauri::command]
pub async fn save_avatar(data: Vec<u8>, file_name: String) -> Result<String, CmdError> {
    let dir = paths::avatars_dir()?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(&file_name);
    std::fs::write(&path, &data).context("Failed to write avatar")?;

    Ok(path.to_string_lossy().to_string())
}

/// Get the system's IANA timezone name
#[tauri::command]
pub async fn get_system_timezone() -> Result<String, CmdError> {
    // Try reading /etc/localtime symlink (macOS/Linux)
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let path_str = link.to_string_lossy().to_string();
        // Extract timezone from path like /var/db/timezone/zoneinfo/Asia/Shanghai
        if let Some(pos) = path_str.find("zoneinfo/") {
            return Ok(path_str[pos + 9..].to_string());
        }
    }
    // Fallback: TZ env var
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return Ok(tz);
        }
    }
    Ok("UTC".to_string())
}

#[tauri::command]
pub async fn get_tool_timeout() -> Result<u64, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.tool_timeout)
}

#[tauri::command]
pub async fn set_tool_timeout(seconds: u64) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("tool_timeout", "settings-ui"), move |store| {
        store.tool_timeout = seconds;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_timeout_policy_config() -> Result<ha_core::config::TimeoutPolicyConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.timeout_policy)
}

#[tauri::command]
pub async fn save_timeout_policy_config(
    config: ha_core::config::TimeoutPolicyConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("timeout_policy", "settings-ui"), move |store| {
        store.timeout_policy = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_approval_timeout() -> Result<u64, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.permission.approval_timeout_secs)
}

#[tauri::command]
pub async fn get_approval_timeout_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.permission.approval_timeout_enabled)
}

#[tauri::command]
pub async fn set_approval_timeout(seconds: u64) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("approval_timeout", "settings-ui"), move |store| {
        store.permission.approval_timeout_secs = seconds;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn set_approval_timeout_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(
        ("approval_timeout_enabled", "settings-ui"),
        move |store| {
            store.permission.approval_timeout_enabled = enabled;
            Ok(())
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_approval_timeout_action(
) -> Result<ha_core::config::ApprovalTimeoutAction, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.permission.approval_timeout_action)
}

#[tauri::command]
pub async fn set_approval_timeout_action(
    action: ha_core::config::ApprovalTimeoutAction,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("approval_timeout_action", "settings-ui"), move |store| {
        store.permission.approval_timeout_action = action;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_unattended_approval_action(
) -> Result<ha_core::config::UnattendedApprovalAction, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.permission.unattended_approval_action)
}

#[tauri::command]
pub async fn set_unattended_approval_action(
    action: ha_core::config::UnattendedApprovalAction,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(
        ("unattended_approval_action", "settings-ui"),
        move |store| {
            store.permission.unattended_approval_action = action;
            Ok(())
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_tool_result_disk_threshold() -> Result<usize, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.tool_result_disk_threshold.unwrap_or(50_000))
}

#[tauri::command]
pub async fn set_tool_result_disk_threshold(bytes: usize) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(
        ("tool_result_disk_threshold", "settings-ui"),
        move |store| {
            store.tool_result_disk_threshold = if bytes == 0 { Some(0) } else { Some(bytes) };
            Ok(())
        },
    )
    .await
    .map_err(Into::into)
}

// ── Tool Limits ────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLimitsConfig {
    pub max_images: usize,
    pub max_pdfs: usize,
    pub max_vision_pages: usize,
}

#[tauri::command]
pub async fn get_tool_limits() -> Result<ToolLimitsConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(ToolLimitsConfig {
        max_images: store.image.max_images,
        max_pdfs: store.pdf.max_pdfs,
        max_vision_pages: store.pdf.max_vision_pages,
    })
}

#[tauri::command]
pub async fn set_tool_limits(config: ToolLimitsConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("tool_limits", "settings-ui"), move |store| {
        store.image.max_images = config.max_images;
        store.pdf.max_pdfs = config.max_pdfs;
        store.pdf.max_vision_pages = config.max_vision_pages;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Temperature ─────────────────────────────────────────────────

#[tauri::command]
pub async fn get_global_temperature() -> Result<Option<f64>, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.temperature)
}

#[tauri::command]
pub async fn set_global_temperature(temperature: Option<f64>) -> Result<(), CmdError> {
    if let Some(t) = temperature {
        if !(0.0..=2.0).contains(&t) {
            return Err(CmdError::msg("Temperature must be between 0.0 and 2.0"));
        }
    }
    ha_core::config::mutate_config_async(("temperature", "settings-ui"), move |store| {
        store.temperature = temperature;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_global_reasoning_effort() -> Result<String, CmdError> {
    Ok(ha_core::config::cached_config().reasoning_effort.clone())
}

#[tauri::command]
pub async fn set_global_reasoning_effort(effort: String) -> Result<(), CmdError> {
    if !ha_core::agent::is_valid_reasoning_effort(&effort) {
        return Err(CmdError::msg(format!("Invalid reasoning effort: {effort}")));
    }
    ha_core::config::mutate_config_async(("reasoning_effort", "settings-ui"), {
        let effort = effort.clone();
        move |store| {
            store.reasoning_effort = effort;
            Ok(())
        }
    })
    .await?;
    if let Some(cell) = ha_core::get_reasoning_effort_cell() {
        *cell.lock().await = effort;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_plan_subagent() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.plan_subagent)
}

#[tauri::command]
pub async fn set_plan_subagent(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("plan_subagent", "settings-ui"), move |store| {
        store.plan_subagent = enabled;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ask_user_question_timeout() -> Result<u64, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ask_user_question_timeout_secs)
}

#[tauri::command]
pub async fn get_ask_user_question_timeout_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ask_user_question_timeout_enabled)
}

#[tauri::command]
pub async fn set_ask_user_question_timeout(secs: u64) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(
        ("ask_user_question_timeout", "settings-ui"),
        move |store| {
            store.ask_user_question_timeout_secs = secs;
            Ok(())
        },
    )
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn set_ask_user_question_timeout_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(
        ("ask_user_question_timeout_enabled", "settings-ui"),
        move |store| {
            store.ask_user_question_timeout_enabled = enabled;
            Ok(())
        },
    )
    .await
    .map_err(Into::into)
}

// ── Recap Config ────────────────────────────────────────────────

#[tauri::command]
pub async fn get_recap_config() -> Result<ha_core::config::RecapConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.recap)
}

#[tauri::command]
pub async fn save_recap_config(config: ha_core::config::RecapConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("recap", "settings-ui"), move |store| {
        store.recap = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Recall Summary Config ───────────────────────────────────────

#[tauri::command]
pub async fn get_recall_summary_config() -> Result<ha_core::memory::RecallSummaryConfig, CmdError> {
    Ok(ha_core::config::cached_config().recall_summary.clone())
}

#[tauri::command]
pub async fn save_recall_summary_config(
    config: ha_core::memory::RecallSummaryConfig,
) -> Result<ha_core::memory::RecallSummaryConfig, CmdError> {
    let to_save = config.clone();
    ha_core::config::mutate_config_async(("recall_summary", "settings-ui"), move |store| {
        store.recall_summary = to_save.clone();
        Ok(())
    })
    .await?;
    Ok(config)
}

// ── Dreaming Config ─────────────────────────────────────────────

#[tauri::command]
pub async fn get_dreaming_config() -> Result<ha_core::memory::dreaming::DreamingConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.dreaming)
}

#[tauri::command]
pub async fn save_dreaming_config(
    config: ha_core::memory::dreaming::DreamingConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("dreaming", "settings-ui"), move |store| {
        store.dreaming = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn validate_cron_expression(expression: String) -> Result<(), CmdError> {
    ha_core::cron::validate_cron_expression(&expression).map_err(Into::into)
}

// ── Weather ─────────────────────────────────────────────────────

/// Search cities by name using Open-Meteo Geocoding API.
#[tauri::command]
pub async fn geocode_search(
    query: String,
    language: Option<String>,
) -> Result<Vec<crate::weather::GeoResult>, CmdError> {
    let lang = language.as_deref().unwrap_or("zh");
    crate::weather::geocode_search(&query, lang)
        .await
        .map_err(Into::into)
}

/// Fetch real-time weather preview explicitly for the provided settings, bypassing config layout.
#[tauri::command]
pub async fn preview_weather(
    lat: f64,
    lon: f64,
    city: String,
) -> Result<crate::weather::WeatherData, CmdError> {
    crate::weather::fetch_weather(lat, lon, &city, 1)
        .await
        .map(|w| w.current)
        .map_err(Into::into)
}

/// Get the currently cached weather data for frontend preview.
#[tauri::command]
pub async fn get_current_weather() -> Result<Option<crate::weather::WeatherData>, CmdError> {
    Ok(crate::weather::get_cached_weather().await)
}

/// Force refresh weather cache and return fresh data.
#[tauri::command]
pub async fn refresh_weather() -> Result<Option<crate::weather::WeatherData>, CmdError> {
    crate::weather::force_refresh_weather()
        .await
        .map_err(Into::into)
}

// ── Async Tools ───────────────────────────────────────────────────

#[tauri::command]
pub async fn get_async_tools_config() -> Result<ha_core::config::AsyncToolsConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.async_tools)
}

#[tauri::command]
pub async fn save_async_tools_config(
    config: ha_core::config::AsyncToolsConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("async_tools", "settings-ui"), move |store| {
        store.async_tools = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Cron (scheduled tasks) ────────────────────────────────────────

#[tauri::command]
pub async fn get_cron_config() -> Result<ha_core::config::CronConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.cron)
}

#[tauri::command]
pub async fn save_cron_config(config: ha_core::config::CronConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("cron", "settings-ui"), move |store| {
        store.cron = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

// ── Deferred Tool Loading ─────────────────────────────────────────

#[tauri::command]
pub async fn get_deferred_tools_config() -> Result<ha_core::config::DeferredToolsConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.deferred_tools)
}

#[tauri::command]
pub async fn save_deferred_tools_config(
    config: ha_core::config::DeferredToolsConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("deferred_tools", "settings-ui"), move |store| {
        store.deferred_tools = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

/// Detect user location automatically (CoreLocation → IP fallback).
#[tauri::command]
pub async fn detect_location() -> Result<crate::weather::DetectedLocation, CmdError> {
    crate::weather::detect_location().await.map_err(Into::into)
}

// ── Behavior Awareness ────────────────────────────────────────────

#[tauri::command]
pub async fn get_awareness_config() -> Result<ha_core::awareness::AwarenessConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.awareness)
}

#[tauri::command]
pub async fn save_awareness_config(
    config: ha_core::awareness::AwarenessConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("awareness", "settings-ui"), move |store| {
        store.awareness = config;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_awareness_override(
    session_id: String,
) -> Result<Option<String>, CmdError> {
    let db =
        ha_core::get_session_db().ok_or_else(|| CmdError::msg("Session DB not initialized"))?;
    db.run(move |db| db.get_session_awareness_config_json(&session_id))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_session_awareness_override(
    session_id: String,
    json: Option<String>,
) -> Result<(), CmdError> {
    // Validate before persisting.
    if let Some(ref j) = json {
        if !j.trim().is_empty() {
            let base = ha_core::awareness::AwarenessConfig::default();
            ha_core::awareness::config::validate_override(&base, j)
                .context("invalid override JSON")?;
        }
    }
    let db =
        ha_core::get_session_db().ok_or_else(|| CmdError::msg("Session DB not initialized"))?;
    db.run(move |db| db.set_session_awareness_config_json(&session_id, json.as_deref()))
        .await
        .map_err(Into::into)
}

/// Read the hooks settings for the Settings → Hooks GUI: the
/// `disable_all_hooks` master switch + the user-scope `hooks` map. Project /
/// local / managed scopes are file-based and not surfaced here.
#[tauri::command]
pub async fn get_hooks_config() -> Result<ha_core::hooks::config::HooksSettings, CmdError> {
    let cfg = ha_core::config::cached_config();
    Ok(ha_core::hooks::config::HooksSettings {
        disable_all_hooks: cfg.disable_all_hooks,
        allow_project_scope: cfg.hooks_allow_project_scope,
        hooks: cfg.hooks.clone(),
    })
}

/// Persist the user-scope hooks settings. Writes both the master switch and the
/// `hooks` map through the config contract; `config:changed` then rebuilds the
/// hook registry. The GUI is the only writer for user-scope hooks (the
/// `ha-settings` skill is read-only — hooks run arbitrary commands).
#[tauri::command]
pub async fn save_hooks_config(
    config: ha_core::hooks::config::HooksSettings,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config_async(("hooks", "settings-ui"), move |store| {
        store.disable_all_hooks = config.disable_all_hooks;
        store.hooks_allow_project_scope = config.allow_project_scope;
        store.hooks = config.hooks;
        Ok(())
    })
    .await
    .map_err(Into::into)
}

use crate::chat_engine::save_agent_context;
use crate::commands::CmdError;
use crate::context_compact;
use crate::paths;
use crate::provider;
use crate::tools;
use crate::user_config;
use crate::AppState;
use anyhow::Context;

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
    ha_core::config::mutate_config(("web_search", "settings-ui"), |store| {
        store.web_search = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("web_fetch", "settings-ui"), |store| {
        store.web_fetch = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ssrf_config() -> Result<ha_core::security::ssrf::SsrfConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ssrf)
}

#[tauri::command]
pub async fn save_ssrf_config(config: ha_core::security::ssrf::SsrfConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("security.ssrf", "settings-ui"), |store| {
        store.ssrf = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_compact_config() -> Result<context_compact::CompactConfig, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.compact)
}

#[tauri::command]
pub async fn save_compact_config(config: context_compact::CompactConfig) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("compact", "settings-ui"), |store| {
        store.compact = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("session_title", "settings-ui"), |store| {
        store.session_title = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("notification", "settings-ui"), |store| {
        store.notification = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("image_generate", "settings-ui"), |store| {
        store.image_generate = config;
        Ok(())
    })
    .map_err(Into::into)
}

/// Core logic for manual context compaction. Usable from both Tauri commands
/// and internal callers (e.g. channel worker).
pub(crate) async fn compact_context_now_core(
    session_id: &str,
    state: &AppState,
) -> Result<context_compact::CompactResult, CmdError> {
    let agent = state.agent.lock().await;
    let agent = agent
        .as_ref()
        .ok_or_else(|| CmdError::msg("No active agent"))?;

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

    let store = ha_core::config::load_config()?;
    let compact_config = store.compact;

    let system_prompt_estimate = "system";
    let max_tokens: u32 = 16384;

    // Run Tier 1 + Tier 2
    let result = context_compact::compact_if_needed(
        &mut history,
        system_prompt_estimate,
        agent.get_context_window(),
        max_tokens,
        &compact_config,
    );

    // If thresholds not reached, force compaction with lowered thresholds
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

            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "info",
                    "context",
                    "compact::manual",
                    &format!(
                        "Manual compaction: {} → {} tokens, {} affected",
                        forced_result.tokens_before,
                        forced_result.tokens_after,
                        forced_result.messages_affected
                    ),
                    None,
                    None,
                    None,
                );
            }
        }
        return Ok(forced_result);
    }

    agent.set_conversation_history(history);
    save_agent_context(&state.session_db, session_id, agent);

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "info",
            "context",
            "compact::manual",
            &format!(
                "Manual compaction: tier={}, {} → {} tokens, {} affected",
                result.tier_applied,
                result.tokens_before,
                result.tokens_after,
                result.messages_affected
            ),
            None,
            None,
            None,
        );
    }

    Ok(result)
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

    ha_core::config::mutate_config(("shortcuts", "settings-ui"), |store| {
        store.shortcuts = config.clone();
        Ok(())
    })?;

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
    Ok(serde_json::json!({
        "bindAddr": server.bind_addr,
        "apiKey": masked_key,
        "hasApiKey": server.api_key.is_some(),
    }))
}

#[tauri::command]
pub async fn save_server_config(
    config: ha_core::config::EmbeddedServerConfig,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("server", "settings-ui"), |store| {
        store.server = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("proxy", "settings-ui"), |store| {
        store.proxy = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("theme", "settings-ui"), |store| {
        store.theme = theme;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_language() -> Result<String, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.language)
}

#[tauri::command]
pub async fn set_language(language: String) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("language", "settings-ui"), |store| {
        store.language = language;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ui_effects_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ui_effects_enabled)
}

#[tauri::command]
pub async fn set_ui_effects_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("ui_effects", "settings-ui"), |store| {
        store.ui_effects_enabled = enabled;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_tool_call_narration_enabled() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.tool_call_narration_enabled)
}

#[tauri::command]
pub async fn set_tool_call_narration_enabled(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("tool_call_narration", "settings-ui"), |store| {
        store.tool_call_narration_enabled = enabled;
        Ok(())
    })
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
    ha_core::config::mutate_config(("tool_timeout", "settings-ui"), |store| {
        store.tool_timeout = seconds;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_approval_timeout() -> Result<u64, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.approval_timeout_secs)
}

#[tauri::command]
pub async fn set_approval_timeout(seconds: u64) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("approval_timeout", "settings-ui"), |store| {
        store.approval_timeout_secs = seconds;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_approval_timeout_action(
) -> Result<ha_core::config::ApprovalTimeoutAction, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.approval_timeout_action)
}

#[tauri::command]
pub async fn set_approval_timeout_action(
    action: ha_core::config::ApprovalTimeoutAction,
) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("approval_timeout_action", "settings-ui"), |store| {
        store.approval_timeout_action = action;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_tool_result_disk_threshold() -> Result<usize, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.tool_result_disk_threshold.unwrap_or(50_000))
}

#[tauri::command]
pub async fn set_tool_result_disk_threshold(bytes: usize) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("tool_result_disk_threshold", "settings-ui"), |store| {
        store.tool_result_disk_threshold = if bytes == 0 { Some(0) } else { Some(bytes) };
        Ok(())
    })
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
    ha_core::config::mutate_config(("tool_limits", "settings-ui"), |store| {
        store.image.max_images = config.max_images;
        store.pdf.max_pdfs = config.max_pdfs;
        store.pdf.max_vision_pages = config.max_vision_pages;
        Ok(())
    })
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
    ha_core::config::mutate_config(("temperature", "settings-ui"), |store| {
        store.temperature = temperature;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_plan_subagent() -> Result<bool, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.plan_subagent)
}

#[tauri::command]
pub async fn set_plan_subagent(enabled: bool) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("plan_subagent", "settings-ui"), |store| {
        store.plan_subagent = enabled;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_ask_user_question_timeout() -> Result<u64, CmdError> {
    let store = ha_core::config::load_config()?;
    Ok(store.ask_user_question_timeout_secs)
}

#[tauri::command]
pub async fn set_ask_user_question_timeout(secs: u64) -> Result<(), CmdError> {
    ha_core::config::mutate_config(("ask_user_question_timeout", "settings-ui"), |store| {
        store.ask_user_question_timeout_secs = secs;
        Ok(())
    })
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
    ha_core::config::mutate_config(("recap", "settings-ui"), |store| {
        store.recap = config;
        Ok(())
    })
    .map_err(Into::into)
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
    ha_core::config::mutate_config(("async_tools", "settings-ui"), |store| {
        store.async_tools = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("deferred_tools", "settings-ui"), |store| {
        store.deferred_tools = config;
        Ok(())
    })
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
    ha_core::config::mutate_config(("awareness", "settings-ui"), |store| {
        store.awareness = config;
        Ok(())
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_session_awareness_override(
    session_id: String,
) -> Result<Option<String>, CmdError> {
    let db =
        ha_core::get_session_db().ok_or_else(|| CmdError::msg("Session DB not initialized"))?;
    db.get_session_awareness_config_json(&session_id)
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
    db.set_session_awareness_config_json(&session_id, json.as_deref())
        .map_err(Into::into)
}

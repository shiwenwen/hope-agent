use crate::chat_engine::save_agent_context;
use crate::context_compact;
use crate::paths;
use crate::provider;
use crate::tools;
use crate::user_config;
use crate::AppState;

#[tauri::command]
pub async fn get_web_search_config() -> Result<tools::web_search::WebSearchConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    let mut config = store.web_search;
    tools::web_search::backfill_providers(&mut config);
    Ok(config)
}

#[tauri::command]
pub async fn save_web_search_config(
    config: tools::web_search::WebSearchConfig,
) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.web_search = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_web_fetch_config() -> Result<tools::web_fetch::WebFetchConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.web_fetch)
}

#[tauri::command]
pub async fn save_web_fetch_config(config: tools::web_fetch::WebFetchConfig) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.web_fetch = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_compact_config() -> Result<context_compact::CompactConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.compact)
}

#[tauri::command]
pub async fn save_compact_config(config: context_compact::CompactConfig) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.compact = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_notification_config() -> Result<provider::NotificationConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.notification)
}

#[tauri::command]
pub async fn save_notification_config(config: provider::NotificationConfig) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.notification = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_generate_config() -> Result<tools::image_generate::ImageGenConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    let mut config = store.image_generate;
    tools::image_generate::backfill_providers(&mut config);
    Ok(config)
}

#[tauri::command]
pub async fn save_image_generate_config(
    config: tools::image_generate::ImageGenConfig,
) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.image_generate = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

/// Core logic for manual context compaction. Usable from both Tauri commands
/// and internal callers (e.g. channel worker).
pub(crate) async fn compact_context_now_core(
    session_id: &str,
    state: &AppState,
) -> Result<context_compact::CompactResult, String> {
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

    let store = provider::load_store().map_err(|e| e.to_string())?;
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
) -> Result<context_compact::CompactResult, String> {
    compact_context_now_core(&session_id, &state).await
}

// ── Shortcuts ────────────────────────────────────────────────────

/// Temporarily unregister all global shortcuts (for recording mode)
/// or re-register them from config.
#[tauri::command]
pub async fn set_shortcuts_paused(app: tauri::AppHandle, paused: bool) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let manager = app.global_shortcut();

    if paused {
        // Clear pending chord state and unregister all
        *crate::chord_state().lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = manager.unregister_all();
    } else {
        // Re-register from saved config
        let store = provider::load_store().map_err(|e| e.to_string())?;
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
pub async fn get_shortcut_config() -> Result<provider::ShortcutConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.shortcuts)
}

#[tauri::command]
pub async fn save_shortcut_config(
    app: tauri::AppHandle,
    config: provider::ShortcutConfig,
) -> Result<(), String> {
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
                return Err(format!("Invalid shortcut key combination: {}", part));
            }
        }
    }

    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.shortcuts = config.clone();
    provider::save_store(&store).map_err(|e| e.to_string())?;

    // Clear any pending chord state
    *crate::chord_state().lock().unwrap_or_else(|e| e.into_inner()) = None;

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

// ── Proxy ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_proxy_config() -> Result<provider::ProxyConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.proxy)
}

#[tauri::command]
pub async fn save_proxy_config(config: provider::ProxyConfig) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.proxy = config;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_proxy(config: provider::ProxyConfig) -> Result<String, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15));
    builder = provider::apply_proxy_from_config(builder, &config);
    let client = builder
        .build()
        .map_err(|e| format!("Failed to build client: {}", e))?;

    let start = std::time::Instant::now();
    let resp = client
        .get("https://httpbin.org/ip")
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let elapsed = start.elapsed().as_millis();
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status));
    }
    let body = resp.text().await.unwrap_or_default();
    Ok(format!("OK ({}ms)\n{}", elapsed, body))
}

// ── Theme & Language ─────────────────────────────────────────────

#[tauri::command]
pub async fn get_theme() -> Result<String, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.theme)
}

#[tauri::command]
pub async fn set_theme(theme: String) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.theme = theme;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_language() -> Result<String, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.language)
}

#[tauri::command]
pub async fn set_language(language: String) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.language = language;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ui_effects_enabled() -> Result<bool, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.ui_effects_enabled)
}

#[tauri::command]
pub async fn set_ui_effects_enabled(enabled: bool) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.ui_effects_enabled = enabled;
    provider::save_store(&store).map_err(|e| e.to_string())
}

// ── User Config Commands ─────────────────────────────────────────

#[tauri::command]
pub async fn get_user_config() -> Result<user_config::UserConfig, String> {
    user_config::load_user_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_user_config(config: user_config::UserConfig) -> Result<(), String> {
    user_config::save_user_config_to_disk(&config).map_err(|e| e.to_string())
}

// ── Autostart ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

/// Save a cropped avatar image (base64-encoded) to ~/.opencomputer/avatars/
/// Returns the absolute path to the saved file.
#[tauri::command]
pub async fn save_avatar(image_data: String, file_name: String) -> Result<String, String> {
    use base64::Engine;

    let dir = paths::avatars_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image_data)
        .map_err(|e| format!("Base64 decode error: {}", e))?;

    let path = dir.join(&file_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("Failed to write avatar: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

/// Get the system's IANA timezone name
#[tauri::command]
pub async fn get_system_timezone() -> Result<String, String> {
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
pub async fn get_tool_timeout() -> Result<u64, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.tool_timeout)
}

#[tauri::command]
pub async fn set_tool_timeout(seconds: u64) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.tool_timeout = seconds;
    provider::save_store(&store).map_err(|e| e.to_string())
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
pub async fn get_tool_limits() -> Result<ToolLimitsConfig, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(ToolLimitsConfig {
        max_images: store.image.max_images,
        max_pdfs: store.pdf.max_pdfs,
        max_vision_pages: store.pdf.max_vision_pages,
    })
}

#[tauri::command]
pub async fn set_tool_limits(config: ToolLimitsConfig) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.image.max_images = config.max_images;
    store.pdf.max_pdfs = config.max_pdfs;
    store.pdf.max_vision_pages = config.max_vision_pages;
    provider::save_store(&store).map_err(|e| e.to_string())
}

// ── Temperature ─────────────────────────────────────────────────

#[tauri::command]
pub async fn get_global_temperature() -> Result<Option<f64>, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.temperature)
}

#[tauri::command]
pub async fn set_global_temperature(temperature: Option<f64>) -> Result<(), String> {
    if let Some(t) = temperature {
        if !(0.0..=2.0).contains(&t) {
            return Err("Temperature must be between 0.0 and 2.0".to_string());
        }
    }
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.temperature = temperature;
    provider::save_store(&store).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plan_subagent() -> Result<bool, String> {
    let store = provider::load_store().map_err(|e| e.to_string())?;
    Ok(store.plan_subagent)
}

#[tauri::command]
pub async fn set_plan_subagent(enabled: bool) -> Result<(), String> {
    let mut store = provider::load_store().map_err(|e| e.to_string())?;
    store.plan_subagent = enabled;
    provider::save_store(&store).map_err(|e| e.to_string())
}

// ── Weather ─────────────────────────────────────────────────────

/// Search cities by name using Open-Meteo Geocoding API.
#[tauri::command]
pub async fn geocode_search(
    query: String,
    language: Option<String>,
) -> Result<Vec<crate::weather::GeoResult>, String> {
    let lang = language.as_deref().unwrap_or("zh");
    crate::weather::geocode_search(&query, lang)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch real-time weather preview explicitly for the provided settings, bypassing config layout.
#[tauri::command]
pub async fn preview_weather(
    lat: f64,
    lon: f64,
    city: String,
) -> Result<crate::weather::WeatherData, String> {
    crate::weather::fetch_weather(lat, lon, &city, 1)
        .await
        .map(|w| w.current)
        .map_err(|e| e.to_string())
}

/// Get the currently cached weather data for frontend preview.
#[tauri::command]
pub async fn get_current_weather() -> Result<Option<crate::weather::WeatherData>, String> {
    Ok(crate::weather::get_cached_weather().await)
}

/// Force refresh weather cache and return fresh data.
#[tauri::command]
pub async fn refresh_weather() -> Result<Option<crate::weather::WeatherData>, String> {
    crate::weather::force_refresh_weather()
        .await
        .map_err(|e| e.to_string())
}

/// Detect user location automatically (CoreLocation → IP fallback).
#[tauri::command]
pub async fn detect_location(
    app_handle: tauri::AppHandle,
) -> Result<crate::weather::DetectedLocation, String> {
    crate::weather::detect_location(app_handle)
        .await
        .map_err(|e| e.to_string())
}

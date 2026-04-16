use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config;
use crate::user_config;

const BLOCKED_UPDATE_CATEGORIES: &[&str] = &["active_model", "fallback_models"];

// ── get_settings ────────────────────────────────────────────────

pub(crate) async fn tool_get_settings(args: &Value) -> Result<String> {
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    if category == "all" {
        return get_all_overview();
    }

    let value = read_category(category)?;
    Ok(serde_json::to_string_pretty(&json!({
        "category": category,
        "settings": value,
    }))?)
}

fn read_category(category: &str) -> Result<Value> {
    let cfg = config::cached_config();

    match category {
        "user" => {
            let uc = user_config::load_user_config()?;
            Ok(serde_json::to_value(&uc)?)
        }
        "theme" => Ok(json!({ "theme": cfg.theme })),
        "language" => Ok(json!({ "language": cfg.language })),
        "ui_effects" => Ok(json!({ "uiEffectsEnabled": cfg.ui_effects_enabled })),
        "proxy" => Ok(serde_json::to_value(&cfg.proxy)?),
        "web_search" => Ok(serde_json::to_value(&cfg.web_search)?),
        "web_fetch" => Ok(serde_json::to_value(&cfg.web_fetch)?),
        "compact" => Ok(serde_json::to_value(&cfg.compact)?),
        "notification" => Ok(serde_json::to_value(&cfg.notification)?),
        "temperature" => Ok(json!({ "temperature": cfg.temperature })),
        "tool_timeout" => Ok(json!({ "toolTimeout": cfg.tool_timeout })),
        "approval" => Ok(json!({
            "approvalTimeoutSecs": cfg.approval_timeout_secs,
            "approvalTimeoutAction": cfg.approval_timeout_action,
        })),
        "image_generate" => Ok(serde_json::to_value(&cfg.image_generate)?),
        "canvas" => Ok(serde_json::to_value(&cfg.canvas)?),
        "image" => Ok(serde_json::to_value(&cfg.image)?),
        "pdf" => Ok(serde_json::to_value(&cfg.pdf)?),
        "async_tools" => Ok(serde_json::to_value(&cfg.async_tools)?),
        "deferred_tools" => Ok(serde_json::to_value(&cfg.deferred_tools)?),
        "memory_extract" => Ok(serde_json::to_value(&cfg.memory_extract)?),
        "memory_selection" => Ok(serde_json::to_value(&cfg.memory_selection)?),
        "embedding" => Ok(serde_json::to_value(&cfg.embedding)?),
        "embedding_cache" => Ok(serde_json::to_value(&cfg.embedding_cache)?),
        "dedup" => Ok(serde_json::to_value(&cfg.dedup)?),
        "hybrid_search" => Ok(serde_json::to_value(&cfg.hybrid_search)?),
        "temporal_decay" => Ok(serde_json::to_value(&cfg.temporal_decay)?),
        "mmr" => Ok(serde_json::to_value(&cfg.mmr)?),
        "recap" => Ok(serde_json::to_value(&cfg.recap)?),
        "cross_session" => Ok(serde_json::to_value(&cfg.cross_session)?),
        "shortcuts" => Ok(serde_json::to_value(&cfg.shortcuts)?),
        "active_model" => Ok(serde_json::to_value(&cfg.active_model)?),
        "fallback_models" => Ok(serde_json::to_value(&cfg.fallback_models)?),
        "skills" => Ok(json!({
            "extraSkillsDirs": cfg.extra_skills_dirs,
            "disabledSkills": cfg.disabled_skills,
            "skillEnvCheck": cfg.skill_env_check,
        })),
        _ => bail!("Unknown settings category: '{category}'"),
    }
}

fn get_all_overview() -> Result<String> {
    let cfg = config::cached_config();
    let uc = user_config::load_user_config().unwrap_or_default();

    let overview = json!({
        "user": {
            "name": uc.name,
            "role": uc.role,
            "language": uc.language,
            "timezone": uc.timezone,
            "weatherEnabled": uc.weather_enabled,
            "weatherCity": uc.weather_city,
        },
        "theme": cfg.theme,
        "language": cfg.language,
        "uiEffectsEnabled": cfg.ui_effects_enabled,
        "temperature": cfg.temperature,
        "toolTimeout": cfg.tool_timeout,
        "approvalTimeoutSecs": cfg.approval_timeout_secs,
        "notification": { "enabled": cfg.notification.enabled },
        "proxy": {
            "mode": cfg.proxy.mode,
            "url": cfg.proxy.url,
        },
        "compact": {
            "enabled": cfg.compact.enabled,
            "cacheTtlSecs": cfg.compact.cache_ttl_secs,
        },
        "asyncTools": { "enabled": cfg.async_tools.enabled },
        "deferredTools": { "enabled": cfg.deferred_tools.enabled },
        "crossSession": { "enabled": cfg.cross_session.enabled },
        "activeModel": cfg.active_model,
        "fallbackModels": cfg.fallback_models.len(),
        "skills": {
            "extraDirs": cfg.extra_skills_dirs.len(),
            "disabled": cfg.disabled_skills,
        },
    });

    Ok(serde_json::to_string_pretty(&json!({
        "category": "all",
        "overview": overview,
        "hint": "Use get_settings with a specific category for full details.",
    }))?)
}

// ── update_settings ─────────────────────────────────────────────

pub(crate) async fn tool_update_settings(args: &Value) -> Result<String> {
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: category"))?;

    let values = args
        .get("values")
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: values"))?;

    if !values.is_object() {
        bail!("'values' must be a JSON object");
    }

    if BLOCKED_UPDATE_CATEGORIES.contains(&category) {
        bail!(
            "Category '{category}' cannot be modified through this tool for safety reasons. \
             Please guide the user to change it in the Settings UI.",
        );
    }

    if category == "all" {
        bail!("Cannot update 'all' — specify a single category.");
    }

    if category == "user" {
        return update_user_config(values);
    }

    update_app_config(category, values)
}

fn update_user_config(values: &Value) -> Result<String> {
    let uc = user_config::load_user_config()?;
    let mut uc_json = serde_json::to_value(&uc)?;
    crate::merge_json(&mut uc_json, values.clone());
    let updated: user_config::UserConfig = serde_json::from_value(uc_json.clone())?;
    user_config::save_user_config_to_disk(&updated)?;

    // Notify frontend about user config change
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": "user" }),
        );
    }

    // Hot-reload: refresh weather cache if weather-related fields changed
    trigger_weather_refresh_if_needed(values);

    Ok(serde_json::to_string_pretty(&json!({
        "category": "user",
        "updated": true,
        "settings": uc_json,
    }))?)
}

fn update_app_config(category: &str, values: &Value) -> Result<String> {
    let mut store = config::load_config()?;

    match category {
        "theme" => {
            if let Some(v) = values.get("theme").and_then(|v| v.as_str()) {
                match v {
                    "auto" | "light" | "dark" => store.theme = v.to_string(),
                    _ => bail!("Invalid theme: '{v}'. Must be auto/light/dark."),
                }
            }
        }
        "language" => {
            if let Some(v) = values.get("language").and_then(|v| v.as_str()) {
                store.language = v.to_string();
            }
        }
        "ui_effects" => {
            if let Some(v) = values.get("uiEffectsEnabled").and_then(|v| v.as_bool()) {
                store.ui_effects_enabled = v;
            }
        }
        "temperature" => {
            if let Some(v) = values.get("temperature") {
                if v.is_null() {
                    store.temperature = None;
                } else if let Some(t) = v.as_f64() {
                    if !(0.0..=2.0).contains(&t) {
                        bail!("Temperature must be between 0.0 and 2.0, got {t}");
                    }
                    store.temperature = Some(t);
                }
            }
        }
        "tool_timeout" => {
            if let Some(v) = values.get("toolTimeout").and_then(|v| v.as_u64()) {
                store.tool_timeout = v;
            }
        }
        "approval" => {
            if let Some(v) = values.get("approvalTimeoutSecs").and_then(|v| v.as_u64()) {
                store.approval_timeout_secs = v;
            }
            if let Some(v) = values.get("approvalTimeoutAction") {
                store.approval_timeout_action = serde_json::from_value(v.clone())?;
            }
        }
        "proxy" => merge_field(&mut store.proxy, values)?,
        "web_search" => merge_field(&mut store.web_search, values)?,
        "web_fetch" => merge_field(&mut store.web_fetch, values)?,
        "compact" => merge_field(&mut store.compact, values)?,
        "notification" => merge_field(&mut store.notification, values)?,
        "image_generate" => merge_field(&mut store.image_generate, values)?,
        "canvas" => merge_field(&mut store.canvas, values)?,
        "image" => merge_field(&mut store.image, values)?,
        "pdf" => merge_field(&mut store.pdf, values)?,
        "async_tools" => merge_field(&mut store.async_tools, values)?,
        "deferred_tools" => merge_field(&mut store.deferred_tools, values)?,
        "memory_extract" => merge_field(&mut store.memory_extract, values)?,
        "memory_selection" => merge_field(&mut store.memory_selection, values)?,
        "embedding" => merge_field(&mut store.embedding, values)?,
        "embedding_cache" => merge_field(&mut store.embedding_cache, values)?,
        "dedup" => merge_field(&mut store.dedup, values)?,
        "hybrid_search" => merge_field(&mut store.hybrid_search, values)?,
        "temporal_decay" => merge_field(&mut store.temporal_decay, values)?,
        "mmr" => merge_field(&mut store.mmr, values)?,
        "recap" => merge_field(&mut store.recap, values)?,
        "cross_session" => merge_field(&mut store.cross_session, values)?,
        "shortcuts" => merge_field(&mut store.shortcuts, values)?,
        "skills" => {
            if let Some(v) = values.get("extraSkillsDirs") {
                store.extra_skills_dirs = serde_json::from_value(v.clone())?;
            }
            if let Some(v) = values.get("disabledSkills") {
                store.disabled_skills = serde_json::from_value(v.clone())?;
            }
            if let Some(v) = values.get("skillEnvCheck").and_then(|v| v.as_bool()) {
                store.skill_env_check = v;
            }
        }
        _ => bail!("Unknown settings category: '{category}'"),
    }

    config::save_config(&store)?;

    // Notify frontend about config change so UI can react immediately
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "config:changed",
            serde_json::json!({ "category": category }),
        );
    }

    // Backend hot-reload: trigger side-effects for categories that cache state
    trigger_backend_hot_reload(category, &store);

    // Return the saved value directly from the mutated store (avoids re-reading cache)
    let updated_value = read_category(category)?;
    Ok(serde_json::to_string_pretty(&json!({
        "category": category,
        "updated": true,
        "settings": updated_value,
    }))?)
}

/// Trigger backend hot-reload side-effects for categories that cache state in memory.
fn trigger_backend_hot_reload(category: &str, store: &config::AppConfig) {
    match category {
        "embedding" => {
            // Re-initialize embedding provider when config changes
            if let Some(backend) = crate::get_memory_backend() {
                if store.embedding.enabled {
                    match crate::memory::create_embedding_provider(&store.embedding) {
                        Ok(provider) => {
                            backend.set_embedder(provider);
                            app_info!(
                                "settings",
                                "hot_reload",
                                "Embedding provider re-initialized after config change"
                            );
                        }
                        Err(e) => {
                            app_warn!(
                                "settings",
                                "hot_reload",
                                "Failed to re-initialize embedding provider: {}",
                                e
                            );
                        }
                    }
                } else {
                    backend.clear_embedder();
                    app_info!(
                        "settings",
                        "hot_reload",
                        "Embedding provider cleared (disabled)"
                    );
                }
            }
        }
        "web_search" => {
            // SearXNG config may affect Docker container — no cached state to invalidate,
            // but weather system may use web search indirectly. No action needed.
        }
        _ => {} // Other categories: config cache (ArcSwap) already updated by save_config
    }
}

/// Trigger weather cache refresh when user_config weather settings change.
fn trigger_weather_refresh_if_needed(values: &Value) {
    let dominated_keys = [
        "weather_enabled",
        "weatherEnabled",
        "weather_city",
        "weatherCity",
        "weather_latitude",
        "weatherLatitude",
        "weather_longitude",
        "weatherLongitude",
    ];
    let needs_refresh = dominated_keys.iter().any(|k| values.get(k).is_some());
    if needs_refresh {
        tokio::spawn(async {
            if let Err(e) = crate::weather::force_refresh_weather().await {
                app_warn!(
                    "settings",
                    "hot_reload",
                    "Failed to refresh weather after user config change: {}",
                    e
                );
            }
        });
    }
}

/// Merge `patch` into a serializable field using deep JSON merge, then deserialize back.
fn merge_field<T>(field: &mut T, patch: &Value) -> Result<()>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut current = serde_json::to_value(&*field)?;
    crate::merge_json(&mut current, patch.clone());
    *field = serde_json::from_value(current)?;
    Ok(())
}

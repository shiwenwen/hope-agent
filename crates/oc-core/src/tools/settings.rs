use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config;
use crate::user_config;

const BLOCKED_UPDATE_CATEGORIES: &[&str] = &["active_model", "fallback_models"];

/// Risk classification for a settings category.
/// The skill / model uses this to decide whether to double-confirm with the user.
/// - `low`: cosmetic / preference changes, trivially reversible
/// - `medium`: behavioral changes that may affect cost, context, or output quality
/// - `high`: security, network exposure, global keybindings, or changes that require restart
fn risk_level(category: &str) -> &'static str {
    match category {
        // ── LOW ────────────────────────────────────────────────
        "user" | "theme" | "language" | "ui_effects" | "notification" | "canvas" | "image"
        | "pdf" | "image_generate" | "temperature" | "tool_timeout" => "low",

        // ── MEDIUM ─────────────────────────────────────────────
        "compact"
        | "memory_extract"
        | "memory_selection"
        | "memory_budget"
        | "embedding_cache"
        | "dedup"
        | "hybrid_search"
        | "temporal_decay"
        | "mmr"
        | "recap"
        | "awareness"
        | "web_fetch"
        | "web_search"
        | "deferred_tools"
        | "async_tools"
        | "approval"
        | "tool_result_disk_threshold"
        | "ask_user_question_timeout"
        | "plan"
        | "skills_auto_review"
        | "recall_summary"
        | "tool_call_narration"
        | "teams" => "medium",

        // ── HIGH ───────────────────────────────────────────────
        "proxy" | "embedding" | "shortcuts" | "skills" | "server" | "acp_control"
        | "skill_env" | "security.ssrf" => "high",

        // Read-only categories — no risk since they can't be mutated here.
        "active_model" | "fallback_models" | "all" => "low",

        _ => "medium",
    }
}

/// Human-readable note about side effects (e.g. "requires app restart").
fn side_effect_note(category: &str) -> Option<&'static str> {
    match category {
        "server" => Some("Changes take effect on next app restart."),
        "shortcuts" => Some("Global shortcut re-registration happens immediately; conflicts may silently fail."),
        "embedding" => {
            Some("Switching embedding provider/model may invalidate existing vector indexes.")
        }
        "proxy" => Some("Proxy change affects ALL outgoing HTTP requests immediately."),
        "skill_env" => Some("Environment variables may contain secrets; values are stored in plaintext in config.json."),
        "acp_control" => Some("Affects external agent delegation; restart recommended after backend changes."),
        "teams" => Some(
            "Team templates are rows in the team_templates DB table, not AppConfig fields. \
             To modify, pass values = { \"action\": \"save\", \"template\": {...} } or \
             { \"action\": \"delete\", \"templateId\": \"...\" }. A saved template becomes \
             discoverable by the model via team(action=\"list_templates\")."
        ),
        "memory_budget" => Some(
            "Reducing totalChars may hide parts of memory.md from the system prompt. \
             Full content is still retrievable via recall_memory / memory_get tools."
        ),
        _ => None,
    }
}

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
    let mut response = json!({
        "category": category,
        "riskLevel": risk_level(category),
        "settings": value,
    });
    if let Some(note) = side_effect_note(category) {
        response["sideEffect"] = json!(note);
    }
    Ok(serde_json::to_string_pretty(&response)?)
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
        "security.ssrf" => Ok(serde_json::to_value(&cfg.ssrf)?),
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
        "memory_budget" => Ok(serde_json::to_value(&cfg.memory_budget)?),
        "embedding" => Ok(serde_json::to_value(&cfg.embedding)?),
        "embedding_cache" => Ok(serde_json::to_value(&cfg.embedding_cache)?),
        "dedup" => Ok(serde_json::to_value(&cfg.dedup)?),
        "hybrid_search" => Ok(serde_json::to_value(&cfg.hybrid_search)?),
        "temporal_decay" => Ok(serde_json::to_value(&cfg.temporal_decay)?),
        "mmr" => Ok(serde_json::to_value(&cfg.mmr)?),
        "recap" => Ok(serde_json::to_value(&cfg.recap)?),
        "awareness" => Ok(serde_json::to_value(&cfg.awareness)?),
        "shortcuts" => Ok(serde_json::to_value(&cfg.shortcuts)?),
        "active_model" => Ok(serde_json::to_value(&cfg.active_model)?),
        "fallback_models" => Ok(serde_json::to_value(&cfg.fallback_models)?),
        "skills" => Ok(json!({
            "extraSkillsDirs": cfg.extra_skills_dirs,
            "disabledSkills": cfg.disabled_skills,
            "skillEnvCheck": cfg.skill_env_check,
        })),
        "server" => Ok(serde_json::to_value(&cfg.server)?),
        "acp_control" => Ok(serde_json::to_value(&cfg.acp_control)?),
        "skill_env" => Ok(serde_json::to_value(&cfg.skill_env)?),
        "tool_result_disk_threshold" => Ok(json!({
            "toolResultDiskThreshold": cfg.tool_result_disk_threshold,
        })),
        "ask_user_question_timeout" => Ok(json!({
            "askUserQuestionTimeoutSecs": cfg.ask_user_question_timeout_secs,
        })),
        "plan" => Ok(json!({
            "planSubagent": cfg.plan_subagent,
            "plansDirectory": cfg.plans_directory,
        })),
        "skills_auto_review" => Ok(serde_json::to_value(&cfg.skills.auto_review)?),
        "recall_summary" => Ok(serde_json::to_value(&cfg.recall_summary)?),
        "tool_call_narration" => Ok(json!({
            "toolCallNarrationEnabled": cfg.tool_call_narration_enabled,
        })),
        "teams" => {
            let db = crate::globals::get_session_db()
                .ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;
            let templates = db.list_team_templates()?;
            Ok(serde_json::to_value(&templates)?)
        }
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
            "reactiveMicrocompactEnabled": cfg.compact.reactive_microcompact_enabled,
            "reactiveTriggerRatio": cfg.compact.reactive_trigger_ratio,
        },
        "asyncTools": { "enabled": cfg.async_tools.enabled },
        "deferredTools": { "enabled": cfg.deferred_tools.enabled },
        "awareness": { "enabled": cfg.awareness.enabled },
        "security": {
            "ssrfDefaultPolicy": cfg.ssrf.default_policy,
            "trustedHostsCount": cfg.ssrf.trusted_hosts.len(),
        },
        "activeModel": cfg.active_model,
        "fallbackModels": cfg.fallback_models.len(),
        "skills": {
            "extraDirs": cfg.extra_skills_dirs.len(),
            "disabled": cfg.disabled_skills,
        },
    });

    // Expose risk classification so the model can decide when to double-confirm.
    let risk_levels = json!({
        "low": [
            "user", "theme", "language", "ui_effects", "notification",
            "canvas", "image", "pdf", "image_generate", "temperature", "tool_timeout"
        ],
        "medium": [
            "compact", "memory_extract", "memory_selection", "memory_budget",
            "embedding_cache", "dedup", "hybrid_search", "temporal_decay",
            "mmr", "recap", "awareness", "web_fetch", "web_search",
            "deferred_tools", "async_tools", "approval",
            "tool_result_disk_threshold", "ask_user_question_timeout", "plan",
            "skills_auto_review", "recall_summary", "tool_call_narration",
            "teams"
        ],
        "high": [
            "proxy", "embedding", "shortcuts", "skills", "server",
            "acp_control", "skill_env", "security.ssrf"
        ],
    });

    Ok(serde_json::to_string_pretty(&json!({
        "category": "all",
        "overview": overview,
        "riskLevels": risk_levels,
        "hint": "Use get_settings with a specific category for full details. HIGH-risk categories require explicit user confirmation before calling update_settings.",
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
    // Tag the autosave snapshot so rollback listings know this came from the skill.
    let _reason = crate::backup::scope_save_reason("user", "skill");
    user_config::save_user_config_to_disk(&updated)?;
    drop(_reason);

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
        "security.ssrf" => merge_field(&mut store.ssrf, values)?,
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
        "memory_budget" => merge_field(&mut store.memory_budget, values)?,
        "embedding" => merge_field(&mut store.embedding, values)?,
        "embedding_cache" => merge_field(&mut store.embedding_cache, values)?,
        "dedup" => merge_field(&mut store.dedup, values)?,
        "hybrid_search" => merge_field(&mut store.hybrid_search, values)?,
        "temporal_decay" => merge_field(&mut store.temporal_decay, values)?,
        "mmr" => merge_field(&mut store.mmr, values)?,
        "recap" => merge_field(&mut store.recap, values)?,
        "awareness" => merge_field(&mut store.awareness, values)?,
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
        "server" => merge_field(&mut store.server, values)?,
        "acp_control" => merge_field(&mut store.acp_control, values)?,
        "skill_env" => {
            // Per-skill env vars: support full replace via `skillEnv` or per-skill
            // patches via `set` / `remove` to avoid forcing the model to echo
            // every skill's entire env block.
            if let Some(v) = values.get("skillEnv") {
                store.skill_env = serde_json::from_value(v.clone())?;
            }
            if let Some(set) = values.get("set").and_then(|v| v.as_object()) {
                for (skill, vars) in set {
                    let entry = store.skill_env.entry(skill.clone()).or_default();
                    if let Some(vars_obj) = vars.as_object() {
                        for (k, val) in vars_obj {
                            if let Some(s) = val.as_str() {
                                entry.insert(k.clone(), s.to_string());
                            } else if val.is_null() {
                                entry.remove(k);
                            } else {
                                bail!(
                                    "skill_env.set[{skill}].{k} must be a string or null, got {val}"
                                );
                            }
                        }
                    }
                }
            }
            if let Some(remove) = values.get("remove").and_then(|v| v.as_array()) {
                for item in remove {
                    if let Some(skill) = item.as_str() {
                        store.skill_env.remove(skill);
                    }
                }
            }
        }
        "tool_result_disk_threshold" => {
            if let Some(v) = values.get("toolResultDiskThreshold") {
                if v.is_null() {
                    store.tool_result_disk_threshold = None;
                } else if let Some(n) = v.as_u64() {
                    store.tool_result_disk_threshold = Some(n as usize);
                } else {
                    bail!("toolResultDiskThreshold must be a non-negative integer or null");
                }
            }
        }
        "ask_user_question_timeout" => {
            if let Some(v) = values.get("askUserQuestionTimeoutSecs").and_then(|v| v.as_u64()) {
                store.ask_user_question_timeout_secs = v;
            }
        }
        "plan" => {
            if let Some(v) = values.get("planSubagent").and_then(|v| v.as_bool()) {
                store.plan_subagent = v;
            }
            if let Some(v) = values.get("plansDirectory") {
                if v.is_null() {
                    store.plans_directory = None;
                } else if let Some(s) = v.as_str() {
                    store.plans_directory = Some(s.to_string());
                } else {
                    bail!("plansDirectory must be a string or null");
                }
            }
        }
        "skills_auto_review" => merge_field(&mut store.skills.auto_review, values)?,
        "recall_summary" => merge_field(&mut store.recall_summary, values)?,
        "tool_call_narration" => {
            if let Some(v) = values
                .get("toolCallNarrationEnabled")
                .and_then(|v| v.as_bool())
            {
                store.tool_call_narration_enabled = v;
            }
        }
        "teams" => {
            // Teams are DB rows, not AppConfig fields. Perform CRUD directly on the
            // team_templates table and return early (skip save_config / hot reload).
            return update_team_templates(values);
        }
        _ => bail!("Unknown settings category: '{category}'"),
    }

    // Tag the autosave snapshot so rollback listings carry (category, source).
    let _reason = crate::backup::scope_save_reason(category, "skill");
    config::save_config(&store)?;
    drop(_reason);

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
    let mut response = json!({
        "category": category,
        "riskLevel": risk_level(category),
        "updated": true,
        "settings": updated_value,
    });
    if let Some(note) = side_effect_note(category) {
        response["sideEffect"] = json!(note);
    }
    Ok(serde_json::to_string_pretty(&response)?)
}

/// Handle CRUD on the `team_templates` DB table. This category bypasses the
/// usual AppConfig read-modify-save path because templates live in SQLite.
fn update_team_templates(values: &Value) -> Result<String> {
    let action = values
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "teams: missing 'action'. Expected 'save' (with 'template') or 'delete' (with 'templateId')."
            )
        })?;

    let db = crate::globals::get_session_db()
        .ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;

    match action {
        "save" => {
            let payload = values
                .get("template")
                .ok_or_else(|| anyhow::anyhow!("teams.save: missing 'template' payload"))?;
            let template: crate::team::TeamTemplate =
                serde_json::from_value(payload.clone())?;
            if template.template_id.trim().is_empty() {
                bail!("teams.save: template.templateId must not be empty");
            }
            let saved = crate::team::templates::save_template(&db, template)?;
            Ok(serde_json::to_string_pretty(&json!({
                "category": "teams",
                "riskLevel": risk_level("teams"),
                "action": "save",
                "updated": true,
                "template": saved,
                "sideEffect": side_effect_note("teams"),
            }))?)
        }
        "delete" => {
            let template_id = values
                .get("templateId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("teams.delete: missing 'templateId'"))?;
            crate::team::templates::delete_template(&db, template_id)?;
            Ok(serde_json::to_string_pretty(&json!({
                "category": "teams",
                "riskLevel": risk_level("teams"),
                "action": "delete",
                "updated": true,
                "templateId": template_id,
            }))?)
        }
        other => bail!(
            "teams: unknown action '{other}'. Expected 'save' or 'delete'."
        ),
    }
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

// ── list_settings_backups ───────────────────────────────────────

pub(crate) async fn tool_list_settings_backups(args: &Value) -> Result<String> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(200) as usize;
    let kind_filter = args.get("kind").and_then(|v| v.as_str());

    let mut entries = crate::backup::list_autosaves().map_err(|e| anyhow::anyhow!(e))?;
    if let Some(k) = kind_filter {
        entries.retain(|e| e.kind == k);
    }
    entries.truncate(limit);

    Ok(serde_json::to_string_pretty(&json!({
        "count": entries.len(),
        "backups": entries,
        "hint": "Use restore_settings_backup({id}) to roll back. A pre-restore snapshot is created automatically so the rollback itself is reversible.",
    }))?)
}

// ── restore_settings_backup ─────────────────────────────────────

pub(crate) async fn tool_restore_settings_backup(args: &Value) -> Result<String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: id"))?;

    let entry = crate::backup::restore_autosave(id).map_err(|e| anyhow::anyhow!(e))?;

    app_info!(
        "settings",
        "rollback",
        "Restored autosave id={} kind={} category={}",
        entry.id,
        entry.kind,
        entry.category
    );

    Ok(serde_json::to_string_pretty(&json!({
        "restored": true,
        "entry": entry,
        "note": "A pre-restore snapshot of the previous state was also saved so you can undo this rollback.",
    }))?)
}

use tauri::State;
use crate::AppState;
use crate::skills;
use crate::provider;

#[tauri::command]
pub async fn get_skills(
    state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_extra(&store.extra_skills_dirs);
    let disabled = &store.disabled_skills;
    Ok(entries
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            let requires_env = e.requires.env.clone();
            skills::SkillSummary {
                name: e.name,
                description: e.description,
                source: e.source,
                base_dir: e.base_dir,
                enabled,
                requires_env,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn get_skill_detail(
    name: String,
    state: State<'_, AppState>,
) -> Result<skills::SkillDetail, String> {
    let store = state.provider_store.lock().await;
    skills::get_skill_content(&name, &store.extra_skills_dirs, &store.disabled_skills)
        .ok_or_else(|| format!("Skill not found: {}", name))
}

#[tauri::command]
pub async fn get_extra_skills_dirs(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let store = state.provider_store.lock().await;
    Ok(store.extra_skills_dirs.clone())
}

#[tauri::command]
pub async fn add_extra_skills_dir(
    dir: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    // Avoid duplicates
    if !store.extra_skills_dirs.contains(&dir) {
        store.extra_skills_dirs.push(dir);
        provider::save_store(&store).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn remove_extra_skills_dir(
    dir: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    store.extra_skills_dirs.retain(|d| d != &dir);
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_skill(
    name: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    if enabled {
        store.disabled_skills.retain(|n| n != &name);
    } else if !store.disabled_skills.contains(&name) {
        store.disabled_skills.push(name);
    }
    provider::save_store(&store).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_skill_env_check(
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let store = state.provider_store.lock().await;
    Ok(store.skill_env_check)
}

#[tauri::command]
pub async fn set_skill_env_check(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    store.skill_env_check = enabled;
    provider::save_store(&store).map_err(|e| e.to_string())
}

/// Get the configured env vars for a specific skill (values masked).
#[tauri::command]
pub async fn get_skill_env(
    name: String,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let store = state.provider_store.lock().await;
    let env_map = store.skill_env.get(&name).cloned().unwrap_or_default();
    Ok(env_map
        .into_iter()
        .map(|(k, v)| (k, skills::mask_value(&v)))
        .collect())
}

/// Set a single env var for a skill. Skips masked placeholder values.
#[tauri::command]
pub async fn set_skill_env_var(
    skill: String,
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Don't overwrite real value with a masked placeholder
    if skills::is_masked_value(&value) {
        return Ok(());
    }
    let mut store = state.provider_store.lock().await;
    store.skill_env.entry(skill).or_default().insert(key, value);
    provider::save_store(&store).map_err(|e| e.to_string())
}

/// Remove a configured env var for a skill.
#[tauri::command]
pub async fn remove_skill_env_var(
    skill: String,
    key: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.provider_store.lock().await;
    if let Some(map) = store.skill_env.get_mut(&skill) {
        map.remove(&key);
        if map.is_empty() {
            store.skill_env.remove(&skill);
        }
    }
    provider::save_store(&store).map_err(|e| e.to_string())
}

/// Batch-return env configuration status for all skills.
/// Returns skill_name -> { env_var_name -> is_configured }.
#[tauri::command]
pub async fn get_skills_env_status(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, bool>>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_extra(&store.extra_skills_dirs);
    let mut result = std::collections::HashMap::new();
    for entry in &entries {
        if entry.requires.env.is_empty() {
            continue;
        }
        let configured = store.skill_env.get(&entry.name);
        let mut status = std::collections::HashMap::new();
        for key in &entry.requires.env {
            let has_configured = configured
                .and_then(|m| m.get(key))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let has_system = std::env::var(key).map(|v| !v.is_empty()).unwrap_or(false);
            status.insert(key.clone(), has_configured || has_system);
        }
        result.insert(entry.name.clone(), status);
    }
    Ok(result)
}

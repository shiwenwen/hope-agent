use tauri::State;
use crate::AppState;
use crate::skills;
use crate::provider;

#[tauri::command]
pub async fn get_skills(
    state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_budget(
        &store.extra_skills_dirs,
        &store.skill_prompt_budget,
    );
    let disabled = &store.disabled_skills;
    Ok(entries
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            let requires_env = e.requires.env.clone();
            let any_bins = e.requires.any_bins.clone();
            let always = e.requires.always;
            skills::SkillSummary {
                name: e.name,
                description: e.description,
                source: e.source,
                base_dir: e.base_dir,
                enabled,
                requires_env,
                skill_key: e.skill_key,
                user_invocable: e.user_invocable,
                disable_model_invocation: e.disable_model_invocation,
                has_install: !e.install.is_empty(),
                any_bins,
                always,
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
    skills::bump_skill_version();
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
    skills::bump_skill_version();
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
    skills::bump_skill_version();
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    skills::bump_skill_version();
    Ok(())
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    skills::bump_skill_version();
    Ok(())
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
    provider::save_store(&store).map_err(|e| e.to_string())?;
    skills::bump_skill_version();
    Ok(())
}

/// Batch-return env configuration status for all skills.
/// Returns skill_name -> { env_var_name -> is_configured }.
#[tauri::command]
pub async fn get_skills_env_status(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, bool>>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_budget(
        &store.extra_skills_dirs,
        &store.skill_prompt_budget,
    );
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

/// Get health status for all skills.
#[tauri::command]
pub async fn get_skills_status(
    state: State<'_, AppState>,
) -> Result<Vec<skills::SkillStatusEntry>, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_budget(
        &store.extra_skills_dirs,
        &store.skill_prompt_budget,
    );
    Ok(skills::check_all_skills_status(
        &entries,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_allow_bundled,
    ))
}

/// Install a skill dependency.
#[tauri::command]
pub async fn install_skill_dependency(
    skill_name: String,
    spec_index: usize,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let store = state.provider_store.lock().await;
    let entries = skills::load_all_skills_with_budget(
        &store.extra_skills_dirs,
        &store.skill_prompt_budget,
    );
    drop(store); // Release lock before running install

    let skill = entries.into_iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| format!("Skill not found: {}", skill_name))?;

    let spec = skill.install.get(spec_index)
        .ok_or_else(|| format!("Install spec index {} out of range", spec_index))?;

    // Check OS constraint
    if !spec.os.is_empty() {
        let current = std::env::consts::OS;
        let os_ok = spec.os.iter().any(|os| {
            os == current
                || (os == "darwin" && current == "macos")
                || (os == "mac" && current == "macos")
        });
        if !os_ok {
            return Err(format!(
                "Install spec is not available on this platform ({}), requires: {:?}",
                current, spec.os
            ));
        }
    }

    let output = match spec.kind.as_str() {
        "brew" => {
            let formula = spec.formula.as_deref()
                .ok_or("Brew install spec missing 'formula' field")?;
            // Validate formula name (basic safety check)
            if formula.contains("..") || formula.contains('\\') || formula.starts_with('-') {
                return Err("Invalid brew formula name".to_string());
            }
            run_install_command("brew", &["install", formula]).await?
        }
        "node" => {
            let package = spec.package.as_deref()
                .ok_or("Node install spec missing 'package' field")?;
            if package.contains("..") || package.contains('\\') {
                return Err("Invalid npm package name".to_string());
            }
            run_install_command("npm", &["install", "-g", package]).await?
        }
        "go" => {
            let module = spec.go_module.as_deref()
                .ok_or("Go install spec missing 'module' field")?;
            if module.contains("..") || module.contains('\\') {
                return Err("Invalid go module path".to_string());
            }
            run_install_command("go", &["install", module]).await?
        }
        "uv" => {
            let package = spec.package.as_deref()
                .ok_or("UV install spec missing 'package' field")?;
            run_install_command("uv", &["tool", "install", package]).await?
        }
        _ => return Err(format!("Unsupported install kind: {}", spec.kind)),
    };

    // Verify binaries after install
    let mut verification = String::new();
    for bin in &spec.bins {
        if skills::binary_in_path_public(bin) {
            verification.push_str(&format!("\n✓ {} found in PATH", bin));
        } else {
            verification.push_str(&format!("\n✗ {} not found in PATH", bin));
        }
    }

    skills::bump_skill_version();
    Ok(format!("{}{}", output, verification))
}

/// Run an install command and return its output.
async fn run_install_command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to run {} {}: {}", program, args.join(" "), e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        Ok(format!("{}{}", stdout, stderr))
    } else {
        Err(format!(
            "{} {} failed (exit code {:?}):\n{}\n{}",
            program,
            args.join(" "),
            output.status.code(),
            stdout,
            stderr
        ))
    }
}

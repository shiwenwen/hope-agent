use crate::skills;
use crate::AppState;
use tauri::State;

use ha_core::skills::commands as core;

const SOURCE: &str = "settings-ui";

#[tauri::command]
pub async fn get_skills(_state: State<'_, AppState>) -> Result<Vec<skills::SkillSummary>, String> {
    Ok(core::list_skills())
}

#[tauri::command]
pub async fn get_skill_detail(
    name: String,
    _state: State<'_, AppState>,
) -> Result<skills::SkillDetail, String> {
    core::get_skill_detail(&name).ok_or_else(|| format!("Skill not found: {}", name))
}

#[tauri::command]
pub async fn get_extra_skills_dirs(_state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(core::get_extra_skills_dirs())
}

#[tauri::command]
pub async fn add_extra_skills_dir(dir: String, _state: State<'_, AppState>) -> Result<(), String> {
    core::add_extra_skills_dir(dir, SOURCE).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_extra_skills_dir(
    dir: String,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    core::remove_extra_skills_dir(&dir, SOURCE).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_skill(
    name: String,
    enabled: bool,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    core::toggle_skill(name, enabled, SOURCE).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_skill_env_check(_state: State<'_, AppState>) -> Result<bool, String> {
    Ok(core::get_skill_env_check())
}

#[tauri::command]
pub async fn set_skill_env_check(enabled: bool, _state: State<'_, AppState>) -> Result<(), String> {
    core::set_skill_env_check(enabled, SOURCE).map_err(|e| e.to_string())
}

/// Get the configured env vars for a specific skill (values masked).
#[tauri::command]
pub async fn get_skill_env(
    name: String,
    _state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    Ok(core::get_skill_env_masked(&name))
}

/// Set a single env var for a skill. Skips masked placeholder values.
#[tauri::command]
pub async fn set_skill_env_var(
    skill: String,
    key: String,
    value: String,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    core::set_skill_env_var(skill, key, value, SOURCE).map_err(|e| e.to_string())
}

/// Remove a configured env var for a skill.
#[tauri::command]
pub async fn remove_skill_env_var(
    skill: String,
    key: String,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    core::remove_skill_env_var(&skill, &key, SOURCE).map_err(|e| e.to_string())
}

/// Batch-return env configuration status for all skills.
/// Returns skill_name -> { env_var_name -> is_configured }.
#[tauri::command]
pub async fn get_skills_env_status(
    _state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, bool>>, String> {
    Ok(core::get_skills_env_status())
}

/// Get health status for all skills.
#[tauri::command]
pub async fn get_skills_status(
    _state: State<'_, AppState>,
) -> Result<Vec<skills::SkillStatusEntry>, String> {
    Ok(core::get_skills_status())
}

/// Install a skill dependency. (Tauri-only — HTTP surface doesn't expose
/// process-spawn semantics and no route calls this.)
#[tauri::command]
pub async fn install_skill_dependency(
    skill_name: String,
    spec_index: usize,
    _state: State<'_, AppState>,
) -> Result<String, String> {
    let store = ha_core::config::cached_config();
    let entries =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    drop(store); // Release Arc before running install

    let skill = entries
        .into_iter()
        .find(|s| s.name == skill_name)
        .ok_or_else(|| format!("Skill not found: {}", skill_name))?;

    let spec = skill
        .install
        .get(spec_index)
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
            let formula = spec
                .formula
                .as_deref()
                .ok_or("Brew install spec missing 'formula' field")?;
            // Validate formula name (basic safety check)
            if formula.contains("..") || formula.contains('\\') || formula.starts_with('-') {
                return Err("Invalid brew formula name".to_string());
            }
            run_install_command("brew", &["install", formula]).await?
        }
        "node" => {
            let package = spec
                .package
                .as_deref()
                .ok_or("Node install spec missing 'package' field")?;
            if package.contains("..") || package.contains('\\') {
                return Err("Invalid npm package name".to_string());
            }
            run_install_command("npm", &["install", "-g", package]).await?
        }
        "go" => {
            let module = spec
                .go_module
                .as_deref()
                .ok_or("Go install spec missing 'module' field")?;
            if module.contains("..") || module.contains('\\') {
                return Err("Invalid go module path".to_string());
            }
            run_install_command("go", &["install", module]).await?
        }
        "uv" => {
            let package = spec
                .package
                .as_deref()
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

// ── Phase B' Auto-Review ────────────────────────────────────────

#[tauri::command]
pub async fn list_draft_skills(
    _state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, String> {
    Ok(core::list_draft_skills())
}

#[tauri::command]
pub async fn activate_draft_skill(name: String) -> Result<(), String> {
    core::activate_draft_skill(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn discard_draft_skill(name: String) -> Result<(), String> {
    core::discard_draft_skill(&name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn trigger_skill_review_now(session_id: String) -> Result<serde_json::Value, String> {
    core::trigger_skill_review_now(&session_id)
        .await
        .map_err(|e| e.to_string())
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

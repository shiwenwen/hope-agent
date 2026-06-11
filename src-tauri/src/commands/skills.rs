use crate::commands::CmdError;
use crate::skills;
use crate::AppState;
use tauri::State;

use ha_core::skills::commands as core;

const SOURCE: &str = "settings-ui";

#[tauri::command]
pub async fn get_skills(
    _state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, CmdError> {
    Ok(core::list_skills())
}

#[tauri::command]
pub async fn get_skill_detail(
    name: String,
    _state: State<'_, AppState>,
) -> Result<skills::SkillDetail, CmdError> {
    core::get_skill_detail(&name).ok_or_else(|| CmdError::msg(format!("Skill not found: {}", name)))
}

/// Curated, fixed allowlist of built-in skills offered by the composer's
/// `@skill` menu (office trio + browser + macOS-only mac control), filtered to
/// what's invocable on this host.
#[tauri::command]
pub async fn list_mentionable_skills(
    _state: State<'_, AppState>,
) -> Result<Vec<ha_core::skills::MentionableSkill>, CmdError> {
    Ok(ha_core::skills::list_mentionable_skills())
}

#[tauri::command]
pub async fn get_extra_skills_dirs(_state: State<'_, AppState>) -> Result<Vec<String>, CmdError> {
    Ok(core::get_extra_skills_dirs())
}

#[tauri::command]
pub async fn add_extra_skills_dir(
    dir: String,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::add_extra_skills_dir(dir, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn remove_extra_skills_dir(
    dir: String,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::remove_extra_skills_dir(&dir, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn discover_preset_skill_sources(
    _state: State<'_, AppState>,
) -> Result<Vec<core::PresetSkillSource>, CmdError> {
    Ok(core::discover_preset_skill_sources())
}

#[tauri::command]
pub async fn toggle_skill(
    name: String,
    enabled: bool,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::toggle_skill(name, enabled, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn get_skill_env_check(_state: State<'_, AppState>) -> Result<bool, CmdError> {
    Ok(core::get_skill_env_check())
}

#[tauri::command]
pub async fn set_skill_env_check(
    enabled: bool,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::set_skill_env_check(enabled, SOURCE).map_err(Into::into)
}

/// Get the configured env vars for a specific skill (values masked).
#[tauri::command]
pub async fn get_skill_env(
    name: String,
    _state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, CmdError> {
    Ok(core::get_skill_env_masked(&name))
}

/// Set a single env var for a skill. Skips masked placeholder values.
#[tauri::command]
pub async fn set_skill_env_var(
    skill: String,
    key: String,
    value: String,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::set_skill_env_var(skill, key, value, SOURCE).map_err(Into::into)
}

/// Remove a configured env var for a skill.
#[tauri::command]
pub async fn remove_skill_env_var(
    skill: String,
    key: String,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::remove_skill_env_var(&skill, &key, SOURCE).map_err(Into::into)
}

/// Batch-return env configuration status for all skills.
/// Returns skill_name -> { env_var_name -> is_configured }.
#[tauri::command]
pub async fn get_skills_env_status(
    _state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, bool>>, CmdError> {
    Ok(core::get_skills_env_status())
}

/// Get health status for all skills.
#[tauri::command]
pub async fn get_skills_status(
    _state: State<'_, AppState>,
) -> Result<Vec<skills::SkillStatusEntry>, CmdError> {
    Ok(core::get_skills_status())
}

/// Install a skill dependency. Desktop path is unconditional — clicking the
/// "Install" button in the native GUI is itself the user consent. The HTTP
/// surface gates on `skills.allow_remote_install`; see
/// [`ha_core::skills::commands::install_skill_dependency`] for the shared
/// spawn logic.
#[tauri::command]
pub async fn install_skill_dependency(
    skill_name: String,
    spec_index: usize,
    _state: State<'_, AppState>,
) -> Result<String, CmdError> {
    core::install_skill_dependency(&skill_name, spec_index)
        .await
        .map_err(Into::into)
}

// ── Phase B' Auto-Review ────────────────────────────────────────

#[tauri::command]
pub async fn list_draft_skills(
    _state: State<'_, AppState>,
) -> Result<Vec<skills::SkillSummary>, CmdError> {
    Ok(core::list_draft_skills())
}

#[tauri::command]
pub async fn activate_draft_skill(name: String) -> Result<(), CmdError> {
    core::activate_draft_skill(&name).map_err(Into::into)
}

#[tauri::command]
pub async fn discard_draft_skill(name: String) -> Result<(), CmdError> {
    core::discard_draft_skill(&name).map_err(Into::into)
}

#[tauri::command]
pub async fn trigger_skill_review_now(session_id: String) -> Result<serde_json::Value, CmdError> {
    core::trigger_skill_review_now(&session_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn get_skills_auto_review_promotion(
    _state: State<'_, AppState>,
) -> Result<bool, CmdError> {
    Ok(core::get_auto_review_promotion())
}

#[tauri::command]
pub async fn set_skills_auto_review_promotion(
    auto: bool,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::set_auto_review_promotion(auto, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn get_skills_auto_review_enabled(_state: State<'_, AppState>) -> Result<bool, CmdError> {
    Ok(core::get_auto_review_enabled())
}

#[tauri::command]
pub async fn set_skills_auto_review_enabled(
    enabled: bool,
    _state: State<'_, AppState>,
) -> Result<(), CmdError> {
    core::set_auto_review_enabled(enabled, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn get_skills_auto_review_config(
    _state: State<'_, AppState>,
) -> Result<ha_core::skills::auto_review::SkillsAutoReviewConfig, CmdError> {
    Ok(core::get_auto_review_config_snapshot())
}

#[tauri::command]
pub async fn set_skills_auto_review_config(
    patch: serde_json::Value,
    _state: State<'_, AppState>,
) -> Result<ha_core::skills::auto_review::SkillsAutoReviewConfig, CmdError> {
    core::set_auto_review_config_patch(patch, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn reset_skills_auto_review_config(
    fields: Option<Vec<String>>,
    _state: State<'_, AppState>,
) -> Result<ha_core::skills::auto_review::SkillsAutoReviewConfig, CmdError> {
    core::reset_auto_review_config(fields, SOURCE).map_err(Into::into)
}

#[tauri::command]
pub async fn get_skills_auto_review_recent_rejects(
    limit: Option<usize>,
    _state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, CmdError> {
    Ok(core::recent_auto_review_skips(limit.unwrap_or(20)))
}

#[tauri::command]
pub async fn run_skills_curator_now(
    _state: State<'_, AppState>,
) -> Result<ha_core::skills::auto_review::curator::CuratorReport, CmdError> {
    core::run_curator_pass_sync().map_err(Into::into)
}

#[tauri::command]
pub async fn apply_skills_curator_merge(
    keep_id: String,
    member_ids: Vec<String>,
    _state: State<'_, AppState>,
) -> Result<usize, CmdError> {
    core::apply_curator_merge(&keep_id, &member_ids).map_err(Into::into)
}

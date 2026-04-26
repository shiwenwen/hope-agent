//! Tauri commands for the first-run onboarding wizard.
//!
//! Thin shells around [`ha_core::onboarding`] — errors are stringified at
//! the IPC boundary and inputs are lightly validated. The same surface is
//! exposed over HTTP in `ha-server::routes::onboarding` so the web GUI
//! (browser-mode wizard) shares the exact same semantics.

use crate::commands::CmdError;
use ha_core::onboarding::{
    apply::{self, ProfileStepInput, SafetyStepInput, ServerStepInput},
    personality_preset_by_id, state, OnboardingState,
};
use serde_json::Value;

#[tauri::command]
pub async fn get_onboarding_state() -> Result<OnboardingState, CmdError> {
    state::get_state().map_err(Into::into)
}

#[tauri::command]
pub async fn save_onboarding_draft(step: u32, draft: Value) -> Result<(), CmdError> {
    state::save_draft(step, draft).map_err(Into::into)
}

#[tauri::command]
pub async fn mark_onboarding_completed() -> Result<(), CmdError> {
    state::mark_completed().map_err(Into::into)
}

#[tauri::command]
pub async fn mark_onboarding_skipped(step_key: String) -> Result<(), CmdError> {
    state::mark_skipped(&step_key).map_err(Into::into)
}

#[tauri::command]
pub async fn reset_onboarding() -> Result<(), CmdError> {
    state::reset().map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_language(language: String) -> Result<(), CmdError> {
    apply::apply_language(&language).map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_profile(
    name: Option<String>,
    timezone: Option<String>,
    ai_experience: Option<String>,
    response_style: Option<String>,
) -> Result<(), CmdError> {
    apply::apply_profile(ProfileStepInput {
        name,
        timezone,
        ai_experience,
        response_style,
    })
    .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_personality_preset_cmd(preset_id: String) -> Result<(), CmdError> {
    let preset = personality_preset_by_id(&preset_id)
        .ok_or_else(|| CmdError::msg(format!("unknown personality preset: {}", preset_id)))?;
    apply::apply_personality_preset(preset).map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_safety(approvals_enabled: bool) -> Result<(), CmdError> {
    apply::apply_safety(SafetyStepInput { approvals_enabled }).map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_skills(disabled: Vec<String>) -> Result<(), CmdError> {
    apply::apply_skills(disabled).map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_server(
    bind_addr: Option<String>,
    api_key: Option<String>,
) -> Result<(), CmdError> {
    apply::apply_server(ServerStepInput { bind_addr, api_key }).map_err(Into::into)
}

#[tauri::command]
pub async fn generate_api_key() -> Result<String, CmdError> {
    Ok(apply::generate_api_key())
}

/// List local non-loopback IPv4 addresses, capped at 3 entries, so the
/// Summary page / Launch Banner can show a "same-LAN" URL. Returns an
/// empty vec if interface enumeration fails.
#[tauri::command]
pub async fn list_local_ips() -> Result<Vec<String>, CmdError> {
    Ok(crate::cli_onboarding::banner::local_ipv4_addresses())
}

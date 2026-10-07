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
    ha_core::blocking::run_blocking(state::get_state)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn save_onboarding_draft(step: u32, draft: Value) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || state::save_draft(step, draft))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_onboarding_completed() -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(state::mark_completed)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn mark_onboarding_skipped(step_key: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || state::mark_skipped(&step_key))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn reset_onboarding() -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(state::reset)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_language(language: String) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || apply::apply_language(&language))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_profile(
    name: Option<String>,
    timezone: Option<String>,
    ai_experience: Option<String>,
    response_style: Option<String>,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        apply::apply_profile(ProfileStepInput {
            name,
            timezone,
            ai_experience,
            response_style,
        })
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_personality_preset_cmd(preset_id: String) -> Result<(), CmdError> {
    let preset = personality_preset_by_id(&preset_id)
        .ok_or_else(|| CmdError::msg(format!("unknown personality preset: {}", preset_id)))?;
    ha_core::blocking::run_blocking(move || apply::apply_personality_preset(preset))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_safety(approvals_enabled: bool) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        apply::apply_safety(SafetyStepInput { approvals_enabled })
    })
    .await
    .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_skills(disabled: Vec<String>) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || apply::apply_skills(disabled))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn apply_onboarding_server(
    bind_addr: Option<String>,
    api_key: Option<String>,
) -> Result<(), CmdError> {
    ha_core::blocking::run_blocking(move || {
        apply::apply_server(ServerStepInput { bind_addr, api_key })
    })
    .await
    .map_err(Into::into)
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

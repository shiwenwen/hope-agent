//! First-run onboarding wizard HTTP routes.
//!
//! Thin wrappers around [`ha_core::onboarding`]. The web GUI browser
//! wizard (served by this same axum process via the static-file fallback
//! in `web_assets`) drives the exact same surface that Tauri IPC does on
//! the desktop.

use axum::Json;
use ha_core::onboarding::{
    apply::{self, ProfileStepInput, SafetyStepInput, ServerStepInput},
    personality_preset_by_id, state, OnboardingState,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

pub async fn get_state() -> Result<Json<OnboardingState>, AppError> {
    Ok(Json(state::get_state()?))
}

#[derive(Debug, Deserialize)]
pub struct DraftPayload {
    pub step: u32,
    pub draft: Value,
}

pub async fn save_draft(Json(p): Json<DraftPayload>) -> Result<Json<Value>, AppError> {
    state::save_draft(p.step, p.draft)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn mark_completed() -> Result<Json<Value>, AppError> {
    state::mark_completed()?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct SkipPayload {
    pub step_key: String,
}

pub async fn mark_skipped(Json(p): Json<SkipPayload>) -> Result<Json<Value>, AppError> {
    state::mark_skipped(&p.step_key)?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn reset() -> Result<Json<Value>, AppError> {
    state::reset()?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct LanguagePayload {
    pub language: String,
}

pub async fn apply_language(Json(p): Json<LanguagePayload>) -> Result<Json<Value>, AppError> {
    apply::apply_language(&p.language)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
pub struct ProfilePayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default, rename = "aiExperience")]
    pub ai_experience: Option<String>,
    #[serde(default, rename = "responseStyle")]
    pub response_style: Option<String>,
}

pub async fn apply_profile(Json(p): Json<ProfilePayload>) -> Result<Json<Value>, AppError> {
    apply::apply_profile(ProfileStepInput {
        name: p.name,
        timezone: p.timezone,
        ai_experience: p.ai_experience,
        response_style: p.response_style,
    })?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct PersonalityPresetPayload {
    #[serde(rename = "presetId")]
    pub preset_id: String,
}

pub async fn apply_personality_preset(
    Json(p): Json<PersonalityPresetPayload>,
) -> Result<Json<Value>, AppError> {
    let preset = personality_preset_by_id(&p.preset_id).ok_or_else(|| {
        AppError::bad_request(format!("unknown personality preset: {}", p.preset_id))
    })?;
    apply::apply_personality_preset(preset)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct SafetyPayload {
    #[serde(rename = "approvalsEnabled")]
    pub approvals_enabled: bool,
}

pub async fn apply_safety(Json(p): Json<SafetyPayload>) -> Result<Json<Value>, AppError> {
    apply::apply_safety(SafetyStepInput {
        approvals_enabled: p.approvals_enabled,
    })?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct SkillsPayload {
    pub disabled: Vec<String>,
}

pub async fn apply_skills(Json(p): Json<SkillsPayload>) -> Result<Json<Value>, AppError> {
    apply::apply_skills(p.disabled)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
pub struct ServerPayload {
    #[serde(default, rename = "bindAddr")]
    pub bind_addr: Option<String>,
    #[serde(default, rename = "apiKey")]
    pub api_key: Option<String>,
}

pub async fn apply_server(Json(p): Json<ServerPayload>) -> Result<Json<Value>, AppError> {
    apply::apply_server(ServerStepInput {
        bind_addr: p.bind_addr,
        api_key: p.api_key,
    })?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn generate_api_key() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "apiKey": apply::generate_api_key() })))
}

pub async fn list_local_ips() -> Result<Json<Value>, AppError> {
    Ok(Json(
        json!({ "ips": crate::banner::local_ipv4_addresses() }),
    ))
}

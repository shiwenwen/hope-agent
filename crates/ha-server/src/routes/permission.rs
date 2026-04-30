//! HTTP routes mirroring `commands/permission.rs` Tauri commands.

use axum::Json;
use ha_core::permission::{dangerous_commands, edit_commands, protected_paths, SmartModeConfig};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternListPayload {
    pub current: Vec<String>,
    pub defaults: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPatternsBody {
    pub patterns: Vec<String>,
}

// ── Protected paths ──────────────────────────────────────────────

pub async fn get_protected_paths() -> Result<Json<PatternListPayload>, AppError> {
    Ok(Json(PatternListPayload {
        current: (*protected_paths::current_patterns()).clone(),
        defaults: protected_paths::defaults().to_vec(),
    }))
}

pub async fn set_protected_paths(
    Json(body): Json<SetPatternsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    protected_paths::save_patterns(&body.patterns)?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

pub async fn reset_protected_paths() -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(protected_paths::reset_defaults()?))
}

// ── Dangerous commands ───────────────────────────────────────────

pub async fn get_dangerous_commands() -> Result<Json<PatternListPayload>, AppError> {
    Ok(Json(PatternListPayload {
        current: (*dangerous_commands::current_patterns()).clone(),
        defaults: dangerous_commands::defaults().to_vec(),
    }))
}

pub async fn set_dangerous_commands(
    Json(body): Json<SetPatternsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    dangerous_commands::save_patterns(&body.patterns)?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

pub async fn reset_dangerous_commands() -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(dangerous_commands::reset_defaults()?))
}

// ── Edit commands ────────────────────────────────────────────────

pub async fn get_edit_commands() -> Result<Json<PatternListPayload>, AppError> {
    Ok(Json(PatternListPayload {
        current: (*edit_commands::current_patterns()).clone(),
        defaults: edit_commands::defaults().to_vec(),
    }))
}

pub async fn set_edit_commands(
    Json(body): Json<SetPatternsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    edit_commands::save_patterns(&body.patterns)?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

pub async fn reset_edit_commands() -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(edit_commands::reset_defaults()?))
}

// ── Smart mode config ────────────────────────────────────────────

pub async fn get_smart_mode_config() -> Result<Json<SmartModeConfig>, AppError> {
    Ok(Json(
        ha_core::config::cached_config().permission.smart.clone(),
    ))
}

/// Body shape mirrors the Tauri `set_smart_mode_config(config: SmartModeConfig)`
/// IPC contract — frontend always wraps the config object so a single
/// `getTransport().call("set_smart_mode_config", { config })` works on
/// both transports.
#[derive(Debug, Deserialize)]
pub struct SetSmartModeBody {
    pub config: SmartModeConfig,
}

pub async fn set_smart_mode_config(
    Json(body): Json<SetSmartModeBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    ha_core::config::mutate_config(("permission.smart", "http"), |store| {
        store.permission.smart = body.config;
        Ok(())
    })?;
    Ok(Json(serde_json::json!({ "saved": true })))
}

// ── Global YOLO status ───────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalYoloStatus {
    pub cli_flag: bool,
    pub config_flag: bool,
    pub active: bool,
}

pub async fn get_global_yolo_status() -> Result<Json<GlobalYoloStatus>, AppError> {
    let s = ha_core::security::dangerous::status();
    Ok(Json(GlobalYoloStatus {
        cli_flag: s.cli_flag,
        config_flag: s.config_flag,
        active: s.active,
    }))
}

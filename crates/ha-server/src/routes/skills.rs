use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use ha_core::skills::{self, commands as core};

use crate::error::AppError;

const SOURCE: &str = "http";

/// `GET /api/skills`
pub async fn list_skills() -> Result<Json<Vec<skills::SkillSummary>>, AppError> {
    Ok(Json(core::list_skills()))
}

/// `GET /api/skills/{name}`
pub async fn get_skill_detail(
    Path(name): Path<String>,
) -> Result<Json<skills::SkillDetail>, AppError> {
    core::get_skill_detail(&name)
        .map(Json)
        .ok_or_else(|| AppError::not_found(format!("Skill not found: {}", name)))
}

/// `GET /api/skills/extra-dirs`
pub async fn get_extra_skills_dirs() -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(core::get_extra_skills_dirs()))
}

#[derive(Debug, Deserialize)]
pub struct DirBody {
    pub dir: String,
}

/// `POST /api/skills/extra-dirs`
pub async fn add_extra_skills_dir(Json(body): Json<DirBody>) -> Result<Json<Value>, AppError> {
    core::add_extra_skills_dir(body.dir, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/skills/extra-dirs?dir=...`
pub async fn remove_extra_skills_dir(Query(body): Query<DirBody>) -> Result<Json<Value>, AppError> {
    core::remove_extra_skills_dir(&body.dir, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/preset-sources` — probes known third-party skill catalog
/// locations (Claude Code user-level + plugins, Anthropic agent-skills
/// marketplace, OpenClaw / Hermes Agent clones) and returns the candidates
/// for the Quick Import UI. Read-only; adding paths is done via the existing
/// `POST /api/skills/extra-dirs` route.
pub async fn discover_preset_skill_sources() -> Result<Json<Vec<core::PresetSkillSource>>, AppError>
{
    Ok(Json(core::discover_preset_skill_sources()))
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

/// `POST /api/skills/{name}/toggle`
pub async fn toggle_skill(
    Path(name): Path<String>,
    Json(body): Json<ToggleBody>,
) -> Result<Json<Value>, AppError> {
    core::toggle_skill(name, body.enabled, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/env-check`
pub async fn get_skill_env_check() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "enabled": core::get_skill_env_check() })))
}

/// `PUT /api/skills/env-check`
pub async fn set_skill_env_check(Json(body): Json<ToggleBody>) -> Result<Json<Value>, AppError> {
    core::set_skill_env_check(body.enabled, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/{name}/env` (values masked)
pub async fn get_skill_env(
    Path(name): Path<String>,
) -> Result<Json<HashMap<String, String>>, AppError> {
    Ok(Json(core::get_skill_env_masked(&name)))
}

#[derive(Debug, Deserialize)]
pub struct EnvVarBody {
    pub key: String,
    pub value: String,
}

/// `POST /api/skills/{name}/env`
pub async fn set_skill_env_var(
    Path(name): Path<String>,
    Json(body): Json<EnvVarBody>,
) -> Result<Json<Value>, AppError> {
    core::set_skill_env_var(name, body.key, body.value, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct RemoveEnvVarQuery {
    pub key: String,
}

/// `DELETE /api/skills/{name}/env?key=...`
pub async fn remove_skill_env_var(
    Path(name): Path<String>,
    Query(q): Query<RemoveEnvVarQuery>,
) -> Result<Json<Value>, AppError> {
    core::remove_skill_env_var(&name, &q.key, SOURCE)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/env-status`
pub async fn get_skills_env_status(
) -> Result<Json<HashMap<String, HashMap<String, bool>>>, AppError> {
    Ok(Json(core::get_skills_env_status()))
}

/// `GET /api/skills/status`
pub async fn get_skills_status() -> Result<Json<Vec<skills::SkillStatusEntry>>, AppError> {
    Ok(Json(core::get_skills_status()))
}

// ── Dependency install ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallDepBody {
    pub spec_index: usize,
}

/// `POST /api/skills/{name}/install` — run the install spec at `specIndex`.
///
/// Gated on `AppConfig.skills.allow_remote_install` — returns 403 with a
/// clear error when disabled, so the frontend can surface actionable guidance
/// instead of a silent 404. The spawn core lives in
/// [`ha_core::skills::commands::install_skill_dependency`]; the Tauri handler
/// calls the same function without the gate (local user consent = clicking
/// the button in the desktop GUI).
pub async fn install_skill_dependency(
    Path(name): Path<String>,
    Json(body): Json<InstallDepBody>,
) -> Result<Json<Value>, AppError> {
    if !ha_core::config::cached_config().skills.allow_remote_install {
        return Err(AppError::forbidden(
            "Remote skill dependency install is disabled. Set \
             `skills.allowRemoteInstall = true` in config (or run the \
             install manually on the server) before retrying.",
        ));
    }
    let output = core::install_skill_dependency(&name, body.spec_index)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "output": output })))
}

// ── Phase B' Auto-Review ────────────────────────────────────────

/// `GET /api/skills/drafts` — list skills in `status: draft`.
pub async fn list_draft_skills() -> Result<Json<Vec<skills::SkillSummary>>, AppError> {
    Ok(Json(core::list_draft_skills()))
}

/// `POST /api/skills/{name}/activate` — promote a draft to active.
pub async fn activate_draft_skill(Path(name): Path<String>) -> Result<Json<Value>, AppError> {
    core::activate_draft_skill(&name)?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/skills/{name}/draft` — delete a draft skill.
pub async fn discard_draft_skill(Path(name): Path<String>) -> Result<Json<Value>, AppError> {
    core::discard_draft_skill(&name)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerReviewBody {
    pub session_id: String,
}

/// `POST /api/skills/review/run` — manually fire the auto-review pipeline.
pub async fn trigger_skill_review_now(
    Json(body): Json<TriggerReviewBody>,
) -> Result<Json<Value>, AppError> {
    let report = core::trigger_skill_review_now(&body.session_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(report))
}

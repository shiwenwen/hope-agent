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

/// `GET /api/skills/auto-review/promotion` — read the auto-review promotion mode.
/// Response: `{ "auto": bool }`. `true` = auto-created skills land directly as
/// `Active`; `false` = they land as `Draft` for manual review.
pub async fn get_auto_review_promotion() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "auto": core::get_auto_review_promotion() })))
}

#[derive(Debug, Deserialize)]
pub struct PromotionBody {
    pub auto: bool,
}

/// `PUT /api/skills/auto-review/promotion` — toggle the auto-review promotion mode.
pub async fn set_auto_review_promotion(
    Json(body): Json<PromotionBody>,
) -> Result<Json<Value>, AppError> {
    core::set_auto_review_promotion(body.auto, "http")
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "auto": body.auto })))
}

/// `GET /api/skills/auto-review/enabled` — read the auto-review master enabled
/// flag. Response: `{ "enabled": bool }`. `false` fully suppresses the
/// post-turn review pipeline.
pub async fn get_auto_review_enabled() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({ "enabled": core::get_auto_review_enabled() })))
}

#[derive(Debug, Deserialize)]
pub struct EnabledBody {
    pub enabled: bool,
}

/// `PUT /api/skills/auto-review/enabled` — toggle the master switch.
pub async fn set_auto_review_enabled(
    Json(body): Json<EnabledBody>,
) -> Result<Json<Value>, AppError> {
    core::set_auto_review_enabled(body.enabled, "http")
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "enabled": body.enabled })))
}

/// `GET /api/skills/auto-review/config` — full sanitized auto-review
/// config snapshot. Used by the Settings panel; UI binds to camelCase
/// keys directly.
pub async fn get_auto_review_config() -> Result<Json<Value>, AppError> {
    serde_json::to_value(core::get_auto_review_config_snapshot())
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct PatchConfigBody {
    pub patch: Value,
}

/// `PATCH /api/skills/auto-review/config` — deep-merge a partial
/// config object. Unknown keys are ignored by the serde round-trip.
/// Returns the resulting sanitized config.
pub async fn set_auto_review_config(
    Json(body): Json<PatchConfigBody>,
) -> Result<Json<Value>, AppError> {
    let snapshot = core::set_auto_review_config_patch(body.patch, SOURCE)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    serde_json::to_value(snapshot)
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetConfigBody {
    /// `None` (or missing) → reset every field; `Some([...])` → reset
    /// only those snake_case keys.
    pub fields: Option<Vec<String>>,
}

/// `POST /api/skills/auto-review/config/reset` — reset per-field or
/// whole-config to built-in defaults. Returns the resulting config.
pub async fn reset_auto_review_config(
    Json(body): Json<ResetConfigBody>,
) -> Result<Json<Value>, AppError> {
    let snapshot = core::reset_auto_review_config(body.fields, SOURCE)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    serde_json::to_value(snapshot)
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct RecentRejectsQuery {
    pub limit: Option<usize>,
}

/// `GET /api/skills/auto-review/recent-rejects?limit=N` — most recent
/// `skill_review_skipped` learning events, parsed into camelCase JSON.
pub async fn get_auto_review_recent_rejects(
    Query(q): Query<RecentRejectsQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    Ok(Json(Value::Array(core::recent_auto_review_skips(limit))))
}

/// `POST /api/skills/curator/run` — synchronous draft-consolidation
/// scan. Returns merge proposals; UI shows them and the user picks
/// which to apply via `apply_skills_curator_merge`.
pub async fn run_skills_curator_now() -> Result<Json<Value>, AppError> {
    let report = core::run_curator_pass_sync().map_err(|e| AppError::internal(e.to_string()))?;
    serde_json::to_value(report)
        .map(Json)
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyMergeBody {
    pub keep_id: String,
    pub member_ids: Vec<String>,
}

/// `POST /api/skills/curator/apply` — apply a single merge proposal.
/// Returns `{ "discarded": N }` indicating how many drafts were
/// actually deleted.
pub async fn apply_skills_curator_merge(
    Json(body): Json<ApplyMergeBody>,
) -> Result<Json<Value>, AppError> {
    let n = core::apply_curator_merge(&body.keep_id, &body.member_ids)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "discarded": n })))
}

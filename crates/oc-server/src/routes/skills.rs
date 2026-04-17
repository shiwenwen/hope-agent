use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use oc_core::skills;

use crate::error::AppError;
use crate::routes::helpers::app_state as state;

/// `GET /api/skills`
pub async fn list_skills() -> Result<Json<Vec<skills::SkillSummary>>, AppError> {
    let store = state()?.config.lock().await;
    let entries =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let disabled = &store.disabled_skills;
    let out: Vec<skills::SkillSummary> = entries
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            e.to_summary(enabled)
        })
        .collect();
    Ok(Json(out))
}

/// `GET /api/skills/{name}`
pub async fn get_skill_detail(
    Path(name): Path<String>,
) -> Result<Json<skills::SkillDetail>, AppError> {
    let store = state()?.config.lock().await;
    skills::get_skill_content(&name, &store.extra_skills_dirs, &store.disabled_skills)
        .map(Json)
        .ok_or_else(|| AppError::not_found(format!("Skill not found: {}", name)))
}

/// `GET /api/skills/extra-dirs`
pub async fn get_extra_skills_dirs() -> Result<Json<Vec<String>>, AppError> {
    Ok(Json(state()?.config.lock().await.extra_skills_dirs.clone()))
}

#[derive(Debug, Deserialize)]
pub struct DirBody {
    pub dir: String,
}

/// `POST /api/skills/extra-dirs`
pub async fn add_extra_skills_dir(Json(body): Json<DirBody>) -> Result<Json<Value>, AppError> {
    let mut store = state()?.config.lock().await;
    if !store.extra_skills_dirs.contains(&body.dir) {
        store.extra_skills_dirs.push(body.dir);
        oc_core::config::save_config(&store)?;
    }
    skills::bump_skill_version();
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/skills/extra-dirs?dir=...`
pub async fn remove_extra_skills_dir(Query(body): Query<DirBody>) -> Result<Json<Value>, AppError> {
    let mut store = state()?.config.lock().await;
    store.extra_skills_dirs.retain(|d| d != &body.dir);
    oc_core::config::save_config(&store)?;
    skills::bump_skill_version();
    Ok(Json(json!({ "ok": true })))
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
    let mut store = state()?.config.lock().await;
    if body.enabled {
        store.disabled_skills.retain(|n| n != &name);
    } else if !store.disabled_skills.contains(&name) {
        store.disabled_skills.push(name);
    }
    oc_core::config::save_config(&store)?;
    skills::bump_skill_version();
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/env-check`
pub async fn get_skill_env_check() -> Result<Json<Value>, AppError> {
    Ok(Json(
        json!({ "enabled": state()?.config.lock().await.skill_env_check }),
    ))
}

/// `PUT /api/skills/env-check`
pub async fn set_skill_env_check(Json(body): Json<ToggleBody>) -> Result<Json<Value>, AppError> {
    let mut store = state()?.config.lock().await;
    store.skill_env_check = body.enabled;
    oc_core::config::save_config(&store)?;
    skills::bump_skill_version();
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/{name}/env` (values masked)
pub async fn get_skill_env(
    Path(name): Path<String>,
) -> Result<Json<HashMap<String, String>>, AppError> {
    let store = state()?.config.lock().await;
    let env_map = store.skill_env.get(&name).cloned().unwrap_or_default();
    Ok(Json(
        env_map
            .into_iter()
            .map(|(k, v)| (k, skills::mask_value(&v)))
            .collect(),
    ))
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
    if skills::is_masked_value(&body.value) {
        return Ok(Json(json!({ "ok": true })));
    }
    let mut store = state()?.config.lock().await;
    store
        .skill_env
        .entry(name)
        .or_default()
        .insert(body.key, body.value);
    oc_core::config::save_config(&store)?;
    skills::bump_skill_version();
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
    let mut store = state()?.config.lock().await;
    if let Some(map) = store.skill_env.get_mut(&name) {
        map.remove(&q.key);
        if map.is_empty() {
            store.skill_env.remove(&name);
        }
    }
    oc_core::config::save_config(&store)?;
    skills::bump_skill_version();
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/skills/env-status`
pub async fn get_skills_env_status(
) -> Result<Json<HashMap<String, HashMap<String, bool>>>, AppError> {
    let store = state()?.config.lock().await;
    let entries =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    let mut result: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for entry in &entries {
        if entry.requires.env.is_empty() {
            continue;
        }
        let configured = store.skill_env.get(&entry.name);
        let mut status = HashMap::new();
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
    Ok(Json(result))
}

/// `GET /api/skills/status`
pub async fn get_skills_status() -> Result<Json<Vec<skills::SkillStatusEntry>>, AppError> {
    let store = state()?.config.lock().await;
    let entries =
        skills::load_all_skills_with_budget(&store.extra_skills_dirs, &store.skill_prompt_budget);
    Ok(Json(skills::check_all_skills_status(
        &entries,
        &store.disabled_skills,
        store.skill_env_check,
        &store.skill_env,
        &store.skill_allow_bundled,
    )))
}

// ── Phase B' Auto-Review ────────────────────────────────────────

/// `GET /api/skills/drafts` — list skills in `status: draft`.
pub async fn list_draft_skills() -> Result<Json<Vec<skills::SkillSummary>>, AppError> {
    let store = state()?.config.lock().await;
    let drafts = skills::author::list_drafts(&store.extra_skills_dirs);
    let disabled = &store.disabled_skills;
    let out: Vec<skills::SkillSummary> = drafts
        .into_iter()
        .map(|e| {
            let enabled = !disabled.contains(&e.name);
            e.to_summary(enabled)
        })
        .collect();
    Ok(Json(out))
}

/// `POST /api/skills/{name}/activate` — promote a draft to active.
pub async fn activate_draft_skill(Path(name): Path<String>) -> Result<Json<Value>, AppError> {
    skills::author::set_skill_status(&name, skills::SkillStatus::Active)?;
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /api/skills/{name}/draft` — delete a draft skill.
pub async fn discard_draft_skill(Path(name): Path<String>) -> Result<Json<Value>, AppError> {
    skills::author::delete_skill(&name)?;
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
    let gate = skills::auto_review::acquire_manual(&body.session_id).ok_or_else(|| {
        AppError::bad_request("another review is already running for this session")
    })?;
    let report = skills::auto_review::run_review_cycle(
        &body.session_id,
        skills::auto_review::ReviewTrigger::Manual,
        gate,
        None,
    )
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(report).unwrap_or(Value::Null)))
}

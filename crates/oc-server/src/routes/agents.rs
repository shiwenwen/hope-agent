use axum::extract::Path;
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Handlers ────────────────────────────────────────────────────

/// `GET /api/agents` -- list all agents.
pub async fn list_agents(
) -> Result<Json<Vec<oc_core::agent_config::AgentSummary>>, AppError> {
    let agents = oc_core::agent_loader::list_agents()?;
    Ok(Json(agents))
}

/// `GET /api/agents/{id}` -- get a single agent's config.
pub async fn get_agent(
    Path(id): Path<String>,
) -> Result<Json<oc_core::agent_config::AgentConfig>, AppError> {
    let def = oc_core::agent_loader::load_agent(&id)?;
    Ok(Json(def.config))
}

/// `PUT /api/agents/{id}` -- save (create or update) an agent's config.
pub async fn save_agent(
    Path(id): Path<String>,
    Json(config): Json<oc_core::agent_config::AgentConfig>,
) -> Result<Json<Value>, AppError> {
    oc_core::agent_loader::save_agent_config(&id, &config)?;
    Ok(Json(json!({ "saved": true })))
}

/// `DELETE /api/agents/{id}` -- delete an agent and all its files.
pub async fn delete_agent(
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    oc_core::agent_loader::delete_agent(&id)?;
    Ok(Json(json!({ "deleted": true })))
}

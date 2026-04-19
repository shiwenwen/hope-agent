use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;

// ── Handlers ────────────────────────────────────────────────────

/// `GET /api/agents` -- list all agents.
pub async fn list_agents() -> Result<Json<Vec<oc_core::agent_config::AgentSummary>>, AppError> {
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

/// Body wrapper for `save_agent_config_cmd` — frontend ships
/// `{ id, config: <AgentConfig> }` (id is in the path).
#[derive(Debug, Deserialize)]
pub struct SaveAgentBody {
    pub config: oc_core::agent_config::AgentConfig,
}

/// `PUT /api/agents/{id}` -- save (create or update) an agent's config.
pub async fn save_agent(
    Path(id): Path<String>,
    Json(body): Json<SaveAgentBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::agent_loader::save_agent_config(&id, &body.config)?;
    Ok(Json(json!({ "saved": true })))
}

/// `DELETE /api/agents/{id}` -- delete an agent and all its files.
pub async fn delete_agent(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    oc_core::agent_loader::delete_agent(&id)?;
    Ok(Json(json!({ "deleted": true })))
}

// ── Markdown files (agent.md / persona.md / tools.md / ...) ────

#[derive(Debug, Deserialize)]
pub struct GetMarkdownQuery {
    pub file: String,
}

/// `GET /api/agents/{id}/markdown?file=agent.md` — read a single agent
/// markdown file. Returns `{content: string | null}`.
pub async fn get_agent_markdown(
    Path(id): Path<String>,
    Query(q): Query<GetMarkdownQuery>,
) -> Result<Json<Value>, AppError> {
    let content = oc_core::agent_loader::get_agent_markdown(&id, &q.file)?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Debug, Deserialize)]
pub struct SaveMarkdownBody {
    pub file: String,
    pub content: String,
}

/// `PUT /api/agents/{id}/markdown` — write a single agent markdown file.
pub async fn save_agent_markdown(
    Path(id): Path<String>,
    Json(body): Json<SaveMarkdownBody>,
) -> Result<Json<Value>, AppError> {
    oc_core::agent_loader::save_agent_markdown(&id, &body.file, &body.content)?;
    Ok(Json(json!({ "saved": true })))
}

/// `POST /api/agents/{id}/persona/render-soul-md` — render the agent's
/// structured `PersonalityConfig` into a SOUL.md markdown draft. Used by the
/// UI when the user switches the persona authoring surface to SoulMd for
/// the first time. Does not write to disk — caller persists via
/// save_agent_markdown when ready.
pub async fn render_persona_to_soul_md(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let content = oc_core::agent_loader::render_persona_to_soul_md(&id)?;
    Ok(Json(json!({ "content": content })))
}

// ── Agent-scoped memory.md ─────────────────────────────────────

/// `GET /api/agents/{id}/memory-md` — read an agent's `memory.md`.
pub async fn get_agent_memory_md(Path(id): Path<String>) -> Result<Json<Value>, AppError> {
    let path = oc_core::paths::agent_dir(&id)?.join("memory.md");
    let content = if path.exists() {
        Some(std::fs::read_to_string(&path).map_err(|e| AppError::internal(e.to_string()))?)
    } else {
        None
    };
    Ok(Json(json!({ "content": content })))
}

#[derive(Debug, Deserialize)]
pub struct MemoryMdBody {
    pub content: String,
}

/// `PUT /api/agents/{id}/memory-md` — write an agent's `memory.md`.
pub async fn save_agent_memory_md(
    Path(id): Path<String>,
    Json(body): Json<MemoryMdBody>,
) -> Result<Json<Value>, AppError> {
    let dir = oc_core::paths::agent_dir(&id)?;
    std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(e.to_string()))?;
    std::fs::write(dir.join("memory.md"), body.content)
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "saved": true })))
}

// ── OpenClaw import ──────────────────────────────────────────

/// `GET /api/agents/openclaw/scan` — scan OpenClaw agents for import.
pub async fn scan_openclaw_agents(
) -> Result<Json<Vec<oc_core::openclaw_import::OpenClawAgentPreview>>, AppError> {
    let previews = oc_core::openclaw_import::scan_openclaw_agents()?;
    Ok(Json(previews))
}

#[derive(Debug, Deserialize)]
pub struct ImportOpenClawBody {
    pub requests: Vec<oc_core::openclaw_import::ImportAgentRequest>,
}

/// `POST /api/agents/openclaw/import` — import selected OpenClaw agents.
pub async fn import_openclaw_agents(
    Json(body): Json<ImportOpenClawBody>,
) -> Result<Json<Vec<oc_core::openclaw_import::ImportResult>>, AppError> {
    let results = oc_core::openclaw_import::import_openclaw_agents(&body.requests)?;
    Ok(Json(results))
}

// ── Agent templates ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TemplateQuery {
    pub name: String,
    #[serde(default)]
    pub locale: Option<String>,
}

/// `GET /api/agents/template?name=...&locale=...` — fetch a built-in agent
/// markdown template (agent / persona / tools / ...). Returns an empty
/// string when no template matches, mirroring the Tauri behaviour.
pub async fn get_agent_template(Query(q): Query<TemplateQuery>) -> Result<Json<Value>, AppError> {
    let locale = q.locale.as_deref().unwrap_or("en");
    let content = oc_core::agent_loader::get_template(&q.name, locale).unwrap_or_default();
    Ok(Json(json!({ "content": content })))
}

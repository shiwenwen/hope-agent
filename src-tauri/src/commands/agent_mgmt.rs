use crate::agent_config;
use crate::agent_loader;
use crate::commands::CmdError;
use ha_core::openclaw_import;
use serde_json::json;

#[tauri::command]
pub async fn list_agents() -> Result<Vec<agent_config::AgentSummary>, CmdError> {
    agent_loader::list_agents().map_err(Into::into)
}

#[tauri::command]
pub async fn get_agent_config(id: String) -> Result<agent_config::AgentConfig, CmdError> {
    let def = agent_loader::load_agent(&id)?;
    Ok(def.config)
}

#[tauri::command]
pub async fn get_agent_markdown(id: String, file: String) -> Result<Option<String>, CmdError> {
    agent_loader::get_agent_markdown(&id, &file).map_err(Into::into)
}

#[tauri::command]
pub async fn save_agent_config_cmd(
    id: String,
    config: agent_config::AgentConfig,
) -> Result<(), CmdError> {
    agent_loader::save_agent_config(&id, &config)?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit("agents:changed", json!({ "id": id, "kind": "saved" }));
    }
    Ok(())
}

#[tauri::command]
pub async fn save_agent_markdown(
    id: String,
    file: String,
    content: String,
) -> Result<(), CmdError> {
    agent_loader::save_agent_markdown(&id, &file, &content).map_err(Into::into)
}

#[tauri::command]
pub async fn delete_agent(id: String) -> Result<(), CmdError> {
    agent_loader::delete_agent(&id)?;
    if let Some(bus) = ha_core::get_event_bus() {
        bus.emit("agents:changed", json!({ "id": id, "kind": "deleted" }));
    }
    Ok(())
}

#[tauri::command]
pub async fn render_persona_to_soul_md(id: String) -> Result<String, CmdError> {
    agent_loader::render_persona_to_soul_md(&id).map_err(Into::into)
}

#[tauri::command]
pub async fn get_agent_template(name: String, locale: String) -> Result<String, CmdError> {
    agent_loader::get_template(&name, &locale)
        .ok_or_else(|| CmdError::msg(format!("Template not found: {}", name)))
}

#[tauri::command]
pub async fn scan_openclaw_agents() -> Result<Vec<openclaw_import::OpenClawAgentPreview>, CmdError>
{
    openclaw_import::scan_openclaw_agents().map_err(Into::into)
}

#[tauri::command]
pub async fn import_openclaw_agents(
    requests: Vec<openclaw_import::ImportAgentRequest>,
) -> Result<Vec<openclaw_import::ImportResult>, CmdError> {
    openclaw_import::import_openclaw_agents(&requests).map_err(Into::into)
}

#[tauri::command]
pub async fn scan_openclaw_full() -> Result<openclaw_import::OpenClawImportPreview, CmdError> {
    openclaw_import::scan_openclaw_full().map_err(Into::into)
}

#[tauri::command]
pub async fn import_openclaw_full(
    request: openclaw_import::OpenClawImportRequest,
) -> Result<openclaw_import::OpenClawImportSummary, CmdError> {
    let summary = openclaw_import::import_openclaw_full(&request)?;
    if let Some(bus) = ha_core::get_event_bus() {
        let imported_count = summary.agents.iter().filter(|r| r.success).count();
        if imported_count > 0 {
            bus.emit(
                "agents:changed",
                serde_json::json!({ "kind": "imported", "count": imported_count }),
            );
        }
        if !summary.providers_added.is_empty() {
            bus.emit(
                "config:changed",
                serde_json::json!({ "category": "providers", "source": "openclaw-import" }),
            );
        }
    }
    Ok(summary)
}

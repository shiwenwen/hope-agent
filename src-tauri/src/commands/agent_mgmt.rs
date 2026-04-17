use crate::agent_config;
use crate::agent_loader;
use oc_core::openclaw_import;

#[tauri::command]
pub async fn list_agents() -> Result<Vec<agent_config::AgentSummary>, String> {
    agent_loader::list_agents().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_config(id: String) -> Result<agent_config::AgentConfig, String> {
    let def = agent_loader::load_agent(&id).map_err(|e| e.to_string())?;
    Ok(def.config)
}

#[tauri::command]
pub async fn get_agent_markdown(id: String, file: String) -> Result<Option<String>, String> {
    agent_loader::get_agent_markdown(&id, &file).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_agent_config_cmd(
    id: String,
    config: agent_config::AgentConfig,
) -> Result<(), String> {
    agent_loader::save_agent_config(&id, &config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_agent_markdown(id: String, file: String, content: String) -> Result<(), String> {
    agent_loader::save_agent_markdown(&id, &file, &content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_agent(id: String) -> Result<(), String> {
    agent_loader::delete_agent(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn render_persona_to_soul_md(id: String) -> Result<String, String> {
    agent_loader::render_persona_to_soul_md(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_template(name: String, locale: String) -> Result<String, String> {
    agent_loader::get_template(&name, &locale)
        .ok_or_else(|| format!("Template not found: {}", name))
}

#[tauri::command]
pub async fn scan_openclaw_agents(
) -> Result<Vec<openclaw_import::OpenClawAgentPreview>, String> {
    openclaw_import::scan_openclaw_agents().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_openclaw_agents(
    requests: Vec<openclaw_import::ImportAgentRequest>,
) -> Result<Vec<openclaw_import::ImportResult>, String> {
    openclaw_import::import_openclaw_agents(&requests).map_err(|e| e.to_string())
}

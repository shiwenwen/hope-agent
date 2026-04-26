use crate::AppState;
use ha_core::agent::AssistantAgent;
use ha_core::local_llm::{
    detect_hardware, detect_ollama, install_ollama_via_script, pull_and_activate, recommend_model,
    start_ollama, HardwareInfo, ModelCandidate, ModelRecommendation, OllamaStatus,
};
use serde_json::json;
use tauri::State;

#[tauri::command]
pub async fn local_llm_detect_hardware() -> Result<HardwareInfo, String> {
    Ok(detect_hardware())
}

#[tauri::command]
pub async fn local_llm_recommend_model() -> Result<ModelRecommendation, String> {
    let hw = detect_hardware();
    Ok(recommend_model(&hw))
}

#[tauri::command]
pub async fn local_llm_detect_ollama() -> Result<OllamaStatus, String> {
    Ok(detect_ollama().await)
}

/// Run the bundled installer (Unix only). Progress is emitted via the
/// shared event bus on `local_llm:install_progress`; the frontend listens
/// for those events instead of receiving a Tauri Channel.
#[tauri::command]
pub async fn local_llm_install_ollama() -> Result<(), String> {
    let bus = ha_core::get_event_bus()
        .cloned()
        .ok_or_else(|| "EventBus not initialized".to_string())?;
    install_ollama_via_script(move |p| {
        bus.emit("local_llm:install_progress", json!(p));
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn local_llm_start_ollama() -> Result<(), String> {
    start_ollama().await.map_err(|e| e.to_string())
}

/// Pull the requested model, register the local-Ollama provider, and switch
/// the active model. Progress streams through the event bus on
/// `local_llm:pull_progress`. Returns the new `(provider_id, model_id)`.
#[tauri::command]
pub async fn local_llm_pull_and_activate(
    model: ModelCandidate,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let bus = ha_core::get_event_bus()
        .cloned()
        .ok_or_else(|| "EventBus not initialized".to_string())?;
    let (provider_id, model_id) = pull_and_activate(model, move |p| {
        bus.emit("local_llm:pull_progress", json!(p));
    })
    .await
    .map_err(|e| e.to_string())?;

    // Rebuild the cached agent so the next chat call uses the new local
    // provider without requiring a frontend reload.
    let provider = ha_core::config::cached_config()
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .cloned()
        .ok_or_else(|| format!("Provider not found after register: {provider_id}"))?;
    let agent = AssistantAgent::new_from_provider(&provider, &model_id);
    *state.agent.lock().await = Some(agent);
    Ok(json!({ "providerId": provider_id, "modelId": model_id }))
}

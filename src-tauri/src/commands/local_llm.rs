use crate::commands::CmdError;
use crate::AppState;
use ha_core::agent::AssistantAgent;
use ha_core::event_bus::EventBusProgressExt;
use ha_core::local_llm::{
    detect_hardware, detect_ollama, install_ollama_via_script, pull_and_activate, recommend_model,
    start_ollama, HardwareInfo, ModelCandidate, ModelRecommendation, OllamaStatus,
    EVENT_LOCAL_LLM_INSTALL_PROGRESS, EVENT_LOCAL_LLM_PULL_PROGRESS,
};
use ha_core::provider::{known_local_backends, KnownLocalBackend};
use serde_json::json;
use tauri::State;

#[tauri::command]
pub async fn local_llm_detect_hardware() -> Result<HardwareInfo, CmdError> {
    Ok(detect_hardware())
}

#[tauri::command]
pub async fn local_llm_recommend_model() -> Result<ModelRecommendation, CmdError> {
    let hw = detect_hardware();
    Ok(recommend_model(&hw))
}

#[tauri::command]
pub async fn local_llm_detect_ollama() -> Result<OllamaStatus, CmdError> {
    Ok(detect_ollama().await)
}

#[tauri::command]
pub async fn local_llm_known_backends() -> Result<Vec<KnownLocalBackend>, CmdError> {
    Ok(known_local_backends())
}

/// Run the bundled installer (Unix only). Progress is emitted via the
/// shared event bus on `EVENT_LOCAL_LLM_INSTALL_PROGRESS`; the frontend listens
/// for those events instead of receiving a Tauri Channel.
#[tauri::command]
pub async fn local_llm_install_ollama() -> Result<(), CmdError> {
    let bus = ha_core::get_event_bus()
        .cloned()
        .ok_or_else(|| CmdError::msg("EventBus not initialized"))?;
    install_ollama_via_script(bus.emit_progress(EVENT_LOCAL_LLM_INSTALL_PROGRESS))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_start_ollama() -> Result<(), CmdError> {
    start_ollama().await.map_err(Into::into)
}

/// Pull the requested model, register the local-Ollama provider, and switch
/// the active model. Progress streams through the event bus on
/// `EVENT_LOCAL_LLM_PULL_PROGRESS`. Returns the new `(provider_id, model_id)`.
#[tauri::command]
pub async fn local_llm_pull_and_activate(
    model: ModelCandidate,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, CmdError> {
    let bus = ha_core::get_event_bus()
        .cloned()
        .ok_or_else(|| CmdError::msg("EventBus not initialized"))?;
    let (provider_id, model_id) =
        pull_and_activate(model, bus.emit_progress(EVENT_LOCAL_LLM_PULL_PROGRESS)).await?;

    // Rebuild the cached agent so the next chat call uses the new local
    // provider without requiring a frontend reload.
    let provider = ha_core::config::cached_config()
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .cloned()
        .ok_or_else(|| {
            CmdError::msg(format!("Provider not found after register: {provider_id}"))
        })?;
    let agent = AssistantAgent::new_from_provider(&provider, &model_id);
    *state.agent.lock().await = Some(agent);
    Ok(json!({ "providerId": provider_id, "modelId": model_id }))
}

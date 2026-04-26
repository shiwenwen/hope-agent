use crate::commands::CmdError;
use crate::AppState;
use ha_core::local_llm::{
    add_ollama_model_as_embedding_config, delete_ollama_model, detect_hardware, detect_ollama,
    get_ollama_library_model, list_local_ollama_models, preload_ollama_model, recommend_model,
    register_ollama_model_as_provider, search_ollama_library, start_ollama, stop_ollama_model,
    HardwareInfo, LocalModelDeleteResult, LocalOllamaModel, ModelRecommendation,
    OllamaLibraryModelDetail, OllamaLibrarySearchResponse, OllamaModelActionResult,
    OllamaModelRegistration, OllamaStatus,
};
use ha_core::memory::EmbeddingModelConfig;
use ha_core::provider::{known_local_backends, KnownLocalBackend};
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

#[tauri::command]
pub async fn local_llm_start_ollama() -> Result<(), CmdError> {
    start_ollama().await.map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_list_models() -> Result<Vec<LocalOllamaModel>, CmdError> {
    list_local_ollama_models().await.map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_search_library(
    query: Option<String>,
) -> Result<OllamaLibrarySearchResponse, CmdError> {
    search_ollama_library(query.as_deref().unwrap_or(""))
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_get_library_model(
    model: String,
) -> Result<OllamaLibraryModelDetail, CmdError> {
    get_ollama_library_model(&model).await.map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_preload_model(
    model_id: String,
) -> Result<OllamaModelActionResult, CmdError> {
    preload_ollama_model(&model_id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_stop_model(model_id: String) -> Result<OllamaModelActionResult, CmdError> {
    stop_ollama_model(&model_id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_delete_model(
    model_id: String,
    state: State<'_, AppState>,
) -> Result<LocalModelDeleteResult, CmdError> {
    let result = delete_ollama_model(&model_id).await?;
    if result.removed_active_model || result.removed_provider {
        *state.agent.lock().await = None;
    }
    Ok(result)
}

#[tauri::command]
pub async fn local_llm_add_provider_model(
    model_id: String,
) -> Result<OllamaModelRegistration, CmdError> {
    register_ollama_model_as_provider(&model_id, None, false)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn local_llm_set_default_model(
    model_id: String,
    state: State<'_, AppState>,
) -> Result<OllamaModelRegistration, CmdError> {
    let registration = register_ollama_model_as_provider(&model_id, None, true).await?;
    if let Some(provider_id) = registration.provider_id.as_deref() {
        crate::commands::provider::set_active_model_core(
            provider_id,
            &registration.model_id,
            &state,
        )
        .await?;
    }
    Ok(registration)
}

#[tauri::command]
pub async fn local_llm_add_embedding_config(
    model_id: String,
) -> Result<EmbeddingModelConfig, CmdError> {
    add_ollama_model_as_embedding_config(&model_id)
        .await
        .map_err(Into::into)
}

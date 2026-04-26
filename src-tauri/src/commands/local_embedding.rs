use crate::commands::CmdError;
use ha_core::local_embedding::{
    list_models_with_status, pull_and_activate, OllamaEmbeddingModel,
    EVENT_LOCAL_EMBEDDING_PULL_PROGRESS,
};
use serde_json::json;

#[tauri::command]
pub async fn local_embedding_list_models() -> Result<Vec<OllamaEmbeddingModel>, CmdError> {
    Ok(list_models_with_status().await)
}

#[tauri::command]
pub async fn local_embedding_pull_and_activate(
    model: OllamaEmbeddingModel,
) -> Result<ha_core::memory::EmbeddingConfig, CmdError> {
    let bus = ha_core::get_event_bus()
        .cloned()
        .ok_or_else(|| CmdError::msg("EventBus not initialized"))?;
    pull_and_activate(model, move |p| {
        bus.emit(EVENT_LOCAL_EMBEDDING_PULL_PROGRESS, json!(p));
    })
    .await
    .map_err(Into::into)
}

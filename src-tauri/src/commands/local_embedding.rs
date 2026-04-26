use crate::commands::CmdError;
use ha_core::local_embedding::{list_models_with_status, OllamaEmbeddingModel};

#[tauri::command]
pub async fn local_embedding_list_models() -> Result<Vec<OllamaEmbeddingModel>, CmdError> {
    Ok(list_models_with_status().await)
}

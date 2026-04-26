use crate::commands::CmdError;
use crate::memory;

#[tauri::command]
pub async fn test_embedding(config: memory::EmbeddingConfig) -> Result<String, CmdError> {
    ha_core::provider::test::test_embedding(config)
        .await
        .map_err(CmdError::msg)
}

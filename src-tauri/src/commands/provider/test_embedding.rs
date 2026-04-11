use crate::memory;

#[tauri::command]
pub async fn test_embedding(config: memory::EmbeddingConfig) -> Result<String, String> {
    oc_core::provider::test::test_embedding(config).await
}

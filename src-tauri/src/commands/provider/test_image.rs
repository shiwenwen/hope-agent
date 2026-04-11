#[tauri::command]
pub async fn test_image_generate(
    provider_id: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<String, String> {
    oc_core::provider::test::test_image_generate(provider_id, api_key, base_url).await
}

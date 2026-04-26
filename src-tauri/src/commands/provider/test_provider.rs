use crate::commands::CmdError;
use crate::provider::ProviderConfig;

#[tauri::command]
pub async fn test_provider(config: ProviderConfig) -> Result<String, CmdError> {
    ha_core::provider::test::test_provider(config)
        .await
        .map_err(CmdError::msg)
}

/// Single-turn chat probe used by the Settings panel's "Test model" button.
/// Full implementation lives in [`ha_core::provider::test::test_model`] so
/// both the Tauri shell and the HTTP route share the same body.
#[tauri::command]
pub async fn test_model(config: ProviderConfig, model_id: String) -> Result<String, CmdError> {
    ha_core::provider::test::test_model(config, model_id)
        .await
        .map_err(CmdError::msg)
}

use crate::commands::CmdError;
use ha_core::local_llm::auto_maintainer;

#[tauri::command]
pub async fn local_model_alert_dismiss_temporary(model_id: String) -> Result<(), CmdError> {
    auto_maintainer::dismiss_alert_temporary(&model_id).await;
    Ok(())
}

#[tauri::command]
pub async fn local_model_alert_silence_session(model_id: String) -> Result<(), CmdError> {
    auto_maintainer::silence_for_session(&model_id).await;
    Ok(())
}

#[tauri::command]
pub async fn get_local_llm_auto_maintenance_enabled() -> Result<bool, CmdError> {
    Ok(auto_maintainer::get_auto_maintenance_enabled())
}

#[tauri::command]
pub async fn set_local_llm_auto_maintenance_enabled(enabled: bool) -> Result<(), CmdError> {
    auto_maintainer::set_auto_maintenance_enabled(enabled).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_auto_maintenance_disable() -> Result<(), CmdError> {
    auto_maintainer::disable_via_alert_dialog().map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_auto_maintenance_trigger() -> Result<(), CmdError> {
    auto_maintainer::trigger();
    Ok(())
}

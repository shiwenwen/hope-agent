use crate::commands::CmdError;

#[tauri::command]
pub async fn cancel_runtime_task(
    kind: ha_core::runtime_tasks::RuntimeTaskKind,
    id: String,
) -> Result<ha_core::runtime_tasks::CancelRuntimeTaskResult, CmdError> {
    ha_core::runtime_tasks::cancel_runtime_task(kind, &id)
        .await
        .map_err(Into::into)
}

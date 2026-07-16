use ha_core::file_upload::{FileUploadLease, FileUploadStartInput};

use super::CmdError;

#[tauri::command]
pub async fn file_upload_start(input: FileUploadStartInput) -> Result<FileUploadLease, CmdError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::start_upload(input))
        .await
        .map_err(|error| CmdError::msg(error.to_string()))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn file_upload_status(upload_id: String) -> Result<FileUploadLease, CmdError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::upload_status(&upload_id))
        .await
        .map_err(|error| CmdError::msg(error.to_string()))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn file_upload_chunk(
    request: tauri::ipc::Request<'_>,
) -> Result<FileUploadLease, CmdError> {
    let upload_id = request
        .headers()
        .get("x-hope-upload-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CmdError::msg("missing x-hope-upload-id header"))?
        .to_string();
    let offset = request
        .headers()
        .get("x-hope-upload-offset")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CmdError::msg("missing x-hope-upload-offset header"))?
        .parse::<u64>()
        .map_err(|_| CmdError::msg("invalid x-hope-upload-offset header"))?;
    let data = match request.body() {
        tauri::ipc::InvokeBody::Raw(data) => data.clone(),
        _ => {
            return Err(CmdError::msg(
                "file upload chunk requires a binary IPC body",
            ))
        }
    };
    drop(request);
    tokio::task::spawn_blocking(move || {
        ha_core::file_upload::upload_chunk(&upload_id, offset, &data)
    })
    .await
    .map_err(|error| CmdError::msg(error.to_string()))?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn file_upload_complete(upload_id: String) -> Result<FileUploadLease, CmdError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::complete_upload(&upload_id))
        .await
        .map_err(|error| CmdError::msg(error.to_string()))?
        .map_err(Into::into)
}

#[tauri::command]
pub async fn file_upload_discard(upload_id: String) -> Result<(), CmdError> {
    tokio::task::spawn_blocking(move || ha_core::file_upload::discard_upload(&upload_id))
        .await
        .map_err(|error| CmdError::msg(error.to_string()))?
        .map_err(Into::into)
}

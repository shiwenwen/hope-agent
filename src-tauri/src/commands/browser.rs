use crate::browser_ui;
use crate::commands::CmdError;

#[tauri::command]
pub async fn browser_get_status() -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::get_status().await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_list_profiles() -> Result<Vec<browser_ui::BrowserProfileInfo>, CmdError> {
    browser_ui::list_profiles().await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_create_profile(
    name: String,
) -> Result<browser_ui::BrowserProfileInfo, CmdError> {
    browser_ui::create_profile(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_delete_profile(name: String) -> Result<(), CmdError> {
    browser_ui::delete_profile(&name).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_launch(
    options: browser_ui::LaunchOptions,
) -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::launch(options).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_connect(url: String) -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::connect(&url).await.map_err(Into::into)
}

#[tauri::command]
pub async fn browser_disconnect() -> Result<browser_ui::BrowserStatus, CmdError> {
    browser_ui::disconnect().await.map_err(Into::into)
}

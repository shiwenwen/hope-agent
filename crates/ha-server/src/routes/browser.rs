//! REST routes for the embedded browser settings panel.

use axum::{extract::Path, Json};
use ha_core::browser_ui;
use serde::Deserialize;

use crate::error::AppError;

/// `GET /api/browser/status`
pub async fn get_status() -> Result<Json<browser_ui::BrowserStatus>, AppError> {
    Ok(Json(browser_ui::get_status().await?))
}

/// `GET /api/browser/profiles`
pub async fn list_profiles() -> Result<Json<Vec<browser_ui::BrowserProfileInfo>>, AppError> {
    Ok(Json(browser_ui::list_profiles().await?))
}

#[derive(Debug, Deserialize)]
pub struct CreateProfileBody {
    pub name: String,
}

/// `POST /api/browser/profiles`
pub async fn create_profile(
    Json(body): Json<CreateProfileBody>,
) -> Result<Json<browser_ui::BrowserProfileInfo>, AppError> {
    browser_ui::create_profile(&body.name)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

/// `DELETE /api/browser/profiles/{name}`
pub async fn delete_profile(Path(name): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    browser_ui::delete_profile(&name)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct LaunchBody {
    pub options: browser_ui::LaunchOptions,
}

/// `POST /api/browser/launch`
pub async fn launch(
    Json(body): Json<LaunchBody>,
) -> Result<Json<browser_ui::BrowserStatus>, AppError> {
    Ok(Json(browser_ui::launch(body.options).await?))
}

#[derive(Debug, Deserialize)]
pub struct ConnectBody {
    pub url: String,
}

/// `POST /api/browser/connect`
pub async fn connect(
    Json(body): Json<ConnectBody>,
) -> Result<Json<browser_ui::BrowserStatus>, AppError> {
    browser_ui::connect(&body.url)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

/// `POST /api/browser/disconnect`
pub async fn disconnect() -> Result<Json<browser_ui::BrowserStatus>, AppError> {
    Ok(Json(browser_ui::disconnect().await?))
}

/// `POST /api/browser/capture-frame`
///
/// Returns the most recent JPEG frame of the active tab (base64-encoded
/// inside the JSON envelope) for the chat-side BrowserPanel mirror. `null`
/// when no backend is active — the panel uses that as its "empty" signal.
pub async fn capture_frame(
) -> Result<Json<Option<ha_core::browser::frame::BrowserFramePayload>>, AppError> {
    Ok(Json(ha_core::browser::frame::capture_frame().await?))
}

/// `POST /api/browser/spawn-user-chrome`
pub async fn spawn_user_chrome(
    Json(args): Json<ha_core::browser::user_attach::SpawnUserChromeArgs>,
) -> Result<Json<ha_core::browser::user_attach::SpawnUserChromeResult>, AppError> {
    ha_core::browser::user_attach::spawn_user_chrome(args)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

/// `GET /api/browser/doctor`
pub async fn doctor() -> Result<Json<ha_core::browser::user_attach::BrowserDoctorReport>, AppError>
{
    Ok(Json(ha_core::browser::user_attach::browser_doctor().await))
}

/// `GET /api/browser/config`
pub async fn get_config() -> Result<Json<ha_core::browser::BrowserConfig>, AppError> {
    Ok(Json(
        ha_core::config::cached_config()
            .browser
            .clone()
            .unwrap_or_default(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct SetConfigBody {
    pub config: ha_core::browser::BrowserConfig,
}

/// `POST /api/browser/config`
///
/// Body matches the Tauri command shape: `{ "config": { ... } }`. Without
/// the wrapper, serde silently coerces unknown top-level fields and writes
/// an all-default `BrowserConfig` (which would wipe `userAttach`).
pub async fn set_config(
    Json(SetConfigBody { config }): Json<SetConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    ha_core::config::mutate_config::<_, ()>(("browser", "settings-ui"), |cfg| {
        cfg.browser = Some(config);
        Ok(())
    })
    .map_err(|e| AppError::bad_request(e.to_string()))?;
    // Force the next `acquire_backend()` to honor the new preference;
    // otherwise `ACTIVE_BACKEND` stays cached at the previous choice.
    ha_core::browser::reset_backend().await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

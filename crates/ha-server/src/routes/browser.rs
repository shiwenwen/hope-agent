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

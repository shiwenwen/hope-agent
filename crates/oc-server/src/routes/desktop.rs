//! Desktop-only UI commands.
//!
//! Commands like "open URL in external browser" or "reveal file in Finder"
//! only make sense when the client and server share a desktop session. In
//! headless server mode they're accepted for API-compat but logged + no-oped.

use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

/// `POST /api/desktop/open-url` — desktop-only, no-op in server mode.
pub async fn open_url(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "ok": false,
        "note": "desktop-only: server mode cannot open URLs in a remote user's browser",
    })))
}

/// `POST /api/desktop/open-directory` — desktop-only, no-op in server mode.
pub async fn open_directory(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "ok": false,
        "note": "desktop-only: server mode has no file-manager to open",
    })))
}

/// `POST /api/desktop/reveal-in-folder` — desktop-only, no-op in server mode.
pub async fn reveal_in_folder(Json(_body): Json<Value>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "ok": false,
        "note": "desktop-only: server mode has no file-manager to reveal in",
    })))
}

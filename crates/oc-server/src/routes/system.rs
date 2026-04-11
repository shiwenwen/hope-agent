//! Desktop-only system-level commands.
//!
//! In the Tauri desktop shell these manipulate the host application window
//! / autostart registration / process lifecycle. The HTTP server has no
//! window to restart, so every handler in this file is a no-op acknowledgement
//! returning 200 with `note: "desktop-only"` — enough to keep the client
//! from receiving a 404.

use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

/// `POST /api/system/restart` — desktop-only. Ignored in server mode.
pub async fn request_app_restart() -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "ok": false,
        "note": "desktop-only: server mode does not own an app process to restart",
    })))
}

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

/// `GET /api/system/timezone` — server's IANA timezone.
///
/// Mirrors the Tauri `get_system_timezone` command. Reads `/etc/localtime`
/// (macOS/Linux) and falls back to the `TZ` env var, finally `"UTC"`.
///
/// **Important**: returning server time — not the browser's. See the
/// `UserConfig.timezone` injection at `user_config.rs` and
/// `system_prompt::helpers::current_date`: the system prompt's "today is X"
/// line comes from `date +%Y-%m-%d %Z` on the server, so the profile
/// default must be the same reference to stay internally consistent when
/// the model interprets relative times like "tomorrow at 3pm".
pub async fn get_system_timezone() -> Result<Json<String>, AppError> {
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let path_str = link.to_string_lossy().to_string();
        if let Some(pos) = path_str.find("zoneinfo/") {
            return Ok(Json(path_str[pos + 9..].to_string()));
        }
    }
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return Ok(Json(tz));
        }
    }
    Ok(Json("UTC".to_string()))
}

//! System-level commands. The desktop GUI delegates here for restart now
//! that `ha_core::lifecycle::restart` covers both server and desktop modes;
//! the same endpoint also handles foreground / installed-service restarts
//! when called by an HTTP client (`hope-agent server` daemon).

use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;

/// `POST /api/system/restart` — restart the running process. Routes through
/// [`ha_core::lifecycle::restart`] which picks the right strategy for the
/// current runtime (desktop / installed-service / foreground / acp).
///
/// Note: this endpoint does NOT run the pre-flight / Yes-No confirmation
/// gates that the `app_restart` tool wraps around the same call. It's
/// meant for the GUI's own "Restart App" button, which has already done
/// its own UX-level "Are you sure?" — and for advanced HTTP clients that
/// want a programmatic restart without involving the LLM.
pub async fn request_app_restart() -> Result<Json<Value>, AppError> {
    match ha_core::lifecycle::restart() {
        Ok(outcome) => Ok(Json(json!({
            "ok": true,
            "route": outcome.route.as_str(),
            "detail": outcome.detail,
        }))),
        Err(e) => Ok(Json(json!({
            "ok": false,
            "error": e.to_string(),
            "runtime_role": ha_core::app_init::runtime_role().unwrap_or("unknown"),
        }))),
    }
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

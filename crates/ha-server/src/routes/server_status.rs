//! `/api/server/status` — unauthenticated (mirrors `/api/health`) so the
//! Transport layer can probe without an API key. No secrets in the payload.

use axum::Json;
use serde_json::Value;

pub async fn server_status() -> Json<Value> {
    Json(ha_core::server_status::runtime_status_json(false))
}

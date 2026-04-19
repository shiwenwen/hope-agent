use axum::Json;
use serde_json::{json, Value};

/// `GET /api/health` — basic health check.
pub async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

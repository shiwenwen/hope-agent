//! Local Ollama embedding assistant routes.
//!
//! The pull/activate flow is now a background job under
//! `/api/local-model-jobs/*`; this module only lists the catalog.

use axum::Json;
use serde_json::{json, Value};

use ha_core::local_embedding::list_models_with_status;

/// `GET /api/local-embedding/models` — static catalog plus local install state.
pub async fn list_models() -> Json<Value> {
    Json(json!(list_models_with_status().await))
}

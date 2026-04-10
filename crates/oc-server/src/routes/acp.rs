use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use oc_core::acp_control::config::AcpControlConfig;
use oc_core::acp_control::types::{AcpBackendInfo, AcpRun};

use crate::error::AppError;

/// `GET /api/acp/backends`
pub async fn list_backends() -> Result<Json<Vec<AcpBackendInfo>>, AppError> {
    let store = oc_core::config::cached_config();
    if !store.acp_control.enabled {
        return Ok(Json(Vec::new()));
    }
    let mut backends = Vec::new();
    for b in &store.acp_control.backends {
        let binary_path = if std::path::Path::new(&b.binary).is_absolute() {
            if std::path::Path::new(&b.binary).exists() {
                Some(b.binary.clone())
            } else {
                None
            }
        } else {
            oc_core::acp_control::registry::resolve_binary(&b.binary)
        };
        let health = if let Some(path) = &binary_path {
            oc_core::acp_control::health::probe_binary(path).await
        } else {
            oc_core::acp_control::health::build_health_status(
                false,
                None,
                None,
                Some(format!("Binary '{}' not found in PATH", b.binary)),
            )
        };
        backends.push(AcpBackendInfo {
            id: b.id.clone(),
            name: b.name.clone(),
            enabled: b.enabled,
            health,
            capabilities: oc_core::acp_control::types::AcpRuntimeCapabilities::default(),
        });
    }
    Ok(Json(backends))
}

/// `POST /api/acp/refresh`
pub async fn refresh_backends() -> Result<Json<Value>, AppError> {
    if let Some(_manager) = oc_core::get_acp_manager() {
        let store = oc_core::config::cached_config();
        let registry = std::sync::Arc::new(oc_core::acp_control::AcpRuntimeRegistry::new());
        oc_core::acp_control::registry::auto_discover_and_register(&registry, &store.acp_control)
            .await;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
pub struct ListRunsQuery {
    pub parent_session_id: Option<String>,
}

/// `GET /api/acp/runs?parent_session_id=...`
pub async fn list_runs(
    Query(q): Query<ListRunsQuery>,
) -> Result<Json<Vec<AcpRun>>, AppError> {
    if let Some(manager) = oc_core::get_acp_manager() {
        Ok(Json(manager.list_runs(q.parent_session_id.as_deref()).await))
    } else if let Some(db) = oc_core::get_session_db() {
        if let Some(pid) = q.parent_session_id {
            Ok(Json(db.list_acp_runs(&pid)?))
        } else {
            Ok(Json(Vec::new()))
        }
    } else {
        Ok(Json(Vec::new()))
    }
}

/// `POST /api/acp/runs/{run_id}/kill`
pub async fn kill_run(Path(run_id): Path<String>) -> Result<Json<Value>, AppError> {
    let manager = oc_core::get_acp_manager()
        .ok_or_else(|| AppError::internal("ACP control plane not initialized"))?;
    manager
        .kill_run(&run_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "killed": true })))
}

/// `GET /api/acp/runs/{run_id}/result`
pub async fn get_run_result(Path(run_id): Path<String>) -> Result<Json<Value>, AppError> {
    let manager = oc_core::get_acp_manager()
        .ok_or_else(|| AppError::internal("ACP control plane not initialized"))?;
    let result = manager
        .get_result(&run_id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "result": result })))
}

/// `GET /api/acp/config`
pub async fn get_config() -> Result<Json<AcpControlConfig>, AppError> {
    Ok(Json(oc_core::config::cached_config().acp_control.clone()))
}

/// `PUT /api/acp/config`
pub async fn set_config(
    Json(config): Json<AcpControlConfig>,
) -> Result<Json<Value>, AppError> {
    let mut store = oc_core::config::load_config()?;
    store.acp_control = config;
    oc_core::config::save_config(&store)?;
    Ok(Json(json!({ "saved": true })))
}

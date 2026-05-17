//! HTTP routes for macOS control readiness.
//!
//! Server/headless mode has no authorized desktop bridge, so these routes
//! intentionally return the same JSON shape with `supported=false`.

use axum::Json;
use serde::Deserialize;

use crate::error::AppError;

const HTTP_UNSUPPORTED_MESSAGE: &str =
    "macOS control is unavailable from the HTTP/server transport.";

pub async fn status() -> Result<Json<ha_core::mac_control::MacControlStatus>, AppError> {
    Ok(Json(ha_core::mac_control::unsupported_status(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

pub async fn permissions(
) -> Result<Json<ha_core::mac_control::MacControlPermissionsResponse>, AppError> {
    Ok(Json(
        ha_core::mac_control::unsupported_permissions_response(HTTP_UNSUPPORTED_MESSAGE),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBody {
    pub options: Option<ha_core::mac_control::MacControlSnapshotRequest>,
}

pub async fn snapshot(
    Json(body): Json<SnapshotBody>,
) -> Result<Json<ha_core::mac_control::MacControlSnapshotResponse>, AppError> {
    let _requested = body.options.unwrap_or_default();
    Ok(Json(ha_core::mac_control::unsupported_snapshot_response(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

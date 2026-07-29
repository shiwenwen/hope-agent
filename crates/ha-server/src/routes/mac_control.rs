//! HTTP routes for macOS control readiness.
//!
//! Server/headless mode has no authorized desktop bridge, so these routes
//! intentionally return the same JSON shape with `supported=false`.

use axum::Json;
use serde::Deserialize;

use crate::error::AppError;

const HTTP_UNSUPPORTED_MESSAGE: &str =
    "macOS control is unavailable from the HTTP/server transport.";

pub async fn status() -> Result<Json<ha_mac::MacControlStatus>, AppError> {
    Ok(Json(ha_mac::unsupported_status(HTTP_UNSUPPORTED_MESSAGE)))
}

pub async fn permissions() -> Result<Json<ha_mac::MacControlPermissionsResponse>, AppError> {
    Ok(Json(ha_mac::unsupported_permissions_response(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBody {
    pub options: Option<ha_mac::MacControlSnapshotRequest>,
}

pub async fn snapshot(
    body: Option<Json<SnapshotBody>>,
) -> Result<Json<ha_mac::MacControlSnapshotResponse>, AppError> {
    let _requested = body.and_then(|Json(body)| body.options).unwrap_or_default();
    Ok(Json(ha_mac::unsupported_snapshot_response(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementsBody {
    pub options: Option<ha_mac::MacControlElementsRequest>,
}

pub async fn elements(
    body: Option<Json<ElementsBody>>,
) -> Result<Json<ha_mac::MacControlElementsResponse>, AppError> {
    let _requested = body.and_then(|Json(body)| body.options).unwrap_or_default();
    Ok(Json(ha_mac::unsupported_elements_response(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

pub async fn capture_frame() -> Result<Json<ha_mac::MacControlFrameResponse>, AppError> {
    Ok(Json(ha_mac::unsupported_frame_response(
        HTTP_UNSUPPORTED_MESSAGE,
    )))
}

pub async fn displays() -> Result<Json<ha_mac::MacControlDisplaysResponse>, AppError> {
    Ok(Json(ha_mac::MacControlDisplaysResponse {
        displays: Vec::new(),
        error: Some(HTTP_UNSUPPORTED_MESSAGE.to_string()),
    }))
}

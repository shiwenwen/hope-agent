use axum::extract::Path;
use axum::Json;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn get_lsp_status(
    Path(session_id): Path<String>,
) -> Result<Json<ha_core::lsp::LspStatusSnapshot>, AppError> {
    Ok(Json(
        ha_core::lsp::status_for_session(session_db()?, &session_id)
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}

pub async fn get_lsp_diagnostics(
    Path(session_id): Path<String>,
) -> Result<Json<ha_core::lsp::LspDiagnosticsSnapshot>, AppError> {
    Ok(Json(
        ha_core::lsp::diagnostics_for_session(session_db()?, &session_id)
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?,
    ))
}

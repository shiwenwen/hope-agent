//! HTTP adapter for interactive terminal PTY sessions.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use ha_core::terminal::{
    CreateTerminalRequest, TerminalSnapshot, TerminalSummary, REMOTE_TERMINAL_ACCESS_DISABLED,
};

use crate::error::AppError;
use crate::AppContext;

fn ensure_terminal_access_allowed() -> Result<(), AppError> {
    ensure_terminal_access_allowed_for(
        ha_core::config::cached_config()
            .filesystem
            .allow_remote_writes,
    )
}

fn ensure_terminal_access_allowed_for(allow_remote_writes: bool) -> Result<(), AppError> {
    if allow_remote_writes {
        Ok(())
    } else {
        Err(AppError::forbidden(REMOTE_TERMINAL_ACCESS_DISABLED))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBody {
    pub request: CreateTerminalRequest,
}

pub async fn create(
    State(ctx): State<Arc<AppContext>>,
    Json(body): Json<CreateBody>,
) -> Result<Json<TerminalSnapshot>, AppError> {
    ensure_terminal_access_allowed()?;
    let manager = Arc::clone(&ctx.terminal_manager);
    let snapshot = tokio::task::spawn_blocking(move || manager.create_remote(body.request))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map_err(|error| {
            if error.to_string() == REMOTE_TERMINAL_ACCESS_DISABLED {
                AppError::forbidden(REMOTE_TERMINAL_ACCESS_DISABLED)
            } else {
                AppError::bad_request(error.to_string())
            }
        })?;
    Ok(Json(snapshot))
}

pub async fn list(
    State(ctx): State<Arc<AppContext>>,
) -> Result<Json<Vec<TerminalSummary>>, AppError> {
    ensure_terminal_access_allowed()?;
    Ok(Json(ctx.terminal_manager.list()))
}

pub async fn snapshot(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<TerminalSnapshot>, AppError> {
    ensure_terminal_access_allowed()?;
    let manager = Arc::clone(&ctx.terminal_manager);
    tokio::task::spawn_blocking(move || manager.snapshot(&id))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map(Json)
        .map_err(|error| AppError::not_found(error.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct WriteBody {
    pub data: String,
}

pub async fn write(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<WriteBody>,
) -> Result<Json<Value>, AppError> {
    ensure_terminal_access_allowed()?;
    let manager = Arc::clone(&ctx.terminal_manager);
    tokio::task::spawn_blocking(move || manager.write_input(&id, &body.data))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(json!({ "written": true })))
}

#[derive(Debug, Deserialize)]
pub struct ResizeBody {
    pub cols: u16,
    pub rows: u16,
}

pub async fn resize(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
    Json(body): Json<ResizeBody>,
) -> Result<Json<Value>, AppError> {
    ensure_terminal_access_allowed()?;
    let manager = Arc::clone(&ctx.terminal_manager);
    tokio::task::spawn_blocking(move || manager.resize(&id, body.cols, body.rows))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    Ok(Json(json!({ "resized": true })))
}

pub async fn close(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    ensure_terminal_access_allowed()?;
    let manager = Arc::clone(&ctx.terminal_manager);
    tokio::task::spawn_blocking(move || manager.close(&id))
        .await
        .map_err(|error| AppError::internal(error.to_string()))?
        .map_err(|error| AppError::not_found(error.to_string()))?;
    Ok(Json(json!({ "closed": true })))
}

#[cfg(test)]
mod tests {
    use super::ensure_terminal_access_allowed_for;

    #[test]
    fn remote_terminal_access_fails_closed() {
        assert!(ensure_terminal_access_allowed_for(false).is_err());
    }

    #[test]
    fn remote_terminal_access_requires_explicit_opt_in() {
        assert!(ensure_terminal_access_allowed_for(true).is_ok());
    }
}

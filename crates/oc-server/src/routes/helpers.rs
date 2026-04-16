//! Shared accessors for oc-core globals, wrapped as `Result<_, AppError>`
//! so handlers can use `?` instead of re-implementing the unwrap boilerplate
//! in every route file.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::Multipart;
use oc_core::channel::{ChannelDB, ChannelRegistry};
use oc_core::cron::CronDB;
use oc_core::session::SessionDB;
use oc_core::AppState;

use crate::error::AppError;

// ── Multipart file upload parsing ──────────────────────────────

/// Parsed result from a multipart file upload request.
pub struct ParsedUpload {
    pub file_data: Vec<u8>,
    pub file_name: String,
    pub mime_type: Option<String>,
    /// Any additional text fields beyond `file`/`fileName`/`mimeType`.
    pub extra_fields: HashMap<String, String>,
}

/// Parse a multipart request that contains a single `file` part plus
/// optional `fileName`, `mimeType`, and arbitrary text metadata fields.
pub async fn parse_file_upload(mut multipart: Multipart) -> Result<ParsedUpload, AppError> {
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut extra = HashMap::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart parse error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                if file_name.is_none() {
                    file_name = field.file_name().map(|s| s.to_string());
                }
                if mime_type.is_none() {
                    mime_type = field.content_type().map(|s| s.to_string());
                }
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            AppError::bad_request(format!("failed to read file field: {e}"))
                        })?
                        .to_vec(),
                );
            }
            "fileName" => {
                file_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::bad_request(e.to_string()))?,
                );
            }
            "mimeType" => {
                mime_type = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::bad_request(e.to_string()))?,
                );
            }
            _ => {
                if let Ok(text) = field.text().await {
                    extra.insert(name, text);
                }
            }
        }
    }

    let file_data = file_data.ok_or_else(|| AppError::bad_request("missing 'file' field"))?;
    let file_name = file_name.unwrap_or_else(|| "attachment".to_string());

    Ok(ParsedUpload {
        file_data,
        file_name,
        mime_type,
        extra_fields: extra,
    })
}

pub fn app_state() -> Result<&'static Arc<AppState>, AppError> {
    oc_core::get_app_state().ok_or_else(|| AppError::internal("AppState not initialized"))
}

pub fn session_db() -> Result<&'static Arc<SessionDB>, AppError> {
    oc_core::get_session_db().ok_or_else(|| AppError::internal("Session DB not initialized"))
}

pub fn cron_db() -> Result<&'static Arc<CronDB>, AppError> {
    oc_core::get_cron_db().ok_or_else(|| AppError::internal("Cron DB not initialized"))
}

pub fn channel_registry() -> Result<&'static Arc<ChannelRegistry>, AppError> {
    oc_core::get_channel_registry()
        .ok_or_else(|| AppError::internal("Channel registry not initialized"))
}

pub fn channel_db() -> Result<&'static Arc<ChannelDB>, AppError> {
    oc_core::get_channel_db().ok_or_else(|| AppError::internal("Channel DB not initialized"))
}

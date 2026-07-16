//! Shared accessors for ha-core globals, wrapped as `Result<_, AppError>`
//! so handlers can use `?` instead of re-implementing the unwrap boilerplate
//! in every route file.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::multipart::Field;
use axum::extract::Multipart;
use ha_core::channel::{ChannelDB, ChannelRegistry};
use ha_core::cron::CronDB;
use ha_core::logging::{AppLogger, LogDB};
use ha_core::session::SessionDB;
use ha_core::subagent::SubagentCancelRegistry;
use tokio::io::AsyncWriteExt;

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

/// Disk-backed multipart result for potentially large chat attachments.
pub struct ParsedTempUpload {
    pub file_path: tempfile::TempPath,
    pub file_name: String,
    pub mime_type: Option<String>,
    pub extra_fields: HashMap<String, String>,
}

const MAX_UPLOAD_TEXT_FIELD_BYTES: usize = 64 * 1024;
const MAX_UPLOAD_FIELDS: usize = 32;

async fn read_small_text_field(mut field: Field<'_>) -> Result<String, AppError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart field read error: {e}")))?
    {
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| AppError::bad_request("multipart text field is too large"))?;
        if next_len > MAX_UPLOAD_TEXT_FIELD_BYTES {
            return Err(AppError::bad_request("multipart text field is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| AppError::bad_request("multipart text field is not valid UTF-8"))
}

/// Parse one multipart file directly to a temporary file while enforcing the
/// current configured limit. Any route-level cap is only a hard protocol
/// ceiling; it does not determine memory use.
pub async fn parse_file_upload_to_temp(
    mut multipart: Multipart,
    max_file_bytes: usize,
) -> Result<ParsedTempUpload, AppError> {
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut file_path: Option<tempfile::TempPath> = None;
    let mut extra = HashMap::new();
    let mut field_count = 0usize;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart parse error: {e}")))?
    {
        field_count += 1;
        if field_count > MAX_UPLOAD_FIELDS {
            return Err(AppError::bad_request("too many multipart fields"));
        }
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                if file_path.is_some() {
                    return Err(AppError::bad_request(
                        "multiple 'file' fields are not supported",
                    ));
                }
                if file_name.is_none() {
                    file_name = field.file_name().map(str::to_string);
                }
                if mime_type.is_none() {
                    mime_type = field.content_type().map(str::to_string);
                }
                let named = tempfile::NamedTempFile::new()
                    .map_err(|e| AppError::internal(format!("create upload temp file: {e}")))?;
                let (std_file, temp_path) = named.into_parts();
                let mut output = tokio::fs::File::from_std(std_file);
                let mut written = 0usize;
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| AppError::bad_request(format!("failed to read file field: {e}")))?
                {
                    written = written
                        .checked_add(chunk.len())
                        .ok_or_else(|| AppError::bad_request("attachment is too large"))?;
                    if written > max_file_bytes {
                        return Err(AppError::bad_request(format!(
                            "file exceeds the configured {} MiB limit",
                            max_file_bytes / (1024 * 1024)
                        )));
                    }
                    output
                        .write_all(&chunk)
                        .await
                        .map_err(|e| AppError::internal(format!("write upload temp file: {e}")))?;
                }
                output
                    .flush()
                    .await
                    .map_err(|e| AppError::internal(format!("flush upload temp file: {e}")))?;
                output
                    .sync_all()
                    .await
                    .map_err(|e| AppError::internal(format!("sync upload temp file: {e}")))?;
                drop(output);
                file_path = Some(temp_path);
            }
            "fileName" => file_name = Some(read_small_text_field(field).await?),
            "mimeType" => mime_type = Some(read_small_text_field(field).await?),
            _ => {
                extra.insert(name, read_small_text_field(field).await?);
            }
        }
    }

    Ok(ParsedTempUpload {
        file_path: file_path.ok_or_else(|| AppError::bad_request("missing 'file' field"))?,
        file_name: file_name.unwrap_or_else(|| "attachment".to_string()),
        mime_type,
        extra_fields: extra,
    })
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

pub fn session_db() -> Result<&'static Arc<SessionDB>, AppError> {
    Ok(ha_core::require_session_db()?)
}

pub fn cron_db() -> Result<&'static Arc<CronDB>, AppError> {
    Ok(ha_core::require_cron_db()?)
}

pub fn log_db() -> Result<&'static Arc<LogDB>, AppError> {
    Ok(ha_core::require_log_db()?)
}

pub fn logger() -> Result<&'static AppLogger, AppError> {
    Ok(ha_core::require_logger()?)
}

pub fn subagent_cancels() -> Result<&'static Arc<SubagentCancelRegistry>, AppError> {
    Ok(ha_core::require_subagent_cancels()?)
}

pub fn channel_registry() -> Result<&'static Arc<ChannelRegistry>, AppError> {
    ha_core::get_channel_registry()
        .ok_or_else(|| AppError::internal("Channel registry not initialized"))
}

pub fn channel_db() -> Result<&'static Arc<ChannelDB>, AppError> {
    ha_core::get_channel_db().ok_or_else(|| AppError::internal("Channel DB not initialized"))
}

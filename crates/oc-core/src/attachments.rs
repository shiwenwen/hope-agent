//! Attachment helpers shared by Tauri commands and HTTP routes.
//!
//! Writes uploaded bytes to the per-session attachments directory (or a
//! temporary bucket when the session hasn't been created yet) and returns
//! the absolute path so the caller can hand it to the agent/chat engine.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;

/// Kind of media item — drives frontend rendering (image preview vs file card).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    File,
}

/// Structured media attachment produced by a tool result.
/// Used by `send_attachment` and future tools that need to ship files with
/// filename + MIME metadata to the frontend. Emitted via the `__MEDIA_ITEMS__`
/// prefix in the tool result string (parallel to the simpler `__MEDIA_URLS__`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    /// Absolute path on disk (the frontend uses `convertFileSrc` for Tauri).
    pub url: String,
    /// Display filename (already sanitized).
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    pub kind: MediaKind,
}

/// Save an attachment's raw bytes to disk.
///
/// When `session_id` is `Some(non-empty)`, writes to
/// `~/.opencomputer/attachments/{session_id}/`. Otherwise falls back to a
/// shared temp bucket (`~/.opencomputer/attachments/_temp/`) so the caller
/// can stage files before a session exists.
///
/// The filename is prefixed with a Unix millisecond timestamp to avoid
/// collisions. Returns the absolute path of the written file.
pub fn save_attachment_bytes(
    session_id: Option<&str>,
    file_name: &str,
    data: &[u8],
) -> Result<String> {
    let att_dir: PathBuf = match session_id {
        Some(sid) if !sid.is_empty() => paths::attachments_dir(sid)?,
        _ => paths::root_dir()?.join("attachments").join("_temp"),
    };
    std::fs::create_dir_all(&att_dir)
        .with_context(|| format!("create attachments dir {}", att_dir.display()))?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let safe_name = file_name.replace(['/', '\\', ':'], "_");
    let filename = format!("{}_{}", ts, safe_name);
    let file_path = att_dir.join(&filename);

    std::fs::write(&file_path, data)
        .with_context(|| format!("write attachment {}", file_path.display()))?;

    Ok(file_path.to_string_lossy().to_string())
}

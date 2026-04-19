//! Attachment helpers shared by Tauri commands and HTTP routes.
//!
//! Writes uploaded bytes to the per-session attachments directory (or a
//! temporary bucket when the session hasn't been created yet) and returns
//! the absolute path so the caller can hand it to the agent/chat engine.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths;

/// Pseudo-session id for pre-session attachments (uploads that predate a
/// chat session). Maps to `~/.hope-agent/attachments/_temp/`.
pub const TEMP_SESSION_ID: &str = "_temp";

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
///
/// URL semantics: `url` is the logical reference
/// `/api/attachments/{sessionId}/{filename}` — frontend consumes directly
/// (HTTP sink appends `?token=`; Tauri sink leaves as-is, and the frontend
/// prefers `local_path` via `convertFileSrc`). `local_path` is the absolute
/// path on the server, used by IM channel workers to read bytes and by the
/// Tauri frontend to open/reveal locally. HTTP sinks strip `local_path`
/// from events so it never leaks to web clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    /// Logical URL `/api/attachments/{sessionId}/{filename}`. Frontends resolve
    /// this through the transport layer (Tauri uses `local_path`, HTTP adds
    /// `?token=`).
    pub url: String,
    /// Absolute server-side path. Present for outbound delivery (IM workers,
    /// Tauri file ops). Stripped before forwarding events over HTTP.
    #[serde(rename = "localPath", default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// Display filename (already sanitized).
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    pub kind: MediaKind,
    /// Optional caption / description shown with the attachment. Used as the
    /// IM caption when a channel API supports one (Telegram/WhatsApp/etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

impl MediaItem {
    /// Build a MediaItem for a file that was just persisted by
    /// `save_attachment_bytes`. Handles basename extraction, URL encoding,
    /// and the `_temp` session fallback so every callsite stays consistent.
    pub fn from_saved_path(
        session_id: Option<&str>,
        saved_path: &str,
        display_name: &str,
        mime_type: String,
        size_bytes: u64,
        kind: MediaKind,
        caption: Option<String>,
    ) -> Self {
        let basename = Path::new(saved_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(display_name);
        let sid = session_id
            .filter(|s| !s.is_empty())
            .unwrap_or(TEMP_SESSION_ID);
        let url = format!("/api/attachments/{}/{}", sid, urlencoding::encode(basename));
        Self {
            url,
            local_path: Some(saved_path.to_string()),
            name: display_name.to_string(),
            mime_type,
            size_bytes,
            kind,
            caption,
        }
    }
}

/// Save an attachment's raw bytes to disk.
///
/// When `session_id` is `Some(non-empty)`, writes to
/// `~/.hope-agent/attachments/{session_id}/`. Otherwise falls back to a
/// shared temp bucket (`~/.hope-agent/attachments/_temp/`) so the caller
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
        _ => paths::root_dir()?.join("attachments").join(TEMP_SESSION_ID),
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

// ── MIME Sniffing ───────────────────────────────────────────────

/// Sniff a MIME type: try magic bytes first, then extension, then fall back
/// to `application/octet-stream`. Shared between `send_attachment` and the
/// HTTP `/api/attachments/...` download route.
pub fn sniff_mime(data: &[u8], path: &Path) -> String {
    if let Some(m) = sniff_mime_magic(data) {
        return m.to_string();
    }
    if let Some(ext) = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
    {
        if let Some(m) = mime_from_extension(&ext) {
            return m.to_string();
        }
    }
    "application/octet-stream".to_string()
}

/// Match a prefix of the file against well-known magic bytes. Returns `None`
/// when no known signature matches.
pub fn sniff_mime_magic(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if data.len() >= 3 && &data[..3] == b"\xFF\xD8\xFF" {
        return Some("image/jpeg");
    }
    if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if data.len() >= 2 && &data[..2] == b"BM" {
        return Some("image/bmp");
    }
    if data.len() >= 4 && &data[..4] == b"%PDF" {
        return Some("application/pdf");
    }
    // ZIP family (also docx / xlsx / pptx / odt). Callers can drill down if
    // they need to distinguish Office from plain zip; `application/zip` is a
    // reasonable default for generic display.
    if data.len() >= 4 && &data[..4] == b"PK\x03\x04" {
        return Some("application/zip");
    }
    if data.len() >= 2 && &data[..2] == b"\x1F\x8B" {
        return Some("application/gzip");
    }
    if data.len() >= 6 && &data[..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some("application/x-7z-compressed");
    }
    if data.len() >= 7 && &data[..7] == b"Rar!\x1A\x07\x01" {
        return Some("application/vnd.rar");
    }
    // MP4 / QuickTime (ftyp box at offset 4).
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return Some("video/mp4");
    }
    None
}

/// Map a lowercase file extension to a best-guess MIME type.
pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "js" | "mjs" => "application/javascript",
        "ts" | "tsx" => "text/typescript",
        "py" => "text/x-python",
        "rs" => "text/rust",
        "go" => "text/x-go",
        "sh" | "bash" | "zsh" => "application/x-sh",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_png_magic() {
        assert_eq!(
            sniff_mime(b"\x89PNG\r\n\x1a\nrest", Path::new("x")),
            "image/png"
        );
    }

    #[test]
    fn sniff_pdf_magic() {
        assert_eq!(
            sniff_mime(b"%PDF-1.4\n...", Path::new("x.bin")),
            "application/pdf"
        );
    }

    #[test]
    fn sniff_fallback_ext() {
        assert_eq!(
            sniff_mime(b"plain text body", Path::new("/tmp/foo.txt")),
            "text/plain"
        );
    }

    #[test]
    fn sniff_fallback_octet_stream() {
        assert_eq!(
            sniff_mime(b"\x00\x01\x02unknown", Path::new("/tmp/x")),
            "application/octet-stream"
        );
    }
}

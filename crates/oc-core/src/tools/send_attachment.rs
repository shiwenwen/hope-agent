//! `send_attachment` tool — let the model push a file attachment to the user
//! in the frontend chat UI as a downloadable file card.
//!
//! Scope: desktop (Tauri) UI sessions only. IM channel sessions must use
//! their own native media path (Telegram/WeChat/etc. handle attachments via
//! the channel plugin's `ReplyPayload.media`), so this tool is blocked
//! when the session is bound to a channel — defense-in-depth matching the
//! schema-level filter in `AssistantAgent::build_tool_schemas`.
//!
//! Input: absolute path to an existing file inside the user's home dir.
//! Output: copied into `~/.opencomputer/attachments/{session_id}/` and
//! surfaced to the frontend via the `__MEDIA_ITEMS__` result prefix.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::execution::ToolExecContext;
use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{MediaItem, MediaKind};

/// 20 MB — aligned with `project::files::MAX_PROJECT_FILE_BYTES`.
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Max length of the optional description string.
const MAX_DESCRIPTION_CHARS: usize = 200;

pub(crate) async fn tool_send_attachment(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    // ── IM session guard (defense-in-depth) ───────────────────────
    let session_id = ctx
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("send_attachment requires a session; it is only available in the desktop UI."))?;

    if let Some(db) = crate::globals::get_session_db() {
        if let Some(meta) = db.get_session(&session_id)? {
            if meta.channel_info.is_some() {
                return Err(anyhow!(
                    "send_attachment is not available in IM channel sessions. \
                     The channel plugin handles media natively — produce the file \
                     and reference it by path in text, or use image_generate for images."
                ));
            }
        }
    }

    // ── Parse args ───────────────────────────────────────────────
    let path_arg = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("send_attachment: missing required arg `path`"))?
        .trim();
    if path_arg.is_empty() {
        return Err(anyhow!("send_attachment: `path` cannot be empty"));
    }
    let display_name_arg = args
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| crate::truncate_utf8(s.trim(), MAX_DESCRIPTION_CHARS).to_string())
        .filter(|s| !s.is_empty());

    // ── Path resolution & safety ─────────────────────────────────
    let expanded = super::expand_tilde(path_arg);
    let source_path = PathBuf::from(&expanded);
    let canonical = std::fs::canonicalize(&source_path).with_context(|| {
        format!(
            "send_attachment: cannot resolve path `{}` — does the file exist?",
            path_arg
        )
    })?;

    let home = dirs::home_dir()
        .ok_or_else(|| anyhow!("send_attachment: cannot determine user home directory"))?;
    let canonical_home = home
        .canonicalize()
        .unwrap_or_else(|_| home.clone());
    if !canonical.starts_with(&canonical_home) {
        return Err(anyhow!(
            "send_attachment: refusing to send a file outside the user home directory ({})",
            canonical.display()
        ));
    }

    if is_sensitive_path(&canonical, &canonical_home) {
        return Err(anyhow!(
            "send_attachment: refusing to send a file from a sensitive path ({})",
            canonical.display()
        ));
    }

    // ── File stat & read ─────────────────────────────────────────
    let metadata = std::fs::metadata(&canonical)
        .with_context(|| format!("send_attachment: stat failed for {}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "send_attachment: `{}` is not a regular file",
            canonical.display()
        ));
    }
    let size_bytes = metadata.len();
    if size_bytes > MAX_ATTACHMENT_BYTES {
        return Err(anyhow!(
            "send_attachment: file too large ({} bytes, max {} bytes)",
            size_bytes,
            MAX_ATTACHMENT_BYTES
        ));
    }

    let data = std::fs::read(&canonical)
        .with_context(|| format!("send_attachment: read failed for {}", canonical.display()))?;

    // ── Filename & MIME ──────────────────────────────────────────
    let source_basename = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment")
        .to_string();
    let display_name = display_name_arg
        .map(|s| s.to_string())
        .unwrap_or_else(|| source_basename.clone());

    let mime_type = sniff_mime(&data, &canonical);
    let kind = if mime_type.starts_with("image/") {
        MediaKind::Image
    } else {
        MediaKind::File
    };

    // ── Persist into attachments dir ─────────────────────────────
    let saved_path = crate::attachments::save_attachment_bytes(
        Some(session_id.as_str()),
        &display_name,
        &data,
    )
    .with_context(|| "send_attachment: failed to persist attachment")?;

    // ── Build structured result ──────────────────────────────────
    let item = MediaItem {
        url: saved_path.clone(),
        name: display_name.clone(),
        mime_type: mime_type.clone(),
        size_bytes,
        kind,
    };
    let items = vec![item];
    let items_json = serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string());

    let size_human = format_bytes(size_bytes);
    let mut text_parts = Vec::new();
    text_parts.push(format!(
        "Sent attachment \"{}\" ({}) to the user.",
        display_name, size_human
    ));
    text_parts.push(format!("MIME: {}", mime_type));
    text_parts.push(format!("Saved to: {}", saved_path));
    if let Some(d) = description {
        text_parts.push(format!("Caption: {}", d));
    }

    Ok(format!(
        "{}{}\n{}",
        MEDIA_ITEMS_PREFIX,
        items_json,
        text_parts.join("\n")
    ))
}

/// Reject a short deny list of sensitive files/directories even inside
/// the home dir (SSH keys, OAuth tokens, and OpenComputer's own
/// credentials store). Matching is prefix-based on the canonical path.
fn is_sensitive_path(canonical: &Path, canonical_home: &Path) -> bool {
    let deny_rel: &[&[&str]] = &[
        &[".ssh"],
        &[".aws", "credentials"],
        &[".opencomputer", "credentials"],
        &[".opencomputer", "backups"],
    ];
    for parts in deny_rel {
        let mut p = canonical_home.to_path_buf();
        for seg in *parts {
            p.push(seg);
        }
        if canonical.starts_with(&p) {
            return true;
        }
    }
    false
}

/// Sniff MIME from magic bytes first, fall back to extension, then
/// `application/octet-stream` as a last resort.
fn sniff_mime(data: &[u8], path: &Path) -> String {
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

fn sniff_mime_magic(data: &[u8]) -> Option<&'static str> {
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
    // ZIP family (also matches docx / xlsx / pptx / odt — callers can drill
    // down if needed; a generic application/zip is fine for display).
    if data.len() >= 4 && &data[..4] == b"PK\x03\x04" {
        return Some("application/zip");
    }
    // gzip
    if data.len() >= 2 && &data[..2] == b"\x1F\x8B" {
        return Some("application/gzip");
    }
    // 7-Zip
    if data.len() >= 6 && &data[..6] == b"7z\xBC\xAF\x27\x1C" {
        return Some("application/x-7z-compressed");
    }
    // RAR 5.x
    if data.len() >= 8 && &data[..7] == b"Rar!\x1A\x07\x01" {
        return Some("application/vnd.rar");
    }
    // MP4 / QuickTime (ftyp box at offset 4)
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return Some("video/mp4");
    }
    None
}

fn mime_from_extension(ext: &str) -> Option<&'static str> {
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

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{} B", n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_png_magic() {
        let data = b"\x89PNG\r\n\x1a\nrest";
        assert_eq!(
            sniff_mime(data, Path::new("x")),
            "image/png"
        );
    }

    #[test]
    fn sniff_pdf_magic() {
        let data = b"%PDF-1.4\n...";
        assert_eq!(
            sniff_mime(data, Path::new("x.bin")),
            "application/pdf"
        );
    }

    #[test]
    fn sniff_fallback_ext() {
        let data = b"plain text body";
        assert_eq!(sniff_mime(data, Path::new("/tmp/foo.txt")), "text/plain");
    }

    #[test]
    fn sniff_fallback_octet_stream() {
        let data = b"\x00\x01\x02unknown";
        assert_eq!(
            sniff_mime(data, Path::new("/tmp/x")),
            "application/octet-stream"
        );
    }

    #[test]
    fn format_bytes_basic() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2 * 1024), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}

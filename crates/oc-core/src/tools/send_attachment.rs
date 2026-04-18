//! `send_attachment` — model pushes a file (from server-local path) to the
//! user. Copies into the session attachments dir and emits a
//! `__MEDIA_ITEMS__` header carrying logical `url` + `local_path`; the
//! downstream sink / dispatcher handles transport-specific delivery.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::execution::ToolExecContext;
use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};

/// 20 MB — aligned with `project::files::MAX_PROJECT_FILE_BYTES`.
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Max length of the optional description string.
const MAX_DESCRIPTION_CHARS: usize = 200;

pub(crate) async fn tool_send_attachment(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("send_attachment requires an active session."))?;

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

    let mime_type = attachments::sniff_mime(&data, &canonical);
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

    let item = MediaItem::from_saved_path(
        Some(&session_id),
        &saved_path,
        &display_name,
        mime_type.clone(),
        size_bytes,
        kind,
        description.clone(),
    );
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
    fn format_bytes_basic() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2 * 1024), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}

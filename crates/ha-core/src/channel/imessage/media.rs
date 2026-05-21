//! iMessage outbound attachments.
//!
//! `imsg rpc` sends files through the same `send` method as text: pass
//! `file` with an optional `text` caption. It stages the file into
//! `~/Library/Messages/Attachments/imsg/` before invoking Messages.app.

#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::fs;
use uuid::Uuid;

use crate::channel::media_helpers::materialize_to_bytes;
use crate::channel::types::{MediaData, OutboundMedia};

const MAX_IMESSAGE_TEMP_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;

pub(super) struct PreparedIMessageAttachment {
    pub(super) path: String,
    pub(super) caption: Option<String>,
}

pub(super) struct PreparedIMessageAttachments {
    attachments: Vec<PreparedIMessageAttachment>,
    cleanup_paths: Vec<PathBuf>,
}

impl PreparedIMessageAttachments {
    pub(super) fn attachments(&self) -> &[PreparedIMessageAttachment] {
        &self.attachments
    }

    pub(super) async fn cleanup(self) {
        for path in self.cleanup_paths {
            if let Err(e) = fs::remove_file(&path).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    app_warn!(
                        "channel",
                        "imessage",
                        "Failed to remove outbound temp attachment {:?}: {}",
                        path,
                        e
                    );
                }
            }
        }
    }
}

pub(super) async fn prepare_imessage_attachments(
    media: &[OutboundMedia],
) -> Result<PreparedIMessageAttachments> {
    let mut attachments = Vec::with_capacity(media.len());
    let mut cleanup_paths = Vec::new();

    for item in media {
        let path = match &item.data {
            MediaData::FilePath(path) => {
                let trimmed = path.trim();
                if trimmed.is_empty() {
                    bail!("iMessage attachment path is empty");
                }
                let meta = fs::metadata(trimmed)
                    .await
                    .with_context(|| format!("Failed to stat iMessage attachment '{}'", trimmed))?;
                if !meta.is_file() {
                    bail!("iMessage attachment '{}' is not a regular file", trimmed);
                }
                trimmed.to_string()
            }
            MediaData::Url(_) | MediaData::Bytes(_) => {
                let materialized = materialize_to_bytes(
                    &item.data,
                    &item.media_type,
                    MAX_IMESSAGE_TEMP_ATTACHMENT_BYTES,
                )
                .await?;
                let path = outbound_temp_path(&materialized.filename).await?;
                fs::write(&path, materialized.bytes)
                    .await
                    .with_context(|| {
                        format!("Failed to write iMessage temp attachment {:?}", path)
                    })?;
                cleanup_paths.push(path.clone());
                path.to_string_lossy().to_string()
            }
        };

        attachments.push(PreparedIMessageAttachment {
            path,
            caption: item.caption.clone(),
        });
    }

    Ok(PreparedIMessageAttachments {
        attachments,
        cleanup_paths,
    })
}

pub(super) fn attachment_text(
    base_text: Option<&str>,
    caption: Option<&str>,
    include_base: bool,
) -> Option<String> {
    let mut parts = Vec::new();
    if include_base {
        if let Some(text) = base_text.map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(text.to_string());
        }
    }
    if let Some(caption) = caption.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(caption.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

async fn outbound_temp_path(filename: &str) -> Result<PathBuf> {
    let dir = crate::paths::channel_dir("imessage")?.join("outbound-temp");
    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("Failed to create iMessage outbound temp dir {:?}", dir))?;
    Ok(dir.join(format!(
        "{}-{}",
        Uuid::new_v4().simple(),
        sanitize_filename(filename)
    )))
}

fn sanitize_filename(filename: &str) -> String {
    let base = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment.bin");
    let sanitized: String = base
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect();
    let trimmed = sanitized.trim_matches('.');
    if trimmed.is_empty() {
        "attachment.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{attachment_text, sanitize_filename};

    #[test]
    fn attachment_text_combines_first_base_text_and_caption() {
        assert_eq!(
            attachment_text(Some("hello"), Some("cap"), true).as_deref(),
            Some("hello\n\ncap")
        );
        assert_eq!(
            attachment_text(Some("hello"), Some("cap"), false).as_deref(),
            Some("cap")
        );
        assert!(attachment_text(Some("  "), None, true).is_none());
    }

    #[test]
    fn sanitize_filename_strips_paths_and_unsafe_chars() {
        assert_eq!(sanitize_filename("../../猫.png"), "_.png");
        assert_eq!(sanitize_filename(""), "attachment.bin");
        assert_eq!(sanitize_filename("voice memo.m4a"), "voice_memo.m4a");
    }
}

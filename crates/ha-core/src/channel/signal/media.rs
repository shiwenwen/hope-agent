//! Signal outbound attachments.
//!
//! signal-cli JSON-RPC accepts the same multi-value attachment parameter as
//! the CLI: `--attachment a b` maps to `"attachments":["a","b"]`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::fs;
use uuid::Uuid;

use crate::channel::media_helpers::materialize_to_bytes;
use crate::channel::types::{MediaData, OutboundMedia};

const MAX_SIGNAL_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;

pub(super) struct PreparedSignalAttachments {
    paths: Vec<String>,
    cleanup_paths: Vec<PathBuf>,
}

impl PreparedSignalAttachments {
    pub(super) fn paths(&self) -> &[String] {
        &self.paths
    }

    pub(super) async fn cleanup(self) {
        for path in self.cleanup_paths {
            if let Err(e) = fs::remove_file(&path).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    app_warn!(
                        "channel",
                        "signal",
                        "Failed to remove outbound temp attachment {:?}: {}",
                        path,
                        e
                    );
                }
            }
        }
    }
}

pub(super) async fn prepare_signal_attachments(
    media: &[OutboundMedia],
) -> Result<PreparedSignalAttachments> {
    let mut paths = Vec::with_capacity(media.len());
    let mut cleanup_paths = Vec::new();

    for item in media {
        match &item.data {
            MediaData::FilePath(path) => {
                let trimmed = path.trim();
                if trimmed.is_empty() {
                    bail!("Signal attachment path is empty");
                }
                paths.push(trimmed.to_string());
            }
            MediaData::Url(_) | MediaData::Bytes(_) => {
                let materialized =
                    materialize_to_bytes(&item.data, &item.media_type, MAX_SIGNAL_ATTACHMENT_BYTES)
                        .await?;
                let path = outbound_temp_path(&materialized.filename).await?;
                fs::write(&path, materialized.bytes)
                    .await
                    .with_context(|| {
                        format!("Failed to write Signal temp attachment {:?}", path)
                    })?;
                paths.push(path.to_string_lossy().to_string());
                cleanup_paths.push(path);
            }
        }
    }

    Ok(PreparedSignalAttachments {
        paths,
        cleanup_paths,
    })
}

async fn outbound_temp_path(filename: &str) -> Result<PathBuf> {
    let dir = crate::paths::channel_dir("signal")?.join("outbound-temp");
    fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("Failed to create Signal outbound temp dir {:?}", dir))?;
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
    use super::sanitize_filename;

    #[test]
    fn sanitize_filename_strips_paths_and_unsafe_chars() {
        assert_eq!(sanitize_filename("../../猫.png"), "__.png");
        assert_eq!(sanitize_filename(""), "attachment.bin");
        assert_eq!(sanitize_filename("report final.pdf"), "report_final.pdf");
    }
}

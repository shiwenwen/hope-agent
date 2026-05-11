//! Signal inbound media — parse to refs (no I/O), materialize by lifting
//! the attachment file that `signal-cli` has already written to its local
//! data store.
//!
//! signal-cli 0.13+ saves every inbound attachment to
//! `<data-dir>/attachments/<attachment-id>` (no extension) as part of
//! receiving the message — it's the daemon's default behavior, not an
//! opt-in flag. Before F-082 hope-agent ignored those files and gave the
//! LLM `file_url: None`; now we copy the file to our own inbound-temp/
//! with a media-type-derived extension so the agent can read it back via
//! the usual `read_file` path.
//!
//! We **copy** rather than move because the daemon owns the lifecycle of
//! its attachment store; moving would break signal-cli's own bookkeeping.
//! F-081's move-not-copy semantics still apply downstream when the
//! `persist_channel_media_to_session` worker hand-off lifts the file out
//! of our inbound-temp/.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

/// Signal-specific parsed media ref — pre-copy form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// signal-cli attachment id — also the filename inside the daemon's
    /// `<data-dir>/attachments/` directory.
    pub attachment_id: String,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

/// Parse Signal `dataMessage.attachments` array into deferred-copy refs.
pub fn parse_message_attachments(data_message: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let arr = match data_message.get("attachments") {
        Some(serde_json::Value::Array(a)) => a,
        _ => return Vec::new(),
    };

    arr.iter()
        .filter_map(|att| {
            let attachment_id = att.get("id").and_then(|v| v.as_str())?.to_string();
            let mime_type = att
                .get("contentType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = att.get("size").and_then(|v| v.as_u64());

            let media_type = match mime_type.as_deref() {
                Some(ct) if ct.starts_with("image/") => MediaType::Photo,
                Some(ct) if ct.starts_with("video/") => MediaType::Video,
                Some(ct) if ct.starts_with("audio/ogg") => MediaType::Voice,
                Some(ct) if ct.starts_with("audio/") => MediaType::Audio,
                _ => MediaType::Document,
            };

            Some(ParsedMediaRef {
                media_type,
                attachment_id,
                mime_type,
                file_size,
            })
        })
        .collect()
}

/// Resolve the signal-cli default data directory. signal-cli follows
/// platform conventions: macOS uses `~/Library/Application Support/signal-cli`,
/// Linux honors `XDG_DATA_HOME` (falling back to `~/.local/share/signal-cli`),
/// Windows uses `%LOCALAPPDATA%\signal-cli`. We don't try to read
/// `signal-cli --config` overrides — the GUI's account config doesn't
/// surface that knob yet, and the default covers the install paths we
/// document in [`SignalDaemon::start`].
fn signal_default_data_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            std::path::PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("signal-cli"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            if !xdg.is_empty() {
                return Some(std::path::PathBuf::from(xdg).join("signal-cli"));
            }
        }
        let home = std::env::var_os("HOME")?;
        Some(
            std::path::PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("signal-cli"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(std::path::PathBuf::from(local).join("signal-cli"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// Copy a parsed Signal attachment from the daemon's local store into
/// hope-agent's inbound-temp/, returning an [`InboundMedia`] pointing at
/// our local copy. Returns `None` (with warn log) if the source file is
/// missing or the copy fails — the surrounding message still reaches the
/// agent so the round can proceed without the attachment.
pub async fn materialize_inbound(
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "signal:inbound",
                "[{}] Skipping inbound attachment_id='{}' — declared {} bytes > {} cap",
                account_id,
                parsed.attachment_id,
                declared,
                INBOUND_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
    }

    let data_dir = match signal_default_data_dir() {
        Some(d) => d,
        None => {
            app_warn!(
                "channel",
                "signal:inbound",
                "[{}] Cannot resolve signal-cli data dir on this platform",
                account_id
            );
            return None;
        }
    };
    let src = data_dir.join("attachments").join(&parsed.attachment_id);
    let on_disk_size = match tokio::fs::metadata(&src).await {
        Ok(m) => m.len(),
        Err(e) => {
            app_warn!(
                "channel",
                "signal:inbound",
                "[{}] signal-cli attachment file missing at {:?}: {} \
                 (verify signal-cli has receive-attachments enabled and \
                 data dir matches the default for this platform)",
                account_id,
                src,
                e
            );
            return None;
        }
    };
    if on_disk_size > INBOUND_DOWNLOAD_MAX_BYTES {
        app_warn!(
            "channel",
            "signal:inbound",
            "[{}] Skipping attachment_id='{}' — on-disk {} bytes > {} cap",
            account_id,
            parsed.attachment_id,
            on_disk_size,
            INBOUND_DOWNLOAD_MAX_BYTES
        );
        return None;
    }

    let ext = ext_for(None, &parsed.media_type);
    let dest = match crate::channel::inbound_media_common::inbound_temp_path(
        "signal",
        &parsed.attachment_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "signal:inbound",
                "[{}] Failed to resolve inbound path for attachment_id='{}': {}",
                account_id,
                parsed.attachment_id,
                e
            );
            return None;
        }
    };

    if let Err(e) = tokio::fs::copy(&src, &dest).await {
        app_warn!(
            "channel",
            "signal:inbound",
            "[{}] Failed to copy Signal attachment {:?} → {:?}: {}",
            account_id,
            src,
            dest,
            e
        );
        crate::channel::inbound_media_common::abort_partial_download(&dest).await;
        return None;
    }

    Some(InboundMedia {
        media_type: parsed.media_type.clone(),
        file_id: parsed.attachment_id.clone(),
        file_url: Some(dest.to_string_lossy().to_string()),
        mime_type: parsed.mime_type.clone(),
        file_size: Some(parsed.file_size.unwrap_or(on_disk_size)),
        caption: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_attachment() {
        let data_message = serde_json::json!({
            "attachments": [{
                "id": "abc123",
                "contentType": "image/jpeg",
                "size": 4096
            }]
        });
        let refs = parse_message_attachments(&data_message);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].attachment_id, "abc123");
        assert_eq!(refs[0].file_size, Some(4096));
    }

    #[test]
    fn parse_voice_audio_distinct_from_audio() {
        // audio/ogg is treated as Voice (Signal voice memo); other audio/* is Audio.
        let voice = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id": "v1", "contentType": "audio/ogg"}]
        }));
        let audio = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id": "a1", "contentType": "audio/mp3"}]
        }));
        assert_eq!(voice[0].media_type, MediaType::Voice);
        assert_eq!(audio[0].media_type, MediaType::Audio);
    }

    #[test]
    fn parse_video_and_document() {
        let v = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id": "vid1", "contentType": "video/mp4"}]
        }));
        let d = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id": "doc1", "contentType": "application/pdf"}]
        }));
        assert_eq!(v[0].media_type, MediaType::Video);
        assert_eq!(d[0].media_type, MediaType::Document);
    }

    #[test]
    fn parse_skips_attachments_without_id() {
        let refs = parse_message_attachments(&serde_json::json!({
            "attachments": [{"contentType": "image/png"}]
        }));
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_no_attachments_yields_empty() {
        let refs = parse_message_attachments(&serde_json::json!({"body": "hi"}));
        assert!(refs.is_empty());
    }
}

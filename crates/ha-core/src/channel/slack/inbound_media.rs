//! Slack inbound media — parse to deferred refs (no I/O), materialize via
//! authenticated server-side download (bot token required).
//!
//! Slack's message events carry a `files` array whose entries point at
//! `url_private` / `url_private_download` URLs hosted on `files.slack.com`.
//! Those URLs **require** an `Authorization: Bearer xoxb-…` header — a
//! plain `web_fetch` from the LLM side gets a 200 OK with HTML login page,
//! not the file. F-082 fixes this by downloading server-side: the parsed
//! ref carries the URL through `MsgContext.raw`, and
//! [`crate::channel::SlackPlugin::materialize_pending_media`] uses the
//! bot's stored token to fetch the bytes and persist a local path.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::SlackApi;

/// Slack-specific parsed media ref — pre-download form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    pub file_id: String,
    /// `url_private_download` (preferred — has `Content-Disposition: attachment`)
    /// or `url_private` fallback. Must always be a `files.slack.com` host.
    pub url: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
}

/// Parse Slack `files` array from an event into deferred-download refs.
/// Files without an `id` or without any URL field are silently dropped.
pub fn parse_message_media(event: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let files = match event.get("files").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    files
        .iter()
        .filter_map(|file| {
            let file_id = file.get("id").and_then(|v| v.as_str())?.to_string();
            let url = file
                .get("url_private_download")
                .or_else(|| file.get("url_private"))
                .and_then(|v| v.as_str())?
                .to_string();
            let mime_type = file
                .get("mimetype")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = file.get("size").and_then(|v| v.as_u64());
            let file_name = file
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let media_type = mime_type.as_deref().map_or(MediaType::Document, |mime| {
                if mime.starts_with("image/") {
                    MediaType::Photo
                } else if mime.starts_with("video/") {
                    MediaType::Video
                } else if mime.starts_with("audio/") {
                    MediaType::Audio
                } else {
                    MediaType::Document
                }
            });

            Some(ParsedMediaRef {
                media_type,
                file_id,
                url,
                mime_type,
                file_name,
                file_size,
            })
        })
        .collect()
}

/// Download a parsed Slack media ref to disk using the bot's token.
/// Returns `None` on download or persistence failure — the surrounding
/// message + text still reaches the agent so the round can proceed
/// without the attachment.
pub async fn materialize_inbound(
    api: &SlackApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "slack:inbound",
                "[{}] Skipping inbound file_id='{}' — declared {} bytes > {} cap",
                account_id,
                parsed.file_id,
                declared,
                INBOUND_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
    }

    let ext = ext_for(parsed.file_name.as_deref(), &parsed.media_type);
    let path = match crate::channel::inbound_media_common::inbound_temp_path(
        "slack",
        &parsed.file_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "slack:inbound",
                "[{}] Failed to resolve inbound path for file_id='{}': {}",
                account_id,
                parsed.file_id,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_file_to_disk(&parsed.url, &path, INBOUND_DOWNLOAD_MAX_BYTES)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "slack:inbound",
                "[{}] Failed to download Slack file_id='{}': {}",
                account_id,
                parsed.file_id,
                e
            );
            return None;
        }
    };

    Some(InboundMedia {
        media_type: parsed.media_type.clone(),
        file_id: parsed.file_id.clone(),
        file_url: Some(path.to_string_lossy().to_string()),
        mime_type: parsed.mime_type.clone(),
        file_size: Some(parsed.file_size.unwrap_or(on_disk_size)),
        caption: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_file_event() {
        let event = serde_json::json!({
            "files": [{
                "id": "F123",
                "url_private": "https://files.slack.com/files-pri/T1-F123/cat.jpg",
                "url_private_download": "https://files.slack.com/files-pri/T1-F123/download/cat.jpg",
                "mimetype": "image/jpeg",
                "name": "cat.jpg",
                "size": 1024,
            }]
        });
        let refs = parse_message_media(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].file_id, "F123");
        assert_eq!(refs[0].file_name.as_deref(), Some("cat.jpg"));
        assert_eq!(refs[0].file_size, Some(1024));
        // Prefer url_private_download over url_private.
        assert!(refs[0].url.contains("download"));
    }

    #[test]
    fn parse_video_file_event() {
        let event = serde_json::json!({
            "files": [{
                "id": "F124",
                "url_private": "https://files.slack.com/files-pri/T1-F124/clip.mp4",
                "mimetype": "video/mp4",
                "name": "clip.mp4",
            }]
        });
        let refs = parse_message_media(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Video);
    }

    #[test]
    fn parse_audio_file_event() {
        let event = serde_json::json!({
            "files": [{
                "id": "F125",
                "url_private": "https://files.slack.com/files-pri/T1-F125/voice.mp3",
                "mimetype": "audio/mp3",
            }]
        });
        let refs = parse_message_media(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Audio);
    }

    #[test]
    fn parse_document_file_event() {
        let event = serde_json::json!({
            "files": [{
                "id": "F126",
                "url_private": "https://files.slack.com/files-pri/T1-F126/report.pdf",
                "mimetype": "application/pdf",
                "name": "report.pdf",
            }]
        });
        let refs = parse_message_media(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Document);
    }

    #[test]
    fn parse_skips_files_without_id() {
        let event = serde_json::json!({
            "files": [{
                "url_private": "https://files.slack.com/files-pri/T1-X/x",
            }]
        });
        assert!(parse_message_media(&event).is_empty());
    }

    #[test]
    fn parse_skips_files_without_url() {
        let event = serde_json::json!({
            "files": [{
                "id": "F127",
                "mimetype": "image/png",
            }]
        });
        assert!(parse_message_media(&event).is_empty());
    }

    #[test]
    fn parse_no_files_yields_empty() {
        let event = serde_json::json!({"text": "hi"});
        assert!(parse_message_media(&event).is_empty());
    }
}

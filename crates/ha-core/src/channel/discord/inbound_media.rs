//! Discord inbound media — parse to refs (no I/O), materialize via
//! server-side CDN download so the local path outlives Discord's 24-hour
//! CDN URL TTL.
//!
//! Before F-082 the dispatcher saw `file_url: Some(<cdn-url>)` and passed
//! it to the LLM as-is. The URL works for the first ~24h, but any
//! reference from a later session round (or after a restart) hits HTTP
//! 403/410 because Discord re-signs CDN URLs daily. This module fetches
//! the bytes via the bot's client and keeps a local copy that's safe to
//! reference for the full session lifetime.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::DiscordApi;

/// Discord-specific parsed media ref — pre-download form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    pub file_id: String,
    pub url: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
}

/// Parse a Discord MESSAGE_CREATE event's `attachments` array into refs.
pub fn parse_message_attachments(event: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let attachments = match event.get("attachments").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };

    attachments
        .iter()
        .filter_map(|att| {
            let file_id = att.get("id").and_then(|v| v.as_str())?.to_string();
            let url = att.get("url").and_then(|v| v.as_str())?.to_string();
            let mime_type = att
                .get("content_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = att.get("size").and_then(|v| v.as_u64());
            let file_name = att
                .get("filename")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let media_type = match mime_type.as_deref() {
                Some(m) if m.starts_with("image/") => MediaType::Photo,
                Some(m) if m.starts_with("video/") => MediaType::Video,
                Some(m) if m.starts_with("audio/") => MediaType::Audio,
                _ => MediaType::Document,
            };

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

pub async fn materialize_inbound(
    api: &DiscordApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "discord:inbound",
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
        "discord",
        &parsed.file_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "discord:inbound",
                "[{}] Failed to resolve inbound path for file_id='{}': {}",
                account_id,
                parsed.file_id,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_cdn_to_disk(&parsed.url, &path, INBOUND_DOWNLOAD_MAX_BYTES)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "discord:inbound",
                "[{}] Failed to download Discord CDN file_id='{}': {}",
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
    fn parse_image_attachment() {
        let event = serde_json::json!({
            "attachments": [{
                "id": "111",
                "url": "https://cdn.discordapp.com/attachments/9/111/cat.png",
                "content_type": "image/png",
                "filename": "cat.png",
                "size": 2048
            }]
        });
        let refs = parse_message_attachments(&event);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].file_id, "111");
        assert_eq!(refs[0].file_size, Some(2048));
        assert_eq!(refs[0].file_name.as_deref(), Some("cat.png"));
    }

    #[test]
    fn parse_video_audio_document() {
        let video = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id":"v","url":"https://cdn.discordapp.com/x/v.mp4","content_type":"video/mp4"}]
        }));
        let audio = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id":"a","url":"https://cdn.discordapp.com/x/a.mp3","content_type":"audio/mpeg"}]
        }));
        let doc = parse_message_attachments(&serde_json::json!({
            "attachments": [{"id":"d","url":"https://cdn.discordapp.com/x/d.pdf","content_type":"application/pdf"}]
        }));
        assert_eq!(video[0].media_type, MediaType::Video);
        assert_eq!(audio[0].media_type, MediaType::Audio);
        assert_eq!(doc[0].media_type, MediaType::Document);
    }

    #[test]
    fn parse_skips_attachment_without_id_or_url() {
        assert!(parse_message_attachments(&serde_json::json!({
            "attachments": [{"url": "https://cdn.discordapp.com/x/y"}]
        }))
        .is_empty());
        assert!(parse_message_attachments(&serde_json::json!({
            "attachments": [{"id": "1"}]
        }))
        .is_empty());
    }

    #[test]
    fn parse_no_attachments_yields_empty() {
        assert!(parse_message_attachments(&serde_json::json!({"content": "hi"})).is_empty());
    }
}

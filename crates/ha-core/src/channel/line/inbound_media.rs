//! LINE inbound media — parse `message` event types to deferred refs and
//! materialize via the LINE Content API (`api-data.line.me`) with the
//! channel access token.
//!
//! LINE Messaging API differs from most platforms: webhook events carry
//! only the message id and type (`image` / `video` / `audio` / `file`);
//! the actual bytes have to be fetched via a separate API call to a
//! different host. Before F-082 hope-agent ignored those types entirely
//! (only `text` was forwarded as `[image]` / `[video]` placeholders).
//!
//! Reference: <https://developers.line.biz/en/reference/messaging-api/#get-content>

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::LineApi;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// LINE message id — also the path component in the Content API URL.
    pub message_id: String,
    /// File name (only present for `file` messages); image/video/audio
    /// arrive without a user-visible name.
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
}

/// Parse a LINE webhook `message` object into a deferred-download ref
/// when the message type carries binary content. Returns at most one
/// ref per LINE message — text / sticker / location events yield empty.
pub fn parse_message(message: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let msg_type = match message.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let media_type = match msg_type {
        "image" => MediaType::Photo,
        "video" => MediaType::Video,
        "audio" => MediaType::Voice,
        "file" => MediaType::Document,
        // text / sticker / location / etc. have no inbound binary.
        _ => return Vec::new(),
    };

    let message_id = match message.get("id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return Vec::new(),
    };

    let file_name = message
        .get("fileName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let file_size = message.get("fileSize").and_then(|v| v.as_u64());

    vec![ParsedMediaRef {
        media_type,
        message_id,
        file_name,
        file_size,
    }]
}

pub async fn materialize_inbound(
    api: &LineApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "line:inbound",
                "[{}] Skipping inbound message_id='{}' — declared {} bytes > {} cap",
                account_id,
                parsed.message_id,
                declared,
                INBOUND_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
    }

    let ext = ext_for(parsed.file_name.as_deref(), &parsed.media_type);
    let path = match crate::channel::inbound_media_common::inbound_temp_path(
        "line",
        &parsed.message_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "line:inbound",
                "[{}] Failed to resolve inbound path for message_id='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_message_content_to_disk(&parsed.message_id, &path, INBOUND_DOWNLOAD_MAX_BYTES)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "line:inbound",
                "[{}] Failed to fetch LINE content for message_id='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            return None;
        }
    };

    let mime_type = match parsed.media_type {
        MediaType::Photo => Some("image/jpeg".to_string()),
        MediaType::Video => Some("video/mp4".to_string()),
        MediaType::Voice => Some("audio/m4a".to_string()),
        _ => None,
    };

    Some(InboundMedia {
        media_type: parsed.media_type.clone(),
        file_id: parsed.message_id.clone(),
        file_url: Some(path.to_string_lossy().to_string()),
        mime_type,
        file_size: Some(parsed.file_size.unwrap_or(on_disk_size)),
        caption: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_message() {
        let m = serde_json::json!({"type": "image", "id": "100001"});
        let refs = parse_message(&m);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].message_id, "100001");
    }

    #[test]
    fn parse_video_message() {
        let refs = parse_message(&serde_json::json!({"type": "video", "id": "200002"}));
        assert_eq!(refs[0].media_type, MediaType::Video);
    }

    #[test]
    fn parse_audio_message_as_voice() {
        // LINE inbound audio messages are voice memos in practice.
        let refs = parse_message(&serde_json::json!({"type": "audio", "id": "300003"}));
        assert_eq!(refs[0].media_type, MediaType::Voice);
    }

    #[test]
    fn parse_file_message_preserves_name_and_size() {
        let refs = parse_message(&serde_json::json!({
            "type": "file",
            "id": "400004",
            "fileName": "doc.pdf",
            "fileSize": 8192
        }));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Document);
        assert_eq!(refs[0].file_name.as_deref(), Some("doc.pdf"));
        assert_eq!(refs[0].file_size, Some(8192));
    }

    #[test]
    fn parse_text_message_yields_empty() {
        let refs = parse_message(&serde_json::json!({"type": "text", "text": "hi"}));
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_sticker_message_yields_empty() {
        let refs = parse_message(&serde_json::json!({"type": "sticker", "id": "x"}));
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_missing_id_yields_empty() {
        let refs = parse_message(&serde_json::json!({"type": "image"}));
        assert!(refs.is_empty());
    }
}

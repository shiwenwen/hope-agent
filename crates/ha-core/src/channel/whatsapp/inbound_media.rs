//! WhatsApp inbound media — parse `BridgeMessage.attachments` to deferred
//! refs and download via the bridge-supplied URL (with optional bearer).
//!
//! hope-agent talks to WhatsApp through a user-run bridge (HTTP long-
//! polling); the bridge is responsible for resolving WhatsApp Cloud
//! API media records (`media_id → media_url + access_token`) and
//! handing them off through [`BridgeAttachment`]. This module assumes
//! that contract and stays bridge-implementation-agnostic — any
//! reachable HTTPS URL works.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::{BridgeAttachment, WhatsAppApi};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// Stable id used as the on-disk filename stem. Filename → fall back
    /// to a synthetic `"whatsapp-<idx>"` when the bridge omits it.
    pub file_id: String,
    pub url: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    /// Optional Bearer token to send with the GET request — non-empty
    /// when the bridge surfaces a WhatsApp Cloud media_url that still
    /// needs the app access token.
    pub auth_bearer: Option<String>,
}

/// Convert the bridge's attachments into deferred refs. Entries without
/// a URL are dropped silently — there's nothing to materialize.
pub fn parse_attachments(attachments: &[BridgeAttachment]) -> Vec<ParsedMediaRef> {
    attachments
        .iter()
        .enumerate()
        .filter_map(|(idx, att)| {
            let url = att.url.clone().filter(|u| !u.is_empty())?;
            let mime_type = att.content_type.clone();
            let file_name = att.filename.clone();
            let file_size = att.size;

            // Prefer MIME for classification; fall back to bridge's
            // coarse `media_type` string when MIME is missing.
            let media_type = if let Some(ref mime) = mime_type {
                if mime.starts_with("image/") {
                    MediaType::Photo
                } else if mime.starts_with("video/") {
                    MediaType::Video
                } else if mime.starts_with("audio/ogg") || mime == "audio/opus" {
                    MediaType::Voice
                } else if mime.starts_with("audio/") {
                    MediaType::Audio
                } else {
                    MediaType::Document
                }
            } else {
                match att.media_type.as_deref() {
                    Some("image") => MediaType::Photo,
                    Some("video") => MediaType::Video,
                    Some("voice") => MediaType::Voice,
                    Some("audio") => MediaType::Audio,
                    _ => MediaType::Document,
                }
            };

            let file_id = file_name
                .clone()
                .unwrap_or_else(|| format!("whatsapp-{}", idx));

            Some(ParsedMediaRef {
                media_type,
                file_id,
                url,
                mime_type,
                file_name,
                file_size,
                auth_bearer: att.auth_bearer.clone().filter(|s| !s.is_empty()),
            })
        })
        .collect()
}

pub async fn materialize_inbound(
    api: &WhatsAppApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "whatsapp:inbound",
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
        "whatsapp",
        &parsed.file_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "whatsapp:inbound",
                "[{}] Failed to resolve inbound path for file_id='{}': {}",
                account_id,
                parsed.file_id,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_attachment_to_disk(
            &parsed.url,
            parsed.auth_bearer.as_deref(),
            &path,
            INBOUND_DOWNLOAD_MAX_BYTES,
        )
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "whatsapp:inbound",
                "[{}] Failed to download WhatsApp attachment file_id='{}': {}",
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

    fn att(url: &str, mime: Option<&str>, name: Option<&str>, size: Option<u64>) -> BridgeAttachment {
        BridgeAttachment {
            url: Some(url.to_string()),
            media_type: None,
            content_type: mime.map(|s| s.to_string()),
            filename: name.map(|s| s.to_string()),
            size,
            auth_bearer: None,
        }
    }

    #[test]
    fn parse_image_attachment() {
        let refs = parse_attachments(&[att("https://wa.example/x.jpg", Some("image/jpeg"), Some("cat.jpg"), Some(1024))]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].file_id, "cat.jpg");
        assert_eq!(refs[0].file_size, Some(1024));
        assert!(refs[0].auth_bearer.is_none());
    }

    #[test]
    fn parse_voice_distinct_from_audio() {
        let v = parse_attachments(&[att("https://wa.example/v.ogg", Some("audio/ogg"), None, None)]);
        let a = parse_attachments(&[att("https://wa.example/a.mp3", Some("audio/mpeg"), None, None)]);
        assert_eq!(v[0].media_type, MediaType::Voice);
        assert_eq!(a[0].media_type, MediaType::Audio);
    }

    #[test]
    fn parse_uses_coarse_media_type_when_mime_missing() {
        let refs = parse_attachments(&[BridgeAttachment {
            url: Some("https://wa.example/x".to_string()),
            media_type: Some("image".to_string()),
            content_type: None,
            filename: None,
            size: None,
            auth_bearer: None,
        }]);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert!(refs[0].file_id.starts_with("whatsapp-"));
    }

    #[test]
    fn parse_preserves_auth_bearer() {
        let refs = parse_attachments(&[BridgeAttachment {
            url: Some("https://lookaside.fbsbx.com/x".to_string()),
            media_type: None,
            content_type: Some("image/jpeg".to_string()),
            filename: None,
            size: None,
            auth_bearer: Some("Bearer EAAxxx".to_string()),
        }]);
        assert_eq!(refs[0].auth_bearer.as_deref(), Some("Bearer EAAxxx"));
    }

    #[test]
    fn parse_drops_attachments_without_url() {
        let refs = parse_attachments(&[BridgeAttachment {
            url: None,
            media_type: Some("image".to_string()),
            content_type: None,
            filename: None,
            size: None,
            auth_bearer: None,
        }]);
        assert!(refs.is_empty());
    }
}

//! Google Chat inbound media — parse `message.attachments[]` to deferred
//! refs and materialize via the Chat `media.download` REST API with the
//! bot's OAuth access token.
//!
//! Google Chat splits attachments into two `source` flavors:
//!
//! - **`UPLOADED_CONTENT`** — the user dropped a file directly into the
//!   message. Reachable through `https://chat.googleapis.com/v1/media/{resourceName}?alt=media`
//!   with a Bearer token; bytes are streamed back. This is what we
//!   implement.
//! - **`DRIVE_FILE`** — the message references a Google Drive file. The
//!   Chat bot's OAuth scope doesn't include Drive content access, so a
//!   `media.download` call here would 403. For now we surface the file
//!   metadata + a warn log; a future PR can add a Drive-scoped flow.
//!
//! Reference: <https://developers.google.com/chat/api/reference/rest/v1/Message#Attachment>

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::GoogleChatApi;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentSource {
    /// User uploaded the file directly into the message — downloadable
    /// via Chat `media.download`.
    Uploaded,
    /// Message references a Drive file — not downloadable with the bot's
    /// Chat-scoped token. Metadata is preserved but no local copy is made.
    Drive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// Chat attachment resource name (`spaces/.../attachments/...`) — also
    /// the value passed to `download_attachment_to_disk` for UPLOADED.
    pub resource_name: String,
    pub source: AttachmentSource,
    pub content_name: Option<String>,
    pub content_type: Option<String>,
}

/// Parse a Google Chat `MESSAGE` event's `message.attachments[]` array.
/// Returns one [`ParsedMediaRef`] per attachment that has either a
/// resourceName (for UPLOADED) or driveFileId. Attachments missing both
/// identifiers are dropped silently.
pub fn parse_message_attachments(message: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let arr = match message.get("attachments").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };

    arr.iter()
        .filter_map(|att| {
            let content_name = att
                .get("contentName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let content_type = att
                .get("contentType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let source_str = att.get("source").and_then(|v| v.as_str()).unwrap_or("");

            let (source, resource_name) = match source_str {
                "DRIVE_FILE" => {
                    // Best identifier we can preserve — drive_file_id can't
                    // be downloaded with Chat scopes anyway.
                    let drive_id = att
                        .pointer("/driveDataRef/driveFileId")
                        .and_then(|v| v.as_str())?
                        .to_string();
                    (AttachmentSource::Drive, drive_id)
                }
                _ => {
                    // UPLOADED_CONTENT (or unset — defaults match the
                    // resourceName branch). Both `attachmentDataRef.resourceName`
                    // and the top-level `name` field can carry the value.
                    let rn = att
                        .pointer("/attachmentDataRef/resourceName")
                        .or_else(|| att.get("name"))
                        .and_then(|v| v.as_str())?
                        .to_string();
                    (AttachmentSource::Uploaded, rn)
                }
            };

            let media_type = match content_type.as_deref() {
                Some(m) if m.starts_with("image/") => MediaType::Photo,
                Some(m) if m.starts_with("video/") => MediaType::Video,
                Some(m) if m.starts_with("audio/") => MediaType::Audio,
                _ => MediaType::Document,
            };

            Some(ParsedMediaRef {
                media_type,
                resource_name,
                source,
                content_name,
                content_type,
            })
        })
        .collect()
}

pub async fn materialize_inbound(
    api: &GoogleChatApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if parsed.source == AttachmentSource::Drive {
        app_warn!(
            "channel",
            "googlechat:inbound",
            "[{}] Skipping DRIVE_FILE attachment resource='{}' — Drive scope not \
             configured for the Chat bot (file metadata reaches the agent but \
             content does not)",
            account_id,
            parsed.resource_name
        );
        return None;
    }

    // resource_name acts as a stable, unique stem for the temp file.
    let safe_stem = parsed
        .content_name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            // Use last segment of resource_name; falls back to whole.
            parsed
                .resource_name
                .rsplit('/')
                .next()
                .unwrap_or(&parsed.resource_name)
                .to_string()
        });

    let ext = ext_for(parsed.content_name.as_deref(), &parsed.media_type);
    let path = match crate::channel::inbound_media_common::inbound_temp_path(
        "googlechat",
        &safe_stem,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "googlechat:inbound",
                "[{}] Failed to resolve inbound path for resource='{}': {}",
                account_id,
                parsed.resource_name,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_attachment_to_disk(&parsed.resource_name, &path, INBOUND_DOWNLOAD_MAX_BYTES)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "googlechat:inbound",
                "[{}] Failed to download Google Chat attachment resource='{}': {}",
                account_id,
                parsed.resource_name,
                e
            );
            return None;
        }
    };

    Some(InboundMedia {
        media_type: parsed.media_type.clone(),
        file_id: parsed.resource_name.clone(),
        file_url: Some(path.to_string_lossy().to_string()),
        mime_type: parsed.content_type.clone(),
        file_size: Some(on_disk_size),
        caption: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uploaded_image_attachment() {
        let message = serde_json::json!({
            "attachments": [{
                "name": "spaces/AAA/messages/xxx.xxx/attachments/yyy",
                "contentName": "cat.jpg",
                "contentType": "image/jpeg",
                "source": "UPLOADED_CONTENT",
                "attachmentDataRef": {
                    "resourceName": "spaces/AAA/messages/xxx.xxx/attachments/yyy"
                }
            }]
        });
        let refs = parse_message_attachments(&message);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].source, AttachmentSource::Uploaded);
        assert_eq!(refs[0].content_name.as_deref(), Some("cat.jpg"));
        assert!(refs[0].resource_name.contains("attachments/yyy"));
    }

    #[test]
    fn parse_drive_file_attachment_preserves_id() {
        let message = serde_json::json!({
            "attachments": [{
                "name": "spaces/AAA/messages/m.m/attachments/zz",
                "contentName": "report.pdf",
                "contentType": "application/pdf",
                "source": "DRIVE_FILE",
                "driveDataRef": {"driveFileId": "1ABC"}
            }]
        });
        let refs = parse_message_attachments(&message);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].source, AttachmentSource::Drive);
        assert_eq!(refs[0].resource_name, "1ABC");
    }

    #[test]
    fn parse_uploaded_without_dataref_falls_back_to_name() {
        // Older payloads may omit attachmentDataRef.resourceName; the
        // top-level `name` field carries the same path in that case.
        let message = serde_json::json!({
            "attachments": [{
                "name": "spaces/AAA/messages/x.x/attachments/yyy",
                "contentName": "doc.pdf",
                "contentType": "application/pdf",
                "source": "UPLOADED_CONTENT"
            }]
        });
        let refs = parse_message_attachments(&message);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].resource_name, "spaces/AAA/messages/x.x/attachments/yyy");
    }

    #[test]
    fn parse_skips_uploaded_without_any_identifier() {
        let message = serde_json::json!({
            "attachments": [{
                "contentType": "image/png",
                "source": "UPLOADED_CONTENT"
            }]
        });
        assert!(parse_message_attachments(&message).is_empty());
    }

    #[test]
    fn parse_no_attachments_yields_empty() {
        assert!(parse_message_attachments(&serde_json::json!({"text": "hi"})).is_empty());
    }
}

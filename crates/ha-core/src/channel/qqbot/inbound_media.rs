//! QQ Bot inbound media — parse `attachments[]` to deferred refs and
//! download from Tencent CDN.
//!
//! QQ Bot gateway events (`C2C_MESSAGE_CREATE` / `GROUP_AT_MESSAGE_CREATE` /
//! `AT_MESSAGE_CREATE` / `DIRECT_MESSAGE_CREATE`) all carry attachments
//! in the same shape:
//!
//! ```json
//! "attachments": [{
//!   "content_type": "image/jpeg",
//!   "filename": "x.jpg",
//!   "height": 1024, "width": 768,
//!   "size": 102400,
//!   "url": "https://gchat.qpic.cn/..."
//! }]
//! ```
//!
//! The `url` field is a short-lived signed Tencent CDN URL — no
//! `Authorization` header needed, but the signature lives in the URL
//! query and the host varies (`gchat.qpic.cn` / `qzonestyle.gtimg.cn` /
//! `multimedia.nt.qq.com.cn` / etc.). Host pinning is enforced inside
//! [`QqBotApi::download_cdn_to_disk`].

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::QqBotApi;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// Stable id within hope-agent's bookkeeping. QQ doesn't expose a
    /// dedicated `file_id`, so we use the `filename` (if present) and
    /// fall back to a derived hash of the URL + index.
    pub file_id: String,
    pub url: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
}

/// Parse a QQ Bot MESSAGE_CREATE event's `attachments[]` array.
pub fn parse_message_attachments(d: &serde_json::Value) -> Vec<ParsedMediaRef> {
    let arr = match d.get("attachments").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };

    arr.iter()
        .enumerate()
        .filter_map(|(idx, att)| {
            let url = att.get("url").and_then(|v| v.as_str())?.to_string();
            let mime_type = att
                .get("content_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_name = att
                .get("filename")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let file_size = att.get("size").and_then(|v| v.as_u64());

            let media_type = match mime_type.as_deref() {
                Some(m) if m.starts_with("image/") => MediaType::Photo,
                Some(m) if m.starts_with("video/") => MediaType::Video,
                Some(m) if m.starts_with("audio/") => MediaType::Audio,
                _ => MediaType::Document,
            };

            // file_id heuristic: filename → fall back to "qq-<idx>-<short-host>".
            let file_id = file_name.clone().unwrap_or_else(|| {
                let host = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "cdn".to_string());
                format!("qq-{}-{}", idx, host)
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

pub async fn materialize_inbound(
    api: &QqBotApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "qqbot:inbound",
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
        "qqbot",
        &parsed.file_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "qqbot:inbound",
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
                "qqbot:inbound",
                "[{}] Failed to download QQ CDN file_id='{}': {}",
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
        let d = serde_json::json!({
            "attachments": [{
                "url": "https://gchat.qpic.cn/x/y.jpg",
                "content_type": "image/jpeg",
                "filename": "y.jpg",
                "size": 1024
            }]
        });
        let refs = parse_message_attachments(&d);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].file_id, "y.jpg");
        assert_eq!(refs[0].file_size, Some(1024));
    }

    #[test]
    fn parse_video_audio_document() {
        let video = parse_message_attachments(&serde_json::json!({
            "attachments": [{"url":"https://multimedia.nt.qq.com.cn/v.mp4","content_type":"video/mp4"}]
        }));
        let audio = parse_message_attachments(&serde_json::json!({
            "attachments": [{"url":"https://multimedia.nt.qq.com.cn/a.amr","content_type":"audio/amr"}]
        }));
        let doc = parse_message_attachments(&serde_json::json!({
            "attachments": [{"url":"https://multimedia.nt.qq.com.cn/d.pdf","content_type":"application/pdf"}]
        }));
        assert_eq!(video[0].media_type, MediaType::Video);
        assert_eq!(audio[0].media_type, MediaType::Audio);
        assert_eq!(doc[0].media_type, MediaType::Document);
    }

    #[test]
    fn parse_falls_back_to_synthetic_file_id() {
        let refs = parse_message_attachments(&serde_json::json!({
            "attachments": [{"url":"https://gchat.qpic.cn/anon"}]
        }));
        assert_eq!(refs.len(), 1);
        assert!(refs[0].file_id.starts_with("qq-0-"));
    }

    #[test]
    fn parse_skips_attachment_without_url() {
        assert!(parse_message_attachments(&serde_json::json!({
            "attachments": [{"content_type":"image/png"}]
        }))
        .is_empty());
    }

    #[test]
    fn parse_no_attachments_yields_empty() {
        assert!(parse_message_attachments(&serde_json::json!({"content": "hi"})).is_empty());
    }
}

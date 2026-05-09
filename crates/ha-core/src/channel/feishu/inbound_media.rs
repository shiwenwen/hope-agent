//! Parse and materialize inbound media references from Feishu message events.
//!
//! Feishu's `im.message.receive_v1` event carries a `message.content` JSON
//! string whose shape varies by `message_type`:
//!
//! ```text
//! image    → {"image_key": "img_v2_xxx"}
//! file     → {"file_key": "...", "file_name": ..., "file_size": "<bytes>"}
//! audio    → {"file_key": "...", "duration": <ms>}
//! media    → {"file_key": "...", "image_key": <cover>, "file_name": ..., "duration": <ms>}
//! sticker  → {"file_key": "..."}
//! ```
//!
//! Reference: <https://open.feishu.cn/document/uAjLw4CM/ukTMukTMukTM/reference/im-v1/message/events/receive>
//!
//! Parsing is sync (no I/O); materialization streams bytes chunk-by-chunk
//! through `FeishuApi::download_resource_to_file` straight onto disk under
//! `~/.hope-agent/channels/feishu/inbound-temp/` (no in-memory buffer), and
//! produces an [`InboundMedia`] whose `file_url` is the local path.
//! Failures are logged and yield `None` — the surrounding text + raw event
//! still reach the dispatcher so the agent can fall back gracefully.

use serde::{Deserialize, Serialize};

use crate::channel::types::{InboundMedia, MediaType, MsgContext};

use super::api::FeishuApi;

/// Sanity tripwire on `download_resource_to_file` — generous enough to
/// cover Feishu's documented platform limits with headroom (image ≤ 30 MB,
/// file ≤ 100 MB, video ≤ 200 MB) so legitimate user uploads always fit,
/// strict enough to catch a misconfigured-proxy or attack scenario where
/// the gateway returns a multi-GB body. RAM is not a factor: downloads
/// stream chunk-by-chunk straight to disk, so the cap is really about
/// disk-fill / latency containment for anomalous responses, not memory.
pub const INBOUND_DOWNLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// JSON key used to smuggle deferred-download media refs through
/// `MsgContext.raw` from the WS event handler to the dispatcher. Picked to
/// be obviously app-internal so a future Feishu schema change can't collide.
pub const PENDING_MEDIA_KEY: &str = "_hopePendingMedia";

/// Result of parsing a message's content JSON for media references — pre-download form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    /// `image_key` for image, `file_key` otherwise.
    pub key: String,
    pub resource_type: ResourceType,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
}

/// Maps to the `?type=` query parameter on the resource download endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Image,
    File,
}

impl ResourceType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ImageContent {
    image_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileContent {
    file_key: Option<String>,
    file_name: Option<String>,
    /// Feishu encodes file_size as a string in the content JSON.
    file_size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AudioContent {
    file_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MediaContent {
    file_key: Option<String>,
    file_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StickerContent {
    file_key: Option<String>,
}

/// Parse a Feishu message's `msg_type` + `content` JSON string into media
/// references. Unsupported types (`text`, `post`, `share_chat`, etc.) and
/// malformed content both yield an empty vec; malformed cases log a warning.
pub fn parse_message_media(msg_type: &str, content: &str, account_id: &str) -> Vec<ParsedMediaRef> {
    match msg_type {
        "image" => match serde_json::from_str::<ImageContent>(content) {
            Ok(ImageContent {
                image_key: Some(key),
            }) => vec![ParsedMediaRef {
                media_type: MediaType::Photo,
                key,
                resource_type: ResourceType::Image,
                mime_type: Some("image/jpeg".to_string()),
                file_name: None,
                file_size: None,
            }],
            Ok(_) => Vec::new(),
            Err(e) => {
                warn_parse_failed(account_id, msg_type, &e, content);
                Vec::new()
            }
        },
        "file" => match serde_json::from_str::<FileContent>(content) {
            Ok(FileContent {
                file_key: Some(key),
                file_name,
                file_size,
            }) => vec![ParsedMediaRef {
                media_type: MediaType::Document,
                key,
                resource_type: ResourceType::File,
                mime_type: None,
                file_name,
                file_size: file_size.as_deref().and_then(|s| s.parse::<u64>().ok()),
            }],
            Ok(_) => Vec::new(),
            Err(e) => {
                warn_parse_failed(account_id, msg_type, &e, content);
                Vec::new()
            }
        },
        // Feishu's `audio` msg_type is a recorded voice memo (analogous to
        // Telegram Voice, not Telegram Audio).
        "audio" => match serde_json::from_str::<AudioContent>(content) {
            Ok(AudioContent {
                file_key: Some(key),
            }) => vec![ParsedMediaRef {
                media_type: MediaType::Voice,
                key,
                resource_type: ResourceType::File,
                mime_type: Some("audio/ogg".to_string()),
                file_name: None,
                file_size: None,
            }],
            Ok(_) => Vec::new(),
            Err(e) => {
                warn_parse_failed(account_id, msg_type, &e, content);
                Vec::new()
            }
        },
        "media" => match serde_json::from_str::<MediaContent>(content) {
            Ok(MediaContent {
                file_key: Some(key),
                file_name,
            }) => vec![ParsedMediaRef {
                media_type: MediaType::Video,
                key,
                resource_type: ResourceType::File,
                mime_type: None,
                file_name,
                file_size: None,
            }],
            Ok(_) => Vec::new(),
            Err(e) => {
                warn_parse_failed(account_id, msg_type, &e, content);
                Vec::new()
            }
        },
        "sticker" => match serde_json::from_str::<StickerContent>(content) {
            Ok(StickerContent {
                file_key: Some(key),
            }) => vec![ParsedMediaRef {
                media_type: MediaType::Sticker,
                key,
                resource_type: ResourceType::File,
                mime_type: Some("image/png".to_string()),
                file_name: None,
                file_size: None,
            }],
            Ok(_) => Vec::new(),
            Err(e) => {
                warn_parse_failed(account_id, msg_type, &e, content);
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

fn warn_parse_failed(account_id: &str, msg_type: &str, err: &serde_json::Error, content: &str) {
    app_warn!(
        "channel",
        "feishu:inbound",
        "[{}] Failed to parse {} content: {} (raw={})",
        account_id,
        msg_type,
        err,
        crate::truncate_utf8(content, 256)
    );
}

/// Embed deferred-download refs into an event payload that becomes
/// `MsgContext.raw`. The dispatcher pulls them back out via
/// [`take_pending_refs`] only after access + mention gating passes,
/// keeping the WS handler's per-event hot path I/O-free so the gateway
/// ack lands within its expected window.
pub fn embed_pending_refs(raw: &mut serde_json::Value, refs: Vec<ParsedMediaRef>) {
    if refs.is_empty() {
        return;
    }
    if !raw.is_object() {
        *raw = serde_json::json!({});
    }
    if let serde_json::Value::Object(map) = raw {
        if let Ok(value) = serde_json::to_value(refs) {
            map.insert(PENDING_MEDIA_KEY.to_string(), value);
        }
    }
}

/// Take and remove deferred-download refs from `msg.raw`. Returns an empty
/// vec when no refs were embedded or the payload is malformed (we
/// silently drop bad payloads rather than fail the message — the
/// surrounding text still reaches the agent).
pub fn take_pending_refs(msg: &mut MsgContext) -> Vec<ParsedMediaRef> {
    let serde_json::Value::Object(ref mut map) = msg.raw else {
        return Vec::new();
    };
    let Some(value) = map.remove(PENDING_MEDIA_KEY) else {
        return Vec::new();
    };
    serde_json::from_value(value).unwrap_or_default()
}

/// Download a parsed media ref straight to local disk (no in-memory buffer)
/// and return an [`InboundMedia`] pointing at the on-disk path. Returns
/// `None` (with warn log) on download or persistence failure — the caller
/// should keep the surrounding message reaching the dispatcher even if
/// media materialization fails. Aligns with the GUI attachment model:
/// non-image files only ever flow as `file_path` on the LLM side
/// ([`channel/worker/media.rs::convert_inbound_media_to_attachments`])
/// so streaming chunk-by-chunk to disk avoids ever holding the full body
/// in memory — a 1 GB inbound video occupies ~16 KB of buffer at a time.
pub async fn materialize_inbound(
    api: &FeishuApi,
    message_id: &str,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "feishu:inbound",
                "[{}] Skipping inbound resource key='{}' — declared size {} bytes exceeds {} cap",
                account_id,
                parsed.key,
                declared,
                INBOUND_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
    }

    let dir = match crate::paths::channel_dir("feishu") {
        Ok(d) => d.join("inbound-temp"),
        Err(e) => {
            app_warn!(
                "channel",
                "feishu:inbound",
                "[{}] Failed to resolve feishu inbound dir: {}",
                account_id,
                e
            );
            return None;
        }
    };
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        app_warn!(
            "channel",
            "feishu:inbound",
            "[{}] Failed to create inbound dir {:?}: {}",
            account_id,
            dir,
            e
        );
        return None;
    }

    let safe_key = parsed.key.replace(['/', '\\', ':'], "_");
    let ext = ext_for(parsed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!("{}-{}.{}", ts, safe_key, ext));

    let on_disk_size = match api
        .download_resource_to_file(
            message_id,
            &parsed.key,
            parsed.resource_type.as_str(),
            &path,
        )
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "feishu:inbound",
                "[{}] Failed to download media key='{}': {}",
                account_id,
                parsed.key,
                e
            );
            return None;
        }
    };

    Some(InboundMedia {
        media_type: parsed.media_type.clone(),
        file_id: parsed.key.clone(),
        file_url: Some(path.to_string_lossy().to_string()),
        mime_type: parsed.mime_type.clone(),
        file_size: Some(parsed.file_size.unwrap_or(on_disk_size)),
        caption: None,
    })
}

/// Pick a file extension for the on-disk filename. Trusts the original
/// `file_name` extension only if it is short and alphanumeric — otherwise
/// falls back to a media-type-specific default to keep paths well-formed.
fn ext_for(parsed: &ParsedMediaRef) -> String {
    if let Some(name) = parsed.file_name.as_deref() {
        if let Some(ext) = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
        {
            if !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return ext.to_ascii_lowercase();
            }
        }
    }
    match parsed.media_type {
        MediaType::Photo | MediaType::Sticker => "jpg",
        MediaType::Video => "mp4",
        MediaType::Audio | MediaType::Voice => "opus",
        MediaType::Animation => "gif",
        MediaType::Document => "bin",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_with_key() {
        let refs = parse_message_media("image", r#"{"image_key":"img_v2_abc"}"#, "test");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Photo);
        assert_eq!(refs[0].key, "img_v2_abc");
        assert_eq!(refs[0].resource_type, ResourceType::Image);
        assert_eq!(refs[0].mime_type.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn parse_file_with_metadata() {
        let refs = parse_message_media(
            "file",
            r#"{"file_key":"file_v2_xyz","file_name":"report.pdf","file_size":"2048"}"#,
            "test",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Document);
        assert_eq!(refs[0].key, "file_v2_xyz");
        assert_eq!(refs[0].resource_type, ResourceType::File);
        assert_eq!(refs[0].file_name.as_deref(), Some("report.pdf"));
        assert_eq!(refs[0].file_size, Some(2048));
    }

    #[test]
    fn parse_audio_as_voice() {
        let refs = parse_message_media(
            "audio",
            r#"{"file_key":"audio_v2_a","duration":3500}"#,
            "test",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Voice);
        assert_eq!(refs[0].key, "audio_v2_a");
        assert_eq!(refs[0].resource_type, ResourceType::File);
    }

    #[test]
    fn parse_media_as_video() {
        let refs = parse_message_media(
            "media",
            r#"{"file_key":"media_v2_v","image_key":"cover_x","file_name":"clip.mov","duration":12000}"#,
            "test",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Video);
        assert_eq!(refs[0].key, "media_v2_v");
        assert_eq!(refs[0].file_name.as_deref(), Some("clip.mov"));
    }

    #[test]
    fn parse_sticker() {
        let refs = parse_message_media("sticker", r#"{"file_key":"sticker_v2_s"}"#, "test");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].media_type, MediaType::Sticker);
        assert_eq!(refs[0].key, "sticker_v2_s");
    }

    #[test]
    fn parse_text_yields_empty() {
        let refs = parse_message_media("text", r#"{"text":"hi"}"#, "test");
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_unknown_type_yields_empty() {
        let refs = parse_message_media("share_chat", r#"{"chat_id":"oc_xxx"}"#, "test");
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_malformed_json_yields_empty() {
        let refs = parse_message_media("image", "not-json", "test");
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_image_missing_key_yields_empty() {
        let refs = parse_message_media("image", r#"{}"#, "test");
        assert!(refs.is_empty());
    }

    #[test]
    fn parse_file_missing_size_still_works() {
        let refs = parse_message_media("file", r#"{"file_key":"f","file_name":"x.txt"}"#, "test");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].file_size, None);
    }

    #[test]
    fn ext_for_uses_filename_extension_when_safe() {
        let parsed = ParsedMediaRef {
            media_type: MediaType::Document,
            key: "k".into(),
            resource_type: ResourceType::File,
            mime_type: None,
            file_name: Some("report.PDF".into()),
            file_size: None,
        };
        assert_eq!(ext_for(&parsed), "pdf");
    }

    #[test]
    fn ext_for_falls_back_when_filename_extension_unsafe() {
        let parsed = ParsedMediaRef {
            media_type: MediaType::Photo,
            key: "k".into(),
            resource_type: ResourceType::Image,
            mime_type: None,
            file_name: Some("evil.../etc/passwd".into()),
            file_size: None,
        };
        assert_eq!(ext_for(&parsed), "jpg");
    }

    #[test]
    fn ext_for_falls_back_when_no_filename() {
        let parsed = ParsedMediaRef {
            media_type: MediaType::Voice,
            key: "k".into(),
            resource_type: ResourceType::File,
            mime_type: None,
            file_name: None,
            file_size: None,
        };
        assert_eq!(ext_for(&parsed), "opus");
    }

    fn sample_msg() -> MsgContext {
        MsgContext {
            channel_id: crate::channel::types::ChannelId::Feishu,
            account_id: "acc".into(),
            sender_id: "u1".into(),
            sender_name: None,
            sender_username: None,
            chat_id: "c1".into(),
            chat_type: crate::channel::types::ChatType::Dm,
            chat_title: None,
            thread_id: None,
            message_id: "m1".into(),
            text: None,
            media: Vec::new(),
            reply_to_message_id: None,
            timestamp: chrono::Utc::now(),
            was_mentioned: true,
            raw: serde_json::json!({"existing": 1}),
        }
    }

    #[test]
    fn embed_pending_refs_round_trips_through_msgcontext() {
        let parsed = vec![ParsedMediaRef {
            media_type: MediaType::Photo,
            key: "img_v2_abc".into(),
            resource_type: ResourceType::Image,
            mime_type: Some("image/jpeg".into()),
            file_name: None,
            file_size: None,
        }];
        let mut msg = sample_msg();
        embed_pending_refs(&mut msg.raw, parsed.clone());
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_some());

        let took = take_pending_refs(&mut msg);
        assert_eq!(took, parsed);
        // After take, the key is gone — second call yields empty.
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_none());
        assert!(take_pending_refs(&mut msg).is_empty());
        // Surrounding fields preserved.
        assert_eq!(msg.raw.get("existing"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn embed_pending_refs_skips_when_empty() {
        let mut msg = sample_msg();
        embed_pending_refs(&mut msg.raw, Vec::new());
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_none());
    }

    #[test]
    fn take_pending_refs_yields_empty_for_non_object_raw() {
        let mut msg = sample_msg();
        msg.raw = serde_json::Value::Null;
        assert!(take_pending_refs(&mut msg).is_empty());
    }

    #[test]
    fn take_pending_refs_yields_empty_when_payload_malformed() {
        let mut msg = sample_msg();
        if let serde_json::Value::Object(ref mut map) = msg.raw {
            map.insert(PENDING_MEDIA_KEY.into(), serde_json::json!("not-an-array"));
        }
        assert!(take_pending_refs(&mut msg).is_empty());
        // Even on parse failure we strip the key so the dispatcher doesn't
        // re-attempt on a follow-up call.
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_none());
    }
}

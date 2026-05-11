//! WeChat inbound media — parse to deferred refs, materialize via the
//! existing AES-decrypt + save pipeline (with a size cap up front).
//!
//! Pre-F-082 the polling loop in [`super::polling::run_wechat_polling`]
//! inline-called [`super::media::download_inbound_media`] for every non-
//! text item, fetching the full ciphertext into a `Vec<u8>`, AES-128-ECB
//! decrypting it in memory, and writing the plaintext to disk. A 100 MB
//! group file therefore burned ≥100 MB RSS during decrypt (peak ~2× for
//! the cipher+plain buffers) while `getUpdates` was blocked.
//!
//! This module switches WeChat to the same deferred pattern the other 10
//! channels already use:
//!
//! 1. `parse_message_items` runs synchronously inside the polling loop
//!    and produces one [`ParsedMediaRef`] per non-text item.
//! 2. The refs ride through `MsgContext.raw` to the dispatcher (no I/O,
//!    no AES, no buffering on the polling task).
//! 3. After gating passes, [`WeChatPlugin::materialize_pending_media`]
//!    calls [`materialize_inbound`] which still uses the legacy in-mem
//!    AES path but now rejects oversize attachments up front via the
//!    `declared_size` metadata. Commit 14 swaps the in-mem path for a
//!    disk-buffered two-stage decrypt to plug the RSS leak entirely.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::INBOUND_DOWNLOAD_MAX_BYTES;
use crate::channel::types::InboundMedia;
use crate::channel::wechat::api::{
    MessageItem, MESSAGE_ITEM_TYPE_FILE, MESSAGE_ITEM_TYPE_IMAGE, MESSAGE_ITEM_TYPE_TEXT,
    MESSAGE_ITEM_TYPE_VIDEO, MESSAGE_ITEM_TYPE_VOICE,
};

/// WeChat parsed media ref — embeds the full `MessageItem` because the
/// AES key, encrypted query param, file metadata, and item-type
/// discriminator all live on its sub-structs (`image_item.aeskey`,
/// `image_item.media.encrypt_query_param`, `file_item.file_name`, …).
/// Re-using the struct keeps a single source of truth and lets the
/// downstream materializer share code with the outbound upload path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub message_id: String,
    pub item: MessageItem,
}

/// Pick out non-text items as deferred-download refs. Text items
/// continue to feed `extract_body` for the `text` field — they don't
/// enter the materialize pipeline.
pub fn parse_message_items(items: &[MessageItem], message_id: &str) -> Vec<ParsedMediaRef> {
    items
        .iter()
        .filter(|item| item.item_type != MESSAGE_ITEM_TYPE_TEXT)
        .filter(|item| {
            matches!(
                item.item_type,
                MESSAGE_ITEM_TYPE_IMAGE
                    | MESSAGE_ITEM_TYPE_FILE
                    | MESSAGE_ITEM_TYPE_VIDEO
                    | MESSAGE_ITEM_TYPE_VOICE
            )
        })
        .cloned()
        .map(|item| ParsedMediaRef {
            message_id: message_id.to_string(),
            item,
        })
        .collect()
}

/// Best-effort declared size: WeChat exposes ciphertext size on each
/// item-type's metadata under different field names. `None` when the
/// upstream didn't include it (we still cap via Content-Length / stream
/// inside `download_plain_media`).
pub fn declared_size(item: &MessageItem) -> Option<u64> {
    match item.item_type {
        MESSAGE_ITEM_TYPE_IMAGE => item.image_item.as_ref().and_then(|i| i.mid_size),
        MESSAGE_ITEM_TYPE_VIDEO => item.video_item.as_ref().and_then(|v| v.video_size),
        MESSAGE_ITEM_TYPE_FILE => item
            .file_item
            .as_ref()
            .and_then(|f| f.len.as_ref())
            .and_then(|s| s.parse::<u64>().ok()),
        // Voice items lack a declared size in WeChat's schema.
        _ => None,
    }
}

/// Materialize a parsed ref. Currently delegates to the legacy in-mem
/// AES path in [`super::media::download_inbound_media`]; commit 14
/// will replace that with a disk-buffered two-stage decrypt. Returns
/// `None` (with warn log) on declared-size cap rejection or any
/// download / decrypt failure — the surrounding message still reaches
/// the agent so the round can proceed without the attachment.
pub async fn materialize_inbound(
    parsed: &ParsedMediaRef,
    cdn_base_url: &str,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = declared_size(&parsed.item) {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Skipping inbound msg='{}' item_type={} — declared {} bytes > {} cap",
                account_id,
                parsed.message_id,
                parsed.item.item_type,
                declared,
                INBOUND_DOWNLOAD_MAX_BYTES
            );
            return None;
        }
    }

    match super::media::download_inbound_media(&parsed.message_id, &parsed.item, cdn_base_url).await
    {
        Ok(Some(media)) => Some(media),
        Ok(None) => None,
        Err(e) => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Failed to download/decrypt msg='{}' item_type={}: {}",
                account_id,
                parsed.message_id,
                parsed.item.item_type,
                e
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::wechat::api::{CdnMedia, FileItem, ImageItem, VideoItem};

    fn image_item(mid_size: Option<u64>) -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_IMAGE,
            image_item: Some(ImageItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: Some("dummy".into()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                aeskey: None,
                mid_size,
            }),
            ..Default::default()
        }
    }

    fn file_item(len: Option<&str>) -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_FILE,
            file_item: Some(FileItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: Some("dummy".into()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                file_name: Some("report.pdf".into()),
                len: len.map(|s| s.to_string()),
            }),
            ..Default::default()
        }
    }

    fn video_item(video_size: Option<u64>) -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_VIDEO,
            video_item: Some(VideoItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: Some("dummy".into()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                video_size,
            }),
            ..Default::default()
        }
    }

    fn text_item() -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_TEXT,
            ..Default::default()
        }
    }

    #[test]
    fn parse_skips_text_items() {
        let items = vec![text_item(), image_item(Some(1024))];
        let refs = parse_message_items(&items, "m1");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].item.item_type, MESSAGE_ITEM_TYPE_IMAGE);
        assert_eq!(refs[0].message_id, "m1");
    }

    #[test]
    fn parse_picks_up_all_supported_types() {
        let items = vec![
            text_item(),
            image_item(None),
            file_item(None),
            video_item(None),
            MessageItem {
                item_type: MESSAGE_ITEM_TYPE_VOICE,
                ..Default::default()
            },
        ];
        let refs = parse_message_items(&items, "m");
        assert_eq!(refs.len(), 4);
    }

    #[test]
    fn declared_size_uses_per_type_field() {
        assert_eq!(declared_size(&image_item(Some(1024))), Some(1024));
        assert_eq!(declared_size(&video_item(Some(2048))), Some(2048));
        assert_eq!(declared_size(&file_item(Some("4096"))), Some(4096));
    }

    #[test]
    fn declared_size_none_for_missing_or_bad_metadata() {
        assert_eq!(declared_size(&image_item(None)), None);
        assert_eq!(declared_size(&file_item(Some("not-a-number"))), None);
        assert_eq!(
            declared_size(&MessageItem {
                item_type: MESSAGE_ITEM_TYPE_VOICE,
                ..Default::default()
            }),
            None
        );
    }
}

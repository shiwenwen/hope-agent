//! Discord 出站附件物化：把 `OutboundMedia` 翻译成 [`MaterializedMedia`]，由
//! [`super::api::DiscordApi::create_message_with_attachments`] 拼 `files[N]`
//! 部分发走 `POST /channels/{id}/messages`。

use anyhow::Result;

use crate::channel::media_helpers::{materialize_to_bytes, MaterializedMedia};
use crate::channel::types::OutboundMedia;

/// Discord 单附件硬上限 25 MiB（部分服务器 boost 后更高，这里按最严的免费档处理）。
/// 超出由 `materialize_to_bytes` 在流式下载阶段直接 bail，dispatcher 走链接兜底。
pub const MAX_DISCORD_FILE_BYTES: usize = 25 * 1024 * 1024;

pub async fn build_discord_files(media: &[OutboundMedia]) -> Result<Vec<MaterializedMedia>> {
    let mut out = Vec::with_capacity(media.len());
    for m in media {
        out.push(materialize_to_bytes(&m.data, &m.media_type, MAX_DISCORD_FILE_BYTES).await?);
    }
    Ok(out)
}

/// 把 `payload.text` 与每个媒体的 caption 拼成 Discord `content`，避免拆条。
/// 全部为空时返回 `None`，Discord 允许 content 缺失只发附件。
pub fn merge_captions(text: Option<&str>, media: &[OutboundMedia]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = text.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(t.to_string());
    }
    for m in media {
        if let Some(cap) = m.caption.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(cap.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::{MediaData, MediaType};

    fn media_with_caption(caption: Option<&str>) -> OutboundMedia {
        OutboundMedia {
            media_type: MediaType::Photo,
            data: MediaData::Bytes(vec![0u8; 8]),
            caption: caption.map(str::to_string),
        }
    }

    #[test]
    fn merge_captions_combines_text_and_captions() {
        let media = vec![
            media_with_caption(Some("cap A")),
            media_with_caption(Some("cap B")),
        ];
        let merged = merge_captions(Some("hello"), &media).unwrap();
        assert_eq!(merged, "hello\n\ncap A\n\ncap B");
    }

    #[test]
    fn merge_captions_none_when_all_empty() {
        let media = vec![media_with_caption(None), media_with_caption(Some("   "))];
        assert!(merge_captions(None, &media).is_none());
        assert!(merge_captions(Some(""), &media).is_none());
    }

    #[tokio::test]
    async fn build_discord_files_rejects_oversize_bytes() {
        let over = OutboundMedia {
            media_type: MediaType::Document,
            data: MediaData::Bytes(vec![0u8; MAX_DISCORD_FILE_BYTES + 1]),
            caption: None,
        };
        let err = build_discord_files(&[over]).await.unwrap_err();
        assert!(format!("{err}").contains("exceeds"));
    }

    #[tokio::test]
    async fn build_discord_files_accepts_under_limit() {
        let ok = OutboundMedia {
            media_type: MediaType::Photo,
            data: MediaData::Bytes(vec![0u8; 32]),
            caption: None,
        };
        let parts = build_discord_files(&[ok]).await.expect("under limit");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].bytes.len(), 32);
        assert!(parts[0].filename.ends_with(".jpg"), "filename={}", parts[0].filename);
        assert_eq!(parts[0].mime, "image/jpeg");
    }
}

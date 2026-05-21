//! Slack outbound attachments via external file upload v2.

use anyhow::Result;

use crate::channel::media_helpers::{materialize_to_bytes, MaterializedMedia};
use crate::channel::types::OutboundMedia;

/// Keep memory use bounded while Slack/team-side policy remains the final
/// authority for larger files.
pub const MAX_SLACK_FILE_BYTES: usize = 100 * 1024 * 1024;

pub async fn build_slack_files(media: &[OutboundMedia]) -> Result<Vec<MaterializedMedia>> {
    let mut out = Vec::with_capacity(media.len());
    for item in media {
        out.push(materialize_to_bytes(&item.data, &item.media_type, MAX_SLACK_FILE_BYTES).await?);
    }
    Ok(out)
}

pub fn merge_initial_comment(text: Option<&str>, media: &[OutboundMedia]) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(text) = text.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(text.to_string());
    }
    for item in media {
        if let Some(caption) = item
            .caption
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            parts.push(caption.to_string());
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
    use crate::channel::types::{MediaData, MediaType, OutboundMedia};

    fn media_with_caption(caption: Option<&str>) -> OutboundMedia {
        OutboundMedia {
            media_type: MediaType::Photo,
            data: MediaData::Bytes(vec![0u8; 8]),
            caption: caption.map(str::to_string),
        }
    }

    #[test]
    fn merge_initial_comment_combines_text_and_captions() {
        let media = vec![
            media_with_caption(Some("cap A")),
            media_with_caption(Some("cap B")),
        ];
        let merged = merge_initial_comment(Some("hello"), &media).unwrap();
        assert_eq!(merged, "hello\n\ncap A\n\ncap B");
    }

    #[test]
    fn merge_initial_comment_none_when_all_empty() {
        let media = vec![media_with_caption(None), media_with_caption(Some("   "))];
        assert!(merge_initial_comment(None, &media).is_none());
        assert!(merge_initial_comment(Some(""), &media).is_none());
    }

    #[tokio::test]
    async fn build_slack_files_rejects_oversize_bytes() {
        let over = OutboundMedia {
            media_type: MediaType::Document,
            data: MediaData::Bytes(vec![0u8; MAX_SLACK_FILE_BYTES + 1]),
            caption: None,
        };
        let err = build_slack_files(&[over]).await.unwrap_err();
        assert!(format!("{err}").contains("exceeds"));
    }
}

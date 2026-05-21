//! WhatsApp outbound media for the bridge contract.
//!
//! The bridge may run on a different machine, so local file paths are not
//! portable. We send a self-contained data URL in the `media` field; the
//! bridge can upload it to WhatsApp Cloud API (or an equivalent backend)
//! and then send the media message.

use anyhow::Result;
use base64::Engine as _;

use crate::channel::media_helpers::{materialize_to_bytes, MaterializedMedia};
use crate::channel::types::{MediaType, OutboundMedia};

pub(super) struct PreparedWhatsAppMedia {
    pub(super) media_type: &'static str,
    pub(super) media: String,
    pub(super) filename: String,
    pub(super) mime_type: String,
    pub(super) caption: Option<String>,
}

pub(super) async fn prepare_whatsapp_media(
    media: &[OutboundMedia],
) -> Result<Vec<PreparedWhatsAppMedia>> {
    let mut out = Vec::with_capacity(media.len());
    for item in media {
        let materialized = materialize_to_bytes(
            &item.data,
            &item.media_type,
            max_bytes_for(&item.media_type),
        )
        .await?;
        out.push(PreparedWhatsAppMedia {
            media_type: whatsapp_media_type(&item.media_type),
            media: to_data_url(&materialized),
            filename: materialized.filename,
            mime_type: materialized.mime,
            caption: item.caption.clone(),
        });
    }
    Ok(out)
}

pub(super) fn caption_text(
    base_text: Option<&str>,
    caption: Option<&str>,
    include_base: bool,
) -> Option<String> {
    let mut parts = Vec::new();
    if include_base {
        if let Some(text) = base_text.map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(text.to_string());
        }
    }
    if let Some(caption) = caption.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(caption.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn to_data_url(media: &MaterializedMedia) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(&media.bytes);
    format!(
        "data:{};name={};base64,{}",
        media.mime,
        urlencoding::encode(&media.filename),
        encoded
    )
}

fn whatsapp_media_type(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Photo => "image",
        MediaType::Video | MediaType::Animation => "video",
        MediaType::Audio | MediaType::Voice => "audio",
        MediaType::Document => "document",
        MediaType::Sticker => "sticker",
    }
}

fn max_bytes_for(media_type: &MediaType) -> usize {
    match media_type {
        MediaType::Photo => 5 * 1024 * 1024,
        MediaType::Video | MediaType::Audio | MediaType::Voice | MediaType::Animation => {
            16 * 1024 * 1024
        }
        MediaType::Document => 100 * 1024 * 1024,
        MediaType::Sticker => 500 * 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_media_types_to_whatsapp_wire_types() {
        assert_eq!(whatsapp_media_type(&MediaType::Photo), "image");
        assert_eq!(whatsapp_media_type(&MediaType::Video), "video");
        assert_eq!(whatsapp_media_type(&MediaType::Animation), "video");
        assert_eq!(whatsapp_media_type(&MediaType::Voice), "audio");
        assert_eq!(whatsapp_media_type(&MediaType::Document), "document");
        assert_eq!(whatsapp_media_type(&MediaType::Sticker), "sticker");
    }

    #[test]
    fn builds_data_url_with_mime_and_name() {
        let media = MaterializedMedia {
            bytes: b"hi".to_vec(),
            filename: "hello world.txt".to_string(),
            mime: "text/plain".to_string(),
        };
        assert_eq!(
            to_data_url(&media),
            "data:text/plain;name=hello%20world.txt;base64,aGk="
        );
    }

    #[test]
    fn caption_text_combines_first_base_text_and_caption() {
        assert_eq!(
            caption_text(Some("hello"), Some("cap"), true).as_deref(),
            Some("hello\n\ncap")
        );
        assert_eq!(
            caption_text(Some("hello"), Some("cap"), false).as_deref(),
            Some("cap")
        );
        assert!(caption_text(Some("  "), None, true).is_none());
    }
}

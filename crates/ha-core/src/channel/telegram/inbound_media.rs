//! Telegram inbound media — parse to refs, materialize via authenticated
//! download (cap + cleanup).
//!
//! Pre-F-082 Telegram downloaded every inbound photo / document inline
//! inside the polling loop ([`polling::download_inbound_media_to_temp`]),
//! blocking `getUpdates` until the bytes hit disk, with no size cap and
//! no cleanup. Group messages mentioning *other* users still got their
//! attachments pulled down — wasted bandwidth + disk. This module moves
//! Telegram to the same deferred pattern the other 9 channels already
//! use: parse stays sync, materialization runs only after dispatcher
//! gating, and bytes stream through [`stream_to_disk`] (cap-aware,
//! failure-cleaning) rather than teloxide's raw downloader.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{ext_for, INBOUND_DOWNLOAD_MAX_BYTES};
use crate::channel::types::{InboundMedia, MediaType};

use super::api::TelegramBotApi;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub media_type: MediaType,
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<u64>,
    pub caption: Option<String>,
}

/// Parse a teloxide message into deferred-download refs. Currently
/// covers photo + document (matching pre-F-082 behaviour). Other binary
/// types (video / audio / voice / sticker / animation) are not yet
/// emitted to the dispatcher — adding them is a follow-up to F-082
/// scope, not a regression introduced here.
pub fn parse_message_media(msg: &teloxide::types::Message) -> Vec<ParsedMediaRef> {
    let mut refs = Vec::new();

    if let Some(photos) = msg.photo() {
        if let Some(best) = photos.iter().max_by_key(|p| p.width * p.height) {
            refs.push(ParsedMediaRef {
                media_type: MediaType::Photo,
                file_id: best.file.id.to_string(),
                mime_type: Some("image/jpeg".to_string()),
                file_name: None,
                file_size: Some(best.file.size as u64),
                caption: msg.caption().map(|c| c.to_string()),
            });
        }
    }

    if let Some(doc) = msg.document() {
        refs.push(ParsedMediaRef {
            media_type: MediaType::Document,
            file_id: doc.file.id.to_string(),
            mime_type: doc.mime_type.as_ref().map(|m| m.to_string()),
            file_name: doc.file_name.clone(),
            file_size: Some(doc.file.size as u64),
            caption: msg.caption().map(|c| c.to_string()),
        });
    }

    refs
}

/// Resolve a parsed ref through `getFile` and stream bytes to disk.
/// Returns `None` (with warn log) on download / persistence failure so
/// the surrounding message still reaches the agent.
pub async fn materialize_inbound(
    api: &TelegramBotApi,
    parsed: &ParsedMediaRef,
    account_id: &str,
) -> Option<InboundMedia> {
    if let Some(declared) = parsed.file_size {
        if declared > INBOUND_DOWNLOAD_MAX_BYTES {
            app_warn!(
                "channel",
                "telegram:inbound",
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
    let dest = match crate::channel::inbound_media_common::inbound_temp_path(
        "telegram",
        &parsed.file_id,
        &ext,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "telegram:inbound",
                "[{}] Failed to resolve inbound path for file_id='{}': {}",
                account_id,
                parsed.file_id,
                e
            );
            return None;
        }
    };

    let on_disk_size = match api
        .download_file_to_disk(&parsed.file_id, &dest, INBOUND_DOWNLOAD_MAX_BYTES)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "telegram:inbound",
                "[{}] Failed to download Telegram file_id='{}': {}",
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
        file_url: Some(dest.to_string_lossy().to_string()),
        mime_type: parsed.mime_type.clone(),
        file_size: Some(parsed.file_size.unwrap_or(on_disk_size)),
        caption: parsed.caption.clone(),
    })
}

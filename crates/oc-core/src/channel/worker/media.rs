use crate::channel::types::*;

/// Convert channel inbound media items to agent Attachment structs
/// so the LLM can see images/files sent by users.
pub(super) fn convert_inbound_media_to_attachments(
    media: &[InboundMedia],
    session_id: &str,
) -> Vec<crate::agent::Attachment> {
    let mut attachments = Vec::new();
    let session_att_dir = crate::paths::attachments_dir(session_id).ok();
    if let Some(ref dir) = session_att_dir {
        if let Err(err) = std::fs::create_dir_all(dir) {
            app_warn!(
                "channel",
                "worker",
                "Failed to create session attachment dir '{}': {}",
                dir.to_string_lossy(),
                err
            );
        }
    }
    for m in media {
        let Some(ref file_url) = m.file_url else {
            continue;
        };
        let persisted_path = persist_channel_media_to_session(session_att_dir.as_deref(), m, file_url);
        let effective_path = persisted_path.as_deref().unwrap_or(file_url);
        let mime = m
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let is_image = mime.starts_with("image/");

        if is_image {
            // Images: read file data and encode as base64 for multimodal LLM input
            match std::fs::read(effective_path) {
                Ok(data) => {
                    use base64::Engine as _;
                    attachments.push(crate::agent::Attachment {
                        name: m.file_id.clone(),
                        mime_type: mime,
                        data: Some(base64::engine::general_purpose::STANDARD.encode(&data)),
                        file_path: None,
                    });
                }
                Err(err) => {
                    app_warn!(
                        "channel",
                        "worker",
                        "Failed to read inbound image '{}': {}",
                        effective_path,
                        err
                    );
                }
            }
        } else {
            // Non-image files: pass file_path, let file_extract handle text extraction
            attachments.push(crate::agent::Attachment {
                name: m.file_id.clone(),
                mime_type: mime,
                data: None,
                file_path: Some(effective_path.to_string()),
            });
        }
    }
    attachments
}

fn persist_channel_media_to_session(
    session_dir: Option<&std::path::Path>,
    media: &InboundMedia,
    source_path: &str,
) -> Option<String> {
    let dir = session_dir?;
    let src = std::path::Path::new(source_path);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .trim_start_matches('.');
    let safe_id = media.file_id.replace(['/', '\\', ':'], "_");
    let media_kind = match media.media_type {
        MediaType::Photo => "photo",
        MediaType::Video => "video",
        MediaType::Audio => "audio",
        MediaType::Document => "document",
        MediaType::Sticker => "sticker",
        MediaType::Voice => "voice",
        MediaType::Animation => "animation",
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("{}-channel-{}-{}.{}", ts, media_kind, safe_id, ext);
    let dest = dir.join(filename);
    if src == dest {
        return Some(dest.to_string_lossy().to_string());
    }
    match std::fs::copy(src, &dest) {
        Ok(_) => Some(dest.to_string_lossy().to_string()),
        Err(err) => {
            app_warn!(
                "channel",
                "worker",
                "Failed to persist inbound media '{}' to session dir: {}",
                source_path,
                err
            );
            None
        }
    }
}

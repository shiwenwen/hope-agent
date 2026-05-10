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
        let persisted_path =
            persist_channel_media_to_session(session_att_dir.as_deref(), m, file_url);
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

/// Replace every byte not in `[A-Za-z0-9_-]` with `_` to produce a safe
/// filename fragment from a free-form channel file_id. This is strictly
/// filename sanitization — the full path safety check is `canonicalize()`
/// of the source file (see below).
fn sanitize_file_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        crate::truncate_utf8(&out, 64).to_string()
    }
}

fn persist_channel_media_to_session(
    session_dir: Option<&std::path::Path>,
    media: &InboundMedia,
    source_path: &str,
) -> Option<String> {
    let dir = session_dir?;
    let src = std::path::Path::new(source_path);

    // Verify the source lives under the shared channels runtime root before
    // copying. This defeats both "../../etc/passwd"-style traversal and
    // symlink swaps that would otherwise copy arbitrary host files into the
    // session attachments folder. We allow anything under
    // `~/.hope-agent/channels/<id>/...` so Telegram / WeChat / etc. share
    // the same rule.
    let channels_root = match crate::paths::channels_dir() {
        Ok(root) => root,
        Err(err) => {
            app_warn!("channel", "worker", "Cannot resolve channels root: {}", err);
            return None;
        }
    };
    let canonical_src = match src.canonicalize() {
        Ok(p) => p,
        Err(err) => {
            app_warn!(
                "channel",
                "worker",
                "Failed to canonicalize inbound media '{}': {}",
                source_path,
                err
            );
            return None;
        }
    };
    let canonical_root = channels_root.canonicalize().unwrap_or(channels_root);
    if !canonical_src.starts_with(&canonical_root) {
        app_warn!(
            "channel",
            "worker",
            "Refusing to copy inbound media '{}' outside {}",
            canonical_src.display(),
            canonical_root.display()
        );
        return None;
    }

    let ext = canonical_src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .trim_start_matches('.');
    let safe_id = sanitize_file_id(&media.file_id);
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
    if canonical_src == dest {
        return Some(dest.to_string_lossy().to_string());
    }
    // Move (rename) the inbound-temp file into the session attachments dir so
    // the source doesn't accumulate forever (per F-081). Cross-fs renames fall
    // back to copy + remove. canonicalize() above + the channels_root prefix
    // check still gates path traversal — only files genuinely living under
    // ~/.hope-agent/channels/<id>/ are eligible.
    match std::fs::rename(&canonical_src, &dest) {
        Ok(()) => Some(dest.to_string_lossy().to_string()),
        Err(err) if is_cross_device_rename(&err) => {
            app_debug!(
                "channel",
                "worker",
                "Cross-fs rename for inbound media '{}' — falling back to copy + remove",
                source_path
            );
            match std::fs::copy(&canonical_src, &dest) {
                Ok(_) => {
                    if let Err(rm_err) = std::fs::remove_file(&canonical_src) {
                        app_warn!(
                            "channel",
                            "worker",
                            "Copied inbound media '{}' but failed to remove source: {}",
                            source_path,
                            rm_err
                        );
                    }
                    Some(dest.to_string_lossy().to_string())
                }
                Err(copy_err) => {
                    app_warn!(
                        "channel",
                        "worker",
                        "Failed to persist inbound media '{}' to session dir (cross-fs copy): {}",
                        source_path,
                        copy_err
                    );
                    None
                }
            }
        }
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

/// `std::fs::rename` returns `ErrorKind::CrossesDevices` on Rust 1.85+ when
/// source and destination live on different filesystems. Older toolchains
/// surface this as raw `errno = EXDEV`; check both for portability.
fn is_cross_device_rename(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    if err.kind() == ErrorKind::CrossesDevices {
        return true;
    }
    #[cfg(unix)]
    {
        const EXDEV: i32 = 18;
        if err.raw_os_error() == Some(EXDEV) {
            return true;
        }
    }
    #[cfg(windows)]
    {
        // ERROR_NOT_SAME_DEVICE
        if err.raw_os_error() == Some(17) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// F-081: persist must move (not copy) inbound-temp files into the
    /// session attachments dir, so the source doesn't accumulate forever.
    /// Uses `HA_DATA_DIR` to redirect channels_dir into a tempdir so the
    /// safety check (`canonical_src starts_with channels_root`) passes.
    #[test]
    fn persist_moves_source_into_session_dir() {
        let data_root = tempfile::tempdir().unwrap();
        // Redirect ~/.hope-agent root into the tempdir for the duration of
        // this test. `HA_DATA_DIR` is read by paths::root_dir().
        std::env::set_var("HA_DATA_DIR", data_root.path());

        let inbound_dir = crate::paths::channels_dir()
            .unwrap()
            .join("feishu")
            .join("inbound-temp");
        std::fs::create_dir_all(&inbound_dir).unwrap();
        let src = inbound_dir.join("inbound-fixture.bin");
        {
            let mut f = std::fs::File::create(&src).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let session_dir = data_root.path().join("session-attachments");
        std::fs::create_dir_all(&session_dir).unwrap();
        let src_string = src.to_string_lossy().to_string();
        let media = InboundMedia {
            media_type: MediaType::Document,
            file_id: "fixture".to_string(),
            file_url: Some(src_string.clone()),
            mime_type: Some("application/octet-stream".to_string()),
            file_size: None,
            caption: None,
        };
        let dest_path =
            persist_channel_media_to_session(Some(&session_dir), &media, &src_string)
                .expect("persist should succeed");

        let dest = std::path::PathBuf::from(&dest_path);
        assert!(dest.exists(), "destination file should exist after move");
        assert!(
            !src.exists(),
            "source file should be gone after move (F-081)"
        );
        let content = std::fs::read(&dest).unwrap();
        assert_eq!(content, b"hello world");

        std::env::remove_var("HA_DATA_DIR");
    }
}

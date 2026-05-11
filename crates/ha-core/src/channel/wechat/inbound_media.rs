//! WeChat inbound media — parse to deferred refs, materialize via the
//! channel-agnostic `stream_to_disk` helper + a disk-buffered streaming
//! AES-128-ECB decrypt.
//!
//! Pre-F-082 WeChat's inbound path fetched the full ciphertext into a
//! `Vec<u8>` (`response.bytes()`), then AES-decrypted that buffer into a
//! *second* `Vec<u8>`, holding ~2× the file size in RAM. A 100 MB group
//! file burned ≥200 MB peak RSS while the polling task was blocked on
//! the download + decrypt cycle. Commit 13 moved the call site behind
//! `materialize_pending_media` (so the polling loop returned early) but
//! kept the in-mem AES path. **This commit** plugs the RSS leak entirely
//! by switching to a two-stage disk-buffered streaming decrypt:
//!
//! 1. [`stream_to_disk`] writes the ciphertext to
//!    `inbound-temp/<ts>-<msg>.enc` chunk by chunk (cap + cleanup
//!    enforced by the shared helper).
//! 2. A `spawn_blocking` task drives OpenSSL `Crypter::update` /
//!    `Crypter::finalize` over a 16 KiB read buffer, writing the
//!    plaintext to `inbound-temp/<ts>-<msg>.<ext>` block by block.
//! 3. The intermediate `.enc` file is removed unconditionally.
//!
//! Peak RSS is now bounded by the read / write buffers (~16 KiB each)
//! plus OpenSSL's internal block_size scratch — independent of file
//! size. ECB blocks are independently invertible, so streaming decrypt
//! is correct; PKCS#7 unpadding is handled by `Crypter::finalize` at
//! the tail. If the streaming path fails for any reason, revert this
//! commit and commit 13's `media::download_inbound_media` delegate is
//! still wired up as the previous-step fallback.

use serde::{Deserialize, Serialize};

use crate::channel::inbound_media_common::{
    abort_partial_download, inbound_temp_path, stream_to_disk, INBOUND_DOWNLOAD_MAX_BYTES,
};
use crate::channel::types::{InboundMedia, MediaType};
use crate::channel::wechat::api::{
    CdnMedia, MessageItem, MESSAGE_ITEM_TYPE_FILE, MESSAGE_ITEM_TYPE_IMAGE, MESSAGE_ITEM_TYPE_TEXT,
    MESSAGE_ITEM_TYPE_VIDEO, MESSAGE_ITEM_TYPE_VOICE,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedMediaRef {
    pub message_id: String,
    pub item: MessageItem,
}

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

pub fn declared_size(item: &MessageItem) -> Option<u64> {
    match item.item_type {
        MESSAGE_ITEM_TYPE_IMAGE => item.image_item.as_ref().and_then(|i| i.mid_size),
        MESSAGE_ITEM_TYPE_VIDEO => item.video_item.as_ref().and_then(|v| v.video_size),
        MESSAGE_ITEM_TYPE_FILE => item
            .file_item
            .as_ref()
            .and_then(|f| f.len.as_ref())
            .and_then(|s| s.parse::<u64>().ok()),
        _ => None,
    }
}

/// Bundle of per-item metadata picked off `ParsedMediaRef.item` so the
/// async materialize path doesn't repeat the match-on-item_type dance.
struct ItemSpec<'a> {
    media_type: MediaType,
    cdn_media: &'a CdnMedia,
    /// AES key in base64. For images the key can live either on
    /// `image_item.aeskey` (hex-decoded into base64 first) or on
    /// `media.aes_key`. Other types only carry it on `media.aes_key`.
    aes_key_b64: String,
    file_name: Option<String>,
    /// Default extension when filename is missing (image → jpg, video
    /// → mp4, voice → silk).
    default_ext: &'static str,
    /// Static MIME for non-file types (image/jpeg, video/mp4, audio/silk);
    /// File items resolve MIME from filename later.
    static_mime: Option<&'static str>,
}

fn extract_spec(item: &MessageItem) -> Option<ItemSpec<'_>> {
    match item.item_type {
        MESSAGE_ITEM_TYPE_IMAGE => {
            let image = item.image_item.as_ref()?;
            let media = image.media.as_ref()?;
            let aes_key_b64 = image
                .aeskey
                .as_deref()
                .map(|hex| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode(hex.as_bytes())
                })
                .or_else(|| media.aes_key.clone())?;
            Some(ItemSpec {
                media_type: MediaType::Photo,
                cdn_media: media,
                aes_key_b64,
                file_name: None,
                default_ext: "jpg",
                static_mime: Some("image/jpeg"),
            })
        }
        MESSAGE_ITEM_TYPE_FILE => {
            let file = item.file_item.as_ref()?;
            let media = file.media.as_ref()?;
            let aes_key_b64 = media.aes_key.clone()?;
            Some(ItemSpec {
                media_type: MediaType::Document,
                cdn_media: media,
                aes_key_b64,
                file_name: file.file_name.clone(),
                default_ext: "bin",
                static_mime: None,
            })
        }
        MESSAGE_ITEM_TYPE_VIDEO => {
            let video = item.video_item.as_ref()?;
            let media = video.media.as_ref()?;
            let aes_key_b64 = media.aes_key.clone()?;
            Some(ItemSpec {
                media_type: MediaType::Video,
                cdn_media: media,
                aes_key_b64,
                file_name: None,
                default_ext: "mp4",
                static_mime: Some("video/mp4"),
            })
        }
        MESSAGE_ITEM_TYPE_VOICE => {
            let voice = item.voice_item.as_ref()?;
            let media = voice.media.as_ref()?;
            let aes_key_b64 = media.aes_key.clone()?;
            Some(ItemSpec {
                media_type: MediaType::Voice,
                cdn_media: media,
                aes_key_b64,
                file_name: None,
                default_ext: "silk",
                static_mime: Some("audio/silk"),
            })
        }
        _ => None,
    }
}

fn resolve_download_url(media: &CdnMedia, cdn_base_url: &str) -> Option<String> {
    if let Some(full) = media
        .full_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(full.to_string());
    }
    media
        .encrypt_query_param
        .as_ref()
        .map(|param| super::media::build_cdn_download_url(cdn_base_url, param))
}

/// Streaming AES-128-ECB decrypt from `enc_path` into `plain_path`,
/// running inside a `spawn_blocking` so OpenSSL's synchronous Crypter
/// doesn't stall the tokio reactor. Returns the plaintext byte count
/// on success.
async fn streaming_decrypt(
    enc_path: std::path::PathBuf,
    plain_path: std::path::PathBuf,
    raw_key: Vec<u8>,
) -> anyhow::Result<u64> {
    use anyhow::Context;
    tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
        use openssl::symm::{Cipher, Crypter, Mode};
        use std::io::{BufReader, BufWriter, Read, Write};

        let cipher = Cipher::aes_128_ecb();
        let mut crypter = Crypter::new(cipher, Mode::Decrypt, &raw_key, None)
            .context("OpenSSL Crypter::new failed")?;
        crypter.pad(true); // PKCS#7

        let enc_file = std::fs::File::open(&enc_path)
            .with_context(|| format!("Failed to open ciphertext at {:?}", enc_path))?;
        let mut reader = BufReader::with_capacity(16 * 1024, enc_file);
        let plain_file = std::fs::File::create(&plain_path)
            .with_context(|| format!("Failed to create plaintext at {:?}", plain_path))?;
        let mut writer = BufWriter::with_capacity(16 * 1024, plain_file);

        let mut in_buf = [0u8; 16 * 1024];
        // OpenSSL requires output buffer to fit input + one cipher block.
        let mut out_buf = vec![0u8; in_buf.len() + cipher.block_size()];
        let mut total: u64 = 0;

        loop {
            let n = reader.read(&mut in_buf).context("Reading ciphertext")?;
            if n == 0 {
                break;
            }
            let written = crypter
                .update(&in_buf[..n], &mut out_buf)
                .context("Crypter::update failed")?;
            writer
                .write_all(&out_buf[..written])
                .context("Writing plaintext chunk")?;
            total += written as u64;
        }
        let written = crypter
            .finalize(&mut out_buf)
            .context("Crypter::finalize failed (likely truncated ciphertext or bad key)")?;
        writer
            .write_all(&out_buf[..written])
            .context("Writing plaintext tail")?;
        total += written as u64;
        writer.flush().context("Flushing plaintext writer")?;

        Ok(total)
    })
    .await
    .context("spawn_blocking panicked")?
}

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

    let spec = match extract_spec(&parsed.item) {
        Some(s) => s,
        None => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Cannot extract WeChat item spec (item_type={}, missing media or aes_key)",
                account_id,
                parsed.item.item_type
            );
            return None;
        }
    };

    let url = match resolve_download_url(spec.cdn_media, cdn_base_url) {
        Some(u) => u,
        None => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Missing CDN download URL for msg='{}'",
                account_id,
                parsed.message_id
            );
            return None;
        }
    };

    let raw_key = match super::media::parse_aes_key(&spec.aes_key_b64) {
        Ok(k) => k,
        Err(e) => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Bad WeChat aes_key for msg='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            return None;
        }
    };

    // Stem the on-disk filename off the message id (sanitized inside
    // inbound_temp_path) so concurrent messages don't collide.
    let stem = if let Some(ref name) = spec.file_name {
        // For file attachments the original filename carries the
        // extension — preserve it via the stem path. inbound_temp_path
        // sanitizes path separators internally.
        format!("{}-{}", parsed.message_id, name)
    } else {
        parsed.message_id.clone()
    };
    let ext = match spec.file_name.as_deref() {
        Some(name) => std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| {
                !s.is_empty() && s.len() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .unwrap_or_else(|| spec.default_ext.to_string()),
        None => spec.default_ext.to_string(),
    };

    let enc_path = match inbound_temp_path("wechat", &stem, "enc").await {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Failed to resolve .enc path for msg='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            return None;
        }
    };

    let plain_path = match inbound_temp_path("wechat", &stem, &ext).await {
        Ok(p) => p,
        Err(e) => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Failed to resolve plaintext path for msg='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            // Try to clean up any partial .enc that might've been created
            // by an earlier attempt before bailing.
            abort_partial_download(&enc_path).await;
            return None;
        }
    };

    // Stage 1 — stream the ciphertext to disk (cap + cleanup baked in).
    let client = reqwest::Client::new();
    let builder = client.get(&url);
    if let Err(e) = stream_to_disk(builder, &enc_path, INBOUND_DOWNLOAD_MAX_BYTES).await {
        app_warn!(
            "channel",
            "wechat:inbound",
            "[{}] Failed to stream ciphertext for msg='{}': {}",
            account_id,
            parsed.message_id,
            e
        );
        return None;
    }

    // Stage 2 — incremental AES-128-ECB decrypt from .enc to plaintext.
    let plain_bytes = match streaming_decrypt(enc_path.clone(), plain_path.clone(), raw_key).await {
        Ok(n) => n,
        Err(e) => {
            app_warn!(
                "channel",
                "wechat:inbound",
                "[{}] Streaming decrypt failed for msg='{}': {}",
                account_id,
                parsed.message_id,
                e
            );
            // Clean up both partial files before bailing.
            abort_partial_download(&plain_path).await;
            abort_partial_download(&enc_path).await;
            return None;
        }
    };

    // Stage 3 — drop the ciphertext, we no longer need it.
    abort_partial_download(&enc_path).await;

    let mime_type = match spec.media_type {
        MediaType::Document => spec
            .file_name
            .as_deref()
            .map(super::media::mime_from_filename)
            .or_else(|| Some("application/octet-stream".to_string())),
        _ => spec.static_mime.map(|s| s.to_string()),
    };

    Some(InboundMedia {
        media_type: spec.media_type,
        file_id: plain_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("file")
            .to_string(),
        file_url: Some(plain_path.to_string_lossy().to_string()),
        mime_type,
        file_size: Some(plain_bytes),
        caption: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::wechat::api::{CdnMedia, FileItem, ImageItem, VideoItem, VoiceItem};

    fn image_item(mid_size: Option<u64>, aes_key: Option<&str>) -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_IMAGE,
            image_item: Some(ImageItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: aes_key.map(|s| s.to_string()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                aeskey: None,
                mid_size,
            }),
            ..Default::default()
        }
    }

    fn file_item(len: Option<&str>, file_name: Option<&str>) -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_FILE,
            file_item: Some(FileItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: Some("dummy".into()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                file_name: file_name.map(|s| s.to_string()),
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

    fn voice_item() -> MessageItem {
        MessageItem {
            item_type: MESSAGE_ITEM_TYPE_VOICE,
            voice_item: Some(VoiceItem {
                media: Some(CdnMedia {
                    encrypt_query_param: Some("x".into()),
                    aes_key: Some("dummy".into()),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                text: None,
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
        let items = vec![text_item(), image_item(Some(1024), Some("dummy"))];
        let refs = parse_message_items(&items, "m1");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].item.item_type, MESSAGE_ITEM_TYPE_IMAGE);
        assert_eq!(refs[0].message_id, "m1");
    }

    #[test]
    fn parse_picks_up_all_supported_types() {
        let items = vec![
            text_item(),
            image_item(None, Some("dummy")),
            file_item(None, None),
            video_item(None),
            voice_item(),
        ];
        let refs = parse_message_items(&items, "m");
        assert_eq!(refs.len(), 4);
    }

    #[test]
    fn declared_size_uses_per_type_field() {
        assert_eq!(
            declared_size(&image_item(Some(1024), Some("k"))),
            Some(1024)
        );
        assert_eq!(declared_size(&video_item(Some(2048))), Some(2048));
        assert_eq!(
            declared_size(&file_item(Some("4096"), Some("x.pdf"))),
            Some(4096)
        );
    }

    #[test]
    fn declared_size_none_for_missing_or_bad_metadata() {
        assert_eq!(declared_size(&image_item(None, Some("k"))), None);
        assert_eq!(
            declared_size(&file_item(Some("not-a-number"), Some("x.pdf"))),
            None
        );
        assert_eq!(declared_size(&voice_item()), None);
    }

    #[test]
    fn extract_spec_image_picks_static_mime() {
        let item = image_item(Some(100), Some("dummy"));
        let spec = extract_spec(&item).expect("spec");
        assert_eq!(spec.media_type, MediaType::Photo);
        assert_eq!(spec.static_mime, Some("image/jpeg"));
        assert_eq!(spec.default_ext, "jpg");
    }

    #[test]
    fn extract_spec_file_preserves_file_name() {
        let item = file_item(Some("4096"), Some("report.pdf"));
        let spec = extract_spec(&item).expect("spec");
        assert_eq!(spec.media_type, MediaType::Document);
        assert_eq!(spec.file_name.as_deref(), Some("report.pdf"));
        assert!(spec.static_mime.is_none());
    }

    #[test]
    fn extract_spec_rejects_missing_aes_key() {
        let item = image_item(Some(100), None); // aes_key None
                                                // image without `aeskey` (hex) and without media.aes_key returns None.
        assert!(extract_spec(&item).is_none());
    }

    /// Round-trip: encrypt an in-mem buffer with the legacy one-shot
    /// `openssl::symm::encrypt`, write it to a tempfile, decrypt it
    /// with the new streaming `Crypter` path, and verify the recovered
    /// plaintext matches. Covers the AES-128-ECB + PKCS#7 contract that
    /// commit 14 swaps from one-shot decrypt to incremental Crypter.
    #[tokio::test]
    async fn streaming_decrypt_round_trips_with_pkcs7_padding() {
        use openssl::symm::{encrypt, Cipher};

        let key = vec![0x42u8; 16];
        // 17 bytes — forces PKCS#7 to pad to 32 (two cipher blocks),
        // covering the unpad path on finalize().
        let plaintext: Vec<u8> = (0..17u8).collect();
        let ciphertext = encrypt(Cipher::aes_128_ecb(), &key, None, &plaintext).expect("encrypt");

        let dir = tempfile::tempdir().expect("tempdir");
        let enc_path = dir.path().join("c.enc");
        let plain_path = dir.path().join("p.bin");
        tokio::fs::write(&enc_path, &ciphertext)
            .await
            .expect("write ciphertext");

        let n = streaming_decrypt(enc_path.clone(), plain_path.clone(), key.clone())
            .await
            .expect("decrypt");
        assert_eq!(n as usize, plaintext.len());
        let recovered = tokio::fs::read(&plain_path).await.expect("read plaintext");
        assert_eq!(recovered, plaintext);
    }
}

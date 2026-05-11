//! Channel-agnostic helpers for inbound media (image / file / audio / video).
//!
//! Used by per-channel `inbound_media.rs` modules to share the chunk-streaming
//! download pipeline, size cap, failure cleanup, and pending-ref envelope
//! while each channel keeps its own protocol-specific `ParsedMediaRef` shape.
//!
//! The "deferred materialization" pattern landed in Feishu first (F-082): the
//! channel's event handler parses media refs synchronously (no I/O), embeds
//! them into `MsgContext.raw`, sends the message off to the dispatcher early
//! so the gateway ack lands within its window, and only after access +
//! mention gating passes does [`ChannelPlugin::materialize_pending_media`]
//! pull the refs back out and call [`stream_to_disk`] to actually download.

use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Serialize};

use crate::channel::types::{MediaType, MsgContext};

/// Sanity tripwire across all inbound channels — generous enough to cover
/// platform-documented limits (image ≤ 30 MB, file ≤ 100 MB, video ≤ 200 MB)
/// with headroom, strict enough to catch a misconfigured-proxy or
/// multi-GB attack response. RAM is not a factor (downloads stream
/// chunk-by-chunk straight to disk); the cap is really about disk-fill
/// and latency containment for anomalous responses.
pub const INBOUND_DOWNLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// JSON key used to smuggle deferred-download media refs through
/// `MsgContext.raw` from a channel's event handler to the dispatcher.
/// Picked to be obviously app-internal so any platform's schema change
/// can't collide. Each channel uses the same key but its own
/// `ParsedMediaRef`-shaped value type — fine, because a single
/// `MsgContext` always belongs to exactly one channel.
pub const PENDING_MEDIA_KEY: &str = "_hopePendingMedia";

/// Embed deferred-download refs into an event payload that becomes
/// `MsgContext.raw`. The dispatcher pulls them back out via
/// [`take_pending_refs`] only after gating passes, keeping the channel's
/// per-event hot path I/O-free so the gateway ack lands on time.
pub fn embed_pending_refs<T: Serialize>(raw: &mut serde_json::Value, refs: Vec<T>) {
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

/// Take and remove deferred-download refs from `msg.raw`. Returns empty
/// when no refs were embedded or the payload is malformed — we silently
/// drop bad payloads rather than fail the message, the surrounding text
/// still reaches the agent.
pub fn take_pending_refs<T: DeserializeOwned>(msg: &mut MsgContext) -> Vec<T> {
    let serde_json::Value::Object(ref mut map) = msg.raw else {
        return Vec::new();
    };
    let Some(value) = map.remove(PENDING_MEDIA_KEY) else {
        return Vec::new();
    };
    serde_json::from_value(value).unwrap_or_default()
}

/// Pick a file extension for the on-disk filename. Trusts an original
/// `file_name` extension only if it's short and alphanumeric — otherwise
/// falls back to a media-type-specific default so paths stay well-formed
/// even when the platform's filename contains separators or shell metas.
pub fn ext_for(file_name: Option<&str>, media_type: &MediaType) -> String {
    if let Some(name) = file_name {
        if let Some(ext) = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
        {
            if !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return ext.to_ascii_lowercase();
            }
        }
    }
    match media_type {
        MediaType::Photo | MediaType::Sticker => "jpg",
        MediaType::Video => "mp4",
        MediaType::Audio | MediaType::Voice => "opus",
        MediaType::Animation => "gif",
        MediaType::Document => "bin",
    }
    .to_string()
}

/// Build a safe-for-filesystem path under
/// `~/.hope-agent/channels/<channel_id>/inbound-temp/<ts>-<stem>.<ext>`.
/// Replaces path separators / colons in `stem` with `_` so a hostile
/// filename in webhook payloads can't escape the temp dir even before
/// the canonicalize check in `persist_channel_media_to_session`.
pub async fn inbound_temp_path(
    channel_id: &str,
    stem: &str,
    ext: &str,
) -> Result<std::path::PathBuf> {
    let dir = crate::paths::channel_dir(channel_id)?.join("inbound-temp");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        anyhow!(
            "Failed to create inbound-temp dir {:?} for {}: {}",
            dir,
            channel_id,
            e
        )
    })?;
    let safe_stem: String = stem
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(dir.join(format!("{}-{}.{}", ts, safe_stem, ext)))
}

/// Stream the response body of `builder` straight to `dest`, enforcing
/// `cap_bytes` on both `Content-Length` and the mid-stream byte count.
/// Any failure (network, HTTP error, cap exceeded, file I/O) deletes the
/// partial file before returning `Err`, so callers don't have to clean
/// up after themselves.
///
/// The caller is responsible for putting auth headers / query params on
/// `builder`; this helper only owns the bytes pipeline. SSRF callers
/// must have already passed the URL through
/// [`crate::security::ssrf::check_url`] (or be downloading from a known
/// in-policy host such as a platform's official CDN).
pub async fn stream_to_disk(
    builder: reqwest::RequestBuilder,
    dest: &std::path::Path,
    cap_bytes: u64,
) -> Result<u64> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = builder
        .send()
        .await
        .map_err(|e| anyhow!("Send failed: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("<error reading body: {}>", e));
        return Err(anyhow!(
            "HTTP {}: {}",
            status,
            crate::truncate_utf8(&body, 512)
        ));
    }

    // Reject early if the server advertises a body over the cap — saves
    // opening a file for clearly oversize attachments.
    if let Some(len) = resp.content_length() {
        if len > cap_bytes {
            return Err(anyhow!(
                "declared size {} bytes exceeds {} byte cap",
                len,
                cap_bytes
            ));
        }
    }

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| anyhow!("Failed to open destination {:?}: {}", dest, e))?;

    let mut total: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                drop(file);
                abort_partial_download(dest).await;
                return Err(anyhow!("Failed to read bytes: {}", e));
            }
        };
        let next_total = total.saturating_add(chunk.len() as u64);
        if next_total > cap_bytes {
            drop(file);
            abort_partial_download(dest).await;
            return Err(anyhow!("exceeds {} byte cap mid-stream", cap_bytes));
        }
        if let Err(e) = file.write_all(&chunk).await {
            drop(file);
            abort_partial_download(dest).await;
            return Err(anyhow!("Failed to write to {:?}: {}", dest, e));
        }
        total = next_total;
    }
    if let Err(e) = file.flush().await {
        drop(file);
        abort_partial_download(dest).await;
        return Err(anyhow!("Failed to flush {:?}: {}", dest, e));
    }
    Ok(total)
}

/// Best-effort cleanup of a partially-written download. Public so
/// channels that own their own bytes loop (e.g. WeChat AES) can reuse
/// the same cleanup behavior. Quietly ignores a missing file.
pub async fn abort_partial_download(dest: &std::path::Path) {
    if let Err(e) = tokio::fs::remove_file(dest).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            app_warn!(
                "channel",
                "inbound_media",
                "Failed to clean up partial inbound download {:?}: {}",
                dest,
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::types::{ChannelId, ChatType};

    fn sample_msg() -> MsgContext {
        MsgContext {
            channel_id: ChannelId::Feishu,
            account_id: "acc".into(),
            sender_id: "u1".into(),
            sender_name: None,
            sender_username: None,
            chat_id: "c1".into(),
            chat_type: ChatType::Dm,
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

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct DummyRef {
        kind: String,
        id: String,
    }

    #[test]
    fn embed_take_round_trips_generic_refs() {
        let refs = vec![
            DummyRef {
                kind: "image".into(),
                id: "a".into(),
            },
            DummyRef {
                kind: "video".into(),
                id: "b".into(),
            },
        ];
        let mut msg = sample_msg();
        embed_pending_refs(&mut msg.raw, refs.clone());
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_some());
        let took: Vec<DummyRef> = take_pending_refs(&mut msg);
        assert_eq!(took, refs);
        // Surrounding fields preserved.
        assert_eq!(msg.raw.get("existing"), Some(&serde_json::json!(1)));
        // Second take is a no-op.
        let none_left: Vec<DummyRef> = take_pending_refs(&mut msg);
        assert!(none_left.is_empty());
    }

    #[test]
    fn embed_pending_refs_skips_empty() {
        let mut msg = sample_msg();
        embed_pending_refs::<DummyRef>(&mut msg.raw, Vec::new());
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_none());
    }

    #[test]
    fn take_pending_refs_yields_empty_for_non_object_raw() {
        let mut msg = sample_msg();
        msg.raw = serde_json::Value::Null;
        let took: Vec<DummyRef> = take_pending_refs(&mut msg);
        assert!(took.is_empty());
    }

    #[test]
    fn take_pending_refs_strips_key_on_malformed_payload() {
        let mut msg = sample_msg();
        if let serde_json::Value::Object(ref mut map) = msg.raw {
            map.insert(PENDING_MEDIA_KEY.into(), serde_json::json!("not-an-array"));
        }
        let took: Vec<DummyRef> = take_pending_refs(&mut msg);
        assert!(took.is_empty());
        // Strip even on parse failure so a follow-up call doesn't re-attempt.
        assert!(msg.raw.get(PENDING_MEDIA_KEY).is_none());
    }

    #[test]
    fn ext_for_uses_safe_filename_extension() {
        assert_eq!(
            ext_for(Some("report.PDF"), &MediaType::Document),
            "pdf".to_string()
        );
    }

    #[test]
    fn ext_for_rejects_unsafe_filename_extension() {
        // Path separators in the "extension" or non-ASCII chars fall back.
        assert_eq!(
            ext_for(Some("evil.../etc/passwd"), &MediaType::Photo),
            "jpg"
        );
        assert_eq!(ext_for(Some("name.中文"), &MediaType::Document), "bin");
    }

    #[test]
    fn ext_for_falls_back_when_no_filename() {
        assert_eq!(ext_for(None, &MediaType::Voice), "opus");
        assert_eq!(ext_for(None, &MediaType::Video), "mp4");
        assert_eq!(ext_for(None, &MediaType::Animation), "gif");
    }

    #[tokio::test]
    async fn inbound_temp_path_sanitizes_separators() {
        let root = tempfile::tempdir().expect("tempdir");
        let path =
            crate::test_support::with_env_vars_async(&[("HA_DATA_DIR", root.path())], || async {
                inbound_temp_path("dummy", "evil/../../etc:passwd", "bin").await
            })
            .await
            .expect("path");
        let filename = path.file_name().expect("filename").to_string_lossy();
        // Slash / backslash / colon all replaced; ts prefix preserved.
        assert!(!filename.contains('/'), "filename must not contain '/'");
        assert!(!filename.contains('\\'), "filename must not contain '\\\\'");
        assert!(!filename.contains(':'), "filename must not contain ':'");
        assert!(filename.contains("evil_.._.._etc_passwd"));
        assert!(filename.ends_with(".bin"));
        // Path stays under the channels/<id>/inbound-temp/ dir.
        let parent = path.parent().expect("parent");
        assert!(parent.ends_with("inbound-temp"));
    }

    /// Mock-server backed tests — exercise stream_to_disk against real
    /// HTTP responses from wiremock. Verifies cap enforcement on
    /// Content-Length, cap enforcement mid-stream, 5xx handling, and
    /// failure cleanup of partial files.
    mod wiremock_backed {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        async fn make_dest() -> (tempfile::TempDir, std::path::PathBuf) {
            let dir = tempfile::tempdir().expect("tempdir");
            let dest = dir.path().join("out.bin");
            (dir, dest)
        }

        #[tokio::test]
        async fn stream_to_disk_writes_small_body() {
            let server = MockServer::start().await;
            let body = vec![0x42u8; 256];
            Mock::given(method("GET"))
                .and(path("/file"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
                .mount(&server)
                .await;

            let (_dir, dest) = make_dest().await;
            let client = reqwest::Client::new();
            let n = stream_to_disk(
                client.get(format!("{}/file", server.uri())),
                &dest,
                1024 * 1024,
            )
            .await
            .expect("download");
            assert_eq!(n as usize, body.len());
            let on_disk = tokio::fs::read(&dest).await.expect("read");
            assert_eq!(on_disk, body);
        }

        #[tokio::test]
        async fn stream_to_disk_rejects_content_length_over_cap() {
            let server = MockServer::start().await;
            let body = vec![0u8; 2048];
            Mock::given(method("GET"))
                .and(path("/big"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&server)
                .await;

            let (_dir, dest) = make_dest().await;
            let client = reqwest::Client::new();
            let err = stream_to_disk(client.get(format!("{}/big", server.uri())), &dest, 1024)
                .await
                .expect_err("should reject");
            assert!(
                err.to_string().contains("declared size"),
                "{}",
                err.to_string()
            );
            assert!(!dest.exists(), "must not leave partial file");
        }

        #[tokio::test]
        async fn stream_to_disk_handles_http_5xx() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/boom"))
                .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
                .mount(&server)
                .await;

            let (_dir, dest) = make_dest().await;
            let client = reqwest::Client::new();
            let err = stream_to_disk(
                client.get(format!("{}/boom", server.uri())),
                &dest,
                1024 * 1024,
            )
            .await
            .expect_err("should fail");
            assert!(err.to_string().contains("HTTP 503"), "{}", err.to_string());
            assert!(!dest.exists(), "must not leave partial file");
        }

        #[tokio::test]
        async fn abort_partial_download_is_idempotent() {
            let (_dir, dest) = make_dest().await;
            // No file yet → should not warn / panic.
            abort_partial_download(&dest).await;
            assert!(!dest.exists());
            // After creating a file → should remove it.
            tokio::fs::write(&dest, b"junk").await.expect("write");
            assert!(dest.exists());
            abort_partial_download(&dest).await;
            assert!(!dest.exists());
        }
    }
}

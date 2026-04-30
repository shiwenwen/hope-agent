//! 出站附件物化辅助：把 `MediaData::{Url|FilePath|Bytes}` 拉成内存字节，
//! 同时尽力推断文件名与 MIME。给走 reqwest multipart 的 Discord / 飞书等渠道复用。
//!
//! WeChat 走自己的 CDN 加密上传链路（[`wechat::media`]），不复用本模块。

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::CONTENT_TYPE;
use uuid::Uuid;

use super::types::{MediaData, MediaType};
use crate::security::{http_stream::read_bytes_capped, ssrf::check_url};

/// URL 下载超时上限。
const URL_DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// 物化好的附件三件套，由 [`materialize_to_bytes`] 产出，喂给 reqwest multipart。
#[derive(Debug)]
pub struct MaterializedMedia {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime: String,
}

/// 把 `MediaData` 物化成内存字节 + 文件名 + MIME。`max_bytes` 在 URL 路径上同时作为
/// SSRF 限流和流式下载的硬上限，超过即 bail 防 OOM；FilePath / Bytes 路径在 read 后检查。
///
/// URL 路径强制走 `crate::security::ssrf::check_url`（默认策略 = `cached_config().ssrf.default_policy`）
/// + 30s 超时，符合"模型可控的出站 URL 必须 SSRF 校验"的项目红线（见 AGENTS.md "SSRF 统一策略"）。
pub async fn materialize_to_bytes(
    data: &MediaData,
    media_type: &MediaType,
    max_bytes: usize,
) -> Result<MaterializedMedia> {
    match data {
        MediaData::FilePath(path) => {
            let bytes = tokio::fs::read(path)
                .await
                .with_context(|| format!("Failed to read media file '{}'", path))?;
            enforce_size(bytes.len(), max_bytes, path)?;
            let filename = Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| fallback_filename(media_type));
            let mime = guess_mime_from_filename(&filename).to_string();
            Ok(MaterializedMedia {
                bytes,
                filename,
                mime,
            })
        }
        MediaData::Url(url) => {
            let ssrf_cfg = &crate::config::cached_config().ssrf;
            let parsed = check_url(url, ssrf_cfg.default_policy, &ssrf_cfg.trusted_hosts).await?;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(URL_DOWNLOAD_TIMEOUT_SECS))
                .build()
                .context("Failed to build media download client")?;
            let resp = client
                .get(parsed)
                .send()
                .await
                .with_context(|| format!("Failed to download media URL '{}'", url))?;
            if !resp.status().is_success() {
                return Err(anyhow!(
                    "Media URL returned HTTP {}: {}",
                    resp.status().as_u16(),
                    url
                ));
            }
            let header_mime = resp
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.split(';').next().unwrap_or(s).trim().to_ascii_lowercase());
            // read_bytes_capped 静默截断到 max_bytes，渠道层后续仍会按自己的硬上限再校；
            // 这里 +1 是为了让"恰好 == max_bytes 的 URL"通过、刚超的就拒绝。
            let bytes = read_bytes_capped(resp, max_bytes.saturating_add(1))
                .await
                .with_context(|| format!("Failed to stream media URL '{}'", url))?;
            enforce_size(bytes.len(), max_bytes, url)?;

            let url_filename = url::Url::parse(url)
                .ok()
                .and_then(|u| {
                    Path::new(u.path())
                        .file_name()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty());
            let filename = url_filename.unwrap_or_else(|| {
                let ext = header_mime
                    .as_deref()
                    .map(extension_for_mime)
                    .unwrap_or_else(|| default_extension_for_media(media_type));
                format!(
                    "{}.{}",
                    Uuid::new_v4().simple(),
                    ext.trim_start_matches('.')
                )
            });
            // 优先采用 server 给的 Content-Type（保留 image/heic 等本地 table 没收的类型），
            // 缺失时按文件名扩展名兜底。
            let mime =
                header_mime.unwrap_or_else(|| guess_mime_from_filename(&filename).to_string());
            Ok(MaterializedMedia {
                bytes,
                filename,
                mime,
            })
        }
        MediaData::Bytes(bytes) => {
            enforce_size(bytes.len(), max_bytes, "inline bytes")?;
            let ext = default_extension_for_media(media_type);
            let filename = format!(
                "{}.{}",
                Uuid::new_v4().simple(),
                ext.trim_start_matches('.')
            );
            let mime = guess_mime_from_filename(&filename).to_string();
            Ok(MaterializedMedia {
                bytes: bytes.clone(),
                filename,
                mime,
            })
        }
    }
}

fn enforce_size(actual: usize, max_bytes: usize, source: &str) -> Result<()> {
    if actual > max_bytes {
        bail!(
            "Media source '{}' is {} bytes, exceeds {} bytes limit",
            source,
            actual,
            max_bytes
        );
    }
    Ok(())
}

/// 按文件扩展名猜 MIME。未知扩展名返回 `application/octet-stream`。
pub fn guess_mime_from_filename(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" => "application/zip",
        "txt" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn fallback_filename(media_type: &MediaType) -> String {
    let ext = default_extension_for_media(media_type).trim_start_matches('.');
    format!("{}.{}", Uuid::new_v4().simple(), ext)
}

fn default_extension_for_media(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Photo => "jpg",
        MediaType::Video | MediaType::Animation => "mp4",
        MediaType::Audio | MediaType::Voice => "m4a",
        MediaType::Sticker => "webp",
        MediaType::Document => "bin",
    }
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用临时目录 RAII guard：测试 panic 时仍能清理。
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const TEST_MAX: usize = 16 * 1024;

    #[test]
    fn guess_mime_known_extensions() {
        assert_eq!(guess_mime_from_filename("foo.png"), "image/png");
        assert_eq!(guess_mime_from_filename("clip.MP4"), "video/mp4");
        assert_eq!(guess_mime_from_filename("doc.pdf"), "application/pdf");
        assert_eq!(
            guess_mime_from_filename("sheet.xlsx"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
    }

    #[test]
    fn guess_mime_unknown_falls_back() {
        assert_eq!(
            guess_mime_from_filename("blob.xyz"),
            "application/octet-stream"
        );
        assert_eq!(
            guess_mime_from_filename("noext"),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn materialize_bytes_passthrough_preserves_size() {
        let payload = vec![7u8; 4096];
        let data = MediaData::Bytes(payload.clone());
        let m = materialize_to_bytes(&data, &MediaType::Document, TEST_MAX)
            .await
            .expect("materialize bytes should succeed");
        assert_eq!(m.bytes, payload);
        assert!(m.filename.ends_with(".bin"), "filename = {}", m.filename);
        assert_eq!(m.mime, "application/octet-stream");
    }

    #[tokio::test]
    async fn materialize_bytes_rejects_oversize() {
        let data = MediaData::Bytes(vec![0u8; TEST_MAX + 1]);
        let err = materialize_to_bytes(&data, &MediaType::Document, TEST_MAX)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("exceeds"));
    }

    #[tokio::test]
    async fn materialize_filepath_reads_disk_and_infers_mime() {
        let dir = TempDir(
            std::env::temp_dir().join(format!("ha-media-test-{}", Uuid::new_v4().simple())),
        );
        std::fs::create_dir_all(&dir.0).unwrap();
        let path = dir.0.join("hello.png");
        std::fs::write(&path, b"PNGFAKE").unwrap();
        let data = MediaData::FilePath(path.to_string_lossy().to_string());
        let m = materialize_to_bytes(&data, &MediaType::Photo, TEST_MAX)
            .await
            .expect("materialize file should succeed");
        assert_eq!(m.bytes, b"PNGFAKE");
        assert_eq!(m.filename, "hello.png");
        assert_eq!(m.mime, "image/png");
    }
}

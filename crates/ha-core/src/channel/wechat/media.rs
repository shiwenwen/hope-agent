use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reqwest::header::{HeaderValue, CONTENT_TYPE};
use serde_json::json;
use tokio::fs;
use uuid::Uuid;

use base64::Engine as _;

use crate::channel::types::{MediaData, MediaType, OutboundMedia};

use super::api::{
    CdnMedia, FileItem, ImageItem, VideoItem, WeChatApi, DEFAULT_WECHAT_CDN_BASE_URL,
    MESSAGE_ITEM_TYPE_FILE, MESSAGE_ITEM_TYPE_IMAGE, MESSAGE_ITEM_TYPE_VIDEO,
};

const MAX_MEDIA_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone)]
struct UploadedFileInfo {
    download_encrypted_query_param: String,
    aes_key_hex: String,
    plaintext_size: usize,
    ciphertext_size: usize,
}

pub async fn send_outbound_media(
    api: &WeChatApi,
    media: &OutboundMedia,
    to_user_id: &str,
    text: Option<&str>,
    context_token: Option<&str>,
    cdn_base_url: Option<&str>,
) -> Result<String> {
    let local_path = materialize_media_data(&media.data, &media.media_type).await?;
    let upload = upload_media_to_wechat(
        api,
        &local_path,
        to_user_id,
        cdn_base_url.unwrap_or(DEFAULT_WECHAT_CDN_BASE_URL),
        &media.media_type,
    )
    .await?;

    let caption = combine_text(text, media.caption.as_deref());
    let item = build_outbound_item(&media.media_type, &local_path, &upload)?;

    // iLink bot's sendmessage renders only the first item per request when
    // item_list mixes text + media — confirmed from inbound parsing
    // (download_inbound_media handles one item per message). Send caption as
    // its own message first so the image (or file/video) isn't silently
    // dropped, then return the media message id (callers reply/thread to it).
    if let Some(caption_text) = caption {
        api.send_text(to_user_id, &caption_text, context_token)
            .await?;
    }
    api.send_message_items(to_user_id, vec![item], context_token)
        .await
}

fn build_outbound_item(
    media_type: &MediaType,
    local_path: &Path,
    upload: &UploadedFileInfo,
) -> Result<serde_json::Value> {
    let aes_key_base64 =
        base64::engine::general_purpose::STANDARD.encode(upload.aes_key_hex.as_bytes());
    let _media = json!({
        "encrypt_query_param": upload.download_encrypted_query_param,
        "aes_key": aes_key_base64,
        "encrypt_type": 1,
    });

    Ok(match media_type {
        MediaType::Photo => {
            let image_item = ImageItem {
                media: Some(CdnMedia {
                    encrypt_query_param: upload.download_encrypted_query_param.clone().into(),
                    aes_key: Some(aes_key_base64),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                aeskey: None,
                mid_size: Some(upload.ciphertext_size as u64),
            };
            serde_json::to_value(json!({
                "type": MESSAGE_ITEM_TYPE_IMAGE,
                "image_item": image_item
            }))?
        }
        MediaType::Video => {
            let video_item = VideoItem {
                media: Some(CdnMedia {
                    encrypt_query_param: upload.download_encrypted_query_param.clone().into(),
                    aes_key: Some(aes_key_base64),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                video_size: Some(upload.ciphertext_size as u64),
            };
            serde_json::to_value(json!({
                "type": MESSAGE_ITEM_TYPE_VIDEO,
                "video_item": video_item
            }))?
        }
        _ => {
            let file_item = FileItem {
                media: Some(CdnMedia {
                    encrypt_query_param: upload.download_encrypted_query_param.clone().into(),
                    aes_key: Some(aes_key_base64),
                    encrypt_type: Some(1),
                    full_url: None,
                }),
                file_name: Some(
                    local_path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("file")
                        .to_string(),
                ),
                len: Some(upload.plaintext_size.to_string()),
            };
            serde_json::to_value(json!({
                "type": MESSAGE_ITEM_TYPE_FILE,
                "file_item": file_item
            }))?
        }
    })
}

async fn upload_media_to_wechat(
    api: &WeChatApi,
    file_path: &Path,
    to_user_id: &str,
    cdn_base_url: &str,
    media_type: &MediaType,
) -> Result<UploadedFileInfo> {
    let plaintext = fs::read(file_path)
        .await
        .with_context(|| format!("Failed to read outbound media '{}'", file_path.display()))?;
    if plaintext.len() as u64 > MAX_MEDIA_BYTES {
        return Err(anyhow::anyhow!(
            "Media file exceeds maximum size ({}MB)",
            MAX_MEDIA_BYTES / 1024 / 1024
        ));
    }
    let plaintext_size = plaintext.len();
    let ciphertext_size = aes_ecb_padded_size(plaintext_size);
    let raw_md5 = md5_hex(&plaintext);
    let aes_key = rand::random::<[u8; 16]>();
    let aes_key_hex = hex_lower(&aes_key);
    let filekey = Uuid::new_v4().simple().to_string();

    let upload_response = api
        .get_upload_url(json!({
            "filekey": filekey,
            "media_type": upload_media_type(media_type),
            "to_user_id": to_user_id,
            "rawsize": plaintext_size,
            "rawfilemd5": raw_md5,
            "filesize": ciphertext_size,
            "no_need_thumb": true,
            "aeskey": aes_key_hex,
            "base_info": {
                "channel_version": format!("hope-agent/{}", env!("CARGO_PKG_VERSION")),
            }
        }))
        .await?;

    let ciphertext = aes128_ecb_encrypt_pkcs7(&aes_key, &plaintext);
    let upload_url = upload_response
        .upload_full_url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            build_cdn_upload_url(
                cdn_base_url,
                upload_response.upload_param.as_deref().unwrap_or_default(),
                &filekey,
            )
        });

    let client = reqwest::Client::new();

    // Retry CDN upload: up to 3 attempts, retry on 5xx, abort on 4xx
    let mut last_error = None;
    let mut response_headers = None;
    for attempt in 0..3 {
        let resp = client
            .post(upload_url.clone())
            .header(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            )
            .body(ciphertext.clone())
            .send()
            .await
            .with_context(|| format!("Failed to upload WeChat media to CDN: {}", upload_url))?;

        let status = resp.status();
        let headers = resp.headers().clone();
        let body_preview = resp.text().await.unwrap_or_default();

        if status.is_success() {
            response_headers = Some(headers);
            last_error = None;
            break;
        }

        let err_msg = format!(
            "WeChat CDN upload failed with {}: {}",
            status,
            crate::truncate_utf8(&body_preview, 300)
        );

        if status.is_client_error() {
            return Err(anyhow::anyhow!(err_msg));
        }

        // 5xx server error — retry
        last_error = Some(err_msg);
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    if let Some(err) = last_error {
        return Err(anyhow::anyhow!(err));
    }

    let headers = response_headers
        .ok_or_else(|| anyhow::anyhow!("WeChat CDN upload succeeded without response headers"))?;
    let download_param = headers
        .get("x-encrypted-param")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("WeChat CDN upload missing x-encrypted-param header"))?;

    Ok(UploadedFileInfo {
        download_encrypted_query_param: download_param,
        aes_key_hex,
        plaintext_size,
        ciphertext_size,
    })
}

async fn materialize_media_data(data: &MediaData, media_type: &MediaType) -> Result<PathBuf> {
    match data {
        MediaData::FilePath(path) => Ok(PathBuf::from(path)),
        MediaData::Url(url) => download_remote_media(url, media_type).await,
        MediaData::Bytes(bytes) => {
            let ext = default_extension_for_media(media_type);
            save_outbound_bytes(ext, bytes).await
        }
    }
}

async fn download_remote_media(url: &str, media_type: &MediaType) -> Result<PathBuf> {
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("Failed to download remote media '{}'", url))?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .bytes()
        .await
        .context("Failed to read remote media response body")?;
    let ext = infer_extension(content_type.as_deref(), Some(url));
    let ext = if ext == ".bin" {
        default_extension_for_media(media_type)
    } else {
        ext
    };
    save_outbound_bytes(ext, &bytes).await
}

pub(super) fn parse_aes_key(aes_key_base64: &str) -> Result<Vec<u8>> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(aes_key_base64)
        .context("Invalid WeChat aes_key base64")?;
    if decoded.len() == 16 {
        return Ok(decoded);
    }
    if decoded.len() == 32 && decoded.iter().all(|byte| byte.is_ascii_hexdigit()) {
        let hex_str = std::str::from_utf8(&decoded).context("Invalid WeChat hex aes_key")?;
        return hex_to_bytes(hex_str);
    }
    Err(anyhow::anyhow!(
        "Unsupported WeChat aes_key length: {}",
        decoded.len()
    ))
}

async fn save_outbound_bytes(ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = outbound_temp_dir()?;
    fs::create_dir_all(&dir).await?;
    let path = dir.join(format!(
        "{}.{}",
        Uuid::new_v4().simple(),
        ext.trim_start_matches('.')
    ));
    fs::write(&path, bytes).await?;
    Ok(path)
}

fn outbound_temp_dir() -> Result<PathBuf> {
    Ok(crate::paths::channel_dir("wechat")?.join("outbound-temp"))
}

fn build_cdn_upload_url(cdn_base_url: &str, upload_param: &str, filekey: &str) -> String {
    format!(
        "{}/upload?encrypted_query_param={}&filekey={}",
        cdn_base_url.trim_end_matches('/'),
        urlencoding::encode(upload_param),
        urlencoding::encode(filekey)
    )
}

pub(super) fn build_cdn_download_url(cdn_base_url: &str, encrypted_query_param: &str) -> String {
    format!(
        "{}/download?encrypted_query_param={}",
        cdn_base_url.trim_end_matches('/'),
        urlencoding::encode(encrypted_query_param)
    )
}

fn upload_media_type(media_type: &MediaType) -> i32 {
    match media_type {
        MediaType::Photo => 1,
        MediaType::Video => 2,
        MediaType::Voice => 4,
        _ => 3, // FILE
    }
}

fn default_extension_for_media(media_type: &MediaType) -> &'static str {
    match media_type {
        MediaType::Photo => ".jpg",
        MediaType::Video => ".mp4",
        MediaType::Audio | MediaType::Voice => ".wav",
        _ => ".bin",
    }
}

fn infer_extension(content_type: Option<&str>, url: Option<&str>) -> &'static str {
    let content_type = content_type
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    match content_type.as_str() {
        "image/jpeg" | "image/jpg" => ".jpg",
        "image/png" => ".png",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "video/mp4" => ".mp4",
        "video/quicktime" => ".mov",
        "audio/wav" => ".wav",
        "audio/mpeg" => ".mp3",
        "audio/silk" => ".silk",
        "application/pdf" => ".pdf",
        _ => {
            if let Some(url) = url {
                if let Ok(parsed) = url::Url::parse(url) {
                    if let Some(ext) = Path::new(parsed.path())
                        .extension()
                        .and_then(|value| value.to_str())
                    {
                        return match ext.to_ascii_lowercase().as_str() {
                            "jpg" | "jpeg" => ".jpg",
                            "png" => ".png",
                            "gif" => ".gif",
                            "webp" => ".webp",
                            "pdf" => ".pdf",
                            "mp4" => ".mp4",
                            "mov" => ".mov",
                            "mp3" => ".mp3",
                            "wav" => ".wav",
                            _ => ".bin",
                        };
                    }
                }
            }
            ".bin"
        }
    }
}

pub(super) fn mime_from_filename(filename: &str) -> String {
    match Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "pdf" => "application/pdf".to_string(),
        "txt" => "text/plain".to_string(),
        "csv" => "text/csv".to_string(),
        "zip" => "application/zip".to_string(),
        "mp4" => "video/mp4".to_string(),
        "mov" => "video/quicktime".to_string(),
        "wav" => "audio/wav".to_string(),
        "mp3" => "audio/mpeg".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn aes_ecb_padded_size(plaintext_size: usize) -> usize {
    ((plaintext_size + 16) / 16) * 16
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

/// MD5 of `data` as lowercase hex (WeChat's `rawfilemd5` upload field).
/// Pure-Rust `md-5`, byte-identical to the former `openssl` MD5.
fn md5_hex(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    hex_lower(Md5::digest(data).as_slice())
}

/// AES-128-ECB encrypt with PKCS#7 padding — byte-compatible with the
/// former `openssl::symm::encrypt(Cipher::aes_128_ecb(), key, None, data)`.
/// ECB has no IV; PKCS#7 always appends a whole padding block when the
/// input is already block-aligned (so a 16-byte input yields 32 bytes).
pub(super) fn aes128_ecb_encrypt_pkcs7(key: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    use aes::cipher::generic_array::GenericArray;
    use aes::cipher::{BlockEncrypt, KeyInit};

    let cipher = aes::Aes128::new(GenericArray::from_slice(key));
    let pad = 16 - (plaintext.len() % 16); // 1..=16
    let mut buf = Vec::with_capacity(plaintext.len() + pad);
    buf.extend_from_slice(plaintext);
    buf.resize(plaintext.len() + pad, pad as u8);
    for chunk in buf.chunks_exact_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    buf
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return Err(anyhow::anyhow!("Invalid hex length"));
    }
    let mut output = Vec::with_capacity(hex.len() / 2);
    let chars: Vec<char> = hex.chars().collect();
    for idx in (0..chars.len()).step_by(2) {
        let pair = [chars[idx], chars[idx + 1]];
        let pair_str: String = pair.iter().collect();
        let byte = u8::from_str_radix(&pair_str, 16)
            .with_context(|| format!("Invalid hex byte '{}'", pair_str))?;
        output.push(byte);
    }
    Ok(output)
}

fn combine_text(primary: Option<&str>, secondary: Option<&str>) -> Option<String> {
    let first = primary.map(str::trim).filter(|value| !value.is_empty());
    let second = secondary.map(str::trim).filter(|value| !value.is_empty());
    match (first, second) {
        (Some(a), Some(b)) if a != b => Some(format!("{}\n{}", a, b)),
        (Some(a), _) => Some(a.to_string()),
        (_, Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FIPS-197 AES-128 single-block known-answer vector. Guards that the
    /// pure-Rust `aes` primitive matches the spec (hence the former OpenSSL
    /// output) byte-for-byte after dropping the openssl dependency.
    #[test]
    fn aes128_block_matches_fips197_vector() {
        use aes::cipher::generic_array::GenericArray;
        use aes::cipher::{BlockEncrypt, KeyInit};

        let key: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let mut block = GenericArray::clone_from_slice(&[
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        aes::Aes128::new(GenericArray::from_slice(&key)).encrypt_block(&mut block);
        assert_eq!(
            hex_lower(block.as_slice()),
            "69c4e0d86a7b0430d8cdb78070b4c55a"
        );
    }

    /// PKCS#7 always appends a full block on block-aligned input, matching
    /// OpenSSL and the `aes_ecb_padded_size` size predictor.
    #[test]
    fn pkcs7_padded_len_matches_predictor() {
        let key = [0x22u8; 16];
        for len in [0usize, 1, 15, 16, 17, 31, 32, 100] {
            let ct = aes128_ecb_encrypt_pkcs7(&key, &vec![7u8; len]);
            assert_eq!(ct.len(), aes_ecb_padded_size(len), "len={}", len);
        }
        // 16-byte aligned input → 32-byte ciphertext (full pad block).
        assert_eq!(aes128_ecb_encrypt_pkcs7(&key, &[0u8; 16]).len(), 32);
    }

    /// RFC 1321 well-known vector: md5("abc").
    #[test]
    fn md5_hex_matches_known_vector() {
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}

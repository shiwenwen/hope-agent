use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use openssl::hash::{hash, MessageDigest};
use openssl::symm::{decrypt, encrypt, Cipher};
use reqwest::header::{HeaderValue, CONTENT_TYPE};
use serde_json::json;
use tokio::fs;
use uuid::Uuid;

use base64::Engine as _;

use crate::channel::types::{InboundMedia, MediaData, MediaType, OutboundMedia};

use super::api::{
    CdnMedia, FileItem, ImageItem, MessageItem, VideoItem, WeChatApi, DEFAULT_WECHAT_CDN_BASE_URL,
    MESSAGE_ITEM_TYPE_FILE, MESSAGE_ITEM_TYPE_IMAGE, MESSAGE_ITEM_TYPE_TEXT,
    MESSAGE_ITEM_TYPE_VIDEO, MESSAGE_ITEM_TYPE_VOICE,
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
    let mut items = Vec::new();
    if let Some(caption_text) = caption {
        items.push(json!({
            "type": MESSAGE_ITEM_TYPE_TEXT,
            "text_item": { "text": caption_text }
        }));
    }
    items.push(item);
    api.send_message_items(to_user_id, items, context_token)
        .await
}

pub async fn download_inbound_media(
    message_id: &str,
    item: &MessageItem,
    cdn_base_url: &str,
) -> Result<Option<InboundMedia>> {
    match item.item_type {
        MESSAGE_ITEM_TYPE_IMAGE => {
            let image = match item.image_item.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let media = match image.media.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let aes_key = image
                .aeskey
                .as_deref()
                .map(|hex| base64::engine::general_purpose::STANDARD.encode(hex.as_bytes()))
                .or_else(|| media.aes_key.clone());
            let bytes = if let Some(aes_key_base64) = aes_key.as_deref() {
                download_and_decrypt_media(media, aes_key_base64, cdn_base_url).await?
            } else {
                download_plain_media(media, cdn_base_url).await?
            };
            let path = save_inbound_bytes(
                message_id,
                "image",
                infer_extension(Some("image/jpeg"), media.full_url.as_deref()),
                &bytes,
            )
            .await?;
            return Ok(Some(InboundMedia {
                media_type: MediaType::Photo,
                file_id: file_identifier(&path),
                file_url: Some(path.to_string_lossy().to_string()),
                mime_type: Some("image/jpeg".to_string()),
                file_size: Some(bytes.len() as u64),
                caption: None,
            }));
        }
        MESSAGE_ITEM_TYPE_FILE => {
            let file = match item.file_item.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let media = match file.media.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let aes_key_base64 = match media.aes_key.as_deref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let bytes = download_and_decrypt_media(media, aes_key_base64, cdn_base_url).await?;
            let filename = file
                .file_name
                .clone()
                .unwrap_or_else(|| format!("{}.bin", message_id));
            let mime = mime_from_filename(&filename);
            let path = save_inbound_named_file(message_id, &filename, &bytes).await?;
            return Ok(Some(InboundMedia {
                media_type: MediaType::Document,
                file_id: file_identifier(&path),
                file_url: Some(path.to_string_lossy().to_string()),
                mime_type: Some(mime),
                file_size: Some(bytes.len() as u64),
                caption: None,
            }));
        }
        MESSAGE_ITEM_TYPE_VIDEO => {
            let video = match item.video_item.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let media = match video.media.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let aes_key_base64 = match media.aes_key.as_deref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let bytes = download_and_decrypt_media(media, aes_key_base64, cdn_base_url).await?;
            let path = save_inbound_bytes(message_id, "video", ".mp4", &bytes).await?;
            return Ok(Some(InboundMedia {
                media_type: MediaType::Video,
                file_id: file_identifier(&path),
                file_url: Some(path.to_string_lossy().to_string()),
                mime_type: Some("video/mp4".to_string()),
                file_size: Some(bytes.len() as u64),
                caption: None,
            }));
        }
        MESSAGE_ITEM_TYPE_VOICE => {
            let voice = match item.voice_item.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let media = match voice.media.as_ref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let aes_key_base64 = match media.aes_key.as_deref() {
                Some(value) => value,
                None => return Ok(None),
            };
            let bytes = download_and_decrypt_media(media, aes_key_base64, cdn_base_url).await?;
            let path = save_inbound_bytes(message_id, "voice", ".silk", &bytes).await?;
            return Ok(Some(InboundMedia {
                media_type: MediaType::Voice,
                file_id: file_identifier(&path),
                file_url: Some(path.to_string_lossy().to_string()),
                mime_type: Some("audio/silk".to_string()),
                file_size: Some(bytes.len() as u64),
                caption: None,
            }));
        }
        _ => {}
    }

    Ok(None)
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
    let raw_md5 = hex_lower(hash(MessageDigest::md5(), &plaintext)?.as_ref());
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

    let ciphertext = encrypt(Cipher::aes_128_ecb(), &aes_key, None, &plaintext)
        .context("Failed to AES-encrypt WeChat media")?;
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

    let headers = response_headers.expect("headers must be set on success");
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

async fn download_and_decrypt_media(
    media: &CdnMedia,
    aes_key_base64: &str,
    cdn_base_url: &str,
) -> Result<Vec<u8>> {
    let encrypted = download_plain_media(media, cdn_base_url).await?;
    let raw_key = parse_aes_key(aes_key_base64)?;
    let decrypted = decrypt(Cipher::aes_128_ecb(), &raw_key, None, &encrypted)
        .context("Failed to AES-decrypt WeChat inbound media")?;
    Ok(decrypted)
}

async fn download_plain_media(media: &CdnMedia, cdn_base_url: &str) -> Result<Vec<u8>> {
    let download_url = media
        .full_url
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            media
                .encrypt_query_param
                .as_ref()
                .map(|param| build_cdn_download_url(cdn_base_url, param))
        })
        .ok_or_else(|| anyhow::anyhow!("Missing WeChat CDN download URL"))?;

    let response = reqwest::get(download_url.clone())
        .await
        .with_context(|| format!("Failed to download WeChat media '{}'", download_url))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("Failed to read WeChat CDN body")?;
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "WeChat CDN download failed with {}",
            status
        ));
    }
    Ok(bytes.to_vec())
}

fn parse_aes_key(aes_key_base64: &str) -> Result<Vec<u8>> {
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

async fn save_inbound_bytes(
    message_id: &str,
    prefix: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    let dir = inbound_temp_dir()?;
    fs::create_dir_all(&dir).await?;
    let path = dir.join(format!(
        "{}-{}{}",
        sanitize_name(message_id),
        prefix,
        normalize_extension(ext)
    ));
    fs::write(&path, bytes).await?;
    Ok(path)
}

async fn save_inbound_named_file(
    message_id: &str,
    original_name: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    let dir = inbound_temp_dir()?;
    fs::create_dir_all(&dir).await?;
    let filename = format!(
        "{}-{}",
        sanitize_name(message_id),
        sanitize_name(original_name)
    );
    let path = dir.join(filename);
    fs::write(&path, bytes).await?;
    Ok(path)
}

fn outbound_temp_dir() -> Result<PathBuf> {
    Ok(crate::paths::channel_dir("wechat")?.join("outbound-temp"))
}

fn inbound_temp_dir() -> Result<PathBuf> {
    Ok(crate::paths::channel_dir("wechat")?.join("inbound-temp"))
}

fn build_cdn_upload_url(cdn_base_url: &str, upload_param: &str, filekey: &str) -> String {
    format!(
        "{}/upload?encrypted_query_param={}&filekey={}",
        cdn_base_url.trim_end_matches('/'),
        urlencoding::encode(upload_param),
        urlencoding::encode(filekey)
    )
}

fn build_cdn_download_url(cdn_base_url: &str, encrypted_query_param: &str) -> String {
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

fn mime_from_filename(filename: &str) -> String {
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

fn normalize_extension(ext: &str) -> String {
    if ext.starts_with('.') {
        ext.to_string()
    } else {
        format!(".{}", ext)
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '?' | '&' | '=' | '*' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
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

fn file_identifier(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file")
        .to_string()
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

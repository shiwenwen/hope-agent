use base64::Engine as _;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::browser::IMAGE_BASE64_PREFIX;

pub(crate) const IMAGE_FILE_PREFIX: &str = "__IMAGE_FILE__";
const MAX_IMAGE_FILE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
enum MarkerKind {
    Base64,
    File,
}

#[derive(Debug, Clone)]
pub(crate) enum ImageMarkerPayload {
    Base64(String),
    FilePath(String),
}

#[derive(Debug)]
pub(crate) struct ImageMarker {
    pub mime: String,
    pub payload: ImageMarkerPayload,
    pub text: String,
}

#[derive(Debug)]
pub(crate) struct ParsedImageMarkers {
    pub leading_text: String,
    pub markers: Vec<ImageMarker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageFileSpec {
    mime: String,
    path: String,
}

/// Parse internal image transport markers used by visual tools.
///
/// Supported forms:
/// - `__IMAGE_BASE64__<mime>__<base64>__\n<text>`
/// - `__IMAGE_FILE__{"mime":"image/png","path":"/managed/path.png"}\n<text>`
///
/// Returns `None` for absent or malformed markers so callers can safely fall
/// back to plain text instead of sending invalid image payloads to providers.
pub(crate) fn parse_image_markers(result: &str) -> Option<ParsedImageMarkers> {
    let (mut marker_start, mut kind) = find_next_marker(result, 0)?;
    let leading_text = result[..marker_start].trim().to_string();
    let mut markers = Vec::new();

    loop {
        let text_start = match kind {
            MarkerKind::Base64 => {
                let after_prefix = marker_start + IMAGE_BASE64_PREFIX.len();
                let part = &result[after_prefix..];
                let (raw_mime, rest) = part.split_once("__")?;
                let mime = raw_mime.trim();
                if !is_image_mime(mime) {
                    return None;
                }

                let (raw_b64, _) = rest.split_once("__")?;
                let b64 = raw_b64.trim();
                if !is_valid_standard_base64(b64) {
                    return None;
                }

                let text_start = after_prefix + raw_mime.len() + 2 + raw_b64.len() + 2;
                markers.push(ImageMarker {
                    mime: mime.to_string(),
                    payload: ImageMarkerPayload::Base64(b64.to_string()),
                    text: String::new(),
                });
                text_start
            }
            MarkerKind::File => {
                let after_prefix = marker_start + IMAGE_FILE_PREFIX.len();
                let line_end = result[after_prefix..]
                    .find('\n')
                    .map(|p| after_prefix + p)?;
                let spec: ImageFileSpec =
                    serde_json::from_str(result[after_prefix..line_end].trim()).ok()?;
                let mime = spec.mime.trim();
                if !is_image_mime(mime) || !is_safe_managed_image_path(&spec.path) {
                    return None;
                }
                markers.push(ImageMarker {
                    mime: mime.to_string(),
                    payload: ImageMarkerPayload::FilePath(spec.path),
                    text: String::new(),
                });
                line_end + 1
            }
        };

        let next = find_next_marker(result, text_start);
        let text_end = next.map(|(idx, _)| idx).unwrap_or(result.len());
        let text = result[text_start..text_end]
            .strip_prefix('\n')
            .unwrap_or(&result[text_start..text_end])
            .trim()
            .to_string();
        if let Some(last) = markers.last_mut() {
            last.text = text;
        }

        let Some((next_start, next_kind)) = next else {
            break;
        };
        marker_start = next_start;
        kind = next_kind;
    }

    if markers.is_empty() {
        return None;
    }

    Some(ParsedImageMarkers {
        leading_text,
        markers,
    })
}

pub(crate) fn encode_marker_image(marker: &ImageMarker) -> anyhow::Result<String> {
    match &marker.payload {
        ImageMarkerPayload::Base64(b64) => Ok(b64.clone()),
        ImageMarkerPayload::FilePath(path) => encode_managed_image_file(path, &marker.mime),
    }
}

pub(crate) fn build_image_file_marker(mime: &str, path: &str, text: &str) -> String {
    let spec = serde_json::json!({
        "mime": mime,
        "path": path,
    });
    format!("{IMAGE_FILE_PREFIX}{spec}\n{text}")
}

pub(crate) fn build_image_base64_marker(mime: &str, b64: &str, text: &str) -> String {
    format!("{IMAGE_BASE64_PREFIX}{mime}__{b64}__\n{text}")
}

pub(crate) fn contains_image_marker(result: &str) -> bool {
    result.contains(IMAGE_BASE64_PREFIX) || result.contains(IMAGE_FILE_PREFIX)
}

pub(crate) fn has_valid_image_markers(result: &str) -> bool {
    parse_image_markers(result).is_some()
}

fn find_next_marker(result: &str, start: usize) -> Option<(usize, MarkerKind)> {
    let base64_pos = result[start..]
        .find(IMAGE_BASE64_PREFIX)
        .map(|p| (start + p, MarkerKind::Base64));
    let file_pos = result[start..]
        .find(IMAGE_FILE_PREFIX)
        .map(|p| (start + p, MarkerKind::File));

    match (base64_pos, file_pos) {
        (Some((b_idx, b_kind)), Some((f_idx, f_kind))) => {
            if b_idx <= f_idx {
                Some((b_idx, b_kind))
            } else {
                Some((f_idx, f_kind))
            }
        }
        (Some(p), None) | (None, Some(p)) => Some(p),
        (None, None) => None,
    }
}

fn encode_managed_image_file(path: &str, declared_mime: &str) -> anyhow::Result<String> {
    let canonical = canonicalize_managed_image_path(path)?;
    let metadata = std::fs::metadata(&canonical)?;
    if metadata.len() > MAX_IMAGE_FILE_BYTES {
        anyhow::bail!(
            "image file too large for provider input: {}B (max {}B)",
            metadata.len(),
            MAX_IMAGE_FILE_BYTES
        );
    }
    let bytes = std::fs::read(&canonical)?;
    let sniffed = crate::attachments::sniff_mime_magic(&bytes)
        .ok_or_else(|| anyhow::anyhow!("image file MIME could not be verified"))?;
    if !sniffed.starts_with("image/") {
        anyhow::bail!("file is not an image: {}", sniffed);
    }
    if sniffed != declared_mime {
        anyhow::bail!(
            "image file MIME mismatch: marker declared {}, file is {}",
            declared_mime,
            sniffed
        );
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

fn canonicalize_managed_image_path(path: &str) -> anyhow::Result<PathBuf> {
    let canonical = Path::new(path).canonicalize()?;
    if !is_under_managed_media_root(&canonical)? {
        anyhow::bail!(
            "image file marker path is outside Hope Agent managed media directories: {}",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn is_safe_managed_image_path(path: &str) -> bool {
    canonicalize_managed_image_path(path).is_ok()
}

fn is_under_managed_media_root(path: &Path) -> anyhow::Result<bool> {
    let root = managed_root()?;
    for subdir in ["attachments", "tool_results"] {
        if let Ok(allowed) = root.join(subdir).canonicalize() {
            if path.starts_with(allowed) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn managed_root() -> anyhow::Result<&'static Path> {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    if let Some(cached) = ROOT.get() {
        return Ok(cached.as_path());
    }
    let canonical = crate::paths::root_dir()?.canonicalize()?;
    Ok(ROOT.get_or_init(|| canonical).as_path())
}

fn is_image_mime(mime: &str) -> bool {
    let Some(subtype) = mime.strip_prefix("image/") else {
        return false;
    };
    !subtype.is_empty()
        && subtype
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

fn is_valid_standard_base64(data: &str) -> bool {
    if data.is_empty() || data.len() % 4 != 0 {
        return false;
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{build_image_file_marker, parse_image_markers, IMAGE_FILE_PREFIX};
    use crate::tools::browser::IMAGE_BASE64_PREFIX;

    #[test]
    fn parses_valid_image_marker() {
        let result = format!(
            "prefix\n{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        let parsed = parse_image_markers(&result).expect("valid marker");

        assert_eq!(parsed.leading_text, "prefix");
        assert_eq!(parsed.markers.len(), 1);
        assert_eq!(parsed.markers[0].mime, "image/png");
        assert_eq!(parsed.markers[0].text, "Screenshot captured.");
    }

    #[test]
    fn rejects_truncated_marker_preview() {
        let result = format!(
            "{}image/png__aGVs\n\n[...527806 bytes omitted...]\n\nbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        assert!(parse_image_markers(&result).is_none());
    }

    #[test]
    fn rejects_non_image_mime() {
        let result = format!(
            "{}text/plain__aGVsbG8=__\nNot an image.",
            IMAGE_BASE64_PREFIX
        );

        assert!(parse_image_markers(&result).is_none());
    }

    #[test]
    fn builds_file_marker_shape() {
        let marker = build_image_file_marker(
            "image/png",
            "/definitely/not/a/managed/file.png",
            "Screenshot captured.",
        );

        assert!(marker.starts_with(IMAGE_FILE_PREFIX));
        assert!(marker.contains("\"mime\":\"image/png\""));
        assert!(marker.contains("\"path\":\"/definitely/not/a/managed/file.png\""));
    }
}

use base64::Engine as _;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use super::browser::IMAGE_BASE64_PREFIX;

pub(crate) const IMAGE_FILE_PREFIX: &str = "__IMAGE_FILE__";
const MAX_IMAGE_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MANAGED_IMAGE_SUBDIRS: &[&str] = &["attachments", "tool_results", "mac-control/snapshots"];

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

pub(crate) fn materialize_base64_image_markers(
    result: &str,
    session_id: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(parsed) = parse_image_markers(result) else {
        return Ok(None);
    };

    let mut changed = false;
    let mut written_paths: Vec<String> = Vec::new();
    let mut rebuilt = Vec::with_capacity(parsed.markers.len() + 1);
    if !parsed.leading_text.is_empty() {
        rebuilt.push(parsed.leading_text);
    }

    for (idx, marker) in parsed.markers.iter().enumerate() {
        let marker_text = match &marker.payload {
            ImageMarkerPayload::Base64(b64) => {
                match materialize_one_base64(session_id, idx, b64, &marker.text) {
                    Ok((path, file_marker)) => {
                        written_paths.push(path);
                        changed = true;
                        file_marker
                    }
                    // Partial failure: remove the files already written this
                    // call so a bailed multi-image result never leaves orphaned
                    // bytes in `tool_results/` (the caller keeps the inline
                    // result on Err).
                    Err(e) => {
                        for path in &written_paths {
                            let _ = std::fs::remove_file(path);
                        }
                        return Err(e);
                    }
                }
            }
            ImageMarkerPayload::FilePath(path) => {
                build_image_file_marker(&marker.mime, path, &marker.text)
            }
        };
        rebuilt.push(marker_text);
    }

    if changed {
        Ok(Some(rebuilt.join("\n")))
    } else {
        Ok(None)
    }
}

/// Decode one base64 image marker, verify it is a real image, write the bytes
/// under `tool_results/<session>/` and return `(written_path, file_marker)`.
fn materialize_one_base64(
    session_id: Option<&str>,
    marker_index: usize,
    b64: &str,
    marker_text: &str,
) -> anyhow::Result<(String, String)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let sniffed_mime = crate::attachments::sniff_mime_magic(&bytes)
        .ok_or_else(|| anyhow::anyhow!("base64 image marker MIME could not be verified"))?;
    if !sniffed_mime.starts_with("image/") {
        anyhow::bail!("base64 image marker is not an image: {}", sniffed_mime);
    }
    let path =
        save_materialized_image_bytes(session_id, marker_index, sniffed_mime, bytes.as_slice())?;
    let file_marker = build_image_file_marker(sniffed_mime, path.as_str(), marker_text);
    Ok((path, file_marker))
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
    is_under_managed_media_root_for_root(root, path)
}

fn is_under_managed_media_root_for_root(root: &Path, path: &Path) -> anyhow::Result<bool> {
    for subdir in MANAGED_IMAGE_SUBDIRS {
        let allowed = root.join(subdir).canonicalize();
        if let Ok(allowed) = allowed {
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

fn save_materialized_image_bytes(
    session_id: Option<&str>,
    marker_index: usize,
    mime: &str,
    bytes: &[u8],
) -> anyhow::Result<String> {
    let session_segment = session_id
        .map(crate::paths::sanitize_path_segment)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "_global".to_string());
    let dir = crate::paths::root_dir()?
        .join("tool_results")
        .join(session_segment);
    std::fs::create_dir_all(&dir)?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let ext = image_extension_for_mime(mime);
    let path = dir.join(format!("vision_input_{nanos}_{marker_index}.{ext}",));
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().to_string())
}

fn image_extension_for_mime(mime: &str) -> String {
    let raw_subtype = mime.strip_prefix("image/").unwrap_or("bin");
    let subtype = raw_subtype
        .split_once('+')
        .map(|(base, _)| base)
        .unwrap_or(raw_subtype);
    let ext = match subtype {
        "jpeg" | "pjpeg" => "jpg",
        "svg" => "svg",
        other => other,
    };
    let sanitized = ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    if sanitized.is_empty() {
        "bin".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_image_base64_marker, build_image_file_marker, is_under_managed_media_root_for_root,
        materialize_base64_image_markers, parse_image_markers, IMAGE_FILE_PREFIX,
    };
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    use base64::Engine as _;
    use std::path::Path;

    const PNG_1X1_B64: &str =
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lT5cWQAAAABJRU5ErkJggg==";

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

    #[test]
    fn sanitized_session_segment_cannot_escape_tool_results_dir() {
        assert_eq!(crate::paths::sanitize_path_segment(".."), "__");
        assert_eq!(
            crate::paths::sanitize_path_segment("session/../x"),
            "session____x"
        );
    }

    #[test]
    fn materializes_base64_marker_to_tool_results_file() {
        let root = tempfile::tempdir().expect("tempdir");

        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let marker = build_image_base64_marker("image/png", PNG_1X1_B64, "Screenshot.");

            let materialized = materialize_base64_image_markers(&marker, Some("session/../x"))
                .expect("materialize marker")
                .expect("base64 marker should be replaced");

            assert!(materialized.starts_with(IMAGE_FILE_PREFIX));
            assert!(materialized.contains("\"mime\":\"image/png\""));
            assert!(materialized.contains("Screenshot."));
            assert!(materialized.contains("session____x"));
            assert!(!materialized.contains(PNG_1X1_B64));

            let spec_line = materialized
                .strip_prefix(IMAGE_FILE_PREFIX)
                .and_then(|rest| rest.split_once('\n').map(|(spec, _)| spec))
                .expect("file marker JSON line");
            let spec: serde_json::Value =
                serde_json::from_str(spec_line).expect("file marker JSON");
            let path = spec
                .get("path")
                .and_then(|v| v.as_str())
                .expect("path in marker");
            assert!(Path::new(path).starts_with(root.path().join("tool_results/session____x")));
            let written = std::fs::read(path).expect("materialized image file");
            let original = base64::engine::general_purpose::STANDARD
                .decode(PNG_1X1_B64)
                .expect("test PNG base64");
            assert_eq!(written, original);
        });
    }

    #[test]
    fn managed_image_root_allows_mac_control_snapshots_only_under_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        let snapshots = root.join("mac-control").join("snapshots");
        std::fs::create_dir_all(&snapshots).expect("create snapshots dir");
        let screenshot = snapshots.join("macsnap_test.jpg");
        std::fs::write(&screenshot, b"fake").expect("write screenshot");
        let canonical_screenshot = screenshot.canonicalize().expect("canonical screenshot");

        assert!(
            is_under_managed_media_root_for_root(root, &canonical_screenshot)
                .expect("managed root check")
        );

        let outside = temp.path().join("mac-control").join("other");
        std::fs::create_dir_all(&outside).expect("create outside dir");
        let outside_file = outside.join("macsnap_test.jpg");
        std::fs::write(&outside_file, b"fake").expect("write outside");
        let canonical_outside = outside_file.canonicalize().expect("canonical outside");

        assert!(
            !is_under_managed_media_root_for_root(root, &canonical_outside)
                .expect("managed root check")
        );
    }
}

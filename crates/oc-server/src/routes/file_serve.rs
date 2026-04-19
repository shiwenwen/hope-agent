//! Shared helpers for the file-serving routes under `/api/`.
//!
//! `attachments.rs`, `avatars.rs`, `generated_images.rs`, and `canvas.rs`
//! all follow the same safety model:
//!
//! 1. Reject suspicious filenames / sub-paths up front (cheap pre-filter).
//! 2. Canonicalize both the candidate and the base directory, then verify
//!    containment with `starts_with` (the real check — catches symlinks
//!    and any exotic traversal the pre-filter misses).
//! 3. Resolve a sensible `Content-Type` for the response.
//! 4. Stamp `Content-Disposition: inline` + a short cache header.
//!
//! This module centralizes steps 1/2/3/4 so a future fifth route can't
//! drop a check by accident. Each helper takes an options struct so the
//! minor per-route variations (html charset, magic-byte sniff fallback,
//! no-referrer for iframe sub-requests) stay explicit at the call site.

use std::path::{Path, PathBuf};

use axum::http::{header, HeaderValue, Response};

use oc_core::attachments;

use crate::error::AppError;

/// Reject filenames that are empty, too long, start with `.`, or contain
/// any of `/`, `\`, or `..`. The canonicalization step inside
/// [`contained_canonical`] is the real traversal guard; this is the cheap
/// pre-filter that fails fast with a 400 before hitting the filesystem.
pub fn validate_safe_filename(name: &str) -> Result<(), AppError> {
    if name.is_empty() || name.len() > 256 {
        return Err(AppError::bad_request("invalid filename"));
    }
    if name.starts_with('.') {
        return Err(AppError::bad_request("invalid filename"));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::bad_request("invalid filename"));
    }
    Ok(())
}

/// Sub-path inside a serving root (e.g. the rest of a canvas project URL
/// past the project-id segment). Disallows `..` segments, absolute paths,
/// and backslashes; the `starts_with` containment check in
/// [`contained_canonical`] still runs afterwards and is the authoritative
/// guard.
pub fn validate_safe_rest_path(rest: &str) -> Result<(), AppError> {
    if rest.is_empty() || rest.len() > 1024 {
        return Err(AppError::bad_request("invalid path"));
    }
    if rest.starts_with('/') || rest.starts_with('\\') {
        return Err(AppError::bad_request("invalid path"));
    }
    for seg in rest.split('/') {
        if seg == ".." || seg.contains('\\') {
            return Err(AppError::bad_request("invalid path"));
        }
    }
    Ok(())
}

/// Canonicalize `candidate`, canonicalize `base_dir`, and verify the
/// former is inside the latter. Returns the canonical file path on
/// success (callers feed it into `tower_http::services::ServeFile`).
///
/// A missing candidate file yields 404; a containment violation yields
/// 403. Either outcome is a dead-end for the request.
pub async fn contained_canonical(
    base_dir: &Path,
    candidate: &Path,
) -> Result<PathBuf, AppError> {
    let file_canon = match tokio::fs::canonicalize(candidate).await {
        Ok(p) => p,
        Err(_) => return Err(AppError::not_found("file not found")),
    };
    let dir_canon = match tokio::fs::canonicalize(base_dir).await {
        Ok(p) => p,
        Err(_) => return Err(AppError::not_found("base directory not found")),
    };
    if !file_canon.starts_with(&dir_canon) {
        return Err(AppError::forbidden("path traversal rejected"));
    }
    Ok(file_canon)
}

/// Flags controlling [`resolve_mime_for_path`]'s behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct MimeOpts {
    /// When true, HTML/HTM files get `text/html; charset=utf-8` (iframe
    /// contexts). Otherwise the raw extension MIME is returned.
    pub html_charset: bool,
    /// When true, files without a known extension trigger a 512-byte
    /// magic-byte sniff (used by `/api/attachments/*` where users upload
    /// arbitrary file types).
    pub sniff_fallback: bool,
}

/// Resolve a `Content-Type` for `path`. See [`MimeOpts`] for per-route
/// toggles.
pub async fn resolve_mime_for_path(path: &Path, opts: MimeOpts) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    if let Some(ext) = ext.as_deref() {
        if opts.html_charset && (ext == "html" || ext == "htm") {
            return "text/html; charset=utf-8".to_string();
        }
        if let Some(m) = attachments::mime_from_extension(ext) {
            return m.to_string();
        }
    }
    if opts.sniff_fallback {
        if let Ok(head) = read_head(path, 512).await {
            return attachments::sniff_mime(&head, path);
        }
    }
    "application/octet-stream".to_string()
}

/// Flags controlling [`apply_inline_media_headers`]'s behavior.
#[derive(Debug, Clone, Copy)]
pub struct HeaderOpts<'a> {
    pub mime: &'a str,
    /// `Cache-Control: private, max-age={cache_secs}`.
    pub cache_secs: u32,
    /// `Content-Disposition` value. Most callers want `"inline"`; the
    /// attachments route builds a filename-bearing variant inline.
    pub disposition: &'a str,
    /// Set `Referrer-Policy: no-referrer` — used by canvas iframe to keep
    /// `?token=...` out of sub-request referrers.
    pub no_referrer: bool,
}

/// Stamp inline-delivery headers on a built response. Safe to call on a
/// response that already carries a default `Content-Type` — this
/// overrides it.
pub fn apply_inline_media_headers<B>(response: &mut Response<B>, opts: HeaderOpts<'_>) {
    if let Ok(hv) = HeaderValue::from_str(opts.mime) {
        response.headers_mut().insert(header::CONTENT_TYPE, hv);
    }
    if let Ok(hv) = HeaderValue::from_str(opts.disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, hv);
    }
    let cache = format!("private, max-age={}", opts.cache_secs);
    if let Ok(hv) = HeaderValue::from_str(&cache) {
        response.headers_mut().insert(header::CACHE_CONTROL, hv);
    }
    if opts.no_referrer {
        response.headers_mut().insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
}

async fn read_head(path: &Path, len: usize) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; len];
    let n = f.read(&mut buf).await?;
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_rejects_traversal_and_dotfiles() {
        assert!(validate_safe_filename("").is_err());
        assert!(validate_safe_filename(".hidden").is_err());
        assert!(validate_safe_filename("..").is_err());
        assert!(validate_safe_filename("a/b").is_err());
        assert!(validate_safe_filename("a\\b").is_err());
        assert!(validate_safe_filename("foo/../bar").is_err());
    }

    #[test]
    fn filename_accepts_typical() {
        assert!(validate_safe_filename("avatar.png").is_ok());
        assert!(validate_safe_filename("report 2026-04-18.pdf").is_ok());
    }

    #[test]
    fn rest_path_rejects_traversal_and_absolute() {
        assert!(validate_safe_rest_path("").is_err());
        assert!(validate_safe_rest_path("/etc/passwd").is_err());
        assert!(validate_safe_rest_path("\\windows").is_err());
        assert!(validate_safe_rest_path("a/../b").is_err());
        assert!(validate_safe_rest_path("a/..").is_err());
        assert!(validate_safe_rest_path("a\\b").is_err());
    }

    #[test]
    fn rest_path_accepts_typical() {
        assert!(validate_safe_rest_path("index.html").is_ok());
        assert!(validate_safe_rest_path("assets/style.css").is_ok());
        assert!(validate_safe_rest_path("deeply/nested/path/file.txt").is_ok());
    }
}

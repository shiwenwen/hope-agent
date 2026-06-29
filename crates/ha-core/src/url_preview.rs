use anyhow::Result;
use base64::Engine as _;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::ttl_cache::TtlCache;

// ── Constants ───────────────────────────────────────────────────

const PREVIEW_TIMEOUT_SECS: u64 = 5;
const PREVIEW_MAX_BYTES: usize = 65_536; // 64 KB – enough for <head>
const PREVIEW_MAX_REDIRECTS: usize = 3;
const PREVIEW_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const PREVIEW_CACHE_MAX_ENTRIES: usize = 100;
const PREVIEW_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const FAVICON_TIMEOUT_SECS: u64 = 3;
const FAVICON_MAX_BYTES: usize = 64 * 1024;
const FAVICON_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const FAVICON_CACHE_MAX_ENTRIES: usize = 256;

/// File extensions that should NOT be previewed (media / binary).
const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "bmp", "mp4", "webm", "mov", "avi", "mp3",
    "wav", "ogg", "flac", "zip", "tar", "gz", "rar", "7z", "pdf", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "exe", "dmg", "iso",
];

// ── Data Types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlPreviewMeta {
    pub url: String,
    pub final_url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub favicon: Option<String>,
    pub site_name: Option<String>,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaviconData {
    pub data_url: String,
    pub mime_type: String,
    pub source_url: String,
}

// ── Cache ───────────────────────────────────────────────────────

static PREVIEW_CACHE: Lazy<TtlCache<String, UrlPreviewMeta>> =
    Lazy::new(|| TtlCache::new(PREVIEW_CACHE_MAX_ENTRIES));
static FAVICON_CACHE: Lazy<TtlCache<String, Option<FaviconData>>> =
    Lazy::new(|| TtlCache::new(FAVICON_CACHE_MAX_ENTRIES));

fn read_cache(url: &str) -> Option<UrlPreviewMeta> {
    PREVIEW_CACHE.get(url, PREVIEW_CACHE_TTL)
}

fn write_cache(url: String, data: UrlPreviewMeta) {
    PREVIEW_CACHE.put(url, data);
}

fn read_favicon_cache(url: &str) -> Option<Option<FaviconData>> {
    FAVICON_CACHE.get(url, FAVICON_CACHE_TTL)
}

fn write_favicon_cache(url: String, data: Option<FaviconData>) {
    FAVICON_CACHE.put(url, data);
}

// ── URL Validation ──────────────────────────────────────────────

fn should_skip_url(url_str: &str) -> bool {
    let lower = url_str.to_lowercase();

    // Must be http/https
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return true;
    }

    // Skip media / binary extensions
    if let Some(path) = lower.split('?').next() {
        if let Some(ext) = path.rsplit('.').next() {
            if SKIP_EXTENSIONS.contains(&ext) {
                return true;
            }
        }
    }

    false
}

fn extract_domain(url_str: &str) -> String {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

fn default_favicon_url(page_url: &url::Url) -> Option<url::Url> {
    let mut favicon_url = page_url.clone();
    favicon_url.set_username("").ok()?;
    favicon_url.set_password(None).ok()?;
    favicon_url.set_path("/favicon.ico");
    favicon_url.set_query(None);
    favicon_url.set_fragment(None);
    Some(favicon_url)
}

async fn checked_get(
    client: &reqwest::Client,
    url_str: &str,
    max_redirects: usize,
) -> Result<reqwest::Response> {
    let ssrf_cfg = crate::config::cached_config().ssrf.clone();
    let mut next =
        crate::security::ssrf::check_url(url_str, ssrf_cfg.url_preview(), &ssrf_cfg.trusted_hosts)
            .await?;

    for _ in 0..=max_redirects {
        let resp = client
            .get(next.clone())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

        if !resp.status().is_redirection() {
            return Ok(resp);
        }

        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("Redirect missing Location header"))?;
        let redirected = resp
            .url()
            .join(location)
            .map_err(|e| anyhow::anyhow!("Invalid redirect Location: {}", e))?;
        next = crate::security::ssrf::check_url(
            redirected.as_str(),
            ssrf_cfg.url_preview(),
            &ssrf_cfg.trusted_hosts,
        )
        .await?;
    }

    Err(anyhow::anyhow!("Too many redirects"))
}

// ── OpenGraph Extraction ────────────────────────────────────────

static RE_OG_TITLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?(?:property|name)\s*=\s*["']og:title["'][^>]*?\scontent\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_OG_TITLE_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?content\s*=\s*["']([^"']*?)["'][^>]*?\s(?:property|name)\s*=\s*["']og:title["']"#).unwrap()
});
static RE_OG_DESC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?(?:property|name)\s*=\s*["']og:description["'][^>]*?\scontent\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_OG_DESC_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?content\s*=\s*["']([^"']*?)["'][^>]*?\s(?:property|name)\s*=\s*["']og:description["']"#).unwrap()
});
static RE_OG_IMAGE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?(?:property|name)\s*=\s*["']og:image["'][^>]*?\scontent\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_OG_IMAGE_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?content\s*=\s*["']([^"']*?)["'][^>]*?\s(?:property|name)\s*=\s*["']og:image["']"#).unwrap()
});
static RE_OG_SITE_NAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?(?:property|name)\s*=\s*["']og:site_name["'][^>]*?\scontent\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_OG_SITE_NAME_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?content\s*=\s*["']([^"']*?)["'][^>]*?\s(?:property|name)\s*=\s*["']og:site_name["']"#).unwrap()
});

// Fallback meta tags
static RE_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)<title[^>]*>([^<]*)</title>"#).unwrap());
static RE_META_DESC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?name\s*=\s*["']description["'][^>]*?\scontent\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_META_DESC_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<meta\s+(?:[^>]*?\s)?content\s*=\s*["']([^"']*?)["'][^>]*?\sname\s*=\s*["']description["']"#).unwrap()
});
static RE_FAVICON: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<link\s+[^>]*?rel\s*=\s*["'](?:icon|shortcut icon)["'][^>]*?href\s*=\s*["']([^"']*?)["']"#).unwrap()
});
static RE_FAVICON_ALT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)<link\s+[^>]*?href\s*=\s*["']([^"']*?)["'][^>]*?rel\s*=\s*["'](?:icon|shortcut icon)["']"#).unwrap()
});

fn extract_og(cap: &Regex, cap_alt: &Regex, html: &str) -> Option<String> {
    cap.captures(html)
        .or_else(|| cap_alt.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| decode_html_entities(m.as_str().trim()))
        .filter(|s| !s.is_empty())
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

fn resolve_url(base: &str, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    url::Url::parse(base)
        .ok()
        .and_then(|base_url| base_url.join(href).ok())
        .map(|u| u.to_string())
}

fn parse_head(
    html: &str,
    final_url: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let title = extract_og(&RE_OG_TITLE, &RE_OG_TITLE_ALT, html).or_else(|| {
        RE_TITLE
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| decode_html_entities(m.as_str().trim()))
            .filter(|s| !s.is_empty())
    });

    let description = extract_og(&RE_OG_DESC, &RE_OG_DESC_ALT, html)
        .or_else(|| extract_og(&RE_META_DESC, &RE_META_DESC_ALT, html));

    let image = extract_og(&RE_OG_IMAGE, &RE_OG_IMAGE_ALT, html)
        .and_then(|href| resolve_url(final_url, &href));

    let site_name = extract_og(&RE_OG_SITE_NAME, &RE_OG_SITE_NAME_ALT, html);

    let favicon = RE_FAVICON
        .captures(html)
        .or_else(|| RE_FAVICON_ALT.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .and_then(|href| resolve_url(final_url, &href));

    (title, description, image, site_name, favicon)
}

fn sniff_favicon_mime(bytes: &[u8], content_type: Option<&str>) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"\0\0\x01\0") || bytes.starts_with(b"\0\0\x02\0") {
        return Some("image/x-icon");
    }

    let content_type = content_type?
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match content_type.as_str() {
        "image/vnd.microsoft.icon" | "image/x-icon" => Some("image/x-icon"),
        _ => None,
    }
}

// ── Core Fetch ──────────────────────────────────────────────────

pub async fn fetch_preview(url_str: &str) -> Result<UrlPreviewMeta> {
    // Validate URL
    if should_skip_url(url_str) {
        return Err(anyhow::anyhow!("URL not eligible for preview"));
    }

    let domain = extract_domain(url_str);
    if domain.is_empty() {
        return Err(anyhow::anyhow!("Could not extract domain"));
    }

    // Check cache
    if let Some(cached) = read_cache(url_str) {
        return Ok(cached);
    }

    // Build HTTP client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(PREVIEW_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(PREVIEW_USER_AGENT)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

    let resp = checked_get(&client, url_str, PREVIEW_MAX_REDIRECTS).await?;

    let final_url = resp.url().to_string();

    // Check content type – only parse HTML
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
        // Non-HTML: return minimal preview with just the domain
        let meta = UrlPreviewMeta {
            url: url_str.to_string(),
            final_url,
            title: None,
            description: None,
            image: None,
            favicon: None,
            site_name: None,
            domain,
        };
        write_cache(url_str.to_string(), meta.clone());
        return Ok(meta);
    }

    // Stream body with byte limit – stop after reading enough for <head>
    let bytes = crate::security::http_stream::read_bytes_capped(resp, PREVIEW_MAX_BYTES).await?;

    let body_slice = if bytes.len() > PREVIEW_MAX_BYTES {
        &bytes[..PREVIEW_MAX_BYTES]
    } else {
        &bytes[..]
    };

    let html = String::from_utf8_lossy(body_slice);

    let (title, description, image, site_name, _) = parse_head(&html, &final_url);

    let meta = UrlPreviewMeta {
        url: url_str.to_string(),
        final_url,
        title,
        description,
        image,
        favicon: None,
        site_name,
        domain,
    };

    write_cache(url_str.to_string(), meta.clone());
    Ok(meta)
}

async fn fetch_favicon_url(favicon_url: &str) -> Result<Option<FaviconData>> {
    let favicon_url = {
        let ssrf_cfg = crate::config::cached_config().ssrf.clone();
        crate::security::ssrf::check_url(
            favicon_url,
            ssrf_cfg.url_preview(),
            &ssrf_cfg.trusted_hosts,
        )
        .await?
        .to_string()
    };

    if let Some(cached) = read_favicon_cache(&favicon_url) {
        return Ok(cached);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FAVICON_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(PREVIEW_USER_AGENT)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

    let resp = match checked_get(&client, &favicon_url, PREVIEW_MAX_REDIRECTS).await {
        Ok(resp) => resp,
        Err(e) => {
            write_favicon_cache(favicon_url, None);
            log::debug!("favicon fetch failed: {}", e);
            return Ok(None);
        }
    };

    if !resp.status().is_success() {
        write_favicon_cache(favicon_url, None);
        return Ok(None);
    }

    let source_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes =
        crate::security::http_stream::read_bytes_capped(resp, FAVICON_MAX_BYTES + 1).await?;
    if bytes.is_empty() || bytes.len() > FAVICON_MAX_BYTES {
        write_favicon_cache(favicon_url, None);
        return Ok(None);
    }

    let Some(mime_type) = sniff_favicon_mime(&bytes, content_type.as_deref()) else {
        write_favicon_cache(favicon_url, None);
        return Ok(None);
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data = FaviconData {
        data_url: format!("data:{mime_type};base64,{encoded}"),
        mime_type: mime_type.to_string(),
        source_url,
    };
    write_favicon_cache(favicon_url, Some(data.clone()));
    Ok(Some(data))
}

async fn fetch_favicon_with_candidate(
    page_url: &str,
    candidate_url: Option<&str>,
) -> Result<Option<FaviconData>> {
    let parsed_page_url = {
        let ssrf_cfg = crate::config::cached_config().ssrf.clone();
        crate::security::ssrf::check_url(page_url, ssrf_cfg.url_preview(), &ssrf_cfg.trusted_hosts)
            .await?
    };

    if let Some(candidate_url) = candidate_url {
        if let Ok(Some(icon)) = fetch_favicon_url(candidate_url).await {
            return Ok(Some(icon));
        }
    }

    let Some(default_url) = default_favicon_url(&parsed_page_url) else {
        return Ok(None);
    };
    fetch_favicon_url(default_url.as_str()).await
}

async fn fetch_declared_favicon_candidate(page_url: &str) -> Result<Option<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FAVICON_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(PREVIEW_USER_AGENT)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

    let resp = checked_get(&client, page_url, PREVIEW_MAX_REDIRECTS).await?;
    if !resp.status().is_success() {
        return Ok(None);
    }

    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
        return Ok(None);
    }

    let bytes = crate::security::http_stream::read_bytes_capped(resp, PREVIEW_MAX_BYTES).await?;
    let html = String::from_utf8_lossy(&bytes);
    let (_, _, _, _, favicon) = parse_head(&html, &final_url);
    Ok(favicon)
}

pub async fn fetch_favicon(page_url: &str) -> Result<Option<FaviconData>> {
    let checked_page_url = {
        let ssrf_cfg = crate::config::cached_config().ssrf.clone();
        crate::security::ssrf::check_url(page_url, ssrf_cfg.url_preview(), &ssrf_cfg.trusted_hosts)
            .await?
            .to_string()
    };

    if let Some(cached) = read_favicon_cache(&checked_page_url) {
        return Ok(cached);
    }

    let candidate = fetch_declared_favicon_candidate(&checked_page_url)
        .await
        .ok()
        .flatten();
    let data = fetch_favicon_with_candidate(&checked_page_url, candidate.as_deref()).await?;
    write_favicon_cache(checked_page_url, data.clone());
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_favicon_url_keeps_origin_only() {
        let page = url::Url::parse("https://user:pass@example.com:8443/a/b?x=1#frag").unwrap();
        let favicon = default_favicon_url(&page).unwrap();
        assert_eq!(favicon.as_str(), "https://example.com:8443/favicon.ico");
    }

    #[test]
    fn parse_head_preserves_declared_favicon_without_default_fallback() {
        let html = r#"<html><head><link rel="icon" href="/assets/icon.png"></head></html>"#;
        let (_, _, _, _, favicon) = parse_head(html, "https://example.com/docs/page");
        assert_eq!(
            favicon.as_deref(),
            Some("https://example.com/assets/icon.png")
        );

        let (_, _, _, _, missing) =
            parse_head("<html><head></head></html>", "https://example.com/");
        assert_eq!(missing, None);
    }

    #[test]
    fn sniff_favicon_mime_accepts_common_icon_formats() {
        assert_eq!(
            sniff_favicon_mime(b"\x89PNG\r\n\x1a\nrest", None),
            Some("image/png")
        );
        assert_eq!(
            sniff_favicon_mime(b"\0\0\x01\0rest", None),
            Some("image/x-icon")
        );
        assert_eq!(
            sniff_favicon_mime(b"<svg></svg>", Some("image/svg+xml")),
            None
        );
    }
}

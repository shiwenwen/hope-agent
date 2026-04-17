use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ── Constants ───────────────────────────────────────────────────

const PREVIEW_TIMEOUT_SECS: u64 = 5;
const PREVIEW_MAX_BYTES: usize = 65_536; // 64 KB – enough for <head>
const PREVIEW_MAX_REDIRECTS: usize = 3;
const PREVIEW_CACHE_TTL_MINUTES: u64 = 5;
const PREVIEW_CACHE_MAX_ENTRIES: usize = 100;
const PREVIEW_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

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

// ── Cache ───────────────────────────────────────────────────────

struct CacheEntry {
    data: UrlPreviewMeta,
    inserted_at: Instant,
}

static PREVIEW_CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn read_cache(url: &str) -> Option<UrlPreviewMeta> {
    let cache = PREVIEW_CACHE.lock().ok()?;
    let entry = cache.get(url)?;
    if entry.inserted_at.elapsed().as_secs() < PREVIEW_CACHE_TTL_MINUTES * 60 {
        Some(entry.data.clone())
    } else {
        None
    }
}

fn write_cache(url: String, data: UrlPreviewMeta) {
    if let Ok(mut cache) = PREVIEW_CACHE.lock() {
        let now = Instant::now();
        let ttl_secs = PREVIEW_CACHE_TTL_MINUTES * 60;
        cache.retain(|_, v| now.duration_since(v.inserted_at).as_secs() < ttl_secs);

        if cache.len() >= PREVIEW_CACHE_MAX_ENTRIES {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
        }

        cache.insert(
            url,
            CacheEntry {
                data,
                inserted_at: now,
            },
        );
    }
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
        .and_then(|href| resolve_url(final_url, &href))
        .or_else(|| {
            // Default favicon fallback
            url::Url::parse(final_url).ok().map(|u| {
                format!(
                    "{}://{}/favicon.ico",
                    u.scheme(),
                    u.host_str().unwrap_or("")
                )
            })
        });

    (title, description, image, site_name, favicon)
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

    let parsed_url = {
        let ssrf_cfg = &crate::config::cached_config().ssrf;
        crate::security::ssrf::check_url(
            url_str,
            ssrf_cfg.url_preview(),
            &ssrf_cfg.trusted_hosts,
        )
        .await?
    };

    // Build HTTP client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(PREVIEW_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(PREVIEW_MAX_REDIRECTS))
        .user_agent(PREVIEW_USER_AGENT)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {}", e))?;

    let resp = client
        .get(parsed_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Request failed: {}", e))?;

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
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read body: {}", e))?;

    let body_slice = if bytes.len() > PREVIEW_MAX_BYTES {
        &bytes[..PREVIEW_MAX_BYTES]
    } else {
        &bytes[..]
    };

    let html = String::from_utf8_lossy(body_slice);

    let (title, description, image, site_name, favicon) = parse_head(&html, &final_url);

    let meta = UrlPreviewMeta {
        url: url_str.to_string(),
        final_url,
        title,
        description,
        image,
        favicon,
        site_name,
        domain,
    };

    write_cache(url_str.to_string(), meta.clone());
    Ok(meta)
}

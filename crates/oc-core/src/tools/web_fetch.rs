use anyhow::Result;
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

use crate::provider;

const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const DEFAULT_WEB_FETCH_MAX_CHARS: usize = 50000;
const DEFAULT_WEB_FETCH_MAX_CHARS_CAP: usize = 200000;
const DEFAULT_WEB_FETCH_MAX_RESPONSE_BYTES: usize = 2_097_152; // 2 MB
const DEFAULT_WEB_FETCH_MAX_REDIRECTS: usize = 5;
const DEFAULT_WEB_FETCH_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_FETCH_CACHE_TTL_MINUTES: u64 = 15;
const WEB_FETCH_CACHE_MAX_ENTRIES: usize = 100;

// ── Web Fetch Config ────────────────────────────────────────────

/// Persistent web fetch configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebFetchConfig {
    /// Default maximum characters returned to the model
    #[serde(default = "default_wf_max_chars")]
    pub max_chars: usize,
    /// Hard cap on max_chars parameter from tool calls
    #[serde(default = "default_wf_max_chars_cap")]
    pub max_chars_cap: usize,
    /// Maximum HTTP response body bytes to download
    #[serde(default = "default_wf_max_response_bytes")]
    pub max_response_bytes: usize,
    /// Maximum redirects to follow
    #[serde(default = "default_wf_max_redirects")]
    pub max_redirects: usize,
    /// Request timeout in seconds
    #[serde(default = "default_wf_timeout_seconds")]
    pub timeout_seconds: u64,
    /// Cache TTL in minutes (0 = disabled)
    #[serde(default = "default_wf_cache_ttl_minutes")]
    pub cache_ttl_minutes: u64,
    /// Custom User-Agent string
    #[serde(default = "default_wf_user_agent")]
    pub user_agent: String,
    /// Enable SSRF protection (block private/internal IPs)
    #[serde(default = "default_wf_ssrf_protection")]
    pub ssrf_protection: bool,
}

fn default_wf_max_chars() -> usize {
    DEFAULT_WEB_FETCH_MAX_CHARS
}
fn default_wf_max_chars_cap() -> usize {
    DEFAULT_WEB_FETCH_MAX_CHARS_CAP
}
fn default_wf_max_response_bytes() -> usize {
    DEFAULT_WEB_FETCH_MAX_RESPONSE_BYTES
}
fn default_wf_max_redirects() -> usize {
    DEFAULT_WEB_FETCH_MAX_REDIRECTS
}
fn default_wf_timeout_seconds() -> u64 {
    DEFAULT_WEB_FETCH_TIMEOUT_SECS
}
fn default_wf_cache_ttl_minutes() -> u64 {
    DEFAULT_WEB_FETCH_CACHE_TTL_MINUTES
}
fn default_wf_user_agent() -> String {
    DEFAULT_WEB_FETCH_USER_AGENT.to_string()
}
fn default_wf_ssrf_protection() -> bool {
    true
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_WEB_FETCH_MAX_CHARS,
            max_chars_cap: DEFAULT_WEB_FETCH_MAX_CHARS_CAP,
            max_response_bytes: DEFAULT_WEB_FETCH_MAX_RESPONSE_BYTES,
            max_redirects: DEFAULT_WEB_FETCH_MAX_REDIRECTS,
            timeout_seconds: DEFAULT_WEB_FETCH_TIMEOUT_SECS,
            cache_ttl_minutes: DEFAULT_WEB_FETCH_CACHE_TTL_MINUTES,
            user_agent: DEFAULT_WEB_FETCH_USER_AGENT.to_string(),
            ssrf_protection: true,
        }
    }
}

// ── SSRF Protection ─────────────────────────────────────────────

/// Check if a URL is safe to fetch (not targeting private/internal networks).
pub(crate) async fn check_ssrf_safe(url_str: &str) -> Result<()> {
    let parsed = url::Url::parse(url_str).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr_str = format!("{}:{}", host, port);

    let addrs = tokio::net::lookup_host(&addr_str)
        .await
        .map_err(|e| anyhow::anyhow!("DNS resolution failed for {}: {}", host, e))?;

    for addr in addrs {
        let ip = addr.ip();
        if is_private_ip(&ip) {
            return Err(anyhow::anyhow!(
                "SSRF protection: blocked request to private/internal address {} (resolved from {})",
                ip, host
            ));
        }
    }

    Ok(())
}

/// Check if an IP address belongs to a private/reserved range.
pub(crate) fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                            // 127.0.0.0/8
                || v4.is_private()                      // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()                   // 169.254.0.0/16
                || v4.is_unspecified()                   // 0.0.0.0
                || v4.octets()[0] == 0                   // 0.0.0.0/8
                || v4.is_broadcast() // 255.255.255.255
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                            // ::1
                || v6.is_unspecified()                   // ::
                // fc00::/7 (unique local)
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn is_blocked_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_private_ip(&ip);
    }
    false
}

fn validate_fetch_url(url: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(anyhow::anyhow!(
                "Invalid URL: only http:// and https:// are supported"
            ))
        }
    }
    Ok(parsed)
}

// ── Web Fetch Cache ─────────────────────────────────────────────

struct CacheEntry {
    response: String,
    inserted_at: Instant,
}

static WEB_FETCH_CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cache_key(url: &str, extract_mode: &str) -> String {
    format!("{}:{}", extract_mode, url.to_lowercase().trim())
}

fn read_cache(key: &str, ttl_minutes: u64) -> Option<String> {
    if ttl_minutes == 0 {
        return None;
    }
    let cache = WEB_FETCH_CACHE.lock().ok()?;
    let entry = cache.get(key)?;
    let elapsed = entry.inserted_at.elapsed();
    if elapsed.as_secs() < ttl_minutes * 60 {
        Some(entry.response.clone())
    } else {
        None
    }
}

fn write_cache(key: String, response: String, ttl_minutes: u64) {
    if ttl_minutes == 0 {
        return;
    }
    if let Ok(mut cache) = WEB_FETCH_CACHE.lock() {
        // Evict expired entries first
        let now = Instant::now();
        let ttl_secs = ttl_minutes * 60;
        cache.retain(|_, v| now.duration_since(v.inserted_at).as_secs() < ttl_secs);

        // Evict oldest if still at capacity
        if cache.len() >= WEB_FETCH_CACHE_MAX_ENTRIES {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(
            key,
            CacheEntry {
                response,
                inserted_at: now,
            },
        );
    }
}

// ── Readability Extraction + HTML→Markdown ──────────────────────

/// Extract article content using Mozilla Readability, with fallback to basic HTML cleaning.
/// Returns (content, title, extractor_name).
fn extract_content(
    html: &str,
    url: &str,
    extract_mode: &str,
) -> (String, Option<String>, &'static str) {
    // Try Readability first
    let parsed_url =
        url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("https://example.com").unwrap());
    match readability::extractor::extract(&mut html.as_bytes(), &parsed_url) {
        Ok(product) => {
            let title = if product.title.is_empty() {
                None
            } else {
                Some(product.title)
            };
            let article_html = product.content;
            if article_html.trim().is_empty() {
                // Readability returned empty → fallback
                let text = extract_readable_text_basic(html);
                return (text, title, "basic");
            }
            match extract_mode {
                "markdown" => {
                    let md = htmd::convert(&article_html)
                        .unwrap_or_else(|_| extract_readable_text_basic(&article_html));
                    (md, title, "readability")
                }
                _ => {
                    let text = extract_readable_text_basic(&article_html);
                    (text, title, "readability")
                }
            }
        }
        Err(_) => {
            // Readability failed → basic fallback
            let text = if extract_mode == "markdown" {
                htmd::convert(html).unwrap_or_else(|_| extract_readable_text_basic(html))
            } else {
                extract_readable_text_basic(html)
            };
            (text, None, "basic")
        }
    }
}

/// Basic HTML text extraction — strips tags, scripts, styles; normalizes whitespace.
/// Kept as fallback when Readability fails.
fn extract_readable_text_basic(html: &str) -> String {
    let mut cleaned = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut skip_tag: Option<String> = None;

    while let Some(ch) = chars.next() {
        if ch != '<' {
            if skip_tag.is_none() {
                cleaned.push(ch);
            }
            continue;
        }

        let mut tag_content = String::new();
        let mut reached_end = false;
        for c in chars.by_ref() {
            if c == '>' {
                reached_end = true;
                break;
            }
            tag_content.push(c);
        }
        if !reached_end {
            break;
        }

        let trimmed = tag_content.trim_start();
        let is_closing = trimmed.starts_with('/');
        let name_src = if is_closing { &trimmed[1..] } else { trimmed };
        let tag_name: String = name_src
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .map(|c| c.to_ascii_lowercase())
            .collect();

        if tag_name.is_empty() {
            if skip_tag.is_none() {
                cleaned.push(' ');
            }
            continue;
        }

        if let Some(current_skip) = skip_tag.as_deref() {
            if is_closing && current_skip == tag_name {
                skip_tag = None;
            }
            continue;
        }

        if matches!(tag_name.as_str(), "script" | "style" | "noscript" | "nav") && !is_closing {
            skip_tag = Some(tag_name);
            continue;
        }

        cleaned.push(' ');
    }

    let mut result = String::with_capacity(cleaned.len() / 2);
    let mut in_tag = false;
    let mut last_was_space = false;
    let mut newline_count = 0;

    for c in cleaned.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if c == '>' {
            in_tag = false;
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
            continue;
        }
        if in_tag {
            continue;
        }
        if c == '\n' || c == '\r' {
            newline_count += 1;
            if newline_count <= 2 && !last_was_space {
                result.push('\n');
                last_was_space = true;
            }
            continue;
        }
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
            continue;
        }
        newline_count = 0;
        last_was_space = false;
        result.push(c);
    }

    html_decode(result.trim())
}

fn truncate_to_char_count(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        return s;
    }

    let cut = s
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(s.len());
    &s[..cut]
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

// ── Tool Entry Point ─────────────────────────────────────────────

pub(crate) async fn tool_web_fetch(args: &Value) -> Result<String> {
    let start_time = Instant::now();

    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

    let extract_mode = args
        .get("extract_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("markdown");

    // Load config
    let config = provider::load_store()
        .map(|s| s.web_fetch)
        .unwrap_or_default();

    let max_chars = {
        let requested = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(config.max_chars as u64) as usize;
        requested.min(config.max_chars_cap)
    };

    app_info!(
        "tool",
        "web_fetch",
        "Fetching URL: {} (mode: {}, max_chars: {})",
        url,
        extract_mode,
        max_chars
    );

    let parsed_url = validate_fetch_url(url)?;

    // Check cache
    let ck = cache_key(url, extract_mode);
    if let Some(cached) = read_cache(&ck, config.cache_ttl_minutes) {
        app_info!("tool", "web_fetch", "Cache hit for {}", url);
        return Ok(cached);
    }

    // SSRF protection
    if config.ssrf_protection {
        check_ssrf_safe(url).await?;
    }

    // Build HTTP client with config
    let max_redirects = config.max_redirects;
    let ssrf_protection = config.ssrf_protection;
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= max_redirects {
            return attempt.error("too many redirects");
        }
        if ssrf_protection {
            if let Some(host) = attempt.url().host_str() {
                if is_blocked_host(host) {
                    return attempt.stop();
                }
            }
        }
        attempt.follow()
    });

    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .redirect(redirect_policy),
    )
    .build()
    .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(parsed_url)
        .header("Accept", "text/html,application/json,text/plain,*/*")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Fetch request failed: {}", e))?;

    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("Fetch failed with status: {}", status));
    }

    let final_url = resp.url().to_string();

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Stream-read body with byte limit
    let mut body_bytes = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut body_truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("Stream read error: {}", e))?;
        body_bytes.extend_from_slice(&chunk);
        if body_bytes.len() > config.max_response_bytes {
            body_bytes.truncate(config.max_response_bytes);
            body_truncated = true;
            break;
        }
    }
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    // Extract content based on content-type
    let (mut text, title, extractor) = if content_type.contains("text/html") {
        extract_content(&body, url, extract_mode)
    } else if content_type.contains("application/json") {
        let formatted = match serde_json::from_str::<Value>(&body) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(body.clone()),
            Err(_) => body.clone(),
        };
        (formatted, None, "json")
    } else if content_type.contains("text/markdown") {
        if extract_mode == "text" {
            (extract_readable_text_basic(&body), None, "raw")
        } else {
            (body.clone(), None, "raw")
        }
    } else {
        (body.clone(), None, "raw")
    };

    // Truncate content
    let total_chars = text.chars().count();
    let truncated = body_truncated || total_chars > max_chars;
    if total_chars > max_chars {
        text = truncate_to_char_count(&text, max_chars).to_string();
    }

    let took_ms = start_time.elapsed().as_millis() as u64;

    // Build structured JSON response
    let response_json = serde_json::json!({
        "url": url,
        "finalUrl": final_url,
        "status": status,
        "contentType": content_type,
        "title": title,
        "extractMode": extract_mode,
        "extractor": extractor,
        "cached": false,
        "truncated": truncated,
        "totalChars": total_chars,
        "tookMs": took_ms,
        "content": text
    });

    let result = format!(
        "<web_fetch_result url=\"{}\" status=\"{}\" extractor=\"{}\">\n{}\n</web_fetch_result>",
        url,
        status,
        extractor,
        serde_json::to_string_pretty(&response_json).unwrap_or_else(|_| response_json.to_string())
    );

    // Write to cache
    write_cache(ck, result.clone(), config.cache_ttl_minutes);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{
        extract_readable_text_basic, is_blocked_host, truncate_to_char_count, validate_fetch_url,
    };

    #[test]
    fn extract_text_handles_unicode_without_panicking() {
        let html = r#"<div>你好<script>bad()</script><p>世界🌍</p></div>"#;
        let out = extract_readable_text_basic(html);
        assert!(out.contains("你好"));
        assert!(out.contains("世界🌍"));
        assert!(!out.contains("bad()"));
    }

    #[test]
    fn truncate_to_char_count_preserves_utf8_boundary() {
        let s = "ab好c";
        assert_eq!(truncate_to_char_count(s, 0), "");
        assert_eq!(truncate_to_char_count(s, 2), "ab");
        assert_eq!(truncate_to_char_count(s, 3), "ab好");
        assert_eq!(truncate_to_char_count(s, 10), s);
    }

    #[test]
    fn validate_fetch_url_rejects_non_http_schemes() {
        assert!(validate_fetch_url("https://example.com").is_ok());
        assert!(validate_fetch_url("file:///etc/passwd").is_err());
        assert!(validate_fetch_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn blocked_host_detection_covers_local_targets() {
        assert!(is_blocked_host("localhost"));
        assert!(is_blocked_host("127.0.0.1"));
        assert!(is_blocked_host("::1"));
        assert!(!is_blocked_host("example.com"));
    }
}

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

fn default_wf_max_chars() -> usize { DEFAULT_WEB_FETCH_MAX_CHARS }
fn default_wf_max_chars_cap() -> usize { DEFAULT_WEB_FETCH_MAX_CHARS_CAP }
fn default_wf_max_response_bytes() -> usize { DEFAULT_WEB_FETCH_MAX_RESPONSE_BYTES }
fn default_wf_max_redirects() -> usize { DEFAULT_WEB_FETCH_MAX_REDIRECTS }
fn default_wf_timeout_seconds() -> u64 { DEFAULT_WEB_FETCH_TIMEOUT_SECS }
fn default_wf_cache_ttl_minutes() -> u64 { DEFAULT_WEB_FETCH_CACHE_TTL_MINUTES }
fn default_wf_user_agent() -> String { DEFAULT_WEB_FETCH_USER_AGENT.to_string() }
fn default_wf_ssrf_protection() -> bool { true }

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
    let parsed = url::Url::parse(url_str)
        .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    let host = parsed.host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr_str = format!("{}:{}", host, port);

    let addrs = tokio::net::lookup_host(&addr_str).await
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
                || v4.is_broadcast()                     // 255.255.255.255
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
            if let Some(oldest_key) = cache.iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(key, CacheEntry {
            response,
            inserted_at: now,
        });
    }
}

// ── Readability Extraction + HTML→Markdown ──────────────────────

/// Extract article content using Mozilla Readability, with fallback to basic HTML cleaning.
/// Returns (content, title, extractor_name).
fn extract_content(html: &str, url: &str, extract_mode: &str) -> (String, Option<String>, &'static str) {
    // Try Readability first
    let parsed_url = url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("https://example.com").unwrap());
    match readability::extractor::extract(&mut html.as_bytes(), &parsed_url) {
        Ok(product) => {
            let title = if product.title.is_empty() { None } else { Some(product.title) };
            let article_html = product.content;
            if article_html.trim().is_empty() {
                // Readability returned empty → fallback
                let text = extract_readable_text_basic(html);
                return (text, title, "basic");
            }
            match extract_mode {
                "markdown" => {
                    let md = htmd::convert(&article_html).unwrap_or_else(|_| {
                        extract_readable_text_basic(&article_html)
                    });
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
    let mut pos = 0;
    let lower = html.to_lowercase();
    let mut cleaned = String::with_capacity(html.len());

    while pos < html.len() {
        let remaining_lower = &lower[pos..];
        if remaining_lower.starts_with("<script") {
            if let Some(end) = lower[pos..].find("</script>") {
                pos += end + 9;
                continue;
            }
        }
        if remaining_lower.starts_with("<style") {
            if let Some(end) = lower[pos..].find("</style>") {
                pos += end + 8;
                continue;
            }
        }
        if remaining_lower.starts_with("<noscript") {
            if let Some(end) = lower[pos..].find("</noscript>") {
                pos += end + 11;
                continue;
            }
        }
        if remaining_lower.starts_with("<nav") {
            if let Some(end) = lower[pos..].find("</nav>") {
                pos += end + 6;
                continue;
            }
        }
        cleaned.push(html.as_bytes()[pos] as char);
        pos += 1;
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

    app_info!("tool", "web_fetch", "Fetching URL: {} (mode: {}, max_chars: {})", url, extract_mode, max_chars);

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "Invalid URL: must start with http:// or https://"
        ));
    }

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
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .redirect(reqwest::redirect::Policy::limited(config.max_redirects))
    )
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(url)
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
    let total_chars = text.len();
    let truncated = body_truncated || text.len() > max_chars;
    if text.len() > max_chars {
        text.truncate(max_chars);
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
        url, status, extractor,
        serde_json::to_string_pretty(&response_json).unwrap_or_else(|_| response_json.to_string())
    );

    // Write to cache
    write_cache(ck, result.clone(), config.cache_ttl_minutes);

    Ok(result)
}

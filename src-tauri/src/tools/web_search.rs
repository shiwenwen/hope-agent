use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::provider;

const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

const DEFAULT_WEB_SEARCH_RESULT_COUNT: usize = 5;
const DEFAULT_WEB_SEARCH_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES: u64 = 15;
const WEB_SEARCH_CACHE_MAX_ENTRIES: usize = 200;

// ── Web Search Provider Config ───────────────────────────────────

/// Supported web search providers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WebSearchProvider {
    /// DuckDuckGo HTML scraping — free, no API key
    DuckDuckGo,
    /// SearXNG self-hosted meta-search — free, needs instance URL
    Searxng,
    /// Brave Search API — requires API key
    Brave,
    /// Perplexity Sonar API — requires API key
    Perplexity,
    /// Google Custom Search JSON API — requires API key + CX
    Google,
    /// Grok (X.AI) — requires API key
    Grok,
    /// Kimi (Moonshot) — requires API key
    Kimi,
    /// Tavily Search API — requires API key (1000 free/month)
    Tavily,
}

impl std::fmt::Display for WebSearchProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuckDuckGo => write!(f, "DuckDuckGo"),
            Self::Searxng => write!(f, "SearXNG"),
            Self::Brave => write!(f, "Brave"),
            Self::Perplexity => write!(f, "Perplexity"),
            Self::Google => write!(f, "Google"),
            Self::Grok => write!(f, "Grok"),
            Self::Kimi => write!(f, "Kimi"),
            Self::Tavily => write!(f, "Tavily"),
        }
    }
}

/// A single search provider entry with enabled state and credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchProviderEntry {
    pub id: WebSearchProvider,
    pub enabled: bool,
    /// API key (Brave / Perplexity / Google / Grok / Kimi)
    #[serde(default)]
    pub api_key: Option<String>,
    /// Second credential (Google CX)
    #[serde(default)]
    pub api_key2: Option<String>,
    /// Instance URL (SearXNG)
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Persistent web search configuration, stored in config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchConfig {
    /// Ordered list of providers. First enabled provider is used.
    #[serde(default = "default_providers")]
    pub providers: Vec<WebSearchProviderEntry>,
    /// Docker-managed SearXNG container
    #[serde(default)]
    pub searxng_docker_managed: Option<bool>,
    /// Default number of search results (1-10)
    #[serde(default = "default_ws_result_count")]
    pub default_result_count: usize,
    /// Request timeout in seconds (5-120)
    #[serde(default = "default_ws_timeout_secs")]
    pub timeout_seconds: u64,
    /// Cache TTL in minutes (0 = disabled)
    #[serde(default = "default_ws_cache_ttl")]
    pub cache_ttl_minutes: u64,
    /// Default country filter (ISO 3166-1 alpha-2)
    #[serde(default)]
    pub default_country: Option<String>,
    /// Default language filter (ISO 639-1)
    #[serde(default)]
    pub default_language: Option<String>,
    /// Default freshness filter (day/week/month/year)
    #[serde(default)]
    pub default_freshness: Option<String>,
}

fn default_providers() -> Vec<WebSearchProviderEntry> {
    vec![
        WebSearchProviderEntry { id: WebSearchProvider::DuckDuckGo, enabled: true, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Searxng, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Brave, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Perplexity, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Google, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Grok, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Kimi, enabled: false, api_key: None, api_key2: None, base_url: None },
        WebSearchProviderEntry { id: WebSearchProvider::Tavily, enabled: false, api_key: None, api_key2: None, base_url: None },
    ]
}

/// Ensure all known providers exist in the list (appends any missing ones).
/// This handles the case where a new provider is added but the user's saved config
/// was created before that provider existed.
pub fn backfill_providers(config: &mut WebSearchConfig) {
    let defaults = default_providers();
    for default_entry in &defaults {
        if !config.providers.iter().any(|p| p.id == default_entry.id) {
            config.providers.push(default_entry.clone());
        }
    }
}

fn default_ws_result_count() -> usize { DEFAULT_WEB_SEARCH_RESULT_COUNT }
fn default_ws_timeout_secs() -> u64 { DEFAULT_WEB_SEARCH_TIMEOUT_SECS }
fn default_ws_cache_ttl() -> u64 { DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES }

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            providers: default_providers(),
            searxng_docker_managed: None,
            default_result_count: DEFAULT_WEB_SEARCH_RESULT_COUNT,
            timeout_seconds: DEFAULT_WEB_SEARCH_TIMEOUT_SECS,
            cache_ttl_minutes: DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES,
            default_country: None,
            default_language: None,
            default_freshness: None,
        }
    }
}

/// Resolve which provider to use: first enabled provider in the ordered list.
/// Falls back to DuckDuckGo if none enabled.
fn resolve_provider(config: &WebSearchConfig) -> (WebSearchProvider, &WebSearchProviderEntry) {
    for entry in &config.providers {
        if entry.enabled {
            return (entry.id.clone(), entry);
        }
    }
    // Fallback: DDG (always works)
    static DDG_FALLBACK: std::sync::LazyLock<WebSearchProviderEntry> = std::sync::LazyLock::new(|| {
        WebSearchProviderEntry { id: WebSearchProvider::DuckDuckGo, enabled: true, api_key: None, api_key2: None, base_url: None }
    });
    (WebSearchProvider::DuckDuckGo, &DDG_FALLBACK)
}

// ── Tool Entry Point ─────────────────────────────────────────────

pub(crate) async fn tool_web_search(args: &Value) -> Result<String> {
    let config = provider::load_store()
        .map(|s| s.web_search)
        .unwrap_or_default();

    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.default_result_count as u64)
        .min(10) as usize;

    let params = SearchParams {
        country: args.get("country").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| config.default_country.clone()),
        language: args.get("language").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| config.default_language.clone()),
        freshness: args.get("freshness").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| config.default_freshness.clone()),
    };

    let (provider_id, entry) = resolve_provider(&config);
    let timeout = config.timeout_seconds;

    app_info!("tool", "web_search", "Web search [{}]: {} (count: {}, country: {:?}, lang: {:?}, freshness: {:?})",
        provider_id, query, count, params.country, params.language, params.freshness);

    // Check cache
    let ck = search_cache_key(&provider_id.to_string(), query, count, &params);
    if let Some(cached) = read_search_cache(&ck, config.cache_ttl_minutes) {
        app_info!("tool", "web_search", "Cache hit for [{}]: {}", provider_id, query);
        return Ok(cached);
    }

    let results = match provider_id {
        WebSearchProvider::DuckDuckGo => search_duckduckgo(query, count, timeout).await,
        WebSearchProvider::Searxng => {
            let url = entry.base_url.as_deref().unwrap_or("http://localhost:8080");
            search_searxng(url, query, count, &params, timeout).await
        }
        WebSearchProvider::Brave => {
            let key = entry.api_key.as_deref().unwrap_or("");
            search_brave(key, query, count, &params, timeout).await
        }
        WebSearchProvider::Perplexity => {
            let key = entry.api_key.as_deref().unwrap_or("");
            search_perplexity(key, query, count, &params, timeout).await
        }
        WebSearchProvider::Google => {
            let key = entry.api_key.as_deref().unwrap_or("");
            let cx = entry.api_key2.as_deref().unwrap_or("");
            search_google(key, cx, query, count, &params, timeout).await
        }
        WebSearchProvider::Grok => {
            let key = entry.api_key.as_deref().unwrap_or("");
            search_grok(key, query, count, timeout).await
        }
        WebSearchProvider::Kimi => {
            let key = entry.api_key.as_deref().unwrap_or("");
            search_kimi(key, query, count, timeout).await
        }
        WebSearchProvider::Tavily => {
            let key = entry.api_key.as_deref().unwrap_or("");
            search_tavily(key, query, count, &params, timeout).await
        }
    }?;

    if results.is_empty() {
        return Ok(format!("No results found for: {}", query));
    }

    let mut output = format!("Search results for: {} (via {})\n\n", query, provider_id);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.snippet
        ));
    }

    // Write to cache
    write_search_cache(ck, output.clone(), config.cache_ttl_minutes);

    Ok(output)
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

// ── Search Params & Helpers ─────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct SearchParams {
    country: Option<String>,
    language: Option<String>,
    freshness: Option<String>,
}

fn brave_freshness(f: &str) -> &str {
    match f { "day" => "pd", "week" => "pw", "month" => "pm", "year" => "py", _ => f }
}

fn google_date_restrict(f: &str) -> &str {
    match f { "day" => "d1", "week" => "w1", "month" => "m1", "year" => "y1", _ => f }
}

fn tavily_days(f: &str) -> u32 {
    match f { "day" => 1, "week" => 7, "month" => 30, "year" => 365, _ => 30 }
}

fn build_search_client(timeout_secs: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(DEFAULT_WEB_FETCH_USER_AGENT)
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))
}

// ── Search Result Cache ─────────────────────────────────────────

struct CacheEntry {
    response: String,
    inserted_at: Instant,
}

static WEB_SEARCH_CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn search_cache_key(provider: &str, query: &str, count: usize, params: &SearchParams) -> String {
    format!("{}:{}:{}:{}:{}:{}",
        provider,
        query.to_lowercase().trim(),
        count,
        params.country.as_deref().unwrap_or(""),
        params.language.as_deref().unwrap_or(""),
        params.freshness.as_deref().unwrap_or(""),
    )
}

fn read_search_cache(key: &str, ttl_minutes: u64) -> Option<String> {
    if ttl_minutes == 0 { return None; }
    let cache = WEB_SEARCH_CACHE.lock().ok()?;
    let entry = cache.get(key)?;
    if entry.inserted_at.elapsed().as_secs() < ttl_minutes * 60 {
        Some(entry.response.clone())
    } else {
        None
    }
}

fn write_search_cache(key: String, response: String, ttl_minutes: u64) {
    if ttl_minutes == 0 { return; }
    if let Ok(mut cache) = WEB_SEARCH_CACHE.lock() {
        let now = Instant::now();
        let ttl_secs = ttl_minutes * 60;
        cache.retain(|_, v| now.duration_since(v.inserted_at).as_secs() < ttl_secs);
        if cache.len() >= WEB_SEARCH_CACHE_MAX_ENTRIES {
            if let Some(oldest_key) = cache.iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(key, CacheEntry { response, inserted_at: now });
    }
}

fn parse_ddg_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < max_results {
        let link_marker = "class=\"result__a\"";
        let link_start = match html[pos..].find(link_marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        let href_start = match html[..link_start].rfind("href=\"") {
            Some(idx) => idx + 6,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let href_end = match html[href_start..].find('"') {
            Some(idx) => href_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let raw_url = &html[href_start..href_end];
        let url = extract_ddg_url(raw_url);

        let title_start = match html[link_start..].find('>') {
            Some(idx) => link_start + idx + 1,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title_end = match html[title_start..].find("</a>") {
            Some(idx) => title_start + idx,
            None => {
                pos = link_start + link_marker.len();
                continue;
            }
        };
        let title = strip_html_tags(&html[title_start..title_end]);

        let snippet_marker = "class=\"result__snippet\"";
        let snippet = if let Some(snippet_start) = html[title_end..].find(snippet_marker) {
            let abs_snippet_start = title_end + snippet_start;
            if let Some(tag_end) = html[abs_snippet_start..].find('>') {
                let content_start = abs_snippet_start + tag_end + 1;
                // Try multiple end markers — DDG wraps snippets in <a> or <span>
                let end_pos = [
                    html[content_start..].find("</a>"),
                    html[content_start..].find("</span>"),
                    html[content_start..].find("</div>"),
                ].iter().filter_map(|x| *x).min().unwrap_or(0);
                if end_pos > 0 {
                    strip_html_tags(&html[content_start..content_start + end_pos])
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet: html_decode(&snippet),
            });
        }

        pos = title_end;
    }

    results
}

fn extract_ddg_url(raw: &str) -> String {
    if let Some(uddg_start) = raw.find("uddg=") {
        let url_start = uddg_start + 5;
        let url_end = raw[url_start..]
            .find('&')
            .map(|i| url_start + i)
            .unwrap_or(raw.len());
        let encoded = &raw[url_start..url_end];
        urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| encoded.to_string())
    } else if raw.starts_with("http") {
        raw.to_string()
    } else {
        raw.to_string()
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result.trim().to_string()
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

// ── Provider: DuckDuckGo ─────────────────────────────────────────

async fn search_duckduckgo(query: &str, count: usize, _timeout_secs: u64) -> Result<Vec<SearchResult>> {
    let client = build_ddg_client()?;

    // 1. Try Instant Answer API first (structured JSON, high quality for factual queries)
    let instant = ddg_instant_answer(&client, query).await;

    // 2. Scrape HTML search results, fallback to Lite endpoint
    let mut results = match ddg_html_search(&client, query, count).await {
        Ok(r) if !r.is_empty() => r,
        _ => ddg_lite_search(&client, query, count).await?,
    };

    // 3. Prepend instant answer if we got one and it's useful
    if let Some(ia) = instant {
        results.insert(0, ia);
    }

    // 4. Deduplicate by URL
    let mut seen = std::collections::HashSet::new();
    results.retain(|r| {
        if r.url.is_empty() { return true; }
        seen.insert(r.url.clone())
    });

    results.truncate(count);
    Ok(results)
}

/// Build a client with browser-like headers to avoid DDG bot detection.
fn build_ddg_client() -> Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, REFERER};
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"));
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(REFERER, HeaderValue::from_static("https://duckduckgo.com/"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));

    reqwest::Client::builder()
        .user_agent(DEFAULT_WEB_FETCH_USER_AGENT)
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(DEFAULT_WEB_SEARCH_TIMEOUT_SECS))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create DDG HTTP client: {}", e))
}

/// DuckDuckGo Instant Answer API — returns structured data for factual queries.
async fn ddg_instant_answer(client: &reqwest::Client, query: &str) -> Option<SearchResult> {
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        urlencoding::encode(query)
    );
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    let data: Value = resp.json().await.ok()?;

    // AbstractText + AbstractURL — encyclopedia-style answer
    let abstract_text = data.get("AbstractText").and_then(|v| v.as_str()).unwrap_or("");
    let abstract_url = data.get("AbstractURL").and_then(|v| v.as_str()).unwrap_or("");
    let abstract_source = data.get("AbstractSource").and_then(|v| v.as_str()).unwrap_or("");

    if !abstract_text.is_empty() && !abstract_url.is_empty() {
        return Some(SearchResult {
            title: format!("{} ({})", query, abstract_source),
            url: abstract_url.to_string(),
            snippet: abstract_text.chars().take(300).collect(),
        });
    }

    // Answer field — direct factual answer
    let answer = data.get("Answer").and_then(|v| v.as_str()).unwrap_or("");
    if !answer.is_empty() {
        return Some(SearchResult {
            title: format!("{} — Instant Answer", query),
            url: String::new(),
            snippet: answer.to_string(),
        });
    }

    None
}

/// Primary DDG search via the HTML endpoint.
async fn ddg_html_search(client: &reqwest::Client, query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let search_url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );
    let resp = client
        .post(&search_url)
        .form(&[("q", query), ("b", ""), ("kl", "")])
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("DuckDuckGo HTML request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("DuckDuckGo HTML failed with status: {}", resp.status()));
    }
    let html = resp.text().await
        .map_err(|e| anyhow::anyhow!("Failed to read DuckDuckGo response: {}", e))?;
    Ok(parse_ddg_results(&html, count))
}

/// Fallback: DDG Lite endpoint (simpler HTML, more resilient).
async fn ddg_lite_search(client: &reqwest::Client, query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let url = format!(
        "https://lite.duckduckgo.com/lite/?q={}",
        urlencoding::encode(query)
    );
    let resp = client.get(&url).send().await
        .map_err(|e| anyhow::anyhow!("DuckDuckGo Lite request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("DuckDuckGo Lite failed with status: {}", resp.status()));
    }
    let html = resp.text().await
        .map_err(|e| anyhow::anyhow!("Failed to read DDG Lite response: {}", e))?;
    Ok(parse_ddg_lite_results(&html, count))
}

/// Parse DDG Lite HTML (table-based layout, simpler structure).
fn parse_ddg_lite_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut pos = 0;

    // DDG Lite uses <a rel="nofollow" ...> for result links inside <td> with class "result-link"
    while results.len() < max_results {
        // Find next result link
        let marker = "class=\"result-link\"";
        let block_start = match html[pos..].find(marker) {
            Some(idx) => pos + idx,
            None => break,
        };

        // Extract href
        let href = if let Some(a_start) = html[block_start..].find("href=\"") {
            let abs_start = block_start + a_start + 6;
            if let Some(end) = html[abs_start..].find('"') {
                html[abs_start..abs_start + end].to_string()
            } else {
                pos = block_start + marker.len();
                continue;
            }
        } else {
            pos = block_start + marker.len();
            continue;
        };

        // Extract title (text inside the <a> tag)
        let title = if let Some(tag_end) = html[block_start..].find('>') {
            let content_start = block_start + tag_end + 1;
            if let Some(a_end) = html[content_start..].find("</a>") {
                strip_html_tags(&html[content_start..content_start + a_end])
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Find snippet in the next <td class="result-snippet">
        let snippet_marker = "class=\"result-snippet\"";
        let snippet = if let Some(snip_start) = html[block_start..].find(snippet_marker) {
            let abs_snip = block_start + snip_start;
            if let Some(tag_end) = html[abs_snip..].find('>') {
                let content_start = abs_snip + tag_end + 1;
                if let Some(td_end) = html[content_start..].find("</td>") {
                    html_decode(&strip_html_tags(&html[content_start..content_start + td_end]))
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let url = extract_ddg_url(&href);
        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_decode(&title),
                url,
                snippet,
            });
        }

        pos = block_start + marker.len();
    }

    results
}

// ── Provider: SearXNG ────────────────────────────────────────────

async fn search_searxng(instance_url: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    let client = build_search_client(timeout_secs)?;
    let mut url = format!(
        "{}/search?q={}&format=json&categories=general&pageno=1",
        instance_url.trim_end_matches('/'),
        urlencoding::encode(query)
    );
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&language={}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&time_range={}", urlencoding::encode(freshness)));
    }
    let resp = client.get(&url).send().await
        .map_err(|e| anyhow::anyhow!("SearXNG request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("SearXNG failed with status: {}", resp.status()));
    }
    let body: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("SearXNG JSON parse failed: {}", e))?;
    let results = body.get("results").and_then(|v| v.as_array());
    Ok(results.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect()
    }))
}

// ── Provider: Brave Search ───────────────────────────────────────

async fn search_brave(api_key: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Brave Search API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count
    );
    if let Some(ref country) = params.country {
        url.push_str(&format!("&country={}", urlencoding::encode(country)));
    }
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&search_lang={}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&freshness={}", brave_freshness(freshness)));
    }
    let resp = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Brave Search request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Brave Search failed ({}): {}", status, body));
    }
    let body: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Brave Search JSON parse failed: {}", e))?;
    let web = body.get("web").and_then(|w| w.get("results")).and_then(|v| v.as_array());
    Ok(web.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect()
    }))
}

// ── Provider: Perplexity ─────────────────────────────────────────

async fn search_perplexity(api_key: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Perplexity API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut body = serde_json::json!({
        "model": "sonar",
        "messages": [{"role": "user", "content": query}],
        "max_tokens": 1024,
        "return_citations": true
    });
    if let Some(ref freshness) = params.freshness {
        body["search_recency_filter"] = serde_json::Value::String(freshness.clone());
    }
    let resp = client
        .post("https://api.perplexity.ai/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Perplexity request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Perplexity failed ({}): {}", status, text));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Perplexity JSON parse failed: {}", e))?;

    // Extract citations as search results
    let citations = data.get("citations").and_then(|v| v.as_array());
    let content = data
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut results: Vec<SearchResult> = citations.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|c| {
                let url = c.as_str()?.to_string();
                // Extract domain as title fallback
                let title = url.split('/').nth(2).unwrap_or(&url).to_string();
                Some(SearchResult { title, url, snippet: String::new() })
            })
            .collect()
    });

    // If we got a summary but no citations, return the summary as a single result
    if results.is_empty() && !content.is_empty() {
        results.push(SearchResult {
            title: "Perplexity Summary".into(),
            url: String::new(),
            snippet: content.chars().take(500).collect(),
        });
    }

    Ok(results)
}

// ── Provider: Google Custom Search ───────────────────────────────

async fn search_google(api_key: &str, cx: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() || cx.is_empty() {
        return Err(anyhow::anyhow!("Google Custom Search API key or CX not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut url = format!(
        "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&num={}",
        urlencoding::encode(api_key),
        urlencoding::encode(cx),
        urlencoding::encode(query),
        count.min(10)
    );
    if let Some(ref country) = params.country {
        url.push_str(&format!("&gl={}", urlencoding::encode(country)));
    }
    if let Some(ref lang) = params.language {
        url.push_str(&format!("&lr=lang_{}", urlencoding::encode(lang)));
    }
    if let Some(ref freshness) = params.freshness {
        url.push_str(&format!("&dateRestrict={}", google_date_restrict(freshness)));
    }
    let resp = client.get(&url).send().await
        .map_err(|e| anyhow::anyhow!("Google Custom Search request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Google Custom Search failed ({}): {}", status, text));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Google Custom Search JSON parse failed: {}", e))?;
    let items = data.get("items").and_then(|v| v.as_array());
    Ok(items.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("link")?.as_str()?.to_string();
                let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect()
    }))
}

// ── Provider: Grok (X.AI) ───────────────────────────────────────

async fn search_grok(api_key: &str, query: &str, count: usize, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Grok (X.AI) API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let body = serde_json::json!({
        "model": "grok-3-mini-fast",
        "messages": [{"role": "user", "content": format!(
            "Search the web for: {}. Return exactly {} results as JSON array with fields: title, url, snippet. Only return the JSON array, no other text.",
            query, count
        )}],
        "search_parameters": {"mode": "auto"}
    });
    let resp = client
        .post("https://api.x.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Grok request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Grok failed ({}): {}", status, text));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Grok JSON parse failed: {}", e))?;

    // Extract search results from response
    let mut results = Vec::new();

    // Try to parse citations/search_results from the response
    if let Some(search_results) = data.get("search_results").and_then(|v| v.as_array()) {
        for item in search_results.iter().take(count) {
            if let (Some(title), Some(url)) = (
                item.get("title").and_then(|v| v.as_str()),
                item.get("url").and_then(|v| v.as_str()),
            ) {
                results.push(SearchResult {
                    title: title.to_string(),
                    url: url.to_string(),
                    snippet: item.get("snippet").or(item.get("description"))
                        .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
            }
        }
    }

    // Fallback: parse model content as JSON array
    if results.is_empty() {
        if let Some(content) = data.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
        {
            // Try to find JSON array in the content
            if let Some(start) = content.find('[') {
                if let Some(end) = content.rfind(']') {
                    if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&content[start..=end]) {
                        for item in arr.iter().take(count) {
                            if let (Some(title), Some(url)) = (
                                item.get("title").and_then(|v| v.as_str()),
                                item.get("url").and_then(|v| v.as_str()),
                            ) {
                                results.push(SearchResult {
                                    title: title.to_string(),
                                    url: url.to_string(),
                                    snippet: item.get("snippet")
                                        .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                });
                            }
                        }
                    }
                }
            }
            // If still empty, return content as a single result
            if results.is_empty() && !content.is_empty() {
                results.push(SearchResult {
                    title: "Grok Summary".into(),
                    url: String::new(),
                    snippet: content.chars().take(500).collect(),
                });
            }
        }
    }

    Ok(results)
}

// ── Provider: Kimi (Moonshot) ────────────────────────────────────

async fn search_kimi(api_key: &str, query: &str, count: usize, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Kimi (Moonshot) API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let body = serde_json::json!({
        "model": "moonshot-v1-8k",
        "messages": [{"role": "user", "content": format!(
            "Search the web for: {}. Return exactly {} results as JSON array with fields: title, url, snippet. Only return the JSON array, no other text.",
            query, count
        )}],
        "tools": [{"type": "builtin_function", "function": {"name": "web_search"}}]
    });
    let resp = client
        .post("https://api.moonshot.cn/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Kimi request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Kimi failed ({}): {}", status, text));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Kimi JSON parse failed: {}", e))?;

    let mut results = Vec::new();

    // Extract search results from Kimi's response
    if let Some(content) = data.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
    {
        // Try to parse as JSON array
        if let Some(start) = content.find('[') {
            if let Some(end) = content.rfind(']') {
                if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&content[start..=end]) {
                    for item in arr.iter().take(count) {
                        if let (Some(title), Some(url)) = (
                            item.get("title").and_then(|v| v.as_str()),
                            item.get("url").and_then(|v| v.as_str()),
                        ) {
                            results.push(SearchResult {
                                title: title.to_string(),
                                url: url.to_string(),
                                snippet: item.get("snippet")
                                    .and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            });
                        }
                    }
                }
            }
        }
        if results.is_empty() && !content.is_empty() {
            results.push(SearchResult {
                title: "Kimi Summary".into(),
                url: String::new(),
                snippet: content.chars().take(500).collect(),
            });
        }
    }

    Ok(results)
}

// ── Provider: Tavily ────────────────────────────────────────────

async fn search_tavily(api_key: &str, query: &str, count: usize, params: &SearchParams, timeout_secs: u64) -> Result<Vec<SearchResult>> {
    if api_key.is_empty() {
        return Err(anyhow::anyhow!("Tavily API key not configured"));
    }
    let client = build_search_client(timeout_secs)?;
    let mut body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "max_results": count,
        "include_answer": false,
    });
    if let Some(ref country) = params.country {
        body["country"] = serde_json::Value::String(country.clone());
    }
    if let Some(ref freshness) = params.freshness {
        body["days"] = serde_json::Value::Number(serde_json::Number::from(tavily_days(freshness)));
    }
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Tavily request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Tavily search failed ({}): {}", status, body));
    }
    let data: Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("Tavily JSON parse failed: {}", e))?;
    let results = data.get("results").and_then(|v| v.as_array());
    Ok(results.map_or_else(Vec::new, |arr| {
        arr.iter()
            .take(count)
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?.to_string();
                let url = item.get("url")?.as_str()?.to_string();
                let snippet = item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .collect()
    }))
}

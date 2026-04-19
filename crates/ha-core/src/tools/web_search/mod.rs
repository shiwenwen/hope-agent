use anyhow::Result;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

mod brave;
mod duckduckgo;
mod google;
mod grok;
mod helpers;
mod kimi;
mod perplexity;
mod searxng;
mod tavily;

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
    /// Whether Docker-managed SearXNG should use the app/system proxy for its
    /// outgoing requests. Disable this when host-level VPN routing is enough.
    #[serde(default = "default_searxng_docker_use_proxy")]
    pub searxng_docker_use_proxy: bool,
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
        WebSearchProviderEntry {
            id: WebSearchProvider::DuckDuckGo,
            enabled: true,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Searxng,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Brave,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Perplexity,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Google,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Grok,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Kimi,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Tavily,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
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

fn default_ws_result_count() -> usize {
    DEFAULT_WEB_SEARCH_RESULT_COUNT
}
fn default_searxng_docker_use_proxy() -> bool {
    true
}
fn default_ws_timeout_secs() -> u64 {
    DEFAULT_WEB_SEARCH_TIMEOUT_SECS
}
fn default_ws_cache_ttl() -> u64 {
    DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            providers: default_providers(),
            searxng_docker_managed: None,
            searxng_docker_use_proxy: default_searxng_docker_use_proxy(),
            default_result_count: DEFAULT_WEB_SEARCH_RESULT_COUNT,
            timeout_seconds: DEFAULT_WEB_SEARCH_TIMEOUT_SECS,
            cache_ttl_minutes: DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES,
            default_country: None,
            default_language: None,
            default_freshness: None,
        }
    }
}

/// Check if any web search provider is enabled in the config.
pub fn has_enabled_provider(config: &WebSearchConfig) -> bool {
    config.providers.iter().any(|p| p.enabled)
}

/// Collect all enabled providers in order. Falls back to DuckDuckGo if none enabled.
fn resolve_providers(config: &WebSearchConfig) -> Vec<&WebSearchProviderEntry> {
    let enabled: Vec<&WebSearchProviderEntry> =
        config.providers.iter().filter(|e| e.enabled).collect();
    if enabled.is_empty() {
        static DDG_FALLBACK: std::sync::LazyLock<WebSearchProviderEntry> =
            std::sync::LazyLock::new(|| WebSearchProviderEntry {
                id: WebSearchProvider::DuckDuckGo,
                enabled: true,
                api_key: None,
                api_key2: None,
                base_url: None,
            });
        vec![&DDG_FALLBACK]
    } else {
        enabled
    }
}

// ── Tool Entry Point ─────────────────────────────────────────────

pub(crate) async fn tool_web_search(args: &Value) -> Result<String> {
    let config = crate::config::cached_config().web_search.clone();

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
        country: args
            .get("country")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_country.clone()),
        language: args
            .get("language")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_language.clone()),
        freshness: args
            .get("freshness")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| config.default_freshness.clone()),
    };

    let providers = resolve_providers(&config);
    let timeout = config.timeout_seconds;

    // Try each enabled provider in order; fallback to next on error or 0 results
    let mut results = Vec::new();
    let mut used_provider = String::new();
    let mut last_error: Option<anyhow::Error> = None;

    for entry in &providers {
        let provider_id = &entry.id;

        app_info!(
            "tool",
            "web_search",
            "Web search [{}]: {} (count: {}, country: {:?}, lang: {:?}, freshness: {:?})",
            provider_id,
            query,
            count,
            params.country,
            params.language,
            params.freshness
        );

        // Check cache
        let ck = search_cache_key(&provider_id.to_string(), query, count, &params);
        if let Some(cached) = read_search_cache(&ck, config.cache_ttl_minutes) {
            app_info!(
                "tool",
                "web_search",
                "Cache hit for [{}]: {}",
                provider_id,
                query
            );
            return Ok(cached);
        }

        let attempt = match provider_id {
            WebSearchProvider::DuckDuckGo => {
                duckduckgo::search_duckduckgo(query, count, timeout).await
            }
            WebSearchProvider::Searxng => {
                let url = entry.base_url.as_deref().unwrap_or("http://127.0.0.1:8080");
                // SearXNG instance URL is user-configurable; gate it through the
                // SSRF policy so a malicious or mis-typed value can't target
                // cloud metadata endpoints or non-loopback private ranges. `?`
                // inside an async block so SSRF failure flows into `attempt` and
                // the outer fallback loop can try the next provider.
                async {
                    helpers::check_search_url(url).await?;
                    searxng::search_searxng(url, query, count, &params, timeout).await
                }
                .await
            }
            WebSearchProvider::Brave => {
                let key = entry.api_key.as_deref().unwrap_or("");
                brave::search_brave(key, query, count, &params, timeout).await
            }
            WebSearchProvider::Perplexity => {
                let key = entry.api_key.as_deref().unwrap_or("");
                perplexity::search_perplexity(key, query, count, &params, timeout).await
            }
            WebSearchProvider::Google => {
                let key = entry.api_key.as_deref().unwrap_or("");
                let cx = entry.api_key2.as_deref().unwrap_or("");
                google::search_google(key, cx, query, count, &params, timeout).await
            }
            WebSearchProvider::Grok => {
                let key = entry.api_key.as_deref().unwrap_or("");
                grok::search_grok(key, query, count, timeout).await
            }
            WebSearchProvider::Kimi => {
                let key = entry.api_key.as_deref().unwrap_or("");
                kimi::search_kimi(key, query, count, timeout).await
            }
            WebSearchProvider::Tavily => {
                let key = entry.api_key.as_deref().unwrap_or("");
                tavily::search_tavily(key, query, count, &params, timeout).await
            }
        };

        match attempt {
            Ok(r) if !r.is_empty() => {
                used_provider = provider_id.to_string();
                results = r;
                break;
            }
            Ok(_) => {
                app_warn!(
                    "tool",
                    "web_search",
                    "Provider [{}] returned 0 results for '{}', trying next provider",
                    provider_id,
                    query
                );
            }
            Err(e) => {
                app_warn!(
                    "tool",
                    "web_search",
                    "Provider [{}] error for '{}': {}, trying next provider",
                    provider_id,
                    query,
                    e
                );
                last_error = Some(e);
            }
        }
    }

    if results.is_empty() {
        if let Some(e) = last_error {
            app_warn!(
                "tool",
                "web_search",
                "All providers failed for '{}', last error: {}",
                query,
                e
            );
        }
        return Ok(format!("No results found for: {}", query));
    }

    let mut output = format!("Search results for: {} (via {})\n\n", query, used_provider);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   Source: {}\n   {}\n\n",
            i + 1,
            result.title,
            result.url,
            result.source,
            result.snippet
        ));
    }

    // Write to cache
    let ck = search_cache_key(&used_provider, query, count, &params);
    write_search_cache(ck, output.clone(), config.cache_ttl_minutes);

    Ok(output)
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    /// Which search engine/provider produced this result
    source: String,
}

// ── Search Params & Helpers ─────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct SearchParams {
    country: Option<String>,
    language: Option<String>,
    freshness: Option<String>,
}

// ── Search Result Cache ─────────────────────────────────────────

struct CacheEntry {
    response: String,
    inserted_at: Instant,
}

static WEB_SEARCH_CACHE: Lazy<Mutex<HashMap<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn search_cache_key(provider: &str, query: &str, count: usize, params: &SearchParams) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        provider,
        query.to_lowercase().trim(),
        count,
        params.country.as_deref().unwrap_or(""),
        params.language.as_deref().unwrap_or(""),
        params.freshness.as_deref().unwrap_or(""),
    )
}

fn read_search_cache(key: &str, ttl_minutes: u64) -> Option<String> {
    if ttl_minutes == 0 {
        return None;
    }
    let cache = WEB_SEARCH_CACHE.lock().ok()?;
    let entry = cache.get(key)?;
    if entry.inserted_at.elapsed().as_secs() < ttl_minutes * 60 {
        Some(entry.response.clone())
    } else {
        None
    }
}

fn write_search_cache(key: String, response: String, ttl_minutes: u64) {
    if ttl_minutes == 0 {
        return;
    }
    if let Ok(mut cache) = WEB_SEARCH_CACHE.lock() {
        let now = Instant::now();
        let ttl_secs = ttl_minutes * 60;
        cache.retain(|_, v| now.duration_since(v.inserted_at).as_secs() < ttl_secs);
        if cache.len() >= WEB_SEARCH_CACHE_MAX_ENTRIES {
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

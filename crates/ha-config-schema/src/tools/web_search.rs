//! Web search configuration (`AppConfig.web_search`).

use serde::{Deserialize, Serialize};

const DEFAULT_WEB_SEARCH_RESULT_COUNT: usize = 5;
/// 可见性升级：ha-core `tools::web_search::duckduckgo` 仍以 `super::` 引用。
pub const DEFAULT_WEB_SEARCH_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_SEARCH_CACHE_TTL_MINUTES: u64 = 15;

// ── Web Search Provider Config ───────────────────────────────────

/// Supported web search providers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WebSearchProvider {
    /// DuckDuckGo HTML scraping — free, no API key
    DuckDuckGo,
    /// SearXNG self-hosted meta-search — free, needs instance URL
    Searxng,
    /// Bocha AI Search API — requires API key
    Bocha,
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
            Self::Bocha => write!(f, "Bocha"),
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
    /// API key (Bocha / Brave / Perplexity / Google / Grok / Kimi)
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

/// 可见性升级：ha-core `tools::web_search::backfill_providers` 仍要调用。
pub fn default_providers() -> Vec<WebSearchProviderEntry> {
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
            id: WebSearchProvider::Tavily,
            enabled: false,
            api_key: None,
            api_key2: None,
            base_url: None,
        },
        WebSearchProviderEntry {
            id: WebSearchProvider::Bocha,
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
    ]
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

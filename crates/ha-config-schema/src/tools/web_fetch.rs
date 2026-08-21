//! Web fetch configuration (`AppConfig.web_fetch`).

use serde::{Deserialize, Serialize};

const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const DEFAULT_WEB_FETCH_MAX_CHARS: usize = 50000;
const DEFAULT_WEB_FETCH_MAX_CHARS_CAP: usize = 200000;
const DEFAULT_WEB_FETCH_MAX_RESPONSE_BYTES: usize = 2_097_152; // 2 MB
const DEFAULT_WEB_FETCH_MAX_REDIRECTS: usize = 5;
const DEFAULT_WEB_FETCH_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_FETCH_CACHE_TTL_MINUTES: u64 = 15;
const DEFAULT_WEB_FETCH_MAX_OUTPUT_TOKENS_CAP: usize = 32_768;
const DEFAULT_WEB_FETCH_RENDER_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_FETCH_CACHE_MAX_ENTRIES: usize = 100;
const DEFAULT_WEB_FETCH_MAX_CONCURRENT_PER_HOST: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebFetchRenderMode {
    #[default]
    Never,
    Auto,
    Always,
}

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
    /// Default dynamic-render behavior for tool calls that omit `render`.
    #[serde(default)]
    pub default_render_mode: WebFetchRenderMode,
    /// Hard cap on the optional model-facing `max_tokens` projection.
    #[serde(default = "default_wf_max_output_tokens_cap")]
    pub max_output_tokens_cap: usize,
    /// Browser-render timeout in seconds.
    #[serde(default = "default_wf_render_timeout_seconds")]
    pub render_timeout_seconds: u64,
    /// Maximum process-memory snapshots retained by web_fetch.
    #[serde(default = "default_wf_cache_max_entries")]
    pub cache_max_entries: usize,
    /// Maximum simultaneous direct requests per origin.
    #[serde(default = "default_wf_max_concurrent_per_host")]
    pub max_concurrent_per_host: usize,
    /// Minimum delay between direct requests to the same origin.
    #[serde(default)]
    pub min_host_delay_ms: u64,
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
fn default_wf_max_output_tokens_cap() -> usize {
    DEFAULT_WEB_FETCH_MAX_OUTPUT_TOKENS_CAP
}
fn default_wf_render_timeout_seconds() -> u64 {
    DEFAULT_WEB_FETCH_RENDER_TIMEOUT_SECS
}
fn default_wf_cache_max_entries() -> usize {
    DEFAULT_WEB_FETCH_CACHE_MAX_ENTRIES
}
fn default_wf_max_concurrent_per_host() -> usize {
    DEFAULT_WEB_FETCH_MAX_CONCURRENT_PER_HOST
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
            default_render_mode: WebFetchRenderMode::Never,
            max_output_tokens_cap: DEFAULT_WEB_FETCH_MAX_OUTPUT_TOKENS_CAP,
            render_timeout_seconds: DEFAULT_WEB_FETCH_RENDER_TIMEOUT_SECS,
            cache_max_entries: DEFAULT_WEB_FETCH_CACHE_MAX_ENTRIES,
            max_concurrent_per_host: DEFAULT_WEB_FETCH_MAX_CONCURRENT_PER_HOST,
            min_host_delay_ms: 0,
        }
    }
}

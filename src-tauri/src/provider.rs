use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;

// ── API Type ──────────────────────────────────────────────────────

/// Supported API protocol types for LLM providers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApiType {
    /// Anthropic Messages API (/v1/messages)
    Anthropic,
    /// OpenAI Chat Completions API (/v1/chat/completions)
    OpenaiChat,
    /// OpenAI Responses API (/v1/responses or Codex endpoint)
    OpenaiResponses,
    /// Built-in Codex OAuth (ChatGPT subscription)
    Codex,
}

impl ApiType {
    /// Returns the default base URL for this API type
    pub fn default_base_url(&self) -> &str {
        match self {
            ApiType::Anthropic => "https://api.anthropic.com",
            ApiType::OpenaiChat => "https://api.openai.com",
            ApiType::OpenaiResponses => "https://api.openai.com",
            ApiType::Codex => "https://chatgpt.com/backend-api/codex",
        }
    }

    /// Display name for UI
    #[allow(dead_code)]
    pub fn display_name(&self) -> &str {
        match self {
            ApiType::Anthropic => "Anthropic",
            ApiType::OpenaiChat => "OpenAI Chat Completions",
            ApiType::OpenaiResponses => "OpenAI Responses",
            ApiType::Codex => "OpenAI Codex (OAuth)",
        }
    }
}

// ── Thinking Style ────────────────────────────────────────────────

/// Thinking/reasoning parameter format for different LLM providers.
/// Controls how the "thinking" capability is communicated to the API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingStyle {
    /// OpenAI format: `reasoning_effort: "low"/"medium"/"high"`
    Openai,
    /// Anthropic format: `thinking: { type: "enabled", budget_tokens: N }`
    Anthropic,
    /// Z.AI format: same as Anthropic (reserved for future differentiation)
    Zai,
    /// Qwen/DashScope format: `enable_thinking: true`
    Qwen,
    /// Do not send any thinking/reasoning parameters
    None,
}

impl Default for ThinkingStyle {
    fn default() -> Self {
        ThinkingStyle::Openai
    }
}

// ── Model Config ──────────────────────────────────────────────────

/// Configuration for a single model within a provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    /// Model identifier, e.g. "claude-sonnet-4-6", "gpt-5.4"
    pub id: String,
    /// Display name, e.g. "Claude Sonnet 4.6"
    pub name: String,
    /// Supported input types: "text", "image", "video"
    #[serde(default = "default_input_types")]
    pub input_types: Vec<String>,
    /// Context window size in tokens
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    /// Maximum output tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Whether the model supports reasoning/thinking
    #[serde(default)]
    pub reasoning: bool,
    /// Input cost per million tokens (USD)
    #[serde(default)]
    pub cost_input: f64,
    /// Output cost per million tokens (USD)
    #[serde(default)]
    pub cost_output: f64,
}

fn default_input_types() -> Vec<String> {
    vec!["text".to_string()]
}

fn default_context_window() -> u32 {
    200_000
}

fn default_max_tokens() -> u32 {
    8192
}

// ── Provider Config ───────────────────────────────────────────────

/// Configuration for a model provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Unique provider ID (UUID)
    pub id: String,
    /// User-defined display name, e.g. "My Anthropic"
    pub name: String,
    /// API protocol type
    pub api_type: ApiType,
    /// Base URL for API calls
    pub base_url: String,
    /// API key (empty for Codex OAuth)
    #[serde(default)]
    pub api_key: String,
    /// List of models available from this provider
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    /// Whether this provider is enabled
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Custom User-Agent header for API requests
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    /// Thinking/reasoning parameter format
    #[serde(default)]
    pub thinking_style: ThinkingStyle,
}

fn default_user_agent() -> String {
    "claude-code/0.1.0".to_string()
}

impl ProviderConfig {
    /// Create a new provider with a generated UUID
    pub fn new(name: String, api_type: ApiType, base_url: String, api_key: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            api_type,
            base_url,
            api_key,
            models: Vec::new(),
            enabled: true,
            user_agent: default_user_agent(),
            thinking_style: ThinkingStyle::default(),
        }
    }

    /// Return a copy with the API key masked for frontend display
    pub fn masked(&self) -> Self {
        let masked_key = if self.api_key.chars().count() > 8 {
            let prefix: String = self.api_key.chars().take(4).collect();
            let suffix: String = self
                .api_key
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            format!("{}...{}", prefix, suffix)
        } else if !self.api_key.is_empty() {
            "****".to_string()
        } else {
            String::new()
        };
        Self {
            api_key: masked_key,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiType, ProviderConfig};

    #[test]
    fn masked_api_key_keeps_utf8_boundaries() {
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenAI,
            "https://api.openai.com".to_string(),
            "密钥🔑abcdef".to_string(),
        );
        let masked = cfg.masked();
        assert!(masked.api_key.contains("..."));
        assert_ne!(masked.api_key, cfg.api_key);
    }
}

// ── Active Model ──────────────────────────────────────────────────

/// Represents the currently active model selection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub provider_id: String,
    pub model_id: String,
}

// ── Serializable Store ────────────────────────────────────────────

// ── Proxy Config ────────────────────────────────────────────────

/// Proxy mode for all HTTP requests in the application
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// Use system proxy (environment variables HTTP_PROXY/HTTPS_PROXY/ALL_PROXY)
    System,
    /// No proxy – direct connection
    None,
    /// Custom proxy URL
    Custom,
}

impl Default for ProxyMode {
    fn default() -> Self {
        Self::System
    }
}

/// Global proxy configuration for all outgoing HTTP requests
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    /// Proxy mode: "system" (default), "none", or "custom"
    #[serde(default)]
    pub mode: ProxyMode,
    /// Custom proxy URL (only used when mode is "custom"), e.g. "http://127.0.0.1:7890"
    #[serde(default)]
    pub url: Option<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            mode: ProxyMode::default(),
            url: None,
        }
    }
}

/// Load global proxy config once.
pub fn load_proxy_config() -> ProxyConfig {
    load_store().map(|s| s.proxy).unwrap_or_default()
}

/// Apply proxy settings to a reqwest async ClientBuilder based on global config.
pub fn apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    apply_proxy_from_config(builder, &load_proxy_config())
}

/// Apply proxy settings for a specific target URL.
/// Loopback destinations should always bypass the global proxy, otherwise local
/// services like Docker-managed SearXNG or Chrome CDP can be routed into the
/// system proxy and fail unexpectedly.
pub fn apply_proxy_for_url(
    builder: reqwest::ClientBuilder,
    target_url: &str,
) -> reqwest::ClientBuilder {
    if should_bypass_proxy(target_url) {
        builder.no_proxy()
    } else {
        apply_proxy(builder)
    }
}

/// Apply proxy settings from a specific ProxyConfig (async builder).
pub fn apply_proxy_from_config(
    mut builder: reqwest::ClientBuilder,
    config: &ProxyConfig,
) -> reqwest::ClientBuilder {
    match config.mode {
        ProxyMode::System => {
            // reqwest default: reads HTTP_PROXY / HTTPS_PROXY / ALL_PROXY env vars.
            // On macOS, apps like Shadowrocket/ClashX set system proxy via Network
            // Preferences but NOT env vars. Detect and apply if env vars are empty.
            let has_env_proxy = [
                "HTTPS_PROXY",
                "HTTP_PROXY",
                "ALL_PROXY",
                "https_proxy",
                "http_proxy",
                "all_proxy",
            ]
            .iter()
            .any(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).is_some());
            if !has_env_proxy {
                if let Some(url) = detect_system_proxy() {
                    if let Ok(proxy) = reqwest::Proxy::all(&url) {
                        builder = builder.proxy(proxy);
                    }
                }
            }
        }
        ProxyMode::None => {
            builder = builder.no_proxy();
        }
        ProxyMode::Custom => {
            if let Some(ref url) = config.url {
                if !url.is_empty() {
                    if let Ok(proxy) = reqwest::Proxy::all(url) {
                        builder = builder.proxy(proxy);
                    }
                }
            }
        }
    }
    builder
}

/// Detect macOS system proxy via `scutil --proxy`.
/// Returns e.g. `Some("http://127.0.0.1:1082")`.
#[cfg(target_os = "macos")]
fn detect_system_proxy() -> Option<String> {
    use std::sync::OnceLock;
    // Cache the result — scutil is a subprocess call, avoid repeated invocations
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let output = std::process::Command::new("scutil")
                .arg("--proxy")
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&output.stdout);
            for prefix in ["HTTPS", "HTTP"] {
                let enabled = text
                    .lines()
                    .find(|l| l.trim().starts_with(&format!("{}Enable", prefix)))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|v| v.trim() == "1")
                    .unwrap_or(false);
                if !enabled {
                    continue;
                }
                let host = text
                    .lines()
                    .find(|l| {
                        let t = l.trim();
                        t.starts_with(&format!("{}Proxy", prefix))
                            && !t.contains("Enable")
                            && !t.contains("Port")
                    })
                    .and_then(|l| l.split(':').nth(1))
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty());
                let port = text
                    .lines()
                    .find(|l| l.trim().starts_with(&format!("{}Port", prefix)))
                    .and_then(|l| l.split(':').nth(1))
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty());
                if let (Some(h), Some(p)) = (host, port) {
                    return Some(format!("http://{}:{}", h, p));
                }
            }
            None
        })
        .clone()
}

#[cfg(not(target_os = "macos"))]
fn detect_system_proxy() -> Option<String> {
    None
}

fn should_bypass_proxy(target_url: &str) -> bool {
    let Ok(url) = url::Url::parse(target_url) else {
        return false;
    };

    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        None => false,
    }
}

/// Apply proxy settings to a reqwest blocking ClientBuilder based on global config.
pub fn apply_proxy_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    let config = load_proxy_config();
    match config.mode {
        ProxyMode::System => {
            let has_env_proxy = [
                "HTTPS_PROXY",
                "HTTP_PROXY",
                "ALL_PROXY",
                "https_proxy",
                "http_proxy",
                "all_proxy",
            ]
            .iter()
            .any(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).is_some());
            if !has_env_proxy {
                if let Some(url) = detect_system_proxy() {
                    if let Ok(proxy) = reqwest::Proxy::all(&url) {
                        return builder.proxy(proxy);
                    }
                }
            }
            builder
        }
        ProxyMode::None => builder.no_proxy(),
        ProxyMode::Custom => {
            if let Some(ref url) = config.url {
                if !url.is_empty() {
                    if let Ok(proxy) = reqwest::Proxy::all(url) {
                        return builder.proxy(proxy);
                    }
                }
            }
            builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_bypass_proxy;

    #[test]
    fn loopback_hosts_bypass_proxy() {
        assert!(should_bypass_proxy("http://localhost:8080/search?q=test"));
        assert!(should_bypass_proxy("http://127.0.0.1:8080/search?q=test"));
        assert!(should_bypass_proxy("http://[::1]:9222/json/version"));
    }

    #[test]
    fn remote_hosts_keep_proxy() {
        assert!(!should_bypass_proxy("https://duckduckgo.com/?q=test"));
        assert!(!should_bypass_proxy("http://192.168.1.10:8080"));
        assert!(!should_bypass_proxy("not-a-url"));
    }
}

// ── Shortcut Config ─────────────────────────────────────────────

/// A single keyboard shortcut binding
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    /// Unique identifier for this shortcut action
    pub id: String,
    /// The shortcut key combination (e.g. "Alt+Space", "CommandOrControl+Shift+K")
    /// Empty string means disabled.
    pub keys: String,
    /// Whether this shortcut is enabled
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

/// Global shortcut configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutConfig {
    /// All shortcut bindings
    #[serde(default = "default_shortcut_bindings")]
    pub bindings: Vec<ShortcutBinding>,
}

fn default_shortcut_bindings() -> Vec<ShortcutBinding> {
    vec![ShortcutBinding {
        id: "quickChat".to_string(),
        keys: "Alt+Space".to_string(),
        enabled: true,
    }]
}

impl ShortcutBinding {
    /// Whether this binding is a chord (two sequential key combos separated by space).
    /// e.g. "CommandOrControl+K CommandOrControl+C"
    pub fn is_chord(&self) -> bool {
        self.chord_parts().len() > 1
    }

    /// Split keys into chord parts. Single combo returns vec of 1.
    pub fn chord_parts(&self) -> Vec<&str> {
        self.keys.split_whitespace().collect()
    }
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            bindings: default_shortcut_bindings(),
        }
    }
}

// ── Notification Config ─────────────────────────────────────────

/// Global notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfig {
    /// Global on/off toggle (default: true)
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Root structure for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStore {
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub active_model: Option<ActiveModel>,
    /// Global fallback model chain (ordered).
    /// When the primary model fails, these are tried in order.
    #[serde(default)]
    pub fallback_models: Vec<ActiveModel>,
    /// Extra directories to scan for skills
    #[serde(default)]
    pub extra_skills_dirs: Vec<String>,
    /// Disabled skill names
    #[serde(default)]
    pub disabled_skills: Vec<String>,
    /// Whether to check skill runtime requirements (bins/env/os) before injecting.
    /// Default true. When false, all skills are injected regardless of environment.
    #[serde(default = "default_skill_env_check")]
    pub skill_env_check: bool,
    /// Embedding model configuration for memory vector search
    #[serde(default)]
    pub embedding: crate::memory::EmbeddingConfig,
    /// Web search provider configuration
    #[serde(default)]
    pub web_search: crate::tools::web_search::WebSearchConfig,
    /// Web fetch tool configuration
    #[serde(default)]
    pub web_fetch: crate::tools::web_fetch::WebFetchConfig,
    /// Per-skill environment variable overrides configured by user.
    /// Outer key: skill name, inner key: env var name, value: env var value.
    #[serde(default)]
    pub skill_env: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Global memory auto-extract configuration
    #[serde(default)]
    pub memory_extract: crate::memory::MemoryExtractConfig,
    /// LLM-based memory selection configuration
    #[serde(default)]
    pub memory_selection: crate::memory::MemorySelectionConfig,
    /// Memory deduplication thresholds
    #[serde(default)]
    pub dedup: crate::memory::DedupConfig,
    /// Hybrid search weight configuration
    #[serde(default)]
    pub hybrid_search: crate::memory::HybridSearchConfig,
    /// Temporal decay configuration for memory search
    #[serde(default)]
    pub temporal_decay: crate::memory::TemporalDecayConfig,
    /// MMR reranking configuration
    #[serde(default)]
    pub mmr: crate::memory::MmrConfig,
    /// Multimodal embedding configuration (image/audio)
    #[serde(default)]
    pub multimodal: crate::memory::MultimodalConfig,
    /// Embedding cache configuration
    #[serde(default)]
    pub embedding_cache: crate::memory::EmbeddingCacheConfig,
    /// Context compaction configuration
    #[serde(default)]
    pub compact: crate::context_compact::CompactConfig,
    /// Notification configuration
    #[serde(default)]
    pub notification: NotificationConfig,
    /// Image generation configuration
    #[serde(default)]
    pub image_generate: crate::tools::image_generate::ImageGenConfig,
    /// Canvas tool configuration
    #[serde(default)]
    pub canvas: crate::tools::canvas::CanvasConfig,
    /// Image tool configuration (max images per call, etc.)
    #[serde(default)]
    pub image: crate::tools::image::ImageToolConfig,
    /// PDF tool configuration (max PDFs, max vision pages, etc.)
    #[serde(default)]
    pub pdf: crate::tools::pdf::PdfToolConfig,
    /// Global hard timeout (seconds) for a single tool execution.
    /// Safety net for when inner tool timeouts don't fire (network issues, etc.).
    /// Default 300 (5 min). Set to 0 to disable.
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout: u64,
    /// Threshold (bytes) for persisting large tool results to disk.
    /// Results exceeding this size are written to disk with a preview in context.
    /// Default: 50000 (50KB). Set to 0 to disable.
    #[serde(default)]
    pub tool_result_disk_threshold: Option<usize>,
    /// UI theme preference: "auto" | "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// UI language preference: "auto" means follow system, otherwise a locale code like "zh", "en"
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether UI background effects (stars, weather) are enabled
    #[serde(default = "crate::default_true")]
    pub ui_effects_enabled: bool,
    /// Global proxy configuration for all outgoing HTTP requests
    #[serde(default)]
    pub proxy: ProxyConfig,
    /// Configurable limits for skill prompt generation
    #[serde(default)]
    pub skill_prompt_budget: crate::skills::SkillPromptBudget,
    /// Bundled skills allowlist (empty = all allowed)
    #[serde(default)]
    pub skill_allow_bundled: Vec<String>,

    /// ACP control plane configuration (external agent management)
    #[serde(default)]
    pub acp_control: crate::acp_control::AcpControlConfig,

    /// Global keyboard shortcut configuration
    #[serde(default)]
    pub shortcuts: ShortcutConfig,

    /// Custom plans directory override. When set, plans are stored here instead of
    /// the default `~/.opencomputer/plans/`.
    #[serde(default)]
    pub plans_directory: Option<String>,

    /// Global default temperature for LLM API calls (0.0–2.0).
    /// Can be overridden at the agent level or session level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Whether to use a dedicated sub-agent for plan creation (Planning phase).
    /// When true, planning runs in an isolated sub-agent (saves main agent context).
    /// When false, planning runs inline in the main agent (preserves context continuity).
    /// Default: false (inline mode)
    #[serde(default)]
    pub plan_subagent: bool,

    /// IM channel configuration (Telegram, Discord, Slack, etc.)
    #[serde(default)]
    pub channels: crate::channel::ChannelStoreConfig,

    /// Deferred tool loading configuration
    #[serde(default)]
    pub deferred_tools: DeferredToolsConfig,
}

/// Configuration for deferred tool loading.
/// When enabled, only core tools are sent to the LLM per request,
/// and remaining tools are discoverable via `tool_search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredToolsConfig {
    /// Enable deferred tool loading (default: false, opt-in)
    #[serde(default)]
    pub enabled: bool,
}

impl Default for DeferredToolsConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

fn default_skill_env_check() -> bool {
    true
}

fn default_tool_timeout() -> u64 {
    300
}

fn default_theme() -> String {
    "auto".to_string()
}

fn default_language() -> String {
    "auto".to_string()
}

impl Default for ProviderStore {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            active_model: None,
            fallback_models: Vec::new(),
            extra_skills_dirs: Vec::new(),
            disabled_skills: Vec::new(),
            skill_env_check: true,
            embedding: crate::memory::EmbeddingConfig::default(),
            memory_extract: crate::memory::MemoryExtractConfig::default(),
            memory_selection: crate::memory::MemorySelectionConfig::default(),
            dedup: crate::memory::DedupConfig::default(),
            hybrid_search: crate::memory::HybridSearchConfig::default(),
            temporal_decay: crate::memory::TemporalDecayConfig::default(),
            mmr: crate::memory::MmrConfig::default(),
            multimodal: crate::memory::MultimodalConfig::default(),
            embedding_cache: crate::memory::EmbeddingCacheConfig::default(),
            web_search: crate::tools::web_search::WebSearchConfig::default(),
            web_fetch: crate::tools::web_fetch::WebFetchConfig::default(),
            skill_env: std::collections::HashMap::new(),
            compact: crate::context_compact::CompactConfig::default(),
            notification: NotificationConfig::default(),
            image_generate: crate::tools::image_generate::ImageGenConfig::default(),
            canvas: crate::tools::canvas::CanvasConfig::default(),
            image: crate::tools::image::ImageToolConfig::default(),
            pdf: crate::tools::pdf::PdfToolConfig::default(),
            tool_timeout: default_tool_timeout(),
            tool_result_disk_threshold: None,
            theme: default_theme(),
            language: default_language(),
            ui_effects_enabled: true,
            proxy: ProxyConfig::default(),
            skill_prompt_budget: crate::skills::SkillPromptBudget::default(),
            skill_allow_bundled: Vec::new(),
            acp_control: crate::acp_control::AcpControlConfig::default(),
            shortcuts: ShortcutConfig::default(),
            plans_directory: None,
            temperature: None,
            plan_subagent: false,
            channels: crate::channel::ChannelStoreConfig::default(),
            deferred_tools: DeferredToolsConfig::default(),
        }
    }
}

// ── Flat model list item for frontend ─────────────────────────────

/// A model entry combining provider info, for the frontend model selector
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModel {
    pub provider_id: String,
    pub provider_name: String,
    pub api_type: ApiType,
    pub model_id: String,
    pub model_name: String,
    pub input_types: Vec<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    pub reasoning: bool,
}

// ── Persistence ───────────────────────────────────────────────────

fn config_path() -> Result<PathBuf> {
    paths::config_path()
}

/// Load provider store from disk. Returns default if file doesn't exist.
pub fn load_store() -> Result<ProviderStore> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ProviderStore::default());
    }
    let data = std::fs::read_to_string(&path)?;
    let store: ProviderStore = serde_json::from_str(&data)?;
    Ok(store)
}

/// Save provider store to disk.
pub fn save_store(store: &ProviderStore) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Debug: log channel account IDs on every save to detect accidental overwrite
    let account_ids: Vec<&str> = store
        .channels
        .accounts
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    app_debug!(
        "provider",
        "save_store",
        "Saving config with {} channel account(s): {:?}",
        account_ids.len(),
        account_ids
    );
    let data = serde_json::to_string_pretty(store)?;
    std::fs::write(&path, data)?;
    Ok(())
}

// ── Helper: Build available models list ───────────────────────────

pub fn build_available_models(providers: &[ProviderConfig]) -> Vec<AvailableModel> {
    let mut models = Vec::new();
    for p in providers {
        if !p.enabled {
            continue;
        }
        for m in &p.models {
            models.push(AvailableModel {
                provider_id: p.id.clone(),
                provider_name: p.name.clone(),
                api_type: p.api_type.clone(),
                model_id: m.id.clone(),
                model_name: m.name.clone(),
                input_types: m.input_types.clone(),
                context_window: m.context_window,
                max_tokens: m.max_tokens,
                reasoning: m.reasoning,
            });
        }
    }
    models
}

// ── Helper: Parse model reference ─────────────────────────────────

/// Parse a "provider_id::model_id" string into an ActiveModel.
/// Returns None if the format is invalid.
pub fn parse_model_ref(ref_str: &str) -> Option<ActiveModel> {
    let parts: Vec<&str> = ref_str.splitn(2, "::").collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(ActiveModel {
            provider_id: parts[0].to_string(),
            model_id: parts[1].to_string(),
        })
    } else {
        None
    }
}

/// Format an ActiveModel as "provider_id::model_id" string.
#[allow(dead_code)]
pub fn format_model_ref(model: &ActiveModel) -> String {
    format!("{}::{}", model.provider_id, model.model_id)
}

/// Resolve the ordered model chain for a given agent.
/// Returns (primary, fallbacks) where primary is the first model to try
/// and fallbacks are tried in order if primary fails.
///
/// Resolution logic:
/// 1. If the agent has a custom primary, use it; otherwise use global active_model
/// 2. If the agent has custom fallbacks, use them; otherwise use global fallback_models
pub fn resolve_model_chain(
    agent_model: &crate::agent_config::AgentModelConfig,
    store: &ProviderStore,
) -> (Option<ActiveModel>, Vec<ActiveModel>) {
    // Resolve primary
    let primary = agent_model
        .primary
        .as_ref()
        .and_then(|s| parse_model_ref(s))
        .or_else(|| store.active_model.clone());

    // Resolve fallbacks
    let fallbacks = if !agent_model.fallbacks.is_empty() {
        // Agent has custom fallbacks
        agent_model
            .fallbacks
            .iter()
            .filter_map(|s| parse_model_ref(s))
            .collect()
    } else {
        // Use global fallbacks
        store.fallback_models.clone()
    };

    (primary, fallbacks)
}

/// Find a ProviderConfig by provider_id from the store.
/// Only returns enabled providers.
pub fn find_provider<'a>(
    providers: &'a [ProviderConfig],
    provider_id: &str,
) -> Option<&'a ProviderConfig> {
    providers.iter().find(|p| p.id == provider_id && p.enabled)
}

// ── Helper: Create built-in Codex provider ────────────────────────

/// Create or update the built-in Codex provider with OAuth token info.
/// Returns the provider ID.
pub fn ensure_codex_provider(store: &mut ProviderStore) -> String {
    // Check if a Codex provider already exists
    if let Some(existing) = store
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::Codex)
    {
        return existing.id.clone();
    }

    let codex_models = vec![
        ModelConfig {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.3-codex".into(),
            name: "GPT-5.3 Codex".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.3-codex-spark".into(),
            name: "GPT-5.3 Codex Spark".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.2".into(),
            name: "GPT-5.2".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.2-codex".into(),
            name: "GPT-5.2 Codex".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1".into(),
            name: "GPT-5.1".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1-codex-max".into(),
            name: "GPT-5.1 Codex Max".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.1-codex-mini".into(),
            name: "GPT-5.1 Codex Mini".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            cost_input: 0.0,
            cost_output: 0.0,
        },
    ];

    let provider = ProviderConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name: "ChatGPT (Codex)".into(),
        api_type: ApiType::Codex,
        base_url: ApiType::Codex.default_base_url().into(),
        api_key: String::new(), // OAuth, no API key
        models: codex_models,
        enabled: true,
        user_agent: default_user_agent(),
        thinking_style: ThinkingStyle::default(),
    };

    let id = provider.id.clone();
    store.providers.push(provider);
    id
}

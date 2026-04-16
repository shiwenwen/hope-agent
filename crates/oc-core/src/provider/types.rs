use serde::{Deserialize, Serialize};

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

// ── Auth Profile ──────────────────────────────────────────────────

/// An API key profile within a provider. Multiple profiles enable automatic
/// key rotation on rate-limit, auth, or billing errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfile {
    /// Unique profile ID (UUID)
    pub id: String,
    /// User-defined label, e.g. "Org A key", "Personal key"
    pub label: String,
    /// API key for this profile
    pub api_key: String,
    /// Optional base_url override (None = use provider's base_url)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether this profile is enabled
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
}

impl AuthProfile {
    /// Create a new auth profile with a generated UUID.
    pub fn new(label: String, api_key: String, base_url: Option<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            label,
            api_key,
            base_url,
            enabled: true,
        }
    }

    /// Return a copy with the API key masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: mask_key(&self.api_key),
            ..self.clone()
        }
    }
}

/// Mask an API key for display: keep first 4 + last 4 chars.
fn mask_key(key: &str) -> String {
    if key.chars().count() > 8 {
        let prefix: String = key.chars().take(4).collect();
        let suffix: String = key.chars().rev().take(4).collect::<String>().chars().rev().collect();
        format!("{}...{}", prefix, suffix)
    } else if !key.is_empty() {
        "****".to_string()
    } else {
        String::new()
    }
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
    /// API key (empty for Codex OAuth). Legacy single-key field;
    /// prefer `auth_profiles` for multi-key rotation.
    #[serde(default)]
    pub api_key: String,
    /// Multiple API key profiles for automatic rotation on errors.
    /// When non-empty, these take precedence over the legacy `api_key` field.
    #[serde(default)]
    pub auth_profiles: Vec<AuthProfile>,
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

pub(super) fn default_user_agent() -> String {
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
            auth_profiles: Vec::new(),
            models: Vec::new(),
            enabled: true,
            user_agent: default_user_agent(),
            thinking_style: ThinkingStyle::default(),
        }
    }

    /// Return a copy with the API key and all profile keys masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: mask_key(&self.api_key),
            auth_profiles: self.auth_profiles.iter().map(|p| p.masked()).collect(),
            ..self.clone()
        }
    }

    /// Return the effective list of auth profiles for this provider.
    ///
    /// - If `auth_profiles` is non-empty, returns enabled profiles from that list.
    /// - If empty but `api_key` is non-empty, synthesizes a single default profile.
    /// - Codex providers always return an empty list (uses OAuth, not API keys).
    pub fn effective_profiles(&self) -> Vec<AuthProfile> {
        if self.api_type == ApiType::Codex {
            return Vec::new();
        }
        if !self.auth_profiles.is_empty() {
            return self.auth_profiles.iter().filter(|p| p.enabled).cloned().collect();
        }
        if !self.api_key.is_empty() {
            return vec![AuthProfile {
                id: format!("__legacy__{}", self.id),
                label: "Default".to_string(),
                api_key: self.api_key.clone(),
                base_url: None,
                enabled: true,
            }];
        }
        Vec::new()
    }

    /// Resolve the effective base URL for a given auth profile.
    /// Uses the profile's override if present, otherwise the provider's base_url.
    pub fn resolve_base_url<'a>(&'a self, profile: &'a AuthProfile) -> &'a str {
        profile.base_url.as_deref().unwrap_or(&self.base_url)
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiType, AuthProfile, ProviderConfig};

    #[test]
    fn masked_api_key_keeps_utf8_boundaries() {
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            "密钥🔑abcdef".to_string(),
        );
        let masked = cfg.masked();
        assert!(masked.api_key.contains("..."));
        assert_ne!(masked.api_key, cfg.api_key);
    }

    #[test]
    fn effective_profiles_legacy_key() {
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            "sk-ant-test-key-12345678".to_string(),
        );
        let profiles = cfg.effective_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].api_key, "sk-ant-test-key-12345678");
        assert!(profiles[0].base_url.is_none());
    }

    #[test]
    fn effective_profiles_with_auth_profiles() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            "legacy-key".to_string(),
        );
        cfg.auth_profiles = vec![
            AuthProfile::new("Org A".to_string(), "key-a".to_string(), None),
            AuthProfile::new("Org B".to_string(), "key-b".to_string(), Some("https://custom.api.com".to_string())),
        ];
        let profiles = cfg.effective_profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].api_key, "key-a");
        assert_eq!(profiles[1].base_url.as_deref(), Some("https://custom.api.com"));
    }

    #[test]
    fn effective_profiles_codex_always_empty() {
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Codex,
            "https://chatgpt.com".to_string(),
            "some-key".to_string(),
        );
        assert!(cfg.effective_profiles().is_empty());
    }

    #[test]
    fn effective_profiles_skips_disabled() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            String::new(),
        );
        let mut disabled = AuthProfile::new("Disabled".to_string(), "key-d".to_string(), None);
        disabled.enabled = false;
        cfg.auth_profiles = vec![
            disabled,
            AuthProfile::new("Enabled".to_string(), "key-e".to_string(), None),
        ];
        let profiles = cfg.effective_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].label, "Enabled");
    }

    #[test]
    fn resolve_base_url_with_override() {
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            String::new(),
        );
        let profile_no_override = AuthProfile::new("A".to_string(), "k".to_string(), None);
        assert_eq!(cfg.resolve_base_url(&profile_no_override), "https://api.anthropic.com");

        let profile_with_override = AuthProfile::new("B".to_string(), "k".to_string(), Some("https://custom.api.com".to_string()));
        assert_eq!(cfg.resolve_base_url(&profile_with_override), "https://custom.api.com");
    }

    #[test]
    fn masked_includes_profiles() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            "sk-ant-main-key-1234".to_string(),
        );
        cfg.auth_profiles = vec![
            AuthProfile::new("A".to_string(), "sk-ant-profile-key-5678".to_string(), None),
        ];
        let masked = cfg.masked();
        assert!(masked.api_key.contains("..."));
        assert!(masked.auth_profiles[0].api_key.contains("..."));
        assert!(!masked.auth_profiles[0].api_key.contains("profile"));
    }

    #[test]
    fn serde_backward_compat_no_auth_profiles() {
        let json = r#"{
            "id": "test-id",
            "name": "Test",
            "apiType": "anthropic",
            "baseUrl": "https://api.anthropic.com",
            "apiKey": "sk-test",
            "models": [],
            "enabled": true,
            "userAgent": "test",
            "thinkingStyle": "anthropic"
        }"#;
        let cfg: ProviderConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.auth_profiles.is_empty());
        assert_eq!(cfg.effective_profiles().len(), 1);
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

// ── Proxy Types ─────────────────────────────────────────────────

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

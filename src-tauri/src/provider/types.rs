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
            ApiType::OpenaiChat,
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

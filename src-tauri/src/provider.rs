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
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Custom User-Agent header for API requests
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    /// Thinking/reasoning parameter format
    #[serde(default)]
    pub thinking_style: ThinkingStyle,
}

fn default_true() -> bool {
    true
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
        let masked_key = if self.api_key.len() > 8 {
            format!("{}...{}", &self.api_key[..4], &self.api_key[self.api_key.len()-4..])
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

// ── Active Model ──────────────────────────────────────────────────

/// Represents the currently active model selection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub provider_id: String,
    pub model_id: String,
}

// ── Serializable Store ────────────────────────────────────────────

// ── Notification Config ─────────────────────────────────────────

/// Global notification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationConfig {
    /// Global on/off toggle (default: true)
    #[serde(default = "default_true")]
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
    /// Context compaction configuration
    #[serde(default)]
    pub compact: crate::context_compact::CompactConfig,
    /// Notification configuration
    #[serde(default)]
    pub notification: NotificationConfig,
}

fn default_skill_env_check() -> bool {
    true
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
            web_search: crate::tools::web_search::WebSearchConfig::default(),
            web_fetch: crate::tools::web_fetch::WebFetchConfig::default(),
            skill_env: std::collections::HashMap::new(),
            compact: crate::context_compact::CompactConfig::default(),
            notification: NotificationConfig::default(),
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
pub fn find_provider<'a>(providers: &'a [ProviderConfig], provider_id: &str) -> Option<&'a ProviderConfig> {
    providers.iter().find(|p| p.id == provider_id && p.enabled)
}

// ── Helper: Create built-in Codex provider ────────────────────────

/// Create or update the built-in Codex provider with OAuth token info.
/// Returns the provider ID.
pub fn ensure_codex_provider(store: &mut ProviderStore) -> String {
    // Check if a Codex provider already exists
    if let Some(existing) = store.providers.iter().find(|p| p.api_type == ApiType::Codex) {
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

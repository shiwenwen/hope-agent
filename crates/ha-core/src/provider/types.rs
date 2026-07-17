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

    /// Whether this is the built-in Codex OAuth API type. Convenience for
    /// the many call sites that branch on Codex-vs-other for auth flow,
    /// failover policy, and re-authorization UX hints.
    pub fn is_codex(&self) -> bool {
        matches!(self, ApiType::Codex)
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingStyle {
    /// OpenAI format: `reasoning_effort: "low"/"medium"/"high"`
    #[default]
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
    /// Optional per-model thinking parameter format override.
    /// `None` = inherit provider-level `thinking_style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_style: Option<ThinkingStyle>,
    /// 每百万输入 token 单价。`None` = 未标价（厂商单价未知），`Some(0.0)` = 明确不按
    /// token 计费（本地模型、包月端点）。二者对大盘结算的含义完全不同：未标价回退内置
    /// 估算表，明确免费则如实记 $0——旧版统一用 `0` 表达两者，导致本机跑的模型也被按估算
    /// 表收费（详见 `dashboard::cost::resolve_cost`）。
    ///
    /// 币种由 Provider 级 [`ProviderConfig::currency`] 声明（缺省 = USD），本字段照厂商
    /// 价目页原文录入；成本入账在 `dashboard::cost` 单点换算，此处不做换算。
    #[serde(default)]
    pub cost_input: Option<f64>,
    /// 每百万输出 token 单价。语义同 `cost_input`。
    #[serde(default)]
    pub cost_output: Option<f64>,
}

fn default_input_types() -> Vec<String> {
    // Empty = "unconfigured" — distinct from a deliberate text-only pick,
    // which is represented by a populated list that lacks `image`. New
    // model entries default to this so the absence of explicit capability
    // info doesn't get conflated with `["text"]` (which is now an
    // intentional opt-out).
    Vec::new()
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
        let suffix: String = key
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
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
    /// Explicitly allow this provider's base_url to resolve into a private/loopback
    /// network. Used for self-hosted Ollama / LM Studio; when true the base host is
    /// auto-appended to `AppConfig.ssrf.trusted_hosts` by the UI on save.
    /// Stored here for round-trip fidelity; back-end SSRF enforcement for LLM
    /// traffic is deferred to Phase B.
    #[serde(default)]
    pub allow_private_network: bool,
    /// 本 Provider 模型单价（`ModelConfig.cost_*`）的币种。`None` = USD（历史默认）。
    /// 成本入账在 `dashboard::cost::resolve_cost` 单点按 `CNY_PER_USD` 换算成 USD，
    /// 其余消费方（模板、GUI、导入导出）原样透传数字不做换算——单价照厂商价目页
    /// 原文录入，可维护性优先于把换算摊到录入侧。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<Currency>,
}

/// 模型单价币种。只收录内置模板实际用到的币种，新增须同步
/// `dashboard::cost` 的换算与前端 `types.ts` 的 union。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "USD")]
    Usd,
    #[serde(rename = "CNY")]
    Cny,
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
            allow_private_network: false,
            currency: None,
        }
    }

    /// First-run onboarding shortcut: construct an Anthropic provider
    /// pre-populated with Claude Sonnet 4.6 metadata. Shared between the
    /// Tauri `initialize_agent` command and the HTTP `POST /api/agents/initialize`
    /// handler so model defaults (context / cost / input types) live in one place.
    pub fn new_default_anthropic(api_key: String) -> Self {
        let mut provider = Self::new(
            "Anthropic".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            api_key,
        );
        provider.models.push(ModelConfig {
            id: "claude-sonnet-4-6".to_string(),
            name: "Claude Sonnet 4.6".to_string(),
            input_types: vec!["text".to_string(), "image".to_string()],
            context_window: 200_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(3.0),
            cost_output: Some(15.0),
        });
        provider
    }

    /// Return a copy with the API key and all profile keys masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: mask_key(&self.api_key),
            auth_profiles: self.auth_profiles.iter().map(|p| p.masked()).collect(),
            ..self.clone()
        }
    }

    /// Trim leading/trailing whitespace from every user-entered text field.
    ///
    /// Copy-pasted base URLs, model IDs, and API keys routinely carry a stray
    /// leading space or trailing newline that silently breaks API calls. This
    /// runs on every provider add/update write and before connectivity tests so
    /// the stored config and the tested config are both clean.
    pub fn sanitize(&mut self) {
        self.name = self.name.trim().to_string();
        self.base_url = self.base_url.trim().to_string();
        self.api_key = self.api_key.trim().to_string();
        // A blank User-Agent would send an empty header (some gateways 403 on
        // it); fall back to the default rather than persist "".
        let user_agent = self.user_agent.trim();
        self.user_agent = if user_agent.is_empty() {
            default_user_agent()
        } else {
            user_agent.to_string()
        };
        for model in &mut self.models {
            model.id = model.id.trim().to_string();
            model.name = model.name.trim().to_string();
        }
        for profile in &mut self.auth_profiles {
            profile.label = profile.label.trim().to_string();
            profile.api_key = profile.api_key.trim().to_string();
            // An override that trims down to empty means "no override".
            profile.base_url = profile
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
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
            return self
                .auth_profiles
                .iter()
                .filter(|p| p.enabled)
                .cloned()
                .collect();
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

    /// Return the configured model entry, if present.
    pub fn model_config(&self, model_id: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.id == model_id)
    }

    /// Whether the given model accepts image input.
    ///
    /// Catalog-driven, no API round-trip: `input_types.contains("image")`
    /// means yes; an empty list means "unconfigured" (assume yes — the API
    /// is the source of truth on first send); any populated list without
    /// `image` (e.g. `["text"]`, `["text", "audio"]`) is an explicit
    /// opt-out set by the template author or by the user in ModelEditor.
    pub fn model_supports_vision(&self, model_id: &str) -> bool {
        let Some(m) = self.model_config(model_id) else {
            return true;
        };
        if m.input_types.is_empty() {
            return true;
        }
        m.input_types.iter().any(|t| t == "image")
    }

    /// Resolve the effective thinking style for a model.
    ///
    /// Precedence:
    /// 1. `reasoning = false` on the model hard-disables thinking
    /// 2. model-level `thinking_style` override
    /// 3. provider-level `thinking_style`
    pub fn effective_thinking_style_for_model(&self, model_id: &str) -> ThinkingStyle {
        match self.model_config(model_id) {
            Some(model) if !model.reasoning => ThinkingStyle::None,
            Some(model) => model
                .thinking_style
                .clone()
                .unwrap_or_else(|| self.thinking_style.clone()),
            None => self.thinking_style.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiType, AuthProfile, ModelConfig, ProviderConfig, ThinkingStyle};

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
    fn sanitize_trims_user_entered_fields() {
        let mut cfg = ProviderConfig::new(
            "  My Provider  ".to_string(),
            ApiType::OpenaiChat,
            "  https://api.example.com/v1 \n".to_string(),
            "\tsk-key-123  ".to_string(),
        );
        cfg.user_agent = "  custom-agent  ".to_string();
        cfg.models = vec![ModelConfig {
            id: "  gpt-5.4\n".to_string(),
            name: "  GPT 5.4 ".to_string(),
            input_types: Vec::new(),
            context_window: 200_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        }];
        cfg.auth_profiles = vec![
            AuthProfile::new(" Org A ".to_string(), " key-a ".to_string(), None),
            AuthProfile::new(
                "Org B".to_string(),
                "key-b".to_string(),
                Some("   ".to_string()),
            ),
        ];

        cfg.sanitize();

        assert_eq!(cfg.name, "My Provider");
        assert_eq!(cfg.base_url, "https://api.example.com/v1");
        assert_eq!(cfg.api_key, "sk-key-123");
        assert_eq!(cfg.user_agent, "custom-agent");
        assert_eq!(cfg.models[0].id, "gpt-5.4");
        assert_eq!(cfg.models[0].name, "GPT 5.4");
        assert_eq!(cfg.auth_profiles[0].label, "Org A");
        assert_eq!(cfg.auth_profiles[0].api_key, "key-a");
        // A base-url override that trims to empty becomes "no override".
        assert!(cfg.auth_profiles[1].base_url.is_none());
    }

    #[test]
    fn sanitize_blank_user_agent_falls_back_to_default() {
        let mut cfg = ProviderConfig::new(
            "p".to_string(),
            ApiType::OpenaiChat,
            "https://api.example.com".to_string(),
            "k".to_string(),
        );
        cfg.user_agent = "   \n".to_string();
        cfg.sanitize();
        assert_eq!(cfg.user_agent, super::default_user_agent());
        assert!(!cfg.user_agent.is_empty());
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
            AuthProfile::new(
                "Org B".to_string(),
                "key-b".to_string(),
                Some("https://custom.api.com".to_string()),
            ),
        ];
        let profiles = cfg.effective_profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].api_key, "key-a");
        assert_eq!(
            profiles[1].base_url.as_deref(),
            Some("https://custom.api.com")
        );
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
        assert_eq!(
            cfg.resolve_base_url(&profile_no_override),
            "https://api.anthropic.com"
        );

        let profile_with_override = AuthProfile::new(
            "B".to_string(),
            "k".to_string(),
            Some("https://custom.api.com".to_string()),
        );
        assert_eq!(
            cfg.resolve_base_url(&profile_with_override),
            "https://custom.api.com"
        );
    }

    #[test]
    fn masked_includes_profiles() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::Anthropic,
            "https://api.anthropic.com".to_string(),
            "sk-ant-main-key-1234".to_string(),
        );
        cfg.auth_profiles = vec![AuthProfile::new(
            "A".to_string(),
            "sk-ant-profile-key-5678".to_string(),
            None,
        )];
        let masked = cfg.masked();
        assert!(masked.api_key.contains("..."));
        assert!(masked.auth_profiles[0].api_key.contains("..."));
        assert!(!masked.auth_profiles[0].api_key.contains("profile"));
    }

    #[test]
    fn model_override_beats_provider_thinking_style() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            String::new(),
        );
        cfg.thinking_style = ThinkingStyle::Openai;
        cfg.models.push(ModelConfig {
            id: "m1".to_string(),
            name: "Model 1".to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: true,
            thinking_style: Some(ThinkingStyle::Qwen),
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });

        assert_eq!(
            cfg.effective_thinking_style_for_model("m1"),
            ThinkingStyle::Qwen
        );
    }

    #[test]
    fn reasoning_false_forces_none_thinking_style() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            String::new(),
        );
        cfg.thinking_style = ThinkingStyle::Openai;
        cfg.models.push(ModelConfig {
            id: "m1".to_string(),
            name: "Model 1".to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: Some(ThinkingStyle::Anthropic),
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });

        assert_eq!(
            cfg.effective_thinking_style_for_model("m1"),
            ThinkingStyle::None
        );
    }

    #[test]
    fn model_supports_vision_when_input_types_contains_image() {
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            String::new(),
        );
        cfg.models.push(ModelConfig {
            id: "gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
            input_types: vec!["text".to_string(), "image".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });
        assert!(cfg.model_supports_vision("gpt-4o"));
    }

    #[test]
    fn model_supports_vision_false_for_explicit_text_only() {
        // `["text"]` is an explicit opt-out: either the template wrote it
        // (e.g. DeepSeek V4 Flash in `international.ts`) or the user
        // populated it via ModelEditor. Trust the catalog.
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.deepseek.com".to_string(),
            String::new(),
        );
        cfg.models.push(ModelConfig {
            id: "deepseek-v4-flash".to_string(),
            name: "DeepSeek V4 Flash".to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: true,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });
        assert!(!cfg.model_supports_vision("deepseek-v4-flash"));
    }

    #[test]
    fn model_supports_vision_false_for_explicit_non_image_input_list() {
        // Any non-empty list lacking `image` (e.g. `["text", "audio"]`)
        // is a deliberate opt-out. Trust it.
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.example.com".to_string(),
            String::new(),
        );
        cfg.models.push(ModelConfig {
            id: "text-audio-only".to_string(),
            name: "Text+Audio Only".to_string(),
            input_types: vec!["text".to_string(), "audio".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });
        assert!(!cfg.model_supports_vision("text-audio-only"));
    }

    #[test]
    fn model_supports_vision_treats_empty_input_list_as_unknown() {
        // Empty `input_types` means "unconfigured" (e.g. a config saved
        // under an older schema where the field was absent). Assume
        // vision so configs upgrading in won't silently lose it; the
        // user can lock it down via ModelEditor when needed.
        let mut cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.example.com".to_string(),
            String::new(),
        );
        cfg.models.push(ModelConfig {
            id: "empty-list".to_string(),
            name: "Empty List".to_string(),
            input_types: vec![],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        });
        assert!(cfg.model_supports_vision("empty-list"));
    }

    #[test]
    fn model_supports_vision_defaults_true_for_uncatalogued_alias() {
        // Preserve legacy behavior for users whose model id isn't in the
        // catalog — assume vision so we don't break working vision flows.
        let cfg = ProviderConfig::new(
            "t".to_string(),
            ApiType::OpenaiChat,
            "https://api.openai.com".to_string(),
            String::new(),
        );
        assert!(cfg.model_supports_vision("unknown-model"));
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub provider_id: String,
    pub model_id: String,
}

impl std::fmt::Display for ActiveModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.provider_id, self.model_id)
    }
}

// ── Model Chain ─────────────────────────────────────────────────────

/// A resolved model chain: try `primary` first, fall through `fallbacks` in
/// order on failure. The unified shape for "which model(s) should this
/// non-chat task use" across background/automation consumers — see
/// `crate::automation`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelChain {
    pub primary: ActiveModel,
    #[serde(default)]
    pub fallbacks: Vec<ActiveModel>,
}

impl ModelChain {
    /// Flatten into `[primary, ...fallbacks]` for iteration.
    pub fn into_vec(self) -> Vec<ActiveModel> {
        let mut v = Vec::with_capacity(1 + self.fallbacks.len());
        v.push(self.primary);
        v.extend(self.fallbacks);
        v
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
    pub thinking_style: ThinkingStyle,
}

// ── Proxy Types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// Use system proxy (environment variables HTTP_PROXY/HTTPS_PROXY/ALL_PROXY)
    #[default]
    System,
    /// No proxy – direct connection
    None,
    /// Custom proxy URL
    Custom,
}

/// Global proxy configuration for all outgoing HTTP requests
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    /// Proxy mode: "system" (default), "none", or "custom"
    #[serde(default)]
    pub mode: ProxyMode,
    /// Custom proxy URL (only used when mode is "custom"), e.g. "http://127.0.0.1:7890"
    #[serde(default)]
    pub url: Option<String>,
}

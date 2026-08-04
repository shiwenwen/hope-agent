use serde::{Deserialize, Serialize};

// 类型已下沉 ha-config-schema：此处原地再导出保持 `crate::provider::types::*`
// / `crate::provider::*` 路径不变；`default_user_agent` 原为 `pub(super)`，
// 因 `helpers.rs` 仍经 `super::types::default_user_agent()` 调用而随迁再导出
// （可见性升级为 pub）。redact / crud 等凭据接线不下沉，仍在本 crate。
pub use ha_config_schema::provider::{
    default_user_agent, ActiveModel, ApiType, AuthProfile, Currency, ModelChain, ModelConfig,
    ProviderConfig, ProxyConfig, ProxyMode, ThinkingStyle,
};

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

/// 人民币兑美元换算率。粗粒度常量、只服务成本展示（非交易汇率），随大幅波动手动
/// 调（2026-07 现汇约 6.8–7.2 区间，取整数中值）。单价按厂商价目页原文录入
/// （[`ProviderConfig.currency`] 标注币种），换算集中在这一处——大盘成本聚合、
/// 估算表里人民币计价厂商的臂、self_diagnosis 与 eval_context 的折算都写成
/// `¥价 / CNY_PER_USD`，全部路径口径一致。
///
/// **住在 kernel 而不是 ha-dash**：三个消费者里有两个（`self_diagnosis` /
/// `eval_context`）是 kernel 模块，且都与 [`Currency`] 配对使用——常量跟着
/// `Currency` 走才不会让 kernel 反向依赖大盘。
pub const CNY_PER_USD: f64 = 7.0;

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

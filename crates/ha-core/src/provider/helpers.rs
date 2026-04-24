//! Helpers operating on provider data inside [`crate::config::AppConfig`].

use super::types::{
    ActiveModel, ApiType, AuthProfile, AvailableModel, ModelConfig, ProviderConfig, ThinkingStyle,
};
use crate::config::AppConfig;

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
                thinking_style: p.effective_thinking_style_for_model(&m.id),
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
    config: &AppConfig,
) -> (Option<ActiveModel>, Vec<ActiveModel>) {
    // Resolve primary
    let primary = agent_model
        .primary
        .as_ref()
        .and_then(|s| parse_model_ref(s))
        .or_else(|| config.active_model.clone());

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
        config.fallback_models.clone()
    };

    (primary, fallbacks)
}

/// Find a ProviderConfig by provider_id from the providers slice.
/// Only returns enabled providers.
pub fn find_provider<'a>(
    providers: &'a [ProviderConfig],
    provider_id: &str,
) -> Option<&'a ProviderConfig> {
    providers.iter().find(|p| p.id == provider_id && p.enabled)
}

// ── Helper: Create built-in Codex provider ────────────────────────

// ── Auth Profile Key Merge ────────────────────────────────────────

/// Merge incoming auth profiles with existing ones, preserving real API keys
/// when the incoming key appears to be masked (contains "..." or is "****").
///
/// Used by update_provider to avoid overwriting keys with masked values.
pub fn merge_profile_keys(existing: &[AuthProfile], incoming: &[AuthProfile]) -> Vec<AuthProfile> {
    incoming
        .iter()
        .map(|inc| {
            if is_masked_key(&inc.api_key) {
                // Find matching existing profile by ID and use its key
                if let Some(prev) = existing.iter().find(|e| e.id == inc.id) {
                    AuthProfile {
                        api_key: prev.api_key.clone(),
                        ..inc.clone()
                    }
                } else {
                    inc.clone()
                }
            } else {
                inc.clone()
            }
        })
        .collect()
}

/// Check if an API key value looks like a masked display string.
pub fn is_masked_key(key: &str) -> bool {
    key.contains("...") || key == "****"
}

/// Default built-in Codex model list. Kept in sync with
/// [`crate::agent::config::get_codex_models`] (same IDs, richer shape).
///
/// New entries added here are auto-merged into any user's existing Codex
/// provider by [`ensure_codex_provider`], so老用户升级后无需重新登录也能拿到新模型。
fn default_codex_models() -> Vec<ModelConfig> {
    vec![
        ModelConfig {
            id: "gpt-5.5".into(),
            name: "GPT-5.5".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.4".into(),
            name: "GPT-5.4".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
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
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        },
    ]
}

/// Create or update the built-in Codex provider with OAuth token info.
/// Returns the provider ID.
///
/// When a Codex provider already exists, any default models missing from the
/// user's local `models` list are appended (preserving existing entries and
/// order). This keeps老用户登录过后本地 config.json 的模型列表，随升级自动跟上新增 Codex 模型。
pub fn ensure_codex_provider(config: &mut AppConfig) -> String {
    let defaults = default_codex_models();

    if let Some(existing) = config
        .providers
        .iter_mut()
        .find(|p| p.api_type == ApiType::Codex)
    {
        let mut added: Vec<String> = Vec::new();
        for m in &defaults {
            if !existing.models.iter().any(|x| x.id == m.id) {
                added.push(m.id.clone());
                existing.models.push(m.clone());
            }
        }
        if !added.is_empty() {
            crate::app_info!(
                "provider",
                "ensure_codex",
                "Backfilled missing Codex default models into existing provider: {}",
                added.join(", ")
            );
        }
        return existing.id.clone();
    }

    let provider = ProviderConfig {
        id: uuid::Uuid::new_v4().to_string(),
        name: "ChatGPT (Codex)".into(),
        api_type: ApiType::Codex,
        base_url: ApiType::Codex.default_base_url().into(),
        api_key: String::new(), // OAuth, no API key
        auth_profiles: Vec::new(),
        models: defaults,
        enabled: true,
        user_agent: super::types::default_user_agent(),
        thinking_style: ThinkingStyle::default(),
        allow_private_network: false,
    };

    let id = provider.id.clone();
    config.providers.push(provider);
    id
}

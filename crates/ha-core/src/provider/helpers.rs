//! Helpers operating on provider data inside [`crate::config::AppConfig`].

use super::types::{
    ActiveModel, ApiType, AuthProfile, AvailableModel, ModelConfig, ProviderConfig, ThinkingStyle,
};
use crate::config::AppConfig;

/// Whether the referenced provider and model are still present in config.
/// Disabled providers still count as existing because disabling is reversible.
pub fn model_ref_exists(providers: &[ProviderConfig], model: &ActiveModel) -> bool {
    providers.iter().any(|provider| {
        provider.id == model.provider_id
            && provider
                .models
                .iter()
                .any(|candidate| candidate.id == model.model_id)
    })
}

/// Whether the referenced provider is enabled and still contains the model.
pub fn model_ref_is_available(providers: &[ProviderConfig], model: &ActiveModel) -> bool {
    providers.iter().any(|provider| {
        provider.enabled
            && provider.id == model.provider_id
            && provider
                .models
                .iter()
                .any(|candidate| candidate.id == model.model_id)
    })
}

/// Return the first available model in persisted provider/model order.
pub fn first_available_model(providers: &[ProviderConfig]) -> Option<ActiveModel> {
    providers
        .iter()
        .filter(|provider| provider.enabled)
        .find_map(|provider| {
            provider.models.first().map(|model| ActiveModel {
                provider_id: provider.id.clone(),
                model_id: model.id.clone(),
            })
        })
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

/// Resolve the ordered model chain for a given agent.
/// Returns (primary, fallbacks) where primary is the first model to try
/// and fallbacks are tried in order if primary fails.
///
/// Resolution logic:
/// 1. Use the first available model from agent primary, then global active model
/// 2. If the agent has custom fallbacks, use its available entries; otherwise
///    use available global fallback models
/// 3. Deduplicate fallbacks against the selected primary while preserving order
pub fn resolve_model_chain(
    agent_model: &crate::agent_config::AgentModelConfig,
    config: &AppConfig,
) -> (Option<ActiveModel>, Vec<ActiveModel>) {
    resolve_model_chain_with_preferred(None, agent_model, config)
}

/// Resolve an available model chain with an optional persisted preference.
///
/// `preferred` is intended for a Session pin or Plan Mode preference. An
/// unavailable preference is skipped so the Agent and global candidates can
/// still be selected as primary. Lower-priority primary candidates are not
/// promoted into the fallback chain. Explicit per-turn overrides must be
/// validated by callers before invoking this resolver because they are not
/// allowed to fall back silently.
pub fn resolve_model_chain_with_preferred(
    preferred: Option<&str>,
    agent_model: &crate::agent_config::AgentModelConfig,
    config: &AppConfig,
) -> (Option<ActiveModel>, Vec<ActiveModel>) {
    let primary = preferred
        .and_then(parse_model_ref)
        .filter(|model| model_ref_is_available(&config.providers, model))
        .or_else(|| {
            agent_model
                .primary
                .as_deref()
                .and_then(parse_model_ref)
                .filter(|model| model_ref_is_available(&config.providers, model))
        })
        .or_else(|| {
            config
                .active_model
                .clone()
                .filter(|model| model_ref_is_available(&config.providers, model))
        });
    let mut chain: Vec<ActiveModel> = primary.into_iter().collect();

    if agent_model.fallbacks.is_empty() {
        for fallback in &config.fallback_models {
            push_model_if_available(&mut chain, Some(fallback.clone()), &config.providers);
        }
    } else {
        for fallback in &agent_model.fallbacks {
            push_model_ref_if_available(&mut chain, Some(fallback), &config.providers);
        }
    }

    let mut resolved = chain.into_iter();
    (resolved.next(), resolved.collect())
}

fn push_model_ref_if_available(
    chain: &mut Vec<ActiveModel>,
    model_ref: Option<&str>,
    providers: &[ProviderConfig],
) {
    push_model_if_available(chain, model_ref.and_then(parse_model_ref), providers);
}

fn push_model_if_available(
    chain: &mut Vec<ActiveModel>,
    model: Option<ActiveModel>,
    providers: &[ProviderConfig],
) {
    let Some(model) = model else {
        return;
    };
    if !model_ref_is_available(providers, &model)
        || chain.iter().any(|candidate| {
            candidate.provider_id == model.provider_id && candidate.model_id == model.model_id
        })
    {
        return;
    }
    chain.push(model);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_config::AgentModelConfig;

    fn model(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: id.to_string(),
            input_types: vec!["text".to_string()],
            context_window: 128_000,
            max_tokens: 8192,
            reasoning: false,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        }
    }

    fn provider(id: &str, enabled: bool, model_ids: &[&str]) -> ProviderConfig {
        let mut provider = ProviderConfig::new(
            id.to_string(),
            ApiType::OpenaiChat,
            format!("https://{id}.example.com"),
            "test-key".to_string(),
        );
        provider.id = id.to_string();
        provider.enabled = enabled;
        provider.models = model_ids.iter().map(|id| model(id)).collect();
        provider
    }

    fn active(provider_id: &str, model_id: &str) -> ActiveModel {
        ActiveModel {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        }
    }

    fn agent_model(primary: Option<&str>, fallbacks: &[&str]) -> AgentModelConfig {
        AgentModelConfig {
            primary: primary.map(str::to_string),
            fallbacks: fallbacks.iter().map(|value| (*value).to_string()).collect(),
            ..Default::default()
        }
    }

    fn resolved_refs(primary: Option<ActiveModel>, fallbacks: Vec<ActiveModel>) -> Vec<String> {
        primary
            .into_iter()
            .chain(fallbacks)
            .map(|model| model.to_string())
            .collect()
    }

    #[test]
    fn disabled_agent_primary_falls_back_to_available_global_active() {
        let config = AppConfig {
            providers: vec![
                provider("disabled", false, &["agent-model"]),
                provider("global", true, &["active-model"]),
            ],
            active_model: Some(active("global", "active-model")),
            ..Default::default()
        };
        let agent_model = agent_model(Some("disabled::agent-model"), &[]);

        let chain = resolve_model_chain_with_preferred(None, &agent_model, &config);

        assert_eq!(resolved_refs(chain.0, chain.1), ["global::active-model"]);
    }

    #[test]
    fn missing_preferred_and_agent_primary_fall_back_to_available_global_active() {
        let config = AppConfig {
            providers: vec![provider("global", true, &["active-model"])],
            active_model: Some(active("global", "active-model")),
            ..Default::default()
        };
        let agent_model = agent_model(Some("missing::agent-model"), &[]);

        let chain = resolve_model_chain_with_preferred(
            Some("missing::session-model"),
            &agent_model,
            &config,
        );

        assert_eq!(resolved_refs(chain.0, chain.1), ["global::active-model"]);
    }

    #[test]
    fn configured_agent_fallbacks_are_filtered_without_appending_global_fallbacks() {
        let config = AppConfig {
            providers: vec![
                provider("primary", true, &["p"]),
                provider("disabled", false, &["d"]),
                provider("agent-fallback", true, &["a"]),
                provider("global-fallback", true, &["g"]),
            ],
            active_model: Some(active("primary", "p")),
            fallback_models: vec![active("global-fallback", "g")],
            ..Default::default()
        };
        let agent_model = agent_model(
            Some("primary::p"),
            &["disabled::d", "missing::m", "agent-fallback::a"],
        );

        let chain = resolve_model_chain_with_preferred(None, &agent_model, &config);

        assert_eq!(
            resolved_refs(chain.0, chain.1),
            ["primary::p", "agent-fallback::a"]
        );
    }

    #[test]
    fn unavailable_agent_fallbacks_do_not_expand_to_global_fallbacks() {
        let config = AppConfig {
            providers: vec![
                provider("primary", true, &["p"]),
                provider("disabled", false, &["d"]),
                provider("global-fallback", true, &["g"]),
            ],
            active_model: Some(active("primary", "p")),
            fallback_models: vec![active("global-fallback", "g")],
            ..Default::default()
        };
        let agent_model = agent_model(Some("primary::p"), &["disabled::d", "missing::m"]);

        let chain = resolve_model_chain_with_preferred(None, &agent_model, &config);

        assert_eq!(resolved_refs(chain.0, chain.1), ["primary::p"]);
    }

    #[test]
    fn global_fallbacks_are_used_when_agent_fallbacks_are_not_configured() {
        let config = AppConfig {
            providers: vec![
                provider("primary", true, &["p"]),
                provider("disabled", false, &["d"]),
                provider("global-fallback", true, &["g"]),
            ],
            active_model: Some(active("primary", "p")),
            fallback_models: vec![
                active("disabled", "d"),
                active("missing", "m"),
                active("global-fallback", "g"),
            ],
            ..Default::default()
        };
        let agent_model = agent_model(Some("primary::p"), &[]);

        let chain = resolve_model_chain_with_preferred(None, &agent_model, &config);

        assert_eq!(
            resolved_refs(chain.0, chain.1),
            ["primary::p", "global-fallback::g"]
        );
    }

    #[test]
    fn resolver_deduplicates_candidates_while_preserving_priority_order() {
        let config = AppConfig {
            providers: vec![
                provider("preferred", true, &["m"]),
                provider("global", true, &["m"]),
                provider("agent-fallback", true, &["m"]),
                provider("global-fallback", true, &["m"]),
            ],
            active_model: Some(active("global", "m")),
            fallback_models: vec![active("global", "m"), active("global-fallback", "m")],
            ..Default::default()
        };
        let agent_model = agent_model(
            Some("preferred::m"),
            &["global::m", "agent-fallback::m", "preferred::m"],
        );

        let chain = resolve_model_chain_with_preferred(Some("preferred::m"), &agent_model, &config);

        assert_eq!(
            resolved_refs(chain.0, chain.1),
            ["preferred::m", "global::m", "agent-fallback::m",]
        );
    }

    #[test]
    fn invalid_session_preferred_continues_through_agent_then_global() {
        let config = AppConfig {
            providers: vec![
                provider("disabled", false, &["session-model"]),
                provider("agent", true, &["primary"]),
                provider("global", true, &["active"]),
            ],
            active_model: Some(active("global", "active")),
            ..Default::default()
        };
        let agent_model = agent_model(Some("agent::primary"), &[]);

        for preferred in ["disabled::session-model", "missing::session-model"] {
            let chain = resolve_model_chain_with_preferred(Some(preferred), &agent_model, &config);

            assert_eq!(resolved_refs(chain.0, chain.1), ["agent::primary"]);
        }
    }

    #[test]
    fn valid_session_preferred_does_not_promote_lower_priority_primaries_to_fallbacks() {
        let config = AppConfig {
            providers: vec![
                provider("session", true, &["preferred"]),
                provider("agent", true, &["primary"]),
                provider("global", true, &["active"]),
                provider("fallback", true, &["configured"]),
            ],
            active_model: Some(active("global", "active")),
            ..Default::default()
        };
        let agent_model = agent_model(Some("agent::primary"), &["fallback::configured"]);

        let chain =
            resolve_model_chain_with_preferred(Some("session::preferred"), &agent_model, &config);

        assert_eq!(
            resolved_refs(chain.0, chain.1),
            ["session::preferred", "fallback::configured"]
        );
    }

    #[test]
    fn resolver_returns_empty_chain_when_no_candidate_is_available() {
        let config = AppConfig {
            providers: vec![provider("disabled", false, &["m"])],
            active_model: Some(active("disabled", "m")),
            fallback_models: vec![active("missing", "m")],
            ..Default::default()
        };
        let agent_model = agent_model(Some("missing::primary"), &["disabled::m"]);

        let chain = resolve_model_chain_with_preferred(
            Some("missing::session-model"),
            &agent_model,
            &config,
        );

        assert!(chain.0.is_none());
        assert!(chain.1.is_empty());
    }
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
            id: "gpt-5.6-sol".into(),
            name: "GPT-5.6 Sol".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.6-terra".into(),
            name: "GPT-5.6 Terra".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        },
        ModelConfig {
            id: "gpt-5.6-luna".into(),
            name: "GPT-5.6 Luna".into(),
            input_types: vec!["text".into()],
            context_window: 200_000,
            max_tokens: 16384,
            reasoning: true,
            thinking_style: None,
            cost_input: 0.0,
            cost_output: 0.0,
        },
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
        // Reorder so the canonical default order (latest first, e.g. gpt-5.5)
        // leads the picker. Older configs appended new models to the tail,
        // burying the newest model at the bottom; this lifts the defaults to the
        // front while keeping any non-default extras in their original order.
        existing.models.sort_by_key(|m| {
            defaults
                .iter()
                .position(|d| d.id == m.id)
                .unwrap_or(usize::MAX)
        });
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

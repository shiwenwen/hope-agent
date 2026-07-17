//! Known local/self-hosted provider backends.

use serde::{Deserialize, Serialize};

use crate::config::mutate_config;

use super::crud::{
    map_config_error, push_model_if_missing, reconcile_model_references,
    repair_hard_deleted_model_references, ProviderWriteError, ProviderWriteResult,
};
use super::types::{ActiveModel, ApiType, ModelConfig, ProviderConfig};

pub const LOCAL_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnownLocalBackend {
    pub key: String,
    pub name: String,
    pub api_type: ApiType,
    pub base_url: String,
    pub hosts: Vec<String>,
    pub port: u16,
}

pub fn known_local_backends() -> Vec<KnownLocalBackend> {
    vec![
        backend(
            "ollama",
            "Ollama",
            LOCAL_OLLAMA_BASE_URL,
            11434,
            &["127.0.0.1", "localhost", "::1", "ollama.local"],
        ),
        backend(
            "litellm",
            "LiteLLM",
            "http://127.0.0.1:4000",
            4000,
            LOCAL_HOSTS,
        ),
        backend("vllm", "vLLM", "http://127.0.0.1:8000", 8000, LOCAL_HOSTS),
        backend(
            "lm-studio",
            "LM Studio",
            "http://127.0.0.1:1234",
            1234,
            LOCAL_HOSTS,
        ),
        backend(
            "sglang",
            "SGLang",
            "http://127.0.0.1:30000",
            30000,
            LOCAL_HOSTS,
        ),
    ]
}

const LOCAL_HOSTS: &[&str] = &["127.0.0.1", "localhost", "::1"];

fn backend(key: &str, name: &str, base_url: &str, port: u16, hosts: &[&str]) -> KnownLocalBackend {
    KnownLocalBackend {
        key: key.to_string(),
        name: name.to_string(),
        api_type: ApiType::OpenaiChat,
        base_url: base_url.to_string(),
        hosts: hosts.iter().map(|h| (*h).to_string()).collect(),
        port,
    }
}

pub fn known_local_backend(key: &str) -> Option<KnownLocalBackend> {
    known_local_backends()
        .into_iter()
        .find(|backend| backend.key == key)
}

pub fn provider_matches_known_local_backend(provider: &ProviderConfig, backend_key: &str) -> bool {
    known_local_backend(backend_key)
        .map(|backend| {
            known_local_backend_matches(&backend, &provider.api_type, &provider.base_url)
        })
        .unwrap_or(false)
}

pub fn known_local_backend_matches(
    backend: &KnownLocalBackend,
    api_type: &ApiType,
    base_url: &str,
) -> bool {
    if &backend.api_type != api_type {
        return false;
    }
    let Some((host, port)) = parse_host_port(base_url) else {
        return false;
    };
    port == backend.port
        && backend
            .hosts
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&host))
}

fn parse_host_port(base_url: &str) -> Option<(String, u16)> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    let host = parsed
        .host_str()?
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    let port = parsed.port_or_known_default()?;
    Some((host, port))
}

/// Upsert a model into a known local backend provider. Unlike generic
/// `add_provider`, this is intentionally keyed by backend host/port.
pub fn upsert_known_local_provider_model(
    backend_key: &str,
    provider: ProviderConfig,
    model: ModelConfig,
    activate: bool,
    source: &'static str,
) -> ProviderWriteResult<(String, String)> {
    let backend = known_local_backend(backend_key)
        .ok_or_else(|| ProviderWriteError::UnknownLocalBackend(backend_key.to_string()))?;
    mutate_config(("providers.upsert-local", source), move |store| {
        Ok(upsert_known_local_provider_model_in_config(
            store, &backend, provider, model, activate,
        ))
    })
    .map_err(map_config_error)
}

#[derive(Debug, Clone, Default)]
pub struct LocalProviderModelRemoval {
    pub removed_provider_model: bool,
    pub removed_provider: bool,
    pub removed_active_model: bool,
    pub removed_fallback_models: usize,
}

/// Remove one model from a known local backend provider and clean references
/// that would otherwise point at a deleted local Ollama tag.
pub fn remove_known_local_provider_model(
    backend_key: &str,
    model_id: &str,
    source: &'static str,
) -> ProviderWriteResult<LocalProviderModelRemoval> {
    let backend = known_local_backend(backend_key)
        .ok_or_else(|| ProviderWriteError::UnknownLocalBackend(backend_key.to_string()))?;
    let model_id = model_id.to_string();
    let result = mutate_config(("providers.remove-local-model", source), move |store| {
        Ok(remove_known_local_provider_model_in_config(
            store, &backend, &model_id,
        ))
    })
    .map_err(map_config_error)?;
    let repair = repair_hard_deleted_model_references();
    if repair.failures > 0 {
        crate::app_warn!(
            "provider",
            source,
            "local model reference repair partially failed: failures={}",
            repair.failures
        );
    }
    Ok(result)
}

fn remove_known_local_provider_model_in_config(
    store: &mut crate::config::AppConfig,
    backend: &KnownLocalBackend,
    model_id: &str,
) -> LocalProviderModelRemoval {
    let Some(provider_idx) = store
        .providers
        .iter()
        .position(|p| known_local_backend_matches(backend, &p.api_type, &p.base_url))
    else {
        return LocalProviderModelRemoval::default();
    };

    let provider_id = store.providers[provider_idx].id.clone();
    let removed_active_model = store
        .active_model
        .as_ref()
        .map(|active| active.provider_id == provider_id && active.model_id == model_id)
        .unwrap_or(false);
    let before = store.providers[provider_idx].models.len();
    store.providers[provider_idx]
        .models
        .retain(|model| model.id != model_id);
    let removed_provider_model = store.providers[provider_idx].models.len() != before;
    if !removed_provider_model {
        return LocalProviderModelRemoval::default();
    }

    let removed_provider = store.providers[provider_idx].models.is_empty();
    if removed_provider {
        store.providers.remove(provider_idx);
    }
    let reconciled = reconcile_model_references(store);

    LocalProviderModelRemoval {
        removed_provider_model,
        removed_provider,
        removed_active_model,
        removed_fallback_models: reconciled.removed_fallback_models,
    }
}

fn upsert_known_local_provider_model_in_config(
    store: &mut crate::config::AppConfig,
    backend: &KnownLocalBackend,
    mut provider: ProviderConfig,
    model: ModelConfig,
    activate: bool,
) -> (String, String) {
    let model_id = model.id.clone();
    let existing_idx = store
        .providers
        .iter()
        .position(|p| known_local_backend_matches(backend, &p.api_type, &p.base_url));

    let provider_id = if let Some(idx) = existing_idx {
        let existing = &mut store.providers[idx];
        push_model_if_missing(existing, model);
        existing.enabled = true;
        existing.allow_private_network = true;
        existing.id.clone()
    } else {
        if provider.id.is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
        provider.enabled = true;
        provider.allow_private_network = true;
        push_model_if_missing(&mut provider, model);
        let id = provider.id.clone();
        store.providers.push(provider);
        id
    };

    if activate {
        store.active_model = Some(ActiveModel {
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
        });
    }

    (provider_id, model_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::provider::ThinkingStyle;

    fn provider(base_url: &str) -> ProviderConfig {
        let mut p = ProviderConfig::new(
            "Ollama".into(),
            ApiType::OpenaiChat,
            base_url.into(),
            String::new(),
        );
        p.thinking_style = ThinkingStyle::Qwen;
        p
    }

    fn model(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.into(),
            name: id.into(),
            input_types: vec!["text".into()],
            context_window: 32_768,
            max_tokens: 8192,
            reasoning: true,
            thinking_style: None,
            cost_input: Some(0.0),
            cost_output: Some(0.0),
        }
    }

    fn configured_provider(
        id: &str,
        base_url: &str,
        enabled: bool,
        model_ids: &[&str],
    ) -> ProviderConfig {
        let mut configured = provider(base_url);
        configured.id = id.to_string();
        configured.enabled = enabled;
        configured.models = model_ids.iter().map(|id| model(id)).collect();
        configured
    }

    fn active(provider_id: &str, model_id: &str) -> ActiveModel {
        ActiveModel {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        }
    }

    fn assert_active_model(config: &AppConfig, provider_id: &str, model_id: &str) {
        let selected = config.active_model.as_ref().expect("active model");
        assert_eq!(selected.provider_id, provider_id);
        assert_eq!(selected.model_id, model_id);
    }

    fn assert_fallback_models(config: &AppConfig, expected: &[(&str, &str)]) {
        let actual: Vec<(&str, &str)> = config
            .fallback_models
            .iter()
            .map(|fallback| (fallback.provider_id.as_str(), fallback.model_id.as_str()))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn known_local_backend_matching_ignores_path() {
        let backend = known_local_backend("ollama").unwrap();
        assert!(known_local_backend_matches(
            &backend,
            &ApiType::OpenaiChat,
            "http://127.0.0.1:11434"
        ));
        assert!(known_local_backend_matches(
            &backend,
            &ApiType::OpenaiChat,
            "http://localhost:11434/v1"
        ));
        assert!(known_local_backend_matches(
            &backend,
            &ApiType::OpenaiChat,
            "http://[::1]:11434/api/tags"
        ));
        assert!(known_local_backend_matches(
            &backend,
            &ApiType::OpenaiChat,
            "http://ollama.local:11434"
        ));
        assert!(!known_local_backend_matches(
            &backend,
            &ApiType::OpenaiResponses,
            "http://localhost:11434"
        ));
        assert!(!known_local_backend_matches(
            &backend,
            &ApiType::OpenaiChat,
            "http://localhost:1234"
        ));
    }

    #[test]
    fn local_provider_upsert_dedupes_and_adds_models() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig::default();
        let mut existing = provider("http://localhost:11434/v1");
        existing.models.push(model("qwen3:8b"));
        let existing_id = existing.id.clone();
        cfg.providers.push(existing);

        upsert_known_local_provider_model_in_config(
            &mut cfg,
            &backend,
            provider("http://127.0.0.1:11434"),
            model("gemma4:e2b"),
            true,
        );
        upsert_known_local_provider_model_in_config(
            &mut cfg,
            &backend,
            provider("http://127.0.0.1:11434"),
            model("gemma4:e2b"),
            true,
        );

        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].id, existing_id);
        assert_eq!(cfg.providers[0].models.len(), 2);
        assert_eq!(cfg.active_model.as_ref().unwrap().model_id, "gemma4:e2b");
    }

    #[test]
    fn removing_active_local_model_promotes_first_available_fallback_and_prunes_refs() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![
                configured_provider(
                    "local",
                    "http://localhost:11434/v1",
                    true,
                    &["remove-me", "local-next"],
                ),
                configured_provider(
                    "disabled",
                    "https://disabled.example.com",
                    false,
                    &["disabled-model"],
                ),
                configured_provider(
                    "fallback",
                    "https://fallback.example.com",
                    true,
                    &["fallback-model"],
                ),
            ],
            active_model: Some(active("local", "remove-me")),
            fallback_models: vec![
                active("local", "remove-me"),
                active("disabled", "disabled-model"),
                active("fallback", "fallback-model"),
            ],
            ..Default::default()
        };

        let result = remove_known_local_provider_model_in_config(&mut cfg, &backend, "remove-me");

        let local = cfg
            .providers
            .iter()
            .find(|provider| provider.id == "local")
            .expect("local provider remains");
        assert_eq!(
            local
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["local-next"]
        );
        assert_active_model(&cfg, "fallback", "fallback-model");
        assert_fallback_models(&cfg, &[("disabled", "disabled-model")]);
        assert!(result.removed_provider_model);
        assert!(!result.removed_provider);
        assert!(result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 2);
    }

    #[test]
    fn removing_active_local_model_without_available_fallback_uses_provider_model_order() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![
                configured_provider("local", "http://127.0.0.1:11434", true, &["remove-me"]),
                configured_provider(
                    "disabled",
                    "https://disabled.example.com",
                    false,
                    &["disabled-model"],
                ),
                configured_provider(
                    "successor",
                    "https://successor.example.com",
                    true,
                    &["first-model", "second-model"],
                ),
                configured_provider("later", "https://later.example.com", true, &["later-model"]),
            ],
            active_model: Some(active("local", "remove-me")),
            fallback_models: vec![active("disabled", "disabled-model")],
            ..Default::default()
        };

        let result = remove_known_local_provider_model_in_config(&mut cfg, &backend, "remove-me");

        assert!(!cfg.providers.iter().any(|provider| provider.id == "local"));
        assert_active_model(&cfg, "successor", "first-model");
        assert_fallback_models(&cfg, &[("disabled", "disabled-model")]);
        assert!(result.removed_provider_model);
        assert!(result.removed_provider);
        assert!(result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 0);
    }

    #[test]
    fn removing_non_active_local_model_preserves_active_and_prunes_deleted_fallback() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![configured_provider(
                "local",
                "http://ollama.local:11434",
                true,
                &["active-model", "remove-me"],
            )],
            active_model: Some(active("local", "active-model")),
            fallback_models: vec![active("local", "remove-me")],
            ..Default::default()
        };

        let result = remove_known_local_provider_model_in_config(&mut cfg, &backend, "remove-me");

        let local = cfg
            .providers
            .iter()
            .find(|provider| provider.id == "local")
            .expect("local provider remains");
        assert_eq!(
            local
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["active-model"]
        );
        assert_active_model(&cfg, "local", "active-model");
        assert_fallback_models(&cfg, &[]);
        assert!(result.removed_provider_model);
        assert!(!result.removed_provider);
        assert!(!result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 1);
    }

    #[test]
    fn removing_local_model_can_select_successor_without_reporting_removed_active() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![
                configured_provider("local", "http://localhost:11434", true, &["remove-me"]),
                configured_provider(
                    "successor",
                    "https://successor.example.com",
                    true,
                    &["successor-model"],
                ),
            ],
            active_model: None,
            fallback_models: vec![],
            ..Default::default()
        };

        let result = remove_known_local_provider_model_in_config(&mut cfg, &backend, "remove-me");

        assert!(!cfg.providers.iter().any(|provider| provider.id == "local"));
        assert_active_model(&cfg, "successor", "successor-model");
        assert_fallback_models(&cfg, &[]);
        assert!(result.removed_provider_model);
        assert!(result.removed_provider);
        assert!(!result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 0);
    }

    #[test]
    fn removing_non_active_local_model_does_not_claim_unrelated_stale_active() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![
                configured_provider("local", "http://localhost:11434", true, &["remove-me"]),
                configured_provider(
                    "successor",
                    "https://successor.example.com",
                    true,
                    &["successor-model"],
                ),
            ],
            active_model: Some(active("missing", "stale-model")),
            fallback_models: vec![],
            ..Default::default()
        };

        let result = remove_known_local_provider_model_in_config(&mut cfg, &backend, "remove-me");

        assert!(!cfg.providers.iter().any(|provider| provider.id == "local"));
        assert_active_model(&cfg, "successor", "successor-model");
        assert_fallback_models(&cfg, &[]);
        assert!(result.removed_provider_model);
        assert!(result.removed_provider);
        assert!(!result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 0);
    }

    #[test]
    fn removing_unknown_local_model_is_a_noop() {
        let backend = known_local_backend("ollama").unwrap();
        let mut cfg = AppConfig {
            providers: vec![configured_provider(
                "local",
                "http://localhost:11434",
                true,
                &["active-model"],
            )],
            active_model: Some(active("local", "active-model")),
            fallback_models: vec![active("missing", "stale-model")],
            ..Default::default()
        };

        let result =
            remove_known_local_provider_model_in_config(&mut cfg, &backend, "not-installed");

        assert_active_model(&cfg, "local", "active-model");
        assert_fallback_models(&cfg, &[("missing", "stale-model")]);
        assert!(!result.removed_provider_model);
        assert!(!result.removed_provider);
        assert!(!result.removed_active_model);
        assert_eq!(result.removed_fallback_models, 0);
    }
}

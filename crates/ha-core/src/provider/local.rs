//! Known local/self-hosted provider backends.

use serde::{Deserialize, Serialize};

use crate::config::mutate_config;

use super::crud::{
    map_config_error, push_model_if_missing, ProviderWriteError, ProviderWriteResult,
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
            cost_input: 0.0,
            cost_output: 0.0,
        }
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
}

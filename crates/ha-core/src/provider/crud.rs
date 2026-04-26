//! Provider config write helpers.
//!
//! These functions are the single entry point for mutating the provider list
//! and active model in `AppConfig`. Tauri, HTTP, onboarding, importers, and
//! local-model flows should call this module instead of editing
//! `config.providers` directly.

use std::fmt;

use crate::config::{cached_config, mutate_config, AppConfig};

use super::helpers::{ensure_codex_provider, is_masked_key, merge_profile_keys};
use super::types::{ActiveModel, ApiType, ModelConfig, ProviderConfig};

pub type ProviderWriteResult<T> = Result<T, ProviderWriteError>;

#[derive(Debug)]
pub enum ProviderWriteError {
    NotFound(String),
    ModelNotFound {
        provider_id: String,
        model_id: String,
    },
    UnknownLocalBackend(String),
    Config(anyhow::Error),
}

impl fmt::Display for ProviderWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Provider not found: {id}"),
            Self::ModelNotFound { model_id, .. } => write!(f, "Model not found: {model_id}"),
            Self::UnknownLocalBackend(key) => write!(f, "Unknown local backend: {key}"),
            Self::Config(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for ProviderWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

fn into_anyhow(err: ProviderWriteError) -> anyhow::Error {
    anyhow::Error::new(err)
}

pub(crate) fn map_config_error(err: anyhow::Error) -> ProviderWriteError {
    match err.downcast::<ProviderWriteError>() {
        Ok(provider_err) => provider_err,
        Err(err) => ProviderWriteError::Config(err),
    }
}

/// How a provider helper should update `active_model`.
#[derive(Debug, Clone)]
pub enum ActiveModelUpdate {
    Always(String),
    IfMissing(String),
    Never,
}

/// Add a provider from the UI/API request shape. This intentionally generates
/// a fresh ID and always appends; local-backend upsert uses a separate helper.
pub fn add_provider(
    config: ProviderConfig,
    source: &'static str,
) -> ProviderWriteResult<ProviderConfig> {
    mutate_config(("providers.add", source), move |store| {
        let provider = add_provider_to_config(store, config);
        Ok(provider.masked())
    })
    .map_err(map_config_error)
}

/// Add an already-constructed provider and activate one of its models.
pub fn add_and_activate_provider(
    provider: ProviderConfig,
    model_id: String,
    source: &'static str,
) -> ProviderWriteResult<String> {
    mutate_config(("providers.add+activate", source), move |store| {
        add_and_activate_provider_in_config(store, provider, model_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Add several already-constructed providers, preserving their IDs.
pub fn add_many_providers(
    providers: Vec<ProviderConfig>,
    source: &'static str,
) -> ProviderWriteResult<Vec<String>> {
    if providers.is_empty() {
        return Ok(Vec::new());
    }
    mutate_config(("providers.add", source), move |store| {
        Ok(providers
            .into_iter()
            .map(|provider| add_existing_provider_to_config(store, provider))
            .collect())
    })
    .map_err(map_config_error)
}

pub fn update_provider(config: ProviderConfig, source: &'static str) -> ProviderWriteResult<()> {
    mutate_config(("providers.update", source), move |store| {
        update_provider_in_config(store, config).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete a single provider. Returns true when the deleted provider was active.
pub fn delete_provider(provider_id: String, source: &'static str) -> ProviderWriteResult<bool> {
    mutate_config(("providers.delete", source), move |store| {
        delete_provider_in_config(store, &provider_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete all providers of one API type. Returns true when active_model was cleared.
pub fn delete_providers_by_api_type(
    api_type: ApiType,
    source: &'static str,
) -> ProviderWriteResult<bool> {
    mutate_config(("providers.delete", source), move |store| {
        Ok(delete_providers_by_api_type_in_config(store, &api_type))
    })
    .map_err(map_config_error)
}

pub fn reorder_providers(
    provider_ids: Vec<String>,
    source: &'static str,
) -> ProviderWriteResult<()> {
    mutate_config(("providers.reorder", source), move |store| {
        reorder_providers_in_config(store, &provider_ids);
        Ok(())
    })
    .map_err(map_config_error)
}

/// Set the active model and return the matching provider snapshot.
pub fn set_active_model(
    provider_id: String,
    model_id: String,
    source: &'static str,
) -> ProviderWriteResult<ProviderConfig> {
    mutate_config(("active_model", source), move |store| {
        set_active_model_in_config(store, &provider_id, &model_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

pub fn ensure_codex_provider_persisted(
    active: ActiveModelUpdate,
    source: &'static str,
) -> ProviderWriteResult<String> {
    if let Some(provider_id) = codex_noop_provider_id(&active) {
        return Ok(provider_id);
    }

    mutate_config(("providers.codex", source), move |store| {
        let provider_id = ensure_codex_provider(store);
        match active {
            ActiveModelUpdate::Always(model_id) => {
                store.active_model = Some(ActiveModel {
                    provider_id: provider_id.clone(),
                    model_id,
                });
            }
            ActiveModelUpdate::IfMissing(model_id) if store.active_model.is_none() => {
                store.active_model = Some(ActiveModel {
                    provider_id: provider_id.clone(),
                    model_id,
                });
            }
            ActiveModelUpdate::IfMissing(_) | ActiveModelUpdate::Never => {}
        }
        Ok(provider_id)
    })
    .map_err(map_config_error)
}

fn codex_noop_provider_id(active: &ActiveModelUpdate) -> Option<String> {
    let store = cached_config();
    let provider = store
        .providers
        .iter()
        .find(|p| p.api_type == ApiType::Codex)?;
    if codex_provider_needs_backfill(provider) {
        return None;
    }
    match active {
        ActiveModelUpdate::Always(model_id) => match store.active_model.as_ref() {
            Some(current)
                if current.provider_id == provider.id && current.model_id == *model_id =>
            {
                Some(provider.id.clone())
            }
            _ => None,
        },
        ActiveModelUpdate::IfMissing(_) if store.active_model.is_some() => {
            Some(provider.id.clone())
        }
        ActiveModelUpdate::IfMissing(_) => None,
        ActiveModelUpdate::Never => Some(provider.id.clone()),
    }
}

fn codex_provider_needs_backfill(provider: &ProviderConfig) -> bool {
    default_codex_model_ids()
        .iter()
        .any(|id| !provider.models.iter().any(|m| &m.id == id))
}

fn default_codex_model_ids() -> &'static [&'static str] {
    &[
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.3-codex",
        "gpt-5.3-codex-spark",
        "gpt-5.2",
        "gpt-5.2-codex",
        "gpt-5.1",
        "gpt-5.1-codex-max",
        "gpt-5.1-codex-mini",
    ]
}

fn new_provider_from_add_request(config: ProviderConfig) -> ProviderConfig {
    let mut provider = ProviderConfig::new(
        config.name,
        config.api_type,
        config.base_url,
        config.api_key,
    );
    provider.models = config.models;
    provider.auth_profiles = config.auth_profiles;
    provider.thinking_style = config.thinking_style;
    provider.allow_private_network = config.allow_private_network;
    provider
}

pub(crate) fn add_provider_to_config(
    store: &mut AppConfig,
    config: ProviderConfig,
) -> ProviderConfig {
    let provider = new_provider_from_add_request(config);
    store.providers.push(provider.clone());
    provider
}

pub(crate) fn add_existing_provider_to_config(
    store: &mut AppConfig,
    mut provider: ProviderConfig,
) -> String {
    if provider.id.is_empty() {
        provider.id = uuid::Uuid::new_v4().to_string();
    }
    let id = provider.id.clone();
    store.providers.push(provider);
    id
}

pub(crate) fn add_and_activate_provider_in_config(
    store: &mut AppConfig,
    provider: ProviderConfig,
    model_id: String,
) -> ProviderWriteResult<String> {
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(ProviderWriteError::ModelNotFound {
            provider_id: provider.id.clone(),
            model_id,
        });
    }
    let provider_id = add_existing_provider_to_config(store, provider);
    store.active_model = Some(ActiveModel {
        provider_id: provider_id.clone(),
        model_id,
    });
    Ok(provider_id)
}

pub(crate) fn update_provider_in_config(
    store: &mut AppConfig,
    config: ProviderConfig,
) -> ProviderWriteResult<()> {
    let Some(existing) = store.providers.iter_mut().find(|p| p.id == config.id) else {
        return Err(ProviderWriteError::NotFound(config.id));
    };

    existing.name = config.name;
    existing.api_type = config.api_type;
    existing.base_url = config.base_url;
    if !is_masked_key(&config.api_key) {
        existing.api_key = config.api_key;
    }
    existing.auth_profiles = merge_profile_keys(&existing.auth_profiles, &config.auth_profiles);
    existing.models = config.models;
    existing.enabled = config.enabled;
    existing.user_agent = config.user_agent;
    existing.thinking_style = config.thinking_style;
    existing.allow_private_network = config.allow_private_network;
    Ok(())
}

pub(crate) fn delete_provider_in_config(
    store: &mut AppConfig,
    provider_id: &str,
) -> ProviderWriteResult<bool> {
    let len_before = store.providers.len();
    store.providers.retain(|p| p.id != provider_id);
    if store.providers.len() == len_before {
        return Err(ProviderWriteError::NotFound(provider_id.to_string()));
    }
    let removed_active = active_model_is_missing(store);
    if removed_active {
        store.active_model = None;
    }
    Ok(removed_active)
}

pub(crate) fn delete_providers_by_api_type_in_config(
    store: &mut AppConfig,
    api_type: &ApiType,
) -> bool {
    store.providers.retain(|p| &p.api_type != api_type);
    let removed_active = active_model_is_missing(store);
    if removed_active {
        store.active_model = None;
    }
    removed_active
}

pub(crate) fn reorder_providers_in_config(store: &mut AppConfig, provider_ids: &[String]) {
    let mut reordered = Vec::with_capacity(provider_ids.len());
    for id in provider_ids {
        if let Some(p) = store.providers.iter().find(|p| &p.id == id) {
            reordered.push(p.clone());
        }
    }
    for p in &store.providers {
        if !provider_ids.contains(&p.id) {
            reordered.push(p.clone());
        }
    }
    store.providers = reordered;
}

pub(crate) fn set_active_model_in_config(
    store: &mut AppConfig,
    provider_id: &str,
    model_id: &str,
) -> ProviderWriteResult<ProviderConfig> {
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .cloned()
        .ok_or_else(|| ProviderWriteError::NotFound(provider_id.to_string()))?;
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(ProviderWriteError::ModelNotFound {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        });
    }
    store.active_model = Some(ActiveModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    });
    Ok(provider)
}

fn active_model_is_missing(store: &AppConfig) -> bool {
    store
        .active_model
        .as_ref()
        .map(|active| !store.providers.iter().any(|p| p.id == active.provider_id))
        .unwrap_or(false)
}

pub(crate) fn push_model_if_missing(provider: &mut ProviderConfig, model: ModelConfig) {
    if !provider.models.iter().any(|m| m.id == model.id) {
        provider.models.push(model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ApiType, AuthProfile, ThinkingStyle};

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

    fn provider(name: &str, base_url: &str) -> ProviderConfig {
        let mut p = ProviderConfig::new(
            name.to_string(),
            ApiType::OpenaiChat,
            base_url.to_string(),
            "sk-real".to_string(),
        );
        p.models.push(model("m1"));
        p.thinking_style = ThinkingStyle::Openai;
        p
    }

    fn codex_provider() -> ProviderConfig {
        let mut cfg = AppConfig::default();
        let id = ensure_codex_provider(&mut cfg);
        cfg.providers.into_iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn add_provider_appends_even_with_same_base_url() {
        let mut cfg = AppConfig::default();
        let first = add_provider_to_config(&mut cfg, provider("A", "http://127.0.0.1:11434"));
        let second = add_provider_to_config(&mut cfg, provider("B", "http://127.0.0.1:11434"));

        assert_ne!(first.id, second.id);
        assert_eq!(cfg.providers.len(), 2);
    }

    #[test]
    fn update_provider_preserves_masked_keys() {
        let mut cfg = AppConfig::default();
        let mut existing = provider("A", "https://api.example.com");
        existing.auth_profiles = vec![AuthProfile {
            id: "profile-1".into(),
            label: "Work".into(),
            api_key: "profile-real-key".into(),
            base_url: None,
            enabled: true,
        }];
        let id = existing.id.clone();
        cfg.providers.push(existing);

        let mut incoming = cfg.providers[0].masked();
        incoming.name = "Renamed".into();
        incoming.auth_profiles[0].label = "Updated".into();
        update_provider_in_config(&mut cfg, incoming).unwrap();

        let updated = &cfg.providers[0];
        assert_eq!(updated.id, id);
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.api_key, "sk-real");
        assert_eq!(updated.auth_profiles[0].api_key, "profile-real-key");
        assert_eq!(updated.auth_profiles[0].label, "Updated");
    }

    #[test]
    fn delete_active_provider_clears_active_model() {
        let mut cfg = AppConfig::default();
        let p = provider("A", "https://api.example.com");
        let id = p.id.clone();
        cfg.providers.push(p);
        cfg.active_model = Some(ActiveModel {
            provider_id: id.clone(),
            model_id: "m1".into(),
        });

        assert!(delete_provider_in_config(&mut cfg, &id).unwrap());
        assert!(cfg.active_model.is_none());
    }

    #[test]
    fn set_active_model_validates_provider_and_model() {
        let mut cfg = AppConfig::default();
        let p = provider("A", "https://api.example.com");
        let id = p.id.clone();
        cfg.providers.push(p);

        assert!(matches!(
            set_active_model_in_config(&mut cfg, "missing", "m1"),
            Err(ProviderWriteError::NotFound(_))
        ));
        assert!(matches!(
            set_active_model_in_config(&mut cfg, &id, "missing"),
            Err(ProviderWriteError::ModelNotFound { .. })
        ));

        let found = set_active_model_in_config(&mut cfg, &id, "m1").unwrap();
        assert_eq!(found.id, id);
        assert_eq!(cfg.active_model.as_ref().unwrap().model_id, "m1");
    }

    #[test]
    fn codex_noop_detects_existing_provider_and_active_model() {
        let mut cfg = AppConfig::default();
        let provider = codex_provider();
        let id = provider.id.clone();
        cfg.providers.push(provider);
        cfg.active_model = Some(ActiveModel {
            provider_id: id.clone(),
            model_id: "gpt-5.4".into(),
        });

        let found = cfg
            .providers
            .iter()
            .find(|p| p.api_type == ApiType::Codex)
            .unwrap();
        assert!(!codex_provider_needs_backfill(found));
        assert_eq!(
            default_codex_model_ids().len(),
            found.models.len(),
            "keep no-op model-id list in sync with default Codex models"
        );
    }

    #[test]
    fn codex_backfill_detection_fires_when_default_model_is_missing() {
        let mut provider = codex_provider();
        provider.models.retain(|m| m.id != "gpt-5.5");
        assert!(codex_provider_needs_backfill(&provider));
    }
}

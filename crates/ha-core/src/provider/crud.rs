//! Provider config write helpers.
//!
//! These functions are the single entry point for mutating the provider list
//! and active model in `AppConfig`. Tauri, HTTP, onboarding, importers, and
//! local-model flows should call this module instead of editing
//! `config.providers` directly.

use std::fmt;

use crate::config::{cached_config, mutate_config, AppConfig};

use super::helpers::{
    ensure_codex_provider, first_available_model, is_masked_key, merge_profile_keys,
    model_ref_exists, model_ref_is_available,
};
use super::types::{ActiveModel, ApiType, ModelConfig, ProviderConfig};

pub type ProviderWriteResult<T> = Result<T, ProviderWriteError>;

#[derive(Debug)]
pub enum ProviderWriteError {
    NotFound(String),
    ModelNotFound {
        provider_id: String,
        model_id: String,
    },
    ProviderUnavailable(String),
    UnknownLocalBackend(String),
    Config(anyhow::Error),
}

impl fmt::Display for ProviderWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Provider not found: {id}"),
            Self::ModelNotFound { model_id, .. } => write!(f, "Model not found: {model_id}"),
            Self::ProviderUnavailable(id) => {
                write!(f, "Provider is disabled and unavailable: {id}")
            }
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

/// Update a provider. Returns whether the cached active Agent must be invalidated.
/// This is true when the active Provider was updated or reconciliation changed
/// the active model reference.
pub fn update_provider(config: ProviderConfig, source: &'static str) -> ProviderWriteResult<bool> {
    mutate_config(("providers.update", source), move |store| {
        update_provider_in_config(store, config).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete a single provider. Returns whether the active model changed.
pub fn delete_provider(provider_id: String, source: &'static str) -> ProviderWriteResult<bool> {
    mutate_config(("providers.delete", source), move |store| {
        delete_provider_in_config(store, &provider_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete all providers of one API type. Returns whether the active model changed.
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
        apply_codex_active_model_update(store, &provider_id, active).map_err(into_anyhow)?;
        Ok(provider_id)
    })
    .map_err(map_config_error)
}

fn apply_codex_active_model_update(
    store: &mut AppConfig,
    provider_id: &str,
    active: ActiveModelUpdate,
) -> ProviderWriteResult<()> {
    match active {
        ActiveModelUpdate::Always(model_id) => {
            if let Some(provider) = store.providers.iter_mut().find(|p| p.id == provider_id) {
                provider.enabled = true;
            }
            set_active_model_in_config(store, provider_id, &model_id).map(|_| ())
        }
        ActiveModelUpdate::IfMissing(model_id) if store.active_model.is_none() => {
            let candidate = ActiveModel {
                provider_id: provider_id.to_string(),
                model_id,
            };
            if model_ref_is_available(&store.providers, &candidate) {
                store.active_model = Some(candidate);
            }
            Ok(())
        }
        ActiveModelUpdate::IfMissing(_) | ActiveModelUpdate::Never => Ok(()),
    }
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
                if provider.enabled
                    && current.provider_id == provider.id
                    && current.model_id == *model_id
                    && model_ref_is_available(&store.providers, current) =>
            {
                Some(provider.id.clone())
            }
            _ => None,
        },
        ActiveModelUpdate::IfMissing(_) if store.active_model.is_some() => {
            Some(provider.id.clone())
        }
        ActiveModelUpdate::IfMissing(_) if !provider.enabled => Some(provider.id.clone()),
        ActiveModelUpdate::IfMissing(_) => None,
        ActiveModelUpdate::Never => Some(provider.id.clone()),
    }
}

fn codex_provider_needs_backfill(provider: &ProviderConfig) -> bool {
    let defaults = default_codex_model_ids();
    // Missing any default model → needs backfill.
    if defaults
        .iter()
        .any(|id| !provider.models.iter().any(|m| &m.id == id))
    {
        return true;
    }
    // All defaults present but not in canonical order → needs reorder. Older
    // configs appended newly-added defaults (e.g. gpt-5.5) to the tail; without
    // this the noop short-circuit skips ensure_codex_provider and the newest
    // model stays buried at the bottom of the picker forever.
    let current: Vec<&str> = provider
        .models
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| defaults.contains(id))
        .collect();
    current.as_slice() != defaults
}

fn default_codex_model_ids() -> &'static [&'static str] {
    &[
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
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

fn new_provider_from_add_request(mut config: ProviderConfig) -> ProviderConfig {
    config.sanitize();
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
    // Carry over the request's own values rather than letting them silently
    // fall back to `ProviderConfig::new` defaults — otherwise a custom
    // User-Agent (already trimmed by sanitize) or `enabled: false` from the add
    // request would be dropped, diverging from the update path.
    provider.user_agent = config.user_agent;
    provider.enabled = config.enabled;
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

/// Returns whether the cached active Agent must be invalidated.
pub(crate) fn update_provider_in_config(
    store: &mut AppConfig,
    mut config: ProviderConfig,
) -> ProviderWriteResult<bool> {
    config.sanitize();
    let updated_active_provider = store
        .active_model
        .as_ref()
        .map(|active| active.provider_id == config.id)
        .unwrap_or(false);
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
    let active_model_changed = reconcile_model_references(store).active_model_changed;
    let active_agent_invalidated = updated_active_provider || active_model_changed;
    Ok(active_agent_invalidated)
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
    Ok(reconcile_model_references(store).active_model_changed)
}

pub(crate) fn delete_providers_by_api_type_in_config(
    store: &mut AppConfig,
    api_type: &ApiType,
) -> bool {
    store.providers.retain(|p| &p.api_type != api_type);
    reconcile_model_references(store).active_model_changed
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
    if !provider.enabled {
        return Err(ProviderWriteError::ProviderUnavailable(
            provider_id.to_string(),
        ));
    }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ModelReferenceReconcile {
    pub(crate) active_model_changed: bool,
    /// Number removed, including hard-invalid refs and active/fallback dedupes.
    pub(crate) removed_fallback_models: usize,
}

pub(crate) fn reconcile_model_references(store: &mut AppConfig) -> ModelReferenceReconcile {
    let previous_active = store.active_model.clone();
    let fallback_count_before = store.fallback_models.len();

    store
        .fallback_models
        .retain(|model| model_ref_exists(&store.providers, model));

    let active_model = store
        .active_model
        .clone()
        .filter(|model| model_ref_is_available(&store.providers, model))
        .or_else(|| {
            store
                .fallback_models
                .iter()
                .find(|model| model_ref_is_available(&store.providers, model))
                .cloned()
        })
        .or_else(|| first_available_model(&store.providers));

    store.active_model = active_model;
    if let Some(active) = store.active_model.as_ref() {
        store
            .fallback_models
            .retain(|fallback| !same_model_ref(fallback, active));
    }

    ModelReferenceReconcile {
        active_model_changed: !same_optional_model_ref(
            previous_active.as_ref(),
            store.active_model.as_ref(),
        ),
        removed_fallback_models: fallback_count_before - store.fallback_models.len(),
    }
}

fn same_optional_model_ref(left: Option<&ActiveModel>, right: Option<&ActiveModel>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_model_ref(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn same_model_ref(left: &ActiveModel, right: &ActiveModel) -> bool {
    left.provider_id == right.provider_id && left.model_id == right.model_id
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
        provider_with_models(name, ApiType::OpenaiChat, base_url, &["m1"])
    }

    fn provider_with_models(
        name: &str,
        api_type: ApiType,
        base_url: &str,
        model_ids: &[&str],
    ) -> ProviderConfig {
        let mut p = ProviderConfig::new(
            name.to_string(),
            api_type,
            base_url.to_string(),
            "sk-real".to_string(),
        );
        p.models = model_ids.iter().map(|id| model(id)).collect();
        p.thinking_style = ThinkingStyle::Openai;
        p
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
    fn updating_active_provider_details_invalidates_cached_agent_without_changing_active_ref() {
        let mut cfg = AppConfig::default();
        let active_provider = provider("A", "https://old.example.com");
        let active_id = active_provider.id.clone();
        cfg.providers.push(active_provider);
        cfg.active_model = Some(active(&active_id, "m1"));

        let mut updated = cfg.providers[0].clone();
        updated.base_url = "https://new.example.com".into();
        updated.api_key = "sk-updated".into();
        updated.thinking_style = ThinkingStyle::Qwen;

        let active_agent_invalidated = update_provider_in_config(&mut cfg, updated).unwrap();

        assert!(active_agent_invalidated);
        assert_active_model(&cfg, &active_id, "m1");
        assert_eq!(cfg.providers[0].base_url, "https://new.example.com");
        assert_eq!(cfg.providers[0].api_key, "sk-updated");
        assert_eq!(cfg.providers[0].thinking_style, ThinkingStyle::Qwen);
    }

    #[test]
    fn delete_active_provider_promotes_first_available_fallback() {
        let mut cfg = AppConfig::default();
        let active_provider = provider("A", "https://a.example.com");
        let fallback_provider = provider_with_models(
            "B",
            ApiType::Anthropic,
            "https://b.example.com",
            &["b-first", "b-second"],
        );
        let later_provider = provider("C", "https://c.example.com");
        let active_id = active_provider.id.clone();
        let fallback_id = fallback_provider.id.clone();
        let later_id = later_provider.id.clone();
        cfg.providers = vec![active_provider, fallback_provider, later_provider];
        cfg.active_model = Some(active(&active_id, "m1"));
        cfg.fallback_models = vec![
            active(&active_id, "m1"),
            active(&fallback_id, "b-second"),
            active(&later_id, "m1"),
        ];

        assert!(delete_provider_in_config(&mut cfg, &active_id).unwrap());

        assert_active_model(&cfg, &fallback_id, "b-second");
        assert_fallback_models(&cfg, &[(later_id.as_str(), "m1")]);
    }

    #[test]
    fn delete_active_provider_without_available_fallback_uses_provider_and_model_order() {
        let mut cfg = AppConfig::default();
        let active_provider = provider("A", "https://a.example.com");
        let mut disabled_provider = provider("B", "https://b.example.com");
        disabled_provider.enabled = false;
        let successor_provider = provider_with_models(
            "C",
            ApiType::OpenaiResponses,
            "https://c.example.com",
            &["c-first", "c-second"],
        );
        let active_id = active_provider.id.clone();
        let disabled_id = disabled_provider.id.clone();
        let successor_id = successor_provider.id.clone();
        cfg.providers = vec![active_provider, disabled_provider, successor_provider];
        cfg.active_model = Some(active(&active_id, "m1"));
        cfg.fallback_models = vec![active(&disabled_id, "m1")];

        assert!(delete_provider_in_config(&mut cfg, &active_id).unwrap());

        assert_active_model(&cfg, &successor_id, "c-first");
        assert_fallback_models(&cfg, &[(disabled_id.as_str(), "m1")]);
    }

    #[test]
    fn disabling_active_provider_selects_successor_and_preserves_fallbacks() {
        let mut cfg = AppConfig::default();
        let active_provider = provider("A", "https://a.example.com");
        let fallback_provider = provider("B", "https://b.example.com");
        let later_provider = provider("C", "https://c.example.com");
        let active_id = active_provider.id.clone();
        let fallback_id = fallback_provider.id.clone();
        let later_id = later_provider.id.clone();
        cfg.providers = vec![active_provider, fallback_provider, later_provider];
        cfg.active_model = Some(active(&active_id, "m1"));
        cfg.fallback_models = vec![
            active(&active_id, "m1"),
            active(&later_id, "m1"),
            active(&fallback_id, "m1"),
        ];

        let mut disabled = cfg.providers[0].clone();
        disabled.enabled = false;
        assert!(update_provider_in_config(&mut cfg, disabled).unwrap());

        assert_active_model(&cfg, &later_id, "m1");
        assert_fallback_models(
            &cfg,
            &[(active_id.as_str(), "m1"), (fallback_id.as_str(), "m1")],
        );
    }

    #[test]
    fn updating_provider_to_remove_active_model_selects_successor_and_prunes_removed_model() {
        let mut cfg = AppConfig::default();
        let active_provider = provider_with_models(
            "A",
            ApiType::OpenaiChat,
            "https://a.example.com",
            &["a-active", "a-next"],
        );
        let fallback_provider = provider("B", "https://b.example.com");
        let active_id = active_provider.id.clone();
        let fallback_id = fallback_provider.id.clone();
        cfg.providers = vec![active_provider, fallback_provider];
        cfg.active_model = Some(active(&active_id, "a-active"));
        cfg.fallback_models = vec![
            active(&active_id, "a-active"),
            active(&fallback_id, "m1"),
            active(&active_id, "a-next"),
        ];

        let mut updated = cfg.providers[0].clone();
        updated.models.remove(0);
        assert!(update_provider_in_config(&mut cfg, updated).unwrap());

        assert_active_model(&cfg, &fallback_id, "m1");
        assert_fallback_models(&cfg, &[(active_id.as_str(), "a-next")]);
    }

    #[test]
    fn updating_non_active_provider_preserves_active_model() {
        let mut cfg = AppConfig::default();
        let active_provider = provider("A", "https://a.example.com");
        let other_provider = provider("B", "https://b.example.com");
        let active_id = active_provider.id.clone();
        let other_id = other_provider.id.clone();
        cfg.providers = vec![active_provider, other_provider];
        cfg.active_model = Some(active(&active_id, "m1"));
        cfg.fallback_models = vec![active(&other_id, "m1")];

        let mut updated = cfg.providers[1].clone();
        updated.name = "B renamed".into();
        assert!(!update_provider_in_config(&mut cfg, updated).unwrap());

        assert_active_model(&cfg, &active_id, "m1");
        assert_fallback_models(&cfg, &[(other_id.as_str(), "m1")]);
    }

    #[test]
    fn deleting_last_available_provider_clears_active_and_invalid_fallbacks() {
        let mut cfg = AppConfig::default();
        let only_provider = provider("A", "https://a.example.com");
        let only_id = only_provider.id.clone();
        cfg.providers.push(only_provider);
        cfg.active_model = Some(active(&only_id, "m1"));
        cfg.fallback_models = vec![active(&only_id, "m1")];

        assert!(delete_provider_in_config(&mut cfg, &only_id).unwrap());

        assert!(cfg.active_model.is_none());
        assert_fallback_models(&cfg, &[]);
    }

    #[test]
    fn reenabling_only_provider_restores_active_model_using_first_model() {
        let mut cfg = AppConfig::default();
        let mut only_provider = provider_with_models(
            "A",
            ApiType::OpenaiChat,
            "https://a.example.com",
            &["a-first", "a-second"],
        );
        only_provider.enabled = false;
        let only_id = only_provider.id.clone();
        cfg.providers.push(only_provider);

        let mut enabled = cfg.providers[0].clone();
        enabled.enabled = true;
        assert!(update_provider_in_config(&mut cfg, enabled).unwrap());

        assert_active_model(&cfg, &only_id, "a-first");
        assert_fallback_models(&cfg, &[]);
    }

    #[test]
    fn deleting_api_type_selects_remaining_provider_and_prunes_deleted_fallbacks() {
        let mut cfg = AppConfig::default();
        let removed_first = provider("A", "https://a.example.com");
        let removed_active = provider("B", "https://b.example.com");
        let remaining = provider_with_models(
            "C",
            ApiType::Anthropic,
            "https://c.example.com",
            &["c-first", "c-second"],
        );
        let removed_first_id = removed_first.id.clone();
        let removed_active_id = removed_active.id.clone();
        let remaining_id = remaining.id.clone();
        cfg.providers = vec![removed_first, removed_active, remaining];
        cfg.active_model = Some(active(&removed_active_id, "m1"));
        cfg.fallback_models = vec![
            active(&removed_first_id, "m1"),
            active(&remaining_id, "c-second"),
            active(&removed_active_id, "m1"),
        ];

        assert!(delete_providers_by_api_type_in_config(
            &mut cfg,
            &ApiType::OpenaiChat
        ));

        assert_active_model(&cfg, &remaining_id, "c-second");
        assert_fallback_models(&cfg, &[]);
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
    fn set_active_model_rejects_disabled_provider_and_preserves_selection() {
        let mut cfg = AppConfig::default();
        let current_provider = provider("A", "https://a.example.com");
        let mut disabled_provider = provider("B", "https://b.example.com");
        disabled_provider.enabled = false;
        let current_id = current_provider.id.clone();
        let disabled_id = disabled_provider.id.clone();
        cfg.providers = vec![current_provider, disabled_provider];
        cfg.active_model = Some(active(&current_id, "m1"));

        assert!(matches!(
            set_active_model_in_config(&mut cfg, &disabled_id, "m1"),
            Err(ProviderWriteError::ProviderUnavailable(provider_id))
                if provider_id == disabled_id
        ));
        assert_active_model(&cfg, &current_id, "m1");
        assert_fallback_models(&cfg, &[]);
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

    #[test]
    fn codex_if_missing_keeps_disabled_provider_inactive() {
        let provider = ProviderConfig {
            enabled: false,
            ..codex_provider()
        };
        let provider_id = provider.id.clone();
        let mut cfg = AppConfig {
            providers: vec![provider],
            ..AppConfig::default()
        };

        apply_codex_active_model_update(
            &mut cfg,
            &provider_id,
            ActiveModelUpdate::IfMissing("gpt-5.4".into()),
        )
        .unwrap();

        assert!(!cfg.providers[0].enabled);
        assert!(cfg.active_model.is_none());
    }

    #[test]
    fn codex_always_reenables_provider_and_activates_model() {
        let provider = ProviderConfig {
            enabled: false,
            ..codex_provider()
        };
        let provider_id = provider.id.clone();
        let mut cfg = AppConfig {
            providers: vec![provider],
            ..AppConfig::default()
        };

        apply_codex_active_model_update(
            &mut cfg,
            &provider_id,
            ActiveModelUpdate::Always("gpt-5.4".into()),
        )
        .unwrap();

        assert!(cfg.providers[0].enabled);
        assert_active_model(&cfg, &provider_id, "gpt-5.4");
    }

    #[test]
    fn codex_never_keeps_disabled_provider_inactive() {
        let provider = ProviderConfig {
            enabled: false,
            ..codex_provider()
        };
        let provider_id = provider.id.clone();
        let mut cfg = AppConfig {
            providers: vec![provider],
            ..AppConfig::default()
        };

        apply_codex_active_model_update(
            &mut cfg,
            &provider_id,
            ActiveModelUpdate::Never,
        )
        .unwrap();

        assert!(!cfg.providers[0].enabled);
        assert!(cfg.active_model.is_none());
    }
}

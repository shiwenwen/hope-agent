//! STT subsystem write helpers.
//!
//! Single entry-point for mutating `AppConfig.stt`. Every callsite —
//! Tauri, HTTP, settings tool, onboarding — must come through here so that
//! all writes are serialized through `mutate_config` and emit
//! `config:changed`. Mirrors `provider::crud` shape.

use std::fmt;

use crate::config::{mutate_config, AppConfig};
use crate::provider::{is_masked_key, merge_profile_keys};

use super::local::{
    known_local_stt_backend, known_local_stt_backend_matches, KnownLocalSttBackend,
};
use super::types::{ActiveSttModel, SttModelConfig, SttProviderConfig};

pub type SttWriteResult<T> = Result<T, SttWriteError>;

#[derive(Debug)]
pub enum SttWriteError {
    NotFound(String),
    ModelNotFound {
        provider_id: String,
        model_id: String,
    },
    UnknownLocalBackend(String),
    Config(anyhow::Error),
}

impl fmt::Display for SttWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "STT provider not found: {id}"),
            Self::ModelNotFound { model_id, .. } => write!(f, "STT model not found: {model_id}"),
            Self::UnknownLocalBackend(key) => {
                write!(f, "Unknown local STT backend: {key}")
            }
            Self::Config(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for SttWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

fn into_anyhow(err: SttWriteError) -> anyhow::Error {
    anyhow::Error::new(err)
}

fn map_config_error(err: anyhow::Error) -> SttWriteError {
    match err.downcast::<SttWriteError>() {
        Ok(stt_err) => stt_err,
        Err(err) => SttWriteError::Config(err),
    }
}

// ── Public API ────────────────────────────────────────────────────

pub fn add_stt_provider(
    config: SttProviderConfig,
    source: &'static str,
) -> SttWriteResult<SttProviderConfig> {
    mutate_config(("stt.add", source), move |store| {
        let provider = add_stt_provider_in_config(store, config);
        Ok(provider.masked())
    })
    .map_err(map_config_error)
}

pub fn update_stt_provider(config: SttProviderConfig, source: &'static str) -> SttWriteResult<()> {
    mutate_config(("stt.update", source), move |store| {
        update_stt_provider_in_config(store, config).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete a single STT provider. Returns true when the deleted provider was
/// referenced by `active_model` / `im_fallback_model` (caller may want to
/// re-pick).
pub fn delete_stt_provider(provider_id: String, source: &'static str) -> SttWriteResult<bool> {
    mutate_config(("stt.delete", source), move |store| {
        delete_stt_provider_in_config(store, &provider_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

pub fn reorder_stt_providers(
    provider_ids: Vec<String>,
    source: &'static str,
) -> SttWriteResult<()> {
    mutate_config(("stt.reorder", source), move |store| {
        reorder_stt_providers_in_config(store, &provider_ids);
        Ok(())
    })
    .map_err(map_config_error)
}

pub fn set_active_stt_model(
    provider_id: String,
    model_id: String,
    source: &'static str,
) -> SttWriteResult<SttProviderConfig> {
    mutate_config(("stt.active", source), move |store| {
        set_active_stt_model_in_config(store, &provider_id, &model_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

pub fn clear_active_stt_model(source: &'static str) -> SttWriteResult<()> {
    mutate_config(("stt.active", source), move |store| {
        store.stt.active_model = None;
        Ok(())
    })
    .map_err(map_config_error)
}

pub fn set_stt_fallback_models(
    chain: Vec<ActiveSttModel>,
    source: &'static str,
) -> SttWriteResult<()> {
    mutate_config(("stt.fallback", source), move |store| {
        store.stt.fallback_models = chain;
        Ok(())
    })
    .map_err(map_config_error)
}

pub fn set_im_fallback_stt_model(
    selection: Option<ActiveSttModel>,
    source: &'static str,
) -> SttWriteResult<()> {
    mutate_config(("stt.im_fallback", source), move |store| {
        store.stt.im_fallback_model = selection;
        Ok(())
    })
    .map_err(map_config_error)
}

/// Upsert a single model into a known local backend provider.
///
/// Returns `(provider_id, model_id)`. When the provider already exists
/// (matched by host/port from the backend catalog), the model is appended
/// if missing and `enabled` + `allow_private_network` are flipped on. When
/// no matching provider exists, `provider` is inserted and seeded with the
/// given model.
pub fn upsert_known_local_stt_provider(
    backend_key: &str,
    provider: SttProviderConfig,
    model: SttModelConfig,
    activate: bool,
    source: &'static str,
) -> SttWriteResult<(String, String)> {
    let backend = known_local_stt_backend(backend_key)
        .ok_or_else(|| SttWriteError::UnknownLocalBackend(backend_key.to_string()))?;
    mutate_config(("stt.upsert-local", source), move |store| {
        Ok(upsert_known_local_stt_provider_in_config(
            store, &backend, provider, model, activate,
        ))
    })
    .map_err(map_config_error)
}

// ── In-config helpers (pure, easy to unit-test) ───────────────────

pub(crate) fn add_stt_provider_in_config(
    store: &mut AppConfig,
    mut config: SttProviderConfig,
) -> SttProviderConfig {
    if config.id.is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }
    store.stt.providers.push(config.clone());
    config
}

pub(crate) fn update_stt_provider_in_config(
    store: &mut AppConfig,
    config: SttProviderConfig,
) -> SttWriteResult<()> {
    let Some(existing) = store.stt.providers.iter_mut().find(|p| p.id == config.id) else {
        return Err(SttWriteError::NotFound(config.id));
    };

    existing.name = config.name;
    existing.kind = config.kind;
    existing.base_url = config.base_url;
    if !is_masked_key(&config.api_key) {
        existing.api_key = config.api_key;
    }
    existing.auth_profiles = merge_profile_keys(&existing.auth_profiles, &config.auth_profiles);
    existing.models = config.models;
    existing.enabled = config.enabled;
    existing.allow_private_network = config.allow_private_network;
    // For `extra` map: incoming masked values should not overwrite real
    // values (same contract as api_key). Keys present in incoming with a
    // non-masked value replace; masked values are dropped in favour of
    // existing.
    let mut merged_extra = existing.extra.clone();
    for (key, value) in &config.extra {
        if is_masked_key(value) {
            continue;
        }
        merged_extra.insert(key.clone(), value.clone());
    }
    // Remove keys the incoming map dropped entirely (caller sent the full
    // map). We can't tell "untouched masked" from "deleted" otherwise, so
    // a delete = "absent from incoming map".
    merged_extra.retain(|k, _| config.extra.contains_key(k));
    existing.extra = merged_extra;
    Ok(())
}

pub(crate) fn delete_stt_provider_in_config(
    store: &mut AppConfig,
    provider_id: &str,
) -> SttWriteResult<bool> {
    let len_before = store.stt.providers.len();
    store.stt.providers.retain(|p| p.id != provider_id);
    if store.stt.providers.len() == len_before {
        return Err(SttWriteError::NotFound(provider_id.to_string()));
    }
    let mut touched = false;
    if store
        .stt
        .active_model
        .as_ref()
        .is_some_and(|m| m.provider_id == provider_id)
    {
        store.stt.active_model = None;
        touched = true;
    }
    if store
        .stt
        .im_fallback_model
        .as_ref()
        .is_some_and(|m| m.provider_id == provider_id)
    {
        store.stt.im_fallback_model = None;
        touched = true;
    }
    let fallback_before = store.stt.fallback_models.len();
    store
        .stt
        .fallback_models
        .retain(|m| m.provider_id != provider_id);
    if store.stt.fallback_models.len() != fallback_before {
        touched = true;
    }
    Ok(touched)
}

pub(crate) fn reorder_stt_providers_in_config(store: &mut AppConfig, provider_ids: &[String]) {
    let mut reordered = Vec::with_capacity(provider_ids.len());
    for id in provider_ids {
        if let Some(p) = store.stt.providers.iter().find(|p| &p.id == id) {
            reordered.push(p.clone());
        }
    }
    for p in &store.stt.providers {
        if !provider_ids.contains(&p.id) {
            reordered.push(p.clone());
        }
    }
    store.stt.providers = reordered;
}

pub(crate) fn set_active_stt_model_in_config(
    store: &mut AppConfig,
    provider_id: &str,
    model_id: &str,
) -> SttWriteResult<SttProviderConfig> {
    let provider = store
        .stt
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .cloned()
        .ok_or_else(|| SttWriteError::NotFound(provider_id.to_string()))?;
    if !provider.models.iter().any(|m| m.id == model_id) {
        return Err(SttWriteError::ModelNotFound {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        });
    }
    store.stt.active_model = Some(ActiveSttModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    });
    Ok(provider)
}

pub(crate) fn upsert_known_local_stt_provider_in_config(
    store: &mut AppConfig,
    backend: &KnownLocalSttBackend,
    mut provider: SttProviderConfig,
    model: SttModelConfig,
    activate: bool,
) -> (String, String) {
    let model_id = model.id.clone();
    let existing_idx = store
        .stt
        .providers
        .iter()
        .position(|p| known_local_stt_backend_matches(backend, p.kind, &p.base_url));

    let provider_id = if let Some(idx) = existing_idx {
        let existing = &mut store.stt.providers[idx];
        if !existing.models.iter().any(|m| m.id == model.id) {
            existing.models.push(model);
        }
        existing.enabled = true;
        existing.allow_private_network = true;
        existing.id.clone()
    } else {
        if provider.id.is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
        provider.kind = backend.kind;
        provider.enabled = true;
        provider.allow_private_network = true;
        if !provider.models.iter().any(|m| m.id == model.id) {
            provider.models.push(model);
        }
        let id = provider.id.clone();
        store.stt.providers.push(provider);
        id
    };

    if activate {
        store.stt.active_model = Some(ActiveSttModel {
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
    use crate::provider::AuthProfile;
    use crate::stt::local::{FUNASR_KEY, WHISPER_CPP_KEY};
    use crate::stt::types::SttProviderKind;

    fn make_provider(name: &str, base_url: &str) -> SttProviderConfig {
        let mut p = SttProviderConfig::new(name, SttProviderKind::OpenaiTranscriptions, base_url);
        p.api_key = "sk-real".into();
        p.models.push(SttModelConfig::new("whisper-1", "Whisper"));
        p
    }

    #[test]
    fn add_then_update_preserves_real_keys_when_incoming_masked() {
        let mut cfg = AppConfig::default();
        let mut p = make_provider("OpenAI", "https://api.openai.com");
        p.auth_profiles = vec![AuthProfile::new(
            "Org".into(),
            "real-profile-key".into(),
            None,
        )];
        let added = add_stt_provider_in_config(&mut cfg, p);

        // Simulate UI round-trip: send back masked values.
        let mut incoming = added.masked();
        incoming.name = "Renamed".into();
        update_stt_provider_in_config(&mut cfg, incoming).unwrap();

        let stored = &cfg.stt.providers[0];
        assert_eq!(stored.name, "Renamed");
        assert_eq!(stored.api_key, "sk-real");
        assert_eq!(stored.auth_profiles[0].api_key, "real-profile-key");
    }

    #[test]
    fn extra_merge_keeps_existing_when_incoming_value_masked() {
        let mut cfg = AppConfig::default();
        let mut p = SttProviderConfig::new(
            "Volcengine",
            SttProviderKind::VolcengineWs,
            "wss://openspeech.bytedance.com",
        );
        p.extra
            .insert("access_key".into(), "real-access-key".into());
        let added = add_stt_provider_in_config(&mut cfg, p);

        let mut incoming = added.masked();
        // Mask is replaced when caller sends a fresh value.
        incoming
            .extra
            .insert("app_id".into(), "new-app-id-1234".into());
        update_stt_provider_in_config(&mut cfg, incoming).unwrap();

        let stored = &cfg.stt.providers[0];
        assert_eq!(stored.extra["access_key"], "real-access-key");
        assert_eq!(stored.extra["app_id"], "new-app-id-1234");
    }

    #[test]
    fn delete_clears_references_in_active_fallback_and_im_fallback() {
        let mut cfg = AppConfig::default();
        let p = make_provider("OpenAI", "https://api.openai.com");
        let pid = p.id.clone();
        cfg.stt.providers.push(p);
        cfg.stt.active_model = Some(ActiveSttModel {
            provider_id: pid.clone(),
            model_id: "whisper-1".into(),
        });
        cfg.stt.fallback_models.push(ActiveSttModel {
            provider_id: pid.clone(),
            model_id: "whisper-1".into(),
        });
        cfg.stt.im_fallback_model = Some(ActiveSttModel {
            provider_id: pid.clone(),
            model_id: "whisper-1".into(),
        });

        assert!(delete_stt_provider_in_config(&mut cfg, &pid).unwrap());
        assert!(cfg.stt.active_model.is_none());
        assert!(cfg.stt.fallback_models.is_empty());
        assert!(cfg.stt.im_fallback_model.is_none());
    }

    #[test]
    fn set_active_validates_provider_and_model() {
        let mut cfg = AppConfig::default();
        let p = make_provider("OpenAI", "https://api.openai.com");
        let pid = p.id.clone();
        cfg.stt.providers.push(p);

        assert!(matches!(
            set_active_stt_model_in_config(&mut cfg, "missing", "whisper-1"),
            Err(SttWriteError::NotFound(_))
        ));
        assert!(matches!(
            set_active_stt_model_in_config(&mut cfg, &pid, "missing"),
            Err(SttWriteError::ModelNotFound { .. })
        ));
        let provider = set_active_stt_model_in_config(&mut cfg, &pid, "whisper-1").unwrap();
        assert_eq!(provider.id, pid);
    }

    #[test]
    fn upsert_local_dedupes_provider_by_host_port() {
        let mut cfg = AppConfig::default();
        let backend = known_local_stt_backend(FUNASR_KEY).unwrap();
        let provider = SttProviderConfig::new(
            "FunASR local",
            SttProviderKind::OpenaiCompatible,
            "http://127.0.0.1:10097",
        );
        let model = SttModelConfig::new("qwen3-asr-flash", "Qwen3-ASR Flash");

        upsert_known_local_stt_provider_in_config(
            &mut cfg,
            &backend,
            provider.clone(),
            model.clone(),
            true,
        );
        upsert_known_local_stt_provider_in_config(
            &mut cfg,
            &backend,
            provider,
            SttModelConfig::new("paraformer-zh", "Paraformer (zh)"),
            false,
        );

        assert_eq!(cfg.stt.providers.len(), 1);
        let stored = &cfg.stt.providers[0];
        assert_eq!(stored.models.len(), 2);
        assert!(stored.allow_private_network);
        assert_eq!(
            cfg.stt.active_model.as_ref().unwrap().model_id,
            "qwen3-asr-flash"
        );
    }

    #[test]
    fn upsert_local_matches_whisper_cpp_by_port() {
        let mut cfg = AppConfig::default();
        let backend = known_local_stt_backend(WHISPER_CPP_KEY).unwrap();
        let provider = SttProviderConfig::new(
            "whisper.cpp local",
            SttProviderKind::OpenaiCompatible,
            "http://localhost:8080/v1",
        );
        upsert_known_local_stt_provider_in_config(
            &mut cfg,
            &backend,
            provider,
            SttModelConfig::new("small", "Whisper small"),
            true,
        );
        assert_eq!(cfg.stt.providers.len(), 1);
        let stored = &cfg.stt.providers[0];
        assert_eq!(stored.kind, SttProviderKind::OpenaiCompatible);
        assert!(stored.allow_private_network);
    }

    #[test]
    fn reorder_keeps_unmentioned_providers_at_tail() {
        let mut cfg = AppConfig::default();
        let p1 = make_provider("p1", "https://a.example.com");
        let p2 = make_provider("p2", "https://b.example.com");
        let p3 = make_provider("p3", "https://c.example.com");
        let ids = [p1.id.clone(), p2.id.clone(), p3.id.clone()];
        cfg.stt.providers = vec![p1, p2, p3];

        reorder_stt_providers_in_config(&mut cfg, &[ids[2].clone(), ids[0].clone()]);
        let resulting: Vec<_> = cfg.stt.providers.iter().map(|p| p.id.clone()).collect();
        assert_eq!(
            resulting,
            vec![ids[2].clone(), ids[0].clone(), ids[1].clone()]
        );
    }
}

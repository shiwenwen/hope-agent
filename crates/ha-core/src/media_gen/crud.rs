//! Media-generation subsystem write helpers.
//!
//! Single entry-point for mutating `AppConfig.media_gen`. Every callsite —
//! Tauri, HTTP, settings tool — must come through here so writes serialize
//! through `mutate_config` and emit `config:changed`. Mirrors `stt::crud`.

use std::fmt;

use crate::config::{mutate_config, AppConfig};
use crate::provider::is_masked_key;

use super::types::{
    AudioGenDefaults, ImageGenDefaults, MediaFunction, MediaModelChain, MediaModelRef,
    MediaProviderConfig,
};

pub type MediaWriteResult<T> = Result<T, MediaWriteError>;

#[derive(Debug)]
pub enum MediaWriteError {
    NotFound(String),
    ModelNotFound {
        provider_id: String,
        model_id: String,
    },
    /// A chain entry points at a model whose modality / audio kinds can't
    /// serve the chain's function.
    FunctionMismatch {
        provider_id: String,
        model_id: String,
        function: MediaFunction,
    },
    Config(anyhow::Error),
}

impl fmt::Display for MediaWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Media provider not found: {id}"),
            Self::ModelNotFound { model_id, .. } => {
                write!(f, "Media model not found: {model_id}")
            }
            Self::FunctionMismatch {
                provider_id,
                model_id,
                function,
            } => write!(
                f,
                "Model {provider_id}::{model_id} cannot serve the `{function}` function \
                 (modality / audio kind mismatch)"
            ),
            Self::Config(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for MediaWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

fn into_anyhow(err: MediaWriteError) -> anyhow::Error {
    anyhow::Error::new(err)
}

fn map_config_error(err: anyhow::Error) -> MediaWriteError {
    match err.downcast::<MediaWriteError>() {
        Ok(media_err) => media_err,
        Err(err) => MediaWriteError::Config(err),
    }
}

// ── Public API ────────────────────────────────────────────────────

/// Returns the stored provider unmasked. Callers handing the value to a
/// non-trusted boundary (HTTP responses) must call `.masked()` themselves —
/// matches the `provider::add_provider` / `stt::add_stt_provider` convention.
pub fn add_media_provider(
    config: MediaProviderConfig,
    source: &'static str,
) -> MediaWriteResult<MediaProviderConfig> {
    mutate_config(("media_gen.add", source), move |store| {
        Ok(add_media_provider_in_config(store, config))
    })
    .map_err(map_config_error)
}

pub fn update_media_provider(
    config: MediaProviderConfig,
    source: &'static str,
) -> MediaWriteResult<()> {
    mutate_config(("media_gen.update", source), move |store| {
        update_media_provider_in_config(store, config).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

/// Delete a provider. Returns true when any default chain referenced it
/// (the affected chain slots are cleaned up in the same write).
pub fn delete_media_provider(provider_id: String, source: &'static str) -> MediaWriteResult<bool> {
    mutate_config(("media_gen.delete", source), move |store| {
        delete_media_provider_in_config(store, &provider_id).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

pub fn reorder_media_providers(
    provider_ids: Vec<String>,
    source: &'static str,
) -> MediaWriteResult<()> {
    mutate_config(("media_gen.reorder", source), move |store| {
        reorder_media_providers_in_config(store, &provider_ids);
        Ok(())
    })
    .map_err(map_config_error)
}

/// Set (or clear, with `None`) the default chain for one function. Every
/// ref is validated: provider exists, model exists, model serves the
/// function.
pub fn set_media_default_chain(
    function: MediaFunction,
    chain: Option<MediaModelChain>,
    source: &'static str,
) -> MediaWriteResult<()> {
    mutate_config(("media_gen.chain", source), move |store| {
        set_media_default_chain_in_config(store, function, chain).map_err(into_anyhow)
    })
    .map_err(map_config_error)
}

pub fn update_media_gen_defaults(
    image: ImageGenDefaults,
    audio: AudioGenDefaults,
    source: &'static str,
) -> MediaWriteResult<()> {
    mutate_config(("media_gen.defaults", source), move |store| {
        store.media_gen.image_defaults = image;
        store.media_gen.audio_defaults = audio;
        Ok(())
    })
    .map_err(map_config_error)
}

// ── In-config helpers (pure, easy to unit-test) ───────────────────

pub(crate) fn add_media_provider_in_config(
    store: &mut AppConfig,
    mut config: MediaProviderConfig,
) -> MediaProviderConfig {
    if config.id.is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }
    store.media_gen.providers.push(config.clone());
    config
}

pub(crate) fn update_media_provider_in_config(
    store: &mut AppConfig,
    config: MediaProviderConfig,
) -> MediaWriteResult<()> {
    let Some(existing) = store
        .media_gen
        .providers
        .iter_mut()
        .find(|p| p.id == config.id)
    else {
        return Err(MediaWriteError::NotFound(config.id));
    };

    existing.name = config.name;
    existing.kind = config.kind;
    existing.base_url = config.base_url;
    if !is_masked_key(&config.api_key) {
        existing.api_key = config.api_key;
    }
    existing.enabled = config.enabled;
    existing.models = config.models;
    existing.default_voice = config.default_voice;
    existing.allow_private_network = config.allow_private_network;
    // `extra` merge contract (same as stt): incoming masked values don't
    // overwrite real ones; keys absent from the incoming full map = delete.
    let mut merged_extra = existing.extra.clone();
    for (key, value) in &config.extra {
        if is_masked_key(value) {
            continue;
        }
        merged_extra.insert(key.clone(), value.clone());
    }
    merged_extra.retain(|k, _| config.extra.contains_key(k));
    existing.extra = merged_extra;
    Ok(())
}

pub(crate) fn delete_media_provider_in_config(
    store: &mut AppConfig,
    provider_id: &str,
) -> MediaWriteResult<bool> {
    let len_before = store.media_gen.providers.len();
    store.media_gen.providers.retain(|p| p.id != provider_id);
    if store.media_gen.providers.len() == len_before {
        return Err(MediaWriteError::NotFound(provider_id.to_string()));
    }
    let mut touched = false;
    for slot in store.media_gen.chains.slots_mut() {
        let Some(chain) = slot.as_mut() else { continue };
        if chain.primary.provider_id == provider_id {
            // Primary gone → promote the first surviving fallback, else
            // clear the slot back to auto.
            let mut rest: Vec<MediaModelRef> = chain
                .fallbacks
                .iter()
                .filter(|r| r.provider_id != provider_id)
                .cloned()
                .collect();
            *slot = if rest.is_empty() {
                None
            } else {
                let primary = rest.remove(0);
                Some(MediaModelChain {
                    primary,
                    fallbacks: rest,
                })
            };
            touched = true;
        } else {
            let before = chain.fallbacks.len();
            chain.fallbacks.retain(|r| r.provider_id != provider_id);
            if chain.fallbacks.len() != before {
                touched = true;
            }
        }
    }
    Ok(touched)
}

pub(crate) fn reorder_media_providers_in_config(store: &mut AppConfig, provider_ids: &[String]) {
    let mut reordered = Vec::with_capacity(provider_ids.len());
    for id in provider_ids {
        if let Some(p) = store.media_gen.providers.iter().find(|p| &p.id == id) {
            reordered.push(p.clone());
        }
    }
    for p in &store.media_gen.providers {
        if !provider_ids.contains(&p.id) {
            reordered.push(p.clone());
        }
    }
    store.media_gen.providers = reordered;
}

pub(crate) fn set_media_default_chain_in_config(
    store: &mut AppConfig,
    function: MediaFunction,
    chain: Option<MediaModelChain>,
) -> MediaWriteResult<()> {
    if let Some(chain) = &chain {
        for entry in chain.iter() {
            check_serves_function(store, entry, function)?;
        }
    }
    store.media_gen.chains.set_for_function(function, chain);
    Ok(())
}

pub(crate) fn check_serves_function(
    store: &AppConfig,
    entry: &MediaModelRef,
    function: MediaFunction,
) -> MediaWriteResult<()> {
    let provider = store
        .media_gen
        .provider(&entry.provider_id)
        .ok_or_else(|| MediaWriteError::NotFound(entry.provider_id.clone()))?;
    let model = provider
        .model_config(&entry.model_id)
        .ok_or_else(|| MediaWriteError::ModelNotFound {
            provider_id: entry.provider_id.clone(),
            model_id: entry.model_id.clone(),
        })?;
    if !model.serves(function) {
        return Err(MediaWriteError::FunctionMismatch {
            provider_id: entry.provider_id.clone(),
            model_id: entry.model_id.clone(),
            function,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_gen::types::{
        AudioKind, AudioModelCaps, ImageModelCaps, MediaModality, MediaModelConfig,
        MediaVendorKind,
    };

    fn provider_with(models: Vec<MediaModelConfig>) -> MediaProviderConfig {
        let mut p = MediaProviderConfig::new("Test", MediaVendorKind::Openai);
        p.api_key = "sk-real".into();
        p.models = models;
        p
    }

    fn image_model(id: &str) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Image);
        m.image = Some(ImageModelCaps::default());
        m
    }

    fn speech_model(id: &str) -> MediaModelConfig {
        let mut m = MediaModelConfig::new(id, id, MediaModality::Audio);
        m.audio = Some(AudioModelCaps {
            kinds: vec![AudioKind::Speech],
            ..Default::default()
        });
        m
    }

    fn chain(pid: &str, mid: &str) -> MediaModelChain {
        MediaModelChain {
            primary: MediaModelRef {
                provider_id: pid.into(),
                model_id: mid.into(),
            },
            fallbacks: vec![],
        }
    }

    #[test]
    fn add_then_update_preserves_real_key_when_incoming_masked() {
        let mut cfg = AppConfig::default();
        let p = provider_with(vec![image_model("gpt-image-1")]);
        let added = add_media_provider_in_config(&mut cfg, p);

        let mut incoming = added.masked();
        incoming.name = "Renamed".into();
        update_media_provider_in_config(&mut cfg, incoming).unwrap();

        let stored = &cfg.media_gen.providers[0];
        assert_eq!(stored.name, "Renamed");
        assert_eq!(stored.api_key, "sk-real");
    }

    #[test]
    fn update_extra_merge_masked_kept_absent_deleted() {
        let mut cfg = AppConfig::default();
        let mut p = provider_with(vec![]);
        p.extra.insert("secret".into(), "real-secret-value".into());
        p.extra.insert("gone".into(), "drop-me".into());
        let added = add_media_provider_in_config(&mut cfg, p);

        let mut incoming = added.masked();
        incoming.extra.remove("gone");
        incoming.extra.insert("fresh".into(), "new-value-123".into());
        update_media_provider_in_config(&mut cfg, incoming).unwrap();

        let stored = &cfg.media_gen.providers[0];
        assert_eq!(stored.extra["secret"], "real-secret-value");
        assert_eq!(stored.extra["fresh"], "new-value-123");
        assert!(!stored.extra.contains_key("gone"));
    }

    #[test]
    fn delete_promotes_fallback_or_clears_chains() {
        let mut cfg = AppConfig::default();
        let a = provider_with(vec![image_model("m1")]);
        let b = provider_with(vec![image_model("m2")]);
        let (aid, bid) = (a.id.clone(), b.id.clone());
        cfg.media_gen.providers.push(a);
        cfg.media_gen.providers.push(b);

        // image chain: primary=a, fallback=b → deleting a promotes b.
        cfg.media_gen.chains.image = Some(MediaModelChain {
            primary: MediaModelRef {
                provider_id: aid.clone(),
                model_id: "m1".into(),
            },
            fallbacks: vec![MediaModelRef {
                provider_id: bid.clone(),
                model_id: "m2".into(),
            }],
        });
        // speech chain: only a → deleting a clears to auto.
        cfg.media_gen.chains.speech = Some(chain(&aid, "m1"));

        assert!(delete_media_provider_in_config(&mut cfg, &aid).unwrap());
        let img = cfg.media_gen.chains.image.as_ref().unwrap();
        assert_eq!(img.primary.provider_id, bid);
        assert!(img.fallbacks.is_empty());
        assert!(cfg.media_gen.chains.speech.is_none());
    }

    #[test]
    fn delete_missing_provider_errors() {
        let mut cfg = AppConfig::default();
        assert!(matches!(
            delete_media_provider_in_config(&mut cfg, "nope"),
            Err(MediaWriteError::NotFound(_))
        ));
    }

    #[test]
    fn set_chain_validates_provider_model_and_function() {
        let mut cfg = AppConfig::default();
        let p = provider_with(vec![image_model("m1"), speech_model("tts")]);
        let pid = p.id.clone();
        cfg.media_gen.providers.push(p);

        assert!(matches!(
            set_media_default_chain_in_config(
                &mut cfg,
                MediaFunction::Image,
                Some(chain("missing", "m1"))
            ),
            Err(MediaWriteError::NotFound(_))
        ));
        assert!(matches!(
            set_media_default_chain_in_config(
                &mut cfg,
                MediaFunction::Image,
                Some(chain(&pid, "missing"))
            ),
            Err(MediaWriteError::ModelNotFound { .. })
        ));
        // TTS model on the image chain → mismatch.
        assert!(matches!(
            set_media_default_chain_in_config(
                &mut cfg,
                MediaFunction::Image,
                Some(chain(&pid, "tts"))
            ),
            Err(MediaWriteError::FunctionMismatch { .. })
        ));
        // Speech model can't serve music either.
        assert!(matches!(
            set_media_default_chain_in_config(
                &mut cfg,
                MediaFunction::Audio(AudioKind::Music),
                Some(chain(&pid, "tts"))
            ),
            Err(MediaWriteError::FunctionMismatch { .. })
        ));

        set_media_default_chain_in_config(&mut cfg, MediaFunction::Image, Some(chain(&pid, "m1")))
            .unwrap();
        set_media_default_chain_in_config(
            &mut cfg,
            MediaFunction::Audio(AudioKind::Speech),
            Some(chain(&pid, "tts")),
        )
        .unwrap();
        assert!(cfg.media_gen.chains.image.is_some());
        assert!(cfg.media_gen.chains.speech.is_some());

        // Clearing back to auto.
        set_media_default_chain_in_config(&mut cfg, MediaFunction::Image, None).unwrap();
        assert!(cfg.media_gen.chains.image.is_none());
    }

    #[test]
    fn reorder_keeps_unmentioned_providers_at_tail() {
        let mut cfg = AppConfig::default();
        let p1 = provider_with(vec![]);
        let p2 = provider_with(vec![]);
        let p3 = provider_with(vec![]);
        let ids = [p1.id.clone(), p2.id.clone(), p3.id.clone()];
        cfg.media_gen.providers = vec![p1, p2, p3];

        reorder_media_providers_in_config(&mut cfg, &[ids[2].clone(), ids[0].clone()]);
        let resulting: Vec<_> = cfg
            .media_gen
            .providers
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert_eq!(
            resulting,
            vec![ids[2].clone(), ids[0].clone(), ids[1].clone()]
        );
    }
}

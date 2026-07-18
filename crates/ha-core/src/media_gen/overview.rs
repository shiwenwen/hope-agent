//! Sanitized availability / capability view for UI consumers (design-space
//! generate dialogs, the tool settings "follows provider order" hint).
//! Carries **no credentials** — safe to hand to any frontend surface.

use serde::Serialize;

use super::resolve::resolve_candidates;
use super::types::{
    AudioKind, AudioModelCaps, ImageGenDefaults, ImageModelCaps, MediaFunction, MediaGenConfig,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCandidateOverview {
    pub provider_id: String,
    pub provider_name: String,
    pub vendor: super::types::MediaVendorKind,
    pub model_id: String,
    pub model_name: String,
    pub supports_voice_listing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageModelCaps>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioModelCaps>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFunctionOverview {
    pub available: bool,
    /// Whether a default chain is pinned (false = follows provider order).
    pub chain_configured: bool,
    /// Resolved candidates in try-order (chain or auto).
    pub candidates: Vec<MediaCandidateOverview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenOverview {
    pub image: MediaFunctionOverview,
    pub speech: MediaFunctionOverview,
    pub music: MediaFunctionOverview,
    pub sfx: MediaFunctionOverview,
    pub image_defaults: ImageGenDefaults,
    pub audio_defaults: super::types::AudioGenDefaults,
}

fn function_overview(cfg: &MediaGenConfig, function: MediaFunction) -> MediaFunctionOverview {
    let candidates = resolve_candidates(cfg, function, None)
        .map(|list| {
            list.into_iter()
                .map(|c| MediaCandidateOverview {
                    provider_id: c.provider.id.clone(),
                    provider_name: c.provider.name.clone(),
                    vendor: c.provider.kind,
                    model_id: c.model.id.clone(),
                    model_name: c.model.name.clone(),
                    supports_voice_listing: c.provider.kind.supports_voice_listing(),
                    image: c.model.image.clone(),
                    audio: c.model.audio.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    MediaFunctionOverview {
        available: !candidates.is_empty(),
        chain_configured: cfg.chains.for_function(function).is_some(),
        candidates,
    }
}

/// Build the sanitized overview from a config snapshot.
pub fn media_gen_overview(cfg: &MediaGenConfig) -> MediaGenOverview {
    MediaGenOverview {
        image: function_overview(cfg, MediaFunction::Image),
        speech: function_overview(cfg, MediaFunction::Audio(AudioKind::Speech)),
        music: function_overview(cfg, MediaFunction::Audio(AudioKind::Music)),
        sfx: function_overview(cfg, MediaFunction::Audio(AudioKind::Sfx)),
        image_defaults: cfg.image_defaults.clone(),
        audio_defaults: cfg.audio_defaults.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_gen::types::{MediaModality, MediaModelConfig, MediaProviderConfig, MediaVendorKind};

    #[test]
    fn overview_never_leaks_credentials() {
        let mut cfg = MediaGenConfig::default();
        let mut p = MediaProviderConfig::new("OpenAI", MediaVendorKind::Openai);
        p.api_key = "sk-super-secret".into();
        p.extra.insert("token".into(), "secret-token".into());
        let mut m = MediaModelConfig::new("gpt-image-1", "GPT Image", MediaModality::Image);
        m.image = Some(ImageModelCaps::default());
        p.models.push(m);
        cfg.providers.push(p);

        let overview = media_gen_overview(&cfg);
        assert!(overview.image.available);
        let json = serde_json::to_string(&overview).unwrap();
        assert!(!json.contains("sk-super-secret"));
        assert!(!json.contains("secret-token"));
    }
}

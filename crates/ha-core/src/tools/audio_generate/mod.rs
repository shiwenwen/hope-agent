//! Audio generation provider stack (BYOK), mirroring
//! [`crate::tools::image_generate`]. Powers the design space's **audio**
//! artifact form (TTS narration / music / SFX → self-contained `<audio>` HTML).
//!
//! Not exposed as an LLM tool yet — driven by the design owner plane
//! (`design::audio::generate_audio_parts`). Provider config is the
//! `audio_generate` settings triad (GUI + ha-settings read-only + SKILL).

mod elevenlabs;
mod openai;
mod types;
pub mod voices;

pub use types::{
    audio_model_catalog, backfill_providers, AudioGenConfig, AudioGenParams, AudioGenProviderEntry,
    AudioGenProviderImpl, AudioGenResult, AudioKind, AudioModelInfo, AUDIO_DURATIONS_SEC,
};
pub use voices::{list_elevenlabs_voices, VoiceOption};

/// Lowercase-normalize a provider id (backward compat: "OpenAI" → "openai").
pub fn normalize_provider_id(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

/// Known provider ids (order = default priority).
pub fn known_provider_ids() -> &'static [&'static str] {
    &["openai", "elevenlabs"]
}

/// Resolve a provider implementation by id.
pub fn resolve_provider(id: &str) -> Option<Box<dyn AudioGenProviderImpl>> {
    match normalize_provider_id(id).as_str() {
        "openai" => Some(Box::new(openai::OpenAiAudioProvider)),
        "elevenlabs" => Some(Box::new(elevenlabs::ElevenLabsAudioProvider)),
        _ => None,
    }
}

/// Effective model for an entry + kind (entry override → provider default).
pub fn effective_model(entry: &AudioGenProviderEntry, kind: AudioKind) -> String {
    entry
        .model
        .clone()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| {
            resolve_provider(&entry.id)
                .map(|p| p.default_model(kind).to_string())
                .unwrap_or_default()
        })
}

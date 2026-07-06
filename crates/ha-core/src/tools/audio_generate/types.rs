//! Audio generation types — provider trait + BYOK config, mirroring
//! [`crate::tools::image_generate::types`] but for audio (TTS / music / SFX).

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Audio sub-capability. Providers differ in what they support, so failover
/// only rotates among candidates that support the requested kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioKind {
    /// Text-to-speech narration.
    Speech,
    /// Generated music from a text prompt.
    Music,
    /// Short sound effects from a text prompt.
    Sfx,
}

impl AudioKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "speech" | "tts" | "voice" | "narration" => Self::Speech,
            "music" | "song" => Self::Music,
            "sfx" | "sound" | "effect" | "soundeffect" => Self::Sfx,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Music => "music",
            Self::Sfx => "sfx",
        }
    }
}

/// Unified parameters for one audio generation call.
pub struct AudioGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub kind: AudioKind,
    pub timeout_secs: u64,
    pub entry: &'a AudioGenProviderEntry,
}

/// Raw generated audio bytes + mime (always self-containable as a data-uri).
pub struct AudioGenResult {
    pub data: Vec<u8>,
    pub mime: String,
}

/// Trait for audio generation providers.
pub trait AudioGenProviderImpl: Send + Sync {
    /// Unique provider id (lowercase), e.g. "openai", "elevenlabs".
    #[allow(dead_code)]
    fn id(&self) -> &str;
    /// Human-readable name.
    fn display_name(&self) -> &str;
    /// Default model for a given sub-capability.
    fn default_model(&self, kind: AudioKind) -> &str;
    /// Whether the provider can produce this kind.
    fn supports(&self, kind: AudioKind) -> bool;
    /// Execute audio generation.
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>>;
}

/// A single audio provider entry with credentials (BYOK).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioGenProviderEntry {
    pub id: String,
    pub enabled: bool,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// TTS voice id / name (provider-specific: OpenAI "alloy", ElevenLabs voice id).
    #[serde(default)]
    pub voice: Option<String>,
}

/// Persistent audio-generation config, stored in config.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioGenConfig {
    /// Ordered providers (order = priority). First enabled with a key + support wins.
    #[serde(default = "default_providers")]
    pub providers: Vec<AudioGenProviderEntry>,
    /// Request timeout in seconds (music can be slow → default 120).
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

fn default_providers() -> Vec<AudioGenProviderEntry> {
    vec![
        AudioGenProviderEntry {
            id: "openai".to_string(),
            ..Default::default()
        },
        AudioGenProviderEntry {
            id: "elevenlabs".to_string(),
            ..Default::default()
        },
    ]
}

fn default_timeout() -> u64 {
    120
}

impl Default for AudioGenConfig {
    fn default() -> Self {
        Self {
            providers: default_providers(),
            timeout_seconds: default_timeout(),
        }
    }
}

/// Normalize ids + ensure all known providers exist (mirrors image_generate).
pub fn backfill_providers(config: &mut AudioGenConfig) {
    for p in &mut config.providers {
        p.id = super::normalize_provider_id(&p.id);
    }
    for id in super::known_provider_ids() {
        if !config.providers.iter().any(|p| p.id == *id) {
            config.providers.push(AudioGenProviderEntry {
                id: id.to_string(),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip_and_aliases() {
        assert_eq!(AudioKind::parse("tts"), Some(AudioKind::Speech));
        assert_eq!(AudioKind::parse("MUSIC"), Some(AudioKind::Music));
        assert_eq!(AudioKind::parse("sound"), Some(AudioKind::Sfx));
        assert_eq!(AudioKind::parse("nope"), None);
        for k in [AudioKind::Speech, AudioKind::Music, AudioKind::Sfx] {
            assert_eq!(AudioKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn backfill_adds_missing_known_providers() {
        let mut cfg = AudioGenConfig {
            providers: Vec::new(),
            timeout_seconds: 120,
        };
        backfill_providers(&mut cfg);
        for id in super::super::known_provider_ids() {
            assert!(cfg.providers.iter().any(|p| &p.id == id), "missing {id}");
        }
    }
}

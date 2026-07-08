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

/// 策展音频模型目录条目（B8-1，单一真相源）：已知模型 + caps/hint/default，供设置面 picker
/// 呈现预设（**GUI-only 消费、不进 config 写入**，同 `local_llm::model_catalog` /
/// `stt::local` 已知后端；用户仍可自填 model 覆盖）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioModelInfo {
    pub id: String,
    pub label: String,
    pub hint: String,
    pub provider: String,
    /// `speech` | `music` | `sfx`。
    pub kind: String,
    pub default: bool,
    /// 是否吃时长参数（music / sfx = true）。
    pub supports_duration: bool,
}

/// 策展音频模型目录（每 kind 已知模型；default 扫描定默认）。
pub fn audio_model_catalog() -> Vec<AudioModelInfo> {
    let m = |id: &str,
             label: &str,
             hint: &str,
             provider: &str,
             kind: &str,
             default: bool,
             dur: bool| {
        AudioModelInfo {
            id: id.into(),
            label: label.into(),
            hint: hint.into(),
            provider: provider.into(),
            kind: kind.into(),
            default,
            supports_duration: dur,
        }
    };
    vec![
        // Speech (TTS)
        m(
            "eleven_v3",
            "ElevenLabs v3",
            "natural, multilingual",
            "elevenlabs",
            "speech",
            true,
            false,
        ),
        m(
            "eleven_multilingual_v2",
            "ElevenLabs Multilingual v2",
            "stable multilingual",
            "elevenlabs",
            "speech",
            false,
            false,
        ),
        m(
            "gpt-4o-mini-tts",
            "OpenAI gpt-4o-mini-tts",
            "fast, low-cost",
            "openai",
            "speech",
            false,
            false,
        ),
        // Music
        m(
            "music_v1",
            "ElevenLabs Music",
            "text-to-music",
            "elevenlabs",
            "music",
            true,
            true,
        ),
        // SFX
        m(
            "eleven_text_to_sound_v2",
            "ElevenLabs Sound Effects",
            "short SFX, 0.5–30s",
            "elevenlabs",
            "sfx",
            true,
            true,
        ),
    ]
}

/// 全局音频时长桶（秒，B8-1；非 per-model，UI 候选）。
pub const AUDIO_DURATIONS_SEC: &[u32] = &[5, 10, 15, 30, 60, 120];

/// Unified parameters for one audio generation call.
pub struct AudioGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub kind: AudioKind,
    pub timeout_secs: u64,
    /// 目标时长（秒，B8-2）：music / sfx 用；`None` = provider 默认。各 provider 自钳到合法区间。
    pub duration_seconds: Option<f64>,
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
    fn audio_catalog_has_one_default_per_kind_and_sfx_duration() {
        let cat = audio_model_catalog();
        for kind in ["speech", "music", "sfx"] {
            let defaults: Vec<_> = cat.iter().filter(|m| m.kind == kind && m.default).collect();
            assert_eq!(defaults.len(), 1, "{kind} 须恰好一个默认模型");
        }
        // SFX 默认模型是专用音效模型且吃时长。
        let sfx = cat.iter().find(|m| m.kind == "sfx" && m.default).unwrap();
        assert_eq!(sfx.id, "eleven_text_to_sound_v2");
        assert!(sfx.supports_duration);
        // duration 桶单调递增。
        assert!(AUDIO_DURATIONS_SEC.windows(2).all(|w| w[0] < w[1]));
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

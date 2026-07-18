//! Built-in media-provider templates + preset model catalog.
//!
//! Single source of truth for "what vendors do we know and what can their
//! models do" — replaces the old per-provider hardcoded trait
//! `capabilities()`, the GUI-only `audio_model_catalog()`, and the
//! frontend's hardcoded preset model lists. GUI-only consumption via the
//! `get_media_provider_templates` owner command: templates seed a
//! `MediaProviderConfig` draft when the user adds a provider; nothing here
//! is read at generation time (the config's own data-driven caps are).
//!
//! Capability data is transcribed from the retired adapter trait
//! declarations — keep faithful when touching (a wrong caps entry silently
//! filters a healthy candidate out of failover).

use serde::Serialize;

use super::types::{
    AudioKind, AudioModelCaps, ImageEditCaps, ImageModelCaps, MediaModality, MediaModelConfig,
    MediaVendorKind,
};

/// One built-in vendor template. `models` returns presets in recommended
/// order (first = suggested default; auto-mode picks the first matching
/// model on a provider, so order is meaningful once copied into config).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProviderTemplate {
    /// Stable key, e.g. "openai" / "elevenlabs".
    pub key: &'static str,
    pub name: &'static str,
    pub kind: MediaVendorKind,
    pub base_url: &'static str,
    /// False for self-hosted OpenAI-compatible endpoints.
    pub requires_api_key: bool,
    pub supports_voice_listing: bool,
    pub models: Vec<MediaModelConfig>,
}

fn img(id: &str, name: &str, caps: ImageModelCaps, extra: &[(&str, &str)]) -> MediaModelConfig {
    let mut m = MediaModelConfig::new(id, name, MediaModality::Image);
    m.image = Some(caps);
    for (k, v) in extra {
        m.extra.insert((*k).to_string(), (*v).to_string());
    }
    m
}

fn aud(id: &str, name: &str, caps: AudioModelCaps) -> MediaModelConfig {
    let mut m = MediaModelConfig::new(id, name, MediaModality::Audio);
    m.audio = Some(caps);
    m
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn speech_caps() -> AudioModelCaps {
    AudioModelCaps {
        kinds: vec![AudioKind::Speech],
        supports_duration: false,
        needs_voice: true,
        default_voice: None,
        min_duration_secs: None,
        max_duration_secs: None,
    }
}

// ── Per-vendor image caps (transcribed from the old adapter traits) ──

fn openai_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&["1024x1024", "1024x1536", "1536x1024"]),
        aspect_ratios: vec![],
        resolutions: vec![],
        // The old trait declared edit disabled for the tool path, but the
        // adapter routes mask requests to `/images/edits` (design inpaint
        // relies on it) — expressed as supports_mask without generic edit.
        supports_mask: true,
        edit: None,
    }
}

fn google_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: true,
        supports_resolution: true,
        sizes: strs(&[
            "1024x1024",
            "1024x1536",
            "1536x1024",
            "1024x1792",
            "1792x1024",
        ]),
        aspect_ratios: strs(&[
            "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
        ]),
        resolutions: strs(&["1K", "2K", "4K"]),
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 4,
            max_input_images: 5,
            supports_size: true,
            supports_aspect_ratio: true,
            supports_resolution: true,
        }),
    }
}

fn fal_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: true,
        supports_resolution: true,
        sizes: strs(&[
            "1024x1024",
            "1024x1536",
            "1536x1024",
            "1024x1792",
            "1792x1024",
        ]),
        aspect_ratios: strs(&["1:1", "4:3", "3:4", "16:9", "9:16"]),
        resolutions: strs(&["1K", "2K", "4K"]),
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 4,
            max_input_images: 1,
            supports_size: true,
            // Fal edit doesn't support aspectRatio.
            supports_aspect_ratio: false,
            supports_resolution: true,
        }),
    }
}

fn minimax_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 9,
        supports_size: false,
        supports_aspect_ratio: true,
        supports_resolution: false,
        sizes: vec![],
        aspect_ratios: strs(&["1:1", "16:9", "4:3", "3:2", "2:3", "3:4", "9:16", "21:9"]),
        resolutions: vec![],
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 9,
            max_input_images: 1,
            supports_size: false,
            supports_aspect_ratio: true,
            supports_resolution: false,
        }),
    }
}

fn siliconflow_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&[
            "1024x1024",
            "1328x1328",
            "1664x928",
            "928x1664",
            "1472x1140",
            "1140x1472",
            "1584x1056",
            "1056x1584",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 1,
            max_input_images: 1,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

fn zhipu_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 1,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&[
            "1024x1024",
            "1024x1536",
            "1536x1024",
            "1024x1792",
            "1792x1024",
            "2048x2048",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: None,
    }
}

fn tongyi_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&["1024x1024", "720x1280", "1280x720"]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 1,
            max_input_images: 1,
            supports_size: false,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

// ── Templates ─────────────────────────────────────────────────────

/// All built-in vendor templates, in suggested display order.
pub fn media_provider_templates() -> Vec<MediaProviderTemplate> {
    vec![
        MediaProviderTemplate {
            key: "openai",
            name: "OpenAI",
            kind: MediaVendorKind::Openai,
            base_url: MediaVendorKind::Openai.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            models: vec![
                img("gpt-image-1", "GPT Image 1", openai_image_caps(), &[]),
                img("gpt-image-2", "GPT Image 2", openai_image_caps(), &[]),
                img(
                    "dall-e-3",
                    "DALL·E 3",
                    ImageModelCaps {
                        // dall-e-3 has no edits endpoint.
                        supports_mask: false,
                        max_n: 1,
                        ..openai_image_caps()
                    },
                    &[],
                ),
                aud("gpt-4o-mini-tts", "GPT-4o mini TTS", speech_caps()),
                aud("tts-1", "TTS-1", speech_caps()),
                aud("tts-1-hd", "TTS-1 HD", speech_caps()),
            ],
        },
        MediaProviderTemplate {
            key: "google",
            name: "Google",
            kind: MediaVendorKind::Google,
            base_url: MediaVendorKind::Google.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![
                img(
                    "gemini-3.1-flash-image-preview",
                    "Gemini 3.1 Flash Image Preview",
                    google_image_caps(),
                    &[("thinking_level", "MINIMAL")],
                ),
                img(
                    "gemini-3-pro-image-preview",
                    "Gemini 3 Pro Image Preview",
                    google_image_caps(),
                    &[("thinking_level", "MINIMAL")],
                ),
                img(
                    "gemini-2.5-flash-image",
                    "Gemini 2.5 Flash Image",
                    google_image_caps(),
                    &[],
                ),
                img(
                    "imagen-4.0-generate-001",
                    "Imagen 4",
                    google_image_caps(),
                    &[],
                ),
                img(
                    "imagen-4.0-ultra-generate-001",
                    "Imagen 4 Ultra",
                    google_image_caps(),
                    &[],
                ),
                img(
                    "imagen-4.0-fast-generate-001",
                    "Imagen 4 Fast",
                    google_image_caps(),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "elevenlabs",
            name: "ElevenLabs",
            kind: MediaVendorKind::Elevenlabs,
            base_url: MediaVendorKind::Elevenlabs.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            models: vec![
                aud("eleven_v3", "ElevenLabs v3", speech_caps()),
                aud(
                    "eleven_multilingual_v2",
                    "ElevenLabs Multilingual v2",
                    speech_caps(),
                ),
                aud(
                    "music_v1",
                    "ElevenLabs Music",
                    AudioModelCaps {
                        kinds: vec![AudioKind::Music],
                        supports_duration: true,
                        needs_voice: false,
                        default_voice: None,
                        min_duration_secs: Some(10.0),
                        max_duration_secs: Some(300.0),
                    },
                ),
                aud(
                    "eleven_text_to_sound_v2",
                    "ElevenLabs Sound Effects",
                    AudioModelCaps {
                        kinds: vec![AudioKind::Sfx],
                        supports_duration: true,
                        needs_voice: false,
                        default_voice: None,
                        min_duration_secs: Some(0.5),
                        max_duration_secs: Some(30.0),
                    },
                ),
            ],
        },
        MediaProviderTemplate {
            key: "fal",
            name: "Fal",
            kind: MediaVendorKind::Fal,
            base_url: MediaVendorKind::Fal.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![img("fal-ai/flux/dev", "FLUX.1 dev", fal_image_caps(), &[])],
        },
        MediaProviderTemplate {
            key: "minimax",
            name: "MiniMax",
            kind: MediaVendorKind::Minimax,
            base_url: MediaVendorKind::Minimax.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![img("image-01", "Image-01", minimax_image_caps(), &[])],
        },
        MediaProviderTemplate {
            key: "siliconflow",
            name: "SiliconFlow",
            kind: MediaVendorKind::Siliconflow,
            base_url: MediaVendorKind::Siliconflow.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![img(
                "Qwen/Qwen-Image",
                "Qwen-Image",
                siliconflow_image_caps(),
                &[],
            )],
        },
        MediaProviderTemplate {
            key: "zhipu",
            name: "ZhipuAI",
            kind: MediaVendorKind::Zhipu,
            base_url: MediaVendorKind::Zhipu.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![img(
                "cogView-4-250304",
                "CogView 4",
                zhipu_image_caps(),
                &[],
            )],
        },
        MediaProviderTemplate {
            key: "tongyi",
            name: "Tongyi Wanxiang",
            kind: MediaVendorKind::Tongyi,
            base_url: MediaVendorKind::Tongyi.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![img("wanx-v1", "Wanxiang v1", tongyi_image_caps(), &[])],
        },
        MediaProviderTemplate {
            key: "openai-compatible",
            name: "OpenAI-compatible",
            kind: MediaVendorKind::OpenaiCompatible,
            base_url: "",
            requires_api_key: false,
            supports_voice_listing: true,
            // No presets — the user declares what their endpoint serves.
            models: vec![],
        },
    ]
}

/// Static voice presets for OpenAI-style TTS (`/v1/audio/speech` has no
/// voices-listing endpoint; these are the documented voice names).
pub const OPENAI_TTS_VOICES: &[&str] = &[
    "alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_gen::types::MediaFunction;

    #[test]
    fn every_template_model_has_matching_caps_group() {
        for tpl in media_provider_templates() {
            for m in &tpl.models {
                match m.modality {
                    MediaModality::Image => {
                        assert!(m.image.is_some(), "{}/{} missing image caps", tpl.key, m.id)
                    }
                    MediaModality::Audio => {
                        assert!(m.audio.is_some(), "{}/{} missing audio caps", tpl.key, m.id)
                    }
                    MediaModality::Video => panic!("video is reserved, no template may ship it"),
                }
            }
        }
    }

    #[test]
    fn template_keys_are_unique() {
        let templates = media_provider_templates();
        let mut keys: Vec<_> = templates.iter().map(|t| t.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), templates.len());
    }

    #[test]
    fn audio_kinds_are_covered_by_templates() {
        // Each audio kind must have at least one preset model somewhere,
        // or the "add from template" flow can't serve that feature.
        let templates = media_provider_templates();
        for kind in [AudioKind::Speech, AudioKind::Music, AudioKind::Sfx] {
            assert!(
                templates.iter().any(|t| t
                    .models
                    .iter()
                    .any(|m| m.serves(MediaFunction::Audio(kind)))),
                "no template model serves {kind:?}"
            );
        }
    }

    #[test]
    fn openai_mask_support_is_model_specific() {
        let templates = media_provider_templates();
        let openai = templates.iter().find(|t| t.key == "openai").unwrap();
        let gpt1 = openai
            .models
            .iter()
            .find(|m| m.id == "gpt-image-1")
            .unwrap();
        assert!(gpt1.image.as_ref().unwrap().supports_mask);
        let dalle3 = openai.models.iter().find(|m| m.id == "dall-e-3").unwrap();
        assert!(!dalle3.image.as_ref().unwrap().supports_mask);
    }

    #[test]
    fn speech_presets_declare_voice_requirement() {
        for tpl in media_provider_templates() {
            for m in &tpl.models {
                if let Some(caps) = &m.audio {
                    if caps.kinds.contains(&AudioKind::Speech) {
                        assert!(caps.needs_voice, "{}/{} TTS must need voice", tpl.key, m.id);
                    }
                }
            }
        }
    }
}

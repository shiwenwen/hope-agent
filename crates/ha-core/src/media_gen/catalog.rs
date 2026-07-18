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

/// Speech models whose voice is optional (the vendor falls back to its own
/// default) or not a separate parameter at all. See
/// [`VOICELESS_SPEECH_VENDORS`] for why this is not a mistake.
fn speech_caps_no_voice() -> AudioModelCaps {
    AudioModelCaps {
        needs_voice: false,
        ..speech_caps()
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

fn stepfun_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        // StepFun documents "currently only 1" for n on every image model.
        max_n: 1,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&[
            "1024x1024",
            "768x768",
            "512x512",
            "256x256",
            "1280x800",
            "800x1280",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        // img2img runs on `/v1/images/image2image` with a `source_url`.
        edit: Some(ImageEditCaps {
            max_n: 1,
            max_input_images: 1,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

/// Ark sizes are tier tokens ("2K") or explicit `WxH`; tiers are listed
/// because they are what the docs steer callers to.
fn volcengine_image_caps(sizes: &[&str], max_input_images: u32) -> ImageModelCaps {
    ImageModelCaps {
        // No `n` on this API — batches need `sequential_image_generation`,
        // which the shared compat profile does not drive.
        max_n: 1,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(sizes),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: Some(ImageEditCaps {
            max_n: 1,
            max_input_images,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

fn together_image_caps(edit: bool) -> ImageModelCaps {
    ImageModelCaps {
        max_n: 4,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        // Width/height are free integers (256–1920, multiples of 8); these
        // are the documented presets.
        sizes: strs(&["1024x1024", "1344x768", "768x1344", "1024x768", "768x1024"]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: edit.then_some(ImageEditCaps {
            max_n: 1,
            max_input_images: 1,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

fn xai_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 10,
        // xAI defines no size/quality/style knobs at all.
        supports_size: false,
        supports_aspect_ratio: true,
        supports_resolution: true,
        sizes: vec![],
        aspect_ratios: strs(&[
            "1:1", "3:4", "4:3", "9:16", "16:9", "2:3", "3:2", "1:2", "2:1",
        ]),
        resolutions: strs(&["1K", "2K"]),
        supports_mask: false,
        // `/v1/images/edits` exists but takes a JSON `image` object rather
        // than the shared profile's field shape.
        edit: None,
    }
}

fn recraft_image_caps(pro: bool) -> ImageModelCaps {
    ImageModelCaps {
        max_n: 6,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: if pro {
            strs(&["2048x2048", "3072x1536", "1536x3072"])
        } else {
            strs(&["1024x1024", "1536x768", "768x1344", "1280x832"])
        },
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        // Recraft's edit/inpaint tools are separate endpoints with a
        // different response envelope (bare `image` object).
        edit: None,
    }
}

fn hunyuan_image_caps(edit: bool) -> ImageModelCaps {
    ImageModelCaps {
        max_n: 1,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        // Stored `WxH`; the adapter rewrites the separator to `W:H`.
        sizes: strs(&[
            "1024x1024",
            "1280x768",
            "768x1280",
            "1024x768",
            "768x1024",
            "1280x720",
            "720x1280",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: edit.then_some(ImageEditCaps {
            max_n: 1,
            max_input_images: 3,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

fn qianfan_image_caps(max_n: u32, edit: bool) -> ImageModelCaps {
    ImageModelCaps {
        max_n,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        sizes: strs(&[
            "1024x1024",
            "1024x768",
            "768x1024",
            "1536x1536",
            "2048x2048",
            "512x512",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: edit.then_some(ImageEditCaps {
            max_n: 1,
            max_input_images: 1,
            supports_size: true,
            supports_aspect_ratio: false,
            supports_resolution: false,
        }),
    }
}

fn sensenova_image_caps() -> ImageModelCaps {
    ImageModelCaps {
        max_n: 1,
        supports_size: true,
        supports_aspect_ratio: false,
        supports_resolution: false,
        // Only these fixed buckets are accepted; anything else fails the
        // task server-side (4K is rejected outright).
        sizes: strs(&[
            "1792x992",
            "992x1792",
            "1344x1344",
            "1088x1632",
            "1632x1088",
            "1152x1536",
            "1536x1152",
            "1184x1472",
            "1472x1184",
            "864x2048",
            "2752x1536",
            "1536x2752",
            "2048x2048",
            "1664x2496",
            "2496x1664",
            "1760x2368",
            "2368x1760",
            "1824x2272",
            "2272x1824",
            "1344x3136",
        ]),
        aspect_ratios: vec![],
        resolutions: vec![],
        supports_mask: false,
        edit: None,
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
                    "music_v2",
                    "ElevenLabs Music v2",
                    AudioModelCaps {
                        kinds: vec![AudioKind::Music],
                        supports_duration: true,
                        needs_voice: false,
                        default_voice: None,
                        min_duration_secs: Some(3.0),
                        max_duration_secs: Some(600.0),
                    },
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
            models: vec![
                img("image-01", "Image-01", minimax_image_caps(), &[]),
                aud("speech-2.8-hd", "Speech 2.8 HD", speech_caps()),
                aud("speech-2.8-turbo", "Speech 2.8 Turbo", speech_caps()),
                aud("speech-2.6-hd", "Speech 2.6 HD", speech_caps()),
                aud("speech-2.6-turbo", "Speech 2.6 Turbo", speech_caps()),
                aud(
                    "music-3.0",
                    "Music 3.0",
                    AudioModelCaps {
                        kinds: vec![AudioKind::Music],
                        // The music endpoint exposes no duration parameter.
                        supports_duration: false,
                        needs_voice: false,
                        default_voice: None,
                        min_duration_secs: None,
                        max_duration_secs: None,
                    },
                ),
                aud(
                    "music-2.6",
                    "Music 2.6",
                    AudioModelCaps {
                        kinds: vec![AudioKind::Music],
                        supports_duration: false,
                        needs_voice: false,
                        default_voice: None,
                        min_duration_secs: None,
                        max_duration_secs: None,
                    },
                ),
            ],
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
            key: "stepfun",
            name: "StepFun",
            kind: MediaVendorKind::Stepfun,
            base_url: MediaVendorKind::Stepfun.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            models: vec![
                img("step-2x-large", "Step-2x Large", stepfun_image_caps(), &[]),
                img(
                    "step-1x-medium",
                    "Step-1x Medium",
                    stepfun_image_caps(),
                    &[],
                ),
                img(
                    "step-image-edit-2",
                    "Step Image Edit 2",
                    ImageModelCaps {
                        // Its edit endpoint is multipart, which the shared
                        // compat profile can't drive — generation only here.
                        edit: None,
                        sizes: strs(&["1024x1024", "768x1360", "1360x768", "896x1184", "1184x896"]),
                        ..stepfun_image_caps()
                    },
                    &[],
                ),
                aud("step-tts-2", "Step TTS 2", speech_caps()),
                aud("stepaudio-2.5-tts", "Step Audio 2.5 TTS", speech_caps()),
                aud("step-tts-mini", "Step TTS Mini", speech_caps()),
            ],
        },
        MediaProviderTemplate {
            key: "volcengine",
            name: "Volcengine Ark",
            kind: MediaVendorKind::Volcengine,
            base_url: MediaVendorKind::Volcengine.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![
                img(
                    "doubao-seedream-5-0-pro-260628",
                    "Doubao Seedream 5.0 Pro",
                    volcengine_image_caps(&["1K", "2K"], 10),
                    &[],
                ),
                img(
                    "doubao-seedream-5-0-260128",
                    "Doubao Seedream 5.0 Lite",
                    volcengine_image_caps(&["2K", "3K"], 14),
                    &[],
                ),
                img(
                    "doubao-seedream-4-5-251128",
                    "Doubao Seedream 4.5",
                    volcengine_image_caps(&["2K", "4K"], 14),
                    &[],
                ),
                img(
                    "doubao-seedream-4-0-250828",
                    "Doubao Seedream 4.0",
                    volcengine_image_caps(&["1K", "2K", "4K"], 14),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "hunyuan",
            name: "Tencent Hunyuan",
            kind: MediaVendorKind::Hunyuan,
            base_url: MediaVendorKind::Hunyuan.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![
                img(
                    "hy-image-v3.0",
                    "Hunyuan Image 3.0",
                    hunyuan_image_caps(true),
                    &[],
                ),
                img(
                    "hy-image-lite",
                    "Hunyuan Image Lite",
                    hunyuan_image_caps(false),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "together",
            name: "Together AI",
            kind: MediaVendorKind::Together,
            base_url: MediaVendorKind::Together.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            // Deliberately a curated subset of Together's ~28 image models:
            // current flagships plus the models not reachable elsewhere here.
            models: vec![
                img(
                    "black-forest-labs/FLUX.2-pro",
                    "FLUX.2 pro",
                    together_image_caps(false),
                    &[],
                ),
                img(
                    "black-forest-labs/FLUX.2-flex",
                    "FLUX.2 flex",
                    together_image_caps(false),
                    &[],
                ),
                img(
                    "black-forest-labs/FLUX.1-kontext-pro",
                    "FLUX.1 Kontext pro",
                    together_image_caps(true),
                    &[],
                ),
                img(
                    "black-forest-labs/FLUX.1-schnell",
                    "FLUX.1 schnell",
                    together_image_caps(false),
                    &[],
                ),
                img(
                    "ByteDance-Seed/Seedream-4.0",
                    "Seedream 4.0",
                    together_image_caps(false),
                    &[],
                ),
                img(
                    "Qwen/Qwen-Image-2.0",
                    "Qwen-Image 2.0",
                    together_image_caps(false),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "xai",
            name: "xAI",
            kind: MediaVendorKind::Xai,
            base_url: MediaVendorKind::Xai.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![
                img(
                    "grok-imagine-image",
                    "Grok Imagine Image",
                    xai_image_caps(),
                    &[],
                ),
                img(
                    "grok-imagine-image-quality",
                    "Grok Imagine Image Quality",
                    xai_image_caps(),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "recraft",
            name: "Recraft",
            kind: MediaVendorKind::Recraft,
            base_url: MediaVendorKind::Recraft.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            models: vec![
                img(
                    "recraftv4_1",
                    "Recraft V4.1",
                    recraft_image_caps(false),
                    &[],
                ),
                img(
                    "recraftv4_1_vector",
                    "Recraft V4.1 Vector (SVG)",
                    recraft_image_caps(false),
                    &[],
                ),
                img(
                    "recraftv4_1_pro",
                    "Recraft V4.1 Pro",
                    recraft_image_caps(true),
                    &[],
                ),
                img("recraftv3", "Recraft V3", recraft_image_caps(false), &[]),
                img(
                    "recraftv3_vector",
                    "Recraft V3 Vector (SVG)",
                    recraft_image_caps(false),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "qianfan",
            name: "Baidu Qianfan",
            kind: MediaVendorKind::Qianfan,
            base_url: MediaVendorKind::Qianfan.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            // `musesteamer-air-image` is intentionally absent: it lives on
            // `/v2/musesteamer/images/generations` with its own parameter
            // table, which this provider's single path can't reach.
            models: vec![
                img(
                    "qwen-image",
                    "Qwen-Image",
                    qianfan_image_caps(1, false),
                    &[],
                ),
                img(
                    "qwen-image-edit",
                    "Qwen-Image Edit",
                    qianfan_image_caps(1, true),
                    &[],
                ),
                img(
                    "ernie-irag-edit",
                    "ERNIE iRAG Edit",
                    qianfan_image_caps(4, true),
                    &[],
                ),
            ],
        },
        MediaProviderTemplate {
            key: "sensenova",
            name: "SenseNova",
            kind: MediaVendorKind::Sensenova,
            base_url: MediaVendorKind::Sensenova.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: false,
            // SenseNova TTS lives on a different host (api.sensenova.cn) with
            // its own auth, so it is not a model of this provider — add it
            // separately as an OpenAI-compatible provider if wanted.
            models: vec![img(
                "sensenova-u1-fast",
                "SenseNova U1 Fast",
                sensenova_image_caps(),
                &[],
            )],
        },
        MediaProviderTemplate {
            key: "cartesia",
            name: "Cartesia",
            kind: MediaVendorKind::Cartesia,
            base_url: MediaVendorKind::Cartesia.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            models: vec![
                aud("sonic-3.5", "Sonic 3.5", speech_caps()),
                aud("sonic-3", "Sonic 3", speech_caps()),
            ],
        },
        MediaProviderTemplate {
            key: "deepgram",
            name: "Deepgram",
            kind: MediaVendorKind::Deepgram,
            base_url: MediaVendorKind::Deepgram.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            // Deepgram has no voice parameter — the voice *is* the model id
            // (`aura-2-{voice}-{lang}`), so each voice ships as its own
            // preset. These are a subset of the ~94 documented voices.
            models: vec![
                aud(
                    "aura-2-thalia-en",
                    "Aura 2 Thalia (en)",
                    speech_caps_no_voice(),
                ),
                aud(
                    "aura-2-asteria-en",
                    "Aura 2 Asteria (en)",
                    speech_caps_no_voice(),
                ),
                aud("aura-2-luna-en", "Aura 2 Luna (en)", speech_caps_no_voice()),
                aud("aura-2-zeus-en", "Aura 2 Zeus (en)", speech_caps_no_voice()),
                aud(
                    "aura-2-andromeda-en",
                    "Aura 2 Andromeda (en)",
                    speech_caps_no_voice(),
                ),
                aud(
                    "aura-2-sirio-es",
                    "Aura 2 Sirio (es)",
                    speech_caps_no_voice(),
                ),
                aud(
                    "aura-asteria-en",
                    "Aura 1 Asteria (en)",
                    speech_caps_no_voice(),
                ),
            ],
        },
        MediaProviderTemplate {
            key: "fishaudio",
            name: "Fish Audio",
            kind: MediaVendorKind::Fishaudio,
            base_url: MediaVendorKind::Fishaudio.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            models: vec![
                aud("s2.1-pro", "Fish Speech S2.1 Pro", speech_caps_no_voice()),
                aud(
                    "s2.1-pro-free",
                    "Fish Speech S2.1 Pro (free)",
                    speech_caps_no_voice(),
                ),
                aud("s2-pro", "Fish Speech S2 Pro", speech_caps_no_voice()),
                aud("s1", "Fish Speech S1", speech_caps_no_voice()),
            ],
        },
        MediaProviderTemplate {
            key: "hume",
            name: "Hume AI",
            kind: MediaVendorKind::Hume,
            base_url: MediaVendorKind::Hume.default_base_url(),
            requires_api_key: true,
            supports_voice_listing: true,
            // Hume takes no `model` field — these ids map to its `version`
            // selector inside the adapter.
            models: vec![
                aud("octave-2", "Octave 2", speech_caps_no_voice()),
                aud("octave-1", "Octave 1", speech_caps_no_voice()),
            ],
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

    /// Vendors whose speech models legitimately declare `needs_voice=false`:
    /// Deepgram has no voice parameter at all (the voice *is* the model id),
    /// while Fish Audio's `reference_id` and Hume's `voice` are optional and
    /// fall back to a vendor default. Forcing a voice on these would make the
    /// UI demand a value the API does not accept or need.
    const VOICELESS_SPEECH_VENDORS: &[&str] = &["deepgram", "fishaudio", "hume"];

    #[test]
    fn speech_presets_declare_voice_requirement() {
        for tpl in media_provider_templates() {
            if VOICELESS_SPEECH_VENDORS.contains(&tpl.key) {
                continue;
            }
            for m in &tpl.models {
                if let Some(caps) = &m.audio {
                    if caps.kinds.contains(&AudioKind::Speech) {
                        assert!(caps.needs_voice, "{}/{} TTS must need voice", tpl.key, m.id);
                    }
                }
            }
        }
    }

    /// The exception list above is only allowed to name vendors that really
    /// ship voiceless speech presets — otherwise a stale entry would silently
    /// disarm the check for a vendor that later grows a voice parameter.
    #[test]
    fn voiceless_exception_list_has_no_stale_entries() {
        let templates = media_provider_templates();
        for key in VOICELESS_SPEECH_VENDORS {
            let tpl = templates
                .iter()
                .find(|t| &t.key == key)
                .unwrap_or_else(|| panic!("{key} is not a template"));
            assert!(
                tpl.models.iter().any(|m| m
                    .audio
                    .as_ref()
                    .is_some_and(|c| c.kinds.contains(&AudioKind::Speech) && !c.needs_voice)),
                "{key} is listed as voiceless but has no voiceless speech preset"
            );
        }
    }
}

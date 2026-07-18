//! Vendor adapters: the wire-protocol implementations behind
//! [`MediaVendorKind`]. Migrated from the retired
//! `tools/{image_generate,audio_generate}` provider stacks with the trait
//! slimmed to `generate` only — identity, default models, and capabilities
//! are data now (`catalog.rs` templates → user config).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::security::ssrf::SsrfPolicy;

use super::types::{AudioKind, MediaVendorKind};

pub mod audio;
pub mod fetch;
pub mod image;

// ── Shared request/response shapes ────────────────────────────────

/// A loaded input/reference image ready for provider consumption.
pub struct InputImage {
    pub data: Vec<u8>,
    pub mime: String,
}

pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime: String,
    pub revised_prompt: Option<String>,
}

/// Result from image generation: images + optional accompanying text
/// (e.g. Gemini returns text alongside images).
pub struct ImageGenResult {
    pub images: Vec<GeneratedImage>,
    pub text: Option<String>,
}

/// Raw generated audio bytes + mime (always self-containable as a data-uri).
pub struct AudioGenResult {
    pub data: Vec<u8>,
    pub mime: String,
}

/// Unified parameters for one image generation call.
pub struct ImageGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub size: &'a str,
    pub n: u32,
    pub timeout_secs: u64,
    /// Merged provider `extra` ← model `extra` (model wins). Vendor-specific
    /// knobs, e.g. Google `thinking_level`.
    pub extra: &'a HashMap<String, String>,
    /// Aspect ratio hint (e.g. "1:1", "16:9", "9:16").
    pub aspect_ratio: Option<&'a str>,
    /// Resolution hint: "1K", "2K", or "4K".
    pub resolution: Option<&'a str>,
    /// Reference/input images for editing.
    pub input_images: &'a [InputImage],
    /// Inpaint mask (PNG bytes; painted region = area to regenerate). Only
    /// mask-capable vendors honor it (OpenAI `/images/edits`); candidate
    /// filtering keeps it away from the rest.
    pub mask: Option<&'a [u8]>,
    /// SSRF policy derived from the provider's `allow_private_network`.
    pub ssrf: SsrfPolicy,
}

/// Unified parameters for one audio generation call.
pub struct AudioGenParams<'a> {
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub kind: AudioKind,
    pub timeout_secs: u64,
    /// Target duration (seconds) for music / SFX; `None` = provider
    /// default. Each adapter clamps to its own legal range.
    pub duration_seconds: Option<f64>,
    /// Resolved voice id (call-level → model default → provider default);
    /// `None` lets the adapter fall back to its built-in voice.
    pub voice: Option<&'a str>,
    /// Merged provider `extra` ← model `extra`.
    pub extra: &'a HashMap<String, String>,
    /// SSRF policy derived from the provider's `allow_private_network`.
    pub ssrf: SsrfPolicy,
}

// ── Adapter traits ────────────────────────────────────────────────

pub trait ImageGenAdapter: Send + Sync {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>>;
}

pub trait AudioGenAdapter: Send + Sync {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>>;
}

// ── Registry ──────────────────────────────────────────────────────

// ── OpenAI-compatible vendor profiles ─────────────────────────────
//
// Each entry is one vendor's documented deviation from the OpenAI images
// body. Transcribed from official API references — a wrong field here is a
// silent 400 at generation time, so cite the doc when changing one.

use image::openai_compat::{CompatProfile, CompatProvider, SizeStyle};

/// StepFun: OpenAI-shaped generation; img2img is a *separate* endpoint
/// keyed on `source_url`. `step-image-edit-2`'s `/v1/images/edits` is
/// multipart and therefore out of this adapter's reach (see catalog caps).
static STEPFUN_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "stepfun",
    path: "/v1/images/generations",
    edit_path: Some("/v1/images/image2image"),
    edit_omits_size: false,
    size_style: SizeStyle::Pixels,
    send_n: true,
    response_format: Some("b64_json"),
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: Some("source_url"),
    input_image_array: false,
    size_allowlist: &[],
    extra_body: &[],
});

/// Volcengine Ark (Doubao Seedream). No `n` — batches go through
/// `sequential_image_generation`. `watermark` defaults to *true* upstream,
/// which stamps "AI 生成" on every image, so we opt out explicitly.
static VOLCENGINE_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "volcengine",
    path: "/api/v3/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::Pixels,
    send_n: false,
    response_format: Some("b64_json"),
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: Some("image"),
    input_image_array: true,
    size_allowlist: &[],
    extra_body: &[("watermark", "false")],
});

/// Tencent Hunyuan via TokenHub: sizes use a colon separator, results come
/// back as URLs only.
static HUNYUAN_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "hunyuan",
    path: "/v1/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::Colon,
    send_n: false,
    response_format: None,
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: Some("images"),
    input_image_array: true,
    size_allowlist: &[],
    extra_body: &[],
});

/// Together AI: no `size` field at all — dimensions are `width`/`height`.
/// Its `response_format` enum is `base64`/`url`, *not* OpenAI's `b64_json`,
/// even though the response field is still named `b64_json`.
static TOGETHER_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "together",
    path: "/v1/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::WidthHeight,
    send_n: true,
    response_format: Some("base64"),
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: Some("image_url"),
    input_image_array: false,
    size_allowlist: &[],
    extra_body: &[],
});

/// xAI Grok Imagine: no size/quality/style knobs; dimensions are expressed
/// as aspect ratio + resolution tier.
static XAI_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "xai",
    path: "/v1/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::Omit,
    send_n: true,
    response_format: Some("b64_json"),
    send_aspect_ratio: true,
    send_resolution: true,
    input_image_field: None,
    input_image_array: false,
    size_allowlist: &[],
    extra_body: &[],
});

/// Recraft: plain OpenAI shape on the main endpoint. Its single-image tool
/// endpoints (vectorize / removeBackground / …) return a bare `image`
/// object instead of `data[]` and are deliberately not routed here.
static RECRAFT_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "recraft",
    path: "/v1/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::Pixels,
    send_n: true,
    response_format: Some("b64_json"),
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: None,
    input_image_array: false,
    size_allowlist: &[],
    extra_body: &[],
});

/// Baidu Qianfan: URL-only results (24h expiry), edits on their own path.
static QIANFAN_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "qianfan",
    path: "/v2/images/generations",
    edit_path: Some("/v2/images/edits"),
    edit_omits_size: false,
    size_style: SizeStyle::Pixels,
    send_n: true,
    response_format: None,
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: Some("image"),
    input_image_array: false,
    size_allowlist: &[],
    extra_body: &[],
});

/// SenseNova: OpenAI-ish request, but the response is a top-level
/// `images_urls` array — an OpenAI SDK cannot deserialize it.
static SENSENOVA_IMAGE: CompatProvider = CompatProvider(CompatProfile {
    vendor: "sensenova",
    path: "/v1/images/generations",
    edit_path: None,
    edit_omits_size: false,
    size_style: SizeStyle::Pixels,
    send_n: false,
    response_format: Some("url"),
    send_aspect_ratio: false,
    send_resolution: false,
    input_image_field: None,
    input_image_array: false,
    // Only these buckets are accepted; anything else fails server-side.
    size_allowlist: &[
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
    ],
    extra_body: &[("output_format", "png")],
});

/// Image adapter for a vendor. `None` = vendor has no image wire
/// (candidate filtering normally prevents this from being hit).
pub fn image_adapter(kind: MediaVendorKind) -> Option<&'static dyn ImageGenAdapter> {
    match kind {
        // OpenAI-compatible endpoints share the OpenAI images wire shape.
        MediaVendorKind::Openai | MediaVendorKind::OpenaiCompatible => {
            Some(&image::openai::OpenAIProvider)
        }
        MediaVendorKind::Google => Some(&image::google::GoogleProvider),
        MediaVendorKind::Fal => Some(&image::fal::FalProvider),
        MediaVendorKind::Minimax => Some(&image::minimax::MiniMaxProvider),
        MediaVendorKind::Siliconflow => Some(&image::siliconflow::SiliconFlowProvider),
        MediaVendorKind::Zhipu => Some(&image::zhipu::ZhipuProvider),
        MediaVendorKind::Tongyi => Some(&image::tongyi::TongyiProvider),
        MediaVendorKind::Stepfun => Some(&STEPFUN_IMAGE),
        MediaVendorKind::Volcengine => Some(&VOLCENGINE_IMAGE),
        MediaVendorKind::Hunyuan => Some(&HUNYUAN_IMAGE),
        MediaVendorKind::Together => Some(&TOGETHER_IMAGE),
        MediaVendorKind::Xai => Some(&XAI_IMAGE),
        MediaVendorKind::Recraft => Some(&RECRAFT_IMAGE),
        MediaVendorKind::Qianfan => Some(&QIANFAN_IMAGE),
        MediaVendorKind::Sensenova => Some(&SENSENOVA_IMAGE),
        MediaVendorKind::Elevenlabs
        | MediaVendorKind::Cartesia
        | MediaVendorKind::Deepgram
        | MediaVendorKind::Fishaudio
        | MediaVendorKind::Hume => None,
    }
}

/// Audio adapter for a vendor.
pub fn audio_adapter(kind: MediaVendorKind) -> Option<&'static dyn AudioGenAdapter> {
    match kind {
        MediaVendorKind::Openai | MediaVendorKind::OpenaiCompatible => {
            Some(&audio::openai::OpenAiAudioProvider)
        }
        MediaVendorKind::Elevenlabs => Some(&audio::elevenlabs::ElevenLabsAudioProvider),
        MediaVendorKind::Cartesia => Some(&audio::cartesia::Provider),
        MediaVendorKind::Deepgram => Some(&audio::deepgram::Provider),
        MediaVendorKind::Fishaudio => Some(&audio::fishaudio::Provider),
        MediaVendorKind::Hume => Some(&audio::hume::Provider),
        MediaVendorKind::Minimax => Some(&audio::minimax::Provider),
        // StepFun / xAI / SenseNova speak OpenAI's `/v1/audio/speech`.
        MediaVendorKind::Stepfun | MediaVendorKind::Xai | MediaVendorKind::Sensenova => {
            Some(&audio::openai::OpenAiAudioProvider)
        }
        MediaVendorKind::Volcengine
        | MediaVendorKind::Hunyuan
        | MediaVendorKind::Together
        | MediaVendorKind::Recraft
        | MediaVendorKind::Qianfan
        | MediaVendorKind::Google
        | MediaVendorKind::Fal
        | MediaVendorKind::Siliconflow
        | MediaVendorKind::Zhipu
        | MediaVendorKind::Tongyi => None,
    }
}

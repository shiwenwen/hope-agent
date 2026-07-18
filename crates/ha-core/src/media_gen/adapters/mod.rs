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
        MediaVendorKind::Elevenlabs => None,
    }
}

/// Audio adapter for a vendor.
pub fn audio_adapter(kind: MediaVendorKind) -> Option<&'static dyn AudioGenAdapter> {
    match kind {
        MediaVendorKind::Openai | MediaVendorKind::OpenaiCompatible => {
            Some(&audio::openai::OpenAiAudioProvider)
        }
        MediaVendorKind::Elevenlabs => Some(&audio::elevenlabs::ElevenLabsAudioProvider),
        MediaVendorKind::Google
        | MediaVendorKind::Fal
        | MediaVendorKind::Minimax
        | MediaVendorKind::Siliconflow
        | MediaVendorKind::Zhipu
        | MediaVendorKind::Tongyi => None,
    }
}

//! Data model for the unified media-generation subsystem (image / audio,
//! `video` reserved).
//!
//! Mirrors the STT subsystem's "independent provider list" approach
//! (`stt::types` in ha-core): media providers are intentionally NOT part of
//! the LLM provider list — their semantic dimensions (modality, geometry
//! capabilities, audio kinds, voices) and wire protocols do not fit
//! `provider::ApiType`. One provider entry (credentials configured once)
//! carries multiple models; each model declares its modality + data-driven
//! capabilities. Per-function default chains (image / speech / music / sfx)
//! pick which model serves each feature.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Audio kind ────────────────────────────────────────────────────

/// Audio sub-capability. Models differ in what they support, so chain
/// validation and auto-candidate filtering only consider models that
/// declare the requested kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// Suggested duration buckets (seconds) for music / SFX pickers. UI hint
/// only — not a per-model constraint.
pub const AUDIO_DURATIONS_SEC: &[u32] = &[5, 10, 15, 30, 60, 120];

// ── Modality ──────────────────────────────────────────────────────

/// What a media model produces. `Video` is reserved for a future provider
/// stack — no adapter, template, or UI ships for it yet; it exists so the
/// config schema doesn't churn when video generation lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaModality {
    Image,
    Audio,
    Video,
}

impl MediaModality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

// ── Vendor kind ───────────────────────────────────────────────────

/// Wire protocol / adapter family used to talk to a media provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MediaVendorKind {
    /// OpenAI images (`/v1/images/generations|edits`) + TTS (`/v1/audio/speech`).
    Openai,
    /// Google Gemini / Imagen image generation.
    Google,
    /// Fal.ai hosted diffusion models.
    Fal,
    /// MiniMax image generation.
    Minimax,
    /// SiliconFlow (OpenAI-ish images endpoint with custom sizes).
    Siliconflow,
    /// ZhipuAI CogView.
    Zhipu,
    /// Alibaba Tongyi Wanxiang (DashScope async task API).
    Tongyi,
    /// ElevenLabs TTS / music / SFX.
    Elevenlabs,
    /// StepFun images (`/v1/images/generations`) + TTS (`/v1/audio/speech`).
    Stepfun,
    /// Volcengine Ark images (ByteDance Doubao Seedream). TTS lives on a
    /// different host with different auth — modelled separately.
    Volcengine,
    /// Tencent Hunyuan images via the TokenHub plane (`tokenhub.tencentmaas.com`).
    Hunyuan,
    /// Together AI hosted image models (OpenAI-ish, `base64` response token).
    Together,
    /// xAI Grok Imagine images.
    Xai,
    /// Recraft images — the only vendor here with native vector (SVG) output.
    Recraft,
    /// Baidu Qianfan images (`qianfan.baidubce.com/v2`).
    Qianfan,
    /// SenseNova images — OpenAI-ish request, `images_urls` response envelope.
    Sensenova,
    /// Cartesia Sonic TTS.
    Cartesia,
    /// Deepgram Aura TTS — voice *is* the model id.
    Deepgram,
    /// Fish Audio TTS — model travels in an HTTP header.
    Fishaudio,
    /// Hume Octave TTS — no `model` field, `version` selects the generation.
    Hume,
    /// Black Forest Labs FLUX — `x-key` auth, submit + poll, model in the path.
    Bfl,
    /// Stability AI — multipart images, plus Stable Audio music / SFX.
    Stability,
    /// Replicate — prediction submit + poll over arbitrary hosted models.
    Replicate,
    /// Kuaishou Kling — async task API, region-split hosts.
    Kling,
    /// iFlytek Spark — HMAC-signed URLs, bespoke three-section JSON.
    Iflytek,
    /// Volcengine "Doubao" speech. Separate from [`Self::Volcengine`]: a
    /// different host, different auth headers, and a streaming NDJSON body.
    VolcengineTts,
    /// Self-hosted or third-party endpoint speaking the OpenAI wire shape
    /// (images `/v1/images/generations`, speech `/v1/audio/speech`).
    /// Requires an explicit `base_url`; routed through the OpenAI adapters.
    OpenaiCompatible,
}

impl MediaVendorKind {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com",
            Self::Google => "https://generativelanguage.googleapis.com",
            Self::Fal => "https://fal.run",
            Self::Minimax => "https://api.minimax.io",
            Self::Siliconflow => "https://api.siliconflow.cn",
            Self::Zhipu => "https://open.bigmodel.cn/api/paas",
            Self::Tongyi => "https://dashscope.aliyuncs.com",
            Self::Elevenlabs => "https://api.elevenlabs.io",
            Self::Stepfun => "https://api.stepfun.com",
            Self::Volcengine => "https://ark.cn-beijing.volces.com",
            Self::Hunyuan => "https://tokenhub.tencentmaas.com",
            Self::Together => "https://api.together.ai",
            Self::Xai => "https://api.x.ai",
            Self::Recraft => "https://external.api.recraft.ai",
            Self::Qianfan => "https://qianfan.baidubce.com",
            Self::Sensenova => "https://token.sensenova.cn",
            Self::Cartesia => "https://api.cartesia.ai",
            Self::Deepgram => "https://api.deepgram.com",
            Self::Fishaudio => "https://api.fish.audio",
            Self::Hume => "https://api.hume.ai",
            Self::Bfl => "https://api.bfl.ai",
            Self::Stability => "https://api.stability.ai",
            Self::Replicate => "https://api.replicate.com",
            Self::Kling => "https://api-singapore.klingai.com",
            Self::Iflytek => "https://spark-api.cn-huabei-1.xf-yun.com",
            Self::VolcengineTts => "https://openspeech.bytedance.com",
            Self::OpenaiCompatible => "",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Openai => "OpenAI",
            Self::Google => "Google",
            Self::Fal => "Fal",
            Self::Minimax => "MiniMax",
            Self::Siliconflow => "SiliconFlow",
            Self::Zhipu => "ZhipuAI",
            Self::Tongyi => "Tongyi Wanxiang",
            Self::Elevenlabs => "ElevenLabs",
            Self::Stepfun => "StepFun",
            Self::Volcengine => "Volcengine Ark",
            Self::Hunyuan => "Tencent Hunyuan",
            Self::Together => "Together AI",
            Self::Xai => "xAI",
            Self::Recraft => "Recraft",
            Self::Qianfan => "Baidu Qianfan",
            Self::Sensenova => "SenseNova",
            Self::Cartesia => "Cartesia",
            Self::Deepgram => "Deepgram",
            Self::Fishaudio => "Fish Audio",
            Self::Hume => "Hume AI",
            Self::Bfl => "Black Forest Labs",
            Self::Stability => "Stability AI",
            Self::Replicate => "Replicate",
            Self::Kling => "Kling",
            Self::Iflytek => "iFlytek Spark",
            Self::VolcengineTts => "Doubao Speech",
            Self::OpenaiCompatible => "OpenAI-compatible",
        }
    }

    /// Whether this vendor exposes a listable voice catalog (used to gate
    /// the "fetch voices" UI + `list_media_voices` command).
    ///
    /// **Must stay in sync with the arms in ha-core's `media_gen::voices`**
    /// — claiming a catalog we cannot fetch turns the UI button into a
    /// guaranteed error. Deepgram is deliberately absent: its voices *are*
    /// model ids, so the model picker already covers them. StepFun / Fish
    /// Audio / Hume do publish listing endpoints, but their response shapes
    /// are not verified yet, so they keep the free-form voice input for now.
    pub fn supports_voice_listing(&self) -> bool {
        matches!(
            self,
            Self::Elevenlabs
                | Self::Openai
                | Self::OpenaiCompatible
                | Self::Cartesia
                | Self::Minimax
        )
    }
}

// ── Model capabilities ────────────────────────────────────────────

fn one() -> u32 {
    1
}

/// Image capabilities in edit mode (with input/reference images).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageEditCaps {
    #[serde(default = "one")]
    pub max_n: u32,
    #[serde(default = "one")]
    pub max_input_images: u32,
    #[serde(default)]
    pub supports_size: bool,
    #[serde(default)]
    pub supports_aspect_ratio: bool,
    #[serde(default)]
    pub supports_resolution: bool,
}

/// Data-driven image model capabilities (replaces the old hardcoded
/// per-provider `ImageGenCapabilities` trait method).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImageModelCaps {
    #[serde(default = "one")]
    pub max_n: u32,
    #[serde(default)]
    pub supports_size: bool,
    #[serde(default)]
    pub supports_aspect_ratio: bool,
    #[serde(default)]
    pub supports_resolution: bool,
    /// Accepted size strings; empty = unconstrained.
    #[serde(default)]
    pub sizes: Vec<String>,
    /// Accepted aspect-ratio strings; empty = unconstrained.
    #[serde(default)]
    pub aspect_ratios: Vec<String>,
    /// Accepted resolution tiers ("1K"/"2K"/"4K"); empty = unconstrained.
    #[serde(default)]
    pub resolutions: Vec<String>,
    /// Honors an inpaint mask (`/images/edits`-style). Vendors without mask
    /// support would silently regenerate the whole image, so the inpaint
    /// path only considers models with this flag.
    #[serde(default)]
    pub supports_mask: bool,
    /// `None` = editing (img2img) unsupported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<ImageEditCaps>,
}

/// Data-driven audio model capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioModelCaps {
    /// Which audio kinds this model can produce.
    #[serde(default)]
    pub kinds: Vec<AudioKind>,
    /// Whether the model accepts a target duration (music / SFX).
    #[serde(default)]
    pub supports_duration: bool,
    /// Whether the model takes a voice id (TTS).
    #[serde(default)]
    pub needs_voice: bool,
    /// Model-level default voice (overrides the provider-level default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_secs: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<f64>,
}

// ── Model ─────────────────────────────────────────────────────────

/// One model on a media provider. Flat struct + optional capability groups
/// (not a tagged enum): serde stays simple for the frontend, and a
/// caps-less user-added model degrades gracefully (lenient validation —
/// see `resolve`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaModelConfig {
    /// Provider-side model id, e.g. `gpt-image-1`, `eleven_v3`.
    pub id: String,
    /// Display name for the UI.
    pub name: String,
    pub modality: MediaModality,
    /// Set when `modality == Image`. `None` on an image model = unknown
    /// capabilities → requests pass through unvalidated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageModelCaps>,
    /// Set when `modality == Audio`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioModelCaps>,
    /// Model-level extra generation params (e.g. Google `thinking_level`).
    /// Merged over provider-level `extra` at request time (model wins).
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl MediaModelConfig {
    pub fn new(id: impl Into<String>, name: impl Into<String>, modality: MediaModality) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            modality,
            image: None,
            audio: None,
            extra: HashMap::new(),
        }
    }

    /// Whether this model can serve `function`. Lenient on missing caps:
    /// an audio model without a caps group is assumed to handle any kind
    /// (the provider will reject what it can't do).
    pub fn serves(&self, function: MediaFunction) -> bool {
        match function {
            MediaFunction::Image => self.modality == MediaModality::Image,
            MediaFunction::Audio(kind) => {
                self.modality == MediaModality::Audio
                    && self
                        .audio
                        .as_ref()
                        .map(|caps| caps.kinds.is_empty() || caps.kinds.contains(&kind))
                        .unwrap_or(true)
            }
        }
    }
}

// ── Provider ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProviderConfig {
    /// Stable UUID.
    pub id: String,
    /// User-defined display name.
    pub name: String,
    /// Adapter family.
    pub kind: MediaVendorKind,
    /// Endpoint override; `None`/empty = vendor default.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub models: Vec<MediaModelConfig>,
    /// Provider-level default TTS voice (overridden by model-level
    /// `audio.default_voice`, then by the per-call voice argument).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_voice: Option<String>,
    /// Allow loopback / private-network destinations (self-hosted
    /// OpenAI-compatible endpoints).
    #[serde(default)]
    pub allow_private_network: bool,
    /// Provider-specific extras. Treated as secrets — redacted in
    /// `masked()` and settings reads.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl MediaProviderConfig {
    pub fn new(name: impl Into<String>, kind: MediaVendorKind) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind,
            base_url: None,
            api_key: String::new(),
            enabled: true,
            models: Vec::new(),
            default_voice: None,
            allow_private_network: false,
            extra: HashMap::new(),
        }
    }

    /// Effective endpoint: explicit `base_url` (non-empty) → vendor default.
    pub fn effective_base_url(&self) -> &str {
        match self.base_url.as_deref() {
            Some(url) if !url.trim().is_empty() => url,
            _ => self.kind.default_base_url(),
        }
    }

    pub fn model_config(&self, model_id: &str) -> Option<&MediaModelConfig> {
        self.models.iter().find(|m| m.id == model_id)
    }

    /// Usable in failover candidate lists: enabled + has credentials.
    /// OpenAI-compatible self-hosted endpoints may legitimately run
    /// key-less, so a configured base_url counts as "has credentials".
    pub fn is_usable(&self) -> bool {
        self.enabled
            && (!self.api_key.trim().is_empty()
                || (self.kind == MediaVendorKind::OpenaiCompatible
                    && !self.effective_base_url().is_empty()))
    }

    /// Return a copy with all secrets masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: crate::mask_secret_middle(&self.api_key, 4, 4),
            extra: self
                .extra
                .iter()
                .map(|(k, v)| (k.clone(), crate::mask_secret_middle(v, 4, 4)))
                .collect(),
            ..self.clone()
        }
    }
}

// ── Function + chains ─────────────────────────────────────────────

/// A feature slot that consumes media generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaFunction {
    Image,
    Audio(AudioKind),
}

impl MediaFunction {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "image" => Self::Image,
            other => Self::Audio(AudioKind::parse(other)?),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio(kind) => kind.as_str(),
        }
    }
}

impl std::fmt::Display for MediaFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reference to one model on one provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaModelRef {
    pub provider_id: String,
    pub model_id: String,
}

impl std::fmt::Display for MediaModelRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.provider_id, self.model_id)
    }
}

/// Primary + ordered fallbacks for one function.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaModelChain {
    pub primary: MediaModelRef,
    #[serde(default)]
    pub fallbacks: Vec<MediaModelRef>,
}

impl MediaModelChain {
    pub fn into_vec(self) -> Vec<MediaModelRef> {
        let mut v = Vec::with_capacity(1 + self.fallbacks.len());
        v.push(self.primary);
        v.extend(self.fallbacks);
        v
    }

    pub fn iter(&self) -> impl Iterator<Item = &MediaModelRef> {
        std::iter::once(&self.primary).chain(self.fallbacks.iter())
    }
}

/// Per-function default chains. `None` = auto (provider order × capability
/// filter). A configured chain is authoritative: exhaustion fails the call
/// rather than sliding to providers the user didn't pick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDefaultChains {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<MediaModelChain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<MediaModelChain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<MediaModelChain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfx: Option<MediaModelChain>,
}

impl MediaDefaultChains {
    pub fn for_function(&self, function: MediaFunction) -> Option<&MediaModelChain> {
        match function {
            MediaFunction::Image => self.image.as_ref(),
            MediaFunction::Audio(AudioKind::Speech) => self.speech.as_ref(),
            MediaFunction::Audio(AudioKind::Music) => self.music.as_ref(),
            MediaFunction::Audio(AudioKind::Sfx) => self.sfx.as_ref(),
        }
    }

    pub fn set_for_function(&mut self, function: MediaFunction, chain: Option<MediaModelChain>) {
        match function {
            MediaFunction::Image => self.image = chain,
            MediaFunction::Audio(AudioKind::Speech) => self.speech = chain,
            MediaFunction::Audio(AudioKind::Music) => self.music = chain,
            MediaFunction::Audio(AudioKind::Sfx) => self.sfx = chain,
        }
    }

    /// All four slots, for reference-cleanup sweeps.
    pub fn slots_mut(&mut self) -> [&mut Option<MediaModelChain>; 4] {
        [
            &mut self.image,
            &mut self.speech,
            &mut self.music,
            &mut self.sfx,
        ]
    }
}

// ── Tool defaults ─────────────────────────────────────────────────

/// Timeout clamp band (seconds). Generation is slow — the floor keeps a
/// mis-set config from failing every call, the ceiling keeps a stuck
/// provider from pinning a slot for an hour.
pub const TIMEOUT_CLAMP_SECS: (u64, u64) = (30, 900);

fn d_img_timeout() -> u64 {
    180
}

fn d_aud_timeout() -> u64 {
    300
}

fn d_size() -> String {
    "1024x1024".to_string()
}

/// Global defaults for image generation (tool + design paths).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenDefaults {
    /// Master switch for the `image_generate` tool surface.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Request timeout (seconds). Read through `effective_timeout_secs`.
    #[serde(default = "d_img_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "d_size")]
    pub default_size: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_aspect_ratio: Option<String>,
    /// "1K" / "2K" / "4K".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_resolution: Option<String>,
}

impl Default for ImageGenDefaults {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_seconds: d_img_timeout(),
            default_size: d_size(),
            default_aspect_ratio: None,
            default_resolution: None,
        }
    }
}

impl ImageGenDefaults {
    pub fn effective_timeout_secs(&self) -> u64 {
        self.timeout_seconds
            .clamp(TIMEOUT_CLAMP_SECS.0, TIMEOUT_CLAMP_SECS.1)
    }
}

/// Global defaults for audio generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioGenDefaults {
    /// Master switch for the `audio_generate` tool surface.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default = "d_aud_timeout")]
    pub timeout_seconds: u64,
    /// Default duration for music / SFX when the caller doesn't specify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_duration_secs: Option<f64>,
}

impl Default for AudioGenDefaults {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_seconds: d_aud_timeout(),
            default_duration_secs: None,
        }
    }
}

impl AudioGenDefaults {
    pub fn effective_timeout_secs(&self) -> u64 {
        self.timeout_seconds
            .clamp(TIMEOUT_CLAMP_SECS.0, TIMEOUT_CLAMP_SECS.1)
    }
}

// ── Subsystem config ──────────────────────────────────────────────

/// Persistent media-generation configuration (`AppConfig.media_gen`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaGenConfig {
    /// User-configured providers. Order = auto-mode candidate priority.
    #[serde(default)]
    pub providers: Vec<MediaProviderConfig>,
    /// Per-function default chains.
    #[serde(default)]
    pub chains: MediaDefaultChains,
    #[serde(default)]
    pub image_defaults: ImageGenDefaults,
    #[serde(default)]
    pub audio_defaults: AudioGenDefaults,
}

impl MediaGenConfig {
    pub fn provider(&self, provider_id: &str) -> Option<&MediaProviderConfig> {
        self.providers.iter().find(|p| p.id == provider_id)
    }

    /// Any usable provider carrying a model of `modality`?
    pub fn has_capable_provider(&self, modality: MediaModality) -> bool {
        self.providers
            .iter()
            .any(|p| p.is_usable() && p.models.iter().any(|m| m.modality == modality))
    }
}

// ── Request-side validation constants ─────────────────────────────

pub const VALID_ASPECT_RATIOS: &[&str] = &[
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9",
];

pub const VALID_RESOLUTIONS: &[&str] = &["1K", "2K", "4K"];

pub const MAX_INPUT_IMAGES: usize = 5;

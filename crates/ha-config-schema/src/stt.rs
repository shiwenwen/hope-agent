//! STT (Speech-to-Text) subsystem config types (`AppConfig.stt`).
//!
//! The STT subsystem is intentionally independent of the LLM provider list:
//! its semantic dimensions (per-minute cost / streaming capability / language
//! coverage) and its multi-protocol surface (OpenAI multipart, SSE, several
//! flavours of WebSocket) do not fit cleanly into `provider::ApiType`. The
//! design mirrors the embedding subsystem's "independent model list"
//! approach.
//!
//! 运行时类型（`Transcript` / `TranscriptDelta` / `AudioPayload` 等）与子系统
//! 逻辑（`check_ssrf` / `require_extra` 自由函数）留在 `ha-core::stt::types`。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::provider::AuthProfile;

// ── Provider kind ─────────────────────────────────────────────────

/// Wire protocol used to talk to an STT provider.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SttProviderKind {
    /// OpenAI `/v1/audio/transcriptions` (multipart upload). Also drives the
    /// `gpt-4o-transcribe` / `gpt-4o-mini-transcribe` SSE stream variants.
    OpenaiTranscriptions,
    /// Third-party OpenAI-compatible endpoints sharing the
    /// `/v1/audio/transcriptions` multipart wire format (Groq, StepFun,
    /// SiliconFlow, whisper.cpp server, faster-whisper-server, FunASR +
    /// OpenAI wrapper, sherpa-onnx server). DashScope is NOT in this set —
    /// it dispatches ASR through chat-completions; use
    /// `OpenaiChatCompletionsAsr` for that wire shape.
    OpenaiCompatible,
    /// OpenAI chat-completions endpoint with `input_audio` content blocks,
    /// as used by Alibaba DashScope's Qwen3-ASR family. The audio is
    /// inlined as a base64 data URI; the model returns the transcript as
    /// the assistant message body.
    OpenaiChatCompletionsAsr,
    /// Deepgram realtime WebSocket.
    DeepgramWs,
    /// AssemblyAI realtime WebSocket.
    AssemblyaiWs,
    /// Azure Speech-to-Text WebSocket.
    AzureWs,
    /// Volcengine / bytedance bigmodel STT (binary WebSocket frames).
    VolcengineWs,
    /// iFlytek IAT WebSocket with hmac-sha256 signed URL.
    XunfeiWs,
    /// ElevenLabs Scribe batch transcription (`POST /v1/speech-to-text`,
    /// multipart with a `model_id` field and `xi-api-key` auth header — not
    /// OpenAI-shaped, so it needs its own batch provider).
    ElevenlabsStt,
    /// xAI Grok STT batch transcription (`POST /v1/stt`, multipart with a
    /// `model` field and Bearer auth — a custom REST wire, not OpenAI's
    /// `/v1/audio/transcriptions`).
    XaiStt,
}

impl SttProviderKind {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            SttProviderKind::OpenaiTranscriptions => "https://api.openai.com",
            SttProviderKind::OpenaiCompatible => "http://127.0.0.1:8080",
            SttProviderKind::OpenaiChatCompletionsAsr => "",
            SttProviderKind::DeepgramWs => "wss://api.deepgram.com",
            SttProviderKind::AssemblyaiWs => "wss://api.assemblyai.com",
            SttProviderKind::AzureWs => "wss://westus.stt.speech.microsoft.com",
            SttProviderKind::VolcengineWs => "wss://openspeech.bytedance.com",
            SttProviderKind::XunfeiWs => "wss://iat-api.xfyun.cn",
            SttProviderKind::ElevenlabsStt => "https://api.elevenlabs.io",
            SttProviderKind::XaiStt => "https://api.x.ai",
        }
    }

    /// Whether the wire protocol supports streaming partial transcripts.
    /// Plain OpenAI Whisper does not; gpt-4o-transcribe does via SSE — but
    /// streaming support is also a per-model capability, so this is just a
    /// coarse hint for UI gating. DashScope chat-completions ASR is batch-
    /// only (no `stream:true` for `input_audio` content blocks yet).
    pub fn supports_streaming(&self) -> bool {
        !matches!(
            self,
            SttProviderKind::OpenaiTranscriptions
                | SttProviderKind::OpenaiChatCompletionsAsr
                | SttProviderKind::ElevenlabsStt
                | SttProviderKind::XaiStt
        )
    }

    /// Whether the wire protocol uploads the audio as multipart form-data
    /// (true for OpenAI-style transcriptions endpoints). False for
    /// WebSocket providers AND for DashScope-style chat-completions ASR
    /// (which sends a JSON body with a base64 data-URI).
    pub fn uses_multipart_upload(&self) -> bool {
        matches!(
            self,
            SttProviderKind::OpenaiTranscriptions
                | SttProviderKind::OpenaiCompatible
                | SttProviderKind::ElevenlabsStt
                | SttProviderKind::XaiStt
        )
    }

    /// Whether `engine::transcribe_with` can fulfil a batch (record-then-
    /// transcribe) request for this kind. The WS-only kinds reject batch
    /// with `Other(...)`. Used to gate `active_model` / `im_fallback_model`
    /// selectors so users can't pin a config that the desktop voice button
    /// / IM auto-transcribe path would always fail to use.
    pub fn supports_batch(&self) -> bool {
        matches!(
            self,
            SttProviderKind::OpenaiTranscriptions
                | SttProviderKind::OpenaiCompatible
                | SttProviderKind::OpenaiChatCompletionsAsr
                | SttProviderKind::ElevenlabsStt
                | SttProviderKind::XaiStt
        )
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SttProviderKind::OpenaiTranscriptions => "OpenAI Audio Transcriptions",
            SttProviderKind::OpenaiCompatible => "OpenAI-compatible",
            SttProviderKind::OpenaiChatCompletionsAsr => "Chat Completions ASR (input_audio)",
            SttProviderKind::DeepgramWs => "Deepgram",
            SttProviderKind::AssemblyaiWs => "AssemblyAI",
            SttProviderKind::AzureWs => "Azure Speech",
            SttProviderKind::VolcengineWs => "Volcengine",
            SttProviderKind::XunfeiWs => "iFlytek IAT",
            SttProviderKind::ElevenlabsStt => "ElevenLabs Scribe",
            SttProviderKind::XaiStt => "xAI Grok STT",
        }
    }
}

// ── Model ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SttModelConfig {
    /// Provider-side model id, e.g. `whisper-1`, `nova-3`, `paraformer-zh`.
    pub id: String,
    /// Display name for the UI.
    pub name: String,
    /// Whether this model supports streaming partial transcripts.
    #[serde(default)]
    pub supports_streaming: bool,
    /// BCP-47 / ISO 639-1 language codes the model handles well. Empty means
    /// "multilingual / auto-detect" — the UI shows it as such.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Cost per minute of audio (USD). `0.0` means free / local / unknown.
    #[serde(default)]
    pub cost_per_minute: f64,
    /// Whether the provider returns word-level timestamps for this model.
    #[serde(default)]
    pub supports_timestamps: bool,
    /// Whether the provider returns speaker labels for this model.
    #[serde(default)]
    pub supports_diarization: bool,
}

impl SttModelConfig {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            supports_streaming: false,
            languages: Vec::new(),
            cost_per_minute: 0.0,
            supports_timestamps: false,
            supports_diarization: false,
        }
    }
}

// ── Provider ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttProviderConfig {
    /// Stable UUID.
    pub id: String,
    /// User-defined display name.
    pub name: String,
    /// Wire protocol family.
    pub kind: SttProviderKind,
    /// API endpoint. HTTPS for OpenAI-style, WSS for streaming providers.
    pub base_url: String,
    /// Legacy single API key. Prefer `auth_profiles` for rotation.
    #[serde(default)]
    pub api_key: String,
    /// Multiple API keys with optional per-key base_url override (rotation).
    #[serde(default)]
    pub auth_profiles: Vec<AuthProfile>,
    /// Available models on this provider.
    #[serde(default)]
    pub models: Vec<SttModelConfig>,
    /// Whether the provider participates in active / failover selection.
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    /// Allow loopback / private network destinations. Required for the local
    /// backends (whisper.cpp / faster-whisper / FunASR / sherpa-onnx servers).
    #[serde(default)]
    pub allow_private_network: bool,
    /// Provider-specific extras that are not API keys: `app_id`, `cluster`,
    /// `resource_id`, `region`, etc. Treated as secrets — redacted in
    /// `masked()` and in `read_settings` output.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl SttProviderConfig {
    pub fn new(
        name: impl Into<String>,
        kind: SttProviderKind,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind,
            base_url: base_url.into(),
            api_key: String::new(),
            auth_profiles: Vec::new(),
            models: Vec::new(),
            enabled: true,
            allow_private_network: false,
            extra: HashMap::new(),
        }
    }

    /// Return the effective list of enabled auth profiles for this provider.
    /// Mirrors `ProviderConfig::effective_profiles` but without the Codex
    /// OAuth special case.
    pub fn effective_profiles(&self) -> Vec<AuthProfile> {
        if !self.auth_profiles.is_empty() {
            return self
                .auth_profiles
                .iter()
                .filter(|p| p.enabled)
                .cloned()
                .collect();
        }
        if !self.api_key.is_empty() {
            return vec![AuthProfile {
                id: format!("__legacy__{}", self.id),
                label: "Default".to_string(),
                api_key: self.api_key.clone(),
                base_url: None,
                enabled: true,
            }];
        }
        Vec::new()
    }

    pub fn resolve_base_url<'a>(&'a self, profile: &'a AuthProfile) -> &'a str {
        profile.base_url.as_deref().unwrap_or(&self.base_url)
    }

    pub fn model_config(&self, model_id: &str) -> Option<&SttModelConfig> {
        self.models.iter().find(|m| m.id == model_id)
    }

    /// Return a copy with all secrets masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: crate::mask_secret_middle(&self.api_key, 4, 4),
            auth_profiles: self.auth_profiles.iter().map(|p| p.masked()).collect(),
            extra: self
                .extra
                .iter()
                .map(|(k, v)| (k.clone(), crate::mask_secret_middle(v, 4, 4)))
                .collect(),
            ..self.clone()
        }
    }
}

// ── Active selection + failover + IM fallback ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSttModel {
    pub provider_id: String,
    pub model_id: String,
}

impl std::fmt::Display for ActiveSttModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.provider_id, self.model_id)
    }
}

// ── Subsystem config ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttConfig {
    /// User-configured STT providers (cloud + local share one list).
    #[serde(default)]
    pub providers: Vec<SttProviderConfig>,
    /// Active STT model for desktop voice input.
    #[serde(default)]
    pub active_model: Option<ActiveSttModel>,
    /// Failover chain tried in order when the active model fails.
    #[serde(default)]
    pub fallback_models: Vec<ActiveSttModel>,
    /// Global fallback used by IM-channel auto-transcribe. Falls back to
    /// `active_model` when unset.
    #[serde(default)]
    pub im_fallback_model: Option<ActiveSttModel>,
    /// Default transcription options applied unless the caller overrides.
    #[serde(default)]
    pub default_options: TranscriptOptions,
}

// ── Transcript options ────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptOptions {
    /// BCP-47 / ISO 639-1 language hint; `None` = auto-detect.
    #[serde(default)]
    pub language: Option<String>,
    /// Free-form prompt that improves named-entity accuracy on supported
    /// providers (OpenAI, gpt-4o-transcribe).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Whether to request punctuation.
    #[serde(default)]
    pub punctuation: Option<bool>,
    /// Whether to request speaker diarization.
    #[serde(default)]
    pub diarization: Option<bool>,
    /// Whether to request word/segment timestamps.
    #[serde(default)]
    pub timestamps: Option<bool>,
    /// Audio sample rate reported by the front-end recorder (used by
    /// streaming providers that need to know the bitrate ahead of time).
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
}

impl TranscriptOptions {
    /// Merge per-request overrides on top of the configured defaults.
    ///
    /// String fields containing only whitespace are treated as absent so
    /// callers that serialize empty form inputs still inherit the default.
    /// Every other field uses `Some` as the explicit override signal.
    pub fn with_defaults(&self, defaults: &Self) -> Self {
        let normalize_string = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };

        Self {
            language: normalize_string(&self.language)
                .or_else(|| normalize_string(&defaults.language)),
            prompt: normalize_string(&self.prompt).or_else(|| normalize_string(&defaults.prompt)),
            punctuation: self.punctuation.or(defaults.punctuation),
            diarization: self.diarization.or(defaults.diarization),
            timestamps: self.timestamps.or(defaults.timestamps),
            sample_rate_hz: self.sample_rate_hz.or(defaults.sample_rate_hz),
        }
    }

    /// Canonicalize user-saved options without introducing provider-specific
    /// defaults. This keeps an empty language/prompt serialized as `None`.
    pub fn normalized(&self) -> Self {
        self.with_defaults(&Self::default())
    }
}

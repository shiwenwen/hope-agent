//! Data model for the STT (Speech-to-Text) subsystem.
//!
//! The STT subsystem is intentionally independent of the LLM provider list:
//! its semantic dimensions (per-minute cost / streaming capability / language
//! coverage) and its multi-protocol surface (OpenAI multipart, SSE, several
//! flavours of WebSocket) do not fit cleanly into `provider::ApiType`. The
//! design mirrors the embedding subsystem's "independent model list"
//! approach.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::provider::AuthProfile;

/// Hard cap on a single batch transcription audio payload. Matches the
/// OpenAI Whisper `/v1/audio/transcriptions` limit (25 MiB) and is enforced
/// at every entry point (Tauri command + HTTP route body limit) so an
/// over-sized base64 payload can't allocate gigabytes before failing.
pub const MAX_BATCH_AUDIO_BYTES: usize = 25 * 1024 * 1024;

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

    /// Resolve a required `extra` field with a uniform error shape so each
    /// provider doesn't repeat the same `ok_or_else` boilerplate. `label`
    /// is the human-readable name printed in the error (e.g. "APISecret").
    pub fn require_extra(&self, key: &str, label: &str) -> Result<&str, super::SttError> {
        self.extra
            .get(key)
            .filter(|v| !v.is_empty())
            .map(|s| s.as_str())
            .ok_or_else(|| {
                super::SttError::Other(format!(
                    "{:?} provider requires `extra.{}` ({})",
                    self.kind, key, label
                ))
            })
    }

    /// Shared SSRF gate for every outbound provider URL. Picks
    /// `AllowPrivate` when `allow_private_network` is set (used by
    /// localhost backends), otherwise falls back to the global default.
    pub async fn check_ssrf(&self, url: &str) -> Result<(), super::SttError> {
        let cfg = crate::config::cached_config();
        let policy = if self.allow_private_network {
            crate::security::ssrf::SsrfPolicy::AllowPrivate
        } else {
            cfg.ssrf.default_policy
        };
        crate::security::ssrf::check_url(url, policy, &cfg.ssrf.trusted_hosts)
            .await
            .map(|_| ())
            .map_err(|e| super::SttError::SsrfBlocked(e.to_string()))
    }

    /// Return a copy with all secrets masked for frontend display.
    pub fn masked(&self) -> Self {
        Self {
            api_key: mask_secret(&self.api_key),
            auth_profiles: self.auth_profiles.iter().map(|p| p.masked()).collect(),
            extra: self
                .extra
                .iter()
                .map(|(k, v)| (k.clone(), mask_secret(v)))
                .collect(),
            ..self.clone()
        }
    }
}

fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if value.chars().count() > 8 {
        let prefix: String = value.chars().take(4).collect();
        let suffix: String = value
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{}...{}", prefix, suffix)
    } else {
        "****".to_string()
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

// ── Transcript shape ──────────────────────────────────────────────

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptDelta {
    pub session_id: String,
    pub text: String,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub start_ms: Option<u64>,
    #[serde(default)]
    pub end_ms: Option<u64>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub language: Option<String>,
    /// Accumulated full-text so far (some providers only emit deltas, others
    /// emit the cumulative buffer). Optional — engines fill what they have.
    #[serde(default)]
    pub accumulated: Option<String>,
}

// ── Audio payload for engines ─────────────────────────────────────

/// Audio handed to an STT engine. Engines pick the cheapest path:
/// `File` lets them stream the bytes from disk without loading into RAM.
#[derive(Debug, Clone)]
pub enum AudioPayload {
    Bytes {
        mime_type: String,
        bytes: Vec<u8>,
        /// Filename hint used by multipart uploads (some providers reject
        /// uploads without a recognisable extension in the part filename).
        filename: String,
    },
    File {
        path: std::path::PathBuf,
        mime_type: String,
    },
}

impl AudioPayload {
    pub fn mime_type(&self) -> &str {
        match self {
            AudioPayload::Bytes { mime_type, .. } => mime_type,
            AudioPayload::File { mime_type, .. } => mime_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_redacts_extra_secrets() {
        let mut p = SttProviderConfig::new(
            "Volcengine",
            SttProviderKind::VolcengineWs,
            "wss://openspeech.bytedance.com",
        );
        p.api_key = "ak-real-key-1234567890".to_string();
        p.extra.insert("app_id".into(), "123456".into());
        p.extra
            .insert("access_key".into(), "secret-access-key-payload".into());

        let masked = p.masked();
        assert_ne!(masked.api_key, p.api_key);
        assert!(masked.api_key.contains("..."));
        assert_ne!(masked.extra["access_key"], "secret-access-key-payload");
        // Short values mask to "****" not "..."
        assert_eq!(masked.extra["app_id"], "****");
    }

    #[test]
    fn streaming_flag_distinguishes_openai_whisper_from_compatible() {
        assert!(!SttProviderKind::OpenaiTranscriptions.supports_streaming());
        assert!(SttProviderKind::OpenaiCompatible.supports_streaming());
        assert!(SttProviderKind::DeepgramWs.supports_streaming());
    }

    #[test]
    fn effective_profiles_falls_back_to_legacy_key() {
        let mut p = SttProviderConfig::new(
            "OpenAI",
            SttProviderKind::OpenaiTranscriptions,
            "https://api.openai.com",
        );
        p.api_key = "sk-test-1234567890".to_string();
        let profiles = p.effective_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].api_key, "sk-test-1234567890");
    }

    #[test]
    fn serde_round_trip_keeps_all_fields() {
        let mut cfg = SttConfig::default();
        cfg.providers.push({
            let mut p = SttProviderConfig::new(
                "OpenAI",
                SttProviderKind::OpenaiTranscriptions,
                "https://api.openai.com",
            );
            p.api_key = "sk-test".into();
            p.models.push(SttModelConfig {
                id: "whisper-1".into(),
                name: "Whisper".into(),
                supports_streaming: false,
                languages: vec!["en".into(), "zh".into()],
                cost_per_minute: 0.006,
                supports_timestamps: true,
                supports_diarization: false,
            });
            p
        });
        cfg.active_model = Some(ActiveSttModel {
            provider_id: cfg.providers[0].id.clone(),
            model_id: "whisper-1".into(),
        });

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: SttConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].models[0].cost_per_minute, 0.006);
        assert_eq!(
            parsed.active_model.unwrap().model_id,
            "whisper-1".to_string()
        );
    }
}

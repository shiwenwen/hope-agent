//! Data model for the STT (Speech-to-Text) subsystem.
//!
//! The STT subsystem is intentionally independent of the LLM provider list:
//! its semantic dimensions (per-minute cost / streaming capability / language
//! coverage) and its multi-protocol surface (OpenAI multipart, SSE, several
//! flavours of WebSocket) do not fit cleanly into `provider::ApiType`. The
//! design mirrors the embedding subsystem's "independent model list"
//! approach.
//!
//! 配置类型已下沉 [`ha_config_schema::stt`]（见下方 `pub use`）；本文件保留
//! 运行时类型（transcript / 音频载荷）与子系统逻辑（SSRF 闸、`require_extra`）。

// 类型已下沉 ha-config-schema：原地再导出保持 `crate::stt::types::*` 路径不变。
pub use ha_config_schema::stt::{
    ActiveSttModel, SttConfig, SttModelConfig, SttProviderConfig, SttProviderKind,
    TranscriptOptions,
};

use serde::{Deserialize, Serialize};

/// Hard cap on a single batch transcription audio payload. Matches the
/// OpenAI Whisper `/v1/audio/transcriptions` limit (25 MiB) and is enforced
/// at every entry point (Tauri command + HTTP route body limit) so an
/// over-sized base64 payload can't allocate gigabytes before failing.
pub const MAX_BATCH_AUDIO_BYTES: usize = 25 * 1024 * 1024;

// ── Transcript shape ──────────────────────────────────────────────

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

/// 每个出站 provider URL 的统一 SSRF 闸。`allow_private_network`（本地后端用）
/// 选 `AllowPrivate`，否则回落全局默认策略。
///
/// 原为 `SttProviderConfig` 的固有方法，因需读**运行时全局配置**而移出——
/// 该类型已下沉 `ha-config-schema`，schema 层是纯数据、不得读运行时状态。
pub async fn check_ssrf(provider: &SttProviderConfig, url: &str) -> Result<(), super::SttError> {
    let cfg = crate::config::cached_config();
    let policy = if provider.allow_private_network {
        crate::security::ssrf::SsrfPolicy::AllowPrivate
    } else {
        cfg.ssrf.default_policy
    };
    crate::security::ssrf::check_url(url, policy, &cfg.ssrf.trusted_hosts)
        .await
        .map(|_| ())
        .map_err(|e| super::SttError::SsrfBlocked(e.to_string()))
}

/// Resolve a required `extra` field with a uniform error shape so each
/// provider doesn't repeat the same `ok_or_else` boilerplate. `label`
/// is the human-readable name printed in the error (e.g. "APISecret").
///
/// 原为 `SttProviderConfig` 的固有方法，因错误类型 [`super::SttError`] 属
/// 子系统逻辑而移出——该类型已下沉 `ha-config-schema`，固有 impl 不能留在
/// 本 crate（coherence），故与 [`check_ssrf`] 同款改为自由函数。
pub fn require_extra<'a>(
    provider: &'a SttProviderConfig,
    key: &str,
    label: &str,
) -> Result<&'a str, super::SttError> {
    provider
        .extra
        .get(key)
        .filter(|v| !v.is_empty())
        .map(|s| s.as_str())
        .ok_or_else(|| {
            super::SttError::Other(format!(
                "{:?} provider requires `extra.{}` ({})",
                provider.kind, key, label
            ))
        })
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

    #[test]
    fn transcript_options_merge_request_overrides_over_defaults() {
        let defaults = TranscriptOptions {
            language: Some("zh-CN".into()),
            prompt: Some("product names".into()),
            punctuation: Some(true),
            diarization: Some(false),
            timestamps: Some(true),
            sample_rate_hz: Some(44_100),
        };
        let request = TranscriptOptions {
            language: Some(" en-US ".into()),
            prompt: Some("   ".into()),
            punctuation: Some(false),
            sample_rate_hz: Some(16_000),
            ..TranscriptOptions::default()
        };

        let merged = request.with_defaults(&defaults);
        assert_eq!(merged.language.as_deref(), Some("en-US"));
        assert_eq!(merged.prompt.as_deref(), Some("product names"));
        assert_eq!(merged.punctuation, Some(false));
        assert_eq!(merged.diarization, Some(false));
        assert_eq!(merged.timestamps, Some(true));
        assert_eq!(merged.sample_rate_hz, Some(16_000));
    }

    #[test]
    fn transcript_options_normalize_empty_strings() {
        let options = TranscriptOptions {
            language: Some("  ".into()),
            prompt: Some(" hint ".into()),
            ..TranscriptOptions::default()
        }
        .normalized();

        assert_eq!(options.language, None);
        assert_eq!(options.prompt.as_deref(), Some("hint"));
    }
}

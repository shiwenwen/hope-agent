//! ElevenLabs Scribe `/v1/speech-to-text` batch engine.
//!
//! Reference docs:
//! - <https://elevenlabs.io/docs/api-reference/speech-to-text/convert>
//! - <https://elevenlabs.io/docs/overview/models>
//!
//! Drives `SttProviderKind::ElevenlabsStt`. ElevenLabs is multipart like
//! OpenAI Whisper but NOT wire-compatible: the endpoint is
//! `/v1/speech-to-text`, the model field is `model_id` (default
//! `scribe_v2`), auth is the `xi-api-key` header (not Bearer), and the
//! response returns the transcript in `text` with per-word timing in
//! `words` (there is no OpenAI-style `segments` array). Realtime streaming
//! (`scribe_v2_realtime` over WebSocket) is a future enhancement — this
//! path is batch (record-then-transcribe) only.

use std::time::Duration;

use crate::provider::{apply_proxy, AuthProfile};
use crate::security::ssrf::{check_url, SsrfPolicy};

use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{
    AudioPayload, SttModelConfig, SttProviderConfig, SttProviderKind, Transcript, TranscriptOptions,
};

use super::{classify_http_status, classify_reqwest_error, load_batch_audio};

/// Batch transcription can run long on multi-minute clips; 120s mirrors the
/// OpenAI batch engine's headroom while still surfacing a stuck request.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Build the `/v1/speech-to-text` URL, trimming a trailing slash so users
/// can configure either form of the base URL.
fn speech_to_text_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}/v1/speech-to-text", trimmed)
}

/// One-shot transcription via ElevenLabs Scribe `/v1/speech-to-text`.
pub async fn transcribe_batch(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    audio: AudioPayload,
    options: &TranscriptOptions,
) -> SttResult<Transcript> {
    debug_assert!(matches!(provider.kind, SttProviderKind::ElevenlabsStt));

    let base_url = provider.resolve_base_url(profile);
    let url = speech_to_text_url(base_url);

    let cfg = crate::config::cached_config();
    let policy = if provider.allow_private_network {
        SsrfPolicy::AllowPrivate
    } else {
        cfg.ssrf.default_policy
    };
    check_url(&url, policy, &cfg.ssrf.trusted_hosts)
        .await
        .map_err(|e| SttError::SsrfBlocked(e.to_string()))?;

    // Disable auto-redirect so a 3xx on the audio upload can't silently
    // bypass the SSRF check above (mirrors the OpenAI batch engine).
    let client = apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("hope-agent/stt")
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .map_err(|e| SttError::Network(format!("HTTP client build failed: {e}")))?;

    let form = build_multipart_form(model, audio, options).await?;

    let response = client
        .post(&url)
        .header("xi-api-key", profile.api_key.clone())
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await
        .map_err(|e| classify_reqwest_error(&e))?;

    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("<unknown>")
            .to_string();
        return Err(SttError::SsrfBlocked(format!(
            "ElevenLabs redirected ({status}) to {location}; redirects are disabled to prevent SSRF bypass"
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| SttError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(classify_http_status(status, &body));
    }

    parse_transcript(provider, model, &body)
}

async fn build_multipart_form(
    model: &SttModelConfig,
    audio: AudioPayload,
    options: &TranscriptOptions,
) -> SttResult<reqwest::multipart::Form> {
    let (bytes, mime_type, filename) = load_batch_audio(audio).await?;

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str(&mime_type)
        .map_err(|e| SttError::UnsupportedAudio(format!("MIME {mime_type:?} rejected: {e}")))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model_id", model.id.clone());

    if let Some(lang) = &options.language {
        if !lang.is_empty() {
            form = form.text("language_code", lang.clone());
        }
    }

    Ok(form)
}

fn parse_transcript(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    body: &str,
) -> SttResult<Transcript> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SttError::Other(format!("Invalid JSON from provider: {e}")))?;

    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SttError::Other("Provider response missing `text` field".to_string()))?
        .to_string();

    // ElevenLabs returns `language_code` (ISO 639) rather than OpenAI's
    // `language`. There is no `segments` array — timing is per-word in
    // `words`, which we don't surface as coarse segments.
    let language = value
        .get("language_code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(Transcript {
        text,
        language,
        duration_ms: None,
        segments: Vec::new(),
        provider_id: provider.id.clone(),
        model_id: model.id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_to_text_url_trims_trailing_slash() {
        assert_eq!(
            speech_to_text_url("https://api.elevenlabs.io/"),
            "https://api.elevenlabs.io/v1/speech-to-text"
        );
    }

    #[test]
    fn parse_transcript_extracts_text_and_language() {
        let provider = SttProviderConfig::new(
            "ElevenLabs",
            SttProviderKind::ElevenlabsStt,
            "https://api.elevenlabs.io",
        );
        let model = SttModelConfig::new("scribe_v2", "Scribe v2");
        let body = r#"{"language_code":"en","language_probability":0.99,"text":"hello world"}"#;
        let t = parse_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "hello world");
        assert_eq!(t.language.as_deref(), Some("en"));
        assert!(t.segments.is_empty());
    }

    #[test]
    fn parse_transcript_rejects_response_missing_text() {
        let provider = SttProviderConfig::new(
            "ElevenLabs",
            SttProviderKind::ElevenlabsStt,
            "https://api.elevenlabs.io",
        );
        let model = SttModelConfig::new("scribe_v2", "Scribe v2");
        let err = parse_transcript(&provider, &model, r#"{"detail":"oops"}"#).unwrap_err();
        assert_eq!(err.code(), "other");
    }
}

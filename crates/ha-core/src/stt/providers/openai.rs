//! OpenAI `/v1/audio/transcriptions` batch engine.
//!
//! Drives both `SttProviderKind::OpenaiTranscriptions` (api.openai.com) and
//! `SttProviderKind::OpenaiCompatible` (Groq, whisper.cpp server,
//! faster-whisper-server, FunASR + OpenAI wrapper, sherpa-onnx server,
//! DashScope compatible mode, StepFun, SiliconFlow) — they share an
//! identical wire shape, only `base_url` and auth differ. Streaming
//! transcripts (`gpt-4o-transcribe` SSE) are Phase 2.

use std::time::Duration;

use crate::provider::{apply_proxy, AuthProfile};
use crate::security::ssrf::{check_url, SsrfPolicy};

use crate::stt::errors::{SttError, SttResult};
use crate::stt::types::{
    AudioPayload, SttModelConfig, SttProviderConfig, SttProviderKind, Transcript,
    TranscriptOptions, TranscriptSegment, MAX_BATCH_AUDIO_BYTES,
};

/// HTTP request timeout for one-shot batch transcription. Whisper requests
/// commonly take 5-30s depending on audio length / model size; 120s gives
/// plenty of headroom while still surfacing a stuck request to the user.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Build the `/v1/audio/transcriptions` URL given the provider's base URL.
/// Trim a trailing slash so users can configure either form.
fn transcriptions_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}/v1/audio/transcriptions", trimmed)
}

/// One-shot transcription via OpenAI-compatible `/v1/audio/transcriptions`.
///
/// Validates the destination URL through the global SSRF policy. For
/// providers with `allow_private_network = true` (the four local backends),
/// the policy is widened to `AllowPrivate`. Public providers still go
/// through `cfg.ssrf.default_policy`.
pub async fn transcribe_batch(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    audio: AudioPayload,
    options: &TranscriptOptions,
) -> SttResult<Transcript> {
    debug_assert!(matches!(
        provider.kind,
        SttProviderKind::OpenaiTranscriptions | SttProviderKind::OpenaiCompatible
    ));

    let base_url = provider.resolve_base_url(profile);
    let url = transcriptions_url(base_url);

    let cfg = crate::config::cached_config();
    let policy = if provider.allow_private_network {
        SsrfPolicy::AllowPrivate
    } else {
        cfg.ssrf.default_policy
    };
    check_url(&url, policy, &cfg.ssrf.trusted_hosts)
        .await
        .map_err(|e| SttError::SsrfBlocked(e.to_string()))?;

    // Disable auto-redirect: the SSRF check above only validated the
    // initial URL. With default reqwest behavior a public STT endpoint
    // could 3xx the multipart audio upload to an internal or metadata
    // address. We surface any 3xx as an explicit SSRF block instead of
    // silently following.
    let client = apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("hope-agent/stt")
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .map_err(|e| SttError::Network(format!("HTTP client build failed: {e}")))?;

    let form = build_multipart_form(model, audio, options).await?;

    let mut request = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", profile.api_key))
        .multipart(form);
    // Some OpenAI-compatible servers (DashScope) accept the optional
    // `X-DashScope-SSE: disable` hint — but we send nothing extra here; the
    // default response is non-streaming JSON which is what we want.
    if matches!(provider.kind, SttProviderKind::OpenaiCompatible) {
        request = request.header("Accept", "application/json");
    }

    let response = request
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;
    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("<unknown>")
            .to_string();
        return Err(SttError::SsrfBlocked(format!(
            "STT provider redirected ({status}) to {location}; redirects are disabled to prevent SSRF bypass"
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
    let (bytes, mime_type, filename) = match audio {
        AudioPayload::Bytes {
            bytes,
            mime_type,
            filename,
        } => {
            if bytes.len() > MAX_BATCH_AUDIO_BYTES {
                return Err(SttError::UnsupportedAudio(format!(
                    "Audio payload {} bytes exceeds {} MiB batch limit",
                    bytes.len(),
                    MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
                )));
            }
            (bytes, mime_type, filename)
        }
        AudioPayload::File { path, mime_type } => {
            // IM auto-transcribe and skill paths construct `File` payloads
            // from inbound media that can easily exceed the 25 MiB Whisper
            // limit (long voice notes, podcasts forwarded as audio
            // attachments). Stat first so we never alloc the giant Vec.
            let meta = tokio::fs::metadata(&path).await?;
            if meta.len() > MAX_BATCH_AUDIO_BYTES as u64 {
                return Err(SttError::UnsupportedAudio(format!(
                    "Audio file {} ({} bytes) exceeds {} MiB batch limit",
                    path.display(),
                    meta.len(),
                    MAX_BATCH_AUDIO_BYTES / (1024 * 1024)
                )));
            }
            let bytes = tokio::fs::read(&path).await?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.bin")
                .to_string();
            (bytes, mime_type, filename)
        }
    };

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str(&mime_type)
        .map_err(|e| SttError::UnsupportedAudio(format!("MIME {mime_type:?} rejected: {e}")))?;

    let mut form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.id.clone())
        .text("response_format", "verbose_json");

    if let Some(lang) = &options.language {
        if !lang.is_empty() {
            form = form.text("language", lang.clone());
        }
    }
    if let Some(prompt) = &options.prompt {
        if !prompt.is_empty() {
            form = form.text("prompt", prompt.clone());
        }
    }
    if options.timestamps.unwrap_or(model.supports_timestamps) {
        form = form.text("timestamp_granularities[]", "segment");
    }

    Ok(form)
}

fn parse_transcript(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    body: &str,
) -> SttResult<Transcript> {
    // Accept either the verbose_json shape (`{ "text": ..., "segments": [...] }`)
    // or the plain `{"text": "..."}` minimalist shape — some compatible
    // servers ignore `response_format=verbose_json`.
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SttError::Other(format!("Invalid JSON from provider: {e}")))?;

    let text = value
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| SttError::Other("Provider response missing `text` field".to_string()))?
        .to_string();

    let language = value
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let duration_ms = value
        .get("duration")
        .and_then(|v| v.as_f64())
        .map(|d| (d * 1000.0).round() as u64);

    let segments = value
        .get("segments")
        .and_then(|v| v.as_array())
        .map(|segs| segs.iter().filter_map(parse_segment).collect::<Vec<_>>())
        .unwrap_or_default();

    Ok(Transcript {
        text,
        language,
        duration_ms,
        segments,
        provider_id: provider.id.clone(),
        model_id: model.id.clone(),
    })
}

fn parse_segment(value: &serde_json::Value) -> Option<TranscriptSegment> {
    let text = value.get("text").and_then(|v| v.as_str())?.to_string();
    let start = value.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let end = value.get("end").and_then(|v| v.as_f64()).unwrap_or(start);
    let confidence = value
        .get("avg_logprob")
        .and_then(|v| v.as_f64())
        .map(|lp| lp.exp() as f32);
    let speaker = value
        .get("speaker")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(TranscriptSegment {
        text,
        start_ms: (start * 1000.0).max(0.0) as u64,
        end_ms: (end * 1000.0).max(0.0) as u64,
        confidence,
        speaker,
    })
}

fn classify_request_error(e: &reqwest::Error) -> SttError {
    if e.is_timeout() {
        SttError::Network(format!("Request timed out: {e}"))
    } else if e.is_connect() {
        SttError::Network(format!("Connect failed: {e}"))
    } else {
        SttError::Network(e.to_string())
    }
}

fn classify_http_status(status: reqwest::StatusCode, body: &str) -> SttError {
    // Truncate provider error body to keep API keys / request payloads out
    // of logs even on the unlikely path where the provider echoes them.
    let snippet = body.chars().take(256).collect::<String>();
    match status.as_u16() {
        401 | 403 => SttError::Auth(format!("HTTP {status}: {snippet}")),
        413 => SttError::UnsupportedAudio(format!("HTTP {status}: payload too large")),
        415 => SttError::UnsupportedAudio(format!("HTTP {status}: unsupported codec")),
        429 => SttError::RateLimit(format!("HTTP {status}: {snippet}")),
        500..=599 => SttError::ProviderUnavailable(format!("HTTP {status}: {snippet}")),
        _ => SttError::Other(format!("HTTP {status}: {snippet}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcriptions_url_trims_trailing_slash() {
        assert_eq!(
            transcriptions_url("https://api.openai.com/"),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(
            transcriptions_url("http://127.0.0.1:10097"),
            "http://127.0.0.1:10097/v1/audio/transcriptions"
        );
    }

    #[test]
    fn parse_transcript_accepts_minimal_text_shape() {
        let provider = SttProviderConfig::new(
            "Local",
            SttProviderKind::OpenaiCompatible,
            "http://127.0.0.1:8080",
        );
        let model = SttModelConfig::new("whisper-1", "Whisper");
        let body = r#"{"text":"hello world"}"#;
        let t = parse_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "hello world");
        assert!(t.segments.is_empty());
    }

    #[test]
    fn parse_transcript_extracts_segments_and_duration() {
        let provider = SttProviderConfig::new(
            "OpenAI",
            SttProviderKind::OpenaiTranscriptions,
            "https://api.openai.com",
        );
        let model = SttModelConfig::new("whisper-1", "Whisper");
        let body = r#"{
            "text": "hello world",
            "language": "en",
            "duration": 2.5,
            "segments": [
                {"text": "hello", "start": 0.0, "end": 1.0, "avg_logprob": -0.2},
                {"text": " world", "start": 1.0, "end": 2.5, "avg_logprob": -0.4}
            ]
        }"#;
        let t = parse_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "hello world");
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.duration_ms, Some(2500));
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[1].start_ms, 1000);
        assert_eq!(t.segments[1].end_ms, 2500);
        assert!(t.segments[0].confidence.unwrap() > 0.0);
    }

    #[test]
    fn parse_transcript_rejects_response_missing_text() {
        let provider = SttProviderConfig::new(
            "OpenAI",
            SttProviderKind::OpenaiTranscriptions,
            "https://api.openai.com",
        );
        let model = SttModelConfig::new("whisper-1", "Whisper");
        let err = parse_transcript(&provider, &model, r#"{"error":"oops"}"#).unwrap_err();
        assert_eq!(err.code(), "other");
    }

    #[test]
    fn classify_http_status_maps_to_error_kinds() {
        assert_eq!(
            classify_http_status(reqwest::StatusCode::UNAUTHORIZED, "bad key").code(),
            "auth"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::PAYLOAD_TOO_LARGE, "").code(),
            "unsupported_audio"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::TOO_MANY_REQUESTS, "").code(),
            "rate_limit"
        );
        assert_eq!(
            classify_http_status(reqwest::StatusCode::BAD_GATEWAY, "").code(),
            "provider_unavailable"
        );
    }

    #[test]
    fn classify_http_status_truncates_long_bodies() {
        let body = "a".repeat(2_000);
        let err =
            classify_http_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR, &body).to_string();
        assert!(err.len() < 400);
    }
}

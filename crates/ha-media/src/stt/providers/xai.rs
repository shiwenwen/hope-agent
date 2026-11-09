//! xAI Grok STT `/v1/stt` batch engine.
//!
//! Reference docs:
//! - <https://docs.x.ai/developers/model-capabilities/audio/speech-to-text>
//!
//! Drives `SttProviderKind::XaiStt`. xAI ships a standalone STT API (model
//! `grok-voice-transcribe-2.0`) on its own `/v1/stt` path — multipart upload with a `model`
//! field and Bearer auth, but NOT the OpenAI `/v1/audio/transcriptions`
//! wire (different path + response shape), so it needs a dedicated engine.
//! Realtime streaming (`wss://api.x.ai/v1/stt`) is a future enhancement —
//! this path is batch (record-then-transcribe) only.

use std::time::Duration;

use ha_core::provider::{apply_proxy, AuthProfile};
use ha_core::security::ssrf::{check_url, SsrfPolicy};

use ha_core::stt::errors::{SttError, SttResult};
use ha_core::stt::types::{
    AudioPayload, SttModelConfig, SttProviderConfig, SttProviderKind, Transcript,
    TranscriptOptions, TranscriptSegment,
};

use super::{classify_http_status, classify_reqwest_error, load_batch_audio};

/// Batch transcription can run long on multi-minute clips; 120s mirrors the
/// OpenAI batch engine's headroom while still surfacing a stuck request.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Build the `/v1/stt` URL, trimming a trailing slash so users can
/// configure either form of the base URL.
fn stt_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    format!("{}/v1/stt", trimmed)
}

/// One-shot transcription via xAI Grok STT `/v1/stt`.
pub async fn transcribe_batch(
    provider: &SttProviderConfig,
    model: &SttModelConfig,
    profile: &AuthProfile,
    audio: AudioPayload,
    options: &TranscriptOptions,
) -> SttResult<Transcript> {
    debug_assert!(matches!(provider.kind, SttProviderKind::XaiStt));

    let base_url = provider.resolve_base_url(profile);
    let url = stt_url(base_url);

    let cfg = ha_core::config::cached_config();
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
        .header("Authorization", format!("Bearer {}", profile.api_key))
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
            "xAI redirected ({status}) to {location}; redirects are disabled to prevent SSRF bypass"
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

    // xAI's STT docs require the `file` part to be appended AFTER all other
    // multipart fields, so build `model` / `language` first and add the
    // audio part last.
    let mut form = reqwest::multipart::Form::new().text("model", model.id.clone());
    if let Some(lang) = &options.language {
        if !lang.is_empty() {
            form = form.text("language", lang.clone());
        }
    }
    form = form.part("file", part);

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

    let language = value
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let duration_ms = value
        .get("duration")
        .and_then(|v| v.as_f64())
        .map(|d| (d * 1000.0).round() as u64);

    // Best-effort: adopt an OpenAI-style `segments` array if present. When
    // xAI returns per-word timing instead, this stays empty.
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
    let speaker = value
        .get("speaker")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(TranscriptSegment {
        text,
        start_ms: (start * 1000.0).max(0.0) as u64,
        end_ms: (end * 1000.0).max(0.0) as u64,
        confidence: None,
        speaker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_and_saved_models_keep_batch_multipart_and_response_contract() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let _config =
            ha_core::test_support::replace_config_cache(ha_core::config::AppConfig::default());
        for id in ["grok-voice-transcribe-2.0", "saved-custom-model"] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let header_end = loop {
                    let mut buf = [0; 4096];
                    let len = socket.read(&mut buf).await.unwrap();
                    assert_ne!(len, 0);
                    raw.extend_from_slice(&buf[..len]);
                    if let Some(end) = raw.windows(4).position(|b| b == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8(raw[..header_end].to_vec())
                    .unwrap()
                    .to_ascii_lowercase();
                assert!(headers.starts_with("post /v1/stt http/1.1"));
                assert!(headers.contains("authorization: bearer synthetic-key\r\n"));
                let len: usize = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                while raw.len() < header_end + len {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert_ne!(n, 0);
                    raw.extend_from_slice(&buf[..n]);
                }
                let body = String::from_utf8_lossy(&raw[header_end..header_end + len]);
                assert!(body.contains(&format!("name=\"model\"\r\n\r\n{id}\r\n")));
                let language = body.find("name=\"language\"\r\n\r\nzh\r\n").unwrap();
                let file = body
                    .find("name=\"file\"; filename=\"synthetic.wav\"")
                    .unwrap();
                assert!(body.find("name=\"model\"").unwrap() < language && language < file);
                assert_eq!(body[file..].matches("Content-Disposition").count(), 0);
                let response = serde_json::json!({"text":"你好 world 123","language":"zh","duration":3.45,"words":[{"text":"你好","start":0.1,"end":0.5}]}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
            });
            let mut provider =
                SttProviderConfig::new("Synthetic xAI", SttProviderKind::XaiStt, &base);
            provider.allow_private_network = true;
            let model = SttModelConfig::new(id, id);
            let profile = AuthProfile::new("Synthetic".into(), "synthetic-key".into(), None);
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                transcribe_batch(
                    &provider,
                    &model,
                    &profile,
                    AudioPayload::Bytes {
                        mime_type: "audio/wav".into(),
                        bytes: b"synthetic-audio".to_vec(),
                        filename: "synthetic.wav".into(),
                    },
                    &TranscriptOptions {
                        language: Some("zh".into()),
                        ..Default::default()
                    },
                ),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.text, "你好 world 123");
            assert_eq!(result.language.as_deref(), Some("zh"));
            assert_eq!(result.duration_ms, Some(3450));
            assert_eq!(result.model_id, id);
            assert_eq!(result.provider_id, provider.id);
            // Word timestamps are not segment timestamps; don't claim new
            // alignment support from the catalog update.
            assert!(result.segments.is_empty());
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[test]
    fn stt_url_trims_trailing_slash() {
        assert_eq!(stt_url("https://api.x.ai/"), "https://api.x.ai/v1/stt");
    }

    #[test]
    fn parse_transcript_extracts_text_and_segments() {
        let provider = SttProviderConfig::new("xAI", SttProviderKind::XaiStt, "https://api.x.ai");
        let model = SttModelConfig::new("grok-stt", "Grok STT");
        let body = r#"{
            "text": "hello world",
            "language": "en",
            "duration": 2.5,
            "segments": [
                {"text": "hello", "start": 0.0, "end": 1.0},
                {"text": " world", "start": 1.0, "end": 2.5, "speaker": "A"}
            ]
        }"#;
        let t = parse_transcript(&provider, &model, body).unwrap();
        assert_eq!(t.text, "hello world");
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.duration_ms, Some(2500));
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[1].end_ms, 2500);
        assert_eq!(t.segments[1].speaker.as_deref(), Some("A"));
    }

    #[test]
    fn parse_transcript_rejects_response_missing_text() {
        let provider = SttProviderConfig::new("xAI", SttProviderKind::XaiStt, "https://api.x.ai");
        let model = SttModelConfig::new("grok-stt", "Grok STT");
        let err = parse_transcript(&provider, &model, r#"{"error":"oops"}"#).unwrap_err();
        assert_eq!(err.code(), "other");
    }
}

//! Doubao Speech (Volcengine) TTS — `POST /api/v3/tts/unidirectional`.
//!
//! This is a *different platform* from Volcengine Ark (the image side):
//! host `openspeech.bytedance.com`, and auth is header-based (see below)
//! rather than an `Authorization: Bearer` token.
//!
//! Two things drive the shape of this file:
//!
//! 1. **`X-Api-Resource-Id` must match the voice family**, or the server
//!    rejects the call with `55000000`. The catalog therefore stores the
//!    resource id *as the model id* (`seed-tts-2.0` / `seed-tts-1.0` /
//!    `seed-icl-2.0` for cloned voices), and we forward it verbatim.
//! 2. **The response is NDJSON, not audio bytes.** Each line carries one
//!    base64 chunk (`{"code":0,"data":"..."}`) and `{"code":20000000}`
//!    terminates the stream. Concatenating the decoded chunks in order is
//!    what produces the actual file, so the whole body is read as text and
//!    parsed by a pure function (`parse_ndjson_audio`) that can be tested
//!    without a network.
//!
//! Music / SFX live behind Volcengine's top-level OpenAPI with AK/SK request
//! signing — a different auth system entirely — so they are rejected here
//! instead of being coerced onto the TTS endpoint.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://openspeech.bytedance.com";
const TTS_PATH: &str = "/api/v3/tts/unidirectional";

/// Used when the provider config leaves the model blank. 2.0 official voices
/// are the catalog default, so this is the resource id least likely to trip
/// the voice/resource mismatch error.
const DEFAULT_RESOURCE_ID: &str = "seed-tts-2.0";

/// mp3 keeps the result embeddable as a data-uri, matching every other audio
/// adapter; 24 kHz is the sample rate the vendor documents for this endpoint.
const AUDIO_FORMAT: &str = "mp3";
const SAMPLE_RATE: u32 = 24_000;
const OUTPUT_MIME: &str = "audio/mpeg";

/// Terminator code on the NDJSON stream — success, not an error.
const CODE_STREAM_END: i64 = 20_000_000;

/// `uid` is required by the request envelope but is only a caller-side
/// grouping key; a fixed value keeps it stable without leaking anything.
const CLIENT_UID: &str = "hope-agent";

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

#[derive(Deserialize)]
struct TtsLine {
    /// Absent on some informational lines; those carry audio and are
    /// equivalent to `code: 0`.
    code: Option<i64>,
    data: Option<String>,
    message: Option<String>,
}

// ── Pure helpers (unit-tested) ────────────────────────────────────

fn resolve_resource_id(model: &str) -> &str {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        DEFAULT_RESOURCE_ID
    } else {
        trimmed
    }
}

fn build_request_body(text: &str, speaker: &str) -> serde_json::Value {
    // `req_params.additions` is deliberately omitted: its value must be a
    // *JSON-serialized string* rather than a nested object, and no knob we
    // expose needs it. Sending it wrong is a server-side parse error.
    serde_json::json!({
        "user": { "uid": CLIENT_UID },
        "req_params": {
            "text": text,
            "speaker": speaker,
            "audio_params": {
                "format": AUDIO_FORMAT,
                "sample_rate": SAMPLE_RATE,
            }
        }
    })
}

/// Concatenate the base64 audio chunks of an NDJSON TTS stream in order.
///
/// A non-zero, non-terminator `code` is a server error even though the HTTP
/// status was 200 — surfacing it here is the only way the caller ever sees
/// the vendor's own diagnostics.
/// The console migrated auth styles and both are live: the current one is a
/// single `X-Api-Key`, the legacy one is an App-Id / Access-Key pair. A key
/// entered as `app_id:access_key` selects the legacy triple — otherwise the
/// legacy accounts would have no working configuration at all.
fn auth_headers(api_key: &str) -> Vec<(&'static str, String)> {
    match api_key.split_once(':') {
        Some((app_id, access_key)) if !app_id.is_empty() && !access_key.is_empty() => vec![
            ("X-Api-App-Id", app_id.to_string()),
            ("X-Api-Access-Key", access_key.to_string()),
        ],
        _ => vec![("X-Api-Key", api_key.to_string())],
    }
}

fn parse_ndjson_audio(body: &str) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut saw_terminator = false;

    for (idx, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed: TtsLine = serde_json::from_str(line).map_err(|e| {
            anyhow::anyhow!(
                "Doubao Speech returned an unparseable stream line {} ({}): {}",
                idx + 1,
                e,
                crate::truncate_utf8(line, 300)
            )
        })?;

        let code = parsed.code.unwrap_or(0);
        if code != 0 && code != CODE_STREAM_END {
            bail!(
                "Doubao Speech API error ({}): {}",
                code,
                parsed.message.as_deref().unwrap_or("no message")
            );
        }

        // The terminator may still carry a final chunk, so decode before
        // deciding the stream is over.
        if let Some(chunk) = parsed.data.as_deref().filter(|c| !c.is_empty()) {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(chunk)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Doubao Speech audio chunk on line {} is not valid base64: {}",
                        idx + 1,
                        e
                    )
                })?;
            out.extend_from_slice(&decoded);
        }

        if code == CODE_STREAM_END {
            saw_terminator = true;
            break;
        }
    }

    if out.is_empty() {
        bail!("Doubao Speech returned no audio data");
    }
    if !saw_terminator {
        // The terminator is the only completion signal this protocol has: a
        // chunked transfer cut short still looks like HTTP 200, so accepting
        // the partial buffer would hand back a clipped file that reads as a
        // success. TTS is cheap and idempotent — failing is the better trade.
        bail!(
            "Doubao Speech stream ended without a terminator after {} bytes; the audio would be truncated",
            out.len()
        );
    }

    Ok(out)
}

// ── Wire ──────────────────────────────────────────────────────────

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    match params.kind {
        AudioKind::Speech => {}
        AudioKind::Music | AudioKind::Sfx => {
            bail!(
                "Doubao Speech only provides text-to-speech; Volcengine's music/sound-effect \
                 models sit behind a separate AK/SK-signed OpenAPI. Pick another provider for {}.",
                params.kind.as_str()
            )
        }
    }

    let speaker = params
        .voice
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Doubao Speech requires a voice id (Settings → Media Generation Models → \
                 Doubao Speech → default voice, or pass one per call)"
            )
        })?;

    let resource_id = resolve_resource_id(params.model);
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}{}", base, TTS_PATH);
    let request_id = uuid::Uuid::new_v4().to_string();
    let body = build_request_body(params.prompt, speaker);

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "tool",
            "media_gen::audio::volcengine::request",
            &format!(
                "Doubao Speech request: resource_id={}, speaker={}, url={}",
                resource_id, speaker, url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "resource_id": resource_id,
                    "speaker": speaker,
                    "prompt_length": params.prompt.chars().count(),
                    "request_id": &request_id,
                    "timeout_secs": params.timeout_secs,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    // SSRF 红线：出站前必过 check_url（base_url 由用户配置，可能指向内网）。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;

    let mut req = client
        .post(&url)
        .header("X-Api-Resource-Id", resource_id)
        .header("X-Api-Request-Id", &request_id)
        .header("Content-Type", "application/json");
    for (name, value) in auth_headers(params.api_key) {
        req = req.header(name, value);
    }
    let resp = req.json(&body).send().await?;

    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "media_gen::audio::volcengine::error",
                &format!(
                    "Doubao Speech failed ({}): {}",
                    status.as_u16(),
                    crate::truncate_utf8(&err, 300)
                ),
                Some(
                    serde_json::json!({
                        "status": status.as_u16(),
                        "resource_id": resource_id,
                        "request_id": &request_id,
                    })
                    .to_string(),
                ),
                None,
                None,
            );
        }
        bail!(
            "Doubao Speech failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }

    // Buffer the whole stream: TTS payloads are small, and reading to text
    // first lets an HTTP-200 error envelope report the vendor's own message.
    let raw = resp.text().await?;
    let data = parse_ndjson_audio(&raw)?;

    app_info!(
        "tool",
        "media_gen::audio::volcengine",
        "Doubao Speech produced {} bytes (resource_id={}, request_id={})",
        data.len(),
        resource_id,
        request_id
    );

    Ok(AudioGenResult {
        data,
        mime: OUTPUT_MIME.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_id_falls_back_only_when_blank() {
        assert_eq!(resolve_resource_id("seed-icl-2.0"), "seed-icl-2.0");
        assert_eq!(resolve_resource_id("  seed-tts-1.0 "), "seed-tts-1.0");
        assert_eq!(resolve_resource_id(""), DEFAULT_RESOURCE_ID);
        assert_eq!(resolve_resource_id("   "), DEFAULT_RESOURCE_ID);
    }

    #[test]
    fn request_body_uses_nested_req_params_and_omits_additions() {
        let body = build_request_body("你好", "zh_female_test");
        assert_eq!(body["user"]["uid"], CLIENT_UID);
        assert_eq!(body["req_params"]["text"], "你好");
        assert_eq!(body["req_params"]["speaker"], "zh_female_test");
        assert_eq!(body["req_params"]["audio_params"]["format"], "mp3");
        assert_eq!(body["req_params"]["audio_params"]["sample_rate"], 24000);
        // `additions` must be a JSON *string* if present; we never send it.
        assert!(body["req_params"].get("additions").is_none());
    }

    #[test]
    fn ndjson_chunks_concatenate_in_order() {
        // base64("ID3") + base64("\x01\x02") then the terminator line.
        let body = "{\"code\":0,\"data\":\"SUQz\"}\n\
                    {\"code\":0,\"data\":\"AQI=\"}\n\
                    \n\
                    {\"code\":20000000}\n";
        assert_eq!(
            parse_ndjson_audio(body).unwrap(),
            vec![b'I', b'D', b'3', 0x01, 0x02]
        );
    }

    #[test]
    fn ndjson_surfaces_vendor_error_codes() {
        let body = "{\"code\":55000000,\"message\":\"resource id mismatch\"}\n";
        let err = parse_ndjson_audio(body).unwrap_err().to_string();
        assert!(err.contains("55000000"), "{err}");
        assert!(err.contains("resource id mismatch"), "{err}");

        // An error arriving after partial audio must still fail loudly rather
        // than handing back a half-written file.
        let partial = "{\"code\":0,\"data\":\"SUQz\"}\n{\"code\":45000001,\"message\":\"bad\"}\n";
        assert!(parse_ndjson_audio(partial).is_err());
    }

    #[test]
    fn ndjson_rejects_empty_and_malformed_streams() {
        assert!(parse_ndjson_audio("").is_err());
        assert!(parse_ndjson_audio("{\"code\":20000000}\n").is_err());
        assert!(parse_ndjson_audio("not json\n").is_err());
        assert!(parse_ndjson_audio("{\"code\":0,\"data\":\"!!!not base64!!!\"}\n").is_err());
    }

    #[test]
    fn ndjson_tolerates_missing_code_and_terminator_payload() {
        // Missing `code` means "audio line"; the terminator may carry a last chunk.
        let body = "{\"data\":\"SUQz\"}\n{\"code\":20000000,\"data\":\"AQ==\"}\n";
        assert_eq!(
            parse_ndjson_audio(body).unwrap(),
            vec![b'I', b'D', b'3', 0x01]
        );
    }
}

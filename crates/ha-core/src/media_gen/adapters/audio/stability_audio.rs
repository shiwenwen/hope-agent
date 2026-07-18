//! Stability AI Stable Audio — music and sound effects from a text prompt.
//!
//! Three vendor traps shape this file:
//! 1. **The URL path segment and the `model` form value are deliberately
//!    different versions**: the endpoint is `.../stable-audio-2/...` while
//!    the multipart body must carry `stable-audio-2.5`. Mirroring the path
//!    segment into `model` is a silent request rejection.
//! 2. **Generation is asynchronous** — the submit answers HTTP 202 with a
//!    bare `{"id": ...}` and the audio only materializes once
//!    `GET /v2beta/audio/results/{id}` flips to 200. Stability documents a
//!    hard floor of 10 seconds between polls, so this adapter never polls
//!    faster no matter how short the caller's timeout is.
//! 3. **Results are key-scoped**: polling with a different API key than the
//!    one that submitted returns 404, indistinguishable from expiry.
//!
//! Stability ships no TTS/voice product at all (their audio line is music +
//! SFX only), so `AudioKind::Speech` is rejected outright rather than
//! quietly coerced into music.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use base64::Engine;
use reqwest::Client;

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.stability.ai";
const SUBMIT_PATH: &str = "/v2beta/audio/stable-audio-2/text-to-audio";
const RESULT_PATH: &str = "/v2beta/audio/results";

/// Form-field value; intentionally *not* the `stable-audio-2` path segment.
const DEFAULT_MODEL: &str = "stable-audio-2.5";

const MIN_DURATION_SECS: f64 = 1.0;
const MAX_DURATION_SECS: f64 = 180.0;

/// Stability's docs require at least 10s between result polls. Undercutting
/// it risks a 429 that burns the (already billed) generation.
const MIN_POLL_INTERVAL_SECS: u64 = 10;
const MAX_POLL_INTERVAL_SECS: u64 = 30;
const POLL_INTERVAL_STEP_SECS: u64 = 5;

/// Floor for the per-request HTTP timeout: the submit round-trip must not
/// inherit a caller budget so small that it dies before the id comes back.
const MIN_REQUEST_TIMEOUT_SECS: u64 = 30;

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

// ── Pure helpers (unit-tested) ────────────────────────────────────

fn resolve_model(model: &str) -> &str {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        DEFAULT_MODEL
    } else {
        trimmed
    }
}

/// `None` (and non-finite garbage) means "let the vendor pick"; anything else
/// is clamped into Stable Audio's supported window.
fn clamp_duration(duration_seconds: Option<f64>) -> Option<u32> {
    duration_seconds
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(MIN_DURATION_SECS, MAX_DURATION_SECS).round() as u32)
}

fn resolve_output_format(extra: &HashMap<String, String>) -> (&'static str, &'static str) {
    match extra
        .get("output_format")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => ("wav", "audio/wav"),
        _ => ("mp3", "audio/mpeg"),
    }
}

/// Text parts of the multipart body. `steps` / `seed` ride along only when
/// the caller set them to something numeric — forwarding unparseable values
/// would turn a config typo into a server-side rejection.
fn build_form_fields(
    prompt: &str,
    model: &str,
    duration: Option<u32>,
    output_format: &str,
    extra: &HashMap<String, String>,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("prompt", prompt.to_string()),
        ("model", model.to_string()),
        ("output_format", output_format.to_string()),
    ];
    if let Some(secs) = duration {
        fields.push(("duration", secs.to_string()));
    }
    if let Some(steps) = extra
        .get("steps")
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        fields.push(("steps", steps.to_string()));
    }
    if let Some(seed) = extra.get("seed").and_then(|s| s.trim().parse::<u64>().ok()) {
        fields.push(("seed", seed.to_string()));
    }
    fields
}

fn next_poll_interval(current: u64) -> u64 {
    current
        .saturating_add(POLL_INTERVAL_STEP_SECS)
        .min(MAX_POLL_INTERVAL_SECS)
}

/// What the completion body handed us. The audio key is not first-hand
/// documented, so several reported shapes are accepted; a URL under any of
/// them is routed through `fetch_asset` rather than decoded.
#[derive(Debug, PartialEq)]
enum AudioPayload {
    Base64(String),
    Url(String),
}

/// Non-success terminal states arrive with HTTP 200, so this must be checked
/// as a field rather than inferred from the status code.
fn check_finish_reason(reason: Option<&str>) -> Result<()> {
    match reason.map(|r| r.trim().to_ascii_uppercase()).as_deref() {
        None | Some("") | Some("SUCCESS") => Ok(()),
        Some("CONTENT_FILTERED") => bail!(
            "Stable Audio content moderation rejected this generation \
             (finish_reason=CONTENT_FILTERED); adjust the prompt and retry"
        ),
        Some(other) => bail!("Stable Audio did not finish successfully (finish_reason={other})"),
    }
}

fn extract_audio_payload(body: &serde_json::Value) -> Option<AudioPayload> {
    // `audio` is the only first-hand documented key; the rest are reported
    // variants kept as a fallback. A bare `url` is deliberately NOT accepted:
    // error and metadata envelopes carry unrelated links, and treating one as
    // the result would download a web page and hand it back as audio.
    const KEYS: &[&str] = &["audio", "audio_base64", "audio_url", "result"];
    for key in KEYS {
        let Some(value) = body.get(*key).and_then(|v| v.as_str()) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        return Some(
            if value.starts_with("http://") || value.starts_with("https://") {
                AudioPayload::Url(value.to_string())
            } else {
                AudioPayload::Base64(value.to_string())
            },
        );
    }
    None
}

// ── Implementation ────────────────────────────────────────────────

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    if matches!(params.kind, AudioKind::Speech) {
        bail!("Stability AI has no text-to-speech capability; Stable Audio only generates music and sound effects");
    }

    let base = params
        .base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let submit_url = format!("{base}{SUBMIT_PATH}");
    let cfg = crate::config::cached_config();
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network
    crate::security::ssrf::check_url(&submit_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let model = resolve_model(params.model);
    let duration = clamp_duration(params.duration_seconds);
    let (output_format, mime) = resolve_output_format(params.extra);

    let request_timeout = params.timeout_secs.max(MIN_REQUEST_TIMEOUT_SECS);
    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(request_timeout)),
    )
    .build()?;

    let mut form = reqwest::multipart::Form::new();
    for (name, value) in
        build_form_fields(params.prompt, model, duration, output_format, params.extra)
    {
        form = form.text(name, value);
    }

    app_info!(
        "tool",
        "audio_generate",
        "Stable Audio submit: model={}, kind={}, duration={:?}, format={}",
        model,
        params.kind.as_str(),
        duration,
        output_format
    );

    let started = Instant::now();
    let resp = client
        .post(&submit_url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "Stable Audio submit failed ({}): {}",
            status.as_u16(),
            crate::truncate_utf8(&body, 300)
        );
    }

    let submit_body: serde_json::Value = resp.json().await?;
    let generation_id = submit_body
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Stable Audio submit returned no generation id: {}",
                crate::truncate_utf8(&submit_body.to_string(), 300)
            )
        })?;

    app_info!(
        "tool",
        "audio_generate",
        "Stable Audio generation queued: id={}",
        generation_id
    );

    let poll_url = format!("{base}{RESULT_PATH}/{generation_id}");
    crate::security::ssrf::check_url(&poll_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    // Poll budget starts *after* submit: the upload can consume most of the
    // caller's timeout (submit has its own 30s floor), and
    // anchoring on the pre-submit instant would abandon an already-billed
    // task without ever polling once.
    let deadline = Instant::now() + Duration::from_secs(params.timeout_secs);
    let mut interval = MIN_POLL_INTERVAL_SECS;

    loop {
        let now = Instant::now();
        if now >= deadline {
            bail!(
                "Stable Audio generation timed out after {}s (id={})",
                params.timeout_secs,
                generation_id
            );
        }
        // The 10s floor is a vendor requirement, not a nicety: undercutting it
        // risks a 429 that burns an already-billed generation. When the
        // remaining budget cannot cover a legal wait, stop rather than firing
        // an early poll.
        let wait = Duration::from_secs(interval);
        if deadline - now < wait {
            bail!(
                "Stable Audio generation timed out after {}s (id={}); \
                 remaining budget is below the {}s minimum poll interval",
                params.timeout_secs,
                generation_id,
                MIN_POLL_INTERVAL_SECS
            );
        }
        tokio::time::sleep(wait).await;

        let poll = client
            .get(&poll_url)
            .header("Authorization", format!("Bearer {}", params.api_key))
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = poll.status();
        if status == reqwest::StatusCode::ACCEPTED {
            interval = next_poll_interval(interval);
            continue;
        }
        // The generation is already billed; a rate-limit on the *read* side
        // must not throw it away. Back off and keep polling until the
        // caller's deadline decides.
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            app_warn!(
                "tool",
                "audio_generate",
                "Stable Audio poll rate-limited (id={}), backing off",
                generation_id
            );
            interval = next_poll_interval(interval);
            continue;
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            bail!(
                "Stable Audio result not found (id={}): results expire after 24h and must be polled with the same API key that submitted them",
                generation_id
            );
        }
        if !status.is_success() {
            let body = poll.text().await.unwrap_or_default();
            bail!(
                "Stable Audio poll failed ({}, id={}): {}",
                status.as_u16(),
                generation_id,
                crate::truncate_utf8(&body, 300)
            );
        }

        // A 200 carrying audio/* means the vendor ignored our JSON Accept and
        // streamed the bytes directly — take them as-is.
        let content_type = poll
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
            .unwrap_or_default();
        // Stability reports moderation as HTTP 200 + `finish_reason`, carried
        // in a response header when the body is raw audio. Mirrors
        // `image/stability.rs::check_finish_reason` — without this a filtered
        // generation is delivered to the user as a success.
        let finish_reason = poll
            .headers()
            .get("finish-reason")
            .or_else(|| poll.headers().get("finish_reason"))
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        check_finish_reason(finish_reason.as_deref())?;

        if content_type.starts_with("audio/") {
            let data = poll.bytes().await?.to_vec();
            return finish(data, content_type, &generation_id, started);
        }

        let body: serde_json::Value = poll.json().await?;
        check_finish_reason(body.get("finish_reason").and_then(|v| v.as_str()))?;
        let Some(payload) = extract_audio_payload(&body) else {
            bail!(
                "Stable Audio completed without audio (id={}): {}",
                generation_id,
                crate::truncate_utf8(&body.to_string(), 300)
            );
        };

        let data = match payload {
            AudioPayload::Base64(b64) => base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Stable Audio returned undecodable base64 (id={}): {e}",
                        generation_id
                    )
                })?,
            AudioPayload::Url(url) => {
                // Vendor-supplied URL, not a sub-path of the configured base —
                // fetch_asset re-gates it through SSRF.
                let (data, _) =
                    crate::media_gen::adapters::fetch::fetch_asset(&url, params.ssrf, mime).await?;
                data
            }
        };

        return finish(data, mime.to_string(), &generation_id, started);
    }
}

fn finish(
    data: Vec<u8>,
    mime: String,
    generation_id: &str,
    started: Instant,
) -> Result<AudioGenResult> {
    if data.is_empty() {
        bail!("Stable Audio returned empty audio (id={generation_id})");
    }
    app_info!(
        "tool",
        "audio_generate",
        "Stable Audio completed: id={}, {} bytes, {}ms",
        generation_id,
        data.len(),
        started.elapsed().as_millis() as u64
    );
    Ok(AudioGenResult { data, mime })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_defaults_to_the_form_value_not_the_path_segment() {
        assert_eq!(resolve_model(""), "stable-audio-2.5");
        assert_eq!(resolve_model("  "), "stable-audio-2.5");
        assert_eq!(resolve_model(" stable-audio-2.5 "), "stable-audio-2.5");
        assert_ne!(resolve_model(""), "stable-audio-2");
    }

    #[test]
    fn duration_clamps_into_the_supported_window() {
        assert_eq!(clamp_duration(None), None);
        assert_eq!(clamp_duration(Some(f64::NAN)), None);
        assert_eq!(clamp_duration(Some(0.0)), Some(1));
        assert_eq!(clamp_duration(Some(30.4)), Some(30));
        assert_eq!(clamp_duration(Some(1_000.0)), Some(180));
    }

    #[test]
    fn form_fields_carry_optional_knobs_only_when_numeric() {
        let mut extra = HashMap::new();
        extra.insert("steps".to_string(), "8".to_string());
        extra.insert("seed".to_string(), "not-a-number".to_string());

        let fields = build_form_fields(
            "rain on a tin roof",
            "stable-audio-2.5",
            Some(12),
            "mp3",
            &extra,
        );
        let get = |k: &str| {
            fields
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.as_str())
        };

        assert_eq!(get("prompt"), Some("rain on a tin roof"));
        assert_eq!(get("model"), Some("stable-audio-2.5"));
        assert_eq!(get("duration"), Some("12"));
        assert_eq!(get("output_format"), Some("mp3"));
        assert_eq!(get("steps"), Some("8"));
        assert_eq!(get("seed"), None);

        let bare = build_form_fields("x", "m", None, "wav", &HashMap::new());
        assert!(bare.iter().all(|(name, _)| *name != "duration"));
    }

    #[test]
    fn output_format_maps_to_mime() {
        let mut extra = HashMap::new();
        assert_eq!(resolve_output_format(&extra), ("mp3", "audio/mpeg"));
        extra.insert("output_format".to_string(), " WAV ".to_string());
        assert_eq!(resolve_output_format(&extra), ("wav", "audio/wav"));
        extra.insert("output_format".to_string(), "flac".to_string());
        assert_eq!(resolve_output_format(&extra), ("mp3", "audio/mpeg"));
    }

    #[test]
    fn poll_interval_never_dips_below_the_documented_floor() {
        let mut interval = MIN_POLL_INTERVAL_SECS;
        assert_eq!(interval, 10);
        interval = next_poll_interval(interval);
        assert_eq!(interval, 15);
        for _ in 0..20 {
            interval = next_poll_interval(interval);
            assert!((MIN_POLL_INTERVAL_SECS..=MAX_POLL_INTERVAL_SECS).contains(&interval));
        }
        assert_eq!(interval, MAX_POLL_INTERVAL_SECS);
    }

    #[test]
    fn moderation_is_rejected_despite_the_200() {
        assert!(check_finish_reason(None).is_ok());
        assert!(check_finish_reason(Some("SUCCESS")).is_ok());
        assert!(check_finish_reason(Some(" success ")).is_ok());

        let err = check_finish_reason(Some("CONTENT_FILTERED"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("CONTENT_FILTERED"), "{err}");

        // An unrecognized state must fail closed, not pass as success.
        assert!(check_finish_reason(Some("SOME_NEW_STATE")).is_err());
    }

    #[test]
    fn audio_payload_accepts_reported_key_shapes() {
        let b64 = serde_json::json!({ "audio": "QUJD" });
        assert_eq!(
            extract_audio_payload(&b64),
            Some(AudioPayload::Base64("QUJD".into()))
        );

        let alt = serde_json::json!({ "result": "QUJD" });
        assert_eq!(
            extract_audio_payload(&alt),
            Some(AudioPayload::Base64("QUJD".into()))
        );

        // A URL under any key must take the fetch_asset branch, never base64.
        let url = serde_json::json!({ "audio": "https://cdn.example/a.mp3" });
        assert_eq!(
            extract_audio_payload(&url),
            Some(AudioPayload::Url("https://cdn.example/a.mp3".into()))
        );

        assert_eq!(
            extract_audio_payload(&serde_json::json!({ "id": "x" })),
            None
        );
        // A bare `url` is deliberately not an audio key: error/metadata
        // envelopes carry unrelated links we must not download as audio.
        assert_eq!(
            extract_audio_payload(&serde_json::json!({ "url": "https://cdn.example/docs" })),
            None
        );
        assert_eq!(
            extract_audio_payload(&serde_json::json!({ "audio": "  " })),
            None
        );
    }
}

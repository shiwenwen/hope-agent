//! Cartesia Sonic TTS — speech via `POST /tts/bytes` (synchronous, returns
//! raw audio bytes). Speech only: Cartesia ships no music/SFX endpoint.
//!
//! The wire shape deliberately does *not* follow OpenAI: the text field is
//! `transcript` (not `input`), `voice` is an object `{mode, id}` (not a bare
//! string), and `output_format` is an object. Sending OpenAI-shaped keys here
//! is a silent 400.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;
use serde_json::{json, Value};

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.cartesia.ai";
/// Cartesia pins its API by date header rather than by URL path; omitting it
/// fails the request outright, so it is not optional.
const API_VERSION: &str = "2026-03-01";

const DEFAULT_SAMPLE_RATE: u32 = 44100;
/// Guard rail for a user-supplied `sample_rate`; well outside this band is a
/// typo rather than a format choice.
const MIN_SAMPLE_RATE: u32 = 8000;
const MAX_SAMPLE_RATE: u32 = 48000;

const SPEED_MIN: f64 = 0.6;
const SPEED_MAX: f64 = 1.5;
const VOLUME_MIN: f64 = 0.5;
const VOLUME_MAX: f64 = 2.0;

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    if params.kind != AudioKind::Speech {
        bail!(
            "Cartesia supports speech only (requested {}); pick a music/SFX-capable provider",
            params.kind.as_str()
        );
    }
    if params.prompt.trim().is_empty() {
        bail!("Cartesia TTS requires non-empty text");
    }
    // Sonic has no stock/public voice: every request must name a voice id, so
    // fail early with an actionable message instead of a server-side 4xx.
    let Some(voice) = params.voice.filter(|s| !s.trim().is_empty()) else {
        bail!("Cartesia TTS requires a voice id (see GET /voices); none configured for this model");
    };

    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/tts/bytes", base);
    let body = build_body(params.model, params.prompt, voice, params.extra);

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Cartesia-Version", API_VERSION)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        bail!(
            "Cartesia TTS failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("Cartesia TTS returned empty audio");
    }
    crate::app_info!(
        "tool",
        "audio",
        "Cartesia TTS produced {} bytes",
        data.len()
    );
    Ok(AudioGenResult {
        data,
        mime: "audio/mpeg".to_string(),
    })
}

fn build_body(
    model: &str,
    transcript: &str,
    voice: &str,
    extra: &HashMap<String, String>,
) -> Value {
    let mut body = json!({
        "model_id": model,
        "transcript": transcript,
        "voice": { "mode": "id", "id": voice.trim() },
        // mp3 is only available on /tts/bytes (SSE/WebSocket are raw-only),
        // which is exactly the endpoint this adapter uses.
        //
        // `encoding` is NOT a mirror of `container`: it names the PCM codec
        // (pcm_s16le / pcm_f32le / pcm_mulaw / pcm_alaw) and applies only to
        // the raw/wav containers. The mp3 container takes `bit_rate` instead,
        // and sending `encoding:"mp3"` fails validation on every request.
        "output_format": {
            "container": "mp3",
            "bit_rate": resolve_bit_rate(extra),
            "sample_rate": resolve_sample_rate(extra),
        },
    });
    if let Some(lang) = extra
        .get("language")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        body["language"] = json!(lang);
    }
    if let Some(cfg) = generation_config(extra) {
        body["generation_config"] = cfg;
    }
    body
}

/// Documented mp3 bit rates; anything else is rejected upstream.
const MP3_BIT_RATES: &[u32] = &[32000, 64000, 96000, 128000, 192000];
const DEFAULT_BIT_RATE: u32 = 128000;

fn resolve_bit_rate(extra: &HashMap<String, String>) -> u32 {
    extra
        .get("bit_rate")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|r| MP3_BIT_RATES.contains(r))
        .unwrap_or(DEFAULT_BIT_RATE)
}

fn resolve_sample_rate(extra: &HashMap<String, String>) -> u32 {
    extra
        .get("sample_rate")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|r| (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(r))
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}

/// `generation_config` is omitted entirely unless the user set a knob —
/// sending defaults would override server-side voice tuning for no reason.
fn generation_config(extra: &HashMap<String, String>) -> Option<Value> {
    let speed = clamped_knob(extra, "speed", SPEED_MIN, SPEED_MAX);
    let volume = clamped_knob(extra, "volume", VOLUME_MIN, VOLUME_MAX);
    if speed.is_none() && volume.is_none() {
        return None;
    }
    let mut cfg = serde_json::Map::new();
    if let Some(v) = speed {
        cfg.insert("speed".into(), json!(v));
    }
    if let Some(v) = volume {
        cfg.insert("volume".into(), json!(v));
    }
    Some(Value::Object(cfg))
}

fn clamped_knob(extra: &HashMap<String, String>, key: &str, min: f64, max: f64) -> Option<f64> {
    extra
        .get(key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn body_uses_cartesia_field_names() {
        let body = build_body("sonic-3", "hello", " voice-uuid ", &extra(&[]));
        assert_eq!(body["model_id"], "sonic-3");
        assert_eq!(body["transcript"], "hello");
        // voice is an object, not a string — the single most common mistake
        // when porting from an OpenAI-shaped adapter.
        assert_eq!(body["voice"]["mode"], "id");
        assert_eq!(body["voice"]["id"], "voice-uuid");
        assert_eq!(body["output_format"]["container"], "mp3");
        // `encoding` must be absent: it is the PCM codec field, not a
        // container mirror, and its presence 400s an mp3 request.
        assert!(body["output_format"].get("encoding").is_none());
        assert_eq!(body["output_format"]["bit_rate"], 128000);
        assert_eq!(body["output_format"]["sample_rate"], 44100);
        assert!(body.get("input").is_none());
        assert!(body.get("language").is_none());
        assert!(body.get("generation_config").is_none());
    }

    #[test]
    fn optional_knobs_are_clamped_and_passed_through() {
        let body = build_body(
            "sonic-3",
            "hi",
            "v1",
            &extra(&[
                ("language", "zh"),
                ("speed", "9"),
                ("volume", "0.1"),
                ("sample_rate", "24000"),
            ]),
        );
        assert_eq!(body["language"], "zh");
        assert_eq!(body["generation_config"]["speed"], SPEED_MAX);
        assert_eq!(body["generation_config"]["volume"], VOLUME_MIN);
        assert_eq!(body["output_format"]["sample_rate"], 24000);
    }

    #[test]
    fn garbage_knobs_fall_back_instead_of_erroring() {
        let e = extra(&[("speed", "fast"), ("volume", "NaN"), ("sample_rate", "1")]);
        assert_eq!(resolve_sample_rate(&e), DEFAULT_SAMPLE_RATE);
        assert!(generation_config(&e).is_none());
    }

    #[test]
    fn generation_config_holds_only_the_knobs_that_were_set() {
        let cfg = generation_config(&extra(&[("speed", "1.2")])).expect("speed set");
        assert_eq!(cfg["speed"], 1.2);
        assert!(cfg.get("volume").is_none());
    }
}

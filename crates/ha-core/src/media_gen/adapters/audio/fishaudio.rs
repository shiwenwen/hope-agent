//! Fish Audio TTS — `POST /v1/tts`, returning the audio byte stream directly.
//! Speech only; the vendor has no music/SFX endpoint.
//!
//! Two shape quirks to keep in mind before editing:
//! - the **model id travels in an HTTP header** (`model: s2.1-pro`), not in the
//!   JSON body like every other adapter here;
//! - the **voice is `reference_id`** — there is no `voice` field — and there is
//!   no top-level `speed`, only a nested `prosody` object.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;
use serde_json::{json, Value};

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.fish.audio";
/// Used when the caller leaves the model blank. `speech-1.5` / `speech-1.6`
/// were retired 2026-02-28 and must never be defaulted to.
const DEFAULT_MODEL: &str = "s2.1-pro";
const OUTPUT_FORMAT: &str = "mp3";
const MP3_BITRATE: u32 = 128;

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
    if !matches!(params.kind, AudioKind::Speech) {
        bail!(
            "Fish Audio only supports speech generation, not {}",
            params.kind.as_str()
        );
    }
    if params.prompt.trim().is_empty() {
        bail!("Fish Audio TTS requires non-empty text");
    }

    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/v1/tts", base);
    let model = resolve_model_header(params.model)?;
    let body = build_body(
        params.prompt,
        params.voice,
        prosody_from_extra(params.extra),
    );

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network
    // （默认仍 Strict 档兜底），self-hosted 部署才放行内网。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .header("model", model)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        bail!(
            "Fish Audio TTS failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    // Read the mime off the response before `bytes()` consumes it.
    let mime = mime_from_content_type(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    );
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("Fish Audio TTS returned empty audio");
    }
    crate::app_info!(
        "tool",
        "audio_generate",
        "Fish Audio TTS produced {} bytes ({})",
        data.len(),
        mime
    );
    Ok(AudioGenResult { data, mime })
}

/// The model id becomes a header value, so anything illegal there has to be
/// rejected up front — otherwise `reqwest` fails with an opaque builder error
/// that says nothing about which field was at fault.
fn resolve_model_header(model: &str) -> Result<&str> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(DEFAULT_MODEL);
    }
    if !model.bytes().all(|b| b.is_ascii_graphic()) {
        bail!(
            "Fish Audio model id must be a plain ASCII token, got {:?}",
            crate::truncate_utf8(model, 64)
        );
    }
    Ok(model)
}

/// `speed` / `volume` live nested under `prosody`, which the flat `extra`
/// passthrough other adapters use cannot express.
fn prosody_from_extra(extra: &HashMap<String, String>) -> Option<Value> {
    let mut prosody = serde_json::Map::new();
    for key in ["speed", "volume"] {
        if let Some(v) = extra
            .get(key)
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
        {
            prosody.insert(key.to_string(), json!(v));
        }
    }
    (!prosody.is_empty()).then(|| Value::Object(prosody))
}

fn build_body(text: &str, voice: Option<&str>, prosody: Option<Value>) -> Value {
    let mut body = json!({
        "text": text,
        "format": OUTPUT_FORMAT,
        "mp3_bitrate": MP3_BITRATE,
        "normalize": true,
        "latency": "normal",
    });
    // Omitted entirely when unset so the account's default voice applies;
    // sending an empty `reference_id` is rejected server-side.
    if let Some(v) = voice.map(str::trim).filter(|v| !v.is_empty()) {
        body["reference_id"] = json!(v);
    }
    if let Some(p) = prosody {
        body["prosody"] = p;
    }
    body
}

/// Content-Type tracks the requested `format`, but fall back to mp3 rather
/// than trusting a proxy that returns `application/octet-stream`.
fn mime_from_content_type(header: Option<&str>) -> String {
    header
        .map(|h| h.split(';').next().unwrap_or("").trim())
        .filter(|m| m.starts_with("audio/"))
        .unwrap_or("audio/mpeg")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_carries_voice_as_reference_id_and_omits_it_when_unset() {
        let with_voice = build_body("hello", Some("  abc123  "), None);
        assert_eq!(with_voice["reference_id"], json!("abc123"));
        assert_eq!(with_voice["text"], json!("hello"));
        assert_eq!(with_voice["format"], json!("mp3"));
        assert_eq!(with_voice["mp3_bitrate"], json!(128));
        assert_eq!(with_voice["normalize"], json!(true));
        assert_eq!(with_voice["latency"], json!("normal"));
        // No `voice` field exists in this API.
        assert!(with_voice.get("voice").is_none());

        for empty in [None, Some(""), Some("   ")] {
            assert!(build_body("hello", empty, None)
                .get("reference_id")
                .is_none());
        }
    }

    #[test]
    fn prosody_only_appears_when_extra_holds_parsable_numbers() {
        let mut extra = HashMap::new();
        assert!(prosody_from_extra(&extra).is_none());

        extra.insert("speed".into(), "not-a-number".into());
        assert!(prosody_from_extra(&extra).is_none());

        extra.insert("speed".into(), " 1.25 ".into());
        extra.insert("volume".into(), "-2".into());
        extra.insert("unrelated".into(), "9".into());
        let prosody = prosody_from_extra(&extra).expect("both knobs parse");
        assert_eq!(prosody["speed"], json!(1.25));
        assert_eq!(prosody["volume"], json!(-2.0));
        assert!(prosody.get("unrelated").is_none());

        let body = build_body("hi", None, prosody_from_extra(&extra));
        assert_eq!(body["prosody"]["speed"], json!(1.25));
        // There is no top-level speed on this API.
        assert!(body.get("speed").is_none());
    }

    #[test]
    fn model_header_defaults_when_blank_and_rejects_illegal_values() {
        assert_eq!(resolve_model_header("").unwrap(), DEFAULT_MODEL);
        assert_eq!(resolve_model_header("   ").unwrap(), DEFAULT_MODEL);
        assert_eq!(resolve_model_header(" s1 ").unwrap(), "s1");
        assert_eq!(
            resolve_model_header("s2.1-pro-free").unwrap(),
            "s2.1-pro-free"
        );
        // Header injection / non-ASCII must fail loudly, not at send time.
        assert!(resolve_model_header("s1\r\nX-Evil: 1").is_err());
        assert!(resolve_model_header("模型").is_err());
    }

    #[test]
    fn mime_falls_back_to_mpeg_for_non_audio_content_types() {
        assert_eq!(mime_from_content_type(Some("audio/mpeg")), "audio/mpeg");
        assert_eq!(
            mime_from_content_type(Some("audio/wav; charset=binary")),
            "audio/wav"
        );
        assert_eq!(
            mime_from_content_type(Some("application/octet-stream")),
            "audio/mpeg"
        );
        assert_eq!(mime_from_content_type(None), "audio/mpeg");
    }
}

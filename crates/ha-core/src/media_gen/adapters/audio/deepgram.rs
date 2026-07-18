//! Deepgram Aura TTS — `POST /v1/speak`. Speech only (no music / SFX wire).
//!
//! Two shape quirks drive this file:
//! * every synthesis knob travels in the **query string**; the JSON body is
//!   only `{"text": "..."}`;
//! * the auth scheme word is **`Token`**, not `Bearer` (a `Bearer` header is
//!   rejected outright).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
/// Fallback voice/model when the caller supplied neither.
const DEFAULT_MODEL: &str = "aura-2-thalia-en";
/// Hard server-side limit: longer input returns `413 Input Text Exceeds
/// Character Limits`.
const MAX_INPUT_CHARS: usize = 2000;

/// Query knobs we forward from provider/model `extra`. Deepgram silently
/// ignores unknown query params, so the allowlist exists to keep typos from
/// looking like working config. `model` is deliberately absent — it is
/// derived from voice/model below and must not be shadowed.
const PASSTHROUGH_QUERY_KEYS: &[&str] = &[
    "encoding",
    "sample_rate",
    "container",
    "bit_rate",
    "speed",
    "mip_opt_out",
    "tag",
];

pub(crate) struct Provider;

impl AudioGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

/// Deepgram has **no separate voice parameter — the voice *is* the model id**
/// (`aura-2-luna-en` is both). Our unified params carry `model` and `voice`
/// independently, so a user who typed a voice into the voice field must still
/// win — but only when it *is* a Deepgram voice.
///
/// The prefix guard matters because the executor resolves the voice once for
/// the whole failover chain: without it, a call naming an ElevenLabs or
/// OpenAI voice would arrive here as `?model=alloy` and 400, breaking the
/// very fallback that was supposed to rescue the call.
fn resolve_model<'a>(voice: Option<&'a str>, model: &'a str) -> &'a str {
    if let Some(v) = voice
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.starts_with("aura"))
    {
        return v;
    }
    let m = model.trim();
    if m.is_empty() {
        DEFAULT_MODEL
    } else {
        m
    }
}

/// Query string for `/v1/speak`. `encoding` defaults to mp3 so the returned
/// mime is predictable; anything the caller put in `extra` overrides it.
fn build_query(model: &str, extra: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut q = vec![("model".to_string(), model.to_string())];
    if !extra.contains_key("encoding") {
        q.push(("encoding".to_string(), "mp3".to_string()));
    }
    for key in PASSTHROUGH_QUERY_KEYS {
        if let Some(v) = extra.get(*key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            q.push(((*key).to_string(), v.to_string()));
        }
    }
    q
}

/// Deepgram counts characters, not bytes, against its 2000 limit. We reject
/// instead of truncating: silently dropping the tail of a script is worse
/// than a clear error the caller can chunk around.
fn check_input_len(text: &str) -> Result<()> {
    let n = text.chars().count();
    if n == 0 {
        bail!("Deepgram TTS requires non-empty text");
    }
    if n > MAX_INPUT_CHARS {
        bail!(
            "Deepgram TTS input is {} characters, over the {} limit — split the text into smaller chunks",
            n,
            MAX_INPUT_CHARS
        );
    }
    Ok(())
}

/// The response `Content-Type` is authoritative (the requested encoding and
/// container interact — mp3 inside a wav container is a legal combination),
/// so only fall back to the requested format when the header is missing.
fn resolve_mime(content_type: Option<&str>, extra: &HashMap<String, String>) -> String {
    if let Some(ct) = content_type {
        let ct = ct.split(';').next().unwrap_or("").trim();
        if ct.starts_with("audio/") {
            return ct.to_string();
        }
    }
    let container = extra
        .get("container")
        .map(|s| s.trim().to_ascii_lowercase());
    let encoding = extra.get("encoding").map(|s| s.trim().to_ascii_lowercase());
    match container.as_deref() {
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        _ => match encoding.as_deref().unwrap_or("mp3") {
            "mp3" => "audio/mpeg",
            "flac" => "audio/flac",
            "opus" => "audio/ogg",
            "aac" => "audio/aac",
            "linear16" | "mulaw" | "alaw" => "audio/wav",
            _ => "audio/mpeg",
        },
    }
    .to_string()
}

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    if params.kind != AudioKind::Speech {
        bail!(
            "Deepgram only supports speech generation, not {}",
            params.kind.as_str()
        );
    }
    check_input_len(params.prompt)?;

    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{}/v1/speak", base);
    let model = resolve_model(params.voice, params.model);
    let query = build_query(model, params.extra);

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network
    // （默认仍 Strict 档兜底），self-hosted 代理端点才放行内网。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;
    let resp = client
        .post(&url)
        .query(&query)
        // `Token`, not `Bearer` — Deepgram rejects the Bearer scheme.
        .header("Authorization", format!("Token {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "text": params.prompt }))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        bail!(
            "Deepgram TTS failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("Deepgram TTS returned empty audio");
    }
    let mime = resolve_mime(content_type.as_deref(), params.extra);
    crate::app_info!(
        "design",
        "audio",
        "Deepgram TTS ({}) produced {} bytes as {}",
        model,
        data.len(),
        mime
    );
    Ok(AudioGenResult { data, mime })
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
    fn voice_field_overrides_model_because_voice_is_the_model() {
        assert_eq!(
            resolve_model(Some("aura-2-luna-en"), "aura-2-thalia-en"),
            "aura-2-luna-en"
        );
        assert_eq!(resolve_model(None, "aura-2-thalia-en"), "aura-2-thalia-en");
        assert_eq!(
            resolve_model(Some("  "), "aura-asteria-en"),
            "aura-asteria-en"
        );
        assert_eq!(resolve_model(None, ""), DEFAULT_MODEL);
    }

    #[test]
    fn query_carries_model_plus_allowlisted_extras_only() {
        let q = build_query(
            "aura-2-thalia-en",
            &extra(&[("sample_rate", "24000"), ("bogus", "x")]),
        );
        assert!(q.contains(&("model".into(), "aura-2-thalia-en".into())));
        assert!(q.contains(&("encoding".into(), "mp3".into())));
        assert!(q.contains(&("sample_rate".into(), "24000".into())));
        assert!(q.iter().all(|(k, _)| k != "bogus"));

        // Caller-specified encoding replaces the default rather than duplicating it.
        let q = build_query("m", &extra(&[("encoding", "opus"), ("container", "ogg")]));
        let encodings: Vec<_> = q.iter().filter(|(k, _)| k == "encoding").collect();
        assert_eq!(encodings.len(), 1);
        assert_eq!(encodings[0].1, "opus");
    }

    #[test]
    fn input_over_two_thousand_chars_is_rejected_not_truncated() {
        assert!(check_input_len("hello").is_ok());
        assert!(check_input_len("").is_err());
        // Multi-byte: 2000 chars is fine even though it is 6000 bytes.
        assert!(check_input_len(&"字".repeat(MAX_INPUT_CHARS)).is_ok());
        assert!(check_input_len(&"字".repeat(MAX_INPUT_CHARS + 1)).is_err());
    }

    #[test]
    fn mime_prefers_response_header_then_requested_format() {
        assert_eq!(
            resolve_mime(Some("audio/wav"), &extra(&[("encoding", "mp3")])),
            "audio/wav"
        );
        assert_eq!(
            resolve_mime(Some("application/json"), &extra(&[])),
            "audio/mpeg"
        );
        assert_eq!(resolve_mime(None, &extra(&[])), "audio/mpeg");
        assert_eq!(
            resolve_mime(None, &extra(&[("encoding", "opus"), ("container", "ogg")])),
            "audio/ogg"
        );
        assert_eq!(
            resolve_mime(
                None,
                &extra(&[("encoding", "linear16"), ("container", "wav")])
            ),
            "audio/wav"
        );
    }
}

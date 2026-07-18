//! MiniMax audio provider — TTS via `POST /v1/t2a_v2`, music via
//! `POST /v1/music_generation`. Both are synchronous single round-trips.
//!
//! Two vendor quirks drive most of this file:
//! 1. **The audio comes back as a hex string inside JSON**, not as raw bytes
//!    and not as base64 — `data.audio` must be hex-decoded.
//! 2. **Errors arrive with HTTP 200** in a `base_resp.status_code != 0`
//!    envelope, so status-code-only error handling would silently succeed
//!    with no audio.
//!
//! MiniMax has no SFX endpoint (confirmed against api-overview + llms.txt),
//! so `AudioKind::Sfx` is rejected outright rather than coerced into music.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;
use serde::Deserialize;

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};
use crate::media_gen::AudioKind;

const DEFAULT_BASE_URL: &str = "https://api.minimax.io";

/// Documented payload ceilings (character counts, *not* bytes).
const MAX_TTS_TEXT_CHARS: usize = 10_000;
const MAX_MUSIC_PROMPT_CHARS: usize = 2_000;
const MAX_MUSIC_LYRICS_CHARS: usize = 3_500;

/// `audio_setting` values shared by both endpoints. mp3 keeps the result
/// directly embeddable as a data-uri like every other audio adapter.
const SAMPLE_RATE: u32 = 32_000;
const BITRATE: u32 = 128_000;
const AUDIO_FORMAT: &str = "mp3";
const OUTPUT_MIME: &str = "audio/mpeg";

/// `data.status` on the music endpoint: 1 = in progress, 2 = completed.
const MUSIC_STATUS_IN_PROGRESS: i32 = 1;

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
struct MiniMaxAudioResponse {
    data: Option<MiniMaxAudioData>,
    base_resp: Option<MiniMaxBaseResp>,
}

#[derive(Deserialize)]
struct MiniMaxAudioData {
    audio: Option<String>,
    status: Option<i32>,
}

#[derive(Deserialize)]
struct MiniMaxBaseResp {
    status_code: Option<i32>,
    status_msg: Option<String>,
}

// ── Pure request/response helpers (unit-tested) ───────────────────

/// Truncate to at most `max_chars` **characters**. The vendor limits are
/// expressed in characters, so `truncate_utf8` (byte-based) would cut CJK
/// text at roughly a third of the allowance.
fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

fn audio_setting() -> serde_json::Value {
    serde_json::json!({
        "sample_rate": SAMPLE_RATE,
        "bitrate": BITRATE,
        "format": AUDIO_FORMAT,
        "channel": 1,
    })
}

fn build_speech_body(model: &str, text: &str, voice: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "text": truncate_chars(text, MAX_TTS_TEXT_CHARS),
        "stream": false,
        "audio_setting": audio_setting(),
    });
    // `voice_id` is required by t2a_v2 — callers are gated on it before we
    // get here (see `generate_impl`), so a missing voice is a bug, not a
    // "let the server decide" case.
    if let Some(voice) = voice.filter(|v| !v.is_empty()) {
        body["voice_setting"] = serde_json::json!({
            "voice_id": voice,
            "speed": 1.0,
            "vol": 1.0,
            "pitch": 0,
        });
    }
    body
}

fn build_music_body(model: &str, prompt: &str, lyrics: Option<&str>) -> serde_json::Value {
    // `duration_seconds` is deliberately dropped: `/v1/music_generation` has
    // no duration/length parameter in any published form, and the vendor only
    // documents an "up to 5 minutes" ceiling. Inventing a field would 400.
    let mut body = serde_json::json!({
        "model": model,
        "prompt": truncate_chars(prompt, MAX_MUSIC_PROMPT_CHARS),
        "audio_setting": audio_setting(),
        "output_format": "hex",
    });
    if let Some(lyrics) = lyrics.filter(|l| !l.is_empty()) {
        body["lyrics"] = serde_json::json!(truncate_chars(lyrics, MAX_MUSIC_LYRICS_CHARS));
    }
    body
}

/// Decode the `data.audio` hex string. MiniMax emits lowercase hex; this
/// accepts either case but rejects anything else loudly — a silent partial
/// decode would surface as a corrupt audio file much later.
fn decode_hex_audio(s: &str) -> Result<Vec<u8>> {
    if s.is_empty() {
        bail!("MiniMax returned an empty audio payload");
    }
    if !s.len().is_multiple_of(2) {
        bail!(
            "MiniMax audio payload is not valid hex: odd length ({} chars)",
            s.len()
        );
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        let hi = hex_nibble(pair[0]);
        let lo = hex_nibble(pair[1]);
        match (hi, lo) {
            (Some(hi), Some(lo)) => out.push((hi << 4) | lo),
            _ => bail!(
                "MiniMax audio payload is not valid hex: bad byte at offset {} ({:?})",
                i * 2,
                String::from_utf8_lossy(pair)
            ),
        }
    }
    Ok(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ── Wire ──────────────────────────────────────────────────────────

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let (url, body) = match params.kind {
        AudioKind::Speech => {
            // t2a_v2 requires a voice identity (`voice_setting.voice_id`,
            // unless timbre weights are supplied). Failing here names the
            // missing input; shipping the body without it returns an opaque
            // invalid-params envelope instead. The catalog marks MiniMax
            // speech `needs_voice`, but nothing in the executor enforces it.
            if params.voice.is_none_or(|v| v.trim().is_empty()) {
                bail!(
                    "MiniMax speech requires a voice id (Settings → Media Generation Models → \
                     MiniMax → default voice, or pass one per call)"
                );
            }
            (
                format!("{}/v1/t2a_v2", base),
                build_speech_body(params.model, params.prompt, params.voice),
            )
        }
        AudioKind::Music => (
            format!("{}/v1/music_generation", base),
            // Lyrics have no slot in the unified params, so they ride the
            // vendor-knob channel.
            build_music_body(
                params.model,
                params.prompt,
                params.extra.get("lyrics").map(|s| s.as_str()),
            ),
        ),
        AudioKind::Sfx => {
            bail!("MiniMax has no sound-effect endpoint; use speech or music, or pick another provider for SFX")
        }
    };

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "tool",
            "media_gen::audio::minimax::request",
            &format!(
                "MiniMax {} request: model={}, url={}",
                params.kind.as_str(),
                params.model,
                url
            ),
            Some(
                serde_json::json!({
                    "api_url": &url,
                    "model": params.model,
                    "kind": params.kind.as_str(),
                    "prompt_length": params.prompt.chars().count(),
                    "has_voice": params.voice.is_some_and(|v| !v.is_empty()),
                    "timeout_secs": params.timeout_secs,
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network
    // （默认仍 Strict 档兜底），self-hosted 才放行内网。
    crate::security::ssrf::check_url(&url, params.ssrf, &[]).await?;
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                "media_gen::audio::minimax::error",
                &format!(
                    "MiniMax {} failed ({}): {}",
                    params.kind.as_str(),
                    status.as_u16(),
                    crate::truncate_utf8(&err, 300)
                ),
                Some(serde_json::json!({ "status": status.as_u16() }).to_string()),
                None,
                None,
            );
        }
        bail!(
            "MiniMax {} failed ({}): {}",
            params.kind.as_str(),
            status,
            crate::truncate_utf8(&err, 300)
        );
    }

    // Read as text first: an HTTP-200 error envelope that fails to fit the
    // response shape should still report the server's own message.
    let raw = resp.text().await?;
    let parsed: MiniMaxAudioResponse = serde_json::from_str(&raw).map_err(|e| {
        anyhow::anyhow!(
            "MiniMax {} returned unparseable response ({}): {}",
            params.kind.as_str(),
            e,
            crate::truncate_utf8(&raw, 300)
        )
    })?;

    if let Some(code) = parsed.base_resp.as_ref().and_then(|b| b.status_code) {
        if code != 0 {
            let msg = parsed
                .base_resp
                .as_ref()
                .and_then(|b| b.status_msg.as_deref())
                .unwrap_or("");
            bail!(
                "MiniMax {} API error ({}): {}",
                params.kind.as_str(),
                code,
                msg
            );
        }
    }

    let audio_hex = parsed
        .data
        .as_ref()
        .and_then(|d| d.audio.as_deref())
        .unwrap_or_default();
    if audio_hex.is_empty() {
        // The music endpoint can report "still generating" instead of
        // returning audio; there is no polling endpoint to follow up with,
        // so say so plainly rather than emitting a zero-byte file.
        if parsed.data.as_ref().and_then(|d| d.status) == Some(MUSIC_STATUS_IN_PROGRESS) {
            bail!("MiniMax {} is still generating (status=1) and returned no audio; retry the request", params.kind.as_str());
        }
        bail!("MiniMax {} returned no audio", params.kind.as_str());
    }

    let data = decode_hex_audio(audio_hex)?;

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "tool",
            "media_gen::audio::minimax::result",
            &format!(
                "MiniMax {} produced {} bytes",
                params.kind.as_str(),
                data.len()
            ),
            Some(
                serde_json::json!({
                    "kind": params.kind.as_str(),
                    "bytes": data.len(),
                })
                .to_string(),
            ),
            None,
            None,
        );
    }

    Ok(AudioGenResult {
        data,
        mime: OUTPUT_MIME.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_hex_and_rejects_malformed() {
        assert_eq!(
            decode_hex_audio("49443303").unwrap(),
            vec![0x49, 0x44, 0x33, 0x03]
        );
        // Mixed case is accepted; base64 input must not decode as hex.
        assert_eq!(decode_hex_audio("FfA0").unwrap(), vec![0xff, 0xa0]);
        assert!(decode_hex_audio("abc").is_err(), "odd length must fail");
        assert!(
            decode_hex_audio("SUQzAw==").is_err(),
            "base64 must not pass"
        );
        assert!(decode_hex_audio("").is_err());
    }

    #[test]
    fn truncates_by_characters_not_bytes() {
        // 5 CJK chars = 15 bytes: a byte-based cut would keep only 2 chars.
        assert_eq!(truncate_chars("你好世界啊", 4), "你好世界");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }

    #[test]
    fn speech_body_omits_voice_setting_when_unset() {
        // The omit branch stays reachable for defensive reasons, but
        // `generate_impl` rejects a voiceless speech call before it runs.
        let with = build_speech_body("speech-2.8-hd", "hi", Some("Deep_Voice_Man"));
        assert_eq!(with["voice_setting"]["voice_id"], "Deep_Voice_Man");
        assert_eq!(with["stream"], false);
        assert_eq!(with["audio_setting"]["format"], "mp3");

        let without = build_speech_body("speech-2.8-hd", "hi", Some(""));
        assert!(without.get("voice_setting").is_none());
        assert!(build_speech_body("speech-2.8-hd", "hi", None)
            .get("voice_setting")
            .is_none());
    }

    #[test]
    fn music_body_clamps_prompt_and_omits_empty_lyrics() {
        let long = "な".repeat(MAX_MUSIC_PROMPT_CHARS + 50);
        let body = build_music_body("music-3.0", &long, Some(""));
        assert_eq!(
            body["prompt"].as_str().unwrap().chars().count(),
            MAX_MUSIC_PROMPT_CHARS
        );
        assert!(body.get("lyrics").is_none());
        assert_eq!(body["output_format"], "hex");

        let with_lyrics = build_music_body("music-3.0", "jazz", Some("la la la"));
        assert_eq!(with_lyrics["lyrics"], "la la la");
    }
}

//! ElevenLabs audio provider — TTS via `POST /v1/text-to-speech/{voice_id}`,
//! music via `POST /v1/music`, and **SFX via its own `POST /v1/sound-generation`**
//! (B8-2：此前 SFX 错误复用音乐端点、质量劣化；改走专用音效端点 + 时长/prompt_influence)。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;

use super::types::{AudioGenParams, AudioGenProviderImpl, AudioGenResult, AudioKind};

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
const DEFAULT_TTS_MODEL: &str = "eleven_multilingual_v2";
const DEFAULT_MUSIC_MODEL: &str = "music_v1";
const DEFAULT_SFX_MODEL: &str = "eleven_text_to_sound_v2";
// A stock public ElevenLabs voice (Rachel) so speech works before a user picks one.
const DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM";
/// SFX 时长上限（ElevenLabs sound-generation 硬上限 30s）+ 默认。
const SFX_MIN_SECS: f64 = 0.5;
const SFX_MAX_SECS: f64 = 30.0;
const SFX_DEFAULT_SECS: f64 = 5.0;
/// Music 时长合法区间（ElevenLabs `/v1/music` 最短 ~10s，防 5s 桶值触 422）。
const MUSIC_MIN_SECS: f64 = 10.0;
const MUSIC_MAX_SECS: f64 = 300.0;
/// SFX prompt 上限（provider 侧 ~450）。
const SFX_MAX_PROMPT_CHARS: usize = 450;

pub(crate) struct ElevenLabsAudioProvider;

impl AudioGenProviderImpl for ElevenLabsAudioProvider {
    fn id(&self) -> &str {
        "elevenlabs"
    }
    fn display_name(&self) -> &str {
        "ElevenLabs"
    }
    fn default_model(&self, kind: AudioKind) -> &str {
        match kind {
            AudioKind::Speech => DEFAULT_TTS_MODEL,
            AudioKind::Music => DEFAULT_MUSIC_MODEL,
            AudioKind::Sfx => DEFAULT_SFX_MODEL,
        }
    }
    fn supports(&self, _kind: AudioKind) -> bool {
        true // speech + music + sfx
    }
    fn generate<'a>(
        &'a self,
        params: AudioGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AudioGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

async fn generate_impl(params: AudioGenParams<'_>) -> Result<AudioGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    let (url, body) = match params.kind {
        AudioKind::Speech => {
            let voice = params
                .entry
                .voice
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_VOICE);
            (
                format!("{}/v1/text-to-speech/{}", base, voice),
                serde_json::json!({ "text": params.prompt, "model_id": params.model }),
            )
        }
        AudioKind::Music => {
            // /v1/music：可选时长（毫秒）——**钳到合法区间**（防 5s 桶值 / NaN 触 422，review 修复）。
            let mut b = serde_json::json!({ "prompt": params.prompt, "model_id": params.model });
            if let Some(secs) = params
                .duration_seconds
                .filter(|s| s.is_finite() && *s > 0.0)
            {
                let secs = secs.clamp(MUSIC_MIN_SECS, MUSIC_MAX_SECS);
                b["music_length_ms"] = serde_json::json!((secs * 1000.0).round() as u64);
            }
            (format!("{}/v1/music", base), b)
        }
        AudioKind::Sfx => {
            // **专用音效端点**（非音乐端点）：duration_seconds 钳 0.5–30、prompt_influence
            // 0.7（用户明示音效建议偏高保真度）；prompt 截断到 provider 上限。NaN/非有限回退默认。
            let dur = params
                .duration_seconds
                .filter(|s| s.is_finite())
                .unwrap_or(SFX_DEFAULT_SECS)
                .clamp(SFX_MIN_SECS, SFX_MAX_SECS);
            let text = crate::truncate_utf8(params.prompt, SFX_MAX_PROMPT_CHARS);
            (
                format!("{}/v1/sound-generation", base),
                serde_json::json!({
                    "text": text,
                    "model_id": params.model,
                    "duration_seconds": dur,
                    "prompt_influence": 0.7
                }),
            )
        }
    };

    // SSRF 红线：base_url 属可写设置项（audio_generate LOW risk、非 BLOCKED_UPDATE），
    // 模型可经 update_settings 改写指向内网/metadata，出站前必过 check_url（与 voices.rs 同源）。
    crate::security::ssrf::check_url(&url, crate::security::ssrf::SsrfPolicy::Strict, &[]).await?;
    let resp = client
        .post(&url)
        .header("xi-api-key", params.api_key)
        .header("Content-Type", "application/json")
        .header("Accept", "audio/mpeg")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        bail!(
            "ElevenLabs {} failed ({}): {}",
            params.kind.as_str(),
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("ElevenLabs returned empty audio");
    }
    crate::app_info!(
        "design",
        "audio",
        "ElevenLabs {} produced {} bytes",
        params.kind.as_str(),
        data.len()
    );
    Ok(AudioGenResult {
        data,
        mime: "audio/mpeg".to_string(),
    })
}

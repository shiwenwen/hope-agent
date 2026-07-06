//! ElevenLabs audio provider — TTS via `POST /v1/text-to-speech/{voice_id}`
//! and music via `POST /v1/music` (both return audio bytes). Covers Speech +
//! Music + SFX (SFX routed through the music endpoint's prompt).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;

use super::types::{AudioGenParams, AudioGenProviderImpl, AudioGenResult, AudioKind};

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
const DEFAULT_TTS_MODEL: &str = "eleven_multilingual_v2";
const DEFAULT_MUSIC_MODEL: &str = "music_v1";
// A stock public ElevenLabs voice (Rachel) so speech works before a user picks one.
const DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM";

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
            AudioKind::Music | AudioKind::Sfx => DEFAULT_MUSIC_MODEL,
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
        AudioKind::Music | AudioKind::Sfx => (
            format!("{}/v1/music", base),
            serde_json::json!({ "prompt": params.prompt, "model_id": params.model }),
        ),
    };

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

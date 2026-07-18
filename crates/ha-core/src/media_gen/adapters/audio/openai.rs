//! OpenAI audio provider — text-to-speech via `POST /v1/audio/speech`
//! (returns mp3 bytes directly). Speech only.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{bail, Result};
use reqwest::Client;

use crate::media_gen::adapters::{AudioGenAdapter, AudioGenParams, AudioGenResult};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_VOICE: &str = "alloy";

pub(crate) struct OpenAiAudioProvider;

impl AudioGenAdapter for OpenAiAudioProvider {
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
    let url = format!("{}/v1/audio/speech", base);
    let voice = params.voice
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_VOICE);
    let body = serde_json::json!({
        "model": params.model,
        "input": params.prompt,
        "voice": voice,
        "response_format": "mp3",
    });

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;
    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network
    // （默认仍 Strict 档兜底），self-hosted OpenAI-compatible 才放行内网。
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
        bail!(
            "OpenAI TTS failed ({}): {}",
            status,
            crate::truncate_utf8(&err, 300)
        );
    }
    let data = resp.bytes().await?.to_vec();
    if data.is_empty() {
        bail!("OpenAI TTS returned empty audio");
    }
    crate::app_info!(
        "design",
        "audio",
        "OpenAI TTS produced {} bytes",
        data.len()
    );
    Ok(AudioGenResult {
        data,
        mime: "audio/mpeg".to_string(),
    })
}

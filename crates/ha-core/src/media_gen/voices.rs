//! Voice catalog per provider (TTS voice pickers).
//!
//! Dispatched by vendor capability (`MediaVendorKind::supports_voice_listing`):
//! ElevenLabs fetches `GET /v2/voices` live (10-minute cache keyed by
//! provider id + credential *fingerprint* — never the plaintext key);
//! OpenAI-style vendors return the documented static voice names (the
//! `/v1/audio/speech` API has no listing endpoint).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Serialize;

use super::catalog::OPENAI_TTS_VOICES;
use super::types::MediaVendorKind;

const CACHE_TTL: Duration = Duration::from_secs(600);

/// Cartesia pins behaviour to a dated API version; the header is required on
/// every request, listing included.
const CARTESIA_VERSION: &str = "2026-03-01";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceOption {
    pub voice_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, Vec<VoiceOption>)>> {
    static C: OnceLock<Mutex<HashMap<String, (Instant, Vec<VoiceOption>)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// List voices for the given configured provider (`limit` clamped 1–100).
pub async fn list_media_voices(provider_id: &str, limit: u32) -> Result<Vec<VoiceOption>> {
    let cfg = crate::config::cached_config();
    let provider = cfg
        .media_gen
        .provider(provider_id)
        .context("media provider not found")?;

    match provider.kind {
        MediaVendorKind::Elevenlabs => list_elevenlabs_voices(provider_id, limit).await,
        MediaVendorKind::Openai => Ok(OPENAI_TTS_VOICES
            .iter()
            .map(|name| VoiceOption {
                voice_id: (*name).to_string(),
                name: (*name).to_string(),
                category: None,
            })
            .collect()),
        MediaVendorKind::Cartesia => list_cartesia_voices(provider_id, limit).await,
        MediaVendorKind::Minimax => list_minimax_voices(provider_id).await,
        // Self-hosted / third-party endpoints have their own voice catalog we
        // can't enumerate — the documented OpenAI voice names would mislead.
        // Return empty; the UI keeps the free-form voice-id input.
        MediaVendorKind::OpenaiCompatible => Ok(Vec::new()),
        other => bail!("{} does not expose a voice catalog", other.display_name()),
    }
}

async fn list_elevenlabs_voices(provider_id: &str, limit: u32) -> Result<Vec<VoiceOption>> {
    let page_size = limit.clamp(1, 100);
    let cfg = crate::config::cached_config();
    let provider = cfg
        .media_gen
        .provider(provider_id)
        .context("media provider not found")?;
    let key = Some(provider.api_key.as_str())
        .filter(|k| !k.is_empty())
        .context("ElevenLabs API Key not set (Settings → Model Providers → Generation Models)")?;
    let base = provider.effective_base_url().trim_end_matches('/');

    // Cache key carries the credential *fingerprint* (not plaintext) plus
    // the provider UUID — multiple ElevenLabs entries never cross-hit.
    let fp = &blake3::hash(key.as_bytes()).to_hex()[..16];
    let cache_key = format!("{provider_id}|{base}|{page_size}|{fp}");
    if let Some((t, v)) = cache().lock().ok().and_then(|m| m.get(&cache_key).cloned()) {
        if t.elapsed() < CACHE_TTL {
            return Ok(v);
        }
    }

    let url = format!("{base}/v2/voices?page_size={page_size}");
    // base_url is owner config (trusted-ish); SSRF still gates intranet /
    // metadata unless the provider opted into private networking.
    crate::security::ssrf::check_url(&url, provider.ssrf_policy(), &cfg.ssrf.trusted_hosts).await?;
    let client =
        crate::provider::apply_proxy(Client::builder().timeout(Duration::from_secs(15))).build()?;
    let resp = client
        .get(&url)
        .header("xi-api-key", key)
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "ElevenLabs voices fetch failed ({status}): {}",
            crate::truncate_utf8(&body, 200)
        );
    }
    let json: serde_json::Value = resp.json().await?;
    let voices = json
        .get("voices")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let voice_id = v.get("voice_id")?.as_str()?.to_string();
                    let name = v
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let category = v.get("category").and_then(|c| c.as_str()).map(String::from);
                    Some(VoiceOption {
                        voice_id,
                        name,
                        category,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Ok(mut m) = cache().lock() {
        m.insert(cache_key, (Instant::now(), voices.clone()));
    }
    Ok(voices)
}

/// Shared plumbing for the vendors below: resolve credentials, SSRF-gate the
/// URL, and hand back a proxied client. Mirrors `list_elevenlabs_voices`.
async fn voice_request(provider_id: &str, path: &str) -> Result<(Client, String, String, String)> {
    let cfg = crate::config::cached_config();
    let provider = cfg
        .media_gen
        .provider(provider_id)
        .context("media provider not found")?;
    let key = Some(provider.api_key.as_str())
        .filter(|k| !k.is_empty())
        .with_context(|| {
            format!(
                "{} API Key not set (Settings → Model Configuration → Media Generation Models)",
                provider.kind.display_name()
            )
        })?
        .to_string();
    let base = provider
        .effective_base_url()
        .trim_end_matches('/')
        .to_string();
    let url = format!("{base}{path}");
    crate::security::ssrf::check_url(&url, provider.ssrf_policy(), &cfg.ssrf.trusted_hosts).await?;
    let client =
        crate::provider::apply_proxy(Client::builder().timeout(Duration::from_secs(15))).build()?;
    Ok((client, url, key, base))
}

/// Pull `(id, name)` pairs out of a vendor array, accepting either the
/// OpenAI-ish `id`/`name` keys or MiniMax-style `voice_id`/`voice_name`.
fn collect_voices(items: &[serde_json::Value], category: Option<&str>) -> Vec<VoiceOption> {
    items
        .iter()
        .filter_map(|v| {
            let voice_id = v
                .get("id")
                .or_else(|| v.get("voice_id"))
                .and_then(|x| x.as_str())?
                .to_string();
            let name = v
                .get("name")
                .or_else(|| v.get("voice_name"))
                .or_else(|| v.get("description"))
                .and_then(|x| x.as_str())
                .unwrap_or(&voice_id)
                .to_string();
            Some(VoiceOption {
                voice_id,
                name,
                category: category.map(String::from),
            })
        })
        .collect()
}

async fn list_cartesia_voices(provider_id: &str, limit: u32) -> Result<Vec<VoiceOption>> {
    let page_size = limit.clamp(1, 100);
    let (client, url, key, _) =
        voice_request(provider_id, &format!("/voices?limit={page_size}")).await?;
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {key}"))
        // Cartesia rejects any request without a dated version header.
        .header("Cartesia-Version", CARTESIA_VERSION)
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "Cartesia voices fetch failed ({status}): {}",
            crate::truncate_utf8(&body, 200)
        );
    }
    let json: serde_json::Value = resp.json().await?;
    // Paginated list endpoints return the page under `data`.
    let items = json
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(collect_voices(&items, None))
}

/// MiniMax lists voices through a POST with a selector body, and splits the
/// result into system / cloned / generated buckets.
async fn list_minimax_voices(provider_id: &str) -> Result<Vec<VoiceOption>> {
    let (client, url, key, _) = voice_request(provider_id, "/v1/get_voice").await?;
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "voice_type": "all" }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "MiniMax voices fetch failed ({status}): {}",
            crate::truncate_utf8(&body, 200)
        );
    }
    let json: serde_json::Value = resp.json().await?;
    let mut out = Vec::new();
    for (field, category) in [
        ("system_voice", "system"),
        ("voice_cloning", "cloned"),
        ("voice_generation", "generated"),
    ] {
        if let Some(items) = json.get(field).and_then(|v| v.as_array()) {
            out.extend(collect_voices(items, Some(category)));
        }
    }
    Ok(out)
}

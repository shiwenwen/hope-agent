//! ElevenLabs voices 实时拉取（B8-1）。用当前配置的 `elevenlabs` provider 的 key/base 调
//! `GET /v2/voices`，规范化为 `{voiceId,name,category}` 供设置面语音 picker。**10 分钟缓存按
//! 凭据指纹（blake3 前 16，不缓明文 key）**；无 key → 明确错误；出站过 SSRF（Strict 挡内网）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Serialize;

const DEFAULT_BASE_URL: &str = "https://api.elevenlabs.io";
const CACHE_TTL: Duration = Duration::from_secs(600);

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

/// 拉 ElevenLabs 语音列表（`limit` 钳 1–100）。缓存命中即返回，避免每次开面板都打网。
pub async fn list_elevenlabs_voices(limit: u32) -> Result<Vec<VoiceOption>> {
    let page_size = limit.clamp(1, 100);
    let cfg = crate::config::cached_config();
    let entry = cfg
        .audio_generate
        .providers
        .iter()
        .find(|p| super::normalize_provider_id(&p.id) == "elevenlabs")
        .context("ElevenLabs 未在音频设置中配置")?;
    let key = entry
        .api_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .context("未设置 ElevenLabs API Key（设置 → 工具 → 音频）")?;
    let base = entry
        .base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    // 缓存键含凭据**指纹**（非明文），key 换了即 miss。
    let fp = &blake3::hash(key.as_bytes()).to_hex()[..16];
    let cache_key = format!("{base}|{page_size}|{fp}");
    if let Some((t, v)) = cache()
        .lock()
        .ok()
        .and_then(|m| m.get(&cache_key).cloned())
    {
        if t.elapsed() < CACHE_TTL {
            return Ok(v);
        }
    }

    let url = format!("{base}/v2/voices?page_size={page_size}");
    // base_url 为 owner 配置（可信），SSRF Strict 兜底挡内网 / 元数据。
    crate::security::ssrf::check_url(&url, crate::security::ssrf::SsrfPolicy::Strict, &[]).await?;
    let client = crate::provider::apply_proxy(Client::builder().timeout(Duration::from_secs(15)))
        .build()?;
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
            "ElevenLabs voices 拉取失败 ({status}): {}",
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

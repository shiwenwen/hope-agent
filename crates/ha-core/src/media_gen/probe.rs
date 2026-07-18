//! Connectivity probes for media providers ("Test connection" button).
//!
//! Migrated from `provider::test::test_image_generate` and extended with
//! audio-capable vendors. Lightweight GETs only — nothing is billed. The
//! JSON result shape (`{success,message,url,status,latencyMs,auth}`)
//! matches the legacy probe so the frontend `TestResultDisplay` parser
//! keeps working; `Ok`/`Err` both carry that JSON (Err = failure).

use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::provider::apply_proxy;

use super::types::MediaVendorKind;

/// Probe input: either a saved provider (`provider_id`, credentials read
/// from config) or a pre-save draft (`kind` + `api_key` + `base_url`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestMediaProviderInput {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub kind: Option<MediaVendorKind>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Draft-only: mirrors the provider's `allow_private_network` so a
    /// self-hosted (localhost) OpenAI-compatible endpoint can be tested
    /// before it's saved. Ignored when `provider_id` is set (the saved
    /// provider's own policy applies).
    #[serde(default)]
    pub allow_private_network: bool,
}

fn fail(message: String) -> String {
    serde_json::to_string(&serde_json::json!({
        "success": false,
        "message": message,
    }))
    .unwrap_or_default()
}

pub async fn test_media_provider(input: TestMediaProviderInput) -> Result<String, String> {
    // Resolve (kind, key, base, ssrf policy) from a saved provider or draft.
    let (kind, api_key, base_url, ssrf_policy) = if let Some(pid) = &input.provider_id {
        let cfg = crate::config::cached_config();
        let Some(provider) = cfg.media_gen.provider(pid) else {
            return Err(fail(format!("Unknown media provider: {pid}")));
        };
        (
            provider.kind,
            provider.api_key.clone(),
            provider.effective_base_url().to_string(),
            provider.ssrf_policy(),
        )
    } else {
        let Some(kind) = input.kind else {
            return Err(fail("Missing provider kind".to_string()));
        };
        let base = input
            .base_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| kind.default_base_url().to_string());
        let policy = if input.allow_private_network {
            crate::security::ssrf::SsrfPolicy::AllowPrivate
        } else {
            crate::config::cached_config().ssrf.default_policy
        };
        (
            kind,
            input.api_key.clone().unwrap_or_default(),
            base,
            policy,
        )
    };
    let base = base_url.trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(fail("Base URL required for this provider".to_string()));
    }
    let display_name = kind.display_name();

    // Per-vendor probe endpoint + auth (transcribed from the legacy image
    // probes; audio vendors added).
    let (url, auth_header, auth_value) = match kind {
        MediaVendorKind::Openai | MediaVendorKind::OpenaiCompatible => (
            format!("{base}/v1/models"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Google => (
            format!("{base}/v1beta/models?key={api_key}"),
            "",
            String::new(),
        ),
        MediaVendorKind::Fal => (
            format!("{base}/fal-ai/flux/dev"),
            "Authorization",
            format!("Key {api_key}"),
        ),
        MediaVendorKind::Minimax => {
            let host = if let Ok(parsed) = url::Url::parse(&base) {
                format!(
                    "{}://{}",
                    parsed.scheme(),
                    parsed.host_str().unwrap_or(&base)
                )
            } else {
                base.clone()
            };
            (
                format!("{host}/v1/image_generation"),
                "Authorization",
                format!("Bearer {api_key}"),
            )
        }
        MediaVendorKind::Siliconflow => (
            format!("{base}/v1/models"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Zhipu => (
            format!("{base}/v4/images/generations"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Tongyi => (
            format!("{base}/api/v1/services/aigc/text2image/image-synthesis"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Elevenlabs => (
            format!("{base}/v2/voices?page_size=1"),
            "xi-api-key",
            api_key.clone(),
        ),
        // OpenAI-compatible planes with a documented model-listing endpoint.
        MediaVendorKind::Stepfun | MediaVendorKind::Together | MediaVendorKind::Xai => (
            format!("{base}/v1/models"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        // Recraft has no model list; `/users/me` is the documented account
        // endpoint (also returns the credit balance).
        MediaVendorKind::Recraft => (
            format!("{base}/v1/users/me"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        // No documented listing endpoint — GET the generation path and let
        // the method-not-allowed / unprocessable reply prove reachability
        // plus credential validity (same trick as Fal).
        MediaVendorKind::Volcengine => (
            format!("{base}/api/v3/images/generations"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Hunyuan | MediaVendorKind::Sensenova => (
            format!("{base}/v1/images/generations"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        // Pure-TTS vendors: probe their voice/model listing endpoint.
        MediaVendorKind::Cartesia => (
            format!("{base}/voices?limit=1"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        // Deepgram's scheme word is `Token`, not `Bearer`.
        MediaVendorKind::Deepgram => (
            format!("{base}/v1/models"),
            "Authorization",
            format!("Token {api_key}"),
        ),
        MediaVendorKind::Fishaudio => (
            format!("{base}/model?page_size=1"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
        MediaVendorKind::Hume => (
            format!("{base}/v0/tts/voices?provider=HUME_AI&page_size=1"),
            "X-Hume-Api-Key",
            api_key.clone(),
        ),
        MediaVendorKind::Qianfan => (
            format!("{base}/v2/images/generations"),
            "Authorization",
            format!("Bearer {api_key}"),
        ),
    };

    // SSRF gate: probes hit user-typed URLs before they're saved.
    {
        let cfg = crate::config::cached_config();
        if let Err(e) =
            crate::security::ssrf::check_url(&url, ssrf_policy, &cfg.ssrf.trusted_hosts).await
        {
            return Err(fail(format!("{display_name} endpoint blocked: {e}")));
        }
    }

    let start = Instant::now();
    let client = apply_proxy(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15)),
    )
    .build()
    .map_err(|e| fail(format!("Client error: {e}")))?;

    let mut req = client.get(&url);
    if !auth_header.is_empty() {
        req = req.header(auth_header, &auth_value);
    }

    let sanitize = |u: &str| {
        if api_key.is_empty() {
            u.to_string()
        } else {
            u.replace(&api_key, "***")
        }
    };

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency = start.elapsed().as_millis() as u64;
            // Vendors probed by GETting a POST-only generation route: 405 /
            // 422 still prove we reached the right service with a key it
            // accepted (401/403 fall through to the auth-failure branch).
            let post_only_route = matches!(
                kind,
                MediaVendorKind::Fal
                    | MediaVendorKind::Volcengine
                    | MediaVendorKind::Hunyuan
                    | MediaVendorKind::Sensenova
                    | MediaVendorKind::Qianfan
            );
            let ok = status < 400 || (post_only_route && (status == 405 || status == 422));
            let msg = if ok {
                format!("{display_name} 连接成功")
            } else if status == 401 || status == 403 {
                format!("{display_name} 认证失败，请检查 API Key")
            } else {
                format!("{display_name} 请求失败 ({status})")
            };
            Ok(serde_json::to_string(&serde_json::json!({
                "success": ok,
                "message": msg,
                "url": sanitize(&url),
                "status": status,
                "latencyMs": latency,
                "auth": if auth_header.is_empty() { "Query Parameter" } else { auth_header },
            }))
            .unwrap_or_default())
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            let msg = if e.is_timeout() {
                format!("{display_name} 连接超时，请检查网络或代理设置")
            } else if e.is_connect() {
                format!("{display_name} 无法连接，请检查网络或 Base URL")
            } else {
                format!("{display_name} 连接失败: {e}")
            };
            Err(serde_json::to_string(&serde_json::json!({
                "success": false,
                "message": msg,
                "url": sanitize(&url),
                "latencyMs": latency,
            }))
            .unwrap_or_default())
        }
    }
}

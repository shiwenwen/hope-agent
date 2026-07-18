//! Kuaishou Kling image generation (`/v1/images/generations`).
//!
//! Async submit + poll, sharing the Kling envelope `{code, message,
//! request_id, data:{task_id, task_status, task_result}}` — `code != 0` is a
//! business error even under HTTP 200. Results are URL-only (no `b64_json`),
//! so every image is re-fetched through the SSRF-gated `fetch_asset`.
//!
//! Auth has two documented shapes. The console-issued static API key is the
//! current recommendation and applies to all models; the legacy AK/SK scheme
//! signs a short-lived HS256 JWT. We pick between them by *shape*: an
//! `ak:sk` pair (colon-separated) means the legacy signer, anything else is
//! used verbatim as a bearer token. That keeps one config field for both.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;

use crate::media_gen::adapters::fetch::fetch_asset;
use crate::media_gen::adapters::{GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult};

/// International endpoint. Kling runs region-split domains (`api-singapore`
/// vs `api-beijing`) with no universal host, so a mainland account must set
/// `base_url` explicitly — this is only the fallback for an empty config.
const DEFAULT_BASE_URL: &str = "https://api-singapore.klingai.com";

const GENERATION_PATH: &str = "/v1/images/generations";

const MAX_PROMPT_CHARS: usize = 2500;
const MAX_N: u32 = 9;

/// Legacy AK/SK JWTs are minted with the vendor's documented window: valid
/// for 30 minutes, backdated 5s to absorb clock skew.
const JWT_TTL_SECS: u64 = 1800;
const JWT_BACKDATE_SECS: u64 = 5;

const POLL_INITIAL_MS: u64 = 1000;
const POLL_STEP_MS: u64 = 1000;
const POLL_MAX_MS: u64 = 3000;

const FALLBACK_MIME: &str = "image/png";

const ASPECT_RATIOS: &[&str] = &["16:9", "9:16", "1:1", "4:3", "3:4", "3:2", "2:3", "21:9"];

/// Only this model family documents 4K output; asking for it elsewhere is a
/// server-side rejection.
const MODEL_SUPPORTING_4K: &str = "kling-v3-omni";

/// `image_reference` is mandatory for this model when an input image is
/// present, and undocumented for the others.
const MODEL_REQUIRING_IMAGE_REFERENCE: &str = "kling-v1-5";

pub(crate) struct Provider;

impl ImageGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

// ── Wire types ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Envelope {
    code: Option<i64>,
    message: Option<String>,
    data: Option<TaskData>,
}

#[derive(Deserialize, Debug)]
struct TaskData {
    task_id: Option<String>,
    task_status: Option<String>,
    task_result: Option<TaskResult>,
    #[serde(default)]
    task_status_msg: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TaskResult {
    #[serde(default)]
    images: Vec<TaskImage>,
}

#[derive(Deserialize, Debug)]
struct TaskImage {
    #[serde(default)]
    index: Option<u32>,
    url: Option<String>,
}

// ── Auth ──────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum Auth {
    Static(String),
    Jwt {
        access_key: String,
        secret_key: String,
    },
}

/// A colon in the configured key marks the legacy AK/SK pair; console API
/// keys never contain one.
fn classify_auth(api_key: &str) -> Auth {
    let key = api_key.trim();
    match key.split_once(':') {
        Some((ak, sk)) if !ak.trim().is_empty() && !sk.trim().is_empty() => Auth::Jwt {
            access_key: ak.trim().to_string(),
            secret_key: sk.trim().to_string(),
        },
        _ => Auth::Static(key.to_string()),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// HS256 JWT with the exact claim set Kling's legacy signer documents.
/// Hand-rolled rather than via a JWT crate so the segment order is fixed and
/// therefore testable.
fn sign_jwt(access_key: &str, secret_key: &str, now_secs: u64) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(
        format!(
            r#"{{"iss":"{}","exp":{},"nbf":{}}}"#,
            json_escape(access_key),
            now_secs.saturating_add(JWT_TTL_SECS),
            now_secs.saturating_sub(JWT_BACKDATE_SECS),
        )
        .as_bytes(),
    );

    let signing_input = format!("{header}.{payload}");
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret_key.as_bytes())
        .expect("HMAC-SHA256 accepts keys of any length");
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    format!("{signing_input}.{signature}")
}

fn bearer_token(auth: &Auth, now_secs: u64) -> String {
    match auth {
        Auth::Static(key) => key.clone(),
        Auth::Jwt {
            access_key,
            secret_key,
        } => sign_jwt(access_key, secret_key, now_secs),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Pure request helpers ──────────────────────────────────────────

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TaskState {
    Pending,
    Succeeded,
    Failed,
    Unknown,
}

fn classify_status(status: &str) -> TaskState {
    match status.trim().to_ascii_lowercase().as_str() {
        "submitted" | "processing" | "pending" | "queued" | "running" => TaskState::Pending,
        "succeed" | "succeeded" | "success" => TaskState::Succeeded,
        "failed" | "failure" | "error" => TaskState::Failed,
        _ => TaskState::Unknown,
    }
}

fn clamp_n(n: u32) -> u32 {
    n.clamp(1, MAX_N)
}

/// Kling takes a ratio enum, never pixel dimensions. Prefer the caller's
/// explicit hint; otherwise recover a ratio from the pixel `size` so a
/// generic "1024x1024" request still lands on 1:1 instead of the vendor's
/// 16:9 default.
fn resolve_aspect_ratio(explicit: Option<&str>, size: &str) -> Option<String> {
    if let Some(raw) = explicit {
        let normalized = raw.trim();
        if let Some(hit) = ASPECT_RATIOS.iter().find(|r| **r == normalized) {
            return Some((*hit).to_string());
        }
    }
    ratio_from_size(size)
}

fn ratio_from_size(size: &str) -> Option<String> {
    let (w, h) = size
        .trim()
        .split_once(['x', 'X', '*'])
        .and_then(|(w, h)| Some((w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?)))?;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let target = w / h;
    // Only accept a near-exact bucket; a stray size must not be silently
    // reshaped into a materially different crop.
    ASPECT_RATIOS
        .iter()
        .filter_map(|r| {
            let (rw, rh) = r.split_once(':')?;
            let value = rw.parse::<f64>().ok()? / rh.parse::<f64>().ok()?;
            let error = (value - target).abs() / target;
            (error <= 0.02).then_some((*r, error))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(r, _)| r.to_string())
}

/// `4k` exists only on the omni model; everything else caps at `2k`.
fn resolve_resolution(explicit: Option<&str>, model: &str) -> Option<String> {
    let normalized = explicit?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1k" => Some("1k".to_string()),
        "2k" => Some("2k".to_string()),
        "4k" => Some(if model.eq_ignore_ascii_case(MODEL_SUPPORTING_4K) {
            "4k".to_string()
        } else {
            "2k".to_string()
        }),
        _ => None,
    }
}

/// `subject` | `face`; anything else would be a server-side 400, so an
/// unrecognized override falls back to the documented default.
fn resolve_image_reference(extra: &HashMap<String, String>, model: &str) -> Option<String> {
    let explicit = extra
        .get("image_reference")
        .map(|s| s.trim().to_ascii_lowercase());
    match explicit.as_deref() {
        Some("subject") => Some("subject".to_string()),
        Some("face") => Some("face".to_string()),
        _ if model.eq_ignore_ascii_case(MODEL_REQUIRING_IMAGE_REFERENCE) => {
            Some("subject".to_string())
        }
        _ => None,
    }
}

fn negative_prompt(extra: &HashMap<String, String>) -> Option<&str> {
    extra
        .get("negative_prompt")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_PROMPT_CHARS)
}

struct BodyInput<'a> {
    model: &'a str,
    prompt: &'a str,
    n: u32,
    aspect_ratio: Option<String>,
    resolution: Option<String>,
    /// Bare base64 (no `data:` prefix) or an http(s) URL.
    image: Option<String>,
    image_reference: Option<String>,
    negative_prompt: Option<&'a str>,
}

fn build_body(input: BodyInput<'_>) -> Value {
    // `model_name`, not `model` — the legacy generations endpoint keys the
    // model off this field and rejects OpenAI's spelling.
    let mut body = serde_json::json!({
        "model_name": input.model,
        "prompt": input.prompt,
        "n": clamp_n(input.n),
    });

    if let Some(ratio) = input.aspect_ratio {
        body["aspect_ratio"] = Value::String(ratio);
    }
    if let Some(resolution) = input.resolution {
        body["resolution"] = Value::String(resolution);
    }
    if let Some(image) = input.image {
        body["image"] = Value::String(image);
        if let Some(reference) = input.image_reference {
            body["image_reference"] = Value::String(reference);
        }
    } else if let Some(negative) = input.negative_prompt {
        // Documented as unsupported in image-to-image mode.
        body["negative_prompt"] = Value::String(negative.to_string());
    }

    body
}

/// `code != 0` is a business error even under HTTP 200.
fn check_envelope(envelope: Envelope, stage: &str) -> Result<TaskData> {
    let code = envelope.code.unwrap_or(0);
    if code != 0 {
        let msg = envelope.message.unwrap_or_default();
        anyhow::bail!(
            "Kling image {} error (code={}): {}",
            stage,
            code,
            crate::truncate_utf8(&msg, 300)
        );
    }
    envelope
        .data
        .ok_or_else(|| anyhow::anyhow!("Kling image: {} response carried no data", stage))
}

// ── Implementation ────────────────────────────────────────────────

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    if params.prompt.trim().is_empty() {
        anyhow::bail!("Kling image: prompt is empty");
    }
    if params.prompt.chars().count() > MAX_PROMPT_CHARS {
        anyhow::bail!(
            "Kling image: prompt is {} characters, exceeding the {}-character limit",
            params.prompt.chars().count(),
            MAX_PROMPT_CHARS
        );
    }

    let base = params
        .base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    // The endpoint takes a single reference image (string), so extra inputs
    // would be silently dropped — say so rather than pretend they were used.
    if params.input_images.len() > 1 {
        app_warn!(
            "tool",
            "image_generate",
            "Kling image: {} input images supplied but the generations endpoint accepts one — using the first",
            params.input_images.len()
        );
    }
    let image = params
        .input_images
        .first()
        // Kling wants raw base64 here; a `data:` prefix is rejected.
        .map(|img| STANDARD.encode(&img.data));

    let body = build_body(BodyInput {
        model: params.model,
        prompt: params.prompt,
        n: params.n,
        aspect_ratio: resolve_aspect_ratio(params.aspect_ratio, params.size),
        resolution: resolve_resolution(params.resolution, params.model),
        image_reference: image
            .is_some()
            .then(|| resolve_image_reference(params.extra, params.model))
            .flatten(),
        image,
        negative_prompt: negative_prompt(params.extra),
    });

    let submit_url = format!("{}{}", base, GENERATION_PATH);
    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(&submit_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let auth = classify_auth(params.api_key);
    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    app_info!(
        "tool",
        "image_generate",
        "Kling image submit: model={}, n={}, edit={}, jwt_auth={}",
        params.model,
        clamp_n(params.n),
        !params.input_images.is_empty(),
        matches!(auth, Auth::Jwt { .. })
    );

    let started = Instant::now();
    let deadline = started + Duration::from_secs(params.timeout_secs);

    let resp = client
        .post(&submit_url)
        .header(
            "Authorization",
            format!("Bearer {}", bearer_token(&auth, now_secs())),
        )
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "Kling image submit failed ({}): {}",
            status.as_u16(),
            crate::truncate_utf8(&text, 300)
        );
    }

    let envelope: Envelope = serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!(
            "Kling image: unparseable submit response ({}): {}",
            e,
            crate::truncate_utf8(&text, 300)
        )
    })?;
    let data = check_envelope(envelope, "submit")?;

    let task_id = data
        .task_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Kling image: submit response carried no task_id"))?;

    let poll_url = format!("{}{}/{}", base, GENERATION_PATH, task_id);
    crate::security::ssrf::check_url(&poll_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let mut poll_interval_ms = POLL_INITIAL_MS;
    loop {
        if Instant::now() >= deadline {
            anyhow::bail!(
                "Kling image task timed out after {}s (task_id={})",
                params.timeout_secs,
                task_id
            );
        }
        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;

        let poll_resp = client
            .get(&poll_url)
            // Re-minted per poll so a long-running task cannot outlive a JWT.
            .header(
                "Authorization",
                format!("Bearer {}", bearer_token(&auth, now_secs())),
            )
            .send()
            .await?;

        let poll_status = poll_resp.status();
        let poll_text = poll_resp.text().await.unwrap_or_default();
        if !poll_status.is_success() {
            anyhow::bail!(
                "Kling image poll failed ({}, task_id={}): {}",
                poll_status.as_u16(),
                task_id,
                crate::truncate_utf8(&poll_text, 300)
            );
        }

        let envelope: Envelope = serde_json::from_str(&poll_text).map_err(|e| {
            anyhow::anyhow!(
                "Kling image: unparseable poll response ({}): {}",
                e,
                crate::truncate_utf8(&poll_text, 300)
            )
        })?;
        let data = check_envelope(envelope, "poll")?;
        let task_status = data.task_status.as_deref().unwrap_or("unknown");

        match classify_status(task_status) {
            TaskState::Succeeded => {
                app_info!(
                    "tool",
                    "image_generate",
                    "Kling image task completed in {}ms (task_id={})",
                    started.elapsed().as_millis() as u64,
                    task_id
                );
                return download_images(data, &params, &task_id).await;
            }
            TaskState::Failed => {
                let reason = data.task_status_msg.unwrap_or_default();
                anyhow::bail!(
                    "Kling image task failed (task_id={}): {}",
                    task_id,
                    crate::truncate_utf8(&reason, 300)
                );
            }
            TaskState::Unknown => {
                app_warn!(
                    "tool",
                    "image_generate",
                    "Kling image unknown task status: {} (task_id={})",
                    task_status,
                    task_id
                );
                poll_interval_ms = (poll_interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);
            }
            TaskState::Pending => {
                poll_interval_ms = (poll_interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);
            }
        }
    }
}

async fn download_images(
    data: TaskData,
    params: &ImageGenParams<'_>,
    task_id: &str,
) -> Result<ImageGenResult> {
    let mut entries = data
        .task_result
        .map(|r| r.images)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|img| img.url.map(|url| (img.index.unwrap_or(0), url)))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        anyhow::bail!(
            "Kling image task succeeded but carried no image URLs (task_id={})",
            task_id
        );
    }
    entries.sort_by_key(|(index, _)| *index);

    let mut images = Vec::with_capacity(entries.len());
    for (_, url) in entries {
        let (bytes, mime) = fetch_asset(&url, params.ssrf, FALLBACK_MIME).await?;
        if bytes.is_empty() {
            anyhow::bail!(
                "Kling image: downloaded asset was empty (task_id={})",
                task_id
            );
        }
        images.push(GeneratedImage {
            data: bytes,
            mime,
            revised_prompt: None,
        });
    }

    Ok(ImageGenResult { images, text: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn auth_shape_selects_static_key_or_ak_sk_pair() {
        assert_eq!(
            classify_auth(" sk-abc123 "),
            Auth::Static("sk-abc123".to_string())
        );
        assert_eq!(
            classify_auth("my-ak:my-sk"),
            Auth::Jwt {
                access_key: "my-ak".to_string(),
                secret_key: "my-sk".to_string(),
            }
        );
        // A dangling colon is not a usable pair — treat it as a static key.
        assert_eq!(classify_auth("ak:"), Auth::Static("ak:".to_string()));
    }

    #[test]
    fn jwt_segments_are_correct_base64url() {
        let token = sign_jwt("test-ak", "test-sk", 1_700_000_000);
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);

        // Canonical HS256 header encoding.
        assert_eq!(parts[0], "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
        assert_eq!(
            String::from_utf8(URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap(),
            r#"{"alg":"HS256","typ":"JWT"}"#
        );

        assert_eq!(
            String::from_utf8(URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap(),
            r#"{"iss":"test-ak","exp":1700001800,"nbf":1699999995}"#
        );

        // 32-byte HMAC output, unpadded base64url.
        assert_eq!(parts[2].len(), 43);
        assert!(!parts[2].contains('=') && !parts[2].contains('+') && !parts[2].contains('/'));

        // Deterministic, and bound to the secret.
        assert_eq!(token, sign_jwt("test-ak", "test-sk", 1_700_000_000));
        assert_ne!(
            token.split('.').nth(2),
            sign_jwt("test-ak", "other-sk", 1_700_000_000)
                .split('.')
                .nth(2)
        );
    }

    #[test]
    fn body_uses_model_name_and_clamps_n() {
        let body = build_body(BodyInput {
            model: "kling-v2-1",
            prompt: "a cat",
            n: 42,
            aspect_ratio: Some("1:1".to_string()),
            resolution: Some("2k".to_string()),
            image: None,
            image_reference: None,
            negative_prompt: Some("blurry"),
        });

        assert_eq!(body["model_name"], "kling-v2-1");
        assert!(body.get("model").is_none());
        assert_eq!(body["n"], MAX_N);
        assert_eq!(body["aspect_ratio"], "1:1");
        assert_eq!(body["resolution"], "2k");
        assert_eq!(body["negative_prompt"], "blurry");
        assert!(body.get("image").is_none());
    }

    #[test]
    fn image_to_image_body_drops_negative_prompt_and_sends_bare_base64() {
        let encoded = STANDARD.encode([0xffu8, 0xd8, 0xff]);
        let body = build_body(BodyInput {
            model: "kling-v1-5",
            prompt: "restyle",
            n: 1,
            aspect_ratio: None,
            resolution: None,
            image: Some(encoded.clone()),
            image_reference: resolve_image_reference(&extra(&[]), "kling-v1-5"),
            negative_prompt: Some("blurry"),
        });

        assert_eq!(body["image"], encoded);
        assert!(!body["image"].as_str().unwrap().starts_with("data:"));
        // Unsupported alongside an input image.
        assert!(body.get("negative_prompt").is_none());
        // Mandatory for kling-v1-5 img2img.
        assert_eq!(body["image_reference"], "subject");

        // Other models only send it when the user asked for one.
        assert_eq!(resolve_image_reference(&extra(&[]), "kling-v3"), None);
        assert_eq!(
            resolve_image_reference(&extra(&[("image_reference", " FACE ")]), "kling-v3"),
            Some("face".to_string())
        );
        assert_eq!(
            resolve_image_reference(&extra(&[("image_reference", "bogus")]), "kling-v3"),
            None
        );
    }

    #[test]
    fn dimension_hints_map_onto_kling_enums() {
        assert_eq!(
            resolve_aspect_ratio(Some("21:9"), "1024x1024"),
            Some("21:9".to_string())
        );
        // Unsupported explicit ratio falls back to the size-derived bucket.
        assert_eq!(
            resolve_aspect_ratio(Some("5:4"), "1024x1024"),
            Some("1:1".to_string())
        );
        assert_eq!(
            resolve_aspect_ratio(None, "1920*1080"),
            Some("16:9".to_string())
        );
        // Nothing close enough — omit and let Kling default.
        assert_eq!(resolve_aspect_ratio(None, "1000x700"), None);
        assert_eq!(resolve_aspect_ratio(None, "auto"), None);

        assert_eq!(
            resolve_resolution(Some("1K"), "kling-v3"),
            Some("1k".into())
        );
        // 4K exists only on the omni model.
        assert_eq!(
            resolve_resolution(Some("4K"), "kling-v3"),
            Some("2k".into())
        );
        assert_eq!(
            resolve_resolution(Some("4k"), "kling-v3-omni"),
            Some("4k".into())
        );
        assert_eq!(resolve_resolution(None, "kling-v3"), None);
    }

    #[test]
    fn status_classification_covers_kling_terminal_states() {
        assert_eq!(classify_status("submitted"), TaskState::Pending);
        assert_eq!(classify_status("processing"), TaskState::Pending);
        assert_eq!(classify_status("succeed"), TaskState::Succeeded);
        assert_eq!(classify_status(" SUCCEED "), TaskState::Succeeded);
        assert_eq!(classify_status("failed"), TaskState::Failed);
        assert_eq!(classify_status("weird"), TaskState::Unknown);
    }

    #[test]
    fn nonzero_code_is_an_error_even_on_http_200() {
        let envelope = Envelope {
            code: Some(1103),
            message: Some("account balance not enough".to_string()),
            data: None,
        };
        let err = check_envelope(envelope, "submit").unwrap_err().to_string();
        assert!(err.contains("1103"), "{err}");
        assert!(err.contains("balance"), "{err}");

        let ok = Envelope {
            code: Some(0),
            message: None,
            data: Some(TaskData {
                task_id: Some("t-1".into()),
                task_status: Some("submitted".into()),
                task_result: None,
                task_status_msg: None,
            }),
        };
        assert_eq!(
            check_envelope(ok, "submit").unwrap().task_id.as_deref(),
            Some("t-1")
        );
    }
}

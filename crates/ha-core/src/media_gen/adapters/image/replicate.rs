//! Replicate — image aggregation gateway.
//!
//! Two wire shapes share one prediction lifecycle:
//! official models (`owner/name`) post to
//! `/v1/models/{owner}/{name}/predictions` with no version, community
//! models post to `/v1/predictions` with a `version` hash in the body.
//!
//! `Prefer: wait=n` is an *optimization, not a guarantee*: when the model
//! does not finish inside the window Replicate still answers 200/201 with a
//! `starting`/`processing` prediction, so the polling loop below is the real
//! completion path, not a fallback we hope never runs.
//!
//! Per-model `input` is free-form JSON defined by the model author (flux uses
//! `aspect_ratio`, others use width/height). We therefore send only `prompt`
//! plus caller-supplied `extra`, and never invent model-specific fields.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::media_gen::adapters::{GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult};

/// Host only — the `/v1` segment is added per path below. The configured
/// provider base URL is a host too (the template and the connectivity probe
/// both assume that), so keeping `/v1` here would yield `…/v1/v1/…` for one
/// of them and a 404 for the other.
const DEFAULT_BASE_URL: &str = "https://api.replicate.com";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const MAX_PREFER_WAIT_SECS: u64 = 60;
/// Kept back from `Prefer: wait` so the server's hold always ends before the
/// client's own timeout fires.
const PREFER_WAIT_HEADROOM_SECS: u64 = 5;
const POLL_INITIAL_MS: u64 = 1000;
const POLL_STEP_MS: u64 = 1000;
const POLL_MAX_MS: u64 = 3000;

// ── Response types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct Prediction {
    id: Option<String>,
    status: Option<String>,
    output: Option<Value>,
    /// Free-form: string on most models, occasionally an object.
    error: Option<Value>,
    urls: Option<PredictionUrls>,
}

#[derive(Deserialize)]
struct PredictionUrls {
    get: Option<String>,
}

// ── Pure helpers ──────────────────────────────────────────────────

/// Which endpoint/body shape a configured model id maps to.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Official model: version is implicit in the path.
    Official { owner: String, name: String },
    /// Community model pinned to a version hash.
    Version { hash: String },
}

fn looks_like_version_hash(s: &str) -> bool {
    s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// `owner/name` → official path; `owner/name:hash` or a bare hash → versioned
/// `/predictions`. Ambiguity is resolved by the version hash winning: pinning
/// a version is always an explicit act by the user.
fn resolve_route(model: &str) -> Result<Route> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("Replicate: empty model id");
    }

    // The owner/name prefix is intentionally dropped: /v1/predictions keys
    // purely off the version hash.
    if let Some((_prefix, version)) = model.rsplit_once(':') {
        let version = version.trim();
        if version.is_empty() {
            anyhow::bail!("Replicate: model '{}' has an empty version hash", model);
        }
        return Ok(Route::Version {
            hash: version.to_string(),
        });
    }

    if let Some((owner, name)) = model.split_once('/') {
        let (owner, name) = (owner.trim(), name.trim());
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            anyhow::bail!(
                "Replicate: model '{}' is not a valid owner/name reference",
                model
            );
        }
        return Ok(Route::Official {
            owner: owner.to_string(),
            name: name.to_string(),
        });
    }

    if looks_like_version_hash(model) {
        return Ok(Route::Version {
            hash: model.to_string(),
        });
    }

    anyhow::bail!(
        "Replicate: model '{}' must be 'owner/name', 'owner/name:version' or a version hash",
        model
    )
}

/// `extra` arrives as strings, but model schemas expect real JSON scalars
/// (`num_inference_steps: 28`, `disable_safety_checker: true`). Coerce what
/// unambiguously parses, keep everything else as a string.
fn coerce_extra_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    match trimmed {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Value::from(i);
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        if f.is_finite() {
            return Value::from(f);
        }
    }
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            return v;
        }
    }
    Value::String(raw.to_string())
}

/// Build the model `input` object. `extra` wins over our defaults so a user
/// can override `aspect_ratio` (or drop in width/height for models that use
/// them) without adapter changes.
fn build_input(
    prompt: &str,
    aspect_ratio: Option<&str>,
    extra: &std::collections::HashMap<String, String>,
) -> Value {
    let mut input = Map::new();
    input.insert("prompt".into(), Value::String(prompt.to_string()));
    if let Some(ar) = aspect_ratio.filter(|s| !s.is_empty()) {
        input.insert("aspect_ratio".into(), Value::String(ar.to_string()));
    }
    for (k, v) in extra {
        if k.is_empty() {
            continue;
        }
        input.insert(k.clone(), coerce_extra_value(v));
    }
    Value::Object(input)
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Pending,
    Succeeded,
    Failed,
    Canceled,
}

/// Unknown statuses are treated as pending: Replicate may add lifecycle
/// states, and bailing on one would turn a still-running job into an error.
fn classify_status(status: &str) -> Phase {
    match status {
        "succeeded" => Phase::Succeeded,
        "failed" => Phase::Failed,
        "canceled" | "cancelled" => Phase::Canceled,
        _ => Phase::Pending,
    }
}

/// `output` is a bare URL string on single-image models and an array on
/// multi-image ones; both shapes are live in the model catalog.
fn collect_output_urls(output: Option<&Value>) -> Vec<String> {
    match output {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn describe_error(err: Option<&Value>) -> String {
    match err {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::Null) | None => "no error detail".to_string(),
        Some(other) => other.to_string(),
    }
}

/// Replicate accepts 1..=60; asking for longer than our own budget only
/// delays the timeout error.
/// `Prefer: wait` must stay strictly under the client's own timeout: if the
/// two are equal the server legitimately holds the connection for the whole
/// window and reqwest aborts at the same instant, so the polling fallback —
/// the actual completion path — is never reached.
fn prefer_wait_secs(timeout_secs: u64) -> u64 {
    timeout_secs
        .saturating_sub(PREFER_WAIT_HEADROOM_SECS)
        .clamp(1, MAX_PREFER_WAIT_SECS)
}

// ── Adapter ───────────────────────────────────────────────────────

pub(crate) struct Provider;

impl ImageGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');

    let route = resolve_route(params.model)?;
    let mut body = Map::new();
    let submit_url = match &route {
        Route::Official { owner, name } => {
            format!("{}/v1/models/{}/{}/predictions", base, owner, name)
        }
        Route::Version { hash } => {
            body.insert("version".into(), Value::String(hash.clone()));
            format!("{}/v1/predictions", base)
        }
    };
    body.insert(
        "input".into(),
        build_input(params.prompt, params.aspect_ratio, params.extra),
    );

    if !params.input_images.is_empty() {
        app_warn!(
            "tool",
            "image_generate",
            "Replicate: {} input image(s) ignored — the reference-image field name is model-specific; pass it explicitly via extra",
            params.input_images.len()
        );
    }

    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(&submit_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    app_info!(
        "tool",
        "image_generate",
        "Replicate submit: model={}, url={}, timeout={}s",
        params.model,
        submit_url,
        params.timeout_secs
    );

    let started = Instant::now();
    // Anchored before submit on purpose here: Replicate's submit already
    // blocks for `Prefer: wait`, so that time is genuinely part of the poll
    // budget rather than upload overhead.
    let deadline = started + Duration::from_secs(params.timeout_secs);

    let resp = client
        .post(&submit_url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .header(
            "Prefer",
            format!("wait={}", prefer_wait_secs(params.timeout_secs)),
        )
        .json(&Value::Object(body))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Replicate prediction submit failed ({}): {}",
            status.as_u16(),
            crate::truncate_utf8(&text, 300)
        );
    }

    let mut prediction: Prediction = resp.json().await?;
    let prediction_id = prediction.id.clone().unwrap_or_default();

    // urls.get is server-supplied and outside the configured base, so it is
    // re-gated through SSRF before every poll.
    let poll_url = prediction
        .urls
        .as_ref()
        .and_then(|u| u.get.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/v1/predictions/{}", base, prediction_id));

    let mut poll_interval_ms = POLL_INITIAL_MS;
    loop {
        let phase = classify_status(prediction.status.as_deref().unwrap_or(""));
        match phase {
            Phase::Succeeded => break,
            Phase::Failed => {
                anyhow::bail!(
                    "Replicate prediction failed (id={}): {}",
                    prediction_id,
                    crate::truncate_utf8(&describe_error(prediction.error.as_ref()), 300)
                );
            }
            Phase::Canceled => {
                anyhow::bail!("Replicate prediction canceled (id={})", prediction_id);
            }
            Phase::Pending => {}
        }

        if Instant::now() >= deadline {
            anyhow::bail!(
                "Replicate prediction timed out after {}s (id={}, status={})",
                params.timeout_secs,
                prediction_id,
                prediction.status.as_deref().unwrap_or("unknown")
            );
        }

        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
        poll_interval_ms = (poll_interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);

        crate::security::ssrf::check_url(&poll_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;
        let poll_resp = client
            .get(&poll_url)
            .header("Authorization", format!("Bearer {}", params.api_key))
            .send()
            .await?;

        let poll_status = poll_resp.status();
        if !poll_status.is_success() {
            let text = poll_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Replicate prediction poll failed ({}, id={}): {}",
                poll_status.as_u16(),
                prediction_id,
                crate::truncate_utf8(&text, 300)
            );
        }
        prediction = poll_resp.json().await?;
    }

    let urls = collect_output_urls(prediction.output.as_ref());
    if urls.is_empty() {
        anyhow::bail!(
            "Replicate prediction succeeded but returned no image URL (id={})",
            prediction_id
        );
    }

    app_info!(
        "tool",
        "image_generate",
        "Replicate prediction succeeded in {}ms: id={}, outputs={}",
        started.elapsed().as_millis() as u64,
        prediction_id,
        urls.len()
    );

    // Result files are deleted an hour after completion — download now.
    let mut images = Vec::with_capacity(urls.len());
    for url in &urls {
        let (data, mime) =
            crate::media_gen::adapters::fetch::fetch_asset(&client, url, params.ssrf, "image/webp")
                .await?;
        images.push(GeneratedImage {
            data,
            mime,
            revised_prompt: None,
        });
    }

    Ok(ImageGenResult { images, text: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn routes_official_versioned_and_bare_hash() {
        assert_eq!(
            resolve_route("black-forest-labs/flux-2-pro").unwrap(),
            Route::Official {
                owner: "black-forest-labs".into(),
                name: "flux-2-pro".into()
            }
        );
        let hash = "a".repeat(64);
        assert_eq!(
            resolve_route(&format!("some-owner/some-model:{hash}")).unwrap(),
            Route::Version { hash: hash.clone() }
        );
        assert_eq!(
            resolve_route(&hash).unwrap(),
            Route::Version { hash: hash.clone() }
        );
        assert!(resolve_route("flux-2-pro").is_err());
        assert!(resolve_route("owner/").is_err());
        assert!(resolve_route("owner/name:").is_err());
        assert!(resolve_route("  ").is_err());
    }

    #[test]
    fn input_carries_prompt_aspect_ratio_and_typed_extra() {
        let mut extra = HashMap::new();
        extra.insert("num_inference_steps".to_string(), "28".to_string());
        extra.insert("guidance".to_string(), "3.5".to_string());
        extra.insert("go_fast".to_string(), "true".to_string());
        extra.insert("output_format".to_string(), "png".to_string());

        let input = build_input("a cat", Some("16:9"), &extra);
        assert_eq!(input["prompt"], Value::String("a cat".into()));
        assert_eq!(input["aspect_ratio"], Value::String("16:9".into()));
        assert_eq!(input["num_inference_steps"], Value::from(28));
        assert_eq!(input["guidance"], Value::from(3.5));
        assert_eq!(input["go_fast"], Value::Bool(true));
        assert_eq!(input["output_format"], Value::String("png".into()));

        // No aspect ratio hint → field omitted entirely, never sent empty.
        let bare = build_input("a cat", None, &HashMap::new());
        assert!(bare.get("aspect_ratio").is_none());

        // extra overrides the adapter default rather than duplicating it.
        let mut override_ar = HashMap::new();
        override_ar.insert("aspect_ratio".to_string(), "1:1".to_string());
        let overridden = build_input("a cat", Some("16:9"), &override_ar);
        assert_eq!(overridden["aspect_ratio"], Value::String("1:1".into()));
    }

    #[test]
    fn unknown_status_stays_pending() {
        assert_eq!(classify_status("succeeded"), Phase::Succeeded);
        assert_eq!(classify_status("failed"), Phase::Failed);
        assert_eq!(classify_status("canceled"), Phase::Canceled);
        assert_eq!(classify_status("starting"), Phase::Pending);
        assert_eq!(classify_status("processing"), Phase::Pending);
        assert_eq!(classify_status(""), Phase::Pending);
        assert_eq!(classify_status("some_new_state"), Phase::Pending);
    }

    #[test]
    fn output_accepts_string_and_array_shapes() {
        assert_eq!(
            collect_output_urls(Some(&serde_json::json!("https://x/1.webp"))),
            vec!["https://x/1.webp".to_string()]
        );
        assert_eq!(
            collect_output_urls(Some(&serde_json::json!([
                "https://x/1.webp",
                "https://x/2.webp"
            ]))),
            vec![
                "https://x/1.webp".to_string(),
                "https://x/2.webp".to_string()
            ]
        );
        // Non-string array members (some models interleave logs/objects) are
        // skipped instead of aborting the whole result.
        assert_eq!(
            collect_output_urls(Some(&serde_json::json!(["https://x/1.webp", 42, null]))),
            vec!["https://x/1.webp".to_string()]
        );
        assert!(collect_output_urls(None).is_empty());
        assert!(collect_output_urls(Some(&Value::Null)).is_empty());
        assert!(collect_output_urls(Some(&serde_json::json!(""))).is_empty());
    }

    #[test]
    fn prefer_wait_stays_within_replicate_range_and_under_client_timeout() {
        assert_eq!(prefer_wait_secs(0), 1);
        // Must stay strictly below the caller's timeout: at parity the server
        // holds the connection for the whole window and reqwest aborts at the
        // same instant, so the polling fallback never runs.
        assert!(prefer_wait_secs(30) < 30);
        assert_eq!(prefer_wait_secs(30), 25);
        // Replicate caps the header at 60s regardless of our budget.
        assert_eq!(prefer_wait_secs(600), 60);
    }
}

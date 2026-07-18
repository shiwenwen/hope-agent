//! Black Forest Labs FLUX (`api.bfl.ai`).
//!
//! Three things make this vendor unlike the OpenAI-shaped adapters:
//! the model lives in the URL path (no `model` body field), submission is
//! async (`{id, polling_url}` → poll until `Ready`), and the dimension
//! parameter forks by model family — FLUX.2 / `flux-pro-1.1` take
//! `width`/`height` while Ultra and Kontext take `aspect_ratio` and reject
//! width/height outright.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::media_gen::adapters::fetch::fetch_asset;
use crate::media_gen::adapters::{
    GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult, InputImage,
};

const DEFAULT_BASE_URL: &str = "https://api.bfl.ai";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const POLL_START_MS: u64 = 1000;
const POLL_STEP_MS: u64 = 1000;
const POLL_MAX_MS: u64 = 3000;
/// Smallest dimension the API accepts for the `width`/`height` family.
const MIN_DIMENSION: u32 = 64;
/// `aspect_ratio` is documented as legal between 21:9 and 9:21.
const MIN_ASPECT: f64 = 9.0 / 21.0;
const MAX_ASPECT: f64 = 21.0 / 9.0;

pub(crate) struct Provider;

impl ImageGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

// ── Pure model-family policy ──────────────────────────────────────

/// Which dimension parameter a model slug accepts. Sending the wrong one is
/// a server-side 422, so this is the single source of truth for the fork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeMode {
    WidthHeight,
    AspectRatio,
}

/// `flux-pro-1.1-ultra`, `flux-kontext-pro` and `flux-kontext-max` take
/// `aspect_ratio`; everything else documented (FLUX.2 family, plain
/// `flux-pro-1.1`) takes `width`/`height`, which is also the fallback for
/// slugs outside the documented table.
fn size_mode(model: &str) -> SizeMode {
    let slug = model.trim().to_ascii_lowercase();
    // Matches the `-finetuned` variants of both families too.
    if slug.contains("kontext") || slug.contains("ultra") {
        SizeMode::AspectRatio
    } else {
        SizeMode::WidthHeight
    }
}

/// Reference-image slots the family exposes (`input_image` .. `input_image_N`).
/// `0` means the slug has no documented reference-image input.
fn max_reference_images(model: &str) -> usize {
    let slug = model.trim().to_ascii_lowercase();
    if slug.starts_with("flux-2") {
        8
    } else if slug.contains("kontext") {
        4
    } else {
        0
    }
}

/// Parse the shared `"WxH"` size string into pixel dimensions.
fn parse_size(size: &str) -> Option<(u32, u32)> {
    let lowered = size.trim().to_ascii_lowercase();
    let (w, h) = lowered.split_once('x')?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    (w >= MIN_DIMENSION && h >= MIN_DIMENSION).then_some((w, h))
}

/// Reduce pixel dimensions to an `"W:H"` ratio for the aspect-ratio family.
/// Returns `None` outside the documented 21:9..9:21 band so the server
/// default applies rather than a rejected value.
fn derive_aspect_ratio(width: u32, height: u32) -> Option<String> {
    if width == 0 || height == 0 {
        return None;
    }
    let ratio = f64::from(width) / f64::from(height);
    if !(MIN_ASPECT..=MAX_ASPECT).contains(&ratio) {
        return None;
    }
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(width, height).max(1);
    Some(format!("{}:{}", width / g, height / g))
}

/// What the poll loop should do with a `status` value. BFL's terminal
/// non-success states (moderation, error, unknown task) must surface as
/// errors instead of spinning until the deadline.
#[derive(Debug, PartialEq, Eq)]
enum PollOutcome {
    Ready,
    Pending,
    Failed(&'static str),
}

fn classify_status(status: &str) -> PollOutcome {
    match status.trim() {
        "Ready" => PollOutcome::Ready,
        "Pending" => PollOutcome::Pending,
        "Request Moderated" => PollOutcome::Failed("request was moderated (prompt rejected)"),
        "Content Moderated" => PollOutcome::Failed("generated content was moderated"),
        "Error" => PollOutcome::Failed("generation failed"),
        "Task not found" => PollOutcome::Failed("task not found (expired or wrong region)"),
        // Unknown states are treated as still-running; the deadline bounds it.
        _ => PollOutcome::Pending,
    }
}

fn json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

/// Build the submission body. `model` is *not* in here — it is the URL path.
fn build_body(
    model: &str,
    prompt: &str,
    size: &str,
    aspect_ratio: Option<&str>,
    input_images: &[InputImage],
    extra: &HashMap<String, String>,
) -> Value {
    let mut body = Map::new();
    body.insert("prompt".into(), json!(prompt));

    let dims = parse_size(size);
    match size_mode(model) {
        SizeMode::WidthHeight => {
            if let Some((w, h)) = dims {
                body.insert("width".into(), json!(w));
                body.insert("height".into(), json!(h));
            }
        }
        SizeMode::AspectRatio => {
            let ratio = aspect_ratio
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| dims.and_then(|(w, h)| derive_aspect_ratio(w, h)));
            if let Some(ratio) = ratio {
                body.insert("aspect_ratio".into(), json!(ratio));
            }
        }
    }

    // Raw base64, no `data:` prefix — the API rejects data URIs here.
    let cap = max_reference_images(model);
    for (i, img) in input_images.iter().take(cap).enumerate() {
        let field = if i == 0 {
            "input_image".to_string()
        } else {
            format!("input_image_{}", i + 1)
        };
        body.insert(
            field,
            json!(base64::engine::general_purpose::STANDARD.encode(&img.data)),
        );
    }

    // Escape hatch for documented-but-unmodeled knobs (seed, output_format,
    // prompt_upsampling, safety_tolerance, …); user config wins.
    for (k, v) in extra {
        body.insert(k.clone(), json_or_string(v));
    }

    Value::Object(body)
}

// ── Wire types ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SubmitResponse {
    id: Option<String>,
    polling_url: Option<String>,
}

#[derive(Deserialize)]
struct PollResponse {
    status: Option<String>,
    result: Option<Value>,
    details: Option<Value>,
}

// ── Execution ─────────────────────────────────────────────────────

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let model = params.model.trim().trim_start_matches('/');
    if model.is_empty() {
        bail!("BFL FLUX: model slug is required (it forms the request path)");
    }
    let submit_url = format!("{}/v1/{}", base, model);

    if params.n > 1 {
        app_warn!(
            "tool",
            "image_generate",
            "BFL FLUX has no batch parameter; returning 1 image instead of {}",
            params.n
        );
    }
    let cap = max_reference_images(model);
    if !params.input_images.is_empty() && params.input_images.len() > cap {
        app_warn!(
            "tool",
            "image_generate",
            "BFL FLUX model {} accepts at most {} reference image(s); dropping {}",
            model,
            cap,
            params.input_images.len() - cap
        );
    }

    let body = build_body(
        model,
        params.prompt,
        params.size,
        params.aspect_ratio,
        params.input_images,
        params.extra,
    );

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(&submit_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    let started = Instant::now();
    let resp = client
        .post(&submit_url)
        .header("x-key", params.api_key)
        .header("accept", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "BFL FLUX submit failed ({}): {}",
            status,
            crate::truncate_utf8(&text, 300)
        );
    }

    let submit: SubmitResponse = resp.json().await?;
    let task_id = submit.id.unwrap_or_default();
    // Must poll the URL the server handed back: it is pinned to the region
    // node that owns the task, so a locally rebuilt path 404s.
    let polling_url = submit
        .polling_url
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("BFL FLUX: submit response had no polling_url"))?;
    // Server-controlled URL, therefore untrusted — re-gate before hitting it.
    crate::security::ssrf::check_url(&polling_url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    app_info!(
        "tool",
        "image_generate",
        "BFL FLUX task submitted: model={}, id={}",
        model,
        task_id
    );

    // Poll budget starts *after* submit: the upload can consume most of the
    // caller's timeout (BFL takes up to 8 base64 reference images), and
    // anchoring on the pre-submit instant would abandon an already-billed
    // task without ever polling once.
    let deadline = Instant::now() + Duration::from_secs(params.timeout_secs);
    let mut interval_ms = POLL_START_MS;

    loop {
        if Instant::now() >= deadline {
            bail!(
                "BFL FLUX task timed out after {}s (id={})",
                params.timeout_secs,
                task_id
            );
        }
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;

        let poll = client
            .get(&polling_url)
            .header("x-key", params.api_key)
            .header("accept", "application/json")
            .send()
            .await?;
        let poll_status = poll.status();
        if !poll_status.is_success() {
            let text = poll.text().await.unwrap_or_default();
            // Polling is a read-only probe against an already-submitted,
            // already-billed task. Rate limits and 5xx blips are expected, so
            // keep probing until the deadline instead of throwing the result
            // away; only genuine client errors (bad key, unknown id) are
            // terminal.
            let transient = poll_status.as_u16() == 408
                || poll_status.as_u16() == 429
                || poll_status.is_server_error();
            if transient {
                app_warn!(
                    "tool",
                    "image_generate",
                    "BFL FLUX poll transient failure ({}, id={}), retrying: {}",
                    poll_status,
                    task_id,
                    crate::truncate_utf8(&text, 200)
                );
                continue;
            }
            bail!(
                "BFL FLUX poll failed ({}, id={}): {}",
                poll_status,
                task_id,
                crate::truncate_utf8(&text, 300)
            );
        }

        let parsed: PollResponse = poll.json().await?;
        let state = parsed.status.as_deref().unwrap_or("");

        match classify_status(state) {
            PollOutcome::Ready => {
                let sample = parsed
                    .result
                    .as_ref()
                    .and_then(|r| r.get("sample"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("BFL FLUX: Ready but result.sample missing (id={task_id})")
                    })?;

                app_info!(
                    "tool",
                    "image_generate",
                    "BFL FLUX task ready in {}ms (id={})",
                    started.elapsed().as_millis() as u64,
                    task_id
                );

                // The delivery URL expires ~10 minutes after completion, so
                // download it immediately rather than handing the link on.
                let fallback_mime = params
                    .extra
                    .get("output_format")
                    .map(|f| format!("image/{}", f.trim().to_ascii_lowercase()))
                    .unwrap_or_else(|| "image/png".to_string());
                let (data, mime) =
                    fetch_asset(&client, sample, params.ssrf, &fallback_mime).await?;
                if data.is_empty() {
                    bail!("BFL FLUX: downloaded image was empty (id={task_id})");
                }

                return Ok(ImageGenResult {
                    images: vec![GeneratedImage {
                        data,
                        mime,
                        revised_prompt: None,
                    }],
                    text: None,
                });
            }
            PollOutcome::Failed(reason) => {
                let detail = parsed
                    .details
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "{}".to_string());
                bail!(
                    "BFL FLUX task ended in '{}' — {} (id={}): {}",
                    state,
                    reason,
                    task_id,
                    crate::truncate_utf8(&detail, 300)
                );
            }
            PollOutcome::Pending => {
                interval_ms = (interval_ms + POLL_STEP_MS).min(POLL_MAX_MS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(bytes: &[u8]) -> InputImage {
        InputImage {
            data: bytes.to_vec(),
            mime: "image/png".into(),
        }
    }

    #[test]
    fn size_mode_forks_by_family() {
        for slug in [
            "flux-2-pro",
            "flux-2-flex",
            "flux-2-klein-9b",
            "flux-pro-1.1",
        ] {
            assert_eq!(size_mode(slug), SizeMode::WidthHeight, "{slug}");
        }
        for slug in [
            "flux-pro-1.1-ultra",
            "flux-pro-1.1-ultra-finetuned",
            "flux-kontext-pro",
            "flux-kontext-max",
        ] {
            assert_eq!(size_mode(slug), SizeMode::AspectRatio, "{slug}");
        }
    }

    #[test]
    fn reference_image_caps_match_family() {
        assert_eq!(max_reference_images("flux-2-pro"), 8);
        assert_eq!(max_reference_images("flux-kontext-max"), 4);
        assert_eq!(max_reference_images("flux-pro-1.1"), 0);
    }

    #[test]
    fn width_height_body_omits_aspect_ratio_and_numbers_ref_slots() {
        let extra = HashMap::new();
        let images = vec![img(b"a"), img(b"b")];
        let body = build_body(
            "flux-2-pro",
            "a cat",
            "1024x768",
            Some("16:9"),
            &images,
            &extra,
        );
        assert_eq!(body["width"], json!(1024));
        assert_eq!(body["height"], json!(768));
        assert!(body.get("aspect_ratio").is_none());
        assert!(body.get("model").is_none());
        assert_eq!(body["input_image"], json!("YQ=="));
        assert_eq!(body["input_image_2"], json!("Yg=="));
    }

    #[test]
    fn aspect_ratio_body_omits_dimensions_and_caps_refs() {
        let extra = HashMap::new();
        let images = vec![img(b"a"), img(b"b"), img(b"c"), img(b"d"), img(b"e")];
        let body = build_body(
            "flux-kontext-pro",
            "a cat",
            "1920x1080",
            None,
            &images,
            &extra,
        );
        assert!(body.get("width").is_none());
        assert!(body.get("height").is_none());
        // Derived from size when no explicit ratio was passed.
        assert_eq!(body["aspect_ratio"], json!("16:9"));
        // 5th reference image is beyond the kontext cap of 4.
        assert!(body.get("input_image_5").is_none());
        assert_eq!(body["input_image_4"], json!("ZA=="));
    }

    #[test]
    fn extra_knobs_land_top_level_with_typed_values() {
        let extra = HashMap::from([
            ("seed".to_string(), "42".to_string()),
            ("output_format".to_string(), "jpeg".to_string()),
            ("prompt_upsampling".to_string(), "true".to_string()),
        ]);
        let body = build_body("flux-2-pro", "x", "1024x1024", None, &[], &extra);
        assert_eq!(body["seed"], json!(42));
        assert_eq!(body["output_format"], json!("jpeg"));
        assert_eq!(body["prompt_upsampling"], json!(true));
    }

    #[test]
    fn status_classification_separates_terminal_from_pending() {
        assert_eq!(classify_status("Ready"), PollOutcome::Ready);
        assert_eq!(classify_status("Pending"), PollOutcome::Pending);
        assert!(matches!(
            classify_status("Content Moderated"),
            PollOutcome::Failed(_)
        ));
        assert!(matches!(classify_status("Error"), PollOutcome::Failed(_)));
        // Unknown states keep polling until the deadline rather than erroring.
        assert_eq!(classify_status("Queued"), PollOutcome::Pending);
    }

    #[test]
    fn size_and_ratio_helpers_reject_out_of_band_values() {
        assert_eq!(parse_size("1024x768"), Some((1024, 768)));
        assert_eq!(parse_size("32x32"), None); // below the 64px minimum
        assert_eq!(parse_size("junk"), None);
        assert_eq!(derive_aspect_ratio(1000, 1000).as_deref(), Some("1:1"));
        // 30:9 is wider than the documented 21:9 limit.
        assert_eq!(derive_aspect_ratio(3000, 900), None);
    }
}

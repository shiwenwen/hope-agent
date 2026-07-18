//! Profile-driven adapter for the "OpenAI-ish JSON images" vendor family.
//!
//! A growing set of vendors speak `POST {base}/v1/images/generations` with a
//! JSON body that is *nearly* OpenAI's — each deviating in one or two spots
//! (size encoding, whether `n` exists, which `response_format` token they
//! accept, where the bytes come back). Rather than one near-duplicate
//! adapter per vendor, the deviations are data: a [`CompatProfile`] per
//! vendor, one shared request/parse path.
//!
//! Keep new vendors here **only while their deviation fits a profile field**.
//! Async task polling, multipart bodies, non-Bearer auth or bespoke result
//! envelopes belong in their own adapter — bending this one to fit them is
//! how it turns back into the tangle it replaced.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use base64::Engine;
use reqwest::Client;
use serde_json::{json, Map, Value};

use crate::media_gen::adapters::fetch::fetch_asset;
use crate::media_gen::adapters::{
    GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult, InputImage,
};

/// How the vendor wants the output dimensions expressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeStyle {
    /// OpenAI's `"1024x1024"`, passed through verbatim. Also covers vendors
    /// that additionally accept tier tokens like `"2K"` — the catalog's
    /// declared sizes decide which form actually reaches us.
    Pixels,
    /// Tencent TokenHub wants `"1024:1024"`.
    Colon,
    /// Vendor has no size parameter at all (xAI sizes via aspect ratio).
    Omit,
    /// Separate `width` / `height` integer fields (Together AI).
    WidthHeight,
}

/// One vendor's deviations from the OpenAI images wire shape.
#[derive(Debug, Clone)]
pub struct CompatProfile {
    /// Used in error messages and log sources.
    pub vendor: &'static str,
    pub path: &'static str,
    /// Path used when reference images are present. `None` = the vendor edits
    /// on the same endpoint (distinguished only by the image field).
    pub edit_path: Option<&'static str>,
    /// StepFun's edit endpoint derives output size from the input image and
    /// rejects an explicit `size`.
    pub edit_omits_size: bool,
    pub size_style: SizeStyle,
    /// False for vendors with no batch parameter (Ark drives batches through
    /// `sequential_image_generation` instead).
    pub send_n: bool,
    /// Token this vendor accepts for `response_format`. `None` = omit the
    /// field entirely (vendors that only ever return URLs).
    pub response_format: Option<&'static str>,
    pub send_aspect_ratio: bool,
    pub send_resolution: bool,
    /// Body field carrying reference images for edit/img2img, if supported.
    pub input_image_field: Option<&'static str>,
    /// Whether that field takes an array (vs. a single data-URI string).
    pub input_image_array: bool,
    /// Vendors that accept only a fixed set of pixel buckets (SenseNova
    /// fails the task server-side for anything else). When non-empty, a
    /// requested size outside the list is replaced by the first entry —
    /// the global default size is not one of these buckets, so without
    /// this every default-config request would 400.
    pub size_allowlist: &'static [&'static str],
    /// Constant body fields appended last (e.g. Ark's `watermark: false`).
    /// Values are JSON source: `"false"` becomes a bool, `"png"` stays a
    /// string. Kept as `&str` so profiles remain const-constructible.
    pub extra_body: &'static [(&'static str, &'static str)],
}

impl CompatProfile {
    fn fallback_mime(&self) -> &'static str {
        "image/png"
    }
}

pub(crate) struct CompatProvider(pub CompatProfile);

impl ImageGenAdapter for CompatProvider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(&self.0, params))
    }
}

fn data_uri(img: &InputImage) -> String {
    let mime = if img.mime.is_empty() {
        "image/png"
    } else {
        &img.mime
    };
    format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&img.data)
    )
}

/// `"false"` / `"7"` / `"[1,2]"` parse as JSON; anything else stays a string.
fn json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

fn build_body(profile: &CompatProfile, params: &ImageGenParams<'_>) -> Value {
    let editing = !params.input_images.is_empty();
    let mut body = Map::new();
    body.insert("model".into(), json!(params.model));
    body.insert("prompt".into(), json!(params.prompt));

    let size_style = if editing && profile.edit_omits_size {
        SizeStyle::Omit
    } else {
        profile.size_style
    };
    let size = if profile.size_allowlist.is_empty() || profile.size_allowlist.contains(&params.size)
    {
        params.size
    } else {
        profile.size_allowlist[0]
    };
    match size_style {
        SizeStyle::Pixels => {
            if !size.is_empty() {
                body.insert("size".into(), json!(size));
            }
        }
        SizeStyle::Colon => {
            if !size.is_empty() {
                body.insert("size".into(), json!(size.replace('x', ":")));
            }
        }
        SizeStyle::Omit => {}
        SizeStyle::WidthHeight => {
            if let Some((w, h)) = size.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    body.insert("width".into(), json!(w));
                    body.insert("height".into(), json!(h));
                }
            }
        }
    }

    if profile.send_n {
        body.insert("n".into(), json!(params.n));
    }
    if let Some(fmt) = profile.response_format {
        body.insert("response_format".into(), json!(fmt));
    }
    if profile.send_aspect_ratio {
        if let Some(ar) = params.aspect_ratio {
            body.insert("aspect_ratio".into(), json!(ar));
        }
    }
    if profile.send_resolution {
        if let Some(res) = params.resolution {
            body.insert("resolution".into(), json!(res.to_lowercase()));
        }
    }

    if let Some(field) = profile.input_image_field {
        if !params.input_images.is_empty() {
            if profile.input_image_array {
                let uris: Vec<Value> = params
                    .input_images
                    .iter()
                    .map(|i| json!(data_uri(i)))
                    .collect();
                body.insert(field.into(), Value::Array(uris));
            } else {
                body.insert(field.into(), json!(data_uri(&params.input_images[0])));
            }
        }
    }

    for (k, v) in profile.extra_body {
        body.insert((*k).into(), json_or_string(v));
    }

    // Vendor knobs from provider/model `extra` win over everything above —
    // that is the documented escape hatch for undocumented body fields.
    for (k, v) in params.extra {
        body.insert(k.clone(), json_or_string(v));
    }

    Value::Object(body)
}

/// Pull images out of whichever envelope the vendor used: OpenAI's
/// `data[].b64_json` / `data[].url`, or SenseNova's top-level `images_urls`.
async fn extract_images(
    profile: &CompatProfile,
    body: &Value,
    params: &ImageGenParams<'_>,
) -> Result<Vec<GeneratedImage>> {
    let mut out = Vec::new();

    if let Some(items) = body.get("data").and_then(|d| d.as_array()) {
        for item in items {
            if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
                out.push(GeneratedImage {
                    data: base64::engine::general_purpose::STANDARD.decode(b64)?,
                    mime: profile.fallback_mime().to_string(),
                    revised_prompt: item
                        .get("revised_prompt")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                });
            } else if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                let (data, mime) = fetch_asset(url, params.ssrf, profile.fallback_mime()).await?;
                out.push(GeneratedImage {
                    data,
                    mime,
                    revised_prompt: item
                        .get("revised_prompt")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                });
            }
        }
    } else if let Some(urls) = body.get("images_urls").and_then(|v| v.as_array()) {
        for url in urls.iter().filter_map(|u| u.as_str()) {
            let (data, mime) = fetch_asset(url, params.ssrf, profile.fallback_mime()).await?;
            out.push(GeneratedImage {
                data,
                mime,
                revised_prompt: None,
            });
        }
    }

    Ok(out)
}

async fn generate_impl(
    profile: &CompatProfile,
    params: ImageGenParams<'_>,
) -> Result<ImageGenResult> {
    let base = params
        .base_url
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/'))
        .ok_or_else(|| anyhow::anyhow!("{}: base URL required", profile.vendor))?;
    let path = match profile.edit_path {
        Some(edit) if !params.input_images.is_empty() => edit,
        _ => profile.path,
    };
    let url = format!("{base}{path}");

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    let body = build_body(profile, &params);
    let started = std::time::Instant::now();

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "tool",
            &format!("media_gen::{}::request", profile.vendor),
            &format!(
                "{} image request: model={}, size={}, n={}, edit={}",
                profile.vendor,
                params.model,
                params.size,
                params.n,
                !params.input_images.is_empty()
            ),
            None,
            None,
            None,
        );
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", params.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        if let Some(logger) = crate::get_logger() {
            logger.log(
                "error",
                "tool",
                &format!("media_gen::{}::error", profile.vendor),
                &format!(
                    "{} image generation failed ({}): {}",
                    profile.vendor,
                    status.as_u16(),
                    crate::truncate_utf8(&text, 500)
                ),
                None,
                None,
                None,
            );
        }
        anyhow::bail!(
            "{} image generation failed ({}): {}",
            profile.vendor,
            status,
            crate::truncate_utf8(&text, 300)
        );
    }

    let payload: Value = resp.json().await?;
    let images = extract_images(profile, &payload, &params).await?;
    if images.is_empty() {
        anyhow::bail!("{} returned no image data", profile.vendor);
    }

    if let Some(logger) = crate::get_logger() {
        logger.log(
            "debug",
            "tool",
            &format!("media_gen::{}::result", profile.vendor),
            &format!(
                "{} returned {} image(s) in {}ms",
                profile.vendor,
                images.len(),
                started.elapsed().as_millis()
            ),
            None,
            None,
            None,
        );
    }

    Ok(ImageGenResult { images, text: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn params<'a>(extra: &'a HashMap<String, String>, size: &'a str) -> ImageGenParams<'a> {
        ImageGenParams {
            api_key: "k",
            base_url: Some("https://example.test"),
            model: "m",
            prompt: "p",
            size,
            n: 2,
            timeout_secs: 30,
            extra,
            aspect_ratio: Some("16:9"),
            resolution: Some("2K"),
            input_images: &[],
            mask: None,
            ssrf: Default::default(),
        }
    }

    fn profile() -> CompatProfile {
        CompatProfile {
            vendor: "test",
            path: "/v1/images/generations",
            edit_path: None,
            edit_omits_size: false,
            size_style: SizeStyle::Pixels,
            send_n: true,
            response_format: Some("b64_json"),
            send_aspect_ratio: false,
            send_resolution: false,
            input_image_field: None,
            input_image_array: false,
            size_allowlist: &[],
            extra_body: &[],
        }
    }

    #[test]
    fn colon_style_rewrites_size_separator() {
        let extra = HashMap::new();
        let p = CompatProfile {
            size_style: SizeStyle::Colon,
            ..profile()
        };
        let body = build_body(&p, &params(&extra, "1024x1024"));
        assert_eq!(body["size"], json!("1024:1024"));
    }

    #[test]
    fn omit_style_drops_size_and_send_n_false_drops_n() {
        let extra = HashMap::new();
        let p = CompatProfile {
            size_style: SizeStyle::Omit,
            send_n: false,
            send_aspect_ratio: true,
            ..profile()
        };
        let body = build_body(&p, &params(&extra, "1024x1024"));
        assert!(body.get("size").is_none());
        assert!(body.get("n").is_none());
        assert_eq!(body["aspect_ratio"], json!("16:9"));
    }

    #[test]
    fn response_format_token_is_per_vendor() {
        let extra = HashMap::new();
        let p = CompatProfile {
            // Together AI rejects OpenAI's `b64_json` here.
            response_format: Some("base64"),
            ..profile()
        };
        let body = build_body(&p, &params(&extra, "1024x1024"));
        assert_eq!(body["response_format"], json!("base64"));

        let p = CompatProfile {
            response_format: None,
            ..profile()
        };
        let body = build_body(&p, &params(&extra, "1024x1024"));
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn size_outside_allowlist_snaps_to_first_bucket() {
        let extra = HashMap::new();
        let p = CompatProfile {
            size_allowlist: &["1792x992", "1344x1344"],
            ..profile()
        };
        // The global default size is not one of the vendor's buckets.
        let body = build_body(&p, &params(&extra, "1024x1024"));
        assert_eq!(body["size"], json!("1792x992"));
        // An explicitly supported bucket passes through untouched.
        let body = build_body(&p, &params(&extra, "1344x1344"));
        assert_eq!(body["size"], json!("1344x1344"));
    }

    #[test]
    fn extra_body_constants_apply_and_user_extra_wins() {
        let mut extra = HashMap::new();
        extra.insert("watermark".to_string(), "true".to_string());
        let p = CompatProfile {
            extra_body: &[("watermark", "false")],
            ..profile()
        };
        let body = build_body(&p, &params(&extra, "1024x1024"));
        // provider/model `extra` is the documented escape hatch: it wins.
        assert_eq!(body["watermark"], json!(true));
    }
}

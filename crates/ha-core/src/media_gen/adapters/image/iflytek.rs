//! iFlytek Spark image generation (`/v2.1/tti`).
//!
//! Zero OpenAI reuse: HMAC-SHA256 request signing folded into the URL query,
//! plus a three-section `header`/`parameter`/`payload` body. The endpoint is
//! **synchronous** — one POST returns the finished base64 image, so there is
//! no submit/poll split like `tongyi.rs`.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Deserialize;
use sha2::Sha256;

use crate::media_gen::adapters::{GeneratedImage, ImageGenAdapter, ImageGenParams, ImageGenResult};

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_BASE_URL: &str = "https://spark-api.cn-huabei-1.xf-yun.com";
const TTI_PATH: &str = "/v2.1/tti";
const DEFAULT_DOMAIN: &str = "general";
const DEFAULT_SIZE: (u32, u32) = (1024, 1024);
const MAX_PROMPT_CHARS: usize = 1000;

/// The only dimension pairs the service accepts; anything else is a
/// server-side rejection, so out-of-range requests are snapped to the nearest
/// legal pair rather than forwarded verbatim.
const LEGAL_SIZES: &[(u32, u32)] = &[
    (512, 512),
    (640, 360),
    (640, 480),
    (640, 640),
    (680, 512),
    (512, 680),
    (768, 768),
    (720, 1280),
    (1280, 720),
    (1024, 1024),
];

// ── Response types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct TtiResponse {
    header: Option<TtiHeader>,
    payload: Option<TtiPayload>,
}

#[derive(Deserialize)]
struct TtiHeader {
    code: Option<i64>,
    message: Option<String>,
    sid: Option<String>,
}

#[derive(Deserialize)]
struct TtiPayload {
    choices: Option<TtiChoices>,
}

#[derive(Deserialize)]
struct TtiChoices {
    text: Option<Vec<TtiText>>,
}

#[derive(Deserialize)]
struct TtiText {
    content: Option<String>,
}

pub(crate) struct Provider;

impl ImageGenAdapter for Provider {
    fn generate<'a>(
        &'a self,
        params: ImageGenParams<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ImageGenResult>> + Send + 'a>> {
        Box::pin(generate_impl(params))
    }
}

// ── Credentials ───────────────────────────────────────────────────

struct Credentials {
    app_id: String,
    api_key: String,
    api_secret: String,
}

/// iFlytek issues a three-part credential; the media-gen config surface only
/// carries one `api_key` string, so the three are packed colon-separated.
fn parse_credentials(raw: &str) -> Result<Credentials> {
    let parts: Vec<&str> = raw.trim().split(':').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.trim().is_empty()) {
        anyhow::bail!(
            "iFlytek requires an API key formatted as `APPID:APIKey:APISecret` \
             (three non-empty colon-separated parts)"
        );
    }
    Ok(Credentials {
        app_id: parts[0].trim().to_string(),
        api_key: parts[1].trim().to_string(),
        api_secret: parts[2].trim().to_string(),
    })
}

// ── Signing ───────────────────────────────────────────────────────

/// RFC1123 in UTC, e.g. `Mon, 01 Jan 2026 00:00:00 GMT`. The server tolerates
/// a ±300s skew and rejects anything else, so this must be real UTC — not
/// local time relabelled GMT.
fn rfc1123_now() -> String {
    chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// Base64 of `HMAC-SHA256(api_secret, "host: …\ndate: …\n{method} {path} HTTP/1.1")`.
///
/// `method` is the *actual* HTTP verb of the request being signed (POST for
/// `/v2.1/tti`); signing "GET" against a POST call fails authentication.
fn sign(api_secret: &str, host: &str, date: &str, method: &str, path: &str) -> Result<String> {
    let origin = format!("host: {host}\ndate: {date}\n{method} {path} HTTP/1.1");
    let mut mac = <HmacSha256 as Mac>::new_from_slice(api_secret.as_bytes())
        .map_err(|_| anyhow::anyhow!("iFlytek: invalid APISecret"))?;
    mac.update(origin.as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

/// The `authorization` query value: a base64 wrapper around the signature and
/// the metadata the server needs to recompute it.
fn authorization_header(api_key: &str, signature: &str) -> String {
    let origin = format!(
        "api_key=\"{api_key}\", algorithm=\"hmac-sha256\", headers=\"host date request-line\", signature=\"{signature}\""
    );
    base64::engine::general_purpose::STANDARD.encode(origin)
}

/// Full signed request URL. Credentials ride in the query string, not headers.
fn signed_url(
    scheme: &str,
    host: &str,
    path: &str,
    date: &str,
    method: &str,
    api_key: &str,
    api_secret: &str,
) -> Result<String> {
    let signature = sign(api_secret, host, date, method, path)?;
    let authorization = authorization_header(api_key, &signature);
    Ok(format!(
        "{scheme}://{host}{path}?authorization={}&date={}&host={}",
        urlencoding::encode(&authorization),
        urlencoding::encode(date),
        urlencoding::encode(host),
    ))
}

/// Split a configured base URL into `(scheme, host)`. `host` keeps an explicit
/// port because the signature must match the `host` query value byte for byte.
fn split_base(base: &str) -> Result<(String, String)> {
    let url = reqwest::Url::parse(base).with_context(|| {
        format!(
            "iFlytek: invalid base URL {}",
            crate::truncate_utf8(base, 200)
        )
    })?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("iFlytek: base URL has no host"))?;
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Ok((url.scheme().to_string(), host))
}

// ── Request shaping ───────────────────────────────────────────────

/// Snap a requested `WxH` to the nearest legal pair (Euclidean on dimensions;
/// ties resolved by `LEGAL_SIZES` order). Unparseable input falls back to the
/// default rather than failing the call.
fn resolve_size(requested: &str) -> (u32, u32) {
    let Some((w, h)) = parse_size(requested) else {
        return DEFAULT_SIZE;
    };
    if LEGAL_SIZES.contains(&(w, h)) {
        return (w, h);
    }
    LEGAL_SIZES
        .iter()
        .copied()
        .min_by_key(|&(lw, lh)| {
            let dw = lw as i64 - w as i64;
            let dh = lh as i64 - h as i64;
            dw * dw + dh * dh
        })
        .unwrap_or(DEFAULT_SIZE)
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let (w, h) = s.split_once(['x', 'X', '*'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn build_body(
    app_id: &str,
    domain: &str,
    prompt: &str,
    width: u32,
    height: u32,
) -> serde_json::Value {
    serde_json::json!({
        "header": { "app_id": app_id },
        "parameter": {
            "chat": { "domain": domain, "width": width, "height": height }
        },
        "payload": {
            "message": {
                "text": [ { "role": "user", "content": prompt } ]
            }
        }
    })
}

/// `header.code == 0` is the only success value; the transport status can be
/// 200 while the body carries a business error.
fn check_header(header: Option<&TtiHeader>) -> Result<()> {
    let Some(header) = header else {
        anyhow::bail!("iFlytek: response is missing `header`");
    };
    let code = header.code.unwrap_or(-1);
    if code == 0 {
        return Ok(());
    }
    anyhow::bail!(
        "iFlytek image generation failed (code={code}, sid={}): {}",
        header.sid.as_deref().unwrap_or("-"),
        header.message.as_deref().unwrap_or("no message")
    )
}

// ── Execution ─────────────────────────────────────────────────────

async fn generate_impl(params: ImageGenParams<'_>) -> Result<ImageGenResult> {
    if !params.input_images.is_empty() || params.mask.is_some() {
        anyhow::bail!("iFlytek Spark image generation supports neither img2img nor masks");
    }

    let creds = parse_credentials(params.api_key)?;

    let base = params
        .base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/');
    let (scheme, host) = split_base(base)?;

    let (width, height) = resolve_size(params.size);
    if !params.size.is_empty() && parse_size(params.size) != Some((width, height)) {
        app_warn!(
            "tool",
            "image_generate",
            "iFlytek: size {} is not in the accepted set; using {}x{}",
            params.size,
            width,
            height
        );
    }
    if params.n > 1 {
        app_warn!(
            "tool",
            "image_generate",
            "iFlytek returns one image per call; ignoring n={}",
            params.n
        );
    }

    // Character-bounded, not byte-bounded: the documented cap is 1000 字符.
    let prompt: String = params.prompt.chars().take(MAX_PROMPT_CHARS).collect();
    if prompt.chars().count() < params.prompt.chars().count() {
        app_warn!(
            "tool",
            "image_generate",
            "iFlytek: prompt truncated to {} characters",
            MAX_PROMPT_CHARS
        );
    }

    let domain = if params.model.trim().is_empty() {
        DEFAULT_DOMAIN
    } else {
        params.model.trim()
    };
    let body = build_body(&creds.app_id, domain, &prompt, width, height);

    let date = rfc1123_now();
    let url = signed_url(
        &scheme,
        &host,
        TTI_PATH,
        &date,
        "POST",
        &creds.api_key,
        &creds.api_secret,
    )?;

    // SSRF 红线：出站前必过 check_url；策略来自 provider 的 allow_private_network。
    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(&url, params.ssrf, &cfg.ssrf.trusted_hosts).await?;

    app_info!(
        "tool",
        "image_generate",
        "iFlytek Spark tti request: domain={}, size={}x{}, host={}",
        domain,
        width,
        height,
        host
    );

    let client = crate::provider::apply_proxy(
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(params.timeout_secs)),
    )
    .build()?;

    let started = std::time::Instant::now();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        anyhow::bail!(
            "iFlytek image generation failed ({}): {}",
            status,
            crate::truncate_utf8(&text, 300)
        );
    }

    let parsed: TtiResponse = serde_json::from_str(&text).with_context(|| {
        format!(
            "iFlytek: unparseable response ({}): {}",
            status,
            crate::truncate_utf8(&text, 300)
        )
    })?;

    check_header(parsed.header.as_ref())?;

    let contents: Vec<String> = parsed
        .payload
        .and_then(|p| p.choices)
        .and_then(|c| c.text)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| t.content)
        .filter(|c| !c.is_empty())
        .collect();

    if contents.is_empty() {
        anyhow::bail!(
            "iFlytek: response carried no image content: {}",
            crate::truncate_utf8(&text, 300)
        );
    }

    let mut images = Vec::with_capacity(contents.len());
    for content in contents {
        let data = base64::engine::general_purpose::STANDARD
            .decode(content.as_bytes())
            .context("iFlytek: image content is not valid base64")?;
        images.push(GeneratedImage {
            data,
            // Spark tti returns JPEG; there is no format selector.
            mime: "image/jpeg".to_string(),
            revised_prompt: None,
        });
    }

    app_info!(
        "tool",
        "image_generate",
        "iFlytek Spark returned {} image(s) in {}ms",
        images.len(),
        started.elapsed().as_millis() as u64
    );

    Ok(ImageGenResult { images, text: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATE: &str = "Mon, 01 Jan 2026 00:00:00 GMT";
    const HOST: &str = "spark-api.cn-huabei-1.xf-yun.com";

    #[test]
    fn signature_is_stable_for_fixed_inputs() {
        // Reference values computed independently via
        // `openssl dgst -sha256 -hmac secret123 | base64`.
        let sig = sign("secret123", HOST, DATE, "POST", TTI_PATH).unwrap();
        assert_eq!(sig, "Aj+qXPUnBYrvh3rPbEfuZc3H8U+S2fWs9SUNZDkp6Fk=");

        let auth = authorization_header("key456", &sig);
        assert_eq!(
            auth,
            "YXBpX2tleT0ia2V5NDU2IiwgYWxnb3JpdGhtPSJobWFjLXNoYTI1NiIsIGhlYWRlcnM9Imhvc3QgZGF0ZSByZXF1ZXN0LWxpbmUiLCBzaWduYXR1cmU9IkFqK3FYUFVuQllydmgzclBiRWZ1WmMzSDhVK1MyZldzOVNVTlpEa3A2Rms9Ig=="
        );
    }

    #[test]
    fn signed_url_escapes_credentials_into_query() {
        let url = signed_url("https", HOST, TTI_PATH, DATE, "POST", "key456", "secret123").unwrap();
        assert!(url.starts_with("https://spark-api.cn-huabei-1.xf-yun.com/v2.1/tti?authorization="));
        // Base64 padding and RFC1123 spaces must be percent-encoded.
        assert!(url.contains("%3D%3D&date=Mon%2C%2001%20Jan%202026"));
        assert!(url.ends_with("&host=spark-api.cn-huabei-1.xf-yun.com"));
    }

    #[test]
    fn credentials_require_three_parts() {
        let c = parse_credentials(" app:key:secret ").unwrap();
        assert_eq!((c.app_id.as_str(), c.api_key.as_str()), ("app", "key"));
        assert_eq!(c.api_secret, "secret");
        assert!(parse_credentials("app:key").is_err());
        assert!(parse_credentials("app::secret").is_err());
        assert!(parse_credentials("app:key:secret:extra").is_err());
    }

    #[test]
    fn sizes_snap_to_the_nearest_legal_pair() {
        assert_eq!(resolve_size("768x768"), (768, 768));
        assert_eq!(resolve_size("2048x2048"), (1024, 1024));
        assert_eq!(resolve_size("1920x1080"), (1280, 720));
        assert_eq!(resolve_size("500x500"), (512, 512));
        // Unparseable / empty falls back rather than failing the call.
        assert_eq!(resolve_size(""), DEFAULT_SIZE);
        assert_eq!(resolve_size("auto"), DEFAULT_SIZE);
    }

    #[test]
    fn body_uses_the_three_section_shape() {
        let body = build_body("app1", "general", "a cat", 640, 480);
        assert_eq!(body["header"]["app_id"], "app1");
        assert_eq!(body["parameter"]["chat"]["domain"], "general");
        assert_eq!(body["parameter"]["chat"]["width"], 640);
        assert_eq!(body["parameter"]["chat"]["height"], 480);
        assert_eq!(body["payload"]["message"]["text"][0]["role"], "user");
        assert_eq!(body["payload"]["message"]["text"][0]["content"], "a cat");
    }

    #[test]
    fn nonzero_header_code_is_an_error() {
        assert!(check_header(Some(&TtiHeader {
            code: Some(0),
            message: Some("Success".into()),
            sid: None,
        }))
        .is_ok());

        let err = check_header(Some(&TtiHeader {
            code: Some(10013),
            message: Some("input data is illegal".into()),
            sid: Some("tti000".into()),
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("10013"));
        assert!(err.contains("input data is illegal"));

        assert!(check_header(None).is_err());
    }
}

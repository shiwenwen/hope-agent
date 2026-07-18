//! Shared outbound helpers for adapters that fetch provider-returned URLs.
//!
//! Many vendors hand back a CDN link instead of inline base64. That link is
//! **server-controlled data**, not a sub-path of the provider's configured
//! base URL, so the executor's one-shot base-URL SSRF check does not cover
//! it — a hostile or compromised endpoint could point us at loopback or the
//! cloud metadata service. Every download therefore re-gates through
//! `security::ssrf::check_url` here.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::security::ssrf::SsrfPolicy;

const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// Fetch a provider-returned asset URL. Returns `(bytes, mime)`, with `mime`
/// taken from the response `Content-Type` and falling back to `fallback_mime`.
pub async fn fetch_asset(
    client: &Client,
    url: &str,
    ssrf: SsrfPolicy,
    fallback_mime: &str,
) -> Result<(Vec<u8>, String)> {
    let cfg = crate::config::cached_config();
    crate::security::ssrf::check_url(url, ssrf, &cfg.ssrf.trusted_hosts)
        .await
        .with_context(|| format!("blocked asset URL: {}", crate::truncate_utf8(url, 200)))?;

    let resp = client
        .get(url)
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .send()
        .await
        .with_context(|| format!("failed to download {}", crate::truncate_utf8(url, 200)))?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!(
            "asset download failed ({status}): {}",
            crate::truncate_utf8(url, 200)
        );
    }

    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_mime.to_string());

    Ok((resp.bytes().await?.to_vec(), mime))
}

//! Shared outbound helpers for adapters that fetch provider-returned URLs.
//!
//! Many vendors hand back a CDN link instead of inline base64. That link is
//! **server-controlled data**, not a sub-path of the provider's configured
//! base URL, so the executor's one-shot base-URL SSRF check does not cover
//! it — a hostile or compromised endpoint could point us at loopback or the
//! cloud metadata service. Every download therefore re-gates through
//! `security::ssrf::check_url` here.
//!
//! Gating the initial URL alone is not enough: the default reqwest policy
//! follows redirects, so a link that passes the check can still 302 into
//! loopback or the metadata service after the gate has run. The download
//! therefore uses its own client whose redirect policy re-checks every hop —
//! which is also why this takes no caller-supplied `Client`, since the policy
//! can only be set at build time.

use std::time::Duration;

use anyhow::{Context, Result};

use crate::security::ssrf::SsrfPolicy;

const DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const MAX_REDIRECTS: usize = 5;

/// Fetch a provider-returned asset URL. Returns `(bytes, mime)`, with `mime`
/// taken from the response `Content-Type` and falling back to `fallback_mime`.
pub async fn fetch_asset(
    url: &str,
    ssrf: SsrfPolicy,
    fallback_mime: &str,
) -> Result<(Vec<u8>, String)> {
    let cfg = crate::config::cached_config();
    let trusted = cfg.ssrf.trusted_hosts.clone();
    crate::security::ssrf::check_url(url, ssrf, &trusted)
        .await
        .with_context(|| format!("blocked asset URL: {}", crate::truncate_utf8(url, 200)))?;

    let redirect_hosts = trusted.clone();
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if let Some(host) = attempt.url().host_str() {
            if crate::security::ssrf::check_host_blocking_sync(host, ssrf, &redirect_hosts) {
                return attempt.stop();
            }
        }
        attempt.follow()
    });
    let client = crate::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .redirect(redirect_policy),
    )
    .build()
    .context("failed to build asset download client")?;

    let resp = client
        .get(url)
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

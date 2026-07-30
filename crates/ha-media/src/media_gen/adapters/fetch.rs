//! Shared outbound helpers for adapters that fetch provider-returned URLs.
//!
//! Many vendors hand back a CDN link instead of inline base64. That link is
//! **server-controlled data**, not a sub-path of the provider's configured
//! base URL, so the executor's one-shot base-URL SSRF check does not cover
//! it — a hostile or compromised endpoint could point us at loopback or the
//! cloud metadata service. Every download therefore re-gates through
//! `security::ssrf::check_url` here.
//!
//! Gating the initial URL alone is not enough: an auto-following client can
//! be 302-bounced into loopback / metadata after the gate has run, and a
//! hostname-only sync check can't see what DNS resolves to. The client is
//! therefore built with **`redirect::Policy::none()`** and每一跳都显式重跑
//! 异步 `check_url`（含 DNS 解析级判定）后才发起下一跳，跳数封顶
//! `MAX_REDIRECTS`。响应体经 `read_bytes_capped` 流式限量读取——异常或
//! 恶意上游给不设 Content-Length 的超大响应也不能 OOM 本进程。

use std::time::Duration;

use anyhow::{Context, Result};

use ha_core::security::ssrf::SsrfPolicy;

const DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const MAX_REDIRECTS: usize = 5;

/// Provider 产物（图/音）下载的默认体积上限。128 MB 覆盖已知 provider 的
/// 最大产物（高分辨率图 / 数分钟音频）仍留余量；超限返回可观测错误。
const MAX_ASSET_BYTES: usize = 128 * 1024 * 1024;

/// Fetch a provider-returned asset URL. Returns `(bytes, mime)`, with `mime`
/// taken from the response `Content-Type` and falling back to `fallback_mime`.
pub async fn fetch_asset(
    url: &str,
    ssrf: SsrfPolicy,
    fallback_mime: &str,
) -> Result<(Vec<u8>, String)> {
    fetch_asset_capped(url, ssrf, fallback_mime, MAX_ASSET_BYTES).await
}

/// [`fetch_asset`] with an explicit response-size cap（`input.rs` 的参考图
/// 下载用 10 MB 档复用同一条 SSRF 逐跳门）。
pub async fn fetch_asset_capped(
    url: &str,
    ssrf: SsrfPolicy,
    fallback_mime: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, String)> {
    let trusted = ha_core::config::cached_config().ssrf.trusted_hosts.clone();

    // 禁自动跳转：每一跳都必须重新过异步 SSRF 门（DNS 解析级），reqwest
    // 的 redirect 回调是同步的、跑不了 `check_url`，所以显式循环跟随。
    let client = ha_core::provider::apply_proxy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none()),
    )
    .build()
    .context("failed to build asset download client")?;

    let mut current = url.to_string();
    let mut response = None;
    for _hop in 0..=MAX_REDIRECTS {
        let parsed = ha_core::security::ssrf::check_url(&current, ssrf, &trusted)
            .await
            .with_context(|| {
                format!(
                    "blocked asset URL: {}",
                    ha_core::truncate_utf8(&current, 200)
                )
            })?;
        let resp = client.get(parsed).send().await.with_context(|| {
            format!(
                "failed to download {}",
                ha_core::truncate_utf8(&current, 200)
            )
        })?;
        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "redirect without Location header: {}",
                        ha_core::truncate_utf8(&current, 200)
                    )
                })?;
            let base = reqwest::Url::parse(&current)?;
            let next = base.join(location).with_context(|| {
                format!(
                    "invalid redirect target: {}",
                    ha_core::truncate_utf8(location, 200)
                )
            })?;
            if !matches!(next.scheme(), "http" | "https") {
                anyhow::bail!("redirect to non-http(s) scheme: {}", next.scheme());
            }
            current = next.to_string();
            continue;
        }
        response = Some(resp);
        break;
    }
    let resp = response.ok_or_else(|| {
        anyhow::anyhow!(
            "too many redirects (>{MAX_REDIRECTS}): {}",
            ha_core::truncate_utf8(url, 200)
        )
    })?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!(
            "asset download failed ({status}): {}",
            ha_core::truncate_utf8(url, 200)
        );
    }

    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_mime.to_string());

    let data = ha_core::security::http_stream::read_bytes_capped(resp, max_bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "asset download exceeded/failed ({}): {e}",
                ha_core::truncate_utf8(url, 200)
            )
        })?;
    Ok((data, mime))
}

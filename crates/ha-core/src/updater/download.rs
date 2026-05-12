//! Streaming download for self-update artifacts.
//!
//! Verification is the caller's responsibility ([`super::signature`]); this
//! module only fetches bytes. Two safety caps:
//!
//! - Hard byte ceiling [`MAX_DOWNLOAD_BYTES`] so a tampered manifest can't
//!   make us stream a multi-GB URL into the user's home directory.
//! - Proxy resolution via [`crate::provider::apply_proxy_for_url`] so users
//!   behind a corporate / system / custom proxy reach the release server.
//!
//! EventBus emit is throttled (5% / 1s, whichever fires first) so a multi-MB
//! download doesn't flood the bus.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::json;
use tokio::io::AsyncWriteExt;

/// Hard ceiling on a single archive download. Bare-binary archives ship at
/// ~10-15 MB today; the cap is generous enough for foreseeable growth and
/// tight enough that a tampered manifest URL pointing at a multi-GB blob
/// can't fill `~/.hope-agent/updater/staging/`.
pub const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// Fetch a small text resource (Minisign `.sig` file). One-shot allocation
/// is fine; sig files are < 1 KB.
pub async fn download_text(url: &str) -> Result<String> {
    ssrf_check(url).await?;
    let builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    let client = crate::provider::apply_proxy_for_url(builder, url)
        .build()
        .context("reqwest client build failed")?;
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {url}");
    }
    resp.text()
        .await
        .with_context(|| format!("read body of {url}"))
}

/// Stream-download `url` to `dest`, emitting `app_update:progress` events
/// as bytes arrive. Returns the total byte count. Aborts and removes the
/// half-written destination if the stream exceeds [`MAX_DOWNLOAD_BYTES`].
///
/// `progress_label` ("download" / "stage" / "verify" …) is included in the
/// event payload so the UI can render per-phase progress without inferring
/// from message order.
pub async fn download_to(
    url: &str,
    dest: &Path,
    job_id: &str,
    progress_label: &str,
) -> Result<u64> {
    ssrf_check(url).await?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create download dir {}", parent.display()))?;
    }

    // No outer timeout — large binaries on slow networks legitimately take
    // minutes. The byte ceiling below caps the worst case; for stalled
    // connections, reqwest's per-read I/O errors will surface and the
    // caller can retry.
    let builder = reqwest::Client::builder();
    let client = crate::provider::apply_proxy_for_url(builder, url)
        .build()
        .context("reqwest client build failed")?;
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {url}");
    }
    let total = resp.content_length();
    if let Some(t) = total {
        if t > MAX_DOWNLOAD_BYTES {
            anyhow::bail!(
                "advertised Content-Length {t} exceeds MAX_DOWNLOAD_BYTES ({MAX_DOWNLOAD_BYTES})"
            );
        }
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("create download dest {}", dest.display()))?;

    let mut written: u64 = 0;
    let mut last_emit = Instant::now();
    let mut last_emit_pct: u32 = 0;
    let mut stream = resp.bytes_stream();

    emit_progress(job_id, progress_label, 0, total, "downloading");

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.with_context(|| format!("read stream chunk from {url}"))?;
        written += bytes.len() as u64;
        if written > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            anyhow::bail!("download exceeded MAX_DOWNLOAD_BYTES ({MAX_DOWNLOAD_BYTES}) — aborted");
        }
        file.write_all(&bytes)
            .await
            .with_context(|| format!("write to {}", dest.display()))?;

        let pct = total
            .map(|t| ((written * 100) / t.max(1)) as u32)
            .unwrap_or(0);
        let elapsed = last_emit.elapsed();
        if pct.saturating_sub(last_emit_pct) >= 5 || elapsed >= Duration::from_secs(1) {
            emit_progress(job_id, progress_label, written, total, "downloading");
            last_emit = Instant::now();
            last_emit_pct = pct;
        }
    }
    file.flush().await.ok();
    file.sync_all().await.ok();
    drop(file);

    emit_progress(job_id, progress_label, written, total, "downloaded");
    Ok(written)
}

/// SSRF gate for every outbound URL in this module. `Default` blocks
/// private / link-local / metadata / unspecified / broadcast (the real
/// SSRF concerns) but still allows loopback — matches the rest of
/// ha-core's outbound HTTP and keeps local mirrors / test wiremock
/// servers usable without explicit trusted-hosts entries.
async fn ssrf_check(url: &str) -> Result<()> {
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(
        url,
        crate::security::ssrf::SsrfPolicy::Default,
        &ssrf_cfg.trusted_hosts,
    )
    .await
    .with_context(|| format!("SSRF check failed for {url}"))?;
    Ok(())
}

fn emit_progress(job_id: &str, label: &str, written: u64, total: Option<u64>, phase: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "app_update:progress",
            json!({
                "job_id": job_id,
                "label": label,
                "phase": phase,
                "written": written,
                "total": total,
                "percent": total
                    .map(|t| ((written * 100) / t.max(1)) as u32)
                    .unwrap_or(0),
            }),
        );
    }
}

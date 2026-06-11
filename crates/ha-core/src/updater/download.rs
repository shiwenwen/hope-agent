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

/// Number of download attempts before giving up. The first try plus two
/// retries covers transient network blips without hammering the release server.
const MAX_ATTEMPTS: u32 = 3;

/// Outcome classification for a single download attempt so the retry loop knows
/// whether to back off and try again or bail immediately.
enum AttemptError {
    /// Network / IO hiccup — safe to retry, partial bytes kept for resume.
    Retryable(anyhow::Error),
    /// Permanent failure (4xx, size cap, bad range) — stop now.
    Fatal(anyhow::Error),
}

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
    // SSRF is a permanent property of the URL — check once, never retry it.
    ssrf_check(url).await?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create download dir {}", parent.display()))?;
    }

    // No outer timeout — large binaries on slow networks legitimately take
    // minutes. The byte ceiling caps the worst case; stalled connections
    // surface as per-read I/O errors and trip the retry loop below.
    let builder = reqwest::Client::builder();
    let client = crate::provider::apply_proxy_for_url(builder, url)
        .build()
        .context("reqwest client build failed")?;

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // Resume from whatever is already on disk from a prior aborted attempt.
        let resume_from = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        match download_attempt(&client, url, dest, job_id, progress_label, resume_from).await {
            Ok(written) => return Ok(written),
            Err(AttemptError::Fatal(e)) => return Err(e),
            Err(AttemptError::Retryable(e)) => {
                if attempt >= MAX_ATTEMPTS {
                    return Err(e.context(format!(
                        "download of {url} failed after {MAX_ATTEMPTS} attempts"
                    )));
                }
                // Exponential backoff: 1s, 2s. Partial bytes on disk are kept
                // so the next attempt resumes via HTTP Range.
                let backoff = Duration::from_secs(1u64 << (attempt - 1));
                app_warn!(
                    "self_update",
                    "download",
                    "attempt {}/{} for {} failed ({}); retrying in {}s (resume from {} bytes)",
                    attempt,
                    MAX_ATTEMPTS,
                    url,
                    e,
                    backoff.as_secs(),
                    resume_from
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

/// One download attempt. Resumes from `resume_from` via an HTTP `Range` request
/// when the server honors it (206); falls back to a clean restart when the
/// server replies `200` (ignored the range). Returns the total bytes on disk.
async fn download_attempt(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    job_id: &str,
    progress_label: &str,
    resume_from: u64,
) -> std::result::Result<u64, AttemptError> {
    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AttemptError::Retryable(anyhow::anyhow!("fetch {url}: {e}")))?;
    let status = resp.status();

    // 416 = the range is past EOF; the file on disk is likely already complete
    // or corrupt. Wipe it and restart fresh on the next attempt.
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(AttemptError::Retryable(anyhow::anyhow!(
            "HTTP 416 for {url} (stale partial removed, will restart)"
        )));
    }
    if !status.is_success() {
        let msg = anyhow::anyhow!("HTTP {status} from {url}");
        // 5xx is transient; 4xx (auth, gone, etc.) is permanent.
        return Err(if status.is_server_error() {
            AttemptError::Retryable(msg)
        } else {
            AttemptError::Fatal(msg)
        });
    }

    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0;
    // Full size: from Content-Range total on 206, else Content-Length on 200.
    let total = if is_partial {
        parse_content_range_total(&resp).or_else(|| resp.content_length().map(|c| resume_from + c))
    } else {
        resp.content_length()
    };
    if let Some(t) = total {
        if t > MAX_DOWNLOAD_BYTES {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AttemptError::Fatal(anyhow::anyhow!(
                "advertised size {t} exceeds MAX_DOWNLOAD_BYTES ({MAX_DOWNLOAD_BYTES})"
            )));
        }
    }

    // 206 → append to the partial; 200 (or fresh) → truncate and start over.
    let (mut written, mut file) = if is_partial {
        let f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .await
            .map_err(|e| {
                AttemptError::Retryable(anyhow::anyhow!("open {} for append: {e}", dest.display()))
            })?;
        (resume_from, f)
    } else {
        let f = tokio::fs::File::create(dest).await.map_err(|e| {
            AttemptError::Retryable(anyhow::anyhow!("create {}: {e}", dest.display()))
        })?;
        (0u64, f)
    };

    let mut last_emit = Instant::now();
    let mut last_emit_pct: u32 = 0;
    let mut stream = resp.bytes_stream();

    emit_progress(job_id, progress_label, written, total, "downloading");

    while let Some(chunk) = stream.next().await {
        let bytes = chunk
            .map_err(|e| AttemptError::Retryable(anyhow::anyhow!("read chunk from {url}: {e}")))?;
        written += bytes.len() as u64;
        if written > MAX_DOWNLOAD_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(AttemptError::Fatal(anyhow::anyhow!(
                "download exceeded MAX_DOWNLOAD_BYTES ({MAX_DOWNLOAD_BYTES}) — aborted"
            )));
        }
        file.write_all(&bytes).await.map_err(|e| {
            AttemptError::Retryable(anyhow::anyhow!("write to {}: {e}", dest.display()))
        })?;

        let pct = total
            .map(|t| ((written * 100) / t.max(1)) as u32)
            .unwrap_or(0);
        if pct.saturating_sub(last_emit_pct) >= 5 || last_emit.elapsed() >= Duration::from_secs(1) {
            emit_progress(job_id, progress_label, written, total, "downloading");
            last_emit = Instant::now();
            last_emit_pct = pct;
        }
    }
    file.flush().await.ok();
    file.sync_all().await.ok();
    drop(file);

    // Guard against a short read masquerading as success — a truncated body
    // would otherwise fail signature verification with a confusing error.
    if let Some(t) = total {
        if written < t {
            return Err(AttemptError::Retryable(anyhow::anyhow!(
                "incomplete download: {written}/{t} bytes from {url}"
            )));
        }
    }

    emit_progress(job_id, progress_label, written, total, "downloaded");
    Ok(written)
}

/// Parse the total size out of a `Content-Range: bytes start-end/total` header.
fn parse_content_range_total(resp: &reqwest::Response) -> Option<u64> {
    let v = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    parse_total_from_content_range(v)
}

/// Pure parser: extract `total` from a `bytes start-end/total` value. Returns
/// `None` for the `*` (unknown total) form or anything unparseable.
fn parse_total_from_content_range(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse::<u64>().ok()
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

#[cfg(test)]
mod tests {
    use super::parse_total_from_content_range;

    #[test]
    fn content_range_total_parses_standard_form() {
        assert_eq!(
            parse_total_from_content_range("bytes 200-1023/1024"),
            Some(1024)
        );
        assert_eq!(parse_total_from_content_range("bytes 0-0/1"), Some(1));
    }

    #[test]
    fn content_range_total_rejects_unknown_and_garbage() {
        assert_eq!(parse_total_from_content_range("bytes 0-1/*"), None);
        assert_eq!(parse_total_from_content_range("garbage"), None);
        assert_eq!(parse_total_from_content_range(""), None);
    }
}

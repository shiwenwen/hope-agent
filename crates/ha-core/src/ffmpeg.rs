//! ffmpeg runtime — on-demand downloads + unpacks a static ffmpeg build when
//! the host has no `ffmpeg` on PATH, so the design space's **MP4 export strong
//! path** (real-browser frames → ffmpeg encode, see `design/render_native.rs`)
//! works out of the box instead of silently degrading to the lower-fidelity
//! client-side WebCodecs encoder.
//!
//! Mirrors [`crate::browser::runtime`] (Chromium on-demand fetch): same trust
//! model — HTTPS from a fixed static-build host + SSRF check + zip extract +
//! `-version` smoke test + ready marker. No hash pin (consistent with the
//! Chromium runtime, which also trusts HTTPS + fixed host + smoke test).
//!
//! **Never triggered automatically**: the download is ~30–90 MB and the user
//! should see progress. Triggered from the export flow's pre-check → explicit
//! "download encoder" action, or Settings. Any failure returns `Err`, and the
//! caller degrades to guide-install + client fallback — **the strong path
//! never blocks or panics on a missing/broken ffmpeg.**

use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::paths;

const READY_MARKER: &str = ".hope-agent-ready";

/// Per-platform descriptor for fetching + unpacking a static ffmpeg build.
#[derive(Debug, Clone)]
pub struct FfmpegSpec {
    /// Cache-dir version tag (bump to force a re-download).
    pub version: &'static str,
    /// HTTPS URL of a **zip** archive containing the ffmpeg binary.
    pub url: &'static str,
    /// Path to the runnable binary RELATIVE to the unzipped archive root.
    pub binary_relpath: &'static str,
}

// Pinned static-build source per platform. Two providers, both shipping **zip**
// archives (we vendor `zip`, not `xz`, so tar.xz builds are out):
//   • macOS (arm64/amd64) + Linux (amd64/arm64) → martin-riedl.de, which packs a
//     single self-contained `ffmpeg` at the archive root.
//   • Windows (amd64) → martin-riedl.de does **not** build Windows, so we use
//     BtbN/FFmpeg-Builds' statically-linked `win64-gpl` zip. Its binary is
//     nested at `ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe`, hence the
//     per-platform `binary_relpath`.
//
// Bump procedure:
// 1. Confirm each platform's `url` still 200s (both hosts publish rolling
//    `latest` builds) and the archive still holds the binary at `binary_relpath`.
// 2. Bump `CACHE_VERSION` so existing users re-download the newer build. The
//    prior version's install dir is auto-reaped on the next successful install
//    (`prune_stale_versions`), so bumps don't accumulate stale copies on disk.
// 3. Run `ensure_ffmpeg` on each platform to confirm `-version` works.
//
// If a URL goes stale the download/extract/smoke test fails → `Err` →
// the export flow degrades to guide-install + client WebCodecs. Nothing breaks.
const CACHE_VERSION: &str = "static-1";

/// Resolve the [`FfmpegSpec`] for the current host, or `None` when we don't
/// ship an auto-download source for this OS/arch (caller falls back to
/// guide-install: `brew`/`winget`/`apt` + `HA_FFMPEG_PATH`).
pub fn spec_for_current_platform() -> Option<FfmpegSpec> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip",
            binary_relpath: "ffmpeg",
        });
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip",
            binary_relpath: "ffmpeg",
        });
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://ffmpeg.martin-riedl.de/redirect/latest/linux/amd64/release/ffmpeg.zip",
            binary_relpath: "ffmpeg",
        });
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://ffmpeg.martin-riedl.de/redirect/latest/linux/arm64/release/ffmpeg.zip",
            binary_relpath: "ffmpeg",
        });
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        // martin-riedl.de ships no Windows build → BtbN's static win64-gpl zip.
        // The binary is nested under the archive's top-level folder.
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
            binary_relpath: "ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe",
        });
    }
    #[allow(unreachable_code)]
    None
}

/// EventBus channel for ffmpeg runtime download progress (mirrors the Chromium
/// `browser:chromium_download_progress` shape).
pub const PROGRESS_EVENT: &str = "design:ffmpeg_download_progress";

/// Three-state provisioning status for the export pre-check UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    /// `ready` = a runnable ffmpeg is available (env / PATH / cached runtime).
    pub ready: bool,
    /// How it resolved: `env` | `path` | `runtime` | `missing`.
    pub source: String,
    /// Resolved binary path when `ready`, else `None`.
    pub binary_path: Option<String>,
    /// Whether this platform has an auto-download source (else guide-install).
    pub can_auto_install: bool,
}

/// Resolve a runnable ffmpeg binary path/command, in priority order:
/// `HA_FFMPEG_PATH` env → cached downloaded runtime → bare `ffmpeg` (PATH).
/// Always returns *something* invokable; existence of the PATH fallback isn't
/// checked here (the encode step surfaces a spawn error if it's absent).
pub fn resolve_bin() -> String {
    if let Some(env) = std::env::var("HA_FFMPEG_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return env;
    }
    if let Some(cached) = cached_binary_path() {
        return cached.to_string_lossy().into_owned();
    }
    "ffmpeg".to_string()
}

/// Non-blocking three-state probe for the export pre-check. Only actually runs
/// `-version` for the PATH candidate (cheap); env/runtime are path-existence.
pub async fn doctor() -> FfmpegStatus {
    let can_auto_install = spec_for_current_platform().is_some();

    if let Some(env) = std::env::var("HA_FFMPEG_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        if Path::new(&env).exists() {
            return FfmpegStatus {
                ready: true,
                source: "env".into(),
                binary_path: Some(env),
                can_auto_install,
            };
        }
    }
    if let Some(cached) = cached_binary_path() {
        return FfmpegStatus {
            ready: true,
            source: "runtime".into(),
            binary_path: Some(cached.to_string_lossy().into_owned()),
            can_auto_install,
        };
    }
    if path_ffmpeg_works().await {
        return FfmpegStatus {
            ready: true,
            source: "path".into(),
            binary_path: Some("ffmpeg".into()),
            can_auto_install,
        };
    }
    FfmpegStatus {
        ready: false,
        source: "missing".into(),
        binary_path: None,
        can_auto_install,
    }
}

async fn path_ffmpeg_works() -> bool {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-version").kill_on_drop(true);
    crate::platform::hide_console_tokio(&mut cmd);
    matches!(cmd.output().await, Ok(o) if o.status.success())
}

/// One-percent–throttled wrapper around [`ensure_ffmpeg`] that emits structured
/// progress on the global EventBus (mirrors the Chromium runtime helper).
pub async fn install_with_event_bus_progress() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    let last_percent = Arc::new(AtomicU64::new(u64::MAX));
    let progress_last_percent = Arc::clone(&last_percent);
    let progress = move |downloaded: u64, total: Option<u64>| {
        let percent = total
            .and_then(|t| downloaded.checked_mul(100).and_then(|n| n.checked_div(t)))
            .map(|p| p.min(100));
        let report_pct = percent.unwrap_or(u64::MAX);
        let prev = progress_last_percent.load(Ordering::Relaxed);
        if prev == u64::MAX || (report_pct != u64::MAX && report_pct != prev) {
            progress_last_percent.store(report_pct, Ordering::Relaxed);
            if let Some(bus) = crate::globals::EVENT_BUS.get() {
                bus.emit(
                    PROGRESS_EVENT,
                    serde_json::json!({
                        "stage": "downloading",
                        "percent": percent,
                        "downloadedBytes": downloaded,
                        "totalBytes": total,
                    }),
                );
            }
        }
    };
    let binary = ensure_ffmpeg(progress).await?;
    if let Some(bus) = crate::globals::EVENT_BUS.get() {
        bus.emit(
            PROGRESS_EVENT,
            serde_json::json!({
                "stage": "ready",
                "percent": 100,
                "binaryPath": binary.display().to_string(),
            }),
        );
    }
    Ok(binary)
}

/// Resolve the cached ffmpeg binary, downloading + unpacking the static build
/// on first call. `progress` is invoked with `(downloaded_bytes, total_bytes)`.
pub async fn ensure_ffmpeg<F>(progress: F) -> Result<PathBuf>
where
    F: Fn(u64, Option<u64>) + Send + Sync + 'static,
{
    let spec = spec_for_current_platform().ok_or_else(|| {
        anyhow!(
            "No bundled ffmpeg download for this platform/architecture. \
             Install ffmpeg (brew / winget / apt) or set HA_FFMPEG_PATH."
        )
    })?;
    let target_dir = paths::ffmpeg_version_dir(spec.version)?;
    let binary = target_dir.join(spec.binary_relpath);
    if runtime_ready(&target_dir, &binary) {
        return Ok(binary);
    }
    if binary.exists() {
        smoke_test_binary(&binary).await?;
        write_ready_marker(&target_dir, &spec)?;
        return Ok(binary);
    }

    let runtime_root = paths::ffmpeg_runtime_dir()?;
    std::fs::create_dir_all(&runtime_root)?;

    // SSRF: fixed static-build host; the default outbound policy lets it
    // through, but stay consistent with every other outbound call.
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(spec.url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts).await?;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let archive_path = runtime_root.join(format!("ffmpeg.{}.tmp.{}.zip", spec.version, nonce));
    let staging_dir = runtime_root.join(format!(".ffmpeg-{}.{}.tmp", spec.version, nonce));

    let install_result: Result<PathBuf> = async {
        download_archive(spec.url, &archive_path, &progress).await?;
        let staged_binary = extract_binary(&archive_path, &staging_dir, spec.binary_relpath)?;

        #[cfg(unix)]
        chmod_executable(&staged_binary)?;

        smoke_test_binary(&staged_binary).await?;
        write_ready_marker(&staging_dir, &spec)?;

        if target_dir.exists() {
            std::fs::remove_dir_all(&target_dir).map_err(|e| {
                anyhow!(
                    "removing incomplete ffmpeg runtime {}: {}",
                    target_dir.display(),
                    e
                )
            })?;
        }
        std::fs::rename(&staging_dir, &target_dir).map_err(|e| {
            anyhow!(
                "promoting ffmpeg runtime {} -> {}: {}",
                staging_dir.display(),
                target_dir.display(),
                e
            )
        })?;
        // Reap prior CACHE_VERSION installs so a bump doesn't leak the old
        // ~90–170 MB build on disk. Best-effort, current version untouched.
        prune_stale_versions(&runtime_root, spec.version);
        Ok(target_dir.join(spec.binary_relpath))
    }
    .await;

    let _ = std::fs::remove_file(&archive_path);
    if install_result.is_err() {
        let _ = std::fs::remove_dir_all(&staging_dir);
    }
    install_result
}

/// Quick path: cached ffmpeg binary for the current platform, or `None` if not
/// downloaded yet / unsupported platform.
pub fn cached_binary_path() -> Option<PathBuf> {
    let spec = spec_for_current_platform()?;
    let dir = paths::ffmpeg_version_dir(spec.version).ok()?;
    let binary = dir.join(spec.binary_relpath);
    if runtime_ready(&dir, &binary) {
        Some(binary)
    } else {
        None
    }
}

/// Hard ceiling for a single ffmpeg archive. Static builds run ~30–170 MB (the
/// Windows BtbN zip is the largest); the cap stops a stale/hijacked URL from
/// streaming an unbounded blob into the user's home dir.
const MAX_ARCHIVE_BYTES: u64 = 300 * 1024 * 1024;

/// Download attempts before giving up (first try + 2 retries) — rides out
/// transient network blips without hammering the host.
const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;

/// Outcome of a single download attempt: retry (partial kept for resume) vs bail.
enum DlAttempt {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

/// Stream `url` → `dest` with retry + HTTP `Range` resume: a dropped connection
/// mid-download resumes from the partial file instead of restarting (matters
/// most for the ~170 MB Windows archive on flaky networks). `progress` gets
/// `(downloaded, total)`. Mirrors the robustness of
/// [`crate::updater::download::download_to`] but keeps the ffmpeg progress event.
async fn download_archive<F>(url: &str, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    let client = crate::provider::apply_proxy_for_url(reqwest::Client::builder(), url).build()?;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // Resume from whatever a prior aborted attempt left on disk.
        let resume_from = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        match download_archive_attempt(&client, url, dest, resume_from, progress).await {
            Ok(()) => return Ok(()),
            Err(DlAttempt::Fatal(e)) => return Err(e),
            Err(DlAttempt::Retryable(e)) => {
                if attempt >= MAX_DOWNLOAD_ATTEMPTS {
                    return Err(e.context(format!(
                        "ffmpeg download failed after {} attempts",
                        MAX_DOWNLOAD_ATTEMPTS
                    )));
                }
                let backoff = Duration::from_secs(1u64 << (attempt - 1));
                crate::app_warn!(
                    "design",
                    "ffmpeg",
                    "download attempt {}/{} for {} failed ({}); retrying in {}s (resume from {} bytes)",
                    attempt,
                    MAX_DOWNLOAD_ATTEMPTS,
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

async fn download_archive_attempt<F>(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    resume_from: u64,
    progress: &F,
) -> std::result::Result<(), DlAttempt>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    use tokio::io::AsyncWriteExt;
    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-", resume_from));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| DlAttempt::Retryable(anyhow!("HTTP GET {} failed: {}", url, e)))?;
    let status = resp.status();

    // 416: the partial is past EOF (already complete or corrupt) — wipe + restart.
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DlAttempt::Retryable(anyhow!(
            "HTTP 416 for {} (stale partial removed, will restart)",
            url
        )));
    }
    if !status.is_success() {
        let msg = anyhow!("HTTP {} from {}", status, url);
        // 5xx transient; 4xx (gone / auth) permanent.
        return Err(if status.is_server_error() {
            DlAttempt::Retryable(msg)
        } else {
            DlAttempt::Fatal(msg)
        });
    }

    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT && resume_from > 0;
    // Full size: Content-Range total on a 206 resume, else Content-Length.
    let total = if is_partial {
        content_range_total(&resp).or_else(|| resp.content_length().map(|c| resume_from + c))
    } else {
        resp.content_length()
    };
    if let Some(t) = total {
        if t > MAX_ARCHIVE_BYTES {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(DlAttempt::Fatal(anyhow!(
                "ffmpeg archive size {} exceeds cap {}",
                t,
                MAX_ARCHIVE_BYTES
            )));
        }
    }

    // 206 → append to the partial; else truncate + start fresh.
    let (mut written, mut file) = if is_partial {
        let f = tokio::fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .await
            .map_err(|e| {
                DlAttempt::Retryable(anyhow!("open {} for append: {}", dest.display(), e))
            })?;
        (resume_from, f)
    } else {
        let f = tokio::fs::File::create(dest)
            .await
            .map_err(|e| DlAttempt::Retryable(anyhow!("create {}: {}", dest.display(), e)))?;
        (0u64, f)
    };

    let mut stream = resp.bytes_stream();
    let mut last_emit = std::time::Instant::now();
    progress(written, total);
    while let Some(chunk) = stream.next().await {
        let bytes =
            chunk.map_err(|e| DlAttempt::Retryable(anyhow!("stream chunk error: {}", e)))?;
        written += bytes.len() as u64;
        if written > MAX_ARCHIVE_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(DlAttempt::Fatal(anyhow!(
                "ffmpeg download exceeded cap {} — aborted",
                MAX_ARCHIVE_BYTES
            )));
        }
        file.write_all(&bytes)
            .await
            .map_err(|e| DlAttempt::Retryable(anyhow!("write to {}: {}", dest.display(), e)))?;
        if last_emit.elapsed() >= Duration::from_millis(40) {
            progress(written, total);
            last_emit = std::time::Instant::now();
        }
    }
    file.flush().await.ok();
    file.sync_all().await.ok();
    drop(file);

    // Short-read guard: a truncated body fails extraction with a confusing error.
    if let Some(t) = total {
        if written < t {
            return Err(DlAttempt::Retryable(anyhow!(
                "incomplete download: {}/{} bytes from {}",
                written,
                t,
                url
            )));
        }
    }
    progress(written, total);
    Ok(())
}

/// Parse the total size out of a `Content-Range: bytes start-end/total` header.
fn content_range_total(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Extract ONLY the binary at `wanted` from the zip into `target`, preserving
/// its relative path (so the caller finds it at `target/wanted`). Static ffmpeg
/// builds are self-contained single executables, so pulling just that one file
/// avoids unpacking the ~290 MB of sibling tools (ffplay / ffprobe) the Windows
/// archive bundles. Returns the extracted path; errors if `wanted` isn't in the
/// archive — the caller's `-version` smoke test would fail anyway, so the export
/// flow degrades cleanly rather than shipping a broken runtime.
fn extract_binary(archive: &Path, target: &Path, wanted: &str) -> Result<PathBuf> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| anyhow!("opening zip {}: {}", archive.display(), e))?;
    std::fs::create_dir_all(target)?;
    let wanted_path = Path::new(wanted);
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| anyhow!("zip entry {}: {}", i, e))?;
        if entry.is_dir() {
            continue;
        }
        // `mangled_name` keeps components within target (zip-slip guard);
        // path-equality normalizes separators so the match is OS-agnostic.
        let rel = entry.mangled_name();
        if rel != wanted_path {
            continue;
        }
        let out_path = target.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))?;
        }
        return Ok(out_path);
    }
    bail!(
        "ffmpeg binary '{}' not found in archive {}",
        wanted,
        archive.display()
    )
}

#[cfg(unix)]
fn chmod_executable(binary: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = binary.metadata().map_err(|e| {
        anyhow!(
            "ffmpeg binary not present after extraction at {}: {}",
            binary.display(),
            e
        )
    })?;
    let mut perms = metadata.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(binary, perms)?;
    Ok(())
}

async fn smoke_test_binary(binary: &Path) -> Result<()> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("-version").kill_on_drop(true);
    crate::platform::hide_console_tokio(&mut cmd);
    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow!("smoke test (ffmpeg -version) failed to spawn: {}", e))?;
    if !output.status.success() {
        bail!(
            "ffmpeg runtime at {} did not start: exit={:?}, stderr={}",
            binary.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.to_ascii_lowercase().contains("ffmpeg version") {
        bail!(
            "ffmpeg runtime smoke test returned unexpected banner: {}",
            stdout.lines().next().unwrap_or("").trim()
        );
    }
    Ok(())
}

fn runtime_ready(target_dir: &Path, binary: &Path) -> bool {
    binary.exists() && target_dir.join(READY_MARKER).exists()
}

fn write_ready_marker(target_dir: &Path, spec: &FfmpegSpec) -> Result<()> {
    std::fs::write(
        target_dir.join(READY_MARKER),
        format!("version={}\nurl={}\n", spec.version, spec.url),
    )?;
    Ok(())
}

/// Best-effort GC of *other* `CACHE_VERSION` install dirs under `runtime_root`
/// (a version bump re-downloads into a fresh dir; without this the old
/// ~90–170 MB build would linger forever). Deliberately conservative: only
/// removes sibling **version** dirs, never `current_version`, and never the
/// transient `.ffmpeg-*.tmp` staging dirs / `ffmpeg.*.tmp.*.zip` archives that a
/// concurrent install may be writing right now. Errors are swallowed (e.g.
/// Windows can refuse to delete a binary still open by an older running build).
fn prune_stale_versions(runtime_root: &Path, current_version: &str) {
    let Ok(entries) = std::fs::read_dir(runtime_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // Keep the current version + anything transient (leading `.` staging
        // dirs, `.tmp`-tagged staging/zip artifacts of an in-flight install).
        if name == current_version || name.starts_with('.') || name.contains(".tmp") {
            continue;
        }
        if entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_for_supported_platform_is_populated() {
        let spec = spec_for_current_platform();
        #[cfg(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64"),
        ))]
        {
            let spec = spec.expect("supported platform must have an FfmpegSpec");
            assert!(spec.url.starts_with("https://"));
            assert!(!spec.binary_relpath.is_empty());
        }
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64"),
        )))]
        assert!(spec.is_none());
    }

    #[test]
    fn resolve_bin_prefers_env_override() {
        // With no env set + nothing cached, falls back to bare `ffmpeg`.
        // (Can't set env in a shared-process test safely; just assert the
        // fallback is a non-empty invokable string.)
        let bin = resolve_bin();
        assert!(!bin.is_empty());
    }

    #[test]
    fn cached_binary_path_none_on_fresh_install() {
        // Must not panic when nothing's downloaded.
        let _ = cached_binary_path();
    }

    /// Build a zip at `path` containing each `(name, bytes)` entry.
    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).expect("create zip"));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, opts).expect("start entry");
            zip.write_all(bytes).expect("write entry");
        }
        zip.finish().expect("finish zip");
    }

    #[test]
    fn extract_binary_pulls_root_entry() {
        // martin-riedl.de layout: a single `ffmpeg` at the archive root.
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("a.zip");
        write_test_zip(&archive, &[("ffmpeg", b"ELF-ish")]);
        let out = tmp.path().join("staging");
        let got = extract_binary(&archive, &out, "ffmpeg").expect("extract");
        assert_eq!(got, out.join("ffmpeg"));
        assert_eq!(std::fs::read(&got).unwrap(), b"ELF-ish");
    }

    #[test]
    fn extract_binary_pulls_nested_and_skips_siblings() {
        // BtbN Windows layout: binary nested under a top-level folder, alongside
        // ffplay/ffprobe we deliberately do NOT unpack (footprint guard).
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("win.zip");
        write_test_zip(
            &archive,
            &[
                ("ffmpeg-master-latest-win64-gpl/bin/ffplay.exe", b"decoy1"),
                ("ffmpeg-master-latest-win64-gpl/bin/ffprobe.exe", b"decoy2"),
                ("ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe", b"WANT"),
            ],
        );
        let out = tmp.path().join("staging");
        let want = "ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe";
        let got = extract_binary(&archive, &out, want).expect("extract");
        assert_eq!(got, out.join(want));
        assert_eq!(std::fs::read(&got).unwrap(), b"WANT");
        // Siblings must be left in the archive, not written to disk.
        assert!(!out
            .join("ffmpeg-master-latest-win64-gpl/bin/ffplay.exe")
            .exists());
        assert!(!out
            .join("ffmpeg-master-latest-win64-gpl/bin/ffprobe.exe")
            .exists());
    }

    #[test]
    fn extract_binary_errors_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let archive = tmp.path().join("bad.zip");
        write_test_zip(&archive, &[("something-else", b"x")]);
        let out = tmp.path().join("staging");
        assert!(extract_binary(&archive, &out, "ffmpeg").is_err());
    }

    #[test]
    fn prune_stale_versions_reaps_old_keeps_current_and_transient() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        for d in [
            "static-0",                  // old version → should be reaped
            "static-1",                  // current → keep
            ".ffmpeg-static-1.7.tmp",    // in-flight staging (leading dot) → keep
            "ffmpeg.static-1.tmp.9.zip", // in-flight zip artifact name → keep
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        // Also a stray zip file that happens to sit here — non-dir, left alone.
        std::fs::write(root.join("static-0.leftover"), b"x").unwrap();

        prune_stale_versions(root, "static-1");

        assert!(!root.join("static-0").exists(), "old version dir reaped");
        assert!(root.join("static-1").exists(), "current version kept");
        assert!(
            root.join(".ffmpeg-static-1.7.tmp").exists(),
            "in-flight staging kept"
        );
        assert!(
            root.join("ffmpeg.static-1.tmp.9.zip").exists(),
            "in-flight zip-named dir kept"
        );
    }
}

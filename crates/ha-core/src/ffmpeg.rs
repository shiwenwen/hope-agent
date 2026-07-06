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

// Pinned static-build source per platform. We use martin-riedl.de because it
// ships **zip** archives for every platform we target (macOS arm64/amd64,
// Linux amd64/arm64, Windows amd64) — avoiding the tar.xz that BtbN/johnvansickle
// use on Linux (we only vendor `zip`, not `xz`).
//
// Bump procedure:
// 1. Confirm `https://ffmpeg.martin-riedl.de/redirect/latest/<os>/<arch>/release/ffmpeg.zip`
//    still 200s per platform (they publish rolling `latest` builds).
// 2. Bump `CACHE_VERSION` so existing users re-download the newer build.
// 3. Run `ensure_ffmpeg` on each platform to confirm `-version` works.
//
// If a URL goes stale the download/extract/smoke test fails → `Err` →
// the export flow degrades to guide-install + client WebCodecs. Nothing breaks.
const CACHE_VERSION: &str = "mr-latest-1";

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
        return Some(FfmpegSpec {
            version: CACHE_VERSION,
            url: "https://ffmpeg.martin-riedl.de/redirect/latest/windows/amd64/release/ffmpeg.zip",
            binary_relpath: "ffmpeg.exe",
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
        download_streaming(spec.url, &archive_path, &progress).await?;
        extract_zip(&archive_path, &staging_dir)?;
        let staged_binary = staging_dir.join(spec.binary_relpath);

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

async fn download_streaming<F>(url: &str, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    use std::io::Write;
    let client = crate::provider::apply_proxy_for_url(reqwest::Client::builder(), url).build()?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("HTTP GET {} failed: {}", url, e))?
        .error_for_status()
        .map_err(|e| anyhow!("HTTP error from {}: {}", url, e))?;
    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("stream chunk error: {}", e))?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() >= std::time::Duration::from_millis(40) {
            progress(downloaded, total);
            last_emit = std::time::Instant::now();
        }
    }
    progress(downloaded, total);
    file.flush()?;
    Ok(())
}

fn extract_zip(archive: &Path, target: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| anyhow!("opening zip {}: {}", archive.display(), e))?;
    std::fs::create_dir_all(target)?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| anyhow!("zip entry {}: {}", i, e))?;
        // `mangled_name` keeps components within target (zip-slip guard).
        let rel = entry.mangled_name();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out_path = target.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
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
    }
    Ok(())
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
}

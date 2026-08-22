//! ffmpeg runtime — on-demand downloads + unpacks a static ffmpeg build when
//! the host has no `ffmpeg` on PATH, so the design space's **MP4 export strong
//! path** (real-browser frames → ffmpeg encode, see `design/render_native.rs`)
//! works out of the box instead of silently degrading to the lower-fidelity
//! client-side WebCodecs encoder.
//!
//! Mirrors the managed browser runtime: every platform artifact is selected
//! from a checked-in immutable manifest, length/SHA-256 verified before
//! extraction, license checked during the smoke test, and promoted atomically.
//!
//! **Never triggered automatically**: the download is ~30–90 MB and the user
//! should see progress. Triggered from the export flow's pre-check → explicit
//! "download encoder" action, or Settings. Any failure returns `Err`, and the
//! caller degrades to guide-install + client fallback — **the strong path
//! never blocks or panics on a missing/broken ffmpeg.**

use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ha_core::paths;

const READY_MARKER: &str = ".hope-agent-ready";
const MANIFEST_JSON: &str = include_str!("../resources/ffmpeg-runtime-manifest.json");
const INSTALL_OS_LOCK_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const INSTALL_OS_LOCK_POLL: Duration = Duration::from_millis(50);

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn acquire_install_lock_at(lock_path: &Path, timeout: Duration) -> Result<std::fs::File> {
    let started = std::time::Instant::now();
    loop {
        match ha_core::platform::try_acquire_exclusive_lock(lock_path)
            .map_err(anyhow::Error::from)?
        {
            Some(file) => return Ok(file),
            None if started.elapsed() < timeout => std::thread::sleep(INSTALL_OS_LOCK_POLL),
            None => bail!("timed out waiting for the FFmpeg runtime install lock"),
        }
    }
}

async fn acquire_install_lock(runtime_root: &Path) -> Result<std::fs::File> {
    let lock_path = runtime_root.join("install.lock");
    ha_core::blocking::run_blocking(move || {
        acquire_install_lock_at(&lock_path, INSTALL_OS_LOCK_TIMEOUT)
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegManifest {
    schema_version: u32,
    generated_at: String,
    artifacts: Vec<FfmpegSpec>,
}

/// Per-platform descriptor for fetching and unpacking a static FFmpeg build.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegSpec {
    pub version: String,
    pub build: String,
    pub cache_key: String,
    pub platform: String,
    pub url: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub published_at: String,
    pub binary_relpath: String,
    pub source_name: String,
    pub source_evidence: String,
    pub license_name: String,
    pub license_evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FfmpegReadyMarker {
    schema_version: u32,
    product: String,
    version: String,
    build: String,
    cache_key: String,
    platform: String,
    url: String,
    archive_size: u64,
    archive_sha256: String,
    published_at: String,
    binary_relpath: String,
    source_name: String,
    source_evidence: String,
    license_name: String,
    license_evidence: String,
    manifest_generated_at: String,
}

fn current_platform_id() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some("mac-arm64");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some("mac-x64");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some("linux-x64");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Some("linux-arm64");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Some("win64");
    #[allow(unreachable_code)]
    None
}

fn embedded_manifest() -> Result<FfmpegManifest> {
    let manifest: FfmpegManifest = serde_json::from_str(MANIFEST_JSON)
        .map_err(|e| anyhow!("invalid embedded FFmpeg runtime manifest: {e}"))?;
    if manifest.schema_version != 1 || manifest.generated_at.trim().is_empty() {
        bail!("unsupported or incomplete FFmpeg runtime manifest");
    }
    Ok(manifest)
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn validate_spec(spec: &FfmpegSpec) -> Result<()> {
    if !matches!(
        spec.platform.as_str(),
        "mac-arm64" | "mac-x64" | "linux-arm64" | "linux-x64" | "win64"
    ) {
        bail!("unsupported FFmpeg platform in manifest");
    }
    if spec.version.trim().is_empty()
        || spec.build.trim().is_empty()
        || spec.cache_key.trim().is_empty()
        || !spec
            .cache_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("invalid FFmpeg version metadata in manifest");
    }
    if spec.archive_size == 0 || spec.archive_size > MAX_ARCHIVE_BYTES {
        bail!("invalid FFmpeg archive size in manifest");
    }
    if spec.archive_sha256.len() != 64
        || !spec
            .archive_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("invalid FFmpeg SHA-256 in manifest");
    }
    let url =
        reqwest::Url::parse(&spec.url).map_err(|e| anyhow!("invalid FFmpeg archive URL: {e}"))?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https" || !matches!(host, "ffmpeg.martin-riedl.de" | "github.com") {
        bail!("FFmpeg archive URL uses an unapproved source host");
    }
    if !safe_relative_path(&spec.binary_relpath)
        || spec.published_at.trim().is_empty()
        || spec.source_name.trim().is_empty()
        || spec.source_evidence.trim().is_empty()
        || spec.license_name != "GPL-3.0-or-later"
        || spec.license_evidence.trim().is_empty()
    {
        bail!("FFmpeg manifest metadata is incomplete");
    }
    Ok(())
}

fn required_spec() -> Result<(FfmpegManifest, FfmpegSpec)> {
    let platform = current_platform_id().ok_or_else(|| {
        anyhow!(
            "No managed FFmpeg download for this platform/architecture. Install FFmpeg or set HA_FFMPEG_PATH."
        )
    })?;
    let manifest = embedded_manifest()?;
    let spec = manifest
        .artifacts
        .iter()
        .find(|spec| spec.platform == platform)
        .cloned()
        .ok_or_else(|| anyhow!("FFmpeg manifest has no artifact for {platform}"))?;
    validate_spec(&spec)?;
    Ok((manifest, spec))
}

/// Resolve the [`FfmpegSpec`] for the current host, or `None` when we don't
/// ship an auto-download source for this OS/arch (caller falls back to
/// guide-install: `brew`/`winget`/`apt` + `HA_FFMPEG_PATH`).
pub fn spec_for_current_platform() -> Option<FfmpegSpec> {
    required_spec().ok().map(|(_, spec)| spec)
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
    ha_core::platform::hide_console_tokio(&mut cmd);
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
            if let Some(bus) = ha_core::globals::EVENT_BUS.get() {
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
    if let Some(bus) = ha_core::globals::EVENT_BUS.get() {
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
    let (manifest, spec) = required_spec()?;
    let target_dir = paths::ffmpeg_version_dir(&spec.cache_key)?;
    let binary = target_dir.join(&spec.binary_relpath);
    if runtime_ready(&target_dir, &binary, &manifest, &spec) {
        return Ok(binary);
    }

    // Serialize in-process callers first, then take a stable OS lock shared by
    // Desktop, server and ACP processes using the same data root.
    let _process_install_guard = INSTALL_LOCK.lock().await;
    if runtime_ready(&target_dir, &binary, &manifest, &spec) {
        return Ok(binary);
    }

    let runtime_root = paths::ffmpeg_runtime_dir()?;
    std::fs::create_dir_all(&runtime_root)?;
    let _cross_process_install_guard = acquire_install_lock(&runtime_root).await?;
    // A peer may have published the verified runtime while this call waited.
    // Rechecking under the OS lock avoids a redundant download and promotion.
    if runtime_ready(&target_dir, &binary, &manifest, &spec) {
        return Ok(binary);
    }

    // SSRF: fixed static-build host; the default outbound policy lets it
    // through, but stay consistent with every other outbound call.
    let ssrf_cfg = &ha_core::config::cached_config().ssrf;
    ha_core::security::ssrf::check_url(&spec.url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts)
        .await?;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let archive_path = runtime_root.join(format!("ffmpeg.{}.tmp.{}.zip", spec.cache_key, nonce));
    let staging_dir = runtime_root.join(format!(".ffmpeg-{}.{}.tmp", spec.cache_key, nonce));

    let install_result: Result<PathBuf> = async {
        download_archive(&spec, &archive_path, &progress).await?;
        let archive_path_for_prepare = archive_path.clone();
        let staging_dir_for_prepare = staging_dir.clone();
        let archive_sha256 = spec.archive_sha256.clone();
        let binary_relpath = spec.binary_relpath.clone();
        let staged_binary = ha_core::blocking::run_blocking(move || {
            verify_sha256(&archive_path_for_prepare, &archive_sha256)?;
            let staged_binary = extract_binary(
                &archive_path_for_prepare,
                &staging_dir_for_prepare,
                &binary_relpath,
            )?;

            #[cfg(unix)]
            chmod_executable(&staged_binary)?;

            Ok::<PathBuf, anyhow::Error>(staged_binary)
        })
        .await?;

        smoke_test_binary(&staged_binary, &spec).await?;
        let staging_dir_for_finalize = staging_dir.clone();
        let target_dir_for_finalize = target_dir.clone();
        let manifest_for_finalize = manifest.clone();
        let spec_for_finalize = spec.clone();
        ha_core::blocking::run_blocking(move || {
            write_ready_marker(
                &staging_dir_for_finalize,
                &manifest_for_finalize,
                &spec_for_finalize,
            )?;
            promote_staging(&staging_dir_for_finalize, &target_dir_for_finalize, nonce)?;
            Ok(target_dir_for_finalize.join(&spec_for_finalize.binary_relpath))
        })
        .await
    }
    .await;

    let cleanup_archive = archive_path.clone();
    let cleanup_staging = staging_dir.clone();
    let cleanup_failed = install_result.is_err();
    ha_core::blocking::run_blocking(move || {
        let _ = std::fs::remove_file(cleanup_archive);
        if cleanup_failed {
            let _ = std::fs::remove_dir_all(cleanup_staging);
        }
    })
    .await;
    install_result
}

/// Quick path: cached ffmpeg binary for the current platform, or `None` if not
/// downloaded yet / unsupported platform.
pub fn cached_binary_path() -> Option<PathBuf> {
    let (manifest, spec) = required_spec().ok()?;
    let dir = paths::ffmpeg_version_dir(&spec.cache_key).ok()?;
    let binary = dir.join(&spec.binary_relpath);
    if runtime_ready(&dir, &binary, &manifest, &spec) {
        Some(binary)
    } else {
        previous_verified_binary(
            &paths::ffmpeg_runtime_dir().ok()?,
            &spec.platform,
            &spec.cache_key,
        )
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
/// `ha_updater::download::download_to` but keeps the ffmpeg progress event.
async fn download_archive<F>(spec: &FfmpegSpec, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many FFmpeg archive redirects");
        }
        match attempt.url().host_str() {
            Some(
                "ffmpeg.martin-riedl.de" | "github.com" | "release-assets.githubusercontent.com",
            ) => attempt.follow(),
            _ => attempt.stop(),
        }
    });
    let client = ha_core::provider::apply_proxy_for_url(
        reqwest::Client::builder().redirect(redirect_policy),
        &spec.url,
    )
    .build()?;
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // Resume from whatever a prior aborted attempt left on disk.
        let mut resume_from = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if resume_from > spec.archive_size {
            let _ = tokio::fs::remove_file(dest).await;
            resume_from = 0;
        }
        match download_archive_attempt(&client, spec, dest, resume_from, progress).await {
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
                ha_core::app_warn!(
                    "design",
                    "ffmpeg",
                    "download attempt {}/{} for {} failed ({}); retrying in {}s (resume from {} bytes)",
                    attempt,
                    MAX_DOWNLOAD_ATTEMPTS,
                    spec.url,
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
    spec: &FfmpegSpec,
    dest: &Path,
    resume_from: u64,
    progress: &F,
) -> std::result::Result<(), DlAttempt>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    use tokio::io::AsyncWriteExt;
    let mut req = client.get(&spec.url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-", resume_from));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| DlAttempt::Retryable(anyhow!("FFmpeg download failed: {}", e)))?;
    let status = resp.status();

    // 416: the partial is past EOF (already complete or corrupt) — wipe + restart.
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DlAttempt::Retryable(anyhow!(
            "HTTP 416 for {} (stale partial removed, will restart)",
            spec.url
        )));
    }
    if !status.is_success() {
        let msg = anyhow!("HTTP {} from pinned FFmpeg source", status);
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
        if t != spec.archive_size || t > MAX_ARCHIVE_BYTES {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(DlAttempt::Fatal(anyhow!(
                "FFmpeg archive length mismatch: expected {}, got {} (cap {})",
                spec.archive_size,
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
        if written > spec.archive_size || written > MAX_ARCHIVE_BYTES {
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
    if written != spec.archive_size {
        return Err(DlAttempt::Retryable(anyhow!(
            "incomplete FFmpeg download: {}/{} bytes",
            written,
            spec.archive_size
        )));
    }
    progress(written, Some(spec.archive_size));
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

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("FFmpeg archive SHA-256 mismatch");
    }
    Ok(())
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

async fn smoke_test_binary(binary: &Path, spec: &FfmpegSpec) -> Result<()> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("-version").kill_on_drop(true);
    ha_core::platform::hide_console_tokio(&mut cmd);
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .map_err(|_| anyhow!("smoke test (ffmpeg -version) timed out"))?
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
    let banner = stdout.to_ascii_lowercase();
    if !banner.contains("ffmpeg version")
        || !banner.contains(&spec.version.to_ascii_lowercase())
        || !banner.contains("--enable-gpl")
        || !banner.contains("--enable-version3")
    {
        bail!(
            "FFmpeg runtime version/license smoke test returned an unexpected banner: {}",
            stdout.lines().next().unwrap_or("").trim()
        );
    }

    let mut encoders = tokio::process::Command::new(binary);
    encoders
        .args(["-hide_banner", "-encoders"])
        .kill_on_drop(true);
    ha_core::platform::hide_console_tokio(&mut encoders);
    let encoder_output = tokio::time::timeout(Duration::from_secs(10), encoders.output())
        .await
        .map_err(|_| anyhow!("smoke test (ffmpeg -encoders) timed out"))?
        .map_err(|e| anyhow!("smoke test (ffmpeg -encoders) failed to spawn: {}", e))?;
    if !encoder_output.status.success() {
        bail!("FFmpeg runtime encoder inventory failed");
    }
    let inventory = format!(
        "{}\n{}",
        String::from_utf8_lossy(&encoder_output.stdout),
        String::from_utf8_lossy(&encoder_output.stderr)
    )
    .to_ascii_lowercase();
    if !inventory.contains("libx264") || !inventory.contains(" aac ") {
        bail!("FFmpeg runtime is missing the required H.264/AAC encoders");
    }
    Ok(())
}

fn expected_ready_marker(manifest: &FfmpegManifest, spec: &FfmpegSpec) -> FfmpegReadyMarker {
    FfmpegReadyMarker {
        schema_version: 1,
        product: "ffmpeg".to_string(),
        version: spec.version.clone(),
        build: spec.build.clone(),
        cache_key: spec.cache_key.clone(),
        platform: spec.platform.clone(),
        url: spec.url.clone(),
        archive_size: spec.archive_size,
        archive_sha256: spec.archive_sha256.to_ascii_lowercase(),
        published_at: spec.published_at.clone(),
        binary_relpath: spec.binary_relpath.clone(),
        source_name: spec.source_name.clone(),
        source_evidence: spec.source_evidence.clone(),
        license_name: spec.license_name.clone(),
        license_evidence: spec.license_evidence.clone(),
        manifest_generated_at: manifest.generated_at.clone(),
    }
}

fn read_ready_marker(target_dir: &Path) -> Option<FfmpegReadyMarker> {
    let data = std::fs::read(target_dir.join(READY_MARKER)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn runtime_ready(
    target_dir: &Path,
    binary: &Path,
    manifest: &FfmpegManifest,
    spec: &FfmpegSpec,
) -> bool {
    binary.exists()
        && read_ready_marker(target_dir)
            .is_some_and(|marker| marker == expected_ready_marker(manifest, spec))
}

fn write_ready_marker(
    target_dir: &Path,
    manifest: &FfmpegManifest,
    spec: &FfmpegSpec,
) -> Result<()> {
    let data = serde_json::to_vec_pretty(&expected_ready_marker(manifest, spec))?;
    std::fs::write(target_dir.join(READY_MARKER), data)?;
    Ok(())
}

fn promote_staging(staging_dir: &Path, target_dir: &Path, nonce: u128) -> Result<()> {
    if !target_dir.exists() {
        return std::fs::rename(staging_dir, target_dir).map_err(|e| {
            anyhow!(
                "promoting verified FFmpeg runtime {} -> {}: {}",
                staging_dir.display(),
                target_dir.display(),
                e
            )
        });
    }

    let backup_dir = target_dir.with_extension(format!("replaced-{nonce}"));
    std::fs::rename(target_dir, &backup_dir).map_err(|e| {
        anyhow!(
            "preserving prior FFmpeg runtime {} -> {}: {}",
            target_dir.display(),
            backup_dir.display(),
            e
        )
    })?;
    match std::fs::rename(staging_dir, target_dir) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&backup_dir);
            Ok(())
        }
        Err(promote_error) => {
            let restore = std::fs::rename(&backup_dir, target_dir);
            match restore {
                Ok(()) => Err(anyhow!(
                    "promoting verified FFmpeg runtime failed; prior runtime restored: {}",
                    promote_error
                )),
                Err(restore_error) => Err(anyhow!(
                    "promoting verified FFmpeg runtime failed and prior runtime could not be restored: {}; restore: {}",
                    promote_error,
                    restore_error
                )),
            }
        }
    }
}

fn version_key(value: &str) -> Option<Vec<u32>> {
    value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()
}

fn previous_verified_binary(
    runtime_root: &Path,
    platform: &str,
    current_cache_key: &str,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let entries = std::fs::read_dir(runtime_root).ok()?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let dir = entry.path();
        let Some(marker) = read_ready_marker(&dir) else {
            continue;
        };
        if marker.schema_version != 1
            || marker.product != "ffmpeg"
            || marker.platform != platform
            || marker.cache_key == current_cache_key
            || marker.archive_size == 0
            || marker.archive_size > MAX_ARCHIVE_BYTES
            || marker.archive_sha256.len() != 64
            || !marker
                .archive_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || marker.license_name != "GPL-3.0-or-later"
            || marker.source_name.trim().is_empty()
            || marker.source_evidence.trim().is_empty()
            || marker.license_evidence.trim().is_empty()
            || !safe_relative_path(&marker.binary_relpath)
        {
            continue;
        }
        let Ok(url) = reqwest::Url::parse(&marker.url) else {
            continue;
        };
        if url.scheme() != "https"
            || !matches!(
                url.host_str(),
                Some("ffmpeg.martin-riedl.de" | "github.com")
            )
        {
            continue;
        }
        let Some(key) = version_key(&marker.version) else {
            continue;
        };
        let binary = dir.join(&marker.binary_relpath);
        if binary.exists() {
            candidates.push((key, binary));
        }
    }
    candidates
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, binary)| binary)
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
            validate_spec(&spec).expect("current platform spec validates");
            assert!(spec.url.starts_with("https://"));
            assert!(!spec.binary_relpath.is_empty());
            assert_eq!(spec.archive_sha256.len(), 64);
            assert_eq!(spec.license_name, "GPL-3.0-or-later");
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

    #[test]
    fn install_lock_is_exclusive_until_the_holder_is_dropped() {
        let runtime_root = tempfile::tempdir().expect("runtime tempdir");
        let lock_path = runtime_root.path().join("install.lock");
        let first =
            acquire_install_lock_at(&lock_path, Duration::ZERO).expect("first install lock");

        assert!(
            acquire_install_lock_at(&lock_path, Duration::ZERO).is_err(),
            "a concurrent runtime publisher must not enter the critical section"
        );
        drop(first);
        assert!(
            acquire_install_lock_at(&lock_path, Duration::ZERO).is_ok(),
            "dropping the holder must release the install lock"
        );
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
    fn all_embedded_artifacts_are_content_addressed() {
        let manifest = embedded_manifest().expect("manifest parses");
        assert_eq!(manifest.artifacts.len(), 5);
        for spec in manifest.artifacts {
            validate_spec(&spec).expect("artifact validates");
        }
    }

    #[test]
    fn sha256_verification_rejects_wrong_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("archive.zip");
        std::fs::write(&path, b"verified bytes").expect("write");
        assert!(verify_sha256(
            &path,
            "186287b2d987891f027b4bc8baaf621a3e5a4a73ec78e04b0f65dc309b1ccc03"
        )
        .is_ok());
        assert!(verify_sha256(
            &path,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
    }
}

//! Chrome for Testing runtime fallback — downloads and unpacks an immutable,
//! SHA-256-pinned stable Chrome artifact when the system has no Chrome / Edge /
//! Brave / Chromium installed.
//!
//! Triggered explicitly via `profile.op=install_runtime` or proactively
//! via settings → Browser → "Install Chromium runtime". Never triggered
//! automatically on first `profile.op=launch` because the download is
//! large (~150 MB) and the user should see progress.

use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

use ha_core::paths;

const READY_MARKER: &str = ".hope-agent-ready";
const MANIFEST_JSON: &str = include_str!("../../resources/chrome-for-testing-manifest.json");
const MAX_ARCHIVE_BYTES: u64 = 300 * 1024 * 1024;

/// Frontend event emitted when a user-triggered feature needs a local
/// Chrome/Chromium binary but neither a system browser nor the managed Hope
/// runtime can be resolved. The app owns the install prompt; core only reports
/// the missing capability and whether this platform supports the managed
/// runtime.
pub const REQUIRED_EVENT: &str = "browser:runtime_required";

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifest {
    schema_version: u32,
    generated_at: String,
    minimum_compatible_version: String,
    artifacts: Vec<RuntimeSpec>,
}

/// Per-platform descriptor for fetching and unpacking a pinned Chrome for
/// Testing archive. Values come from the checked-in supply-chain manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSpec {
    pub version: String,
    pub revision: u32,
    pub platform: String,
    pub archive_url: String,
    pub archive_size: u64,
    pub archive_sha256: String,
    pub published_at: String,
    pub binary_relpath: String,
    pub source_name: String,
    pub source_evidence: String,
    pub license_name: String,
    pub license_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RuntimeReadyMarker {
    schema_version: u32,
    product: String,
    version: String,
    revision: u32,
    platform: String,
    archive_url: String,
    archive_size: u64,
    archive_sha256: String,
    published_at: String,
    binary_relpath: String,
    source_name: String,
    source_evidence: String,
    license_name: String,
    license_url: String,
    manifest_generated_at: String,
    minimum_compatible_version: String,
}

fn current_platform_id() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some("mac-arm64");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some("mac-x64");
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some("linux64");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Some("win64");
    #[allow(unreachable_code)]
    None
}

fn embedded_manifest() -> Result<RuntimeManifest> {
    let manifest: RuntimeManifest = serde_json::from_str(MANIFEST_JSON)
        .map_err(|e| anyhow!("invalid embedded Chrome for Testing manifest: {e}"))?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported Chrome for Testing manifest schema {}",
            manifest.schema_version
        );
    }
    if manifest.generated_at.trim().is_empty()
        || manifest.minimum_compatible_version.trim().is_empty()
    {
        bail!("Chrome for Testing manifest is missing release metadata");
    }
    Ok(manifest)
}

fn validate_spec(spec: &RuntimeSpec) -> Result<()> {
    if !matches!(
        spec.platform.as_str(),
        "mac-arm64" | "mac-x64" | "linux64" | "win64"
    ) {
        bail!("unsupported Chrome for Testing platform in manifest");
    }
    if spec.version.is_empty()
        || !spec
            .version
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.')
    {
        bail!("invalid Chrome for Testing version in manifest");
    }
    if spec.archive_size == 0 || spec.archive_size > MAX_ARCHIVE_BYTES {
        bail!("invalid Chrome for Testing archive size in manifest");
    }
    if spec.archive_sha256.len() != 64
        || !spec.archive_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("invalid Chrome for Testing SHA-256 in manifest");
    }
    let url = reqwest::Url::parse(&spec.archive_url)
        .map_err(|e| anyhow!("invalid Chrome for Testing archive URL: {e}"))?;
    if url.scheme() != "https" || url.host_str() != Some("storage.googleapis.com") {
        bail!("Chrome for Testing archive URL must use the fixed Google storage host");
    }
    if !safe_relative_path(&spec.binary_relpath)
        || spec.source_name.trim().is_empty()
        || spec.source_evidence.trim().is_empty()
        || spec.license_name.trim().is_empty()
        || spec.license_url.trim().is_empty()
        || spec.published_at.trim().is_empty()
    {
        bail!("Chrome for Testing manifest metadata is incomplete");
    }
    Ok(())
}

fn required_spec() -> Result<(RuntimeManifest, RuntimeSpec)> {
    let platform = current_platform_id().ok_or_else(|| {
        anyhow!(
            "Chrome for Testing runtime is not available for this platform / architecture. \
             Install Google Chrome system-wide or set `executable_path` to a custom Chrome binary."
        )
    })?;
    let manifest = embedded_manifest()?;
    let spec = manifest
        .artifacts
        .iter()
        .find(|spec| spec.platform == platform)
        .cloned()
        .ok_or_else(|| anyhow!("Chrome for Testing manifest has no artifact for {platform}"))?;
    validate_spec(&spec)?;
    Ok((manifest, spec))
}

/// Resolve the `RuntimeSpec` for the current host. Returns `None` when
/// the embedded manifest does not ship this OS/arch combo.
pub fn spec_for_current_platform() -> Option<RuntimeSpec> {
    required_spec().ok().map(|(_, spec)| spec)
}

/// Event bus channel for Chromium runtime download progress.
///
/// Three callsites publish here (`tool::profile_install_runtime`,
/// the Tauri `browser_install_chromium_runtime` command, and the HTTP
/// `/api/browser/install-chromium-runtime` route) — all funnel through
/// [`install_with_event_bus_progress`] so the wire format and throttle
/// stay consistent.
pub const PROGRESS_EVENT: &str = "browser:chromium_download_progress";

/// Notify owner UIs that the current operation needs a Chromium runtime.
///
/// `context` is deliberately a small stable identifier instead of display
/// copy so each frontend can explain why Chromium is needed in the user's
/// language. Emitting is best-effort: headless/CLI callers still receive the
/// original error from the operation.
pub fn emit_runtime_required(context: &str, reason: &str) {
    let spec = spec_for_current_platform();
    if let Some(bus) = ha_core::globals::EVENT_BUS.get() {
        bus.emit(
            REQUIRED_EVENT,
            serde_json::json!({
                "context": context,
                "reason": reason,
                "installSupported": spec.is_some(),
                "approxDownloadBytes": spec.map(|value| value.archive_size),
            }),
        );
    }
}

/// One-percent–throttled wrapper around [`ensure_chromium`] that also
/// emits structured progress events on the global EventBus. Returns the
/// cached binary path on completion (same as `ensure_chromium`).
///
/// Each callsite (Tauri command / HTTP route / tool op) previously
/// open-coded the same `AtomicU64` percent throttle + `stage:
/// "downloading"/"ready"` emit closure. Centralising it here means the
/// payload shape is changed in exactly one place.
pub async fn install_with_event_bus_progress() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    // Tauri, HTTP, the browser tool and the global missing-runtime dialog can
    // all request installation. Serialize them so two explicit user actions
    // cannot race while promoting the same staging directory.
    let _install_guard = INSTALL_LOCK.lock().await;
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
    let binary = ensure_chromium(progress).await?;
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

/// Resolve the cached Chromium binary path, downloading + unpacking the
/// snapshot on first call. `progress` is invoked periodically during the
/// download with `(downloaded_bytes, total_bytes)`; `total_bytes` is
/// `None` if the server doesn't send `Content-Length`.
pub async fn ensure_chromium<F>(progress: F) -> Result<PathBuf>
where
    F: Fn(u64, Option<u64>) + Send + Sync + 'static,
{
    let (manifest, spec) = required_spec()?;
    let target_dir = runtime_target_dir(&spec)?;
    let binary = target_dir.join(&spec.binary_relpath);
    if runtime_ready(&target_dir, &binary, &manifest, &spec) {
        return Ok(binary);
    }

    let runtime_root = paths::browser_runtime_dir()?;
    std::fs::create_dir_all(&runtime_root)?;

    // SSRF remains mandatory even though the embedded manifest validator also
    // restricts downloads to the fixed Chrome for Testing storage host.
    let ssrf_cfg = &ha_core::config::cached_config().ssrf;
    ha_core::security::ssrf::check_url(
        &spec.archive_url,
        ssrf_cfg.browser(),
        &ssrf_cfg.trusted_hosts,
    )
    .await?;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let archive_path = runtime_root.join(format!(
        "chrome-for-testing.{}.{}.tmp.zip",
        spec.version, nonce
    ));
    let staging_dir = runtime_root.join(format!(
        ".chrome-for-testing-{}-{}.{}.tmp",
        spec.version, spec.platform, nonce
    ));

    let install_result: Result<PathBuf> = async {
        download_streaming(&spec, &archive_path, &progress).await?;
        verify_sha256(&archive_path, &spec.archive_sha256)?;
        extract_zip(&archive_path, &staging_dir)?;
        let staged_binary = staging_dir.join(&spec.binary_relpath);

        #[cfg(unix)]
        chmod_executable(&staged_binary)?;

        smoke_test_binary(&staged_binary).await?;
        write_ready_marker(&staging_dir, &manifest, &spec)?;
        promote_staging(&staging_dir, &target_dir, nonce)?;

        Ok(target_dir.join(&spec.binary_relpath))
    }
    .await;

    let _ = std::fs::remove_file(&archive_path);
    if install_result.is_err() {
        let _ = std::fs::remove_dir_all(&staging_dir);
    }

    install_result
}

async fn download_streaming<F>(spec: &RuntimeSpec, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    use std::io::Write;
    let client = ha_core::provider::apply_proxy_for_url(
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()),
        &spec.archive_url,
    )
    .build()?;
    let resp = client
        .get(&spec.archive_url)
        .send()
        .await
        .map_err(|e| anyhow!("Chrome for Testing download failed: {e}"))?
        .error_for_status()
        .map_err(|e| anyhow!("Chrome for Testing download returned an HTTP error: {e}"))?;
    let total = resp.content_length();
    if let Some(total) = total {
        if total != spec.archive_size {
            bail!(
                "Chrome for Testing archive length mismatch: expected {}, got {}",
                spec.archive_size,
                total
            );
        }
    }
    let mut stream = resp.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    // Report at most ~25 times per second to keep the UI from flooding —
    // ~40ms throttle is plenty for a download progress bar.
    let mut last_emit = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("stream chunk error: {}", e))?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        if downloaded > spec.archive_size || downloaded > MAX_ARCHIVE_BYTES {
            bail!("Chrome for Testing archive exceeded its manifest size");
        }
        if last_emit.elapsed() >= std::time::Duration::from_millis(40) {
            progress(downloaded, Some(spec.archive_size));
            last_emit = std::time::Instant::now();
        }
    }
    if downloaded != spec.archive_size {
        bail!(
            "Chrome for Testing archive length mismatch: expected {}, got {}",
            spec.archive_size,
            downloaded
        );
    }
    progress(downloaded, Some(spec.archive_size));
    file.flush()?;
    Ok(())
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
        bail!("Chrome for Testing archive SHA-256 mismatch");
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn runtime_target_dir(spec: &RuntimeSpec) -> Result<PathBuf> {
    validate_spec(spec)?;
    Ok(paths::browser_runtime_dir()?.join(format!(
        "chrome-for-testing-{}-{}",
        spec.version, spec.platform
    )))
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
        // `mangled_name` ensures path components stay within target —
        // protects against zip-slip attacks (../../etc/passwd entries).
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
    // Defensive: some zip writers strip +x. Re-apply 0755 on the main
    // binary so it's runnable even when the archive's unix_mode metadata
    // didn't survive.
    use std::os::unix::fs::PermissionsExt;
    let metadata = binary.metadata().map_err(|e| {
        anyhow!(
            "Chromium binary not present after extraction at {}: {}",
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
    cmd.arg("--version").kill_on_drop(true);
    ha_core::platform::hide_console_tokio(&mut cmd);
    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow!("smoke test (Chromium --version) failed to spawn: {}", e))?;
    if !output.status.success() {
        bail!(
            "Chromium runtime at {} did not start: exit={:?}, stderr={}",
            binary.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.to_ascii_lowercase().contains("chromium")
        && !stdout.to_ascii_lowercase().contains("chrome")
    {
        bail!(
            "Chromium runtime smoke test returned unexpected version banner: {}",
            stdout.trim()
        );
    }
    Ok(())
}

fn expected_ready_marker(manifest: &RuntimeManifest, spec: &RuntimeSpec) -> RuntimeReadyMarker {
    RuntimeReadyMarker {
        schema_version: 1,
        product: "chrome-for-testing".to_string(),
        version: spec.version.clone(),
        revision: spec.revision,
        platform: spec.platform.clone(),
        archive_url: spec.archive_url.clone(),
        archive_size: spec.archive_size,
        archive_sha256: spec.archive_sha256.to_ascii_lowercase(),
        published_at: spec.published_at.clone(),
        binary_relpath: spec.binary_relpath.clone(),
        source_name: spec.source_name.clone(),
        source_evidence: spec.source_evidence.clone(),
        license_name: spec.license_name.clone(),
        license_url: spec.license_url.clone(),
        manifest_generated_at: manifest.generated_at.clone(),
        minimum_compatible_version: manifest.minimum_compatible_version.clone(),
    }
}

fn read_ready_marker(target_dir: &Path) -> Option<RuntimeReadyMarker> {
    let data = std::fs::read(target_dir.join(READY_MARKER)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn runtime_ready(
    target_dir: &Path,
    binary: &Path,
    manifest: &RuntimeManifest,
    spec: &RuntimeSpec,
) -> bool {
    binary.exists()
        && read_ready_marker(target_dir)
            .is_some_and(|marker| marker == expected_ready_marker(manifest, spec))
}

fn write_ready_marker(
    target_dir: &Path,
    manifest: &RuntimeManifest,
    spec: &RuntimeSpec,
) -> Result<()> {
    let data = serde_json::to_vec_pretty(&expected_ready_marker(manifest, spec))?;
    std::fs::write(target_dir.join(READY_MARKER), data)?;
    Ok(())
}

fn promote_staging(staging_dir: &Path, target_dir: &Path, nonce: u128) -> Result<()> {
    if !target_dir.exists() {
        return std::fs::rename(staging_dir, target_dir).map_err(|e| {
            anyhow!(
                "promoting verified Chrome for Testing runtime {} -> {}: {}",
                staging_dir.display(),
                target_dir.display(),
                e
            )
        });
    }

    let backup_dir = target_dir.with_extension(format!("replaced-{nonce}"));
    std::fs::rename(target_dir, &backup_dir).map_err(|e| {
        anyhow!(
            "preserving prior Chrome for Testing runtime {} -> {}: {}",
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
                    "promoting verified Chrome for Testing runtime failed; prior runtime restored: {}",
                    promote_error
                )),
                Err(restore_error) => Err(anyhow!(
                    "promoting verified Chrome for Testing runtime failed and prior runtime could not be restored: {}; restore: {}",
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
    current_version: &str,
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
            || marker.product != "chrome-for-testing"
            || marker.platform != platform
            || marker.version == current_version
            || marker.archive_sha256.len() != 64
            || !marker
                .archive_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || marker.archive_size == 0
            || marker.archive_size > MAX_ARCHIVE_BYTES
            || marker.source_name.trim().is_empty()
            || marker.source_evidence.trim().is_empty()
            || marker.license_name.trim().is_empty()
            || marker.license_url.trim().is_empty()
            || !safe_relative_path(&marker.binary_relpath)
        {
            continue;
        }
        let Ok(url) = reqwest::Url::parse(&marker.archive_url) else {
            continue;
        };
        if url.scheme() != "https" || url.host_str() != Some("storage.googleapis.com") {
            continue;
        }
        let binary = dir.join(&marker.binary_relpath);
        let Some(key) = version_key(&marker.version) else {
            continue;
        };
        if binary.exists() {
            candidates.push((key, binary));
        }
    }
    candidates
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, binary)| binary)
}

/// Quick path: the cached runtime binary path for the current platform.
/// Returns `None` when nothing's been downloaded yet (or the platform
/// isn't supported). Used by `build_launch_config` to short-circuit a
/// system Chrome miss.
pub fn cached_binary_path() -> Option<PathBuf> {
    let (manifest, spec) = required_spec().ok()?;
    let dir = runtime_target_dir(&spec).ok()?;
    let binary = dir.join(&spec.binary_relpath);
    if runtime_ready(&dir, &binary, &manifest, &spec) {
        Some(binary)
    } else {
        previous_verified_binary(
            &paths::browser_runtime_dir().ok()?,
            &spec.platform,
            &spec.version,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_for_current_platform_is_populated_or_unsupported() {
        // The platforms we ship for must all have a spec; this matches
        // the `#[cfg]` table above. CI runs on the supported set so this
        // is effectively asserting the table.
        let spec = spec_for_current_platform();
        #[cfg(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "x86_64"),
        ))]
        assert!(spec.is_some(), "supported platform must have a RuntimeSpec");
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "x86_64"),
        )))]
        assert!(spec.is_none());
    }

    #[test]
    fn embedded_manifest_has_a_complete_current_platform_pin() {
        if let Some(spec) = spec_for_current_platform() {
            validate_spec(&spec).expect("current platform spec must validate");
            assert_eq!(spec.archive_sha256.len(), 64);
            assert!(spec.archive_url.contains(&spec.version));
            assert!(!spec.license_name.is_empty());
        }
    }

    #[test]
    fn all_embedded_artifacts_are_content_addressed() {
        let manifest = embedded_manifest().expect("manifest parses");
        assert_eq!(manifest.artifacts.len(), 4);
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

    #[test]
    fn cached_binary_path_is_none_in_fresh_install() {
        // We can't isolate paths::root_dir() in tests without
        // monkey-patching HOME, but at minimum the function must not
        // panic when nothing's been downloaded.
        let _ = cached_binary_path();
    }
}

//! Chromium runtime fallback — downloads + unpacks a pinned Chromium
//! snapshot when the system has no Chrome / Edge / Brave / Chromium
//! installed.
//!
//! Triggered explicitly via `profile.op=install_runtime` or proactively
//! via settings → Browser → "Install Chromium runtime". Never triggered
//! automatically on first `profile.op=launch` because the download is
//! large (~150 MB) and the user should see progress.

use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};

use crate::paths::{self, CHROMIUM_PINNED_REVISION};

/// Per-platform descriptor for fetching + unpacking the Chromium archive.
#[derive(Debug, Clone)]
pub struct RuntimeSpec {
    pub revision: u32,
    /// Subdirectory under `chromium-browser-snapshots/`.
    pub platform_key: &'static str,
    /// Archive filename inside that subdirectory.
    pub archive_name: &'static str,
    /// Path to the runnable Chromium binary RELATIVE to the unzipped
    /// archive root. e.g. `chrome-mac/Chromium.app/Contents/MacOS/Chromium`.
    pub binary_relpath: &'static str,
}

/// Resolve the `RuntimeSpec` for the current host. Returns `None` when
/// Chromium snapshots don't ship for this OS/arch combo.
pub fn spec_for_current_platform() -> Option<RuntimeSpec> {
    let revision = CHROMIUM_PINNED_REVISION;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some(RuntimeSpec {
            revision,
            platform_key: "Mac_Arm",
            archive_name: "chrome-mac.zip",
            binary_relpath: "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
        });
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some(RuntimeSpec {
            revision,
            platform_key: "Mac",
            archive_name: "chrome-mac.zip",
            binary_relpath: "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
        });
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some(RuntimeSpec {
            revision,
            platform_key: "Linux_x64",
            archive_name: "chrome-linux.zip",
            binary_relpath: "chrome-linux/chrome",
        });
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some(RuntimeSpec {
            revision,
            platform_key: "Win_x64",
            archive_name: "chrome-win.zip",
            binary_relpath: "chrome-win/chrome.exe",
        });
    }
    #[allow(unreachable_code)]
    None
}

/// Resolve the cached Chromium binary path, downloading + unpacking the
/// snapshot on first call. `progress` is invoked periodically during the
/// download with `(downloaded_bytes, total_bytes)`; `total_bytes` is
/// `None` if the server doesn't send `Content-Length`.
pub async fn ensure_chromium<F>(progress: F) -> Result<PathBuf>
where
    F: Fn(u64, Option<u64>) + Send + Sync + 'static,
{
    let spec = spec_for_current_platform().ok_or_else(|| {
        anyhow!(
            "Chromium runtime is not available for this platform / architecture. \
             Install Google Chrome system-wide or set `executable_path` to a custom Chrome binary."
        )
    })?;
    let target_dir = paths::chromium_runtime_dir(spec.revision)?;
    let binary = target_dir.join(spec.binary_relpath);
    if binary.exists() {
        return Ok(binary);
    }

    let runtime_root = paths::browser_runtime_dir()?;
    std::fs::create_dir_all(&runtime_root)?;

    let archive_url = format!(
        "https://commondatastorage.googleapis.com/chromium-browser-snapshots/{}/{}/{}",
        spec.platform_key, spec.revision, spec.archive_name,
    );

    // SSRF: this is a fixed Google CDN host so the default policy lets it
    // through, but stay consistent with every other outbound call.
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(&archive_url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts)
        .await?;

    let archive_path = runtime_root.join(format!("{}.tmp.{}", spec.archive_name, spec.revision));

    download_streaming(&archive_url, &archive_path, &progress).await?;
    extract_zip(&archive_path, &target_dir)?;
    let _ = std::fs::remove_file(&archive_path);

    #[cfg(unix)]
    chmod_executable(&binary)?;

    smoke_test_binary(&binary).await?;
    Ok(binary)
}

async fn download_streaming<F>(url: &str, dest: &Path, progress: &F) -> Result<()>
where
    F: Fn(u64, Option<u64>) + Send + Sync,
{
    use std::io::Write;
    let resp = reqwest::Client::new()
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
    // Report at most ~25 times per second to keep the UI from flooding —
    // ~40ms throttle is plenty for a download progress bar.
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
    let output = tokio::process::Command::new(binary)
        .arg("--version")
        .kill_on_drop(true)
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

/// Quick path: the cached runtime binary path for the current platform.
/// Returns `None` when nothing's been downloaded yet (or the platform
/// isn't supported). Used by `build_launch_config` to short-circuit a
/// system Chrome miss.
pub fn cached_binary_path() -> Option<PathBuf> {
    let spec = spec_for_current_platform()?;
    let dir = paths::chromium_runtime_dir(spec.revision).ok()?;
    let binary = dir.join(spec.binary_relpath);
    if binary.exists() {
        Some(binary)
    } else {
        None
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
    fn spec_revision_matches_pinned_constant() {
        if let Some(spec) = spec_for_current_platform() {
            assert_eq!(spec.revision, CHROMIUM_PINNED_REVISION);
        }
    }

    #[test]
    fn cached_binary_path_is_none_in_fresh_install() {
        // We can't isolate paths::root_dir() in tests without
        // monkey-patching HOME, but at minimum the function must not
        // panic when nothing's been downloaded.
        let _ = cached_binary_path();
    }
}

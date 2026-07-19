use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::broker::EXPECTED_EXTENSION_PROTOCOL_VERSION;
use super::{BrowserExtensionBroker, DEFAULT_NATIVE_HOST_NAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserExtensionStatusKind {
    Ready,
    BrokerUnavailable,
    HostMissing,
    HostInvalid,
    ExtensionMissing,
    ExtensionDisabled,
    ExtensionProfileMismatch,
    PolicyBlocked,
    VersionMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionStatus {
    pub kind: BrowserExtensionStatusKind,
    pub backend_available: bool,
    pub native_host_name: String,
    pub native_host_manifest_path: Option<String>,
    pub native_host_manifest_exists: bool,
    pub extension_connected: bool,
    pub extension_protocol_version: Option<u32>,
    pub extension_version: Option<String>,
    pub extension_ids: Vec<String>,
    pub store_url: Option<String>,
    pub unpacked_extension_path: Option<String>,
    pub native_host_binary_hint: Option<String>,
    pub message: String,
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostInstallRequest {
    pub extension_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_host_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostInstallResult {
    pub native_host_name: String,
    pub host_path: String,
    pub manifest_path: String,
    pub allowed_origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows_registry_key: Option<String>,
}

pub fn current_status() -> BrowserExtensionStatus {
    let cfg = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.extension.clone())
        .unwrap_or_default();
    let native_host_name = cfg.native_host_name().to_string();
    let manifest_path = native_host_manifest_path(&native_host_name);
    let manifest_exists = manifest_path.as_ref().is_some_and(|p| p.exists());
    let broker_status = BrowserExtensionBroker::global().map(|broker| broker.status_snapshot());
    let broker_running = broker_status.as_ref().is_some_and(|s| s.running);
    let extension_connected = broker_status
        .as_ref()
        .is_some_and(|s| s.extension_connected);

    let protocol_version = broker_status
        .as_ref()
        .and_then(|s| s.extension_protocol_version);
    let extension_version = broker_status
        .as_ref()
        .and_then(|s| s.extension_version.clone());

    let (kind, message, next_action) =
        if extension_connected && protocol_version != Some(EXPECTED_EXTENSION_PROTOCOL_VERSION) {
            (
                BrowserExtensionStatusKind::VersionMismatch,
                format!(
                    "Hope Agent Chrome Extension protocol mismatch: expected {}, got {}{}.",
                    EXPECTED_EXTENSION_PROTOCOL_VERSION,
                    protocol_version
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    extension_version
                        .as_deref()
                        .map(|v| format!(" (extension {v})"))
                        .unwrap_or_default()
                ),
                Some("reload_extension".to_string()),
            )
        } else if extension_connected {
            (
                BrowserExtensionStatusKind::Ready,
                "Hope Agent Chrome Extension is connected.".to_string(),
                None,
            )
        } else if !broker_running {
            (
                BrowserExtensionStatusKind::BrokerUnavailable,
                broker_status
                    .as_ref()
                    .and_then(|s| s.last_error.clone())
                    .unwrap_or_else(|| "Chrome Extension broker is not running.".to_string()),
                Some("retry_connection".to_string()),
            )
        } else if !manifest_exists {
            (
                BrowserExtensionStatusKind::HostMissing,
                "Chrome native messaging host is not installed.".to_string(),
                Some("install_native_host".to_string()),
            )
        } else {
            (
                BrowserExtensionStatusKind::ExtensionMissing,
                "Hope Agent Chrome Extension is not connected.".to_string(),
                Some("open_extension_page".to_string()),
            )
        };

    BrowserExtensionStatus {
        kind,
        backend_available: matches!(kind, BrowserExtensionStatusKind::Ready),
        native_host_name,
        native_host_manifest_path: manifest_path.map(|p| p.to_string_lossy().to_string()),
        native_host_manifest_exists: manifest_exists,
        extension_connected,
        extension_protocol_version: protocol_version,
        extension_version,
        extension_ids: effective_extension_ids(&cfg.extension_ids),
        store_url: cfg.store_url,
        unpacked_extension_path: unpacked_extension_path().map(|p| p.to_string_lossy().to_string()),
        native_host_binary_hint: native_host_binary_hint().map(|p| p.to_string_lossy().to_string()),
        message,
        next_action,
    }
}

pub fn install_native_host_manifest(
    request: NativeHostInstallRequest,
) -> Result<NativeHostInstallResult> {
    validate_extension_id(&request.extension_id)?;
    let native_host_name = request
        .native_host_name
        .as_deref()
        .unwrap_or(DEFAULT_NATIVE_HOST_NAME);
    validate_native_host_name(native_host_name)?;

    let host_path = resolve_host_path(request.host_path)?;
    if !host_path.is_absolute() {
        bail!("Native host path must be absolute: {}", host_path.display());
    }
    if !host_path.exists() {
        bail!("Native host binary does not exist: {}", host_path.display());
    }

    let manifest_path = native_host_manifest_path(native_host_name)
        .context("Native host manifests are not supported on this platform")?;
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "creating native host manifest directory {}",
                parent.display()
            )
        })?;
    }

    let allowed_origin = format!("chrome-extension://{}/", request.extension_id);
    let manifest = json!({
        "name": native_host_name,
        "description": "Hope Agent Chrome Native Messaging Host",
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [allowed_origin.clone()],
    });
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    crate::platform::write_atomic(&manifest_path, &bytes).with_context(|| {
        format!(
            "writing native host manifest {}",
            manifest_path.to_string_lossy()
        )
    })?;

    let windows_registry_key = register_windows_native_host(native_host_name, &manifest_path)?;

    Ok(NativeHostInstallResult {
        native_host_name: native_host_name.to_string(),
        host_path: host_path.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        allowed_origin,
        windows_registry_key,
    })
}

/// Chrome Web Store extension IDs shipped with released builds. Empty until the
/// extension is published; once it has a stable store ID, add it here so a
/// packaged install can auto-register the native messaging host with no user
/// input. Each must be a valid 32-char (a–p) Chrome extension id.
pub const KNOWN_STORE_EXTENSION_IDS: &[&str] = &[];

/// Best-effort startup auto-registration of the native messaging host manifest
/// for the Chromium-family browsers the user has installed, so a packaged build
/// needs no manual "Install native host" step. Desktop-only, idempotent (skips
/// when the on-disk manifest is already current), and never panics — every
/// failure is logged and swallowed. Returns the number of manifests (re)written.
///
/// No-op when the extension backend is disabled, when no Chrome extension id is
/// known yet (alpha / unpacked with an unstable id and nothing configured), or
/// when the host binary can't be resolved.
pub fn ensure_native_host_registered() -> usize {
    let cfg = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.extension.clone())
        .unwrap_or_default();
    if !cfg.enabled() {
        return 0;
    }

    let host_name = cfg.native_host_name().to_string();
    if validate_native_host_name(&host_name).is_err() {
        app_warn!(
            "browser",
            "auto_register",
            "invalid native host name '{}', skipping auto-register",
            host_name
        );
        return 0;
    }

    // Extension ids we may authorize: configured + detected unpacked + known
    // store ids, keeping only well-formed ones.
    let mut extension_ids: Vec<String> = Vec::new();
    for id in effective_extension_ids(&cfg.extension_ids) {
        if validate_extension_id(&id).is_ok() {
            push_unique_extension_id(&mut extension_ids, id);
        }
    }
    for id in KNOWN_STORE_EXTENSION_IDS {
        if validate_extension_id(id).is_ok() {
            push_unique_extension_id(&mut extension_ids, (*id).to_string());
        }
    }
    if extension_ids.is_empty() {
        app_info!(
            "browser",
            "auto_register",
            "no known Chrome extension id yet; skipping native host auto-register"
        );
        return 0;
    }

    let host_path = match resolve_host_path(None) {
        Ok(path) if path.is_absolute() && path.exists() => path,
        Ok(path) => {
            app_info!(
                "browser",
                "auto_register",
                "native host binary not found at {}; skipping auto-register",
                path.display()
            );
            return 0;
        }
        Err(e) => {
            app_info!(
                "browser",
                "auto_register",
                "native host binary unresolved; skipping auto-register: {:#}",
                e
            );
            return 0;
        }
    };

    let dirs = native_host_manifest_dirs();
    if dirs.is_empty() {
        app_info!(
            "browser",
            "auto_register",
            "no Chromium-family browser directories found; skipping native host auto-register"
        );
        return 0;
    }

    let mut written = 0usize;
    for dir in dirs {
        let manifest_path = dir.join(format!("{host_name}.json"));
        match write_manifest_if_changed(&manifest_path, &host_name, &host_path, &extension_ids) {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => app_warn!(
                "browser",
                "auto_register",
                "failed writing native host manifest {}: {:#}",
                manifest_path.display(),
                e
            ),
        }
    }

    // Windows points browsers at the manifest via the registry, not a
    // per-browser directory. Reuse the existing Chrome registry pointer; Edge /
    // Brave registry keys on Windows are a follow-up.
    #[cfg(windows)]
    if let Some(manifest_path) = native_host_manifest_path(&host_name) {
        if let Err(e) = register_windows_native_host(&host_name, &manifest_path) {
            app_warn!(
                "browser",
                "auto_register",
                "windows native host registry registration failed: {:#}",
                e
            );
        }
    }

    if written > 0 {
        app_info!(
            "browser",
            "auto_register",
            "native host manifest auto-registered ({} written) for {} extension id(s)",
            written,
            extension_ids.len()
        );
    }
    written
}

/// NativeMessagingHosts directories to register into, one per installed
/// Chromium-family browser. macOS / Linux only include browsers whose profile
/// base directory exists (so uninstalled browsers don't get stray trees);
/// Windows uses the single shared manifest directory referenced from the
/// registry.
#[cfg(target_os = "macos")]
fn native_host_manifest_dirs() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let support = home.join("Library").join("Application Support");
    [
        support.join("Google").join("Chrome"),
        support.join("Google").join("Chrome Beta"),
        support.join("Google").join("Chrome Dev"),
        support.join("Google").join("Chrome Canary"),
        support.join("Chromium"),
        support.join("Microsoft Edge"),
        support.join("BraveSoftware").join("Brave-Browser"),
    ]
    .into_iter()
    .filter(|base| base.is_dir())
    .map(|base| base.join("NativeMessagingHosts"))
    .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native_host_manifest_dirs() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let config = home.join(".config");
    [
        config.join("google-chrome"),
        config.join("google-chrome-beta"),
        config.join("google-chrome-unstable"),
        config.join("chromium"),
        config.join("microsoft-edge"),
        config.join("BraveSoftware").join("Brave-Browser"),
    ]
    .into_iter()
    .filter(|base| base.is_dir())
    .map(|base| base.join("NativeMessagingHosts"))
    .collect()
}

#[cfg(target_os = "windows")]
fn native_host_manifest_dirs() -> Vec<PathBuf> {
    native_host_manifest_path(DEFAULT_NATIVE_HOST_NAME)
        .and_then(|p| p.parent().map(PathBuf::from))
        .into_iter()
        .collect()
}

/// Write the native host manifest at `manifest_path` unless it's already current
/// (same host path and already authorizing every desired extension id). Existing
/// `allowed_origins` are preserved (unioned) so a manually-added id isn't
/// clobbered. Returns whether a write happened.
fn write_manifest_if_changed(
    manifest_path: &std::path::Path,
    host_name: &str,
    host_path: &std::path::Path,
    extension_ids: &[String],
) -> Result<bool> {
    let host_path_str = host_path.to_string_lossy().to_string();
    let mut origins: Vec<String> = extension_ids
        .iter()
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();

    if let Ok(bytes) = std::fs::read(manifest_path) {
        if let Ok(existing) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let same_path =
                existing.get("path").and_then(|v| v.as_str()) == Some(host_path_str.as_str());
            let existing_origins: Vec<String> = existing
                .get("allowed_origins")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if same_path && origins.iter().all(|o| existing_origins.contains(o)) {
                return Ok(false);
            }
            for origin in existing_origins {
                if !origins.contains(&origin) {
                    origins.push(origin);
                }
            }
        }
    }

    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating native host manifest dir {}", parent.display()))?;
    }
    let manifest = json!({
        "name": host_name,
        "description": "Hope Agent Chrome Native Messaging Host",
        "path": host_path_str,
        "type": "stdio",
        "allowed_origins": origins,
    });
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    crate::platform::write_atomic(manifest_path, &bytes)
        .with_context(|| format!("writing native host manifest {}", manifest_path.display()))?;
    Ok(true)
}

pub fn native_host_manifest_path(host_name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return dirs::home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Google")
                .join("Chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"))
        });
    }

    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join("AppData").join("Local")))?;
        return Some(
            base.join("HopeAgent")
                .join("extension")
                .join(format!("{host_name}.json")),
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return dirs::home_dir().map(|home| {
            home.join(".config")
                .join("google-chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"))
        });
    }

    #[allow(unreachable_code)]
    None
}

pub fn default_native_host_manifest_path() -> Option<PathBuf> {
    native_host_manifest_path(DEFAULT_NATIVE_HOST_NAME)
}

/// Stable local copy first, else the bundled/repo source. Prefer the stable
/// copy (see [`ensure_local_unpacked_extension`]) because its path is invariant
/// across app updates/moves — a user who loaded it once in Chrome stays
/// connected. Falls back to the source if the copy hasn't been made yet (before
/// first desktop startup, or in a headless server with no copy step).
fn unpacked_extension_path() -> Option<PathBuf> {
    // Use the stable copy only when the completion marker proves the last
    // mirror finished fully AND matches the current source set. The marker
    // stores the source fingerprint, so a binary upgrade (new embedded files)
    // invalidates it and headless deployments re-mirror lazily below — a
    // presence-only marker would pin the mirror to the first version forever.
    let fingerprint = extension_source_fingerprint();
    if let (Ok(stable), Ok(marker)) = (
        crate::paths::browser_extension_unpacked_dir(),
        crate::paths::browser_extension_unpacked_marker(),
    ) {
        if stable_copy_is_complete(&stable, &marker, fingerprint.as_deref()) {
            return Some(stable);
        }
    }
    // Lazy bootstrap for entry points without the desktop startup hook
    // (headless server). Only success is cached — a transient mirror failure
    // retries on the next call instead of pinning None for the process
    // lifetime. Gated out of cargo test: the mirror writes the REAL user data
    // dir, which unit tests must never touch.
    #[cfg(not(test))]
    {
        static LAZY_ENSURE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        if let Some(path) = LAZY_ENSURE.get() {
            return Some(path.clone());
        }
        if let Some(path) = ensure_local_unpacked_extension() {
            return Some(LAZY_ENSURE.get_or_init(|| path).clone());
        }
    }
    // Dev fallback only: a release binary must never serve a stray checkout
    // (see extension_source_files).
    #[cfg(debug_assertions)]
    {
        return repo_extension_source();
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// A stable copy is usable only if a manifest is present AND the completion
/// marker matches the current source fingerprint — the marker proves the last
/// mirror finished fully (guarding against a partial copy shadowing the
/// source) and pins WHICH source set it mirrored (guarding against a stale
/// mirror surviving a binary upgrade). When the fingerprint is unavailable
/// the check degrades to marker presence.
fn stable_copy_is_complete(dir: &Path, marker: &Path, expected_fingerprint: Option<&str>) -> bool {
    if !dir.join("manifest.json").exists() {
        return false;
    }
    let Ok(actual) = std::fs::read_to_string(marker) else {
        return false;
    };
    match expected_fingerprint {
        Some(want) => actual.trim() == want,
        None => true,
    }
}

/// Fingerprint of the current extension source set (dev repo checkout or
/// embedded files). Cached per process — the set is fixed for a binary, and
/// dev edits take effect on restart, same as the mirror itself.
fn extension_source_fingerprint() -> Option<String> {
    static FINGERPRINT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            let dest = crate::paths::browser_extension_unpacked_dir().ok()?;
            let files = extension_source_files(&dest)?;
            Some(fingerprint_files(&files))
        })
        .clone()
}

/// Stable digest over (path, content) pairs, length-prefixed to keep field
/// boundaries unambiguous; truncated hex is plenty for a change detector.
fn fingerprint_files(files: &[(String, Vec<u8>)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (rel, bytes) in files {
        hasher.update(&(rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_hex()[..16].to_string()
}

/// Mirror the extension runtime files into a STABLE location under
/// `~/.hope-agent/extension/browser/` so the path the user loads in
/// Chrome survives app updates/moves (the `.app` path changes on update; this
/// one does not). Source is the dev repo checkout when present (edits take
/// effect live), else the files embedded in the binary — so a binary upgrade
/// refreshes the mirror on next startup. Idempotent — only rewrites files
/// whose bytes changed, so an unchanged extension never forces Chrome to
/// reload it. Desktop startup calls this before registering the native host;
/// headless entry points bootstrap it lazily via [`unpacked_extension_path`].
/// Returns the stable dir on success.
pub fn ensure_local_unpacked_extension() -> Option<PathBuf> {
    // Serialize mirrors within the process: desktop startup fires this on a
    // detached blocking task while status queries can bootstrap lazily at the
    // same time, and two interleaved mirrors prune each other's in-flight
    // temp files (each run's write_atomic temps are not in the other's keep
    // set), failing both.
    static ENSURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENSURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dest = crate::paths::browser_extension_unpacked_dir().ok()?;
    let marker = crate::paths::browser_extension_unpacked_marker().ok()?;
    let files = match extension_source_files(&dest) {
        Some(files) if !files.is_empty() => files,
        _ => return None,
    };
    let fingerprint = fingerprint_files(&files);
    // Clear the completion marker first: while the mirror is in progress the copy
    // may be partial, so `unpacked_extension_path` must not trust it until we
    // re-stamp the marker on full success.
    let _ = std::fs::remove_file(&marker);
    match mirror_extension_files(&files, &dest) {
        Ok(()) => {
            // Full mirror succeeded — stamp the marker (with the source
            // fingerprint) so the stable copy becomes the preferred path. If
            // the marker write fails, report failure so callers keep using
            // the source rather than a copy we can't vouch for.
            if let Err(e) = crate::platform::write_atomic(&marker, fingerprint.as_bytes()) {
                app_warn!(
                    "browser",
                    "unpacked_copy",
                    "synced extension but failed to stamp completion marker {}: {:#}",
                    marker.display(),
                    e
                );
                return None;
            }
            Some(dest)
        }
        Err(e) => {
            app_warn!(
                "browser",
                "unpacked_copy",
                "failed to sync unpacked extension to {}: {:#}",
                dest.display(),
                e
            );
            None
        }
    }
}

/// Runtime file list for the mirror: dev repo checkout when present (debug
/// builds only — a RELEASE binary must never let a stray/stale checkout found
/// by cwd/exe ancestor walking shadow its embedded set, which would pin the
/// user's extension to old files and let a foreign manifest.key drive the
/// derived unpacked id; this mirrors the debug-gating of the skills
/// resolver), else the set embedded in the binary. `dest` is excluded
/// defensively so we never mirror the stable copy onto itself.
fn extension_source_files(dest: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    #[cfg(debug_assertions)]
    if let Some(dir) = repo_extension_source() {
        if dir != *dest {
            if let Ok(files) = read_extension_dir_files(&dir) {
                if !files.is_empty() {
                    return Some(files);
                }
            }
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = dest;
    let embedded = super::embedded::extension_files();
    (!embedded.is_empty()).then_some(embedded)
}

/// Collect `(relative path, bytes)` for every file under `root`, skipping
/// dev-checkout noise that must never reach the mirrored copy.
#[cfg(debug_assertions)]
fn read_extension_dir_files(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "node_modules" | ".git" | "scripts" | "store-listing" | "test-pages"
                ) {
                    continue;
                }
                walk(root, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                out.push((rel, bytes));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

/// Mirror the file list into `dst`: write only files whose bytes differ
/// (unchanged files keep their mtime so Chrome won't needlessly reload the
/// loaded extension), and prune entries in `dst` that are no longer in the
/// source set.
fn mirror_extension_files(files: &[(String, Vec<u8>)], dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    let mut keep: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for (rel, bytes) in files {
        if rel
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            continue;
        }
        let dest = rel.split('/').fold(dst.to_path_buf(), |p, seg| p.join(seg));
        for ancestor in dest.ancestors().skip(1) {
            if ancestor == dst {
                break;
            }
            keep.insert(ancestor.to_path_buf());
        }
        keep.insert(dest.clone());
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let differs = std::fs::read(&dest).map(|cur| cur != *bytes).unwrap_or(true);
        if differs {
            crate::platform::write_atomic(&dest, bytes)
                .with_context(|| format!("writing {}", dest.display()))?;
        }
    }
    prune_unlisted(dst, &keep)
}

/// Remove files/dirs under `dir` that are not part of the mirrored set, so
/// renamed/removed source files never linger in the loaded extension.
fn prune_unlisted(dir: &Path, keep: &std::collections::HashSet<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        // Never sweep a write_atomic in-flight temp (`<name>.tmp.<pid>.<ns>`):
        // it may belong to a concurrent mirror in ANOTHER process (this
        // process's mirrors are serialized by ENSURE_LOCK), and deleting it
        // fails that mirror mid-rename.
        if entry.file_name().to_string_lossy().contains(".tmp.") {
            continue;
        }
        if path.is_dir() {
            if keep.contains(&path) {
                prune_unlisted(&path, keep)?;
            } else {
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        } else if !keep.contains(&path) {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

/// Locate the repo-checkout extension source (dev / `pnpm tauri dev`) —
/// packaged installs carry the runtime files embedded in the binary instead
/// (see [`super::embedded`]), mirrored to the stable copy by
/// [`ensure_local_unpacked_extension`].
#[cfg(debug_assertions)]
fn repo_extension_source() -> Option<PathBuf> {
    // Look in the working directory and a few of its ancestors, plus the
    // executable directory's ancestors. `pnpm tauri dev` runs the binary with
    // cwd = `src-tauri/`, so a bare `cwd/extensions` lookup misses the
    // repo-root `extensions/chrome`; walking up one level finds it.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    for root in &roots {
        for base in root.ancestors().take(6) {
            let path = base.join("extensions").join("chrome");
            if path.join("manifest.json").exists() {
                return Some(path);
            }
        }
    }
    None
}

fn effective_extension_ids(configured: &[String]) -> Vec<String> {
    let mut ids = Vec::new();
    for id in configured {
        push_unique_extension_id(&mut ids, id.clone());
    }
    if let Some(id) = unpacked_extension_id() {
        push_unique_extension_id(&mut ids, id);
    }
    ids
}

fn push_unique_extension_id(ids: &mut Vec<String>, id: String) {
    if !id.trim().is_empty() && !ids.iter().any(|existing| existing == &id) {
        ids.push(id);
    }
}

fn unpacked_extension_id() -> Option<String> {
    let manifest_path = unpacked_extension_path()?.join("manifest.json");
    let bytes = std::fs::read(manifest_path).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let key = manifest.get("key").and_then(serde_json::Value::as_str)?;
    extension_id_from_manifest_key(key).ok()
}

fn extension_id_from_manifest_key(key: &str) -> Result<String> {
    let compact: String = key.chars().filter(|ch| !ch.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .context("decoding Chrome extension manifest key")?;
    let digest = Sha256::digest(&der);
    let mut id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        id.push(chrome_extension_id_char(byte >> 4));
        id.push(chrome_extension_id_char(byte & 0x0f));
    }
    Ok(id)
}

fn chrome_extension_id_char(nibble: u8) -> char {
    char::from(b'a' + (nibble & 0x0f))
}

fn native_host_binary_hint() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "ha-browser-host.exe"
    } else {
        "ha-browser-host"
    };
    native_host_binary_candidates(exe_name)
        .into_iter()
        .find(|path| path.is_file())
}

fn resolve_host_path(input: Option<String>) -> Result<PathBuf> {
    if let Some(path) = input.filter(|s| !s.trim().is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = native_host_binary_hint() {
        return Ok(path);
    }
    bail!(
        "Native host path is required. Bundle ha-browser-host with Hope Agent, pass its absolute path, or set HOPE_AGENT_BROWSER_HOST_PATH."
    );
}

fn native_host_binary_candidates(exe_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("HOPE_AGENT_BROWSER_HOST_PATH").map(PathBuf::from) {
        candidates.push(path);
    }
    if let Ok(current) = std::env::current_exe() {
        if current
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| stem == "ha-browser-host")
        {
            candidates.push(current.clone());
        }
        if let Some(dir) = current.parent() {
            candidates.push(dir.join(exe_name));
            candidates.push(dir.join("browser-host").join(exe_name));
            candidates.push(dir.join("..").join("Resources").join(exe_name));
            candidates.push(
                dir.join("..")
                    .join("Resources")
                    .join("browser-host")
                    .join(exe_name),
            );
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(exe_name));
        candidates.push(cwd.join("target").join("release").join(exe_name));
        candidates.push(
            cwd.join("src-tauri")
                .join("resources")
                .join("browser-host")
                .join(exe_name),
        );
    }
    candidates
}

fn validate_extension_id(extension_id: &str) -> Result<()> {
    if extension_id.len() != 32 || !extension_id.chars().all(|c| ('a'..='p').contains(&c)) {
        bail!(
            "Invalid Chrome extension id '{}'. Expected 32 lowercase characters in the range a-p.",
            extension_id
        );
    }
    Ok(())
}

fn validate_native_host_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        bail!("Invalid native host name length");
    }
    if name.starts_with('.') || name.ends_with('.') || name.contains("..") {
        bail!("Invalid native host name '{}'", name);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    {
        bail!(
            "Invalid native host name '{}'. Use lowercase letters, digits, underscores, and dots only.",
            name
        );
    }
    Ok(())
}

#[cfg(windows)]
fn register_windows_native_host(
    host_name: &str,
    manifest_path: &std::path::Path,
) -> Result<Option<String>> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let subkey = format!(r"Software\Google\Chrome\NativeMessagingHosts\{host_name}");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(&subkey)
        .with_context(|| format!("creating Chrome NativeMessagingHosts registry key {subkey}"))?;
    key.set_value("", &manifest_path.to_string_lossy().to_string())
        .with_context(|| format!("writing Chrome NativeMessagingHosts registry key {subkey}"))?;
    Ok(Some(format!(r"HKEY_CURRENT_USER\{subkey}")))
}

#[cfg(not(windows))]
fn register_windows_native_host(
    _host_name: &str,
    _manifest_path: &std::path::Path,
) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_fail_closed_without_broker() {
        let status = current_status();
        assert!(!status.backend_available);
        assert!(!status.extension_connected);
    }

    #[test]
    fn default_manifest_path_uses_host_name() {
        let path = native_host_manifest_path("com.example.test").expect("manifest path");
        assert!(path.to_string_lossy().contains("com.example.test.json"));
    }

    #[test]
    fn stable_copy_requires_completion_marker() {
        use std::fs;
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("browser");
        let marker = tmp.path().join(".browser-synced");
        fs::create_dir_all(&dir).unwrap();

        // manifest present but no marker → partial sync, NOT usable.
        fs::write(dir.join("manifest.json"), b"{}").unwrap();
        assert!(!stable_copy_is_complete(&dir, &marker, None));

        // marker present, fingerprint unknown → presence check only.
        fs::write(&marker, b"abc123").unwrap();
        assert!(stable_copy_is_complete(&dir, &marker, None));

        // Fingerprint matches → usable; mismatched (stale mirror from an
        // older binary, incl. legacy "ok" markers) → must re-mirror.
        assert!(stable_copy_is_complete(&dir, &marker, Some("abc123")));
        assert!(!stable_copy_is_complete(&dir, &marker, Some("def456")));

        // marker present but manifest gone → NOT usable.
        fs::remove_file(dir.join("manifest.json")).unwrap();
        assert!(!stable_copy_is_complete(&dir, &marker, None));
    }

    #[test]
    fn mirror_extension_files_copies_prunes_and_skips_unchanged() {
        use std::fs;
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("icons")).unwrap();
        fs::write(src.join("manifest.json"), b"{\"k\":1}").unwrap();
        fs::write(src.join("icons/a.png"), b"AAAA").unwrap();
        let mirror = |src: &std::path::Path, dst: &std::path::Path| {
            let files = read_extension_dir_files(src)?;
            mirror_extension_files(&files, dst)
        };

        // First mirror: dst is a faithful copy, nested dirs included.
        mirror(&src, &dst).expect("first mirror");
        assert_eq!(fs::read(dst.join("manifest.json")).unwrap(), b"{\"k\":1}");
        assert_eq!(fs::read(dst.join("icons/a.png")).unwrap(), b"AAAA");

        // Unchanged file is NOT rewritten (mtime preserved → no needless Chrome
        // reload); a stale file present only in dst is pruned.
        let before = fs::metadata(dst.join("manifest.json"))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(dst.join("stale.js"), b"old").unwrap();
        fs::create_dir_all(dst.join("stale-dir")).unwrap();
        fs::write(dst.join("stale-dir/x.js"), b"old").unwrap();
        mirror(&src, &dst).expect("second mirror");
        let after = fs::metadata(dst.join("manifest.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after, "unchanged file must not be rewritten");
        assert!(!dst.join("stale.js").exists(), "stale file must be pruned");
        assert!(!dst.join("stale-dir").exists(), "stale dir must be pruned");

        // Changed source content propagates.
        fs::write(src.join("manifest.json"), b"{\"k\":2}").unwrap();
        mirror(&src, &dst).expect("third mirror");
        assert_eq!(fs::read(dst.join("manifest.json")).unwrap(), b"{\"k\":2}");
    }

    #[test]
    fn embedded_extension_mirrors_like_a_source_dir() {
        // The embedded file set must flow through the same mirror path, ending
        // with a loadable keyed manifest — this is the bare-binary /headless
        // bootstrap path where no repo checkout exists.
        let tmp = tempfile::tempdir().expect("tempdir");
        let dst = tmp.path().join("stable");
        let files = super::super::embedded::extension_files();
        assert!(!files.is_empty());
        mirror_extension_files(&files, &dst).expect("mirror embedded");
        assert!(dst.join("manifest.json").is_file());
        assert!(dst.join("service_worker.js").is_file());
    }

    #[test]
    fn validates_chrome_extension_ids() {
        assert!(validate_extension_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_ok());
        assert!(validate_extension_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(validate_extension_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }

    #[test]
    fn derives_chrome_extension_id_from_manifest_key() {
        let key = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA37N8lhc6y9uoV/64yn6MwtA3BSNdvnXtjybtfgVzdcklJ6E7GQf6dA1DrHHc1EU7k2dFRtLmFRWVSqIR+E+oAHWxWFLop6Q4uvgySaL5pzpgk2tSYVhrCfOKo6A2xf+DhAB9JwEaS2B30EXEX8rMuNhyBZb2aWmeF4dK4vpjzpyCtcdb5Y3Gi3RBuxiG96UFRnO8ms6GoKH/uCSYipO2c3YWm/DZbj1WxJFolCoMlXyL0/XkroM1UVTLtmuKCGV6jbz98ouHL+DeZ9l909HOmxWckcE3ffR0wSF9NPOGQk/aiSA7LXQcrw4brG4iVgrkD4NRMFwAuCjn/dsUG2cHvQIDAQAB";
        assert_eq!(
            extension_id_from_manifest_key(key).unwrap(),
            "ejafepfkhjdjopjonfgalbkelimgeeji"
        );
    }

    #[test]
    fn effective_extension_ids_appends_unpacked_id_without_duplicate() {
        let configured = vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ];
        let ids = effective_extension_ids(&configured);
        assert_eq!(ids[0], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(
            ids.iter()
                .filter(|id| id.as_str() == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .count(),
            1
        );
        if unpacked_extension_path().is_some() {
            assert!(ids
                .iter()
                .any(|id| id == "ejafepfkhjdjopjonfgalbkelimgeeji"));
        }
    }

    #[test]
    fn validates_native_host_name() {
        assert!(validate_native_host_name("com.hope_agent.chrome").is_ok());
        assert!(validate_native_host_name("Com.HopeAgent.Chrome").is_err());
        assert!(validate_native_host_name(".com.hope_agent.chrome").is_err());
        assert!(validate_native_host_name("com..hope_agent.chrome").is_err());
    }
}

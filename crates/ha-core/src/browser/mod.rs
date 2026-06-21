//! Browser automation subsystem.
//!
//! See [`docs/architecture/browser.md`](../../../docs/architecture/browser.md)
//! for the cross-PR contract. Public surface:
//!
//! - [`backend::BrowserBackend`] — trait the CDP backend implements; reserved
//!   as an extension point should we add Playwright / WebDriver / etc. later.
//! - [`backend_select::acquire_backend`] — get the active backend (creating one
//!   if needed). Returns a [`cdp_backend::CdpBackend`].
//! - [`backend_select::reset_backend`] — drop the active backend (used by
//!   `profile.disconnect` / `profile.launch`).
//! - [`observe_buffer::push`] / [`observe_buffer::snapshot`] — ring buffer for
//!   console / network / page-error events feeding the `observe` action.
//!
//! The legacy global [`crate::browser_state`] remains the storage for the CDP
//! backend's chromiumoxide handle and ref table. New code should not touch it
//! directly — go through the backend trait.

pub mod backend;
pub mod backend_select;
pub mod cdp_backend;
pub mod extension;
pub mod frame;
pub mod launch_circuit;
pub mod observe_buffer;
pub mod profile;
pub mod runtime;
pub mod singleton_lock;
pub mod spawn;
pub mod user_attach;

pub use backend::{
    ActKind, ActParams, BackendStatus, BrowserBackend, DialogAction, ElementRef, ImageFormat,
    ObserveEntry, ObserveKind, PdfParams, RawCdpParams, ScreenshotParams, ScrollDirection,
    ScrollParams, Snapshot, SnapshotFormat, TabInfo, WaitParams,
};
pub use backend_select::{
    acquire_backend, acquire_backend_for, peek_active, reset_backend, status_backend,
};
pub use extension::{
    cleanup_extension_session, current_status, ensure_native_host_registered,
    install_native_host_manifest, schedule_extension_turn_finalize, stop_all_extension_control,
    BrowserBackendContext, BrowserBackendRequirement, BrowserBrokerDiscovery,
    BrowserExtensionBroker, BrowserExtensionConfig, BrowserExtensionStatus,
    BrowserExtensionStatusKind, BrowserExtensionStopResult, ExtensionBackend,
    NativeHostInstallRequest, NativeHostInstallResult,
};

// Shared "give me Console / Network / Exception events on the active
// Chrome" entry points. They physically live in `cdp_backend` because
// they're chromiumoxide-driven.
pub use cdp_backend::{
    activate_observe_subscribers_for_all_pages, activate_observe_subscribers_for_target,
};

/// Resolve and authorise a path being handed to `act.upload`. Returns the
/// canonical absolute path the backend should pass to Chrome, or `Err` if
/// the file is missing or falls inside a user-configured protected path.
///
/// The backend MUST call this before sending the path into Chrome — without
/// it, a prompt-injected webpage with a `<input type=file>` could trick the
/// agent into uploading arbitrary local files (e.g. `~/.ssh/id_rsa`,
/// `~/.aws/credentials`) to attacker-controlled endpoints.
pub fn authorise_upload_path(raw: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::anyhow;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("act.upload: file_path is empty"));
    }
    let canonical = std::fs::canonicalize(trimmed)
        .map_err(|e| anyhow!("act.upload: cannot resolve file path '{}': {}", trimmed, e))?;
    let patterns = crate::permission::protected_paths::current_patterns();
    if let Some(matched) = crate::permission::protected_paths::matches(&canonical, &patterns) {
        return Err(anyhow!(
            "act.upload: refusing to upload protected path {} (matches pattern '{}'). \
             Adjust `permission.protected_paths` in settings if this is intentional.",
            canonical.display(),
            matched
        ));
    }
    Ok(canonical)
}

/// Authorise a path being handed to `snapshot.pdf output_path`. Same SSRF
/// equivalent for write: an LLM-controlled path could otherwise overwrite
/// `~/.ssh/authorized_keys`, system config, etc.
///
/// Important ordering: run a lexical protected-path preflight before creating
/// parent directories. A denied tool call must not leave filesystem side
/// effects behind. After the parent exists, canonicalise and check again to
/// catch symlinks / mount indirection before returning the final write path.
pub fn authorise_pdf_output_path(raw: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::anyhow;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("snapshot.pdf: output_path is empty"));
    }
    let target = std::path::PathBuf::from(crate::tools::expand_tilde(trimmed));
    let lexical_target = crate::permission::rules::normalize_lexical(&target);
    let patterns = crate::permission::protected_paths::current_patterns();
    if let Some(matched) = crate::permission::protected_paths::matches(&lexical_target, &patterns) {
        return Err(anyhow!(
            "snapshot.pdf: refusing to write to protected path {} (matches pattern '{}'). \
             Adjust `permission.protected_paths` in settings if this is intentional.",
            lexical_target.display(),
            matched
        ));
    }

    // Resolve via the nearest existing ancestor first so symlinked parents
    // are checked before we create any missing directories.
    let file_name = lexical_target.file_name().ok_or_else(|| {
        anyhow!(
            "snapshot.pdf: output_path '{}' has no file name component",
            trimmed
        )
    })?;
    let parent = lexical_target.parent().ok_or_else(|| {
        anyhow!(
            "snapshot.pdf: output_path '{}' has no parent directory",
            trimmed
        )
    })?;
    let parent_to_resolve = if parent.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        parent
    };
    let mut existing_ancestor = parent_to_resolve;
    let mut missing_components: Vec<std::ffi::OsString> = Vec::new();
    while !existing_ancestor.exists() {
        if let Some(name) = existing_ancestor.file_name() {
            missing_components.push(name.to_os_string());
        }
        existing_ancestor = existing_ancestor
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
    }
    let canonical_ancestor = std::fs::canonicalize(existing_ancestor).map_err(|e| {
        anyhow!(
            "snapshot.pdf: cannot resolve parent ancestor {}: {}",
            existing_ancestor.display(),
            e
        )
    })?;
    let mut resolved_parent = canonical_ancestor;
    for component in missing_components.iter().rev() {
        resolved_parent.push(component);
    }
    let resolved_target = resolved_parent.join(file_name);
    if let Some(matched) = crate::permission::protected_paths::matches(&resolved_target, &patterns)
    {
        return Err(anyhow!(
            "snapshot.pdf: refusing to write to protected path {} (matches pattern '{}'). \
             Adjust `permission.protected_paths` in settings if this is intentional.",
            resolved_target.display(),
            matched
        ));
    }

    std::fs::create_dir_all(&resolved_parent).map_err(|e| {
        anyhow!(
            "snapshot.pdf: cannot create parent directory {}: {}",
            resolved_parent.display(),
            e
        )
    })?;
    let canonical_parent = std::fs::canonicalize(&resolved_parent).map_err(|e| {
        anyhow!(
            "snapshot.pdf: cannot resolve parent directory {}: {}",
            resolved_parent.display(),
            e
        )
    })?;
    let canonical = canonical_parent.join(file_name);
    if let Some(matched) = crate::permission::protected_paths::matches(&canonical, &patterns) {
        return Err(anyhow!(
            "snapshot.pdf: refusing to write to protected path {} (matches pattern '{}'). \
             Adjust `permission.protected_paths` in settings if this is intentional.",
            canonical.display(),
            matched
        ));
    }
    Ok(canonical)
}

/// Validate a CDP debug-endpoint URL before we hand it to `Browser::connect`
/// or `discover_ws_url`. Used by every entry point that takes a user-
/// supplied URL: tool (`profile.connect url=...`), settings UI
/// (`browser_connect`), HTTP `/api/browser/connect`. Single source of
/// truth so a SSRF policy tweak applies everywhere.
///
/// Checks (in order):
/// 1. Non-empty after trim.
/// 2. `http://` or `https://` scheme.
/// 3. `security::ssrf::check_url` with the browser-tool policy — defaults
///    accept loopback (127.0.0.1, ::1); LAN / public IPs require the user
///    to opt in via `permission.ssrf.allow_private` etc.
pub async fn validate_cdp_endpoint_url(url: &str) -> anyhow::Result<()> {
    use anyhow::anyhow;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Debug URL is required"));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(anyhow!("Debug URL must start with http:// or https://"));
    }
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(trimmed, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts).await?;
    Ok(())
}

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// UI-only preference: which tab the settings BrowserPanel opens on
/// (Standalone vs. Take-over-user-Chrome). The actual runtime path is
/// decided by *which profile the user picks*. No backend code reads
/// `default_mode`; treat it as remembered UI state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    #[default]
    Managed,
    UserAttach,
}

/// Browser backend preference. `ExtensionFirst` is the product default:
/// use the Chrome Extension backend when it is connected, and fall back to
/// CDP only for actions that do not require the user's real Chrome state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBackendPreference {
    #[default]
    ExtensionFirst,
    CdpOnly,
    ExtensionOnly,
}

/// Persisted browser configuration. Stored under `AppConfig.browser`.
///
/// All fields are optional so omitting the block in `config.json` yields
/// the same zero-config defaults the legacy version had.
///
/// Schema evolution notes:
/// - A previous `backend` field selected between CDP and an external bridge;
///   the external backend was removed and any leftover `"backend"` key in old
///   `config.json` is silently ignored by serde.
/// - A previous `userAttach: { lastSpawnedPort }` field tracked the
///   user-attach Chrome port bookkeeping; user_attach is now a first-class
///   entry in [`Self::profiles`] (always port 9222), so the legacy key is
///   silently ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    /// Runtime backend preference. `None` = `ExtensionFirst`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_preference: Option<BrowserBackendPreference>,
    /// Chrome Extension + Native Messaging integration config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<BrowserExtensionConfig>,
    /// UI-only opening tab. `None` = `Managed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<BrowserMode>,
    /// Default profile name when `profile.op=launch` is called with no
    /// `profile=` argument. `None` = `"managed"` (ephemeral).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// User-defined Chrome profiles. Two well-known names — `"managed"`
    /// and `"user_attach"` — are always present (synthesised at resolve
    /// time when absent here); user entries override their defaults and
    /// can add arbitrary new profiles.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, BrowserProfileConfig>,
    /// Heartbeat probe interval in seconds. `None` / `Some(0)` use the
    /// default (120s). Clamped to `[30, 600]` at read time. Defeats Chrome's
    /// ~4-minute WebSocket idle close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval_secs: Option<u32>,
    /// Launch failure circuit breaker thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_circuit: Option<LaunchCircuitConfig>,
}

/// Per-profile launch circuit breaker tuning. Defaults: 3 consecutive
/// failures → 60s cooldown. `failure_threshold = 0` disables the breaker.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchCircuitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_secs: Option<u64>,
}

/// Per-profile launch configuration. All fields optional — absent means
/// "use the built-in default for this profile name". See
/// [`crate::browser::profile::resolve_profile`] for the resolution table.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileConfig {
    /// Absolute or `~`-relative path. None → derive from profile name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<String>,
    /// Fixed `--remote-debugging-port`. None means OS-pick for managed,
    /// 9222 for user_attach, OS-pick for user-defined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Chrome / Chromium binary override. None = platform auto-probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    /// Launch headless. None = environment default (headed on desktop,
    /// headless for Docker / no-display Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,
    /// Verbatim extra Chrome args (e.g. `["--proxy-server=..."]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
    /// UI tint (CSS color). Pure UI hint, no behaviour impact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

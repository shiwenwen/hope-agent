//! Browser automation subsystem.
//!
//! See [`docs/architecture/core/browser.md`](../../../docs/architecture/core/browser.md)
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
    cleanup_extension_session, current_status, ensure_local_unpacked_extension,
    ensure_native_host_registered, install_native_host_manifest, schedule_extension_turn_finalize,
    stop_all_extension_control, BrowserBackendContext, BrowserBackendRequirement,
    BrowserBrokerDiscovery, BrowserExtensionBroker, BrowserExtensionConfig, BrowserExtensionStatus,
    BrowserExtensionStatusKind, BrowserExtensionStopResult, ExtensionBackend,
    NativeHostInstallRequest, NativeHostInstallResult,
};

// Shared "give me Console / Network / Exception events on the active
// Chrome" entry points. They physically live in `cdp_backend` because
// they're chromiumoxide-driven.
pub use cdp_backend::{
    activate_observe_subscribers_for_all_pages, activate_observe_subscribers_for_target,
};

// 类型已下沉 ha-config-schema：`AppConfig.browser` 的 wire 类型原地再导出，
// `crate::browser::BrowserConfig` 等既有路径不变。
pub use ha_config_schema::browser::{
    BrowserBackendPreference, BrowserConfig, BrowserMode, BrowserProfileConfig, LaunchCircuitConfig,
};

/// Process-wide serialization lock for tests that mutate browser-module global
/// state — the active-backend cache ([`backend_select`]) and the tab registry
/// ([`extension::registry`]). Sync tests acquire it with `blocking_lock()`,
/// async tests with `lock().await`; sharing a single lock keeps the browser
/// test suite race-free under parallel execution (each global was previously
/// guarded by its own — or no — lock, so cross-test runs flaked).
#[cfg(test)]
pub(crate) fn global_state_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

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
    let patterns = ha_core::permission::protected_paths::current_patterns();
    if let Some(matched) = ha_core::permission::protected_paths::matches(&canonical, &patterns) {
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
    let target = std::path::PathBuf::from(ha_core::tools::expand_tilde(trimmed));
    let lexical_target = ha_core::permission::rules::normalize_lexical(&target);
    let patterns = ha_core::permission::protected_paths::current_patterns();
    if let Some(matched) = ha_core::permission::protected_paths::matches(&lexical_target, &patterns)
    {
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
    if let Some(matched) =
        ha_core::permission::protected_paths::matches(&resolved_target, &patterns)
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
    if let Some(matched) = ha_core::permission::protected_paths::matches(&canonical, &patterns) {
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
    let ssrf_cfg = &ha_core::config::cached_config().ssrf;
    ha_core::security::ssrf::check_url(trimmed, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts)
        .await?;
    Ok(())
}

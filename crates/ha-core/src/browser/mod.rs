//! Browser automation subsystem.
//!
//! See [`docs/architecture/browser.md`](../../../docs/architecture/browser.md)
//! (planned) for the cross-PR contract. Public surface:
//!
//! - [`backend::BrowserBackend`] — trait every backend implements.
//! - [`backend_select::acquire_backend`] — get the active backend (creating one
//!   if needed). Honours `AppConfig.browser.backend` and node-availability.
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
pub mod frame;
pub mod mcp_backend;
pub(crate) mod mcp_client;
pub mod observe_buffer;
pub mod runtime;
pub mod singleton_lock;
pub mod user_attach;

pub use backend::{
    ActKind, ActParams, BackendStatus, BrowserBackend, DialogAction, ElementRef, ImageFormat,
    ObserveEntry, ObserveKind, PdfParams, ScreenshotParams, ScrollDirection, ScrollParams,
    Snapshot, SnapshotFormat, TabInfo, WaitParams,
};
pub use backend_select::{
    acquire_backend, acquire_backend_with, detect_node_available, peek_active, reset_backend,
    BackendPreference,
};

// Shared "give me Console / Network / Exception events on the active
// Chrome" entry points. They physically live in `cdp_backend` because
// they're chromiumoxide-driven, but they're conceptually a property of
// the underlying Chrome handle — both backends route through them.
pub use cdp_backend::{
    activate_observe_subscribers_for_all_pages, activate_observe_subscribers_for_target,
};

/// Resolve and authorise a path being handed to `act.upload`. Returns the
/// canonical absolute path the backend should pass to Chrome, or `Err` if
/// the file is missing or falls inside a user-configured protected path.
///
/// Both CDP and MCP backends MUST call this before sending the path into
/// Chrome — without it, a prompt-injected webpage with a `<input
/// type=file>` could trick the agent into uploading arbitrary local files
/// (e.g. `~/.ssh/id_rsa`, `~/.aws/credentials`) to attacker-controlled
/// endpoints.
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

use serde::{Deserialize, Serialize};

/// UI-only preference: which tab the settings BrowserPanel opens on
/// (Standalone vs. Take-over-user-Chrome). The actual runtime path is
/// decided by *which button the user clicks* — `browser_launch` always
/// runs managed Chrome, `browser_spawn_user_chrome` always runs the
/// user-attach Chrome — independent of this field. No backend code
/// reads `default_mode`; treat it as remembered UI state, not a
/// behaviour switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    #[default]
    Managed,
    UserAttach,
}

/// Persisted browser configuration. Stored under `AppConfig.browser`.
///
/// All fields are optional so omitting the block in `config.json` yields
/// the same zero-config defaults the legacy version had.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    /// Backend preference. `None` = `Auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<backend_select::BackendPreference>,
    /// Default browser mode. `None` = `Managed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<BrowserMode>,
    /// User-attach mode bookkeeping (last-spawned port etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_attach: Option<UserAttachConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAttachConfig {
    /// Remote-debugging port we last spawned the user-attach Chrome on.
    /// Used by the "Reconnect" path in the settings BrowserPanel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_spawned_port: Option<u16>,
}

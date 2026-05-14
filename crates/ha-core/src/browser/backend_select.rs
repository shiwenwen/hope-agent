//! Backend selection + Node.js detection.
//!
//! - [`detect_node_available`] caches the result of `node --version` (and a
//!   minimum-version check) in a `OnceCell`, so repeated calls are free after
//!   the first probe.
//! - [`active_backend`] holds the currently-acquired backend for this process.
//!   A `profile.launch` / `profile.connect` resets it via [`reset_backend`];
//!   `profile.disconnect` clears it via [`reset_backend`] too.
//! - [`acquire_backend`] is the entry point used by 8-action handlers — it
//!   honours `AppConfig.browser.backend` (`auto` / `cdp` / `mcp`) and falls
//!   back to CDP when MCP init fails.
//!
//! The active backend is stored as `Arc<dyn BrowserBackend>` so handlers can
//! grab a cheap clone and release the lock before doing IO.

use std::fmt;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OnceCell};

use super::backend::BrowserBackend;
use super::cdp_backend::CdpBackend;

/// Minimum Node.js major version we accept. `npx -y` is reliable from v18+.
pub const MIN_NODE_MAJOR: u32 = 18;

/// Cached node availability probe.
static NODE_DETECT: OnceCell<bool> = OnceCell::const_new();

/// Cached node version string (raw `v22.3.0` form). Filled on the first
/// successful `node --version` probe. UI surfaces this via
/// [`probe_node_version`] to render "Detected Node v22.3.0 ✓".
static NODE_VERSION: OnceCell<Option<String>> = OnceCell::const_new();

/// Currently-active backend, if any. Cleared on profile disconnect / launch.
static ACTIVE_BACKEND: Mutex<Option<Arc<dyn BrowserBackend>>> = Mutex::const_new(None);

/// Probe whether `node` / `npx` are on PATH and Node.js >= [`MIN_NODE_MAJOR`].
/// Result is cached for the process lifetime.
pub async fn detect_node_available() -> bool {
    *NODE_DETECT
        .get_or_init(|| async {
            // Step 1: which node / which npx (both required)
            if which::which("node").is_err() || which::which("npx").is_err() {
                app_info!(
                    "browser",
                    "backend_select",
                    "node/npx not on PATH; chrome-devtools-mcp backend unavailable"
                );
                return false;
            }
            // Step 2: node --version → "vX.Y.Z" → check major
            let out = match tokio::process::Command::new("node")
                .arg("--version")
                .kill_on_drop(true)
                .output()
                .await
            {
                Ok(out) => out,
                Err(e) => {
                    app_warn!("browser", "backend_select", "node --version failed: {}", e);
                    return false;
                }
            };
            if !out.status.success() {
                return false;
            }
            let version_raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let major = parse_node_major(&version_raw);
            let ok = major.is_some_and(|m| m >= MIN_NODE_MAJOR);
            let _ = NODE_VERSION.set(if ok { Some(version_raw.clone()) } else { None });
            if ok {
                app_info!(
                    "browser",
                    "backend_select",
                    "Detected Node.js {} (>= v{}); MCP backend available",
                    version_raw,
                    MIN_NODE_MAJOR
                );
            } else {
                app_warn!(
                    "browser",
                    "backend_select",
                    "Node.js {} found but < v{}; MCP backend unavailable",
                    version_raw,
                    MIN_NODE_MAJOR
                );
            }
            ok
        })
        .await
}

/// Return the cached Node.js version string (e.g. `v22.3.0`) if the last
/// [`detect_node_available`] probe found a usable Node toolchain. Runs the
/// probe lazily if it hasn't been cached yet.
pub async fn probe_node_version() -> Option<String> {
    if !detect_node_available().await {
        return None;
    }
    NODE_VERSION.get().and_then(|v| v.clone())
}

/// Parse `vMAJOR.MINOR.PATCH` → MAJOR.
fn parse_node_major(s: &str) -> Option<u32> {
    let trimmed = s.trim().trim_start_matches('v');
    let major = trimmed.split('.').next()?;
    major.parse().ok()
}

/// User-facing preference from `AppConfig.browser.backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendPreference {
    #[default]
    Auto,
    Cdp,
    Mcp,
}

impl BackendPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendPreference::Auto => "auto",
            BackendPreference::Cdp => "cdp",
            BackendPreference::Mcp => "mcp",
        }
    }
}

impl fmt::Display for BackendPreference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Acquire a backend, creating one if none is active.
///
/// Selection rules:
/// - `BackendPreference::Cdp` → always CdpBackend.
/// - `BackendPreference::Mcp` → ChromeMcpBackend or error (no silent fallback
///   when the user explicitly asked for MCP).
/// - `BackendPreference::Auto` → ChromeMcpBackend when Node is available and
///   the MCP server initialises within timeout; otherwise CdpBackend.
pub async fn acquire_backend() -> Result<Arc<dyn BrowserBackend>> {
    let cfg = crate::config::cached_config();
    let pref = cfg
        .browser
        .as_ref()
        .and_then(|b| b.backend)
        .unwrap_or_default();
    acquire_backend_with(pref).await
}

/// Like [`acquire_backend`] but with an explicit preference (used by tests
/// and by the settings panel's "Apply backend choice" path).
pub async fn acquire_backend_with(pref: BackendPreference) -> Result<Arc<dyn BrowserBackend>> {
    {
        let guard = ACTIVE_BACKEND.lock().await;
        if let Some(b) = guard.as_ref() {
            return Ok(b.clone());
        }
    }

    let backend: Arc<dyn BrowserBackend> = match pref {
        BackendPreference::Cdp => Arc::new(CdpBackend::new()),
        BackendPreference::Mcp => {
            // Forced MCP: bring Chrome up first (chrome-devtools-mcp needs a
            // running Chrome on 9222 to attach to), then try MCP. No silent
            // fallback — user asked for MCP, surface the error if it fails.
            ensure_chrome_for_mcp().await?;
            try_mcp_backend().await.ok_or_else(|| {
                anyhow!(
                    "MCP backend init failed and 'mcp' was forced; check Node.js installation and \
                     chrome-devtools-mcp availability"
                )
            })?
        }
        BackendPreference::Auto => {
            if detect_node_available().await {
                // First quick attempt: maybe Chrome is already up (e.g. user
                // just ran `profile.launch`). If not, bring it up via
                // `ensure_connected_or_launch_managed` so `try_mcp_backend`
                // has a `connection_url` to read. Only fall back to CDP when
                // MCP really cannot initialise — otherwise we'd cache CDP
                // forever and the "Auto picks MCP when available" promise
                // breaks for zero-config startup.
                if let Some(mcp) = try_mcp_backend().await {
                    mcp
                } else if let Err(e) = ensure_chrome_for_mcp().await {
                    app_warn!(
                        "browser",
                        "backend_select",
                        "Auto: could not bring Chrome up before MCP retry ({}); using CDP",
                        e
                    );
                    Arc::new(CdpBackend::new())
                } else if let Some(mcp) = try_mcp_backend().await {
                    mcp
                } else {
                    app_warn!(
                        "browser",
                        "backend_select",
                        "Auto: MCP backend init failed after ensuring Chrome; falling back to CDP"
                    );
                    Arc::new(CdpBackend::new())
                }
            } else {
                Arc::new(CdpBackend::new())
            }
        }
    };

    let mut guard = ACTIVE_BACKEND.lock().await;
    *guard = Some(backend.clone());
    Ok(backend)
}

/// Ensure a Chrome instance is running on the default CDP port before we
/// hand the debug URL to chrome-devtools-mcp. Without this, the first
/// `tabs.list` call in a session would fail-then-cache CDP, locking the
/// Auto backend out of MCP for the rest of the process lifetime.
async fn ensure_chrome_for_mcp() -> Result<()> {
    crate::browser_state::ensure_connected_or_launch_managed()
        .await
        .map(|_| ())
}

async fn try_mcp_backend() -> Option<Arc<dyn BrowserBackend>> {
    match super::mcp_backend::ChromeMcpBackend::try_new().await {
        Ok(b) => Some(Arc::new(b)),
        Err(e) => {
            app_warn!(
                "browser",
                "backend_select",
                "ChromeMcpBackend::try_new failed: {}",
                e
            );
            None
        }
    }
}

/// Tear down the active backend (e.g. on `profile.disconnect`).
///
/// Future `acquire_backend` calls will reinitialise based on current config.
pub async fn reset_backend() {
    let mut guard = ACTIVE_BACKEND.lock().await;
    *guard = None;
    super::observe_buffer::clear_all();
    super::cdp_backend::clear_subscribed_pages();
}

/// Read-only peek at the currently-active backend without acquiring.
pub async fn peek_active() -> Option<Arc<dyn BrowserBackend>> {
    ACTIVE_BACKEND.lock().await.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_node_major_handles_standard_format() {
        assert_eq!(parse_node_major("v22.3.0"), Some(22));
        assert_eq!(parse_node_major("v18.19.1"), Some(18));
        assert_eq!(parse_node_major("v20.0.0-nightly"), Some(20));
    }

    #[test]
    fn parse_node_major_rejects_garbage() {
        assert_eq!(parse_node_major(""), None);
        assert_eq!(parse_node_major("not a version"), None);
        assert_eq!(parse_node_major("vX.Y.Z"), None);
    }

    #[test]
    fn backend_preference_default_is_auto() {
        assert_eq!(BackendPreference::default(), BackendPreference::Auto);
    }

    #[test]
    fn backend_preference_serde_roundtrip() {
        for pref in [
            BackendPreference::Auto,
            BackendPreference::Cdp,
            BackendPreference::Mcp,
        ] {
            let s = serde_json::to_string(&pref).unwrap();
            let back: BackendPreference = serde_json::from_str(&s).unwrap();
            assert_eq!(pref, back);
        }
        // Lowercase strings deserialize as expected (snake_case wins).
        assert_eq!(
            serde_json::from_str::<BackendPreference>("\"cdp\"").unwrap(),
            BackendPreference::Cdp
        );
    }
}

//! "Take over user Chrome" support.
//!
//! Spawns a Chrome instance configured for hope-agent automation but with a
//! dedicated user-data-dir under [`paths::browser_user_attach_dir`], so the
//! user's real daily-browsing profile is never touched.
//!
//! Once Chrome is up, hope-agent attaches to it via the normal `browser.profile
//! .connect` path (CDP on `--remote-debugging-port`). If the user closes
//! Chrome, hope-agent simply loses the connection.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::platform;

/// Default remote-debugging port we hand to user-attach Chrome.
pub const DEFAULT_USER_ATTACH_PORT: u16 = 9222;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeUserChromeReport {
    pub found: bool,
    pub browser_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Best-effort GET against `http://127.0.0.1:{port}/json/version` with a
/// short timeout. Uses the shared proxy-aware fetcher so this stays in
/// lock-step with `browser_state::discover_ws_url`.
pub async fn probe_user_chrome(port: u16) -> ProbeUserChromeReport {
    let browser_url = format!("http://127.0.0.1:{port}");
    match crate::browser_state::fetch_chrome_json_version(&browser_url, 2).await {
        Ok(body) => ProbeUserChromeReport {
            found: true,
            browser_url,
            version: body
                .get("Browser")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        },
        Err(_) => ProbeUserChromeReport {
            found: false,
            browser_url,
            version: None,
        },
    }
}

/// Single-shot report covering everything the settings panel needs to
/// render its banners: the `--remote-debugging-port=9222` probe, whether
/// a Chrome process is already running, and the cached Chromium runtime
/// path (when one was downloaded via `profile.op=install_runtime`).
/// Bundling avoids round-trips per refresh.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDoctorReport {
    pub probe: ProbeUserChromeReport,
    pub chrome_already_running: bool,
    /// Path to a Chrome / Chromium / Edge / Brave binary discovered by the
    /// platform probe — populated whenever
    /// [`crate::platform::find_chrome_executable`] returns a path. Lets the
    /// settings UI distinguish "system Chrome present" from "needs runtime
    /// download" without the `runtime_chromium` field falsely implying the
    /// host has no browser.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_chrome_path: Option<String>,
    /// Cached Chromium runtime — populated when
    /// `~/.hope-agent/browser/runtime/chromium-{rev}/` has a usable binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_chromium: Option<RuntimeChromiumReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeChromiumReport {
    pub revision: u32,
    pub binary_path: String,
}

pub async fn browser_doctor() -> BrowserDoctorReport {
    let (probe, chrome_already_running, system_chrome_path, runtime_chromium) = tokio::join!(
        probe_user_chrome(DEFAULT_USER_ATTACH_PORT),
        crate::platform::chrome_already_running(),
        async {
            tokio::task::spawn_blocking(|| {
                crate::platform::find_chrome_executable().map(|p| p.display().to_string())
            })
            .await
            .unwrap_or(None)
        },
        async {
            tokio::task::spawn_blocking(|| {
                let p = crate::browser::runtime::cached_binary_path()?;
                let spec = crate::browser::runtime::spec_for_current_platform()?;
                Some(RuntimeChromiumReport {
                    revision: spec.revision,
                    binary_path: p.display().to_string(),
                })
            })
            .await
            .unwrap_or(None)
        },
    );
    BrowserDoctorReport {
        probe,
        chrome_already_running,
        system_chrome_path,
        runtime_chromium,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnUserChromeArgs {
    /// Optional override of the Chrome binary. When `None`, falls back to
    /// [`platform::find_chrome_executable`].
    #[serde(default)]
    pub executable_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnUserChromeResult {
    pub port: u16,
    pub debug_url: String,
    pub user_data_dir: String,
    /// `true` when hope-agent detected a Chrome process running at spawn
    /// time; surfaced to the UI so the modal description can note the
    /// "new isolated Chrome" coexists with the user's daily one.
    pub chrome_was_already_running: bool,
}

/// Spawn the user-attach Chrome and connect via CDP.
///
/// Resolves the `user_attach` profile (built-in: port 9222 + persistent
/// user-data-dir) and delegates to
/// [`crate::browser_state::BrowserState::spawn_chrome_and_connect`] so
/// process management is identical to the managed-launch path.
pub async fn spawn_user_chrome(args: SpawnUserChromeArgs) -> Result<SpawnUserChromeResult> {
    let chrome_was_already_running = platform::chrome_already_running().await;

    // Resolve the user_attach profile from config + built-in defaults.
    let mut resolved =
        crate::browser::profile::resolve_profile(crate::browser::profile::BUILTIN_USER_ATTACH)?;
    // Caller-supplied executable override beats profile-config exec.
    if let Some(exec) = args
        .executable_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        resolved.executable = Some(exec.to_string());
    }

    let port = resolved.port.unwrap_or(DEFAULT_USER_ATTACH_PORT);
    if tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_err()
    {
        bail!(
            "Port {port} is already in use. Close the existing Chrome on this port \
             first, or attach to it directly via settings → Browser → Connect."
        );
    }

    let exec_owned = resolved.executable.clone();
    let extra = resolved.extra_args.clone();
    let spec = crate::browser::spawn::LaunchSpec {
        profile: &resolved.name,
        executable: exec_owned.as_deref(),
        user_data_dir: &resolved.user_data_dir,
        port,
        headless: resolved.headless,
        extra_args: &extra,
    };
    // Match the `profile_launch` / `browser_ui::launch` lifecycle:
    // (1) `needs_cleanup` (not `is_connected`) so a dead-ws Chrome still
    // gets reaped; (2) `reset_backend` clears `ACTIVE_BACKEND` /
    // `observe_buffer` / subscribed-pages so the new session doesn't see
    // leftover events from the previous launch; (3) `acquire_backend`
    // re-initialises a fresh CdpBackend for the new Chrome.
    {
        let mut state = crate::browser_state::get_browser_state().lock().await;
        if state.needs_cleanup() {
            state.disconnect().await;
        }
        state.spawn_chrome_and_connect(spec).await?;
    }
    crate::browser::reset_backend().await;
    let _ = crate::browser::acquire_backend().await;

    app_info!(
        "browser",
        "user_attach",
        "Spawned user-attach Chrome on port {} (existing Chrome detected: {})",
        port,
        chrome_was_already_running
    );

    Ok(SpawnUserChromeResult {
        port,
        debug_url: format!("http://127.0.0.1:{}", port),
        user_data_dir: resolved.user_data_dir.to_string_lossy().into_owned(),
        chrome_was_already_running,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn user_attach_dir_under_root() {
        let p = crate::paths::browser_user_attach_dir().expect("user attach dir");
        let s = p.to_string_lossy();
        assert!(s.contains("browser"));
        assert!(s.ends_with("user-attach") || s.ends_with("user-attach/"));
    }
}

//! "Take over user Chrome" support.
//!
//! Spawns a Chrome instance configured for hope-agent automation but with a
//! dedicated user-data-dir under [`paths::browser_user_attach_dir`], so the
//! user's real daily-browsing profile is never touched.
//!
//! The spawned `std::process::Child` is intentionally not retained — once
//! Chrome is up, hope-agent attaches to it via the normal `browser.profile
//! .connect` path (CDP on `--remote-debugging-port`). If the user closes
//! Chrome, hope-agent simply loses the connection; we never try to keep it
//! alive on their behalf.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::platform;

/// Default remote-debugging port we hand to user-attach Chrome. Matches the
/// CDP backend's expectation and chrome-devtools-mcp's default.
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
/// render its banners: Node toolchain availability, current/active
/// backend, the `--remote-debugging-port=9222` probe, and whether a Chrome
/// process is already running. Bundling avoids 3 round-trips per refresh.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDoctorReport {
    pub node_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_backend: Option<String>,
    pub preference: crate::browser::backend_select::BackendPreference,
    pub probe: ProbeUserChromeReport,
    pub chrome_already_running: bool,
    /// Detected daily browser (Chrome / Edge / Brave / Chromium) for the
    /// `target=system` path. `None` when nothing is installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_chrome: Option<SystemChromeReport>,
    /// Cached Chromium runtime — populated when
    /// `~/.hope-agent/browser/runtime/chromium-{rev}/` has a usable binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_chromium: Option<RuntimeChromiumReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemChromeReport {
    /// "Google Chrome" / "Microsoft Edge" / "Brave" / "Chromium".
    pub brand: String,
    pub executable: String,
    pub user_data_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeChromiumReport {
    pub revision: u32,
    pub binary_path: String,
}

pub async fn browser_doctor() -> BrowserDoctorReport {
    use crate::browser::backend_select;
    let (node_available, node_version, active_backend, probe, chrome_already_running) = tokio::join!(
        backend_select::detect_node_available(),
        backend_select::probe_node_version(),
        async {
            backend_select::peek_active()
                .await
                .map(|b| b.backend_name().to_string())
        },
        probe_user_chrome(DEFAULT_USER_ATTACH_PORT),
        crate::platform::chrome_already_running(),
    );
    let preference = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.backend)
        .unwrap_or_default();
    let system_chrome =
        crate::platform::chrome_paths::detect_daily_browser().map(|inst| SystemChromeReport {
            brand: inst.brand.display_name().to_string(),
            executable: inst.executable.display().to_string(),
            user_data_dir: inst.user_data_dir.display().to_string(),
        });
    let runtime_chromium = crate::browser::runtime::cached_binary_path().map(|p| {
        let revision = crate::browser::runtime::spec_for_current_platform()
            .map(|s| s.revision)
            .unwrap_or(crate::paths::CHROMIUM_PINNED_REVISION);
        RuntimeChromiumReport {
            revision,
            binary_path: p.display().to_string(),
        }
    });
    BrowserDoctorReport {
        node_available,
        node_version,
        active_backend,
        preference,
        probe,
        chrome_already_running,
        system_chrome,
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

/// Build the argv vector for the Chrome spawn. Extracted so it can be
/// unit-tested without actually starting a child process.
pub fn build_spawn_command(exec: &str, user_data_dir: &PathBuf, port: u16) -> Command {
    let mut cmd = Command::new(exec);
    cmd.arg(format!("--remote-debugging-port={}", port));
    let mut user_data_arg = OsString::from("--user-data-dir=");
    user_data_arg.push(user_data_dir);
    cmd.arg(user_data_arg);
    cmd.arg("--no-first-run");
    cmd.arg("--no-default-browser-check");
    cmd.arg("about:blank");
    cmd
}

/// Spawn Chrome for the "Take over user Chrome" flow. Persists the chosen
/// port to `AppConfig.browser.userAttach.lastSpawnedPort` so the settings
/// panel's "Reconnect" path can target it next session.
pub async fn spawn_user_chrome(args: SpawnUserChromeArgs) -> Result<SpawnUserChromeResult> {
    let chrome_was_already_running = platform::chrome_already_running().await;

    let user_data_dir = paths::browser_user_attach_dir()?;
    std::fs::create_dir_all(&user_data_dir)?;

    let exec: String = match args.executable_path {
        Some(p) if !p.trim().is_empty() => p,
        _ => platform::find_chrome_executable()
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Chrome executable not found on this system. \
                     Please set an explicit path in settings → Browser → Advanced \
                     (Executable path)."
                )
            })?,
    };

    // Probe the default port; if it's taken we bail rather than guess,
    // because chrome-devtools-mcp / CDP backend / settings all assume 9222.
    // TOCTOU window is ms-scale and only triggered by an explicit user
    // click — acceptable.
    let port = DEFAULT_USER_ATTACH_PORT;
    if tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .is_err()
    {
        bail!(
            "Port {port} is already in use. Close the existing Chrome on this port \
             first, or attach to it directly via settings → Browser → Connect."
        );
    }

    let mut cmd = build_spawn_command(&exec, &user_data_dir, port);
    cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Failed to launch Chrome at {exec:?}: {e}. \
             Double-check the Executable path in settings → Browser → Advanced."
        )
    })?;

    // Best-effort: persist `lastSpawnedPort`. We do not block the spawn on
    // a config write failure — the user can still connect to 9222 even if
    // we couldn't remember it for next time.
    if let Err(e) =
        crate::config::mutate_config::<_, ()>(("browser.user_attach", "settings"), |cfg| {
            let browser = cfg
                .browser
                .get_or_insert_with(crate::browser::BrowserConfig::default);
            let ua = browser
                .user_attach
                .get_or_insert_with(crate::browser::UserAttachConfig::default);
            ua.last_spawned_port = Some(port);
            Ok(())
        })
    {
        app_warn!(
            "browser",
            "user_attach",
            "Failed to persist lastSpawnedPort: {}",
            e
        );
    }

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
        user_data_dir: user_data_dir.to_string_lossy().into_owned(),
        chrome_was_already_running,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spawn_command_includes_required_flags() {
        let cmd = build_spawn_command(
            "/usr/bin/google-chrome",
            &PathBuf::from("/tmp/user-attach"),
            9222,
        );
        let argv: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(argv.iter().any(|a| a == "--remote-debugging-port=9222"));
        assert!(argv.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert!(argv.iter().any(|a| a == "--no-first-run"));
        assert!(argv.iter().any(|a| a == "--no-default-browser-check"));
        assert!(argv.iter().any(|a| a == "about:blank"));
    }

    #[test]
    fn user_attach_dir_under_root() {
        let p = paths::browser_user_attach_dir().expect("user attach dir");
        let s = p.to_string_lossy();
        assert!(s.contains("browser"));
        assert!(s.ends_with("user-attach") || s.ends_with("user-attach/"));
    }
}

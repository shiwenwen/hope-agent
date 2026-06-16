//! Unified Chrome spawn primitives.
//!
//! Both the managed-runner path and the user_attach path go through
//! [`LaunchSpec`] + [`build_chrome_argv`] + caller-side
//! `BrowserState::spawn_chrome_and_connect`. The chromiumoxide
//! `Browser::launch` helper is no longer used — we own the `Child` handle
//! so we can kill / wait / detect Chrome process death directly instead of
//! relying on `kill_on_drop` heuristics.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

/// Description of one Chrome instance to spawn.
///
/// `port` is fixed (caller must pre-pick a free one for managed launches via
/// [`pick_managed_port`], or hand the well-known 9222 for user_attach). We do
/// not let Chrome auto-pick ports — we need the port up-front for the
/// `/json/version` readiness poll.
pub struct LaunchSpec<'a> {
    /// Profile name. Used as the circuit-breaker key inside
    /// `spawn_chrome_and_connect`, and stored on `BrowserState.profile`.
    /// Built-ins: [`crate::browser::profile::BUILTIN_MANAGED`] /
    /// [`crate::browser::profile::BUILTIN_USER_ATTACH`].
    pub profile: &'a str,
    /// Override of the Chrome binary. `None` = resolve from platform probe →
    /// downloaded chromium runtime → error with remediation.
    pub executable: Option<&'a str>,
    /// Where Chrome will write its profile. Must be writable.
    pub user_data_dir: &'a Path,
    /// `--remote-debugging-port=N`.
    pub port: u16,
    /// `--headless=new` when true; headed mode otherwise.
    pub headless: bool,
    /// Verbatim extra args forwarded to Chrome.
    pub extra_args: &'a [String],
}

const MANAGED_BROWSER_WINDOW_WIDTH: u32 = 1440;
const MANAGED_BROWSER_WINDOW_HEIGHT: u32 = 960;

/// Resolve the Chrome / Chromium binary path. Tries (in order):
/// 1. caller override
/// 2. platform probe ([`crate::platform::find_chrome_executable`])
/// 3. downloaded Chromium runtime ([`crate::browser::runtime::cached_binary_path`])
///
/// Returns a friendly error with remediation when none of the three hits.
pub fn resolve_chrome_executable(override_path: Option<&str>) -> Result<String> {
    if let Some(p) = override_path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(probed) = crate::platform::find_chrome_executable() {
        app_debug!(
            "browser",
            "spawn",
            "Using probed Chrome executable: {}",
            probed.display()
        );
        return Ok(probed.to_string_lossy().into_owned());
    }
    if let Some(cached) = crate::browser::runtime::cached_binary_path() {
        app_info!(
            "browser",
            "spawn",
            "Using cached Chromium runtime: {}",
            cached.display()
        );
        return Ok(cached.to_string_lossy().into_owned());
    }
    anyhow::bail!(
        "No Chrome / Chromium found on this system. Options:\n\
         1. Install Google Chrome from https://www.google.com/chrome/\n\
         2. Run `profile.op=install_runtime` to download a Chromium runtime (~150 MB)\n\
         3. Pass `executable_path` to override with a custom Chrome binary"
    );
}

/// Build the `std::process::Command` for one Chrome spawn. Extracted so it
/// can be unit-tested without actually starting a child process.
pub fn build_chrome_argv(spec: &LaunchSpec<'_>, exec: &str) -> Command {
    let mut cmd = Command::new(exec);
    // Suppress the transient console window on Windows; Chrome's own window
    // (headed) or nothing (headless) is unaffected.
    crate::platform::hide_console(&mut cmd);
    cmd.arg(format!("--remote-debugging-port={}", spec.port));
    cmd.arg("--remote-debugging-address=127.0.0.1");

    let mut udd_arg = OsString::from("--user-data-dir=");
    udd_arg.push(spec.user_data_dir);
    cmd.arg(udd_arg);

    cmd.arg("--no-first-run");
    cmd.arg("--no-default-browser-check");
    // Stability flags. We replaced `chromiumoxide::BrowserConfig::launch`
    // with a hand-written argv but still need the subset of its defaults
    // that protect against environment-specific crashes — especially
    // `--disable-dev-shm-usage` for the documented Docker / headless server
    // deployment where `/dev/shm` defaults to 64 MB and the renderer OOMs
    // mid-navigation on anything non-trivial. The other `--disable-*`
    // flags are crash / IPC / popup / sync hardening with no UX cost.
    //
    // Deliberately omitted: `--enable-automation`, `--use-mock-keychain`,
    // `--password-store=basic`. Those would show Chrome's "controlled by
    // automated test software" infobar AND trip Google's anti-automation
    // sign-in gate, defeating the whole point of `profile=user_attach`.
    cmd.arg("--disable-dev-shm-usage");
    cmd.arg("--disable-background-networking");
    cmd.arg("--disable-background-timer-throttling");
    cmd.arg("--disable-backgrounding-occluded-windows");
    cmd.arg("--disable-renderer-backgrounding");
    cmd.arg("--disable-breakpad");
    cmd.arg("--disable-client-side-phishing-detection");
    cmd.arg("--disable-hang-monitor");
    cmd.arg("--disable-ipc-flooding-protection");
    cmd.arg("--disable-popup-blocking");
    cmd.arg("--disable-prompt-on-repost");
    cmd.arg("--disable-sync");
    cmd.arg("--metrics-recording-only");
    cmd.arg(format!(
        "--window-size={},{}",
        MANAGED_BROWSER_WINDOW_WIDTH, MANAGED_BROWSER_WINDOW_HEIGHT
    ));

    if spec.headless {
        cmd.arg("--headless=new");
    }

    // Debian Chromium inside the official Docker image may fail before opening
    // CDP when the host disables user namespaces. Keep the weaker sandbox only
    // for the container build we control; desktop launches retain Chrome's
    // normal sandbox unless the user adds their own extra arg.
    if crate::browser::profile::deployment_is_docker()
        && !has_chrome_flag(spec.extra_args, "--no-sandbox")
    {
        cmd.arg("--no-sandbox");
    }

    for extra in spec.extra_args {
        cmd.arg(extra);
    }

    cmd.arg("about:blank");
    cmd
}

fn has_chrome_flag(args: &[String], flag: &str) -> bool {
    let flag_with_value = format!("{flag}=");
    args.iter()
        .any(|arg| arg == flag || arg.starts_with(&flag_with_value))
}

/// Pick a free TCP port on 127.0.0.1 for managed-launch Chrome. The OS picks,
/// we close the listener, and Chrome immediately rebinds — the TOCTOU window
/// is ms-scale and we accept it (collision means launch fails fast and the
/// caller retries).
pub async fn pick_managed_port() -> Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_chrome_argv_includes_required_flags() {
        let udd = PathBuf::from("/tmp/test-udd");
        let extra = Vec::<String>::new();
        let spec = LaunchSpec {
            profile: "test",
            executable: None,
            user_data_dir: &udd,
            port: 9333,
            headless: false,
            extra_args: &extra,
        };
        let cmd = build_chrome_argv(&spec, "/usr/bin/google-chrome");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(argv.iter().any(|a| a == "--remote-debugging-port=9333"));
        assert!(argv.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert!(argv.iter().any(|a| a == "--no-first-run"));
        assert!(argv.iter().any(|a| a == "--no-default-browser-check"));
        assert!(argv.iter().any(|a| a == "--window-size=1440,960"));
        assert!(argv.iter().any(|a| a == "about:blank"));
        // headless flag should NOT be present in headful mode
        assert!(!argv.iter().any(|a| a == "--headless=new"));
        // Docker stability flag — sentinel for the full stability set.
        assert!(argv.iter().any(|a| a == "--disable-dev-shm-usage"));
        // Must NOT include automation-tagging flags that would tip Google's
        // anti-automation sign-in gate and show the "automated" infobar.
        assert!(!argv.iter().any(|a| a == "--enable-automation"));
        assert!(!argv.iter().any(|a| a == "--use-mock-keychain"));
    }

    #[test]
    fn build_chrome_argv_emits_headless_flag_when_requested() {
        let udd = PathBuf::from("/tmp/test-udd");
        let extra = Vec::<String>::new();
        let spec = LaunchSpec {
            profile: "test",
            executable: None,
            user_data_dir: &udd,
            port: 9333,
            headless: true,
            extra_args: &extra,
        };
        let cmd = build_chrome_argv(&spec, "/usr/bin/google-chrome");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(argv.iter().any(|a| a == "--headless=new"));
    }

    #[test]
    fn build_chrome_argv_appends_extra_args() {
        let udd = PathBuf::from("/tmp/test-udd");
        let extra = vec![
            "--proxy-server=http://proxy.example:8080".to_string(),
            "--ignore-certificate-errors".to_string(),
        ];
        let spec = LaunchSpec {
            profile: "test",
            executable: None,
            user_data_dir: &udd,
            port: 9333,
            headless: false,
            extra_args: &extra,
        };
        let cmd = build_chrome_argv(&spec, "/usr/bin/google-chrome");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(argv
            .iter()
            .any(|a| a == "--proxy-server=http://proxy.example:8080"));
        assert!(argv.iter().any(|a| a == "--ignore-certificate-errors"));
    }

    #[tokio::test]
    async fn pick_managed_port_returns_available_port() {
        let p1 = pick_managed_port().await.expect("port 1");
        let p2 = pick_managed_port().await.expect("port 2");
        assert!(p1 > 0);
        assert!(p2 > 0);
        // Same kernel call gives different ports most of the time, but
        // collision is technically possible — just assert both are valid.
    }

    #[test]
    fn resolve_chrome_executable_returns_override_when_non_empty() {
        let exec = resolve_chrome_executable(Some("/path/to/chrome")).expect("resolve");
        assert_eq!(exec, "/path/to/chrome");
    }

    #[test]
    fn resolve_chrome_executable_ignores_empty_override_and_probes() {
        // With an empty override the function should fall through to the
        // platform probe. We can't guarantee Chrome is installed on CI, so
        // just assert that the function does *not* return the empty string.
        let result = resolve_chrome_executable(Some("   "));
        match result {
            Ok(p) => assert!(!p.trim().is_empty(), "should not return empty path"),
            Err(_) => { /* CI without Chrome — fine */ }
        }
    }
}

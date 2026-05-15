//! Graceful + forceful quit helpers for the user's daily browser.
//!
//! `target=system` may need to close a running Chrome so it can take over
//! the same user-data-dir. We always try platform-native graceful
//! shutdown first (so Chrome runs its own atexit handlers and saves
//! session state), then escalate to forceful kill only when graceful
//! quit fails to release the SingletonLock within a deadline.

use crate::platform::chrome_paths::ChromeBrand;
use anyhow::Result;
use tokio::process::Command;

/// Best-effort graceful quit. Returns once the request has been issued —
/// callers should `singleton_lock::wait_for_release` to confirm.
pub async fn graceful_quit(brand: ChromeBrand) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // `osascript tell application "..." to quit` triggers the same
        // Quit path as ⌘Q from the menu, so Chrome saves session state
        // and respects the "warn before closing multiple windows" pref.
        let app = brand.macos_app_name();
        let script = format!("tell application \"{}\" to quit", app);
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .kill_on_drop(true)
            .output()
            .await;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // SIGTERM lets Chrome run its shutdown hooks.
        let pat = brand.linux_bin_pattern();
        let _ = Command::new("pkill")
            .args(["-TERM", "-f", pat])
            .kill_on_drop(true)
            .output()
            .await;
    }
    #[cfg(target_os = "windows")]
    {
        // taskkill without /F sends WM_CLOSE / CTRL_BREAK so Chrome can
        // self-clean up before exiting.
        let exe = brand.windows_exe_name();
        let _ = Command::new("taskkill")
            .args(["/IM", exe, "/T"])
            .kill_on_drop(true)
            .output()
            .await;
    }
    let _ = brand; // silence unused warning on unknown OS
    Ok(())
}

/// Escalation when graceful quit didn't release the user-data-dir lock.
/// Data loss risk is real here — callers MUST have user consent already.
pub async fn force_kill(brand: ChromeBrand) -> Result<()> {
    #[cfg(unix)]
    {
        // `pkill -KILL -f <pattern>` works on macOS too; the pattern
        // matches the binary name in argv[0].
        let pat = brand.linux_bin_pattern();
        let _ = Command::new("pkill")
            .args(["-KILL", "-f", pat])
            .kill_on_drop(true)
            .output()
            .await;
    }
    #[cfg(windows)]
    {
        let exe = brand.windows_exe_name();
        let _ = Command::new("taskkill")
            .args(["/IM", exe, "/T", "/F"])
            .kill_on_drop(true)
            .output()
            .await;
    }
    let _ = brand;
    Ok(())
}

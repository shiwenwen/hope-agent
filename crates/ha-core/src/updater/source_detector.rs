//! Detect how the running `hope-agent` binary was installed so the
//! self-update flow can pick the right upgrade path (package manager vs
//! self-contained binary swap).
//!
//! The detector is intentionally coarse — it looks at the path returned by
//! `std::env::current_exe()` and matches against well-known prefixes from
//! each distribution channel ([release.yml](.github/workflows/release.yml)
//! and the `update-*` workflows). When nothing matches we return
//! [`InstallSource::Manual`] and the caller routes to the self-contained
//! path; misclassification only ever falls back to a strictly safer route.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallSource {
    /// Homebrew cask (`brew install --cask hope-agent`). `prefix` is the
    /// detected `brew --prefix` (`/opt/homebrew` on Apple Silicon, `/usr/local`
    /// on Intel macOS / Linuxbrew) so the upgrade helper can pass `--prefix`
    /// explicitly and tolerate the user keeping two brews installed.
    Brew { prefix: PathBuf },
    /// Scoop (`scoop install hope-agent`) under `~/scoop/apps/hope-agent/`.
    Scoop,
    /// Arch / Manjaro AUR (`yay -S hope-agent-bin`).
    Aur,
    /// Debian / Ubuntu apt repo (`apt install hope-agent`).
    Apt,
    /// Fedora / RHEL dnf repo.
    Dnf,
    /// Tauri-bundled desktop app — DMG dragged to `/Applications`, Windows
    /// MSI / NSIS installer, Linux `.AppImage` left in place. These all
    /// still ship with `tauri-plugin-updater` wired up, so the desktop
    /// path handles them via the bundled updater rather than the
    /// self-contained binary swap.
    TauriBundle,
    /// Container deployment (`HA_DEPLOYMENT=docker` baked into the image
    /// ENV). Binary swap inside the container would be wiped on the next
    /// `docker pull`, so the upgrade flow routes to a manual prompt that
    /// tells the user to pull a new image instead.
    Docker,
    /// Anything else — single-file drop into `/usr/local/bin`, dev build
    /// via `cargo run`, custom deploy. Route to the self-contained
    /// updater unless the user picks otherwise.
    Manual,
}

impl InstallSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Brew { .. } => "homebrew",
            Self::Scoop => "scoop",
            Self::Aur => "aur",
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::TauriBundle => "tauri_bundle",
            Self::Docker => "docker",
            Self::Manual => "manual",
        }
    }
}

/// Probe the current process binary and classify it.
pub fn detect_install_source() -> InstallSource {
    let env_deployment = std::env::var("HA_DEPLOYMENT").ok();
    let exe = std::env::current_exe().ok();
    detect_install_source_with(env_deployment.as_deref(), exe.as_deref())
}

/// Pure version of [`detect_install_source`] for unit tests — host
/// look-ups (env var, current_exe) are funneled through arguments so the
/// same matrix can be exercised across platforms in CI. The
/// `HA_DEPLOYMENT` env var short-circuits path-based heuristics so
/// container deployments are classified correctly on any host OS.
pub fn detect_install_source_with(
    env_deployment: Option<&str>,
    exe: Option<&Path>,
) -> InstallSource {
    if env_deployment == Some("docker") {
        return InstallSource::Docker;
    }
    match exe {
        Some(p) => classify_exe_path(p),
        None => InstallSource::Manual,
    }
}

/// Classify an exe path against the per-OS install-layout heuristics.
/// Path-only classifier — `detect_install_source_with` handles the env-var
/// short-circuit before falling through here, and unit tests can exercise
/// the same matrix across platforms in CI by feeding paths directly.
pub fn classify_exe_path(exe: &Path) -> InstallSource {
    let s = exe.to_string_lossy();

    #[cfg(target_os = "macos")]
    {
        // brew cask installs land at
        // `<prefix>/Caskroom/hope-agent/.../Hope Agent.app/Contents/MacOS/hope-agent`,
        // which also matches the `.app/` substring used to detect a raw
        // DMG drop. Check brew first so cask installs route to the
        // package manager rather than the (signing-sensitive) Tauri path.
        if let Some(prefix) = brew_prefix_from_path(&s) {
            return InstallSource::Brew { prefix };
        }
        if s.contains("/Applications/Hope Agent.app/") || s.contains("/Hope Agent.app/") {
            return InstallSource::TauriBundle;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(prefix) = brew_prefix_from_path(&s) {
            return InstallSource::Brew { prefix };
        }
        if s.contains("/.local/share/AppImage")
            || s.ends_with(".AppImage")
            || s.contains("/squashfs-root/")
        {
            return InstallSource::TauriBundle;
        }
        // dpkg-shipped binaries live under /usr/bin or /usr/lib/<pkg>/.
        // Use `dpkg -S <exe>` only as a fallback — the path heuristic
        // already covers the standard install layout from the apt repo
        // generated in `.github/workflows/update-linux-repo.yml`.
        if s.starts_with("/usr/bin/") || s.starts_with("/usr/local/bin/") {
            // Either could be apt / dnf / manual. Probe the package
            // managers in turn; first owner wins. Probes are best-effort
            // so we don't hard-fail when the binary isn't on PATH.
            if owned_by_dpkg(exe) {
                return InstallSource::Apt;
            }
            if owned_by_rpm(exe) {
                return InstallSource::Dnf;
            }
            if owned_by_pacman(exe) {
                return InstallSource::Aur;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let lower = s.to_lowercase();
        if lower.contains("\\scoop\\apps\\hope-agent\\") {
            return InstallSource::Scoop;
        }
        if lower.contains("\\program files\\hope agent\\")
            || lower.contains("\\appdata\\local\\hope agent\\")
        {
            return InstallSource::TauriBundle;
        }
    }

    InstallSource::Manual
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn brew_prefix_from_path(exe_str: &str) -> Option<PathBuf> {
    for prefix in ["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"] {
        if exe_str.starts_with(&format!("{prefix}/Caskroom/hope-agent/"))
            || exe_str.starts_with(&format!("{prefix}/Cellar/hope-agent/"))
            || exe_str.starts_with(&format!("{prefix}/opt/hope-agent/"))
        {
            return Some(PathBuf::from(prefix));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn owned_by_dpkg(exe: &Path) -> bool {
    std::process::Command::new("dpkg")
        .arg("-S")
        .arg(exe)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("hope-agent"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn owned_by_rpm(exe: &Path) -> bool {
    std::process::Command::new("rpm")
        .arg("-qf")
        .arg(exe)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("hope-agent"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn owned_by_pacman(exe: &Path) -> bool {
    std::process::Command::new("pacman")
        .args(["-Qo"])
        .arg(exe)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("hope-agent"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn classifies_application_bundle_as_tauri() {
        let p = Path::new("/Applications/Hope Agent.app/Contents/MacOS/hope-agent");
        assert_eq!(classify_exe_path(p), InstallSource::TauriBundle);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn classifies_apple_silicon_brew_cask() {
        let p = Path::new(
            "/opt/homebrew/Caskroom/hope-agent/0.1.1/Hope Agent.app/Contents/MacOS/hope-agent",
        );
        match classify_exe_path(p) {
            InstallSource::Brew { prefix } => assert_eq!(prefix, PathBuf::from("/opt/homebrew")),
            other => panic!("expected Brew, got {other:?}"),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn classifies_intel_brew_as_usr_local() {
        let p = Path::new(
            "/usr/local/Caskroom/hope-agent/0.1.1/Hope Agent.app/Contents/MacOS/hope-agent",
        );
        match classify_exe_path(p) {
            InstallSource::Brew { prefix } => assert_eq!(prefix, PathBuf::from("/usr/local")),
            other => panic!("expected Brew, got {other:?}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn classifies_appimage() {
        let p = Path::new("/home/me/.local/share/AppImage/hope-agent-0.1.1.AppImage");
        assert_eq!(classify_exe_path(p), InstallSource::TauriBundle);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn classifies_unknown_layout_as_manual() {
        let p = Path::new("/home/me/projects/hope-agent/target/debug/hope-agent");
        assert_eq!(classify_exe_path(p), InstallSource::Manual);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn classifies_scoop() {
        let p = Path::new(r"C:\Users\me\scoop\apps\hope-agent\current\hope-agent.exe");
        assert_eq!(classify_exe_path(p), InstallSource::Scoop);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn classifies_program_files_as_tauri() {
        let p = Path::new(r"C:\Program Files\Hope Agent\hope-agent.exe");
        assert_eq!(classify_exe_path(p), InstallSource::TauriBundle);
    }

    #[test]
    fn ha_deployment_docker_short_circuits_path_heuristics() {
        // Even when the exe path would otherwise classify as TauriBundle /
        // Brew / etc., the env var wins so container deployments are never
        // mis-routed to a binary-swap path.
        #[cfg(target_os = "macos")]
        let exe = Path::new("/Applications/Hope Agent.app/Contents/MacOS/hope-agent");
        #[cfg(target_os = "linux")]
        let exe = Path::new("/usr/local/bin/hope-agent");
        #[cfg(target_os = "windows")]
        let exe = Path::new(r"C:\Program Files\Hope Agent\hope-agent.exe");
        assert_eq!(
            detect_install_source_with(Some("docker"), Some(exe)),
            InstallSource::Docker
        );
    }

    #[test]
    fn no_env_falls_through_to_path_classification() {
        let p = Path::new("/home/me/projects/hope-agent/target/release/hope-agent");
        let got = detect_install_source_with(None, Some(p));
        // Not Docker — falls through to classify_exe_path.
        assert_ne!(got, InstallSource::Docker);
    }
}

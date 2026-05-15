//! Resolve user-data-dir + executable for the user's daily browser.
//!
//! Used by `profile.op=launch target=system` to attach the user's REAL
//! browsing profile (cookies, extensions, logins) rather than an
//! isolated hope-agent dir. Brand + user-data-dir are kept paired so
//! we can't accidentally point a Chrome binary at a Chromium profile
//! (which would corrupt the profile on first write).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeBrand {
    Chrome,
    Edge,
    Brave,
    Chromium,
}

impl ChromeBrand {
    pub fn display_name(self) -> &'static str {
        match self {
            ChromeBrand::Chrome => "Google Chrome",
            ChromeBrand::Edge => "Microsoft Edge",
            ChromeBrand::Brave => "Brave",
            ChromeBrand::Chromium => "Chromium",
        }
    }

    /// macOS .app bundle name used by `osascript tell application "..."`.
    pub fn macos_app_name(self) -> &'static str {
        match self {
            ChromeBrand::Chrome => "Google Chrome",
            ChromeBrand::Edge => "Microsoft Edge",
            ChromeBrand::Brave => "Brave Browser",
            ChromeBrand::Chromium => "Chromium",
        }
    }

    /// Substring used by `pkill -f` to match the running binary.
    pub fn linux_bin_pattern(self) -> &'static str {
        match self {
            ChromeBrand::Chrome => "google-chrome",
            ChromeBrand::Edge => "microsoft-edge",
            ChromeBrand::Brave => "brave",
            ChromeBrand::Chromium => "chromium",
        }
    }

    /// Image name used by `taskkill /IM`.
    pub fn windows_exe_name(self) -> &'static str {
        match self {
            ChromeBrand::Chrome => "chrome.exe",
            ChromeBrand::Edge => "msedge.exe",
            ChromeBrand::Brave => "brave.exe",
            // Chromium's Windows build also ships as chrome.exe — collides
            // with Google Chrome's image name, but we still detect them
            // separately via install path / user-data-dir.
            ChromeBrand::Chromium => "chrome.exe",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChromeInstallation {
    pub brand: ChromeBrand,
    pub executable: PathBuf,
    pub user_data_dir: PathBuf,
}

/// Probe brand by brand, returning the first installation where both
/// the executable AND the user-data-dir exist on disk. Order reflects
/// rough market share among hope-agent users.
pub fn detect_daily_browser() -> Option<ChromeInstallation> {
    for brand in [
        ChromeBrand::Chrome,
        ChromeBrand::Edge,
        ChromeBrand::Brave,
        ChromeBrand::Chromium,
    ] {
        if let (Some(executable), Some(user_data_dir)) =
            (executable_for(brand), user_data_dir_for(brand))
        {
            if executable.exists() && user_data_dir.exists() {
                return Some(ChromeInstallation {
                    brand,
                    executable,
                    user_data_dir,
                });
            }
        }
    }
    None
}

pub fn executable_for(brand: ChromeBrand) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let path = match brand {
            ChromeBrand::Chrome => "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ChromeBrand::Edge => "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ChromeBrand::Brave => "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ChromeBrand::Chromium => "/Applications/Chromium.app/Contents/MacOS/Chromium",
        };
        let p = PathBuf::from(path);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let primary = match brand {
            ChromeBrand::Chrome => "google-chrome",
            ChromeBrand::Edge => "microsoft-edge",
            ChromeBrand::Brave => "brave-browser",
            ChromeBrand::Chromium => "chromium",
        };
        if let Ok(p) = which::which(primary) {
            return Some(p);
        }
        // Distros and forks use different binary names; widen the search.
        let alt = match brand {
            ChromeBrand::Chrome => &["google-chrome-stable"][..],
            ChromeBrand::Edge => &["microsoft-edge-stable", "microsoft-edge-dev"][..],
            ChromeBrand::Brave => &["brave"][..],
            ChromeBrand::Chromium => &["chromium-browser"][..],
        };
        for name in alt {
            if let Ok(p) = which::which(name) {
                return Some(p);
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA").ok().map(PathBuf::from);
        let pf86 = std::env::var("ProgramFiles(x86)").ok().map(PathBuf::from);
        let pf = std::env::var("ProgramFiles").ok().map(PathBuf::from);
        let candidates: Vec<PathBuf> = match brand {
            ChromeBrand::Chrome => [
                pf.as_ref()
                    .map(|p| p.join("Google/Chrome/Application/chrome.exe")),
                pf86.as_ref()
                    .map(|p| p.join("Google/Chrome/Application/chrome.exe")),
                local
                    .as_ref()
                    .map(|p| p.join("Google/Chrome/Application/chrome.exe")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ChromeBrand::Edge => [
                pf.as_ref()
                    .map(|p| p.join("Microsoft/Edge/Application/msedge.exe")),
                pf86.as_ref()
                    .map(|p| p.join("Microsoft/Edge/Application/msedge.exe")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ChromeBrand::Brave => [
                pf.as_ref()
                    .map(|p| p.join("BraveSoftware/Brave-Browser/Application/brave.exe")),
                local
                    .as_ref()
                    .map(|p| p.join("BraveSoftware/Brave-Browser/Application/brave.exe")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ChromeBrand::Chromium => [local
                .as_ref()
                .map(|p| p.join("Chromium/Application/chrome.exe"))]
            .into_iter()
            .flatten()
            .collect(),
        };
        candidates.into_iter().find(|p| p.exists())
    }
}

pub fn user_data_dir_for(brand: ChromeBrand) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    #[cfg(target_os = "macos")]
    {
        let app_support = home.join("Library").join("Application Support");
        Some(match brand {
            ChromeBrand::Chrome => app_support.join("Google/Chrome"),
            ChromeBrand::Edge => app_support.join("Microsoft Edge"),
            ChromeBrand::Brave => app_support.join("BraveSoftware/Brave-Browser"),
            ChromeBrand::Chromium => app_support.join("Chromium"),
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let config = home.join(".config");
        Some(match brand {
            ChromeBrand::Chrome => config.join("google-chrome"),
            ChromeBrand::Edge => config.join("microsoft-edge"),
            ChromeBrand::Brave => config.join("BraveSoftware/Brave-Browser"),
            ChromeBrand::Chromium => config.join("chromium"),
        })
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)?;
        Some(match brand {
            ChromeBrand::Chrome => local.join("Google/Chrome/User Data"),
            ChromeBrand::Edge => local.join("Microsoft/Edge/User Data"),
            ChromeBrand::Brave => local.join("BraveSoftware/Brave-Browser/User Data"),
            ChromeBrand::Chromium => local.join("Chromium/User Data"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_display_names_are_human_readable() {
        assert_eq!(ChromeBrand::Chrome.display_name(), "Google Chrome");
        assert_eq!(ChromeBrand::Edge.display_name(), "Microsoft Edge");
        assert_eq!(ChromeBrand::Brave.display_name(), "Brave");
        assert_eq!(ChromeBrand::Chromium.display_name(), "Chromium");
    }

    #[test]
    fn brand_macos_app_names_are_quit_target() {
        // These must match the bundle's CFBundleName so `osascript` can
        // address the running app. Don't rename casually.
        assert_eq!(ChromeBrand::Chrome.macos_app_name(), "Google Chrome");
        assert_eq!(ChromeBrand::Brave.macos_app_name(), "Brave Browser");
    }

    #[test]
    fn user_data_dir_is_brand_specific() {
        // We don't assert exact paths (varies by HOME under tests), but
        // each brand must map to a distinct dir to prevent profile mixing.
        let dirs: Vec<_> = [
            ChromeBrand::Chrome,
            ChromeBrand::Edge,
            ChromeBrand::Brave,
            ChromeBrand::Chromium,
        ]
        .iter()
        .filter_map(|b| user_data_dir_for(*b))
        .collect();
        let mut unique = dirs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            dirs.len(),
            "brands must map to distinct user-data-dirs to avoid profile corruption"
        );
    }
}

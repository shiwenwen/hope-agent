//! Browser control surface for the desktop UI.
//!
//! Thin helpers on top of [`browser_state`] that let the settings panel manage
//! dedicated browser profiles (each profile is a Chrome `user-data-dir` under
//! `~/.hope-agent/browser-profiles/`) and drive the lifecycle of the
//! app-owned Chrome instance. The underlying CDP connection, tab management
//! and automation tools remain unchanged — this module only exposes what the
//! user-facing panel needs.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::browser_state::get_browser_state;
use crate::paths::{browser_profile_dir, browser_profiles_dir};

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfileInfo {
    pub name: String,
    pub path: String,
    /// Disk size (bytes) of the profile directory, best-effort.
    pub size_bytes: u64,
    /// Last modified timestamp of the profile directory (unix secs), if known.
    pub last_used_at: Option<i64>,
    /// True when this is the profile the current connection was launched with.
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTabInfo {
    pub target_id: String,
    pub url: String,
    pub title: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub connected: bool,
    /// `launch` when this process owns Chrome; `connect` when attached to an
    /// externally started Chrome; `null` when not connected.
    pub mode: Option<String>,
    pub profile: Option<String>,
    pub connection_url: Option<String>,
    pub profiles_dir: String,
    pub tabs: Vec<BrowserTabInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    pub profile: Option<String>,
    pub executable_path: Option<String>,
    #[serde(default)]
    pub headless: bool,
}

// ── Profile management ──────────────────────────────────────────────────

/// Validate a profile name (prevents directory traversal / weird chars).
fn validate_profile_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Profile name cannot be empty"));
    }
    if trimmed.len() > 64 {
        return Err(anyhow!("Profile name too long (max 64 chars)"));
    }
    if trimmed != name {
        return Err(anyhow!(
            "Profile name cannot have leading/trailing whitespace"
        ));
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if !ok {
        return Err(anyhow!(
            "Profile name may only contain letters, digits, '-', '_' and '.'"
        ));
    }
    if name.starts_with('.') {
        return Err(anyhow!("Profile name cannot start with '.'"));
    }
    Ok(())
}

fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let walker = match std::fs::read_dir(path) {
        Ok(w) => w,
        Err(_) => return 0,
    };
    for entry in walker.flatten() {
        let ep = entry.path();
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            } else if meta.is_dir() {
                total = total.saturating_add(dir_size_bytes(&ep));
            }
        }
    }
    total
}

fn last_modified_secs(path: &std::path::Path) -> Option<i64> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let dur = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_secs() as i64)
}

pub async fn list_profiles() -> Result<Vec<BrowserProfileInfo>> {
    let root = browser_profiles_dir()?;
    std::fs::create_dir_all(&root)?;

    let active_profile = {
        let st = get_browser_state().lock().await;
        if st.is_connected() {
            st.profile.clone()
        } else {
            None
        }
    };

    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if validate_profile_name(&name).is_err() {
            // Skip anything that doesn't look like a profile we manage
            continue;
        }
        let size = dir_size_bytes(&path);
        let last = last_modified_secs(&path);
        let is_active = active_profile.as_deref() == Some(name.as_str());
        out.push(BrowserProfileInfo {
            name,
            path: path.to_string_lossy().to_string(),
            size_bytes: size,
            last_used_at: last,
            is_active,
        });
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

pub async fn create_profile(name: &str) -> Result<BrowserProfileInfo> {
    validate_profile_name(name)?;
    let dir = browser_profile_dir(name)?;
    if dir.exists() {
        return Err(anyhow!("Profile '{}' already exists", name));
    }
    std::fs::create_dir_all(&dir)?;
    app_info!("browser", "ui", "Created browser profile '{}'", name);
    Ok(BrowserProfileInfo {
        name: name.to_string(),
        path: dir.to_string_lossy().to_string(),
        size_bytes: 0,
        last_used_at: last_modified_secs(&dir),
        is_active: false,
    })
}

pub async fn delete_profile(name: &str) -> Result<()> {
    validate_profile_name(name)?;

    // Reject if this profile is currently connected — user must disconnect first.
    {
        let st = get_browser_state().lock().await;
        if st.is_connected() && st.profile.as_deref() == Some(name) {
            return Err(anyhow!(
                "Profile '{}' is currently in use. Disconnect the browser first.",
                name
            ));
        }
    }

    let dir = browser_profile_dir(name)?;
    if !dir.exists() {
        return Err(anyhow!("Profile '{}' not found", name));
    }
    std::fs::remove_dir_all(&dir)?;
    app_info!("browser", "ui", "Deleted browser profile '{}'", name);
    Ok(())
}

// ── Lifecycle ───────────────────────────────────────────────────────────

async fn collect_tabs() -> Vec<BrowserTabInfo> {
    let st = get_browser_state().lock().await;
    let mut tabs = Vec::with_capacity(st.pages.len());
    let active = st.active_page_id.clone();
    for (target_id, page) in st.pages.iter() {
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let title: String = page
            .evaluate("document.title")
            .await
            .ok()
            .and_then(|r| r.into_value().ok())
            .unwrap_or_default();
        tabs.push(BrowserTabInfo {
            target_id: target_id.clone(),
            url,
            title,
            is_active: active.as_deref() == Some(target_id),
        });
    }
    tabs
}

pub async fn get_status() -> Result<BrowserStatus> {
    let profiles_dir: PathBuf = browser_profiles_dir()?;
    let _ = std::fs::create_dir_all(&profiles_dir);

    let (connected, profile, connection_url, mode) = {
        let st = get_browser_state().lock().await;
        let connected = st.is_connected();
        let mode = if !connected {
            None
        } else if st.connection_url.is_some() {
            Some("connect".to_string())
        } else {
            Some("launch".to_string())
        };
        (
            connected,
            st.profile.clone(),
            st.connection_url.clone(),
            mode,
        )
    };

    let tabs = if connected {
        // Best-effort refresh so the panel shows tabs opened via the real
        // Chrome window; swallow errors and keep returning whatever we have.
        let _ = {
            let mut st = get_browser_state().lock().await;
            st.refresh_pages().await
        };
        collect_tabs().await
    } else {
        Vec::new()
    };

    Ok(BrowserStatus {
        connected,
        mode,
        profile,
        connection_url,
        profiles_dir: profiles_dir.to_string_lossy().to_string(),
        tabs,
    })
}

pub async fn launch(opts: LaunchOptions) -> Result<BrowserStatus> {
    if let Some(p) = opts.profile.as_deref() {
        validate_profile_name(p)?;
        let dir = browser_profile_dir(p)?;
        std::fs::create_dir_all(&dir)?;
    }

    // If already connected, disconnect first so we don't leak the handle.
    {
        let mut st = get_browser_state().lock().await;
        if st.browser.is_some() {
            st.disconnect().await;
        }
    }

    {
        let mut st = get_browser_state().lock().await;
        st.launch(
            opts.executable_path.as_deref(),
            opts.headless,
            opts.profile.as_deref(),
        )
        .await?;
    }

    app_info!(
        "browser",
        "ui",
        "Launched browser profile={:?} headless={}",
        opts.profile,
        opts.headless
    );
    get_status().await
}

pub async fn connect(debug_url: &str) -> Result<BrowserStatus> {
    let url = debug_url.trim();
    if url.is_empty() {
        return Err(anyhow!("Debug URL is required"));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(anyhow!("Debug URL must start with http:// or https://"));
    }

    {
        let mut st = get_browser_state().lock().await;
        if st.browser.is_some() {
            st.disconnect().await;
        }
    }

    {
        let mut st = get_browser_state().lock().await;
        st.connect(url).await?;
    }

    app_info!("browser", "ui", "Connected to external Chrome at {}", url);
    get_status().await
}

pub async fn disconnect() -> Result<BrowserStatus> {
    {
        let mut st = get_browser_state().lock().await;
        if st.browser.is_some() {
            st.disconnect().await;
        }
    }
    get_status().await
}

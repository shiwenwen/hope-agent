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
    /// `None` = inherit the profile's configured default (per-profile
    /// `headless` in settings, falling back to false). `Some(_)` is a
    /// one-shot override from this specific call. Previously a bare
    /// `bool` with `#[serde(default)]`, which silently degraded every
    /// browser-launch coming in over HTTP/UI without an explicit headless
    /// field into `headless=false` — broke server / Docker deployments
    /// whose profile is configured headless.
    #[serde(default)]
    pub headless: Option<bool>,
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
    // Snapshot phase: hold the lock only long enough to clone the active
    // tab id + page handles. The CDP round-trips (`page.url()`,
    // `page.evaluate("document.title")`) happen with the lock released so
    // a slow tab doesn't queue every other browser_state consumer (the
    // settings BrowserPanel polls this every refresh).
    let (active, handles) = {
        let st = get_browser_state().lock().await;
        let active = st.active_page_id.clone();
        let handles: Vec<(String, chromiumoxide::Page)> = st
            .pages
            .iter()
            .map(|(id, page)| (id.clone(), page.clone()))
            .collect();
        (active, handles)
    };
    let mut tabs = Vec::with_capacity(handles.len());
    for (target_id, page) in &handles {
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
            is_active: active.as_deref() == Some(target_id.as_str()),
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
        // `chrome_child.is_some()` is the source of truth for whether we
        // own the Chrome process. `connection_url` is now populated in both
        // launch and connect paths so it can't disambiguate the two.
        let mode = if !connected {
            None
        } else if st.has_chrome_child() {
            Some("launch".to_string())
        } else {
            Some("connect".to_string())
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
    }

    let profile_name = opts
        .profile
        .clone()
        .unwrap_or_else(crate::browser::profile::default_profile_name);
    let resolved = crate::browser::profile::resolve_profile(&profile_name)?;

    // `opts.headless = None` means "inherit profile default"; only
    // `Some(_)` overrides. This matters for HTTP / server deployments where
    // the profile is configured headless and the request body omits the
    // field — without the fallback the profile default would be silently
    // ignored and the launch would try headful in a no-display container.
    let headless = opts.headless.unwrap_or(resolved.headless);

    crate::browser::reset_backend().await;
    {
        let mut st = get_browser_state().lock().await;
        if st.needs_cleanup() {
            st.disconnect().await;
        }
        let exec_override = opts
            .executable_path
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| resolved.executable.clone());
        let port = match resolved.port {
            Some(p) => p,
            None => crate::browser::spawn::pick_managed_port().await?,
        };
        let extra = resolved.extra_args.clone();
        let spec = crate::browser::spawn::LaunchSpec {
            profile: &resolved.name,
            executable: exec_override.as_deref(),
            user_data_dir: &resolved.user_data_dir,
            port,
            headless,
            extra_args: &extra,
        };
        st.spawn_chrome_and_connect(spec).await?;
    }

    app_info!(
        "browser",
        "ui",
        "Launched browser profile={:?} headless={}",
        opts.profile,
        headless
    );
    get_status().await
}

pub async fn connect(debug_url: &str) -> Result<BrowserStatus> {
    // Shared scheme + SSRF guard so settings UI / HTTP / tool path all reject
    // the same set of dodgy debug URLs. Previously only checked the scheme
    // here, which would happily pass a public IP / private LAN address into
    // `Browser::connect` and bypass the SSRF policy.
    crate::browser::validate_cdp_endpoint_url(debug_url).await?;
    let url = debug_url.trim();

    crate::browser::reset_backend().await;
    {
        let mut st = get_browser_state().lock().await;
        if st.needs_cleanup() {
            st.disconnect().await;
        }
        st.connect(url).await?;
    }

    app_info!("browser", "ui", "Connected to external Chrome at {}", url);
    get_status().await
}

pub async fn disconnect() -> Result<BrowserStatus> {
    crate::browser::reset_backend().await;
    {
        let mut st = get_browser_state().lock().await;
        if st.needs_cleanup() {
            st.disconnect().await;
        }
    }
    get_status().await
}

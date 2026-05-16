//! First-class profile resolution.
//!
//! A "profile" is a named bundle of `{user_data_dir, port, executable,
//! headless, extra_args, color}` that the user picks via `profile.op=launch
//! profile=<name>`. Two profiles are always present:
//!
//! - `managed` — ephemeral runner under `~/.hope-agent/browser/managed-runner/`,
//!   OS-picked debug port. Default when no profile is specified.
//! - `user_attach` — persistent profile under `~/.hope-agent/browser/user-attach/`,
//!   well-known port 9222 (settings panel + "Reconnect" UX rely on it).
//!
//! Users can override either built-in via `AppConfig.browser.profiles` or
//! add their own profile names entirely. Per-profile fields are optional —
//! absent fields fall back to per-profile-name defaults below.

use std::path::PathBuf;

use anyhow::Result;

/// Concrete launch parameters for one profile, resolved from
/// [`crate::browser::BrowserConfig::profiles`] plus per-name defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProfile {
    pub name: String,
    pub user_data_dir: PathBuf,
    /// `None` → caller picks a free port via
    /// [`crate::browser::spawn::pick_managed_port`].
    pub port: Option<u16>,
    pub executable: Option<String>,
    pub headless: bool,
    pub extra_args: Vec<String>,
    pub color: Option<String>,
    /// `true` for the user_attach profile (cookies / logins meant to
    /// persist) and any user-defined profile (we assume persistence by
    /// default for those); `false` for the ephemeral `managed` runner.
    pub persistent: bool,
}

pub const BUILTIN_MANAGED: &str = "managed";
pub const BUILTIN_USER_ATTACH: &str = "user_attach";

/// Resolve a profile name to concrete launch parameters.
///
/// Steps:
/// 1. Look up the name in `AppConfig.browser.profiles` (user override).
/// 2. Apply built-in defaults for `managed` / `user_attach` to any unset
///    fields; for unknown names, default to `~/.hope-agent/browser-profiles/<name>/`
///    with OS-picked port.
pub fn resolve_profile(name: &str) -> Result<ResolvedProfile> {
    let cfg_entry = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.profiles.get(name).cloned())
        .unwrap_or_default();

    let (default_udd, default_port, default_persistent): (PathBuf, Option<u16>, bool) = match name {
        BUILTIN_MANAGED => (crate::paths::browser_managed_runner_dir()?, None, false),
        BUILTIN_USER_ATTACH => (crate::paths::browser_user_attach_dir()?, Some(9222), true),
        other => (crate::paths::browser_profile_dir(other)?, None, true),
    };

    let user_data_dir = match cfg_entry.user_data_dir.as_deref() {
        Some(s) => PathBuf::from(crate::tools::expand_tilde(s)),
        None => default_udd,
    };

    let port = cfg_entry.port.or(default_port);
    let headless = cfg_entry.headless.unwrap_or(false);

    Ok(ResolvedProfile {
        name: name.to_string(),
        user_data_dir,
        port,
        executable: cfg_entry.executable_path.filter(|s| !s.trim().is_empty()),
        headless,
        extra_args: cfg_entry.extra_args,
        color: cfg_entry.color,
        persistent: default_persistent,
    })
}

/// List all known profiles: union of built-in names + configured names.
/// Each entry is a fully resolved [`ResolvedProfile`].
pub fn list_profiles() -> Vec<ResolvedProfile> {
    let mut names: Vec<String> = vec![BUILTIN_MANAGED.into(), BUILTIN_USER_ATTACH.into()];
    if let Some(b) = crate::config::cached_config().browser.as_ref() {
        for key in b.profiles.keys() {
            if !names.iter().any(|n| n == key) {
                names.push(key.clone());
            }
        }
    }
    names
        .iter()
        .filter_map(|n| resolve_profile(n).ok())
        .collect()
}

/// The default profile name to use when `profile.op=launch` is called with
/// no `profile=` argument. Respects `AppConfig.browser.default_profile`.
pub fn default_profile_name() -> String {
    crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.default_profile.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| BUILTIN_MANAGED.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_profile_managed_uses_managed_runner_dir() {
        let p = resolve_profile(BUILTIN_MANAGED).expect("resolve managed");
        assert_eq!(p.name, BUILTIN_MANAGED);
        let s = p.user_data_dir.to_string_lossy();
        assert!(s.contains("managed-runner"), "got: {}", s);
        assert!(!p.persistent);
        assert_eq!(p.port, None);
        assert!(!p.headless);
    }

    #[test]
    fn resolve_profile_user_attach_pins_port_9222_and_persistent() {
        let p = resolve_profile(BUILTIN_USER_ATTACH).expect("resolve user_attach");
        assert_eq!(p.name, BUILTIN_USER_ATTACH);
        let s = p.user_data_dir.to_string_lossy();
        assert!(s.contains("user-attach"), "got: {}", s);
        assert!(p.persistent);
        assert_eq!(p.port, Some(9222));
    }

    #[test]
    fn resolve_profile_unknown_name_uses_browser_profiles_subdir() {
        let p = resolve_profile("work").expect("resolve work");
        let s = p.user_data_dir.to_string_lossy();
        assert!(
            s.contains("browser-profiles") && s.contains("work"),
            "got: {}",
            s
        );
        assert!(p.persistent);
    }

    #[test]
    fn list_profiles_includes_builtins() {
        let names: Vec<String> = list_profiles().into_iter().map(|p| p.name).collect();
        assert!(names.iter().any(|n| n == BUILTIN_MANAGED));
        assert!(names.iter().any(|n| n == BUILTIN_USER_ATTACH));
    }
}

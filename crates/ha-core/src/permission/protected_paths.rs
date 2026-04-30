//! Protected paths list — paths that always require manual approval, even
//! when the session is otherwise permissive (Default + custom_approval_tools
//! disabled, or AllowAlways already granted).
//!
//! YOLO modes still bypass this list (with `app_warn!` audit log) because the
//! user explicitly opted into "max permission".
//!
//! Storage: `~/.hope-agent/permission/protected-paths.json`. The file holds a
//! single `Vec<String>` of patterns. Missing file → defaults apply.

/// Default protected paths shipped with Hope Agent. Users can add / remove
/// via the GUI; "Restore defaults" rewrites the on-disk file with this list.
pub const DEFAULT_PROTECTED_PATHS: &[&str] = &[
    // Credentials and keys
    "~/.ssh/",
    "~/.aws/",
    "~/.gnupg/",
    "~/.config/gh/",
    "~/.hope-agent/credentials/",
    // System directories
    "/etc/",
    "/System/",
    "/Library/",
    "/usr/local/etc/",
    // Wildcards (matched by suffix / glob)
    ".env",
    ".env.*",
    "*secret*",
    "*credential*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
];

const FILE_NAME: &str = "protected-paths.json";

static CACHE: std::sync::LazyLock<super::list_store::Cache> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(None));

/// Currently-active protected path patterns. Backed by
/// `~/.hope-agent/permission/protected-paths.json`; falls back to
/// [`DEFAULT_PROTECTED_PATHS`] when the file is missing. Returns an `Arc`
/// snapshot — engine hot path only pays a refcount bump, not a Vec clone.
pub fn current_patterns() -> std::sync::Arc<Vec<String>> {
    super::list_store::load_or_defaults(&CACHE, FILE_NAME, DEFAULT_PROTECTED_PATHS)
}

/// Overwrite the user-customized list. Persists to disk + updates cache.
pub fn save_patterns(patterns: &[String]) -> anyhow::Result<()> {
    super::list_store::save(&CACHE, FILE_NAME, patterns)
}

/// Restore the compile-time defaults (writes them to disk too).
pub fn reset_defaults() -> anyhow::Result<Vec<String>> {
    super::list_store::reset_to_defaults(&CACHE, FILE_NAME, DEFAULT_PROTECTED_PATHS)
}

/// Read-only borrow of the compile-time defaults — drives the "Restore
/// defaults" preview before the user commits.
pub fn defaults() -> &'static [&'static str] {
    DEFAULT_PROTECTED_PATHS
}

/// Check whether `path` (already `~`-expanded) matches any pattern.
/// Returns the matched pattern so callers can surface it in audit logs.
///
/// Pattern semantics:
/// - Trailing `/` (`~/.ssh/`) → directory prefix match
/// - Plain leaf (`.env`) → exact filename match anywhere in the path
/// - Glob with `*` (`*.pem`, `*secret*`) → simple star-glob match against
///   both the full path string and the basename
/// `true` if `b` is a path-component separator on the current platform.
/// Unix only honors `/`; Windows honors both `/` and `\` — and Windows path
/// strings often mix them (e.g. `C:\Users\u\.ssh/id_rsa` from a join of a
/// `dirs::home_dir()` `PathBuf` and a forward-slash relative literal). The
/// boundary check has to accept either or it'll bail on legitimate prefix
/// matches whenever the join straddles the two separators.
#[inline]
fn is_path_separator_byte(b: u8) -> bool {
    if cfg!(windows) {
        b == b'/' || b == b'\\'
    } else {
        b == b'/'
    }
}

pub fn matches(path: &std::path::Path, patterns: &[String]) -> Option<String> {
    use super::rules::{expand_tilde, glob_match_simple};
    let path_str = path.to_string_lossy();
    for pat in patterns {
        let pat_str = pat.as_str();
        if pat_str.ends_with('/') {
            let expanded = expand_tilde(pat_str);
            let expanded_s = expanded.to_string_lossy();
            let prefix = expanded_s.trim_end_matches('/');
            if path_str == prefix
                || (path_str.len() > prefix.len()
                    && path_str.starts_with(prefix)
                    && is_path_separator_byte(path_str.as_bytes()[prefix.len()]))
            {
                return Some(pat.clone());
            }
        } else if pat_str.contains('*') {
            if glob_match_simple(pat_str, &path_str) {
                return Some(pat.clone());
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if glob_match_simple(pat_str, name) {
                    return Some(pat.clone());
                }
            }
        } else if !pat_str.contains('/') {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == pat_str {
                    return Some(pat.clone());
                }
            }
        } else {
            let expanded = expand_tilde(pat_str);
            let expanded_s = expanded.to_string_lossy();
            if path_str == expanded_s
                || (path_str.len() > expanded_s.len()
                    && path_str.starts_with(&*expanded_s)
                    && is_path_separator_byte(path_str.as_bytes()[expanded_s.len()]))
            {
                return Some(pat.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_non_empty() {
        assert!(!DEFAULT_PROTECTED_PATHS.is_empty());
    }

    #[test]
    fn defaults_include_ssh_dir() {
        assert!(DEFAULT_PROTECTED_PATHS.contains(&"~/.ssh/"));
    }
}

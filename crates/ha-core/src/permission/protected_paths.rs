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

/// Currently-active protected path patterns. The GUI editor will swap this
/// for a `Lazy<RwLock<Vec<String>>>` once user customization lands.
pub fn current_patterns() -> &'static [&'static str] {
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
pub fn matches(path: &std::path::Path, patterns: &[&'static str]) -> Option<&'static str> {
    use super::rules::{expand_tilde, glob_match_simple};
    let path_str = path.to_string_lossy();
    for &pat in patterns {
        if pat.ends_with('/') {
            // Directory prefix.
            let expanded = expand_tilde(pat);
            let expanded_s = expanded.to_string_lossy();
            let prefix = expanded_s.trim_end_matches('/');
            if path_str == prefix
                || (path_str.len() > prefix.len()
                    && path_str.starts_with(prefix)
                    && path_str.as_bytes()[prefix.len()] == b'/')
            {
                return Some(pat);
            }
        } else if pat.contains('*') {
            if glob_match_simple(pat, &path_str) {
                return Some(pat);
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if glob_match_simple(pat, name) {
                    return Some(pat);
                }
            }
        } else if !pat.contains('/') {
            // Plain leaf — match basename only.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == pat {
                    return Some(pat);
                }
            }
        } else {
            // Absolute / non-wildcard path — exact prefix match.
            let expanded = expand_tilde(pat);
            let expanded_s = expanded.to_string_lossy();
            if path_str == expanded_s
                || (path_str.len() > expanded_s.len()
                    && path_str.starts_with(&*expanded_s)
                    && path_str.as_bytes()[expanded_s.len()] == b'/')
            {
                return Some(pat);
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

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

/// Returns the current pattern list. Phase 2.1: falls back to defaults
/// (file IO is wired up in Phase 3 alongside the GUI editor).
pub fn current_patterns() -> Vec<String> {
    DEFAULT_PROTECTED_PATHS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Check whether `path` (already `~`-expanded) matches any pattern.
/// Returns the matched pattern (raw form) so callers can show it in audit logs.
///
/// Pattern semantics:
/// - Trailing `/` (`~/.ssh/`) → directory prefix match
/// - Plain leaf (`.env`) → exact filename match anywhere in the path
/// - Glob with `*` (`*.pem`, `*secret*`) → simple star-glob match against
///   the full path string
pub fn matches(path: &std::path::Path, patterns: &[String]) -> Option<String> {
    use super::rules::{expand_tilde, glob_match_simple};
    let path_str = path.to_string_lossy();
    for pat in patterns {
        if pat.ends_with('/') {
            // Directory prefix
            let expanded = expand_tilde(pat);
            let expanded_s = expanded.to_string_lossy();
            let prefix = expanded_s.trim_end_matches('/');
            if path_str == prefix || path_str.starts_with(&format!("{prefix}/")) {
                return Some(pat.clone());
            }
        } else if pat.contains('*') {
            // Star glob — try against the full path AND against the basename.
            if glob_match_simple(pat, &path_str) {
                return Some(pat.clone());
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if glob_match_simple(pat, name) {
                    return Some(pat.clone());
                }
            }
        } else if !pat.contains('/') {
            // Plain leaf — match by basename.
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == pat {
                    return Some(pat.clone());
                }
            }
        } else {
            // Path with no wildcards — exact prefix.
            let expanded = expand_tilde(pat);
            let expanded_s = expanded.to_string_lossy();
            if path_str == expanded_s || path_str.starts_with(&format!("{expanded_s}/")) {
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

//! Permission rule types — `PermissionRules` + `RuleSpec` + `ArgMatcher`.
//!
//! These are the data primitives used by:
//! - The hardcoded edit-class enforcement
//! - User AllowAlways accumulators (project / session / agent / global)
//! - The protected-paths / dangerous-commands / edit-commands lists
//!
//! Decision merging happens in [`super::engine`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A bag of rules at one scope (e.g. a project's allowlist file, or the
/// global allowlist). The engine collects multiple bags from different
/// scopes and merges them by priority.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRules {
    /// Allow without prompting.
    #[serde(default)]
    pub allow: Vec<RuleSpec>,
    /// Block outright (highest precedence within a scope).
    #[serde(default)]
    pub deny: Vec<RuleSpec>,
    /// Force-ask, even if allow rules would otherwise pass.
    #[serde(default)]
    pub ask: Vec<RuleSpec>,
}

impl PermissionRules {
    /// `true` when no allow / deny / ask rules are configured.
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty() && self.ask.is_empty()
    }
}

/// A single rule. Either matches by tool name alone, or by tool name plus
/// a parameter-level matcher (path prefix, command prefix, domain glob…).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleSpec {
    /// Match the tool by name regardless of args.
    Tool { name: String },
    /// Match the tool by name AND a parameter-level matcher.
    ToolPattern { name: String, matcher: ArgMatcher },
}

impl RuleSpec {
    /// The tool name this rule targets.
    pub fn tool_name(&self) -> &str {
        match self {
            Self::Tool { name } => name,
            Self::ToolPattern { name, .. } => name,
        }
    }

    /// Does this rule match the given tool call? `args` is the tool_call args
    /// JSON, used to extract path / command / domain when applicable.
    pub fn matches(&self, name: &str, args: &serde_json::Value) -> bool {
        if self.tool_name() != name {
            return false;
        }
        match self {
            Self::Tool { .. } => true,
            Self::ToolPattern { matcher, .. } => matcher.matches(name, args),
        }
    }
}

/// Parameter-level matcher. Each variant knows where in `args` to look.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ArgMatcher {
    /// `args.path` (or `cwd` for `exec`) starts with this prefix.
    /// Comparison is path-aware: `~` is expanded, trailing `/` is normalized.
    PathPrefix { prefix: PathBuf },
    /// `args.command` (for `exec`) starts with this prefix string.
    /// Used by `exec-approvals` AllowAlways.
    CommandPrefix { prefix: String },
    /// URL host matches this glob (e.g. `*.github.com`). Used by `web_fetch` /
    /// `browser`.
    DomainGlob { glob: String },
    /// Generic substring match against the JSON-stringified args. Use sparingly
    /// — prefer one of the structured variants when possible.
    Substring { needle: String },
}

impl ArgMatcher {
    pub fn matches(&self, tool: &str, args: &serde_json::Value) -> bool {
        match self {
            Self::PathPrefix { prefix } => {
                if let Some(path) = extract_path_arg(tool, args) {
                    path_starts_with(&path, prefix)
                } else {
                    false
                }
            }
            Self::CommandPrefix { prefix } => {
                if let Some(cmd) = extract_command_arg(args) {
                    cmd.trim_start().starts_with(prefix.as_str())
                } else {
                    false
                }
            }
            Self::DomainGlob { glob } => {
                if let Some(host) = extract_domain_arg(args) {
                    domain_glob_matches(glob, &host)
                } else {
                    false
                }
            }
            Self::Substring { needle } => args.to_string().contains(needle),
        }
    }
}

/// Extract the path-like argument for tools that take one. Returns the raw
/// string with `~` expanded (when `expand_tilde` is true). Used by matchers
/// + the protected-paths gate.
pub fn extract_path_arg(tool: &str, args: &serde_json::Value) -> Option<PathBuf> {
    // The tool registry uses `path` for read/write/edit/ls/grep/find and
    // `cwd` for exec / process. `apply_patch` operates on multiple paths
    // embedded in the patch body — we don't currently inspect those at the
    // permission layer (the patch body is opaque text), so apply_patch
    // matches on optional `cwd` only.
    let candidate = match tool {
        "read" | "write" | "edit" | "ls" | "grep" | "find" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str()),
        "exec" | "process" | "apply_patch" => args.get("cwd").and_then(|v| v.as_str()),
        _ => None,
    };
    candidate.map(expand_tilde)
}

/// Extract the `command` argument for `exec` / `process write`.
pub fn extract_command_arg(args: &serde_json::Value) -> Option<String> {
    args.get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract the host (lowercased) from a `url` field. Used by `web_fetch` /
/// `browser` matchers.
pub fn extract_domain_arg(args: &serde_json::Value) -> Option<String> {
    let url_str = args.get("url").and_then(|v| v.as_str())?;
    let parsed = url::Url::parse(url_str).ok()?;
    parsed.host_str().map(|h| h.to_lowercase())
}

/// `~`-expansion wrapper around the canonical [`crate::tools::expand_tilde`].
/// We need a `PathBuf` for matcher comparisons, while the canonical helper
/// returns `String` for tool-arg parsing.
pub fn expand_tilde(s: &str) -> PathBuf {
    PathBuf::from(crate::tools::expand_tilde(s))
}

/// Lexically resolve `..` and `.` segments without touching the filesystem.
/// `Path::canonicalize` requires the target to exist (and resolves symlinks);
/// the protected-path matcher must work on hypothetical paths the LLM hasn't
/// created yet, so we do a pure-syntactic walk instead.
///
/// Behavior:
/// - `.` segments are dropped.
/// - `..` segments pop the previous component when it's a normal name; when
///   the stack is empty (or the only entry is the root prefix), the `..` is
///   kept verbatim so a relative `../foo` doesn't lose information.
/// - Root prefix (`/` on Unix, `C:\` on Windows) is preserved.
///
/// Used by the protected-path scanner so traversal sequences like
/// `~/Documents/../.ssh/id_rsa` collapse to `~/.ssh/id_rsa` *before* the
/// prefix-match runs. Without this step, a `..`-laden literal slips past
/// every directory-prefix pattern in `DEFAULT_PROTECTED_PATHS`.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = match out.components().next_back() {
                    Some(Component::Normal(_)) => out.pop(),
                    _ => false,
                };
                if !popped {
                    out.push("..");
                }
            }
            Component::Normal(name) => out.push(name),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// `true` if `path` starts with `prefix` at a path component boundary.
/// E.g. `/foo/bar` starts with `/foo` but `/foo-bar` does not.
pub fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    let path_s = path.to_string_lossy();
    let prefix_s = prefix.to_string_lossy();
    let path_norm = path_s.trim_end_matches('/');
    let prefix_norm = prefix_s.trim_end_matches('/');
    if path_norm == prefix_norm {
        return true;
    }
    if path_norm.len() > prefix_norm.len()
        && path_norm.starts_with(prefix_norm)
        && path_norm.as_bytes()[prefix_norm.len()] == b'/'
    {
        return true;
    }
    prefix_norm.contains('*') && glob_match_simple(prefix_norm, path_norm)
}

/// Minimal `*`-only glob matcher (no `?`, no character classes). Used for
/// suffix patterns like `*.env` or `*credential*`.
pub fn glob_match_simple(pattern: &str, input: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == input;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !input[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
        } else if i == parts.len() - 1 {
            if !input.ends_with(part) {
                return false;
            }
            // Final part must come after current cursor.
            if input.len() < cursor + part.len() {
                return false;
            }
        } else {
            if let Some(idx) = input[cursor..].find(part) {
                cursor += idx + part.len();
            } else {
                return false;
            }
        }
    }
    true
}

fn domain_glob_matches(pattern: &str, host: &str) -> bool {
    let host_lower = host.to_lowercase();
    let pat_lower = pattern.to_lowercase();
    if let Some(suffix) = pat_lower.strip_prefix("*.") {
        host_lower == suffix || host_lower.ends_with(&format!(".{suffix}"))
    } else {
        host_lower == pat_lower
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_spec_tool_name_extracts_name() {
        let by_name = RuleSpec::Tool {
            name: "write".into(),
        };
        assert_eq!(by_name.tool_name(), "write");

        let by_pattern = RuleSpec::ToolPattern {
            name: "exec".into(),
            matcher: ArgMatcher::CommandPrefix {
                prefix: "git status".into(),
            },
        };
        assert_eq!(by_pattern.tool_name(), "exec");
    }

    #[test]
    fn rule_spec_matches_filters_by_name() {
        let rule = RuleSpec::Tool {
            name: "write".into(),
        };
        let args = serde_json::json!({});
        assert!(rule.matches("write", &args));
        assert!(!rule.matches("read", &args));
    }

    #[test]
    fn permission_rules_is_empty_when_default() {
        let rules = PermissionRules::default();
        assert!(rules.is_empty());
    }

    #[test]
    fn normalize_lex_collapses_dotdot_traversal() {
        // `~/Documents/../.ssh/id_rsa` after expand_tilde collapses to
        // `<home>/.ssh/id_rsa` so the protected-path prefix matcher sees the
        // real target rather than a traversal-laden surface form.
        let raw = std::path::PathBuf::from("/Users/x/Documents/../.ssh/id_rsa");
        let norm = normalize_lexical(&raw);
        assert_eq!(norm, std::path::PathBuf::from("/Users/x/.ssh/id_rsa"));
    }

    #[test]
    fn normalize_lex_drops_curdir_segments() {
        let raw = std::path::PathBuf::from("/a/./b/./c");
        let norm = normalize_lexical(&raw);
        assert_eq!(norm, std::path::PathBuf::from("/a/b/c"));
    }

    #[test]
    fn normalize_lex_keeps_leading_relative_dotdot() {
        // No anchor → `..` stays as data so callers don't lose info.
        let raw = std::path::PathBuf::from("../sneaky");
        let norm = normalize_lexical(&raw);
        assert_eq!(norm, std::path::PathBuf::from("../sneaky"));
    }
}

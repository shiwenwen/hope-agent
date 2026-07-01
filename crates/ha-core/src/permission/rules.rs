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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        self.matches_with_default_path(name, args, None)
    }

    /// Context-aware variant of [`Self::matches`]. Path-aware matchers use
    /// `default_path` to resolve relative tool paths the same way execution
    /// does, so an AllowAlways grant for a workspace also covers future
    /// relative paths inside that workspace.
    pub fn matches_with_default_path(
        &self,
        name: &str,
        args: &serde_json::Value,
        default_path: Option<&Path>,
    ) -> bool {
        if self.tool_name() != name {
            return false;
        }
        match self {
            Self::Tool { .. } => true,
            Self::ToolPattern { matcher, .. } => {
                matcher.matches_with_default_path(name, args, default_path)
            }
        }
    }
}

/// Parameter-level matcher. Each variant knows where in `args` to look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Top-level JSON string field equals this value.
    FieldEquals { field: String, value: String },
    /// All child matchers must match. Used for action/op pairs.
    All { matchers: Vec<ArgMatcher> },
}

impl ArgMatcher {
    pub fn matches(&self, tool: &str, args: &serde_json::Value) -> bool {
        self.matches_with_default_path(tool, args, None)
    }

    pub fn matches_with_default_path(
        &self,
        tool: &str,
        args: &serde_json::Value,
        default_path: Option<&Path>,
    ) -> bool {
        match self {
            Self::PathPrefix { prefix } => {
                let resolved_prefix = normalize_lexical(prefix);
                if tool == crate::tools::TOOL_APPLY_PATCH {
                    let Some(patch) = args.get("input").and_then(|v| v.as_str()) else {
                        return false;
                    };
                    let paths = paths_in_patch_directives(patch);
                    return !paths.is_empty()
                        && paths.iter().all(|path| {
                            let resolved_path = resolve_path_with_default(path, default_path);
                            path_starts_with(&resolved_path, &resolved_prefix)
                        });
                }
                if let Some(path) = extract_path_arg(tool, args) {
                    let resolved_path = resolve_path_with_default(&path, default_path);
                    path_starts_with(&resolved_path, &resolved_prefix)
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
            Self::FieldEquals { field, value } => args
                .get(field)
                .and_then(|v| v.as_str())
                .map(|s| s == value)
                .unwrap_or(false),
            Self::All { matchers } => matchers
                .iter()
                .all(|m| m.matches_with_default_path(tool, args, default_path)),
        }
    }
}

/// Extract the path-like argument for tools that take one. Returns the raw
/// string with `~` expanded (when `expand_tilde` is true). Used by matchers
/// + the protected-paths gate.
pub fn extract_path_arg(tool: &str, args: &serde_json::Value) -> Option<PathBuf> {
    // The tool registry uses `path` for read/write/edit/ls/grep/find/lsp and
    // `cwd` for exec / process. `apply_patch` also has directive paths inside
    // its patch body; those are handled by `paths_in_patch_directives`, while
    // this helper still returns optional `cwd` for generic path checks.
    let candidate = match tool {
        "read" | "write" | "edit" | "ls" | "grep" | "find" | "lsp" => args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str()),
        // Feishu drive tools touch local paths (upload reads from `path`,
        // download writes to `path`). Treat them like read/write at the
        // protected-path layer so `~/.ssh/...` etc. trigger the same gate.
        n if n == crate::tools::feishu::TOOL_DRIVE_UPLOAD_MEDIA
            || n == crate::tools::feishu::TOOL_DRIVE_DOWNLOAD_MEDIA =>
        {
            args.get("path").and_then(|v| v.as_str())
        }
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

/// Resolve a possibly-relative path against the current tool default path,
/// then lexically normalize the result. This mirrors
/// `ToolExecContext::resolve_path` closely enough for permission matching
/// without making the rules module depend on the tools execution context.
pub fn resolve_path_with_default(path: &Path, default_path: Option<&Path>) -> PathBuf {
    let normalized = normalize_lexical(path);
    if normalized.is_absolute() {
        return normalized;
    }
    match default_path {
        Some(base) => {
            normalize_lexical(&expand_tilde(base.to_string_lossy().as_ref()).join(normalized))
        }
        None => normalized,
    }
}

/// The file path(s) a `write` / `edit` / `apply_patch` call will modify — the
/// edit TARGETS, not the `cwd` that [`extract_path_arg`] returns for
/// `apply_patch`. Raw form (tilde-expanded, lexically normalized, possibly
/// relative); resolve against the tool default before comparing. Internal
/// helper for [`resolved_edit_target_paths`].
fn edit_target_paths(tool: &str, args: &serde_json::Value) -> Vec<PathBuf> {
    match tool {
        // Reuse the canonical path-arg extractor so the `path`/`file_path`
        // precedence stays identical to the protected-path gate — otherwise the
        // gate and the Smart session-edit bypass could disagree on which file is
        // being written.
        "write" | "edit" => extract_path_arg(tool, args)
            .map(|p| vec![normalize_lexical(&p)])
            .unwrap_or_default(),
        "apply_patch" => args
            .get("input")
            .and_then(|v| v.as_str())
            .map(paths_in_patch_directives)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// [`edit_target_paths`] resolved to absolute form against `default_path`, so
/// the Smart-mode auto-allow check and the execution-layer session-edit
/// recorder compare exactly the same canonical paths.
pub fn resolved_edit_target_paths(
    tool: &str,
    args: &serde_json::Value,
    default_path: Option<&Path>,
) -> Vec<PathBuf> {
    edit_target_paths(tool, args)
        .iter()
        .map(|p| resolve_path_with_default(p, default_path))
        .collect()
}

/// Pull each `*** Add File: ` / `*** Delete File: ` / `*** Update File: ` /
/// `*** Move to: ` directive target out of an `apply_patch` body.
pub fn paths_in_patch_directives(patch: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in patch.lines() {
        let trimmed = line.trim_start();
        for prefix in [
            "*** Add File: ",
            "*** Delete File: ",
            "*** Update File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = trimmed.strip_prefix(prefix) {
                out.push(normalize_lexical(&expand_tilde(path.trim())));
            }
        }
    }
    out
}

/// Lexically resolve `..` and `.` segments without touching the filesystem.
/// `Path::canonicalize` requires the target to exist (and resolves symlinks);
/// the protected-path matcher must work on hypothetical paths the LLM hasn't
/// created yet, so we do a pure-syntactic walk instead.
///
/// Behavior:
/// - `.` segments are dropped.
/// - `..` segments pop the previous *normal* component; when the stack has
///   no normal entry to pop, the `..` is kept verbatim if the path isn't
///   rooted (so `../foo` doesn't silently lose info), or dropped if it is
///   (you can't escape above the root).
/// - Root prefix (`/` on Unix, `C:\` on Windows) is preserved.
///
/// Used by the protected-path scanner so traversal sequences like
/// `~/Documents/../.ssh/id_rsa` collapse to `~/.ssh/id_rsa` *before* the
/// prefix-match runs. Without this step, a `..`-laden literal slips past
/// every directory-prefix pattern in `DEFAULT_PROTECTED_PATHS`.
///
/// Implementation note: we avoid `PathBuf::push` on Windows because
/// `Component::RootDir.as_os_str()` is `"\"` which `push` treats as an
/// absolute path and *overwrites* any existing prefix — so a naive walk
/// loses the `C:` drive letter and turns `C:\Users\...` into `\Users\...`.
/// Instead, accumulate segments separately and assemble the final path as a
/// single string with the platform's main separator.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::{Component, MAIN_SEPARATOR_STR};
    let mut prefix_str: Option<String> = None;
    let mut has_root = false;
    let mut segments: Vec<String> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => {
                prefix_str = Some(p.as_os_str().to_string_lossy().into_owned());
            }
            Component::RootDir => {
                has_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => match segments.last() {
                Some(last) if last != ".." => {
                    segments.pop();
                }
                _ => {
                    if !has_root {
                        segments.push("..".to_string());
                    }
                }
            },
            Component::Normal(name) => {
                segments.push(name.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = String::new();
    if let Some(ref p) = prefix_str {
        out.push_str(p);
    }
    if has_root {
        out.push_str(MAIN_SEPARATOR_STR);
    }
    out.push_str(&segments.join(MAIN_SEPARATOR_STR));
    if out.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(out)
    }
}

/// `true` if `path` starts with `prefix` at a path component boundary.
/// E.g. `/foo/bar` starts with `/foo` but `/foo-bar` does not. On Windows
/// either `/` or `\` counts as a boundary — Path strings frequently mix the
/// two when a `dirs::home_dir()` `PathBuf` is joined with a forward-slash
/// relative literal.
pub fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    let path_s = path.to_string_lossy();
    let prefix_s = prefix.to_string_lossy();
    let path_norm = path_s.trim_end_matches(['/', '\\']);
    let prefix_norm = prefix_s.trim_end_matches(['/', '\\']);
    if path_norm == prefix_norm {
        return true;
    }
    if path_norm.len() > prefix_norm.len()
        && path_norm.starts_with(prefix_norm)
        && is_path_separator_byte(path_norm.as_bytes()[prefix_norm.len()])
    {
        return true;
    }
    prefix_norm.contains('*') && glob_match_simple(prefix_norm, path_norm)
}

/// `true` if `b` is a path-component separator on the current platform.
/// See `permission::protected_paths::is_path_separator_byte` for the
/// rationale; mirrored here so cross-module consumers don't need to depend
/// on the protected-paths module.
#[inline]
fn is_path_separator_byte(b: u8) -> bool {
    if cfg!(windows) {
        b == b'/' || b == b'\\'
    } else {
        b == b'/'
    }
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

    #[cfg(windows)]
    #[test]
    fn normalize_lex_preserves_drive_prefix_after_dotdot() {
        // Regression: PathBuf::push("\\") on Windows is treated as an
        // absolute push and *overwrites* the existing prefix, so a naive
        // walk turned `C:\Users\u\Documents\..\.ssh\id_rsa` into
        // `\Users\u\.ssh\id_rsa` (drive letter dropped → protected-path
        // prefix matcher silently misses).
        let raw = std::path::PathBuf::from("C:\\Users\\u\\Documents\\..\\.ssh\\id_rsa");
        let norm = normalize_lexical(&raw);
        assert_eq!(norm, std::path::PathBuf::from("C:\\Users\\u\\.ssh\\id_rsa"));
    }
}

//! Matcher compilation and matching (design doc §6).
//!
//! Three syntaxes, discriminated exactly like the official protocol:
//! - `None` / `""` / `"*"` → wildcard (always matches)
//! - contains only `[A-Za-z0-9_|]` → exact string, or `|`-separated list
//! - any other character → treated as a regex
//!
//! A regex that fails to compile becomes [`MatcherKind::Never`] (matches
//! nothing) with a warn, so a typo can never silently match everything.

use regex::Regex;

use super::types::HookEvent;

/// Map a Claude Code-style tool alias to Hope Agent's internal tool name.
/// `Bash`/`Write`/`Edit`/`Read`/`WebFetch` are the upstream-doc names; the
/// dispatcher routes hook input with the internal tool name (`exec`/`write`/
/// `edit`/`read`/`web_fetch`), so a verbatim `matcher: "Bash"` from a Claude
/// Code script would silently miss the literal/pipe branch without this map.
/// Anything not in the alias table passes through unchanged so user-defined
/// or MCP-namespaced names (`mcp__foo__bar`) keep their literal identity.
///
/// Single source of truth: also reused by [`super::condition`] so that a
/// matcher and an `if` rule never disagree about what `Bash` resolves to.
pub(super) fn tool_alias(name: &str) -> &str {
    match name {
        "Bash" | "bash" | "Shell" | "shell" => "exec",
        "Write" => "write",
        "Edit" => "edit",
        "Read" => "read",
        "WebFetch" => "web_fetch",
        other => other,
    }
}

/// Normalize tool aliases inside a literal/pipe matcher string. Each
/// `|`-separated item is mapped via [`tool_alias`]; regex matchers (any char
/// outside `[A-Za-z0-9_|]`) pass through unchanged because alias substitution
/// inside a regex would be hairy and authors of regex matchers are presumed
/// to know the internal names. Returns the normalized string (allocates only
/// when at least one item differs from its input).
fn normalize_tool_aliases(matcher: &str) -> String {
    if !is_literal_or_pipe(matcher) {
        return matcher.to_string();
    }
    matcher
        .split('|')
        .map(tool_alias)
        .collect::<Vec<&str>>()
        .join("|")
}

/// A compiled matcher.
#[derive(Debug)]
pub enum MatcherKind {
    /// Always matches (`None` / `""` / `"*"`).
    Wildcard,
    /// Exact string or `A|B|C` list — matches when the target equals any item.
    ExactOrPipe(Vec<String>),
    /// Regex match against the target.
    Regex(Regex),
    /// A regex that failed to compile — matches nothing.
    Never,
}

/// True when `s` contains only the "literal/pipe-list" character set. Anything
/// else routes the matcher to the regex branch (official rule).
fn is_literal_or_pipe(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '|')
}

/// Compile a matcher for a specific event, normalizing Claude Code tool
/// aliases (`Bash` → `exec`, `Write` → `write`, …) when the event matches on a
/// tool name. Other events (`Notification`, `SessionStart`, `SubagentStart`,
/// …) get the raw matcher unchanged. This is the production entry point used
/// by [`super::registry::HookRegistry::from_config`]; [`compile`] is the
/// alias-agnostic primitive kept around for unit tests and any future caller
/// that doesn't know the event up front.
pub fn compile_for_event(matcher: Option<&str>, event: HookEvent) -> MatcherKind {
    if let Some(raw) = matcher {
        if event.uses_tool_name_matcher() {
            let normalized = normalize_tool_aliases(raw);
            return compile(Some(&normalized));
        }
    }
    compile(matcher)
}

/// Compile a matcher string into a [`MatcherKind`].
pub fn compile(matcher: Option<&str>) -> MatcherKind {
    let raw = match matcher {
        None => return MatcherKind::Wildcard,
        Some(s) => s,
    };
    if raw.is_empty() || raw == "*" {
        return MatcherKind::Wildcard;
    }
    if is_literal_or_pipe(raw) {
        let items: Vec<String> = raw
            .split('|')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        // `"|"` or `"||"` with no real items degrades to wildcard-ish empty —
        // treat as Never so a malformed matcher doesn't match everything.
        if items.is_empty() {
            return MatcherKind::Never;
        }
        return MatcherKind::ExactOrPipe(items);
    }
    // Anchor the regex so `Write` doesn't match `WriteFile` unless the author
    // writes `Write.*`. Official behavior matches the whole tool name; we use
    // `is_match` against the full string but anchor for parity with exact
    // semantics on common patterns like `mcp__.*__write.*`.
    match Regex::new(&format!("^(?:{raw})$")) {
        Ok(re) => MatcherKind::Regex(re),
        Err(e) => {
            app_warn!(
                "hooks",
                "matcher",
                "invalid regex matcher {:?}: {} — will never match",
                raw,
                e
            );
            MatcherKind::Never
        }
    }
}

impl MatcherKind {
    /// Does this matcher fire for the given target?
    ///
    /// `target == None` (events with no matcher target, e.g. `UserPromptSubmit`)
    /// only fires wildcard matchers.
    pub fn is_match(&self, target: Option<&str>) -> bool {
        match self {
            MatcherKind::Wildcard => true,
            MatcherKind::Never => false,
            MatcherKind::ExactOrPipe(items) => match target {
                Some(t) => items.iter().any(|i| i == t),
                None => false,
            },
            MatcherKind::Regex(re) => match target {
                Some(t) => re.is_match(t),
                None => false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, target: &str) -> bool {
        compile(Some(pat)).is_match(Some(target))
    }

    #[test]
    fn exact_does_not_substring_match() {
        assert!(m("Bash", "Bash"));
        assert!(!m("Bash", "Bash2"));
        assert!(!m("Bash", "MyBash"));
    }

    #[test]
    fn pipe_list() {
        assert!(m("Edit|Write", "Edit"));
        assert!(m("Edit|Write", "Write"));
        assert!(!m("Edit|Write", "Read"));
    }

    #[test]
    fn regex_branch_for_mcp_globs() {
        assert!(m("mcp__memory__.*", "mcp__memory__create_entities"));
        assert!(m("mcp__.*__write.*", "mcp__fs__write_file"));
        assert!(!m("mcp__.*__write.*", "mcp__fs__read_file"));
    }

    #[test]
    fn trap_mcp_without_glob_is_exact_and_misses() {
        // §6.3 trap: `mcp__memory` has no `.*` → exact → matches nothing
        // (real tool names are `mcp__memory__<tool>`).
        assert!(!m("mcp__memory", "mcp__memory__create_entities"));
        // It would only match the literal string `mcp__memory`.
        assert!(m("mcp__memory", "mcp__memory"));
    }

    #[test]
    fn wildcard_variants() {
        for pat in [None, Some(""), Some("*")] {
            let k = compile(pat);
            assert!(k.is_match(Some("anything")));
            assert!(k.is_match(None)); // wildcard fires even with no target
        }
    }

    #[test]
    fn non_wildcard_misses_when_target_none() {
        assert!(!compile(Some("Bash")).is_match(None));
        assert!(!compile(Some("a.*b")).is_match(None));
    }

    #[test]
    fn invalid_regex_becomes_never() {
        // Unbalanced bracket → invalid regex → Never.
        let k = compile(Some("[unclosed"));
        assert!(matches!(k, MatcherKind::Never));
        assert!(!k.is_match(Some("[unclosed")));
        assert!(!k.is_match(Some("anything")));
    }

    #[test]
    fn anchored_regex_full_match() {
        // `Write` as regex would be literal-or-pipe (exact), so use a real
        // regex pattern to exercise anchoring.
        assert!(m("Wr.te", "Write"));
        assert!(!m("Wr.te", "Writexx"));
    }

    fn me(pat: &str, target: &str, event: HookEvent) -> bool {
        compile_for_event(Some(pat), event).is_match(Some(target))
    }

    #[test]
    fn tool_aliases_normalize_for_tool_events() {
        // Single literal: `Bash` → `exec`.
        assert!(me("Bash", "exec", HookEvent::PreToolUse));
        // Pipe list: `Write|Edit` → `write|edit`.
        assert!(me("Write|Edit", "write", HookEvent::PreToolUse));
        assert!(me("Write|Edit", "edit", HookEvent::PostToolUse));
        // Hope Agent internal names already work.
        assert!(me("exec", "exec", HookEvent::PreToolUse));
        assert!(me("read", "read", HookEvent::PostToolUseFailure));
        // Lowercase Bash alias too (mirrors `condition.rs::normalize_tool`).
        assert!(me("bash", "exec", HookEvent::PreToolUse));
        // Web fetch alias.
        assert!(me("WebFetch", "web_fetch", HookEvent::PreToolUse));
    }

    #[test]
    fn aliases_dont_match_unrelated_names() {
        // `Write` normalizes to `write`, NOT `Write` — the literal `Write` no
        // longer matches once aliases are folded.
        assert!(!me("Write", "Write", HookEvent::PreToolUse));
        // MCP-namespaced names pass through (no alias collision).
        assert!(me(
            "mcp__memory__create_entities",
            "mcp__memory__create_entities",
            HookEvent::PreToolUse,
        ));
    }

    #[test]
    fn aliases_skipped_for_non_tool_events() {
        // SessionStart / Notification match on `source` / `notification_type`,
        // not tool names — so `Bash` must NOT silently become `exec` there.
        assert!(me("Bash", "Bash", HookEvent::SessionStart));
        assert!(!me("Bash", "exec", HookEvent::SessionStart));
        assert!(me("Bash", "Bash", HookEvent::Notification));
    }

    #[test]
    fn regex_matchers_dont_get_aliased() {
        // `Bash.*` is a regex (contains `.` and `*`), so it stays literal.
        // It would (rightly) miss the internal `exec`, but the failure mode is
        // a regex author's choice rather than a silent alias trap.
        let k = compile_for_event(Some("Bash.*"), HookEvent::PreToolUse);
        assert!(k.is_match(Some("Bashfoo")));
        assert!(!k.is_match(Some("exec")));
    }

    #[test]
    fn wildcard_and_none_unchanged_for_tool_events() {
        for pat in [None, Some(""), Some("*")] {
            let k = compile_for_event(pat, HookEvent::PreToolUse);
            assert!(k.is_match(Some("exec")));
            assert!(k.is_match(Some("Bash")));
        }
    }
}

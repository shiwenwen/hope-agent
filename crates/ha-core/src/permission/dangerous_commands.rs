//! Dangerous-commands list — `exec` command patterns that always require
//! manual approval and CANNOT be AllowAlways'd, even if the user has
//! previously granted the matching command prefix.
//!
//! YOLO modes bypass this list (with `app_warn!` audit log) — the user is
//! explicitly opting into "max permission".
//!
//! Storage: `~/.hope-agent/permission/dangerous-commands.json`.
//! Companion list: [`super::edit_commands`] (broader, AllowAlways'd-able).

/// Default dangerous patterns shipped with Hope Agent. Users can add / remove
/// via the GUI; "Restore defaults" rewrites the on-disk file with this list.
pub const DEFAULT_DANGEROUS_PATTERNS: &[&str] = &[
    // Filesystem destruction
    "rm -rf /",
    "rm -rf ~",
    "rm -rf $HOME",
    "rm -rf /*",
    "sudo rm",
    "sudo dd",
    "chmod 777",
    "chmod -R 777",
    "chmod -R 000",
    // Git irreversible
    "git push --force",
    "git push -f",
    "git push --force-with-lease",
    "git reset --hard",
    "git clean -fd",
    "git clean -fdx",
    // Fork bomb / block devices
    ":(){ :|:& };:",
    "> /dev/sda",
    "> /dev/nvme",
    "dd if=.* of=/dev/",
    "mkfs",
    "fdisk",
    // Database destructive
    "DROP TABLE",
    "DROP DATABASE",
    "TRUNCATE TABLE",
    "DELETE FROM .* WHERE 1",
    // Container destructive
    "docker system prune -a",
    "kubectl delete .* --all",
];

const FILE_NAME: &str = "dangerous-commands.json";

static CACHE: std::sync::LazyLock<super::list_store::Cache> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(None));

/// Currently-active dangerous-command patterns. Backed by
/// `~/.hope-agent/permission/dangerous-commands.json`; falls back to
/// [`DEFAULT_DANGEROUS_PATTERNS`] when the file is missing. Returns an
/// `Arc` snapshot for cheap hot-path access.
pub fn current_patterns() -> std::sync::Arc<Vec<String>> {
    super::list_store::load_or_defaults(&CACHE, FILE_NAME, DEFAULT_DANGEROUS_PATTERNS)
}

pub fn save_patterns(patterns: &[String]) -> anyhow::Result<()> {
    super::list_store::save(&CACHE, FILE_NAME, patterns)
}

pub fn reset_defaults() -> anyhow::Result<Vec<String>> {
    super::list_store::reset_to_defaults(&CACHE, FILE_NAME, DEFAULT_DANGEROUS_PATTERNS)
}

pub fn defaults() -> &'static [&'static str] {
    DEFAULT_DANGEROUS_PATTERNS
}

/// Return the first dangerous pattern matching `command`.
///
/// Patterns are matched case-insensitively. `.*` inside a pattern means
/// "skip any number of characters left-to-right", letting the default list
/// catch shapes like `dd if=/dev/zero of=/dev/sda` via `dd if=.* of=/dev/`
/// without pulling in a full regex engine. Patterns without `.*` fall
/// through to the plain substring matcher so existing entries keep their
/// original semantics.
pub fn matches(command: &str, patterns: &[String]) -> Option<String> {
    for pat in patterns {
        if pattern_matches(command, pat) {
            return Some(pat.clone());
        }
    }
    None
}

fn pattern_matches(command: &str, pattern: &str) -> bool {
    if !pattern.contains(".*") {
        return super::pattern_match::ascii_contains_ignore_case(command, pattern);
    }
    let mut cursor = 0;
    for seg in pattern.split(".*") {
        if seg.is_empty() {
            // Allow leading / trailing `.*` and adjacent `.*.*`.
            continue;
        }
        let haystack = &command[cursor..];
        match super::pattern_match::ascii_position_ignore_case(haystack, seg) {
            Some(pos) => cursor += pos + seg.len(),
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_non_empty() {
        assert!(!DEFAULT_DANGEROUS_PATTERNS.is_empty());
    }

    #[test]
    fn defaults_include_force_push() {
        assert!(DEFAULT_DANGEROUS_PATTERNS.contains(&"git push --force"));
    }

    #[test]
    fn plain_substring_pattern_matches_case_insensitively() {
        let pats = vec!["rm -rf /".to_string()];
        assert_eq!(matches("RM -RF /tmp/foo", &pats), Some("rm -rf /".into()));
        assert_eq!(matches("ls -la", &pats), None);
    }

    #[test]
    fn dot_star_pattern_skips_intermediate_tokens() {
        // Regression for codex-review P1: `.*` was treated as literal
        // characters, so `dd if=/dev/zero of=/dev/sda` slipped past
        // `dd if=.* of=/dev/` and `kubectl delete pods --all` past
        // `kubectl delete .* --all`.
        let pats = vec![
            "dd if=.* of=/dev/".to_string(),
            "kubectl delete .* --all".to_string(),
            "DELETE FROM .* WHERE 1".to_string(),
        ];
        assert_eq!(
            matches("dd if=/dev/zero of=/dev/sda bs=1M", &pats),
            Some("dd if=.* of=/dev/".into())
        );
        assert_eq!(
            matches("kubectl delete pods --all", &pats),
            Some("kubectl delete .* --all".into())
        );
        assert_eq!(
            matches("DELETE FROM users WHERE 1=1", &pats),
            Some("DELETE FROM .* WHERE 1".into())
        );
    }

    #[test]
    fn dot_star_pattern_requires_left_to_right_order() {
        let pats = vec!["foo.*bar".to_string()];
        assert!(matches("xxx foo yyy bar zzz", &pats).is_some());
        assert!(matches("xxx bar yyy foo zzz", &pats).is_none());
    }

    #[test]
    fn defaults_against_known_dangerous_real_commands() {
        let pats: Vec<String> = DEFAULT_DANGEROUS_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Known-bad commands the default list must classify, including the
        // ones the codex review flagged.
        for cmd in [
            "rm -rf /",
            "git push --force origin main",
            "git reset --hard HEAD~1",
            "dd if=/dev/zero of=/dev/sda",
            "kubectl delete pods --all",
            "kubectl delete deployments --all -n staging",
            "DELETE FROM users WHERE 1=1",
            "DROP TABLE accounts",
            "docker system prune -a",
        ] {
            assert!(
                matches(cmd, &pats).is_some(),
                "expected dangerous classification for: {cmd}"
            );
        }
    }
}

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
/// [`DEFAULT_DANGEROUS_PATTERNS`] when the file is missing.
pub fn current_patterns() -> Vec<String> {
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

/// Return the first dangerous pattern that occurs as a case-insensitive
/// substring in `command`. Patterns containing `.*` are treated as plain
/// substrings — we keep behavior predictable rather than compiling regex
/// (the literal "if=.* of=/dev/" still catches the common case).
pub fn matches(command: &str, patterns: &[String]) -> Option<String> {
    for pat in patterns {
        if super::pattern_match::ascii_contains_ignore_case(command, pat) {
            return Some(pat.clone());
        }
    }
    None
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
}

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

/// Currently-active dangerous-command pattern list. The GUI editor will swap
/// this for a `Lazy<RwLock<Vec<String>>>` once user customization lands.
pub fn current_patterns() -> &'static [&'static str] {
    DEFAULT_DANGEROUS_PATTERNS
}

/// Return the first dangerous pattern that occurs as a case-insensitive
/// substring in `command`. Patterns containing `.*` are treated as plain
/// substrings — we keep behavior boring and predictable rather than compiling
/// them as regex (the literal "if=.* of=/dev/" still catches the common case).
pub fn matches(command: &str, patterns: &[&'static str]) -> Option<&'static str> {
    super::pattern_match::first_substring_match_ignore_ascii_case(command, patterns)
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

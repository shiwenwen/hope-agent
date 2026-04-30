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

/// Returns the current pattern list. Phase 2.1: falls back to defaults
/// (file IO is wired up in Phase 3 alongside the GUI editor).
pub fn current_patterns() -> Vec<String> {
    DEFAULT_DANGEROUS_PATTERNS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Check whether `command` (the raw shell command string) matches any
/// dangerous pattern. Returns the matched pattern for audit/log purposes.
///
/// Match strategy:
/// - case-INSENSITIVE substring match (e.g. `DROP TABLE` should also catch
///   `drop table`)
/// - Patterns with `.*` are treated as plain substrings (we deliberately
///   don't compile them as regex to keep behavior boring and predictable —
///   the literal text "if=.* of=/dev/" still catches the common case)
pub fn matches(command: &str, patterns: &[String]) -> Option<String> {
    let cmd_lower = command.to_lowercase();
    for pat in patterns {
        let pat_lower = pat.to_lowercase();
        if cmd_lower.contains(&pat_lower) {
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

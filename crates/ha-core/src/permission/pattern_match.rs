//! Shared pattern-matching helpers for the permission lists.
//!
//! All built-in pattern lists (protected paths / dangerous commands / edit
//! commands) ship as `&'static [&'static str]` and are matched against
//! ASCII-only inputs (file paths, shell commands). The substring matcher
//! below is allocation-free; both `dangerous_commands::matches` and
//! `edit_commands::matches` consume it.

/// First byte index where `needle` occurs as a contiguous substring of
/// `haystack`, ignoring ASCII case. Returns `Some(0)` for empty needles
/// and `None` when there is no match. Used by the dangerous-command
/// matcher to walk multi-segment patterns (`pat1.*pat2`) left-to-right.
pub fn ascii_position_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return None;
    }
    let last = h.len() - n.len();
    'outer: for i in 0..=last {
        for j in 0..n.len() {
            if !h[i + j].eq_ignore_ascii_case(&n[j]) {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

/// `true` if `haystack` contains `needle` as a contiguous substring,
/// ignoring ASCII case. Empty needle always matches.
pub fn ascii_contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.len() > h.len() {
        return false;
    }
    let last = h.len() - n.len();
    'outer: for i in 0..=last {
        for j in 0..n.len() {
            if !h[i + j].eq_ignore_ascii_case(&n[j]) {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_case_insensitive() {
        assert!(ascii_contains_ignore_case("DROP TABLE users", "drop table"));
        assert!(ascii_contains_ignore_case("rm -rf /", "RM -RF"));
        assert!(!ascii_contains_ignore_case("ls -la", "rm "));
    }

    #[test]
    fn empty_needle_matches() {
        assert!(ascii_contains_ignore_case("anything", ""));
    }

    #[test]
    fn needle_longer_than_haystack() {
        assert!(!ascii_contains_ignore_case("ls", "ls -la"));
    }
}

// ── Helper Functions ─────────────────────────────────────────────

/// Get OS version string via `uname -r`.
pub(super) fn os_version() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get machine hostname.
pub(super) fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Walk up from `start` to find the nearest `.git` directory.
pub(super) fn find_git_root(start: &str) -> Option<String> {
    let mut dir = std::path::PathBuf::from(start);
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_string_lossy().to_string());
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Get current date as a stable string (date-only, no time).
/// Excludes time to maximize prompt cache hit rate — the system prompt
/// stays identical throughout the day. Agents can use `exec date` for
/// the precise time when needed.
pub(super) fn current_date() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %Z")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── Truncation ───────────────────────────────────────────────────

/// Truncate text to a maximum length, preserving head (70%) and tail (20%).
pub(super) fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let head_size = max_chars * 70 / 100;
    let tail_size = max_chars * 20 / 100;
    let head = crate::truncate_utf8(text, head_size);
    let tail = crate::truncate_utf8_tail(text, tail_size);
    let omitted = text.len().saturating_sub(head.len() + tail.len());

    format!(
        "{}\n\n[... truncated {} bytes ...]\n\n{}",
        head, omitted, tail
    )
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_keeps_utf8_boundaries_with_tiny_budget() {
        let text = "甲乙丙丁戊己庚辛";
        let truncated = truncate(text, 5);
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
        assert!(truncated.contains("[... truncated"));
    }
}

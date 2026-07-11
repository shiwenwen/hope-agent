// ── Helper Functions ─────────────────────────────────────────────

/// Get OS version string via `uname -r`.
pub(super) fn os_version() -> String {
    // `uname` is normally Unix-only, but Git-for-Windows / MSYS2 ship a
    // `uname.exe` that can land on a Windows PATH — hide the console so it
    // never flashes when it does resolve.
    let mut cmd = std::process::Command::new("uname");
    cmd.arg("-r");
    crate::platform::hide_console(&mut cmd);
    cmd.output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get machine hostname.
pub(super) fn hostname() -> String {
    let mut cmd = std::process::Command::new("hostname");
    crate::platform::hide_console(&mut cmd);
    cmd.output()
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
    // Same as `uname`: a `date.exe` from Git-for-Windows / MSYS2 can resolve
    // on Windows, so suppress its console window too.
    let mut cmd = std::process::Command::new("date");
    cmd.arg("+%Y-%m-%d %Z");
    crate::platform::hide_console(&mut cmd);
    cmd.output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── Truncation ───────────────────────────────────────────────────

/// Truncate text to a maximum length, preserving head (70%) and tail (20%).
pub(super) fn truncate(text: &str, max_chars: usize) -> String {
    let retained_ranges = truncate_retained_ranges(text, max_chars);
    if retained_ranges.len() == 1 {
        return text.to_string();
    }

    let head = &text[retained_ranges[0].0..retained_ranges[0].1];
    let tail = &text[retained_ranges[1].0..retained_ranges[1].1];
    let omitted = text.len().saturating_sub(head.len() + tail.len());

    format!(
        "{}\n\n[... truncated {} bytes ...]\n\n{}",
        head, omitted, tail
    )
}

/// Byte ranges from the original text that [`truncate`] preserves. Consumers
/// that report provenance for truncated prompt content use these exact ranges
/// so they cannot claim omitted text was injected.
pub(super) fn truncate_retained_ranges(text: &str, max_chars: usize) -> Vec<(usize, usize)> {
    if text.len() <= max_chars {
        return vec![(0, text.len())];
    }

    let head = crate::truncate_utf8(text, max_chars * 70 / 100);
    let tail = crate::truncate_utf8_tail(text, max_chars * 20 / 100);
    vec![(0, head.len()), (text.len() - tail.len(), text.len())]
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

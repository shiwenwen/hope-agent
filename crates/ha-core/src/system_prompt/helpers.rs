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

    // Find safe char boundaries
    let head_end = text
        .char_indices()
        .take_while(|(i, _)| *i < head_size)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(head_size);

    let tail_start = text
        .char_indices()
        .rev()
        .take_while(|(i, _)| text.len() - *i <= tail_size)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(text.len() - tail_size);

    format!(
        "{}\n\n[... truncated {} characters ...]\n\n{}",
        &text[..head_end],
        text.len() - head_end - (text.len() - tail_start),
        &text[tail_start..]
    )
}

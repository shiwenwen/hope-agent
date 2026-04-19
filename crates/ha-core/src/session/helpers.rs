use anyhow::Result;
use std::path::PathBuf;

// ── Auto-title helper ────────────────────────────────────────────

/// Generate a short title from the first user message (truncated to 50 chars).
pub fn auto_title(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return "New Chat".to_string();
    }
    // Take first line only
    let first_line = trimmed.lines().next().unwrap_or(trimmed);
    // Use char count (not byte length) to handle CJK/emoji correctly
    if first_line.chars().count() <= 50 {
        first_line.to_string()
    } else {
        // Find the byte offset of the 47th character boundary
        let cut = first_line
            .char_indices()
            .nth(47)
            .map(|(i, _)| i)
            .unwrap_or(first_line.len());
        format!("{}...", &first_line[..cut])
    }
}

// ── Database path helper ─────────────────────────────────────────

/// Get the database file path: ~/.hope-agent/sessions.db
pub fn db_path() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("sessions.db"))
}

use anyhow::Result;
use std::path::PathBuf;

use super::types::SessionMeta;

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

/// Resolve session metadata from the globally-registered SessionDB.
/// Returns `None` when the global DB is not initialized, the session is
/// missing, or the lookup fails.
pub fn lookup_session_meta(session_id: Option<&str>) -> Option<SessionMeta> {
    let sid = session_id?;
    let db = crate::get_session_db()?;
    db.get_session(sid).ok().flatten()
}

/// Whether the given session is running in incognito mode.
pub fn is_session_incognito(session_id: Option<&str>) -> bool {
    lookup_session_meta(session_id)
        .map(|meta| meta.incognito)
        .unwrap_or(false)
}

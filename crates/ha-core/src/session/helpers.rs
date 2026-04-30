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

/// Set the immediate fallback title from the first user-visible message.
/// Returns the title when a write happened.
pub fn ensure_first_message_title(
    db: &super::SessionDB,
    session_id: &str,
    content: &str,
) -> Result<Option<String>> {
    if let Some(meta) = db.get_session(session_id)? {
        if meta.title.is_none() && meta.message_count <= 1 {
            let title = auto_title(content);
            db.update_session_title_with_source(
                session_id,
                &title,
                crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
            )?;
            return Ok(Some(title));
        }
    }
    Ok(None)
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


/// Resolve the effective working directory for a session: session-level value
/// if set, otherwise falling back to the parent project's default. This is the
/// single source of truth consumed by both system-prompt rendering and tool
/// execution context, so the model's view and the tool runtime never disagree
/// (write_file allowlists, exec cwd, file mention, etc.).
pub fn effective_session_working_dir(session_id: Option<&str>) -> Option<String> {
    let meta = lookup_session_meta(session_id)?;
    if let Some(wd) = meta.working_dir.filter(|s| !s.trim().is_empty()) {
        return Some(wd);
    }
    let pid = meta.project_id?;
    crate::get_project_db()?
        .get(&pid)
        .ok()
        .flatten()?
        .working_dir
        .filter(|s| !s.trim().is_empty())
}

// ── Startup recovery ────────────────────────────────────────────

/// Sweep incognito sessions left behind from a previous run (crash, SIGKILL,
/// power loss). Same shape as `subagent::cleanup_orphan_runs` and
/// `team::cleanup::cleanup_orphan_teams` — `app_init` calls all three back to
/// back. Failures are warned, never propagated.
pub fn cleanup_orphan_incognito(session_db: &super::SessionDB) {
    if let Err(e) = session_db.purge_orphan_incognito_sessions() {
        crate::app_warn!(
            "session",
            "purge_orphan_incognito",
            "startup sweep failed: {}",
            e
        );
    }
}

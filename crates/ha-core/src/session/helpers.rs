use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

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

fn user_attachment_entries(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("user_attachments")
            .and_then(Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_else(|| vec![value]),
        _ => Vec::new(),
    }
}

fn safe_pasted_text_first_line(session_id: &str, attachment: &Value) -> Option<String> {
    if attachment.get("source").and_then(Value::as_str)
        != Some(crate::attachments::PASTED_TEXT_SOURCE)
    {
        return None;
    }
    let raw_path = attachment.get("path").and_then(Value::as_str)?;
    let allowed_dir = crate::paths::attachments_dir(session_id)
        .ok()?
        .canonicalize()
        .ok()?;
    let path = Path::new(raw_path).canonicalize().ok()?;
    if !path.starts_with(&allowed_dir) {
        return None;
    }

    let reader = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    for line in reader.lines().take(16) {
        let line = line.ok()?;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Build a non-empty fallback title from the first user-visible input.
///
/// Large pasted text is persisted as an attachment and leaves the message body
/// empty. In that case, prefer the first readable line from the persisted file,
/// then fall back to the attachment name. The persisted path is only read after
/// canonicalizing it beneath this session's attachment directory.
pub fn first_message_title_candidate(
    session_id: &str,
    content: &str,
    attachments_meta: Option<&str>,
) -> Option<String> {
    if !content.trim().is_empty() {
        return Some(auto_title(content));
    }

    let parsed = attachments_meta.and_then(|raw| serde_json::from_str::<Value>(raw).ok())?;
    let attachments = user_attachment_entries(&parsed);
    for attachment in &attachments {
        if let Some(first_line) = safe_pasted_text_first_line(session_id, attachment) {
            return Some(auto_title(&first_line));
        }
    }
    for attachment in attachments {
        let Some(name) = attachment.get("name").and_then(Value::as_str) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let stem = Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(name);
        return Some(auto_title(stem));
    }
    None
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod title_tests {
    use super::first_message_title_candidate;

    #[test]
    fn title_candidate_uses_pasted_text_first_line_before_file_name() {
        let session_id = format!("title-candidate-{}", uuid::Uuid::new_v4());
        let dir = crate::paths::attachments_dir(&session_id).expect("attachment dir");
        std::fs::create_dir_all(&dir).expect("create attachment dir");
        let path = dir.join("long-paste.txt");
        std::fs::write(&path, "\n这是粘贴内容的第一行\n第二行").expect("write pasted text");
        let meta = serde_json::json!([{
            "name": "long-paste.txt",
            "path": path,
            "source": crate::attachments::PASTED_TEXT_SOURCE,
        }])
        .to_string();

        assert_eq!(
            first_message_title_candidate(&session_id, "", Some(&meta)).as_deref(),
            Some("这是粘贴内容的第一行")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn title_candidate_falls_back_to_nested_attachment_name() {
        let meta = serde_json::json!({
            "goal_trigger": true,
            "user_attachments": [{
                "name": "产品调研记录.txt",
                "source": crate::attachments::PASTED_TEXT_SOURCE,
            }]
        })
        .to_string();

        assert_eq!(
            first_message_title_candidate("missing-session", "", Some(&meta)).as_deref(),
            Some("产品调研记录")
        );
        assert_eq!(
            first_message_title_candidate("missing-session", "", None),
            None
        );
    }
}

/// Set the immediate fallback title from the first user-visible message.
/// Returns the title when a write happened.
pub fn ensure_first_message_title(
    db: &super::SessionDB,
    session_id: &str,
    content: &str,
    attachments_meta: Option<&str>,
) -> Result<Option<String>> {
    let should_update = {
        let conn = db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let Some((title, incognito, message_count)) = conn
            .query_row(
                "SELECT s.title, s.incognito,
                        (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS message_count
                   FROM sessions s
                  WHERE s.id = ?1",
                rusqlite::params![session_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        !incognito && title.is_none() && message_count <= 1
    };

    if should_update {
        let Some(title) = first_message_title_candidate(session_id, content, attachments_meta)
        else {
            return Ok(None);
        };
        db.update_session_title_with_source(
            session_id,
            &title,
            crate::session_title::TITLE_SOURCE_FIRST_MESSAGE,
        )?;
        if let Some(bus) = crate::get_event_bus() {
            bus.emit(
                "session:title_updated",
                serde_json::json!({
                    "sessionId": session_id,
                    "title": title,
                }),
            );
        }
        return Ok(Some(title));
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
///
/// **Fail-closed three-state** (Epic E / INCOG-1). A late-arriving operation
/// (memory extraction, large-result disk persistence, async-job spool) must
/// never leave a trace for a session that was burned on close, so the three DB
/// outcomes are deliberately *not* collapsed into one `false` like the generic
/// [`lookup_session_meta`] helper does:
///   - **DB not initialized** (early startup / unit tests) → `false`: no
///     incognito session can exist before the store is up, so this is safe.
///   - **Row genuinely absent** (`Ok(None)`) → `true` (**fail-closed**): a live
///     session always has its row, so an absent row means it was deleted or
///     burned (incognito close physically removes it). Any trailing work must
///     be treated as incognito and skip every persistence sidecar.
///   - **Transient lookup error** (lock contention / IO) → `false` + warn: a
///     momentary glitch must NOT silently drop a *normal* session's memory
///     extraction & persistence. The privacy-critical burn path is additionally
///     guarded by the watcher purge ([`super::cleanup_watcher`]) and the
///     frontend best-effort cancel.
pub fn is_session_incognito(session_id: Option<&str>) -> bool {
    let Some(sid) = session_id else {
        return false;
    };
    let Some(db) = crate::get_session_db() else {
        // DB not initialized — no incognito sessions can exist yet.
        return false;
    };
    match db.get_session(sid) {
        Ok(Some(meta)) => meta.incognito,
        // Row gone (deleted / incognito-burned) — fail closed.
        Ok(None) => true,
        Err(e) => {
            crate::app_warn!(
                "session",
                "is_session_incognito",
                "meta lookup for {} failed, treating as non-incognito: {}",
                sid,
                e
            );
            false
        }
    }
}

/// Resolve the effective working directory for a session: session-level value
/// if set, otherwise the parent project's directory (its explicitly selected
/// `working_dir`, or its lazily-created default workspace). This is the single
/// source of truth consumed by both system-prompt rendering and tool execution
/// context, so the model's view and the tool runtime never disagree (write_file
/// allowlists, exec cwd, file mention, etc.).
///
/// Any session attached to a project resolves to `Some(<existing dir>)`; only
/// sessions with neither a session-level working dir nor a project return
/// `None` (unchanged pre-project behavior).
pub fn effective_session_working_dir(session_id: Option<&str>) -> Option<String> {
    let meta = lookup_session_meta(session_id)?;
    effective_working_dir_for_meta(&meta)
}

/// Same resolution as [`effective_session_working_dir`] but for a caller that
/// already holds the [`SessionMeta`], avoiding a redundant DB lookup.
pub fn effective_working_dir_for_meta(meta: &SessionMeta) -> Option<String> {
    if let Some(wd) = meta.working_dir.clone().filter(|s| !s.trim().is_empty()) {
        return Some(wd);
    }
    let pid = meta.project_id.as_deref()?;
    // An explicit project `working_dir` wins — but a missing project row or a
    // transient DB error must NOT silently drop the session to the agent home
    // (which would scatter the model's relative writes). Fall through to the
    // project's default workspace, which only needs the id.
    if let Some(db) = crate::get_project_db() {
        match db.get(pid) {
            Ok(Some(project)) => {
                if let Some(wd) = project.working_dir.filter(|s| !s.trim().is_empty()) {
                    return Some(wd);
                }
            }
            Ok(None) => {}
            Err(e) => {
                crate::app_warn!(
                    "session",
                    "resolve_working_dir",
                    "project {} lookup failed, falling back to default workspace: {}",
                    pid,
                    e
                );
            }
        }
    }
    // No explicit working dir (or an unreadable row) → lazily materialize the
    // default workspace and use it. Failure degrades to `None` (no working-dir
    // section injected) rather than panicking.
    let ws = crate::paths::project_workspace_dir(pid).ok()?;
    match crate::util::ensure_dir_canonical(&ws) {
        Ok(path) => Some(path),
        Err(e) => {
            crate::app_warn!(
                "session",
                "ensure_workspace",
                "failed to create default workspace for project {}: {}",
                pid,
                e
            );
            None
        }
    }
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

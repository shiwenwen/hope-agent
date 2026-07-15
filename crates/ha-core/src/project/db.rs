//! ProjectDB — persistence layer for the `projects` table.
//!
//! Shares the same SQLite connection pool as [`crate::session::SessionDB`]
//! (the table lives in `sessions.db`), following the same pattern as
//! [`crate::channel::ChannelDB`]. Project files are no longer tracked in the
//! DB — they live directly in the project working directory.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::sync::Arc;

use super::types::{CreateProjectInput, Project, ProjectMeta, UpdateProjectInput};
use crate::session::SessionDB;

/// Project persistence manager. Wraps `Arc<SessionDB>` to reuse its
/// connection.
pub struct ProjectDB {
    session_db: Arc<SessionDB>,
}

impl ProjectDB {
    pub fn new(session_db: Arc<SessionDB>) -> Self {
        Self { session_db }
    }

    /// Run table-creation DDL. Idempotent — safe to call on every boot.
    /// Called once during app startup from `app_init`.
    pub fn migrate(&self) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id                TEXT PRIMARY KEY,
                name              TEXT NOT NULL,
                description       TEXT,
                color             TEXT,
                default_agent_id  TEXT,
                default_model_id  TEXT,
                created_at        INTEGER NOT NULL,
                updated_at        INTEGER NOT NULL,
                archived          INTEGER NOT NULL DEFAULT 0,
                logo              TEXT,
                working_dir       TEXT,
                sort_order        INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_projects_archived
                ON projects(archived, updated_at DESC);",
        )?;

        // Migration: add `logo` column to existing deployments.
        let has_logo = conn.prepare("SELECT logo FROM projects LIMIT 1").is_ok();
        if !has_logo {
            conn.execute_batch("ALTER TABLE projects ADD COLUMN logo TEXT;")?;
        }

        let has_working_dir = conn
            .prepare("SELECT working_dir FROM projects LIMIT 1")
            .is_ok();
        if !has_working_dir {
            conn.execute_batch("ALTER TABLE projects ADD COLUMN working_dir TEXT;")?;
        }

        let has_sort_order = conn
            .prepare("SELECT sort_order FROM projects LIMIT 1")
            .is_ok();
        if !has_sort_order {
            conn.execute_batch(
                "ALTER TABLE projects ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;",
            )?;
            let mut stmt =
                conn.prepare("SELECT id FROM projects ORDER BY updated_at DESC, created_at DESC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            drop(stmt);
            for (idx, id) in ids.into_iter().enumerate() {
                conn.execute(
                    "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                    params![((idx as i64) + 1) * 1024, id],
                )?;
            }
        }

        let has_emoji = conn.prepare("SELECT emoji FROM projects LIMIT 1").is_ok();
        if has_emoji {
            conn.execute_batch("ALTER TABLE projects DROP COLUMN emoji;")?;
        }

        // Project instructions moved to `<project-root>/AGENTS.md`. The old
        // column is deliberately dropped without data migration: the file is
        // now the only source of truth and is created on demand.
        let has_instructions = conn
            .prepare("SELECT instructions FROM projects LIMIT 1")
            .is_ok();
        if has_instructions {
            conn.execute_batch("ALTER TABLE projects DROP COLUMN instructions;")?;
        }

        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_projects_archived_sort
                ON projects(archived, sort_order ASC, updated_at DESC);",
        )?;

        // Migration: drop the legacy bound_channel columns/index on upgrade.
        // Project ↔ Channel reverse-claim is gone; routing is now explicit
        // via /project <id> in IM. SQLite 3.35+ supports DROP COLUMN.
        conn.execute_batch("DROP INDEX IF EXISTS idx_projects_bound_channel;")?;
        let has_bound_channel = conn
            .prepare("SELECT bound_channel_id FROM projects LIMIT 1")
            .is_ok();
        if has_bound_channel {
            conn.execute_batch(
                "ALTER TABLE projects DROP COLUMN bound_channel_id;
                 ALTER TABLE projects DROP COLUMN bound_channel_account_id;",
            )?;
        }

        // Migration: drop the legacy project_files table. Project files now live
        // directly in the project working directory, browsed via the filesystem
        // API. No data migration — the rows were metadata-only references to
        // bytes that the deletion cascade will reclaim with `projects/{id}/`.
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_project_files_project;
             DROP TABLE IF EXISTS project_files;",
        )?;

        // Release the SQLite mutex before resolving project roots (which reads
        // project rows again). Existing projects get the same invariant as new
        // ones: a root AGENTS.md always exists. Failure is non-fatal at startup
        // (the directory may be temporarily unavailable); opening the settings
        // tab or editing the project will retry and surface the concrete error.
        drop(conn);
        for project_id in self.list_all_ids()? {
            if let Err(error) = super::files::ensure_project_instructions(&project_id, self) {
                crate::app_warn!(
                    "project",
                    "agents_md",
                    "could not ensure AGENTS.md for project {}: {}",
                    project_id,
                    error
                );
            }
        }

        Ok(())
    }

    // ── CRUD: projects ──────────────────────────────────────────

    /// Create a new project.
    pub fn create(&self, input: CreateProjectInput) -> Result<Project> {
        let trimmed_name = input.name.trim();
        if trimmed_name.is_empty() {
            anyhow::bail!("project name cannot be empty");
        }
        let name = trimmed_name.to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        let logo = validate_logo(input.logo.as_deref())?;
        let working_dir = crate::util::canonicalize_working_dir(input.working_dir.as_deref())?;

        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let sort_order = next_project_sort_order(&conn)?;

        conn.execute(
            "INSERT INTO projects (id, name, description, color,
                default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                working_dir, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11)",
            params![
                id,
                name,
                normalize_optional(input.description.as_deref()),
                normalize_optional(input.color.as_deref()),
                normalize_optional(input.default_agent_id.as_deref()),
                normalize_optional(input.default_model_id.as_deref()),
                now,
                now,
                logo.as_deref(),
                working_dir.as_deref(),
                sort_order,
            ],
        )?;

        Ok(Project {
            id,
            name,
            description: normalize_optional(input.description.as_deref()).map(str::to_string),
            logo,
            color: normalize_optional(input.color.as_deref()).map(str::to_string),
            default_agent_id: normalize_optional(input.default_agent_id.as_deref())
                .map(str::to_string),
            default_model_id: normalize_optional(input.default_model_id.as_deref())
                .map(str::to_string),
            working_dir,
            created_at: now,
            updated_at: now,
            sort_order,
            archived: false,
        })
    }

    /// Get a single project by id.
    pub fn get(&self, id: &str) -> Result<Option<Project>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, name, description, color,
                        default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                        working_dir, sort_order
                 FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()?;
        Ok(row)
    }

    /// Patch a project. Fields set to `Some(_)` are updated; empty strings
    /// (after trimming) clear the corresponding column to `NULL`.
    pub fn update(&self, id: &str, patch: UpdateProjectInput) -> Result<Project> {
        // Run filesystem-touching validations BEFORE taking the SQLite lock so
        // a slow `canonicalize` can't block other DB ops.
        let validated_working_dir = match patch.working_dir.as_deref() {
            Some(raw) => Some(crate::util::canonicalize_working_dir(Some(raw))?),
            None => None,
        };

        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let now = chrono::Utc::now().timestamp_millis();

        let mut sets: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        fn push_str_field(
            sets: &mut Vec<String>,
            params_vec: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
            col: &str,
            value: &Option<String>,
        ) {
            if let Some(v) = value {
                let idx = params_vec.len() + 1;
                sets.push(format!("{} = ?{}", col, idx));
                let normalized = if v.trim().is_empty() {
                    None
                } else {
                    Some(v.clone())
                };
                params_vec.push(Box::new(normalized));
            }
        }

        if let Some(name) = &patch.name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                anyhow::bail!("project name cannot be empty");
            }
            let idx = params_vec.len() + 1;
            sets.push(format!("name = ?{}", idx));
            params_vec.push(Box::new(trimmed.to_string()));
        }
        push_str_field(
            &mut sets,
            &mut params_vec,
            "description",
            &patch.description,
        );
        // Logo: size-validate before reaching the generic pusher.
        if let Some(raw) = &patch.logo {
            let validated = validate_logo(Some(raw))?;
            let idx = params_vec.len() + 1;
            sets.push(format!("logo = ?{}", idx));
            params_vec.push(Box::new(validated));
        }

        push_str_field(&mut sets, &mut params_vec, "color", &patch.color);
        push_str_field(
            &mut sets,
            &mut params_vec,
            "default_agent_id",
            &patch.default_agent_id,
        );
        push_str_field(
            &mut sets,
            &mut params_vec,
            "default_model_id",
            &patch.default_model_id,
        );

        if let Some(validated) = validated_working_dir {
            let idx = params_vec.len() + 1;
            sets.push(format!("working_dir = ?{}", idx));
            params_vec.push(Box::new(validated));
        }

        if let Some(archived) = patch.archived {
            let idx = params_vec.len() + 1;
            sets.push(format!("archived = ?{}", idx));
            params_vec.push(Box::new(if archived { 1i64 } else { 0i64 }));
        }

        // Always bump updated_at.
        let idx = params_vec.len() + 1;
        sets.push(format!("updated_at = ?{}", idx));
        params_vec.push(Box::new(now));

        let id_idx = params_vec.len() + 1;
        params_vec.push(Box::new(id.to_string()));

        let sql = format!(
            "UPDATE projects SET {} WHERE id = ?{}",
            sets.join(", "),
            id_idx
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, param_refs.as_slice())?;

        // Re-read to return the authoritative current state.
        let project = conn
            .query_row(
                "SELECT id, name, description, color,
                        default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                        working_dir, sort_order
                 FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("project not found after update: {}", id))?;
        Ok(project)
    }

    /// Delete a project. Sessions are **kept** (their `project_id` is cleared);
    /// project-scoped memories are cross-database and wiped by the caller —
    /// see `delete_project_cascade`.
    ///
    /// Both writes happen inside a single transaction so a crash mid-delete
    /// cannot leave a half-deleted project (e.g. sessions unassigned but the
    /// project row still present).
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let tx = conn.transaction()?;

        // Step 1: detach sessions so they survive the project deletion.
        tx.execute(
            "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
            params![id],
        )?;

        // Step 2: delete the project row.
        tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;

        tx.commit()?;
        Ok(())
    }

    /// Lightweight listing of every project id (including archived). Used by
    /// the cross-database memory reconciler at startup, where loading the
    /// full `ProjectMeta` (with aggregate counts, etc.) for every
    /// row would be wasted work.
    pub fn list_all_ids(&self) -> Result<Vec<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT id FROM projects")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Persist the sidebar project order. The input may include a subset of
    /// active project ids; any omitted active projects keep their relative
    /// order and are appended after the supplied sequence.
    pub fn reorder(&self, project_ids: &[String]) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut seen = HashSet::new();
        for id in project_ids {
            if !seen.insert(id.as_str()) {
                anyhow::bail!("duplicate project id in reorder request: {}", id);
            }
        }

        let tx = conn.transaction()?;
        let existing: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM projects
                 WHERE archived = 0
                 ORDER BY sort_order ASC, updated_at DESC, created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        let existing_set: HashSet<&str> = existing.iter().map(String::as_str).collect();
        for id in project_ids {
            if !existing_set.contains(id.as_str()) {
                anyhow::bail!("project not found or archived: {}", id);
            }
        }

        let mut ordered = Vec::with_capacity(existing.len());
        for id in project_ids {
            ordered.push(id.clone());
        }
        for id in existing {
            if !seen.contains(id.as_str()) {
                ordered.push(id);
            }
        }

        for (idx, id) in ordered.iter().enumerate() {
            tx.execute(
                "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                params![((idx as i64) + 1) * 1024, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// List all projects with aggregated counts.
    /// `include_archived = false` hides archived projects.
    ///
    /// `active_session_id` is the session the user is currently viewing (if
    /// any). It is excluded from the unread rollup in SQL so the project badge
    /// matches the per-session "active session reads as 0" rule without the
    /// frontend having to subtract it from a separately-fetched count (which
    /// could transiently disagree and flicker).
    pub fn list(
        &self,
        include_archived: bool,
        active_session_id: Option<&str>,
    ) -> Result<Vec<ProjectMeta>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let where_sql = if include_archived {
            ""
        } else {
            "WHERE p.archived = 0"
        };

        // Empty string when no active session: no real session id is empty, so
        // `s.id != ''` excludes nothing — keeps a single bound-param code path.
        let active = active_session_id.unwrap_or("");

        // Memory count is cross-database and handled separately (filled in
        // later by the caller that has the MemoryBackend in hand). Here we
        // return zero and let the command layer enrich it.
        let sql = format!(
            "SELECT p.id, p.name, p.description, p.color,
                    p.default_agent_id, p.default_model_id, p.created_at, p.updated_at, p.archived,
                    p.logo, p.working_dir, p.sort_order,
                    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id) AS session_count,
                    (SELECT COUNT(*)
                       FROM messages m
                       JOIN sessions s ON s.id = m.session_id
                       LEFT JOIN channel_conversations cc ON cc.session_id = s.id
                      WHERE s.project_id = p.id
                        AND s.parent_session_id IS NULL
                        AND s.is_cron = 0
                        AND cc.session_id IS NULL
                        AND s.id != ?1
                        AND m.id > COALESCE(s.last_read_message_id, 0)
                        AND m.role = 'assistant'
                        AND COALESCE(m.source, 'desktop') != 'channel') AS unread_count
             FROM projects p
             {}
             ORDER BY p.sort_order ASC, p.updated_at DESC",
            where_sql
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![active], |row| {
            let project = row_to_project(row)?;
            Ok(ProjectMeta {
                project,
                session_count: row.get::<_, i64>(12).unwrap_or(0) as u32,
                unread_count: row.get::<_, i64>(13).unwrap_or(0) as u32,
            })
        })?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

// ── Row helpers ─────────────────────────────────────────────────

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        color: row.get(3)?,
        default_agent_id: row.get(4)?,
        default_model_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        archived: row.get::<_, i64>(8).unwrap_or(0) != 0,
        logo: row.get::<_, Option<String>>(9).unwrap_or(None),
        working_dir: row.get::<_, Option<String>>(10).unwrap_or(None),
        sort_order: row.get::<_, i64>(11).unwrap_or(0),
    })
}

fn next_project_sort_order(conn: &rusqlite::Connection) -> Result<i64> {
    let min_order: Option<i64> = conn.query_row(
        "SELECT MIN(sort_order) FROM projects WHERE archived = 0",
        [],
        |row| row.get(0),
    )?;
    Ok(min_order.unwrap_or(2048) - 1024)
}

/// Maximum accepted length of a logo data URL (512 KB). Frontend is expected
/// to downscale images to ~256px before encoding, so real values are ~20 KB.
const MAX_LOGO_BYTES: usize = 512 * 1024;

/// Normalize and validate an incoming logo string.
///
/// Returns `Ok(None)` for empty / whitespace input (clears the column),
/// `Ok(Some(s))` for an accepted `data:image/...` URL, or an error when the
/// payload is too large or not a recognized data URL.
fn validate_logo(raw: Option<&str>) -> Result<Option<String>> {
    let Some(s) = raw else {
        return Ok(None);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_LOGO_BYTES {
        anyhow::bail!(
            "project logo too large: {} bytes (max {})",
            trimmed.len(),
            MAX_LOGO_BYTES
        );
    }
    // Must be a data URL to match the inline-render contract. Anything else
    // (remote http URLs, local file paths) is rejected so we don't ship
    // SSRF-style surprises into the sidebar.
    if !trimmed.starts_with("data:image/") {
        anyhow::bail!("project logo must be a data:image/... URL");
    }
    Ok(Some(trimmed.to_string()))
}

/// Trim whitespace and return `None` for empty strings so we never insert
/// blank strings into optional columns.
fn normalize_optional(value: Option<&str>) -> Option<&str> {
    match value {
        Some(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Regression: legacy installs that still carry `emoji` and
    /// `bound_channel_*` columns must boot cleanly — the migration drops them
    /// and the legacy index.
    #[test]
    fn migrate_drops_legacy_bound_channel_columns() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());

        // Simulate a legacy install: hand-create the projects table with
        // the obsolete bound_channel_* columns + index.
        {
            let conn = session_db.conn.lock().unwrap();
            conn.execute_batch(
                "DROP TABLE IF EXISTS project_files;
                 DROP TABLE IF EXISTS projects;
                 CREATE TABLE projects (
                    id                TEXT PRIMARY KEY,
                    name              TEXT NOT NULL,
                    description       TEXT,
                    instructions      TEXT,
                    emoji             TEXT,
                    color             TEXT,
                    default_agent_id  TEXT,
                    default_model_id  TEXT,
                    created_at        INTEGER NOT NULL,
                    updated_at        INTEGER NOT NULL,
                    archived          INTEGER NOT NULL DEFAULT 0,
                    logo              TEXT,
                    working_dir       TEXT,
                    bound_channel_id         TEXT,
                    bound_channel_account_id TEXT
                 );
                 CREATE INDEX idx_projects_bound_channel
                    ON projects(bound_channel_id, bound_channel_account_id);",
            )
            .unwrap();
        }

        let project_db = ProjectDB::new(session_db.clone());
        project_db
            .migrate()
            .expect("migrate should drop legacy bound_channel columns");

        let conn = session_db.conn.lock().unwrap();
        // Columns are gone.
        assert!(
            conn.prepare("SELECT bound_channel_id FROM projects LIMIT 1")
                .is_err(),
            "bound_channel_id should be dropped"
        );
        assert!(
            conn.prepare("SELECT emoji FROM projects LIMIT 1").is_err(),
            "emoji should be dropped"
        );
        assert!(
            conn.prepare("SELECT instructions FROM projects LIMIT 1")
                .is_err(),
            "legacy instructions should be dropped"
        );
        // Index is gone.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_projects_bound_channel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "idx_projects_bound_channel should be dropped");
    }

    /// Migration must also be idempotent — running it twice (subsequent
    /// app boots) is a no-op.
    #[test]
    fn migrate_idempotent_on_fresh_schema() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());
        let project_db = ProjectDB::new(session_db);
        project_db.migrate().expect("first migrate");
        project_db.migrate().expect("second migrate (idempotent)");
    }

    #[test]
    fn reorder_projects_persists_sidebar_order() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());
        {
            let conn = session_db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL, account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL, thread_id TEXT,
                    session_id TEXT NOT NULL, chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    created_at TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT ''
                );",
            )
            .unwrap();
        }
        let project_db = ProjectDB::new(session_db);
        project_db.migrate().expect("migrate");

        let create = |name: &str| {
            project_db
                .create(CreateProjectInput {
                    name: name.into(),
                    description: None,
                    logo: None,
                    color: None,
                    default_agent_id: None,
                    default_model_id: None,
                    working_dir: None,
                })
                .expect("create project")
        };

        let a = create("A");
        let b = create("B");
        let c = create("C");

        // Partial reorder: omitted active projects keep their existing relative
        // order and are appended after the supplied ids.
        project_db
            .reorder(&[a.id.clone(), c.id.clone()])
            .expect("reorder projects");

        let names: Vec<String> = project_db
            .list(false, None)
            .expect("list projects")
            .into_iter()
            .map(|p| p.project.name)
            .collect();
        assert_eq!(names, vec!["A", "C", "B"]);

        project_db
            .reorder(&[b.id.clone(), a.id.clone(), c.id.clone()])
            .expect("reorder projects again");
        let names: Vec<String> = project_db
            .list(false, None)
            .expect("list projects")
            .into_iter()
            .map(|p| p.project.name)
            .collect();
        assert_eq!(names, vec!["B", "A", "C"]);
    }

    /// §3 regression: a project-bound cron run persists assistant rows with
    /// source="cron" (was "channel"), so the `!= 'channel'` proxy no longer hides
    /// them from the project unread rollup. The fix excludes cron sessions via
    /// `s.is_cron = 0`; otherwise every scheduled run silently bumps the project's
    /// unread badge. Mirrors `session::db`'s per-session regression test.
    #[test]
    fn list_unread_count_excludes_project_cron_sessions() {
        use crate::chat_engine::ChatSource;
        use crate::session::NewMessage;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());
        // The project unread rollup LEFT JOINs channel_conversations; create the
        // (otherwise ChannelDB-owned) table so the query can run.
        {
            let conn = session_db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL, account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL, thread_id TEXT,
                    session_id TEXT NOT NULL, chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    created_at TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT ''
                );",
            )
            .unwrap();
        }
        let project_db = ProjectDB::new(session_db.clone());
        project_db.migrate().expect("migrate");

        let project = project_db
            .create(CreateProjectInput {
                name: "Proj".into(),
                description: None,
                logo: None,
                color: None,
                default_agent_id: None,
                default_model_id: None,
                working_dir: None,
            })
            .expect("create project");

        // Regular project session with an assistant reply → counts as unread.
        let regular = session_db
            .create_session_with_project("ha-main", Some(&project.id), None)
            .expect("regular session");
        session_db
            .append_message(
                &regular.id,
                &NewMessage::assistant("regular reply").with_source(ChatSource::Desktop),
            )
            .expect("append regular");

        // Project-bound cron session output → must NOT count toward unread.
        let cron = session_db
            .create_session_with_project("ha-main", Some(&project.id), None)
            .expect("cron session");
        session_db.mark_session_cron(&cron.id).expect("mark cron");
        session_db
            .append_message(
                &cron.id,
                &NewMessage::assistant("cron output").with_source(ChatSource::Cron),
            )
            .expect("append cron");

        let meta = project_db
            .list(false, None)
            .expect("list projects")
            .into_iter()
            .find(|p| p.project.id == project.id)
            .expect("project present");
        assert_eq!(
            meta.unread_count, 1,
            "project unread counts the regular reply but must exclude cron output"
        );
    }

    /// §3/§4 regression: the project unread rollup must exclude sub-agent child
    /// sessions (`parent_session_id IS NULL`), channel-attached sessions
    /// (`cc.session_id IS NULL`), and the currently-active session
    /// (`s.id != ?active`). The sub-agent and channel sessions use a *desktop*
    /// source so the only thing keeping them out is their dedicated WHERE clause
    /// — not the `!= 'channel'` proxy — pinning down those exact branches, which
    /// previously had no coverage.
    #[test]
    fn list_unread_count_excludes_subagent_channel_and_active() {
        use crate::chat_engine::ChatSource;
        use crate::session::NewMessage;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());
        {
            let conn = session_db.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS channel_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    channel_id TEXT NOT NULL, account_id TEXT NOT NULL,
                    chat_id TEXT NOT NULL, thread_id TEXT,
                    session_id TEXT NOT NULL, chat_type TEXT NOT NULL DEFAULT 'dm',
                    source TEXT NOT NULL DEFAULT 'inbound',
                    created_at TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT ''
                );",
            )
            .unwrap();
        }
        let project_db = ProjectDB::new(session_db.clone());
        project_db.migrate().expect("migrate");

        let project = project_db
            .create(CreateProjectInput {
                name: "Proj".into(),
                description: None,
                logo: None,
                color: None,
                default_agent_id: None,
                default_model_id: None,
                working_dir: None,
            })
            .expect("create project");

        // Regular project session with a desktop assistant reply → counts.
        let regular = session_db
            .create_session_with_project("ha-main", Some(&project.id), None)
            .expect("regular session");
        session_db
            .append_message(
                &regular.id,
                &NewMessage::assistant("regular reply").with_source(ChatSource::Desktop),
            )
            .expect("append regular");

        // Sub-agent child session in the project (desktop source) → excluded ONLY
        // by `parent_session_id IS NULL`.
        let sub = session_db
            .create_session_full("ha-main", Some(&regular.id), Some(&project.id), false)
            .expect("subagent session");
        session_db
            .append_message(
                &sub.id,
                &NewMessage::assistant("subagent reply").with_source(ChatSource::Desktop),
            )
            .expect("append subagent");

        // Channel-attached project session (desktop source) → excluded ONLY by
        // `cc.session_id IS NULL`.
        let channel = session_db
            .create_session_with_project("ha-main", Some(&project.id), None)
            .expect("channel session");
        session_db
            .append_message(
                &channel.id,
                &NewMessage::assistant("im reply").with_source(ChatSource::Desktop),
            )
            .expect("append channel");
        {
            let conn = session_db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO channel_conversations (channel_id, account_id, chat_id, session_id)
                 VALUES ('tg', 'acc', 'chat', ?1)",
                rusqlite::params![channel.id],
            )
            .unwrap();
        }

        let rollup = |active: Option<&str>| {
            project_db
                .list(false, active)
                .expect("list projects")
                .into_iter()
                .find(|p| p.project.id == project.id)
                .expect("project present")
                .unread_count
        };

        assert_eq!(
            rollup(None),
            1,
            "only the regular reply counts; sub-agent + channel sessions excluded"
        );
        assert_eq!(
            rollup(Some(&regular.id)),
            0,
            "viewing the regular session excludes it from the project rollup"
        );
    }
}

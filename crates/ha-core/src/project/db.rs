//! ProjectDB — persistence layer for `projects` and `project_files`.
//!
//! Shares the same SQLite connection pool as [`crate::session::SessionDB`]
//! (both tables live in `sessions.db`), following the same pattern as
//! [`crate::channel::ChannelDB`].

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;

use super::types::{
    BoundChannel, CreateProjectInput, Project, ProjectFile, ProjectMeta, UpdateProjectInput,
};
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
            CREATE INDEX IF NOT EXISTS idx_projects_archived
                ON projects(archived, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_projects_bound_channel
                ON projects(bound_channel_id, bound_channel_account_id);

            CREATE TABLE IF NOT EXISTS project_files (
                id                 TEXT PRIMARY KEY,
                project_id         TEXT NOT NULL,
                name               TEXT NOT NULL,
                original_filename  TEXT NOT NULL,
                mime_type          TEXT,
                size_bytes         INTEGER NOT NULL,
                file_path          TEXT NOT NULL,
                extracted_path     TEXT,
                extracted_chars    INTEGER,
                summary            TEXT,
                created_at         INTEGER NOT NULL,
                updated_at         INTEGER NOT NULL,
                FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_project_files_project
                ON project_files(project_id);",
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

        // Migration: add IM channel binding columns.
        let has_bound_channel = conn
            .prepare("SELECT bound_channel_id FROM projects LIMIT 1")
            .is_ok();
        if !has_bound_channel {
            conn.execute_batch(
                "ALTER TABLE projects ADD COLUMN bound_channel_id TEXT;
                 ALTER TABLE projects ADD COLUMN bound_channel_account_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_projects_bound_channel
                     ON projects(bound_channel_id, bound_channel_account_id);",
            )?;
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

        let bound_channel = input.bound_channel.clone();

        if let Some(bc) = &bound_channel {
            let conflict: Option<String> = conn
                .query_row(
                    "SELECT id FROM projects
                     WHERE bound_channel_id = ?1 AND bound_channel_account_id = ?2 LIMIT 1",
                    params![&bc.channel_id, &bc.account_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(other) = conflict {
                anyhow::bail!(
                    "channel binding already claimed by project {} (channel={}, account={})",
                    other,
                    bc.channel_id,
                    bc.account_id
                );
            }
        }

        conn.execute(
            "INSERT INTO projects (id, name, description, instructions, emoji, color,
                default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                working_dir, bound_channel_id, bound_channel_account_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?13, ?14)",
            params![
                id,
                name,
                normalize_optional(input.description.as_deref()),
                normalize_optional(input.instructions.as_deref()),
                normalize_optional(input.emoji.as_deref()),
                normalize_optional(input.color.as_deref()),
                normalize_optional(input.default_agent_id.as_deref()),
                normalize_optional(input.default_model_id.as_deref()),
                now,
                now,
                logo.as_deref(),
                working_dir.as_deref(),
                bound_channel.as_ref().map(|b| b.channel_id.as_str()),
                bound_channel.as_ref().map(|b| b.account_id.as_str()),
            ],
        )?;

        Ok(Project {
            id,
            name,
            description: normalize_optional(input.description.as_deref()).map(str::to_string),
            instructions: normalize_optional(input.instructions.as_deref()).map(str::to_string),
            emoji: normalize_optional(input.emoji.as_deref()).map(str::to_string),
            logo,
            color: normalize_optional(input.color.as_deref()).map(str::to_string),
            default_agent_id: normalize_optional(input.default_agent_id.as_deref())
                .map(str::to_string),
            default_model_id: normalize_optional(input.default_model_id.as_deref())
                .map(str::to_string),
            working_dir,
            bound_channel,
            created_at: now,
            updated_at: now,
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
                "SELECT id, name, description, instructions, emoji, color,
                        default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                        working_dir, bound_channel_id, bound_channel_account_id
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
        push_str_field(
            &mut sets,
            &mut params_vec,
            "instructions",
            &patch.instructions,
        );
        push_str_field(&mut sets, &mut params_vec, "emoji", &patch.emoji);

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

        // Bound channel patch — `Some(Some(_))` sets, `Some(None)` clears,
        // `None` (field absent) skips. Reject conflicting bindings.
        if let Some(bc_patch) = &patch.bound_channel {
            if let Some(bc) = bc_patch {
                let conflict: Option<String> = conn
                    .query_row(
                        "SELECT id FROM projects
                         WHERE bound_channel_id = ?1
                           AND bound_channel_account_id = ?2
                           AND id != ?3
                         LIMIT 1",
                        params![&bc.channel_id, &bc.account_id, id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if let Some(other) = conflict {
                    anyhow::bail!(
                        "channel binding already claimed by project {} (channel={}, account={})",
                        other,
                        bc.channel_id,
                        bc.account_id
                    );
                }
                let idx_a = params_vec.len() + 1;
                sets.push(format!("bound_channel_id = ?{}", idx_a));
                params_vec.push(Box::new(bc.channel_id.clone()));
                let idx_b = params_vec.len() + 1;
                sets.push(format!("bound_channel_account_id = ?{}", idx_b));
                params_vec.push(Box::new(bc.account_id.clone()));
            } else {
                let idx_a = params_vec.len() + 1;
                sets.push(format!("bound_channel_id = ?{}", idx_a));
                params_vec.push(Box::new(Option::<String>::None));
                let idx_b = params_vec.len() + 1;
                sets.push(format!("bound_channel_account_id = ?{}", idx_b));
                params_vec.push(Box::new(Option::<String>::None));
            }
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
                "SELECT id, name, description, instructions, emoji, color,
                        default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                        working_dir, bound_channel_id, bound_channel_account_id
                 FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("project not found after update: {}", id))?;
        Ok(project)
    }

    /// Delete a project. Sessions are **kept** (their `project_id` is cleared);
    /// project files (and their disk paths, via the returned list) must be
    /// cleaned up by the caller; project-scoped memories are cross-database
    /// and also wiped by the caller — see `delete_project_cascade`.
    ///
    /// All rows inside the session database are touched inside a single
    /// `IMMEDIATE` transaction so a crash mid-delete cannot leave a
    /// half-deleted project (e.g. sessions unassigned but project row still
    /// present).
    pub fn delete(&self, id: &str) -> Result<Vec<ProjectFile>> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let tx = conn.transaction()?;

        // Step 1: snapshot files for the caller (before the cascade drops them).
        let files = {
            let mut stmt = tx.prepare(
                "SELECT id, project_id, name, original_filename, mime_type, size_bytes,
                        file_path, extracted_path, extracted_chars, summary,
                        created_at, updated_at
                 FROM project_files
                 WHERE project_id = ?1
                 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(params![id], row_to_project_file)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };

        // Step 2: detach sessions so they survive the cascade.
        tx.execute(
            "UPDATE sessions SET project_id = NULL WHERE project_id = ?1",
            params![id],
        )?;

        // Step 3: delete the project row. FK ON DELETE CASCADE on project_files
        // drops the file rows automatically (PRAGMA foreign_keys=ON is set at
        // SessionDB::open time).
        tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;

        tx.commit()?;
        Ok(files)
    }

    /// Lightweight listing of every project id (including archived). Used by
    /// the cross-database memory reconciler at startup, where loading the
    /// full `ProjectMeta` (with file counts, instructions, etc.) for every
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

    /// List all projects with aggregated counts.
    /// `include_archived = false` hides archived projects.
    pub fn list(&self, include_archived: bool) -> Result<Vec<ProjectMeta>> {
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

        // Memory count is cross-database and handled separately (filled in
        // later by the caller that has the MemoryBackend in hand). Here we
        // return zero and let the command layer enrich it.
        let sql = format!(
            "SELECT p.id, p.name, p.description, p.instructions, p.emoji, p.color,
                    p.default_agent_id, p.default_model_id, p.created_at, p.updated_at, p.archived,
                    p.logo, p.working_dir, p.bound_channel_id, p.bound_channel_account_id,
                    (SELECT COUNT(*) FROM sessions s WHERE s.project_id = p.id) AS session_count,
                    (SELECT COUNT(*) FROM project_files f WHERE f.project_id = p.id) AS file_count
             FROM projects p
             {}
             ORDER BY p.updated_at DESC",
            where_sql
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let project = row_to_project(row)?;
            Ok(ProjectMeta {
                project,
                session_count: row.get::<_, i64>(15).unwrap_or(0) as u32,
                file_count: row.get::<_, i64>(16).unwrap_or(0) as u32,
                memory_count: 0,
            })
        })?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Find the (single) project bound to the given IM channel account, if any.
    /// Used by the channel worker on `ensure_conversation` to auto-route a
    /// new session into a project.
    pub fn find_by_bound_channel(
        &self,
        channel_id: &str,
        account_id: &str,
    ) -> Result<Option<Project>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, name, description, instructions, emoji, color,
                        default_agent_id, default_model_id, created_at, updated_at, archived, logo,
                        working_dir, bound_channel_id, bound_channel_account_id
                 FROM projects
                 WHERE bound_channel_id = ?1 AND bound_channel_account_id = ?2
                   AND archived = 0
                 LIMIT 1",
                params![channel_id, account_id],
                row_to_project,
            )
            .optional()?;
        Ok(row)
    }

    // ── CRUD: project_files ─────────────────────────────────────

    /// Insert a new project file row. Callers should have already written
    /// the bytes to disk under `paths::project_files_dir(project_id)`.
    pub fn add_file(&self, file: &ProjectFile) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO project_files (id, project_id, name, original_filename, mime_type,
                size_bytes, file_path, extracted_path, extracted_chars, summary,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                file.id,
                file.project_id,
                file.name,
                file.original_filename,
                file.mime_type,
                file.size_bytes,
                file.file_path,
                file.extracted_path,
                file.extracted_chars,
                file.summary,
                file.created_at,
                file.updated_at,
            ],
        )?;
        Ok(())
    }

    /// List all files for a project, newest first.
    pub fn list_files(&self, project_id: &str) -> Result<Vec<ProjectFile>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, original_filename, mime_type, size_bytes,
                    file_path, extracted_path, extracted_chars, summary,
                    created_at, updated_at
             FROM project_files
             WHERE project_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![project_id], row_to_project_file)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Get a single file by (project_id, file_id).
    pub fn get_file(&self, project_id: &str, file_id: &str) -> Result<Option<ProjectFile>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, project_id, name, original_filename, mime_type, size_bytes,
                        file_path, extracted_path, extracted_chars, summary,
                        created_at, updated_at
                 FROM project_files
                 WHERE project_id = ?1 AND id = ?2",
                params![project_id, file_id],
                row_to_project_file,
            )
            .optional()?;
        Ok(row)
    }

    /// Look up a file by its displayed name within a project. Used by the
    /// `project_read_file` tool when the model passes a human-friendly name
    /// instead of a UUID.
    pub fn find_file_by_name(&self, project_id: &str, name: &str) -> Result<Option<ProjectFile>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, project_id, name, original_filename, mime_type, size_bytes,
                        file_path, extracted_path, extracted_chars, summary,
                        created_at, updated_at
                 FROM project_files
                 WHERE project_id = ?1 AND (name = ?2 OR original_filename = ?2)
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![project_id, name],
                row_to_project_file,
            )
            .optional()?;
        Ok(row)
    }

    /// Rename a file's display name.
    pub fn rename_file(&self, file_id: &str, new_name: &str) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "UPDATE project_files SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_name, now, file_id],
        )?;
        Ok(())
    }

    /// Delete a file row and return the previous metadata so the caller can
    /// remove the bytes from disk.
    pub fn delete_file(&self, file_id: &str) -> Result<Option<ProjectFile>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let existing = conn
            .query_row(
                "SELECT id, project_id, name, original_filename, mime_type, size_bytes,
                        file_path, extracted_path, extracted_chars, summary,
                        created_at, updated_at
                 FROM project_files WHERE id = ?1",
                params![file_id],
                row_to_project_file,
            )
            .optional()?;
        if existing.is_some() {
            conn.execute("DELETE FROM project_files WHERE id = ?1", params![file_id])?;
        }
        Ok(existing)
    }
}

// ── Row helpers ─────────────────────────────────────────────────

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<Project> {
    let channel_id: Option<String> = row.get::<_, Option<String>>(13).unwrap_or(None);
    let account_id: Option<String> = row.get::<_, Option<String>>(14).unwrap_or(None);
    let bound_channel = match (channel_id, account_id) {
        (Some(c), Some(a)) if !c.is_empty() && !a.is_empty() => Some(BoundChannel {
            channel_id: c,
            account_id: a,
        }),
        _ => None,
    };
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        instructions: row.get(3)?,
        emoji: row.get(4)?,
        color: row.get(5)?,
        default_agent_id: row.get(6)?,
        default_model_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        archived: row.get::<_, i64>(10).unwrap_or(0) != 0,
        logo: row.get::<_, Option<String>>(11).unwrap_or(None),
        working_dir: row.get::<_, Option<String>>(12).unwrap_or(None),
        bound_channel,
    })
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

fn row_to_project_file(row: &rusqlite::Row) -> rusqlite::Result<ProjectFile> {
    Ok(ProjectFile {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        original_filename: row.get(3)?,
        mime_type: row.get(4)?,
        size_bytes: row.get(5)?,
        file_path: row.get(6)?,
        extracted_path: row.get(7)?,
        extracted_chars: row.get(8)?,
        summary: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

/// Trim whitespace and return `None` for empty strings so we never insert
/// blank strings into optional columns.
fn normalize_optional(value: Option<&str>) -> Option<&str> {
    match value {
        Some(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

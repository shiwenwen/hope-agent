//! KnowledgeRegistry — the `knowledge_bases` + access-binding tables.
//!
//! Truth source (design D9). Wraps `Arc<SessionDB>` so the tables live inside
//! `sessions.db` next to `projects` / `channel_conversations`, sharing the one
//! SQLite connection. `~/.hope-agent/knowledge/index.db` holds only the
//! rebuildable note/chunk/link cache — never the registry.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;
use std::sync::Arc;

use super::types::{
    CreateKnowledgeBaseInput, GraphNodePosition, KbAccess, KnowledgeBase, KnowledgeBaseMeta,
    UpdateKnowledgeBaseInput,
};
use crate::session::SessionDB;

/// Knowledge base persistence manager. Wraps `Arc<SessionDB>` to reuse its
/// connection (the tables live in `sessions.db`).
pub struct KnowledgeRegistry {
    session_db: Arc<SessionDB>,
}

impl KnowledgeRegistry {
    pub fn new(session_db: Arc<SessionDB>) -> Self {
        Self { session_db }
    }

    /// Idempotent DDL. Called once at startup from `app_init`.
    pub fn migrate(&self) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        // FK targets resolve because SessionDB::open set `PRAGMA foreign_keys=ON`.
        // ON DELETE CASCADE on the join tables auto-cleans attach rows when a
        // session / project / KB is deleted; `delete()` also clears them
        // explicitly inside one transaction as a double safeguard.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge_bases (
                id                   TEXT PRIMARY KEY,
                name                 TEXT NOT NULL,
                emoji                TEXT,
                root_dir             TEXT,
                allow_external_writes INTEGER NOT NULL DEFAULT 0,
                archived             INTEGER NOT NULL DEFAULT 0,
                created_at           INTEGER NOT NULL,
                updated_at           INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_bases_archived
                ON knowledge_bases(archived, updated_at DESC);

            CREATE TABLE IF NOT EXISTS session_knowledge_bases (
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                kb_id      TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                access     TEXT NOT NULL DEFAULT 'read' CHECK (access IN ('read','write')),
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_id, kb_id)
            );
            CREATE INDEX IF NOT EXISTS idx_session_kb_kb
                ON session_knowledge_bases(kb_id);

            CREATE TABLE IF NOT EXISTS project_knowledge_bases (
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                kb_id      TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                access     TEXT NOT NULL DEFAULT 'read' CHECK (access IN ('read','write')),
                created_at INTEGER NOT NULL,
                PRIMARY KEY (project_id, kb_id)
            );
            CREATE INDEX IF NOT EXISTS idx_project_kb_kb
                ON project_knowledge_bases(kb_id);

            -- Layer-2 autonomous-maintenance proposal queue (WS6). Truth source in
            -- sessions.db so it survives index rebuilds; cascades on KB delete.
            CREATE TABLE IF NOT EXISTS kb_maintenance_proposals (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                kb_id       TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                kind        TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'draft',
                title       TEXT NOT NULL,
                detail      TEXT NOT NULL DEFAULT '',
                action_json TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                decided_at  INTEGER,
                error       TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_kb_maint_kb_status
                ON kb_maintenance_proposals(kb_id, status, created_at DESC);
            -- Dedup across ALL statuses: an applied OR dismissed suggestion is not
            -- re-queued (a rejected one stays rejected, respecting the user's call;
            -- an applied one is already done). Pruning old decided rows eventually
            -- frees a fingerprint if the situation recurs much later.
            CREATE UNIQUE INDEX IF NOT EXISTS uq_kb_maint_fingerprint
                ON kb_maintenance_proposals(kb_id, fingerprint);

            -- User-pinned graph-view node positions (Batch J). Keyed by rel_path
            -- (stable across index rebuilds), persisted here (truth source D9) so
            -- the canvas layout survives an index.db wipe; cascades on KB delete.
            CREATE TABLE IF NOT EXISTS kb_graph_layout (
                kb_id    TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                rel_path TEXT NOT NULL,
                x        REAL NOT NULL,
                y        REAL NOT NULL,
                PRIMARY KEY (kb_id, rel_path)
            );

            -- Knowledge-space sidebar conversations (Phase: KB chat panel). Binds
            -- a `kind='knowledge'` session to its KB + the note it was anchored to
            -- at creation (used to default-load \"the latest conversation about this
            -- note\"). Truth source in sessions.db; cascades on session OR KB delete.
            -- The conversation messages themselves live in the shared `messages`
            -- table like any session.
            CREATE TABLE IF NOT EXISTS knowledge_chat_threads (
                session_id       TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                kb_id            TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                anchor_note_path TEXT,
                created_at       INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kb_chat_threads_kb
                ON knowledge_chat_threads(kb_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_kb_chat_threads_note
                ON knowledge_chat_threads(kb_id, anchor_note_path);",
        )?;

        // Additive column for branch DBs created before WS7 (external-writable
        // opt-in). Probe-then-ALTER, the house style — fresh DBs already have it
        // from CREATE TABLE above.
        let has_allow_external_writes = conn
            .prepare("SELECT allow_external_writes FROM knowledge_bases LIMIT 1")
            .is_ok();
        if !has_allow_external_writes {
            conn.execute_batch(
                "ALTER TABLE knowledge_bases
                 ADD COLUMN allow_external_writes INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        Ok(())
    }

    // ── CRUD: knowledge_bases ──────────────────────────────────

    pub fn create(&self, input: CreateKnowledgeBaseInput) -> Result<KnowledgeBase> {
        let trimmed_name = input.name.trim();
        if trimmed_name.is_empty() {
            anyhow::bail!("knowledge base name cannot be empty");
        }
        let name = trimmed_name.to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        // Validate an external root if given: must exist and be a directory so a
        // bind never silently points at a missing/file path.
        let root_dir = match normalize_optional(input.root_dir.as_deref()) {
            Some(raw) => {
                let p = PathBuf::from(raw);
                let canon = p.canonicalize().map_err(|e| {
                    anyhow::anyhow!("cannot resolve external root '{}': {}", raw, e)
                })?;
                if !canon.is_dir() {
                    anyhow::bail!("external root is not a directory: {}", canon.display());
                }
                Some(canon.to_string_lossy().to_string())
            }
            None => None,
        };

        let emoji = normalize_optional(input.emoji.as_deref()).map(str::to_string);

        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        conn.execute(
            "INSERT INTO knowledge_bases
                (id, name, emoji, root_dir, allow_external_writes, archived, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?6)",
            params![id, name, emoji, root_dir, now, now],
        )?;

        Ok(KnowledgeBase {
            id,
            name,
            emoji,
            root_dir,
            allow_external_writes: false,
            archived: false,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get(&self, id: &str) -> Result<Option<KnowledgeBase>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, name, emoji, root_dir, archived, created_at, updated_at, allow_external_writes
                 FROM knowledge_bases WHERE id = ?1",
                params![id],
                row_to_kb,
            )
            .optional()?;
        Ok(row)
    }

    pub fn update(&self, id: &str, patch: UpdateKnowledgeBaseInput) -> Result<KnowledgeBase> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let now = chrono::Utc::now().timestamp_millis();
        let mut sets: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(name) = &patch.name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                anyhow::bail!("knowledge base name cannot be empty");
            }
            let idx = params_vec.len() + 1;
            sets.push(format!("name = ?{}", idx));
            params_vec.push(Box::new(trimmed.to_string()));
        }
        if let Some(emoji) = &patch.emoji {
            let idx = params_vec.len() + 1;
            sets.push(format!("emoji = ?{}", idx));
            let normalized = if emoji.trim().is_empty() {
                None
            } else {
                Some(emoji.clone())
            };
            params_vec.push(Box::new(normalized));
        }
        if let Some(archived) = patch.archived {
            let idx = params_vec.len() + 1;
            sets.push(format!("archived = ?{}", idx));
            params_vec.push(Box::new(if archived { 1i64 } else { 0i64 }));
        }
        if let Some(allow) = patch.allow_external_writes {
            let idx = params_vec.len() + 1;
            sets.push(format!("allow_external_writes = ?{}", idx));
            params_vec.push(Box::new(if allow { 1i64 } else { 0i64 }));
        }

        let idx = params_vec.len() + 1;
        sets.push(format!("updated_at = ?{}", idx));
        params_vec.push(Box::new(now));

        let id_idx = params_vec.len() + 1;
        params_vec.push(Box::new(id.to_string()));

        let sql = format!(
            "UPDATE knowledge_bases SET {} WHERE id = ?{}",
            sets.join(", "),
            id_idx
        );
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, param_refs.as_slice())?;

        let kb = conn
            .query_row(
                "SELECT id, name, emoji, root_dir, archived, created_at, updated_at, allow_external_writes
                 FROM knowledge_bases WHERE id = ?1",
                params![id],
                row_to_kb,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("knowledge base not found after update: {}", id))?;
        Ok(kb)
    }

    /// Delete a KB row + its attach rows inside a single transaction. The
    /// on-disk index rows + internal notes directory are cross-store and handled
    /// by [`super::delete_kb_cascade`].
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM session_knowledge_bases WHERE kb_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM project_knowledge_bases WHERE kb_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM knowledge_bases WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Every KB id (including archived). Used by the index reconciler.
    pub fn list_all_ids(&self) -> Result<Vec<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare("SELECT id FROM knowledge_bases")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List KBs. `note_count` is filled with 0 here (the index lives in a
    /// separate DB) — the command layer enriches it from the index backend,
    /// mirroring `ProjectMeta::memory_count`.
    pub fn list(&self, include_archived: bool) -> Result<Vec<KnowledgeBaseMeta>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let where_sql = if include_archived {
            ""
        } else {
            "WHERE archived = 0"
        };
        let sql = format!(
            "SELECT id, name, emoji, root_dir, archived, created_at, updated_at, allow_external_writes
             FROM knowledge_bases {} ORDER BY updated_at DESC",
            where_sql
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let kb = row_to_kb(row)?;
            let external = kb
                .root_dir
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            Ok(KnowledgeBaseMeta {
                kb,
                note_count: 0,
                external,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Access bindings ────────────────────────────────────────

    pub fn attach_session(&self, session_id: &str, kb_id: &str, access: KbAccess) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO session_knowledge_bases (session_id, kb_id, access, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, kb_id) DO UPDATE SET access = excluded.access",
            params![session_id, kb_id, access.as_str(), now],
        )?;
        Ok(())
    }

    pub fn detach_session(&self, session_id: &str, kb_id: &str) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM session_knowledge_bases WHERE session_id = ?1 AND kb_id = ?2",
            params![session_id, kb_id],
        )?;
        Ok(())
    }

    pub fn attach_project(&self, project_id: &str, kb_id: &str, access: KbAccess) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO project_knowledge_bases (project_id, kb_id, access, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id, kb_id) DO UPDATE SET access = excluded.access",
            params![project_id, kb_id, access.as_str(), now],
        )?;
        Ok(())
    }

    pub fn detach_project(&self, project_id: &str, kb_id: &str) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "DELETE FROM project_knowledge_bases WHERE project_id = ?1 AND kb_id = ?2",
            params![project_id, kb_id],
        )?;
        Ok(())
    }

    /// `(kb_id, access)` rows explicitly attached to a session.
    pub fn list_session_attachments(&self, session_id: &str) -> Result<Vec<(String, KbAccess)>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT kb_id, access FROM session_knowledge_bases WHERE session_id = ?1")?;
        let rows = stmt.query_map(params![session_id], |row| {
            let kb_id: String = row.get(0)?;
            let access: String = row.get(1)?;
            Ok((kb_id, KbAccess::from_str_lenient(&access)))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// `(kb_id, access)` rows attached to a project.
    pub fn list_project_attachments(&self, project_id: &str) -> Result<Vec<(String, KbAccess)>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare("SELECT kb_id, access FROM project_knowledge_bases WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![project_id], |row| {
            let kb_id: String = row.get(0)?;
            let access: String = row.get(1)?;
            Ok((kb_id, KbAccess::from_str_lenient(&access)))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Maintenance proposal queue (WS6) ────────────────────────

    /// Insert a freshly generated proposal as a `draft`. Returns the new row id, or
    /// `None` if a same-fingerprint row already exists in a *sticky* status (draft /
    /// applied / rejected) — the unique `(kb_id, fingerprint)` index dedups it.
    /// A `failed` row is first deleted so a transient apply failure can be retried
    /// next cycle (Failed is not a permanent dismissal).
    pub fn insert_proposal(
        &self,
        kb_id: &str,
        p: &super::maintenance::NewProposal,
    ) -> Result<Option<i64>> {
        let action_json = serde_json::to_string(&p.action)?;
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        // Free a previously-failed fingerprint so it can be re-proposed/retried.
        conn.execute(
            "DELETE FROM kb_maintenance_proposals
             WHERE kb_id = ?1 AND fingerprint = ?2 AND status = 'failed'",
            params![kb_id, p.fingerprint],
        )?;
        let affected = conn.execute(
            "INSERT OR IGNORE INTO kb_maintenance_proposals
                (kb_id, kind, status, title, detail, action_json, fingerprint, created_at)
             VALUES (?1, ?2, 'draft', ?3, ?4, ?5, ?6, ?7)",
            params![
                kb_id,
                p.kind.as_str(),
                p.title,
                p.detail,
                action_json,
                p.fingerprint,
                now
            ],
        )?;
        Ok(if affected == 0 {
            None
        } else {
            Some(conn.last_insert_rowid())
        })
    }

    /// List proposals for a KB (newest first), optionally filtered by status.
    pub fn list_proposals(
        &self,
        kb_id: &str,
        status: Option<super::maintenance::ProposalStatus>,
    ) -> Result<Vec<super::maintenance::MaintenanceProposal>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut out = Vec::new();
        if let Some(st) = status {
            let mut stmt = conn.prepare(
                "SELECT id, kb_id, kind, status, title, detail, action_json, fingerprint,
                        created_at, decided_at, error
                 FROM kb_maintenance_proposals WHERE kb_id = ?1 AND status = ?2
                 ORDER BY created_at DESC, id DESC",
            )?;
            let rows = stmt.query_map(params![kb_id, st.as_str()], row_to_proposal)?;
            for r in rows {
                if let Some(p) = r? {
                    out.push(p);
                }
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, kb_id, kind, status, title, detail, action_json, fingerprint,
                        created_at, decided_at, error
                 FROM kb_maintenance_proposals WHERE kb_id = ?1
                 ORDER BY created_at DESC, id DESC",
            )?;
            let rows = stmt.query_map(params![kb_id], row_to_proposal)?;
            for r in rows {
                if let Some(p) = r? {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    /// Fetch one proposal by id.
    pub fn get_proposal(&self, id: i64) -> Result<Option<super::maintenance::MaintenanceProposal>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row = conn
            .query_row(
                "SELECT id, kb_id, kind, status, title, detail, action_json, fingerprint,
                        created_at, decided_at, error
                 FROM kb_maintenance_proposals WHERE id = ?1",
                params![id],
                row_to_proposal,
            )
            .optional()?;
        Ok(row.flatten())
    }

    /// Transition a proposal's status, stamping `decided_at` and optional `error`.
    pub fn set_proposal_status(
        &self,
        id: i64,
        status: super::maintenance::ProposalStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE kb_maintenance_proposals SET status = ?2, decided_at = ?3, error = ?4
             WHERE id = ?1",
            params![id, status.as_str(), now, error],
        )?;
        Ok(())
    }

    /// Count pending (draft) proposals for a KB — drives the review-queue badge.
    pub fn count_pending_proposals(&self, kb_id: &str) -> Result<usize> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM kb_maintenance_proposals WHERE kb_id = ?1 AND status = 'draft'",
            params![kb_id],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as usize)
    }

    /// Delete *decided* (applied / rejected / failed) proposals older than
    /// `cutoff_ms`. Pending drafts are **never** pruned — they stay in the review
    /// queue until the owner acts on them. Returns rows removed.
    pub fn prune_proposals(&self, cutoff_ms: i64) -> Result<usize> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let n = conn.execute(
            "DELETE FROM kb_maintenance_proposals
             WHERE status != 'draft' AND decided_at IS NOT NULL AND decided_at < ?1",
            params![cutoff_ms],
        )?;
        Ok(n)
    }

    // ── Graph layout (Batch J) ─────────────────────────────────

    /// Read all pinned node positions for a KB.
    pub fn get_graph_layout(&self, kb_id: &str) -> Result<Vec<GraphNodePosition>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT rel_path, x, y FROM kb_graph_layout WHERE kb_id = ?1 ORDER BY rel_path",
        )?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok(GraphNodePosition {
                rel_path: r.get(0)?,
                x: r.get(1)?,
                y: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Replace the full pinned-position set for a KB in one transaction (the set
    /// of nodes the user has dragged to fix). An empty slice clears the layout
    /// (the "reset layout" action). Idempotent.
    pub fn save_graph_layout(&self, kb_id: &str, positions: &[GraphNodePosition]) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM kb_graph_layout WHERE kb_id = ?1",
            params![kb_id],
        )?;
        for p in positions {
            tx.execute(
                "INSERT OR REPLACE INTO kb_graph_layout(kb_id, rel_path, x, y)
                 VALUES (?1, ?2, ?3, ?4)",
                params![kb_id, p.rel_path, p.x, p.y],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Knowledge-space chat threads ───────────────────────────────

    /// Record a `kind='knowledge'` session as a KB chat thread anchored to a
    /// note. Idempotent on `session_id` (re-anchoring keeps the first row).
    pub fn create_chat_thread(
        &self,
        session_id: &str,
        kb_id: &str,
        anchor_note_path: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO knowledge_chat_threads (session_id, kb_id, anchor_note_path, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO NOTHING",
            params![session_id, kb_id, anchor_note_path, now],
        )?;
        Ok(())
    }

    /// Most-recently-active chat thread session anchored to a given note within
    /// a KB (default-load target). `None` when the note has no prior thread.
    pub fn latest_thread_session_for_note(
        &self,
        kb_id: &str,
        anchor_note_path: &str,
    ) -> Result<Option<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let sid: Option<String> = conn
            .query_row(
                "SELECT t.session_id
                 FROM knowledge_chat_threads t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE t.kb_id = ?1 AND t.anchor_note_path = ?2
                 ORDER BY s.updated_at DESC
                 LIMIT 1",
                params![kb_id, anchor_note_path],
                |r| r.get(0),
            )
            .optional()?;
        Ok(sid)
    }

    /// All chat threads in a KB, newest-active first, joined with session
    /// metadata for the history picker. `query` (when non-empty) filters to
    /// threads whose messages match an FTS search.
    pub fn list_chat_threads(
        &self,
        kb_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<super::types::KbChatThread>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        // Optional FTS pre-filter: collect matching session ids first so the
        // join below can restrict to them. Reuses the messages_fts index.
        let fts_filter: Option<std::collections::HashSet<String>> = match query {
            Some(q) if !crate::session::db::sanitize_fts_query(q).is_empty() => {
                let sanitized = crate::session::db::sanitize_fts_query(q);
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT m.session_id
                     FROM messages_fts fts
                     JOIN messages m ON m.id = fts.rowid
                     JOIN knowledge_chat_threads t ON t.session_id = m.session_id
                     WHERE t.kb_id = ?1 AND messages_fts MATCH ?2",
                )?;
                let rows = stmt.query_map(params![kb_id, sanitized], |r| r.get::<_, String>(0))?;
                let mut set = std::collections::HashSet::new();
                for r in rows {
                    set.insert(r?);
                }
                Some(set)
            }
            _ => None,
        };

        let mut stmt = conn.prepare(
            "SELECT t.session_id, t.kb_id, t.anchor_note_path, t.created_at,
                    s.title, s.updated_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = t.session_id) AS msg_count,
                    (SELECT m.content FROM messages m
                       WHERE m.session_id = t.session_id
                         AND m.role IN ('user','assistant') AND length(m.content) > 0
                       ORDER BY m.id DESC LIMIT 1) AS last_snippet
             FROM knowledge_chat_threads t
             JOIN sessions s ON s.id = t.session_id
             WHERE t.kb_id = ?1
             ORDER BY s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![kb_id], |r| {
            Ok(super::types::KbChatThread {
                session_id: r.get(0)?,
                kb_id: r.get(1)?,
                anchor_note_path: r.get(2)?,
                created_at: r.get(3)?,
                title: r.get(4)?,
                updated_at: r.get(5)?,
                message_count: r.get(6)?,
                last_snippet: r.get::<_, Option<String>>(7)?.map(|s| {
                    let trimmed = s.trim();
                    crate::truncate_utf8(trimmed, 160).to_string()
                }),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            let thread = r?;
            if let Some(ref set) = fts_filter {
                if !set.contains(&thread.session_id) {
                    continue;
                }
            }
            out.push(thread);
        }
        Ok(out)
    }
}

// ── Row helpers ─────────────────────────────────────────────────

fn row_to_proposal(
    row: &rusqlite::Row,
) -> rusqlite::Result<Option<super::maintenance::MaintenanceProposal>> {
    use super::maintenance::{MaintenanceProposal, ProposalKind, ProposalStatus};
    let kind_s: String = row.get(2)?;
    let status_s: String = row.get(3)?;
    let action_s: String = row.get(6)?;
    // Skip rows with an unknown kind/status or unparseable action (forward-compat
    // / corruption) rather than failing the whole query.
    let (Some(kind), Some(status)) = (
        ProposalKind::from_str(&kind_s),
        ProposalStatus::from_str(&status_s),
    ) else {
        return Ok(None);
    };
    let Ok(action) = serde_json::from_str(&action_s) else {
        return Ok(None);
    };
    Ok(Some(MaintenanceProposal {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        kind,
        status,
        title: row.get(4)?,
        detail: row.get(5)?,
        action,
        fingerprint: row.get(7)?,
        created_at: row.get(8)?,
        decided_at: row.get(9)?,
        error: row.get(10)?,
    }))
}

fn row_to_kb(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeBase> {
    Ok(KnowledgeBase {
        id: row.get(0)?,
        name: row.get(1)?,
        emoji: row.get::<_, Option<String>>(2).unwrap_or(None),
        root_dir: row.get::<_, Option<String>>(3).unwrap_or(None),
        archived: row.get::<_, i64>(4).unwrap_or(0) != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        allow_external_writes: row.get::<_, i64>(7).unwrap_or(0) != 0,
    })
}

fn normalize_optional(value: Option<&str>) -> Option<&str> {
    match value {
        Some(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// A KB's resolved storage root plus its write posture (WS7).
///
/// `is_external` and `read_only` are deliberately distinct: an external root can
/// be opted into writes (`read_only = false`), but background autonomous
/// maintenance still keys off `is_external` to skip every bound vault regardless.
/// Internal roots are always `is_external = false`, `read_only = false`.
pub struct KbRoot {
    /// Canonical notes directory.
    pub dir: PathBuf,
    /// Bound to an out-of-app directory (vault).
    pub is_external: bool,
    /// Mutations rejected: external AND not opted into external writes.
    pub read_only: bool,
}

/// Resolve a KB's notes directory + write posture.
///
/// Internal KBs (NULL `root_dir`) materialize the default
/// `~/.hope-agent/knowledge/{id}/notes/` lazily (mirrors project workspace), so
/// the path is never written into the DB and `HA_DATA_DIR` stays relocatable.
/// External KBs return their bound path as-is (canonicalized at create time);
/// they are read-only unless `allow_external_writes` is set (WS7).
pub fn resolve_kb_dir(kb_id: &str) -> Result<KbRoot> {
    let db =
        crate::get_knowledge_db().ok_or_else(|| anyhow::anyhow!("knowledge db not initialized"))?;
    let kb = db
        .get(kb_id)?
        .ok_or_else(|| anyhow::anyhow!("knowledge base not found: {}", kb_id))?;
    if kb.is_external() {
        // `is_external()` already checks root_dir is non-empty.
        let root = kb.root_dir.clone().unwrap_or_default();
        Ok(KbRoot {
            dir: PathBuf::from(root),
            is_external: true,
            read_only: !kb.allow_external_writes,
        })
    } else {
        let dir = crate::paths::knowledge_kb_notes_dir(kb_id)?;
        let path = crate::util::ensure_dir_canonical(&dir)?;
        Ok(KbRoot {
            dir: PathBuf::from(path),
            is_external: false,
            read_only: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn registry() -> (tempfile::TempDir, KnowledgeRegistry) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions.db");
        let session_db = Arc::new(SessionDB::open(&db_path).unwrap());
        // `project_knowledge_bases` FKs `projects` — created by ProjectDB::migrate,
        // which runs before the registry in production (app_init).
        crate::project::ProjectDB::new(session_db.clone())
            .migrate()
            .unwrap();
        let reg = KnowledgeRegistry::new(session_db);
        reg.migrate().unwrap();
        (dir, reg)
    }

    #[test]
    fn create_get_update_delete_roundtrip() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "  Work  ".into(),
                emoji: Some("📚".into()),
                root_dir: None,
            })
            .unwrap();
        assert_eq!(kb.name, "Work");
        assert!(!kb.is_external());

        let fetched = reg.get(&kb.id).unwrap().unwrap();
        assert_eq!(fetched.emoji.as_deref(), Some("📚"));

        let updated = reg
            .update(
                &kb.id,
                UpdateKnowledgeBaseInput {
                    name: Some("Personal".into()),
                    archived: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Personal");
        assert!(updated.archived);

        assert_eq!(reg.list(false).unwrap().len(), 0); // archived hidden
        assert_eq!(reg.list(true).unwrap().len(), 1);

        reg.delete(&kb.id).unwrap();
        assert!(reg.get(&kb.id).unwrap().is_none());
    }

    #[test]
    fn empty_name_rejected() {
        let (_d, reg) = registry();
        assert!(reg
            .create(CreateKnowledgeBaseInput {
                name: "   ".into(),
                emoji: None,
                root_dir: None,
            })
            .is_err());
    }

    #[test]
    fn external_writes_opt_in_roundtrip() {
        let (_d, reg) = registry();
        let vault = tempdir().unwrap();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Vault".into(),
                emoji: None,
                root_dir: Some(vault.path().to_string_lossy().to_string()),
            })
            .unwrap();
        assert!(kb.is_external());
        // WS7: an external root is read-only until explicitly unlocked.
        assert!(!kb.allow_external_writes);
        assert!(kb.is_read_only_root());

        let updated = reg
            .update(
                &kb.id,
                UpdateKnowledgeBaseInput {
                    allow_external_writes: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(updated.allow_external_writes);
        assert!(!updated.is_read_only_root());

        // Survives a re-fetch (column persisted, not just on the returned struct).
        let fetched = reg.get(&kb.id).unwrap().unwrap();
        assert!(fetched.allow_external_writes);

        // Re-locking restores read-only.
        let relocked = reg
            .update(
                &kb.id,
                UpdateKnowledgeBaseInput {
                    allow_external_writes: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(relocked.is_read_only_root());
    }

    #[test]
    fn internal_kb_never_read_only_root() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Internal".into(),
                emoji: None,
                root_dir: None,
            })
            .unwrap();
        assert!(!kb.is_external());
        assert!(!kb.is_read_only_root());
    }

    #[test]
    fn graph_layout_save_get_reset_and_cascade() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Graph".into(),
                emoji: None,
                root_dir: None,
            })
            .unwrap();
        assert!(reg.get_graph_layout(&kb.id).unwrap().is_empty());

        let pins = vec![
            GraphNodePosition {
                rel_path: "a.md".into(),
                x: 1.5,
                y: -2.0,
            },
            GraphNodePosition {
                rel_path: "b/c.md".into(),
                x: 10.0,
                y: 20.0,
            },
        ];
        reg.save_graph_layout(&kb.id, &pins).unwrap();
        let got = reg.get_graph_layout(&kb.id).unwrap();
        assert_eq!(got.len(), 2);
        // Ordered by rel_path; f64 round-trips exactly.
        assert_eq!(got[0].rel_path, "a.md");
        assert_eq!((got[0].x, got[0].y), (1.5, -2.0));

        // Save-all replaces (drops a.md, keeps b/c.md, moves it).
        reg.save_graph_layout(
            &kb.id,
            &[GraphNodePosition {
                rel_path: "b/c.md".into(),
                x: 99.0,
                y: 99.0,
            }],
        )
        .unwrap();
        let got = reg.get_graph_layout(&kb.id).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].rel_path, "b/c.md");
        assert_eq!(got[0].x, 99.0);

        // Empty = reset.
        reg.save_graph_layout(&kb.id, &[]).unwrap();
        assert!(reg.get_graph_layout(&kb.id).unwrap().is_empty());

        // Rows cascade away with the KB.
        reg.save_graph_layout(&kb.id, &pins).unwrap();
        reg.delete(&kb.id).unwrap();
        assert!(reg.get_graph_layout(&kb.id).unwrap().is_empty());
    }
}

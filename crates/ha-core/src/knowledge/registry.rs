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
    CompileProposal, CompileProposalAction, CompileProposalKind, CompileProposalStatus, CompileRun,
    CompileRunStatus, CreateKnowledgeBaseInput, GraphNodePosition, KbAccess, KnowledgeBase,
    KnowledgeBaseMeta, KnowledgeSource, KnowledgeSourceChunk, KnowledgeSourceKind,
    KnowledgeSourceStatus, NewCompileProposal, SchemaProfile, UpdateKnowledgeBaseInput,
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

            -- Knowledge Compiler Phase 3: per-KB schema profile truth source.
            -- The default profile is inserted on create and lazily backfilled for
            -- existing KBs when read.
            CREATE TABLE IF NOT EXISTS knowledge_schema_profiles (
                kb_id        TEXT PRIMARY KEY REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                profile_json TEXT NOT NULL,
                updated_at   INTEGER NOT NULL
            );

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
                ON knowledge_chat_threads(kb_id, anchor_note_path);

            -- Knowledge Compiler Phase 1: raw-source inbox. Source metadata is
            -- truth-source state (sessions.db); the stored snapshot file lives
            -- under ~/.hope-agent/knowledge/{kb}/sources/. Raw source chunks are
            -- deliberately separate from note_chunk so compiled-note search is
            -- never polluted by uncompiled source material.
            CREATE TABLE IF NOT EXISTS knowledge_sources (
                id                  TEXT PRIMARY KEY,
                kb_id               TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                kind                TEXT NOT NULL,
                title               TEXT NOT NULL,
                origin_uri          TEXT,
                stored_path         TEXT NOT NULL,
                content_hash        TEXT NOT NULL,
                extracted_text_hash TEXT,
                status              TEXT NOT NULL DEFAULT 'ready',
                compiled_at         INTEGER,
                created_at          INTEGER NOT NULL,
                updated_at          INTEGER NOT NULL,
                size                INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_kb
                ON knowledge_sources(kb_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_hash
                ON knowledge_sources(kb_id, content_hash);

            CREATE TABLE IF NOT EXISTS knowledge_source_chunks (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id    TEXT NOT NULL REFERENCES knowledge_sources(id) ON DELETE CASCADE,
                chunk_index  INTEGER NOT NULL,
                body         TEXT NOT NULL,
                start_offset INTEGER NOT NULL,
                end_offset   INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                UNIQUE(source_id, chunk_index)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_chunks_source
                ON knowledge_source_chunks(source_id, chunk_index);

            -- Knowledge Compiler Phase 2: owner-reviewed compile runs and
            -- proposals. Runs are truth-source state. Proposals are durable
            -- Review Diff drafts; applying one is the only path that mutates
            -- real notes.
            CREATE TABLE IF NOT EXISTS knowledge_compile_runs (
                id              TEXT PRIMARY KEY,
                kb_id           TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                status          TEXT NOT NULL,
                source_ids_json TEXT NOT NULL,
                strategy        TEXT NOT NULL,
                model_label     TEXT,
                fingerprint     TEXT NOT NULL,
                error           TEXT,
                summary         TEXT,
                proposal_count  INTEGER NOT NULL DEFAULT 0,
                created_at      INTEGER NOT NULL,
                started_at      INTEGER,
                finished_at     INTEGER,
                updated_at      INTEGER NOT NULL,
                UNIQUE(kb_id, fingerprint)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_compile_runs_kb
                ON knowledge_compile_runs(kb_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_compile_runs_status
                ON knowledge_compile_runs(kb_id, status, created_at DESC);

            CREATE TABLE IF NOT EXISTS knowledge_compile_proposals (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id          TEXT NOT NULL REFERENCES knowledge_compile_runs(id) ON DELETE CASCADE,
                kb_id           TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                kind            TEXT NOT NULL,
                status          TEXT NOT NULL DEFAULT 'draft',
                title           TEXT NOT NULL,
                detail          TEXT NOT NULL,
                action_json     TEXT NOT NULL,
                fingerprint     TEXT NOT NULL,
                source_ids_json TEXT NOT NULL,
                before_text     TEXT,
                after_text      TEXT,
                created_at      INTEGER NOT NULL,
                decided_at      INTEGER,
                error           TEXT,
                UNIQUE(kb_id, fingerprint)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_compile_proposals_run
                ON knowledge_compile_proposals(run_id, status, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_compile_proposals_kb
                ON knowledge_compile_proposals(kb_id, status, created_at DESC);",
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
        let profile_json = serde_json::to_string(&SchemaProfile::default_for(&id, now))?;
        conn.execute(
            "INSERT OR REPLACE INTO knowledge_schema_profiles (kb_id, profile_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![id, profile_json, now],
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
        tx.execute(
            "DELETE FROM knowledge_sources WHERE kb_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM knowledge_compile_runs WHERE kb_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM knowledge_compile_proposals WHERE kb_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM knowledge_schema_profiles WHERE kb_id = ?1",
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

    // ── Schema profiles (Knowledge Compiler Phase 3) ─────────────

    pub fn get_schema_profile(&self, kb_id: &str) -> Result<Option<SchemaProfile>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT profile_json, updated_at
                 FROM knowledge_schema_profiles WHERE kb_id = ?1",
                params![kb_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((json, updated_at)) = row else {
            return Ok(None);
        };
        let mut profile = serde_json::from_str::<SchemaProfile>(&json)
            .unwrap_or_else(|_| SchemaProfile::default_for(kb_id, updated_at));
        profile.kb_id = kb_id.to_string();
        profile.updated_at = updated_at;
        Ok(Some(profile))
    }

    pub fn upsert_schema_profile(&self, profile: &SchemaProfile) -> Result<()> {
        let json = serde_json::to_string(profile)?;
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO knowledge_schema_profiles (kb_id, profile_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![profile.kb_id, json, profile.updated_at],
        )?;
        Ok(())
    }

    // ── Raw source inbox (Knowledge Compiler Phase 1) ─────────────

    pub fn insert_source(
        &self,
        source: &KnowledgeSource,
        chunks: &[KnowledgeSourceChunk],
    ) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO knowledge_sources
                (id, kb_id, kind, title, origin_uri, stored_path, content_hash,
                 extracted_text_hash, status, compiled_at, created_at, updated_at, size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                source.id,
                source.kb_id,
                source.kind.as_str(),
                source.title,
                source.origin_uri,
                source.stored_path,
                source.content_hash,
                source.extracted_text_hash,
                source.status.as_str(),
                source.compiled_at,
                source.created_at,
                source.updated_at,
                source.size,
            ],
        )?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO knowledge_source_chunks
                    (source_id, chunk_index, body, start_offset, end_offset, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    source.id,
                    chunk.chunk_index,
                    chunk.body,
                    chunk.start_offset,
                    chunk.end_offset,
                    chunk.content_hash,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_sources(&self, kb_id: &str) -> Result<Vec<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1
             GROUP BY s.id
             ORDER BY s.created_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_source)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_source(&self, kb_id: &str, source_id: &str) -> Result<Option<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1 AND s.id = ?2
             GROUP BY s.id",
            params![kb_id, source_id],
            row_to_source,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_source(&self, kb_id: &str, source_id: &str) -> Result<Option<String>> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let stored_path: Option<String> = tx
            .query_row(
                "SELECT stored_path FROM knowledge_sources WHERE kb_id = ?1 AND id = ?2",
                params![kb_id, source_id],
                |r| r.get(0),
            )
            .optional()?;
        if stored_path.is_some() {
            tx.execute(
                "DELETE FROM knowledge_sources WHERE kb_id = ?1 AND id = ?2",
                params![kb_id, source_id],
            )?;
        }
        tx.commit()?;
        Ok(stored_path)
    }

    pub fn replace_source_chunks(
        &self,
        kb_id: &str,
        source_id: &str,
        content_hash: &str,
        extracted_text_hash: Option<&str>,
        size: i64,
        chunks: &[KnowledgeSourceChunk],
    ) -> Result<Option<KnowledgeSource>> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp_millis();
        let affected = tx.execute(
            "UPDATE knowledge_sources
             SET content_hash = ?3,
                 extracted_text_hash = ?4,
                 status = 'ready',
                 updated_at = ?5,
                 size = ?6
             WHERE kb_id = ?1 AND id = ?2",
            params![
                kb_id,
                source_id,
                content_hash,
                extracted_text_hash,
                now,
                size,
            ],
        )?;
        if affected == 0 {
            tx.commit()?;
            return Ok(None);
        }
        tx.execute(
            "DELETE FROM knowledge_source_chunks WHERE source_id = ?1",
            params![source_id],
        )?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO knowledge_source_chunks
                    (source_id, chunk_index, body, start_offset, end_offset, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    source_id,
                    chunk.chunk_index,
                    chunk.body,
                    chunk.start_offset,
                    chunk.end_offset,
                    chunk.content_hash,
                ],
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.get_source(kb_id, source_id)
    }

    pub fn mark_sources_compiled(&self, kb_id: &str, source_ids: &[String]) -> Result<()> {
        if source_ids.is_empty() {
            return Ok(());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        for source_id in source_ids {
            conn.execute(
                "UPDATE knowledge_sources SET compiled_at = ?3, updated_at = ?3
                 WHERE kb_id = ?1 AND id = ?2",
                params![kb_id, source_id, now],
            )?;
        }
        Ok(())
    }

    // ── Knowledge Compiler runs/proposals (Phase 2) ─────────────

    pub fn begin_compile_run(
        &self,
        kb_id: &str,
        source_ids: &[String],
        strategy: &str,
        fingerprint: &str,
    ) -> Result<(CompileRun, bool)> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let existing: Option<CompileRun> = tx
            .query_row(
                "SELECT id, kb_id, status, source_ids_json, strategy, model_label,
                        fingerprint, error, summary, proposal_count, created_at,
                        started_at, finished_at, updated_at
                 FROM knowledge_compile_runs
                 WHERE kb_id = ?1 AND fingerprint = ?2",
                params![kb_id, fingerprint],
                row_to_compile_run,
            )
            .optional()?;
        let now = chrono::Utc::now().timestamp_millis();
        let source_ids_json = serde_json::to_string(source_ids)?;
        if let Some(run) = existing {
            if matches!(
                run.status,
                CompileRunStatus::Running | CompileRunStatus::Completed
            ) {
                tx.commit()?;
                return Ok((run, false));
            }
            tx.execute(
                "DELETE FROM knowledge_compile_proposals WHERE run_id = ?1",
                params![run.id],
            )?;
            tx.execute(
                "UPDATE knowledge_compile_runs
                 SET status='running', source_ids_json=?2, strategy=?3,
                     model_label=NULL, error=NULL, summary=NULL, proposal_count=0,
                     started_at=?4, finished_at=NULL, updated_at=?4
                 WHERE id=?1",
                params![run.id, source_ids_json, strategy, now],
            )?;
            tx.commit()?;
            drop(conn);
            return self
                .get_compile_run(&run.id)?
                .map(|r| (r, true))
                .ok_or_else(|| anyhow::anyhow!("compile run vanished after reset"));
        }

        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO knowledge_compile_runs
                (id, kb_id, status, source_ids_json, strategy, fingerprint,
                 proposal_count, created_at, started_at, updated_at)
             VALUES (?1, ?2, 'running', ?3, ?4, ?5, 0, ?6, ?6, ?6)",
            params![id, kb_id, source_ids_json, strategy, fingerprint, now],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_compile_run(&id)?
            .map(|r| (r, true))
            .ok_or_else(|| anyhow::anyhow!("compile run vanished after insert"))
    }

    pub fn get_compile_run(&self, run_id: &str) -> Result<Option<CompileRun>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, kb_id, status, source_ids_json, strategy, model_label,
                    fingerprint, error, summary, proposal_count, created_at,
                    started_at, finished_at, updated_at
             FROM knowledge_compile_runs WHERE id = ?1",
            params![run_id],
            row_to_compile_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_compile_runs(&self, kb_id: &str) -> Result<Vec<CompileRun>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, kb_id, status, source_ids_json, strategy, model_label,
                    fingerprint, error, summary, proposal_count, created_at,
                    started_at, finished_at, updated_at
             FROM knowledge_compile_runs
             WHERE kb_id = ?1
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_compile_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn finish_compile_run(
        &self,
        run_id: &str,
        status: CompileRunStatus,
        summary: Option<&str>,
        error: Option<&str>,
        proposal_count: u32,
        model_label: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_compile_runs
             SET status=?2, summary=?3, error=?4, proposal_count=?5,
                 model_label=COALESCE(?6, model_label),
                 finished_at=?7, updated_at=?7
             WHERE id=?1",
            params![
                run_id,
                status.as_str(),
                summary,
                error,
                proposal_count,
                model_label,
                now
            ],
        )?;
        Ok(())
    }

    pub fn cancel_compile_run(&self, run_id: &str) -> Result<Option<CompileRun>> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_compile_runs
             SET status='cancelled', error=NULL, finished_at=?2, updated_at=?2
             WHERE id=?1 AND status='running'",
            params![run_id, now],
        )?;
        drop(conn);
        self.get_compile_run(run_id)
    }

    pub fn insert_compile_proposals(
        &self,
        run_id: &str,
        kb_id: &str,
        proposals: &[NewCompileProposal],
    ) -> Result<usize> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut inserted = 0usize;
        for p in proposals {
            tx.execute(
                "DELETE FROM knowledge_compile_proposals
                 WHERE kb_id=?1 AND fingerprint=?2 AND status='failed'",
                params![kb_id, p.fingerprint],
            )?;
            let action_json = serde_json::to_string(&p.action)?;
            let source_ids_json = serde_json::to_string(&p.source_ids)?;
            let affected = tx.execute(
                "INSERT OR IGNORE INTO knowledge_compile_proposals
                    (run_id, kb_id, kind, status, title, detail, action_json,
                     fingerprint, source_ids_json, before_text, after_text, created_at)
                 VALUES (?1, ?2, ?3, 'draft', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    run_id,
                    kb_id,
                    p.kind.as_str(),
                    p.title,
                    p.detail,
                    action_json,
                    p.fingerprint,
                    source_ids_json,
                    p.before_text,
                    p.after_text,
                    now,
                ],
            )?;
            inserted += affected;
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn list_compile_proposals(
        &self,
        kb_id: &str,
        run_id: Option<&str>,
        status: Option<CompileProposalStatus>,
    ) -> Result<Vec<CompileProposal>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let base = "SELECT id, run_id, kb_id, kind, status, title, detail,
                          action_json, fingerprint, source_ids_json, created_at,
                          decided_at, error, before_text, after_text
                   FROM knowledge_compile_proposals";
        let mut out = Vec::new();
        match (run_id, status) {
            (Some(run), Some(st)) => {
                let mut stmt = conn.prepare(&format!(
                    "{base} WHERE kb_id=?1 AND run_id=?2 AND status=?3 ORDER BY created_at DESC, id DESC"
                ))?;
                let rows =
                    stmt.query_map(params![kb_id, run, st.as_str()], row_to_compile_proposal)?;
                for r in rows {
                    out.push(r?);
                }
            }
            (Some(run), None) => {
                let mut stmt = conn.prepare(&format!(
                    "{base} WHERE kb_id=?1 AND run_id=?2 ORDER BY created_at DESC, id DESC"
                ))?;
                let rows = stmt.query_map(params![kb_id, run], row_to_compile_proposal)?;
                for r in rows {
                    out.push(r?);
                }
            }
            (None, Some(st)) => {
                let mut stmt = conn.prepare(&format!(
                    "{base} WHERE kb_id=?1 AND status=?2 ORDER BY created_at DESC, id DESC"
                ))?;
                let rows = stmt.query_map(params![kb_id, st.as_str()], row_to_compile_proposal)?;
                for r in rows {
                    out.push(r?);
                }
            }
            (None, None) => {
                let mut stmt = conn.prepare(&format!(
                    "{base} WHERE kb_id=?1 ORDER BY created_at DESC, id DESC"
                ))?;
                let rows = stmt.query_map(params![kb_id], row_to_compile_proposal)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    pub fn get_compile_proposal(&self, id: i64) -> Result<Option<CompileProposal>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            "SELECT id, run_id, kb_id, kind, status, title, detail,
                    action_json, fingerprint, source_ids_json, created_at,
                    decided_at, error, before_text, after_text
             FROM knowledge_compile_proposals WHERE id = ?1",
            params![id],
            row_to_compile_proposal,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_compile_proposal_status(
        &self,
        id: i64,
        status: CompileProposalStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_compile_proposals
             SET status=?2,
                 decided_at=CASE WHEN ?2 = 'draft' THEN NULL ELSE ?3 END,
                 error=?4
             WHERE id=?1",
            params![id, status.as_str(), now, error],
        )?;
        Ok(())
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

    /// A page of chat threads in a KB, newest-active first, joined with session
    /// metadata for the history picker. `query` (when non-empty) restricts to
    /// threads whose messages match an FTS search. `limit` (default 50, clamped
    /// to 1..=200) + `offset` paginate; the FTS filter is pushed into SQL as an
    /// `IN` subquery so `LIMIT` applies to the *matched* set, not a pre-slice.
    pub fn list_chat_threads(
        &self,
        kb_id: &str,
        query: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<super::types::KbChatThread>> {
        // Nested (env-free) row mapper shared by both query branches.
        fn map_row(r: &rusqlite::Row) -> rusqlite::Result<super::types::KbChatThread> {
            Ok(super::types::KbChatThread {
                session_id: r.get(0)?,
                kb_id: r.get(1)?,
                anchor_note_path: r.get(2)?,
                created_at: r.get(3)?,
                title: r.get(4)?,
                updated_at: r.get(5)?,
                agent_id: r.get(6)?,
                message_count: r.get(7)?,
                last_snippet: r.get::<_, Option<String>>(8)?.map(|s| {
                    let trimmed = s.trim();
                    crate::truncate_utf8(trimmed, 160).to_string()
                }),
            })
        }
        const SELECT: &str = "t.session_id, t.kb_id, t.anchor_note_path, t.created_at,
                    s.title, s.updated_at, s.agent_id,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = t.session_id) AS msg_count,
                    (SELECT m.content FROM messages m
                       WHERE m.session_id = t.session_id
                         AND m.role IN ('user','assistant') AND length(m.content) > 0
                       ORDER BY m.id DESC LIMIT 1) AS last_snippet";

        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let limit = limit.unwrap_or(50).clamp(1, 200);
        let offset = offset.unwrap_or(0).max(0);

        let sanitized = match query {
            Some(q) => {
                let s = crate::session::db::sanitize_fts_query(q);
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            }
            None => None,
        };

        let out = if let Some(q) = sanitized {
            let sql = format!(
                "SELECT {SELECT}
                 FROM knowledge_chat_threads t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE t.kb_id = ?1
                   AND t.session_id IN (
                       SELECT DISTINCT m.session_id FROM messages_fts fts
                       JOIN messages m ON m.id = fts.rowid
                       JOIN knowledge_chat_threads kt ON kt.session_id = m.session_id
                       WHERE kt.kb_id = ?1 AND messages_fts MATCH ?2)
                 ORDER BY s.updated_at DESC
                 LIMIT ?3 OFFSET ?4"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![kb_id, q, limit, offset], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let sql = format!(
                "SELECT {SELECT}
                 FROM knowledge_chat_threads t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE t.kb_id = ?1
                 ORDER BY s.updated_at DESC
                 LIMIT ?2 OFFSET ?3"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![kb_id, limit, offset], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(out)
    }

    /// Session ids of every knowledge chat thread bound to `kb_id`. Used by
    /// `delete_kb_cascade` to tear down the (otherwise hidden) `kind=knowledge`
    /// sessions before the KB row + thread rows are removed — collect first, the
    /// thread rows cascade away with the KB.
    pub fn chat_thread_session_ids(&self, kb_id: &str) -> Result<Vec<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt =
            conn.prepare("SELECT session_id FROM knowledge_chat_threads WHERE kb_id = ?1")?;
        let rows = stmt.query_map(params![kb_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
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

fn row_to_compile_run(row: &rusqlite::Row) -> rusqlite::Result<CompileRun> {
    let status_s: String = row.get(2)?;
    let source_ids_s: String = row.get(3)?;
    let source_ids = serde_json::from_str::<Vec<String>>(&source_ids_s).unwrap_or_default();
    Ok(CompileRun {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        status: CompileRunStatus::from_str_lenient(&status_s),
        source_ids,
        strategy: row.get(4)?,
        model_label: row.get::<_, Option<String>>(5).unwrap_or(None),
        fingerprint: row.get(6)?,
        error: row.get::<_, Option<String>>(7).unwrap_or(None),
        summary: row.get::<_, Option<String>>(8).unwrap_or(None),
        proposal_count: row.get::<_, i64>(9).unwrap_or(0).max(0) as u32,
        created_at: row.get(10)?,
        started_at: row.get::<_, Option<i64>>(11).unwrap_or(None),
        finished_at: row.get::<_, Option<i64>>(12).unwrap_or(None),
        updated_at: row.get(13)?,
    })
}

fn row_to_compile_proposal(row: &rusqlite::Row) -> rusqlite::Result<CompileProposal> {
    let kind_s: String = row.get(3)?;
    let status_s: String = row.get(4)?;
    let action_s: String = row.get(7)?;
    let source_ids_s: String = row.get(9)?;
    let action = serde_json::from_str::<CompileProposalAction>(&action_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let source_ids = serde_json::from_str::<Vec<String>>(&source_ids_s).unwrap_or_default();
    Ok(CompileProposal {
        id: row.get(0)?,
        run_id: row.get(1)?,
        kb_id: row.get(2)?,
        kind: CompileProposalKind::from_str_lenient(&kind_s),
        status: CompileProposalStatus::from_str_lenient(&status_s),
        title: row.get(5)?,
        detail: row.get(6)?,
        action,
        fingerprint: row.get(8)?,
        source_ids,
        created_at: row.get(10)?,
        decided_at: row.get::<_, Option<i64>>(11).unwrap_or(None),
        error: row.get::<_, Option<String>>(12).unwrap_or(None),
        before_text: row.get::<_, Option<String>>(13).unwrap_or(None),
        after_text: row.get::<_, Option<String>>(14).unwrap_or(None),
    })
}

fn row_to_source(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeSource> {
    let kind_s: String = row.get(2)?;
    let status_s: String = row.get(8)?;
    let chunk_count: i64 = row.get(13).unwrap_or(0);
    Ok(KnowledgeSource {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        kind: KnowledgeSourceKind::from_str_lenient(&kind_s),
        title: row.get(3)?,
        origin_uri: row.get::<_, Option<String>>(4).unwrap_or(None),
        stored_path: row.get(5)?,
        content_hash: row.get(6)?,
        extracted_text_hash: row.get::<_, Option<String>>(7).unwrap_or(None),
        status: KnowledgeSourceStatus::from_str_lenient(&status_s),
        compiled_at: row.get::<_, Option<i64>>(9).unwrap_or(None),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        size: row.get::<_, i64>(12).unwrap_or(0),
        chunk_count: chunk_count.max(0) as u32,
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
    fn source_registry_roundtrip_and_delete() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Sources".into(),
                emoji: None,
                root_dir: None,
            })
            .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let source = KnowledgeSource {
            id: "src-1".into(),
            kb_id: kb.id.clone(),
            kind: KnowledgeSourceKind::Markdown,
            title: "Article".into(),
            origin_uri: Some("https://example.com/a".into()),
            stored_path: "src-1.md".into(),
            content_hash: "hash".into(),
            extracted_text_hash: Some("text-hash".into()),
            status: KnowledgeSourceStatus::Ready,
            compiled_at: None,
            created_at: now,
            updated_at: now,
            size: 42,
            chunk_count: 0,
        };
        let chunks = vec![
            KnowledgeSourceChunk {
                id: 0,
                source_id: source.id.clone(),
                chunk_index: 0,
                body: "first".into(),
                start_offset: 0,
                end_offset: 5,
                content_hash: "c1".into(),
            },
            KnowledgeSourceChunk {
                id: 0,
                source_id: source.id.clone(),
                chunk_index: 1,
                body: "second".into(),
                start_offset: 5,
                end_offset: 11,
                content_hash: "c2".into(),
            },
        ];

        reg.insert_source(&source, &chunks).unwrap();
        let listed = reg.list_sources(&kb.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Article");
        assert_eq!(listed[0].chunk_count, 2);

        let fetched = reg.get_source(&kb.id, &source.id).unwrap().unwrap();
        assert_eq!(fetched.origin_uri.as_deref(), Some("https://example.com/a"));
        assert_eq!(fetched.size, 42);
        assert_eq!(fetched.chunk_count, 2);

        let rebuilt = reg
            .replace_source_chunks(
                &kb.id,
                &source.id,
                "new-hash",
                Some("new-text-hash"),
                5,
                &[KnowledgeSourceChunk {
                    id: 0,
                    source_id: source.id.clone(),
                    chunk_index: 0,
                    body: "new".into(),
                    start_offset: 0,
                    end_offset: 3,
                    content_hash: "c3".into(),
                }],
            )
            .unwrap()
            .unwrap();
        assert_eq!(rebuilt.content_hash, "new-hash");
        assert_eq!(rebuilt.chunk_count, 1);
        assert_eq!(rebuilt.size, 5);

        let stored_path = reg.delete_source(&kb.id, &source.id).unwrap();
        assert_eq!(stored_path.as_deref(), Some("src-1.md"));
        assert!(reg.get_source(&kb.id, &source.id).unwrap().is_none());
        assert!(reg.list_sources(&kb.id).unwrap().is_empty());
    }

    #[test]
    fn compile_run_and_proposal_lifecycle_is_durable_and_deduped() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Compile".into(),
                emoji: None,
                root_dir: None,
            })
            .unwrap();
        let source_ids = vec!["src-1".to_string()];
        let (run, should_execute) = reg
            .begin_compile_run(&kb.id, &source_ids, "source_summary_v1", "run-fp")
            .unwrap();
        assert!(should_execute);
        assert_eq!(run.status, CompileRunStatus::Running);
        assert_eq!(run.source_ids, source_ids);

        let (duplicate, should_execute_duplicate) = reg
            .begin_compile_run(&kb.id, &source_ids, "source_summary_v1", "run-fp")
            .unwrap();
        assert!(!should_execute_duplicate);
        assert_eq!(duplicate.id, run.id);

        let proposal = NewCompileProposal {
            kind: CompileProposalKind::CreateNote,
            title: "Compile Article".into(),
            detail: "src-1 -> Source Summaries/Article.md".into(),
            action: CompileProposalAction::CreateNote {
                path: "Source Summaries/Article.md".into(),
                content: "# Article\n".into(),
                overwrite: false,
            },
            fingerprint: "proposal-fp".into(),
            source_ids: source_ids.clone(),
            before_text: Some(String::new()),
            after_text: Some("# Article\n".into()),
        };
        assert_eq!(
            reg.insert_compile_proposals(&run.id, &kb.id, &[proposal.clone()])
                .unwrap(),
            1
        );
        assert_eq!(
            reg.insert_compile_proposals(&run.id, &kb.id, &[proposal])
                .unwrap(),
            0
        );

        let drafts = reg
            .list_compile_proposals(&kb.id, Some(&run.id), Some(CompileProposalStatus::Draft))
            .unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].source_ids, source_ids);
        assert_eq!(drafts[0].before_text.as_deref(), Some(""));
        assert_eq!(drafts[0].after_text.as_deref(), Some("# Article\n"));
        match &drafts[0].action {
            CompileProposalAction::CreateNote { path, content, .. } => {
                assert_eq!(path, "Source Summaries/Article.md");
                assert_eq!(content, "# Article\n");
            }
            other => panic!("unexpected proposal action: {other:?}"),
        }

        reg.finish_compile_run(
            &run.id,
            CompileRunStatus::Completed,
            Some("Generated 1 review proposal."),
            None,
            1,
            Some("provider/model"),
        )
        .unwrap();
        let completed = reg.get_compile_run(&run.id).unwrap().unwrap();
        assert_eq!(completed.status, CompileRunStatus::Completed);
        assert_eq!(completed.proposal_count, 1);
        assert_eq!(completed.model_label.as_deref(), Some("provider/model"));
        assert_eq!(
            completed.summary.as_deref(),
            Some("Generated 1 review proposal.")
        );

        reg.set_compile_proposal_status(drafts[0].id, CompileProposalStatus::Applied, None)
            .unwrap();
        let applied = reg.get_compile_proposal(drafts[0].id).unwrap().unwrap();
        assert_eq!(applied.status, CompileProposalStatus::Applied);
        assert!(applied.decided_at.is_some());
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

//! KnowledgeRegistry — the `knowledge_bases` + access-binding tables.
//!
//! Truth source (design D9). Wraps `Arc<SessionDB>` so the tables live inside
//! `sessions.db` next to `projects` / `channel_conversations`, sharing the one
//! SQLite connection. `~/.hope-agent/knowledge/index.db` holds only the
//! rebuildable note/chunk/link cache — never the registry.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::types::{
    CompileProposal, CompileProposalAction, CompileProposalKind, CompileProposalStatus, CompileRun,
    CompileRunStatus, CreateKnowledgeBaseInput, GraphNodePosition, KbAccess, KnowledgeBase,
    KnowledgeBaseMeta, KnowledgeExternalRawSyncMode, KnowledgeSource, KnowledgeSourceAsset,
    KnowledgeSourceAssetKind, KnowledgeSourceAssets, KnowledgeSourceChunk,
    KnowledgeSourceImportItem, KnowledgeSourceImportItemStatus, KnowledgeSourceImportRun,
    KnowledgeSourceImportRunStatus, KnowledgeSourceKind, KnowledgeSourceOcrPage,
    KnowledgeSourceOcrPageStage, KnowledgeSourceOcrPageStatus, KnowledgeSourceStatus,
    NewCompileProposal, SchemaProfile, UpdateKnowledgeBaseInput,
};
use crate::session::SessionDB;

/// Knowledge base persistence manager. Wraps `Arc<SessionDB>` to reuse its
/// connection (the tables live in `sessions.db`).
pub struct KnowledgeRegistry {
    session_db: Arc<SessionDB>,
}

pub struct StoredSourceImportItem {
    pub item: KnowledgeSourceImportItem,
    pub input_json: String,
}

pub struct DeletedSourceFiles {
    pub stored_path: String,
    pub external_raw_path: Option<String>,
    pub asset_paths: Vec<String>,
}

pub struct SourceAssetPruneCandidate {
    pub kb_id: String,
    pub source_id: String,
    pub stored_paths: Vec<String>,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct EvidenceRefIndexInput {
    pub source_id: String,
    pub cited_in: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EvidenceClaimIndexInput {
    pub claim_index: u32,
    pub section: String,
    pub claim_text: String,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EvidenceRefIndexRow {
    pub kb_id: String,
    pub rel_path: String,
    pub note_title: String,
    pub note_type: Option<String>,
    pub source_id: String,
    pub cited_in: Vec<String>,
    pub note_last_compiled_at: Option<i64>,
    pub claim_count: u32,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct EvidenceClaimIndexRow {
    pub kb_id: String,
    pub rel_path: String,
    pub note_title: String,
    pub source_id: String,
    pub claim_index: u32,
    pub section: String,
    pub claim_text: String,
    pub note_last_compiled_at: Option<i64>,
    pub updated_at: i64,
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
                external_raw_sync    TEXT NOT NULL DEFAULT 'disabled',
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

            -- Derived Evidence index (D17+): rebuildable rows parsed from note
            -- frontmatter / Evidence sections. Stored in sessions.db so source
            -- reverse lookups can join the source truth table without touching
            -- the note index cache.
            CREATE TABLE IF NOT EXISTS knowledge_evidence_refs (
                kb_id                 TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                rel_path              TEXT NOT NULL,
                note_title            TEXT NOT NULL,
                note_type             TEXT,
                source_id             TEXT NOT NULL,
                cited_in_json         TEXT NOT NULL,
                note_last_compiled_at INTEGER,
                claim_count           INTEGER NOT NULL DEFAULT 0,
                updated_at            INTEGER NOT NULL,
                PRIMARY KEY (kb_id, rel_path, source_id)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_evidence_refs_source
                ON knowledge_evidence_refs(kb_id, source_id);
            CREATE INDEX IF NOT EXISTS idx_knowledge_evidence_refs_note
                ON knowledge_evidence_refs(kb_id, rel_path);

            CREATE TABLE IF NOT EXISTS knowledge_evidence_claims (
                kb_id                 TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                rel_path              TEXT NOT NULL,
                note_title            TEXT NOT NULL,
                source_id             TEXT NOT NULL,
                claim_index           INTEGER NOT NULL,
                section               TEXT NOT NULL,
                claim_text            TEXT NOT NULL,
                note_last_compiled_at INTEGER,
                updated_at            INTEGER NOT NULL,
                PRIMARY KEY (kb_id, rel_path, source_id, claim_index)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_evidence_claims_source
                ON knowledge_evidence_claims(kb_id, source_id, rel_path);

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
                external_raw_path   TEXT,
                content_hash        TEXT NOT NULL,
                extracted_text_hash TEXT,
                status              TEXT NOT NULL DEFAULT 'ready',
                compiled_at         INTEGER,
                created_at          INTEGER NOT NULL,
                updated_at          INTEGER NOT NULL,
                size                INTEGER NOT NULL DEFAULT 0,
                version_of_source_id TEXT,
                version_index       INTEGER NOT NULL DEFAULT 1,
                superseded_by_source_id TEXT,
                superseded_at       INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_kb
                ON knowledge_sources(kb_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_hash
                ON knowledge_sources(kb_id, content_hash);
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_extracted_hash
                ON knowledge_sources(kb_id, extracted_text_hash);
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_version_root
                ON knowledge_sources(kb_id, version_of_source_id, version_index DESC);
            CREATE INDEX IF NOT EXISTS idx_knowledge_sources_superseded
                ON knowledge_sources(kb_id, superseded_by_source_id);

            -- Optional original-media retention for source imports. Metadata is
            -- truth-source state so retained binaries can be quota-managed and
            -- cleaned when sources are deleted. Files live under the internal
            -- Hope-managed sources directory, never an external vault.
            CREATE TABLE IF NOT EXISTS knowledge_source_assets (
                id          TEXT PRIMARY KEY,
                kb_id       TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                source_id   TEXT NOT NULL REFERENCES knowledge_sources(id) ON DELETE CASCADE,
                asset_kind  TEXT NOT NULL,
                stored_path TEXT NOT NULL,
                file_name   TEXT NOT NULL,
                mime_type   TEXT NOT NULL,
                size        INTEGER NOT NULL DEFAULT 0,
                width       INTEGER,
                height      INTEGER,
                created_at  INTEGER NOT NULL,
                UNIQUE(source_id, asset_kind)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_assets_kb
                ON knowledge_source_assets(kb_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_assets_source
                ON knowledge_source_assets(source_id, asset_kind);

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

            -- Knowledge Compiler Phase 10: durable source import pipeline.
            -- Runs/items make large imports observable and retriable. The input
            -- JSON is retained only for failed-item retry; API responses never
            -- echo it back to avoid surfacing large base64 payloads.
            CREATE TABLE IF NOT EXISTS knowledge_source_import_runs (
                id                TEXT PRIMARY KEY,
                kb_id             TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                status            TEXT NOT NULL,
                background_job_id TEXT,
                created_at        INTEGER NOT NULL,
                started_at        INTEGER,
                finished_at       INTEGER,
                updated_at        INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_import_runs_kb
                ON knowledge_source_import_runs(kb_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS knowledge_source_import_items (
                id                     INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id                 TEXT NOT NULL REFERENCES knowledge_source_import_runs(id) ON DELETE CASCADE,
                kb_id                  TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                position               INTEGER NOT NULL,
                client_id              TEXT,
                label                  TEXT,
                input_json             TEXT NOT NULL,
                kind                   TEXT,
                status                 TEXT NOT NULL,
                source_id              TEXT REFERENCES knowledge_sources(id) ON DELETE SET NULL,
                duplicate_of_source_id TEXT REFERENCES knowledge_sources(id) ON DELETE SET NULL,
                error                  TEXT,
                created_at             INTEGER NOT NULL,
                started_at             INTEGER,
                finished_at            INTEGER,
                updated_at             INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_import_items_run
                ON knowledge_source_import_items(run_id, position);
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_import_items_kb_status
                ON knowledge_source_import_items(kb_id, status, updated_at DESC);

            -- Scanned-PDF OCR fallback: per-page tracking, one row per
            -- (source, page). A finer grain than knowledge_source_import_items
            -- (file-level) — deliberately not reused, since forcing N pages of
            -- one file into that table would break its one-item-to-at-most-
            -- one-source dedup/count invariants. Never stores OCR text
            -- itself (that stays in the .md snapshot); only status/error/
            -- pointer, same non-duplication principle as import_items.
            CREATE TABLE IF NOT EXISTS knowledge_source_ocr_pages (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id      TEXT NOT NULL REFERENCES knowledge_sources(id) ON DELETE CASCADE,
                kb_id          TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                page_number    INTEGER NOT NULL,
                status         TEXT NOT NULL,
                stage          TEXT,
                error          TEXT,
                model_label    TEXT,
                attempt_count  INTEGER NOT NULL DEFAULT 0,
                ocr_text_chars INTEGER,
                created_at     INTEGER NOT NULL,
                started_at     INTEGER,
                finished_at    INTEGER,
                updated_at     INTEGER NOT NULL,
                UNIQUE(source_id, page_number)
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_ocr_pages_source
                ON knowledge_source_ocr_pages(source_id, page_number);
            CREATE INDEX IF NOT EXISTS idx_knowledge_source_ocr_pages_kb_status
                ON knowledge_source_ocr_pages(kb_id, status, updated_at DESC);

            -- User governance memory for source similarity suggestions. Source
            -- groups are computed, but dismiss decisions are owner truth.
            CREATE TABLE IF NOT EXISTS knowledge_source_similarity_dismissals (
                kb_id        TEXT NOT NULL REFERENCES knowledge_bases(id) ON DELETE CASCADE,
                fingerprint  TEXT NOT NULL,
                reason       TEXT,
                dismissed_at INTEGER NOT NULL,
                PRIMARY KEY (kb_id, fingerprint)
            );

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
        let has_external_raw_sync = conn
            .prepare("SELECT external_raw_sync FROM knowledge_bases LIMIT 1")
            .is_ok();
        if !has_external_raw_sync {
            conn.execute_batch(
                "ALTER TABLE knowledge_bases
                 ADD COLUMN external_raw_sync TEXT NOT NULL DEFAULT 'disabled';",
            )?;
        }

        let has_source_version = conn
            .prepare("SELECT version_index FROM knowledge_sources LIMIT 1")
            .is_ok();
        if !has_source_version {
            conn.execute_batch(
                "ALTER TABLE knowledge_sources ADD COLUMN version_of_source_id TEXT;
                 ALTER TABLE knowledge_sources ADD COLUMN version_index INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE knowledge_sources ADD COLUMN superseded_by_source_id TEXT;
                 ALTER TABLE knowledge_sources ADD COLUMN superseded_at INTEGER;",
            )?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_knowledge_sources_version_root
                ON knowledge_sources(kb_id, version_of_source_id, version_index DESC);
             CREATE INDEX IF NOT EXISTS idx_knowledge_sources_superseded
                ON knowledge_sources(kb_id, superseded_by_source_id);",
        )?;
        let has_source_external_raw_path = conn
            .prepare("SELECT external_raw_path FROM knowledge_sources LIMIT 1")
            .is_ok();
        if !has_source_external_raw_path {
            conn.execute_batch(
                "ALTER TABLE knowledge_sources
                 ADD COLUMN external_raw_path TEXT;",
            )?;
        }
        let has_source_import_background_job_id = conn
            .prepare("SELECT background_job_id FROM knowledge_source_import_runs LIMIT 1")
            .is_ok();
        if !has_source_import_background_job_id {
            conn.execute_batch(
                "ALTER TABLE knowledge_source_import_runs
                 ADD COLUMN background_job_id TEXT;",
            )?;
        }
        drop(conn);
        self.interrupt_running_source_import_runs("interrupted by application restart")?;
        self.interrupt_running_source_ocr_pages("interrupted by application restart")?;

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
                (id, name, emoji, root_dir, allow_external_writes, external_raw_sync,
                 archived, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, 'disabled', 0, ?5, ?6)",
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
            external_raw_sync: KnowledgeExternalRawSyncMode::Disabled,
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
                "SELECT id, name, emoji, root_dir, archived, created_at, updated_at,
                        allow_external_writes, external_raw_sync
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
        if let Some(mode) = patch.external_raw_sync {
            let idx = params_vec.len() + 1;
            sets.push(format!("external_raw_sync = ?{}", idx));
            params_vec.push(Box::new(mode.as_str().to_string()));
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
                "SELECT id, name, emoji, root_dir, archived, created_at, updated_at,
                        allow_external_writes, external_raw_sync
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
            "SELECT id, name, emoji, root_dir, archived, created_at, updated_at,
                    allow_external_writes, external_raw_sync
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

    pub fn replace_note_evidence_index(
        &self,
        kb_id: &str,
        rel_path: &str,
        note_title: &str,
        note_type: Option<&str>,
        note_last_compiled_at: Option<i64>,
        refs: &[EvidenceRefIndexInput],
        claims: &[EvidenceClaimIndexInput],
    ) -> Result<(usize, usize)> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM knowledge_evidence_claims WHERE kb_id = ?1 AND rel_path = ?2",
            params![kb_id, rel_path],
        )?;
        tx.execute(
            "DELETE FROM knowledge_evidence_refs WHERE kb_id = ?1 AND rel_path = ?2",
            params![kb_id, rel_path],
        )?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut inserted_claim_rows = 0usize;
        for r in refs {
            let cited_in_json = serde_json::to_string(&r.cited_in)?;
            let claim_count = claims
                .iter()
                .filter(|claim| claim.source_ids.iter().any(|id| id == &r.source_id))
                .count();
            tx.execute(
                "INSERT INTO knowledge_evidence_refs
                    (kb_id, rel_path, note_title, note_type, source_id, cited_in_json,
                     note_last_compiled_at, claim_count, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    kb_id,
                    rel_path,
                    note_title,
                    note_type,
                    r.source_id,
                    cited_in_json,
                    note_last_compiled_at,
                    claim_count as i64,
                    now,
                ],
            )?;
            for claim in claims
                .iter()
                .filter(|claim| claim.source_ids.iter().any(|id| id == &r.source_id))
            {
                tx.execute(
                    "INSERT INTO knowledge_evidence_claims
                        (kb_id, rel_path, note_title, source_id, claim_index, section,
                         claim_text, note_last_compiled_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        kb_id,
                        rel_path,
                        note_title,
                        r.source_id,
                        claim.claim_index as i64,
                        claim.section,
                        claim.claim_text,
                        note_last_compiled_at,
                        now,
                    ],
                )?;
                inserted_claim_rows = inserted_claim_rows.saturating_add(1);
            }
        }
        tx.commit()?;
        Ok((refs.len(), inserted_claim_rows))
    }

    pub fn delete_note_evidence_index(&self, kb_id: &str, rel_path: &str) -> Result<()> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM knowledge_evidence_claims WHERE kb_id = ?1 AND rel_path = ?2",
            params![kb_id, rel_path],
        )?;
        tx.execute(
            "DELETE FROM knowledge_evidence_refs WHERE kb_id = ?1 AND rel_path = ?2",
            params![kb_id, rel_path],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn evidence_refs_for_note(
        &self,
        kb_id: &str,
        rel_path: &str,
    ) -> Result<Vec<EvidenceRefIndexRow>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kb_id, rel_path, note_title, note_type, source_id, cited_in_json,
                    note_last_compiled_at, claim_count, updated_at
             FROM knowledge_evidence_refs
             WHERE kb_id = ?1 AND rel_path = ?2
             ORDER BY source_id ASC",
        )?;
        let rows = stmt.query_map(params![kb_id, rel_path], row_to_evidence_ref)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn evidence_refs_for_kb(&self, kb_id: &str) -> Result<Vec<EvidenceRefIndexRow>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kb_id, rel_path, note_title, note_type, source_id, cited_in_json,
                    note_last_compiled_at, claim_count, updated_at
             FROM knowledge_evidence_refs
             WHERE kb_id = ?1
             ORDER BY rel_path ASC, source_id ASC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_evidence_ref)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn evidence_claims_for_kb(&self, kb_id: &str) -> Result<Vec<EvidenceClaimIndexRow>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kb_id, rel_path, note_title, source_id, claim_index, section,
                    claim_text, note_last_compiled_at, updated_at
             FROM knowledge_evidence_claims
             WHERE kb_id = ?1
             ORDER BY rel_path ASC, claim_index ASC, source_id ASC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_evidence_claim)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn evidence_claims_for_source(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Vec<EvidenceClaimIndexRow>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kb_id, rel_path, note_title, source_id, claim_index, section,
                    claim_text, note_last_compiled_at, updated_at
             FROM knowledge_evidence_claims
             WHERE kb_id = ?1 AND source_id = ?2
             ORDER BY rel_path ASC, claim_index ASC",
        )?;
        let rows = stmt.query_map(params![kb_id, source_id], row_to_evidence_claim)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
                (id, kb_id, kind, title, origin_uri, stored_path, external_raw_path,
                 content_hash, extracted_text_hash, status, compiled_at, created_at,
                 updated_at, size, version_of_source_id, version_index,
                 superseded_by_source_id, superseded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                source.id,
                source.kb_id,
                source.kind.as_str(),
                source.title,
                source.origin_uri,
                source.stored_path,
                source.external_raw_path,
                source.content_hash,
                source.extracted_text_hash,
                source.status.as_str(),
                source.compiled_at,
                source.created_at,
                source.updated_at,
                source.size,
                source.version_of_source_id,
                source.version_index as i64,
                source.superseded_by_source_id,
                source.superseded_at,
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
        insert_source_assets_tx(&tx, source)?;
        tx.commit()?;
        Ok(())
    }

    pub fn insert_source_version(
        &self,
        previous_source_id: &str,
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
                (id, kb_id, kind, title, origin_uri, stored_path, external_raw_path,
                 content_hash, extracted_text_hash, status, compiled_at, created_at,
                 updated_at, size, version_of_source_id, version_index,
                 superseded_by_source_id, superseded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, NULL, NULL)",
            params![
                source.id,
                source.kb_id,
                source.kind.as_str(),
                source.title,
                source.origin_uri,
                source.stored_path,
                source.external_raw_path,
                source.content_hash,
                source.extracted_text_hash,
                source.status.as_str(),
                source.compiled_at,
                source.created_at,
                source.updated_at,
                source.size,
                source.version_of_source_id,
                source.version_index as i64,
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
        insert_source_assets_tx(&tx, source)?;
        tx.execute(
            "UPDATE knowledge_sources
             SET superseded_by_source_id = ?3, superseded_at = ?4
             WHERE kb_id = ?1 AND id = ?2",
            params![
                source.kb_id,
                previous_source_id,
                source.id,
                source.created_at,
            ],
        )?;
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
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1 AND s.superseded_by_source_id IS NULL
             GROUP BY s.id
             ORDER BY s.created_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_source)?;
        let mut sources = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_source_assets_locked(&conn, &mut sources)?;
        Ok(sources)
    }

    pub fn list_all_sources(&self, kb_id: &str) -> Result<Vec<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1
             GROUP BY s.id
             ORDER BY s.created_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map(params![kb_id], row_to_source)?;
        let mut sources = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_source_assets_locked(&conn, &mut sources)?;
        Ok(sources)
    }

    pub fn list_current_sources_for_similarity(
        &self,
        primary_kb_id: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeSource>> {
        let limit = limit.clamp(1, 1000) as i64;
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             JOIN knowledge_bases kb ON kb.id = s.kb_id
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.superseded_by_source_id IS NULL
               AND kb.archived = 0
             GROUP BY s.id
             ORDER BY CASE WHEN s.kb_id = ?1 THEN 0 ELSE 1 END,
                      s.created_at DESC, s.id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![primary_kb_id, limit], row_to_source)?;
        let mut sources = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_source_assets_locked(&conn, &mut sources)?;
        Ok(sources)
    }

    pub fn dismissed_source_similarity_fingerprints(
        &self,
        kb_id: &str,
    ) -> Result<BTreeSet<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT fingerprint
             FROM knowledge_source_similarity_dismissals
             WHERE kb_id = ?1",
        )?;
        let rows = stmt.query_map(params![kb_id], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(Into::into)
    }

    pub fn dismiss_source_similarity_group(
        &self,
        kb_id: &str,
        fingerprint: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO knowledge_source_similarity_dismissals
                (kb_id, fingerprint, reason, dismissed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kb_id, fingerprint) DO UPDATE SET
                reason = excluded.reason,
                dismissed_at = excluded.dismissed_at",
            params![kb_id, fingerprint, reason, now],
        )?;
        Ok(())
    }

    pub fn get_source(&self, kb_id: &str, source_id: &str) -> Result<Option<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut source = conn
            .query_row(
                "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1 AND s.id = ?2
             GROUP BY s.id",
                params![kb_id, source_id],
                row_to_source,
            )
            .optional()?;
        if let Some(source) = source.as_mut() {
            hydrate_source_assets_locked(&conn, std::slice::from_mut(source))?;
        }
        Ok(source)
    }

    pub fn find_source_by_extracted_text_hash(
        &self,
        kb_id: &str,
        extracted_text_hash: &str,
    ) -> Result<Option<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut source = conn
            .query_row(
                "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1 AND s.extracted_text_hash = ?2
               AND s.superseded_by_source_id IS NULL
             GROUP BY s.id
             ORDER BY s.created_at ASC, s.id ASC
             LIMIT 1",
                params![kb_id, extracted_text_hash],
                row_to_source,
            )
            .optional()?;
        if let Some(source) = source.as_mut() {
            hydrate_source_assets_locked(&conn, std::slice::from_mut(source))?;
        }
        Ok(source)
    }

    pub fn source_versions(&self, kb_id: &str, source_id: &str) -> Result<Vec<KnowledgeSource>> {
        let Some(anchor) = self.get_source(kb_id, source_id)? else {
            return Ok(Vec::new());
        };
        let root_id = anchor
            .version_of_source_id
            .clone()
            .unwrap_or_else(|| anchor.id.clone());
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.kb_id, s.kind, s.title, s.origin_uri, s.stored_path,
                    s.external_raw_path, s.content_hash, s.extracted_text_hash, s.status, s.compiled_at,
                    s.created_at, s.updated_at, s.size, s.version_of_source_id,
                    s.version_index, s.superseded_by_source_id, s.superseded_at,
                    COUNT(c.id) AS chunk_count
             FROM knowledge_sources s
             LEFT JOIN knowledge_source_chunks c ON c.source_id = s.id
             WHERE s.kb_id = ?1 AND (s.id = ?2 OR s.version_of_source_id = ?2)
             GROUP BY s.id
             ORDER BY s.version_index DESC, s.created_at DESC, s.id DESC",
        )?;
        let rows = stmt.query_map(params![kb_id, root_id], row_to_source)?;
        let mut sources = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_source_assets_locked(&conn, &mut sources)?;
        Ok(sources)
    }

    pub fn current_source_for(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Option<KnowledgeSource>> {
        let versions = self.source_versions(kb_id, source_id)?;
        if let Some(current) = versions
            .iter()
            .find(|source| source.superseded_by_source_id.is_none())
        {
            Ok(Some(current.clone()))
        } else {
            Ok(versions.into_iter().next())
        }
    }

    pub fn next_source_version_index(&self, kb_id: &str, root_source_id: &str) -> Result<u32> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let max_index: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version_index), 1)
             FROM knowledge_sources
             WHERE kb_id = ?1 AND (id = ?2 OR version_of_source_id = ?2)",
            params![kb_id, root_source_id],
            |r| r.get(0),
        )?;
        Ok(max_index.max(1).saturating_add(1) as u32)
    }

    pub fn delete_source(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Option<DeletedSourceFiles>> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let asset_paths = source_asset_stored_paths_tx(&tx, kb_id, source_id)?;
        let row: Option<(String, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT stored_path, external_raw_path, superseded_by_source_id
                 FROM knowledge_sources WHERE kb_id = ?1 AND id = ?2",
                params![kb_id, source_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, Option<String>>(1).unwrap_or(None),
                        r.get::<_, Option<String>>(2).unwrap_or(None),
                    ))
                },
            )
            .optional()?;
        if let Some((_, _, next_source_id)) = &row {
            let now = chrono::Utc::now().timestamp_millis();
            tx.execute(
                "UPDATE knowledge_sources
                 SET superseded_by_source_id = ?3,
                     superseded_at = CASE WHEN ?3 IS NULL THEN NULL ELSE ?4 END
                 WHERE kb_id = ?1 AND superseded_by_source_id = ?2",
                params![kb_id, source_id, next_source_id, now],
            )?;
            tx.execute(
                "DELETE FROM knowledge_sources WHERE kb_id = ?1 AND id = ?2",
                params![kb_id, source_id],
            )?;
        }
        tx.commit()?;
        Ok(
            row.map(|(stored_path, external_raw_path, _)| DeletedSourceFiles {
                stored_path,
                external_raw_path,
                asset_paths,
            }),
        )
    }

    pub fn set_source_external_raw_path(
        &self,
        kb_id: &str,
        source_id: &str,
        external_raw_path: Option<&str>,
    ) -> Result<Option<KnowledgeSource>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let now = chrono::Utc::now().timestamp_millis();
        let affected = conn.execute(
            "UPDATE knowledge_sources
             SET external_raw_path = ?3, updated_at = ?4
             WHERE kb_id = ?1 AND id = ?2",
            params![kb_id, source_id, external_raw_path, now],
        )?;
        drop(conn);
        if affected == 0 {
            Ok(None)
        } else {
            self.get_source(kb_id, source_id)
        }
    }

    pub fn source_asset(
        &self,
        kb_id: &str,
        source_id: &str,
        kind: KnowledgeSourceAssetKind,
    ) -> Result<Option<KnowledgeSourceAsset>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut asset = conn
            .query_row(
                "SELECT asset_kind, file_name, mime_type, size, width, height, stored_path, created_at
                 FROM knowledge_source_assets
                 WHERE kb_id = ?1 AND source_id = ?2 AND asset_kind = ?3",
                params![kb_id, source_id, kind.as_str()],
                row_to_source_asset,
            )
            .optional()?;
        if let Some(asset) = asset.as_mut() {
            set_asset_local_path(kb_id, asset);
        }
        Ok(asset)
    }

    pub fn source_asset_stored_paths(&self, kb_id: &str, source_id: &str) -> Result<Vec<String>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        source_asset_stored_paths_conn(&conn, kb_id, source_id)
    }

    pub fn total_source_asset_bytes(&self) -> Result<u64> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size), 0) FROM knowledge_source_assets",
            [],
            |r| r.get(0),
        )?;
        Ok(total.max(0) as u64)
    }

    pub fn list_source_asset_prune_candidates(&self) -> Result<Vec<SourceAssetPruneCandidate>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT kb_id, source_id, COALESCE(SUM(size), 0) AS total_bytes,
                    GROUP_CONCAT(stored_path, char(10)) AS stored_paths,
                    MIN(created_at) AS oldest
             FROM knowledge_source_assets
             GROUP BY kb_id, source_id
             ORDER BY oldest ASC, source_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let paths: Option<String> = row.get(3)?;
            Ok(SourceAssetPruneCandidate {
                kb_id: row.get(0)?,
                source_id: row.get(1)?,
                bytes: row.get::<_, i64>(2)?.max(0) as u64,
                stored_paths: paths
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
                    .collect(),
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn delete_source_assets(&self, kb_id: &str, source_id: &str) -> Result<Vec<String>> {
        let mut conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let tx = conn.transaction()?;
        let paths = source_asset_stored_paths_tx(&tx, kb_id, source_id)?;
        tx.execute(
            "DELETE FROM knowledge_source_assets WHERE kb_id = ?1 AND source_id = ?2",
            params![kb_id, source_id],
        )?;
        tx.commit()?;
        Ok(paths)
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

    pub fn create_source_import_run(
        &self,
        kb_id: &str,
        total_count: usize,
    ) -> Result<KnowledgeSourceImportRun> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO knowledge_source_import_runs
                (id, kb_id, status, background_job_id, created_at, started_at,
                 finished_at, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?4, NULL, ?4)",
            params![
                id,
                kb_id,
                KnowledgeSourceImportRunStatus::Running.as_str(),
                now,
            ],
        )?;
        Ok(KnowledgeSourceImportRun {
            id,
            kb_id: kb_id.to_string(),
            status: KnowledgeSourceImportRunStatus::Running,
            background_job_id: None,
            total_count: total_count as u32,
            imported_count: 0,
            duplicate_count: 0,
            failed_count: 0,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
        })
    }

    pub fn set_source_import_run_background_job_id(
        &self,
        run_id: &str,
        background_job_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_runs
             SET background_job_id = ?2, updated_at = ?3
             WHERE id = ?1",
            params![run_id, background_job_id, now],
        )?;
        Ok(())
    }

    pub fn insert_source_import_item(
        &self,
        run_id: &str,
        kb_id: &str,
        position: u32,
        client_id: Option<&str>,
        label: Option<&str>,
        input_json: &str,
        kind: Option<KnowledgeSourceKind>,
    ) -> Result<KnowledgeSourceImportItem> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "INSERT INTO knowledge_source_import_items
                (run_id, kb_id, position, client_id, label, input_json, kind, status,
                 source_id, duplicate_of_source_id, error, created_at, started_at,
                 finished_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, NULL, ?9, NULL, NULL, ?9)",
            params![
                run_id,
                kb_id,
                position as i64,
                client_id,
                label,
                input_json,
                kind.map(|k| k.as_str().to_string()),
                KnowledgeSourceImportItemStatus::Pending.as_str(),
                now,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(KnowledgeSourceImportItem {
            id,
            run_id: run_id.to_string(),
            kb_id: kb_id.to_string(),
            position,
            client_id: client_id.map(str::to_string),
            label: label.map(str::to_string),
            kind,
            status: KnowledgeSourceImportItemStatus::Pending,
            source_id: None,
            duplicate_of_source_id: None,
            error: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        })
    }

    pub fn set_source_import_item_running(&self, item_id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_items
             SET status = ?2, started_at = COALESCE(started_at, ?3), updated_at = ?3
             WHERE id = ?1",
            params![
                item_id,
                KnowledgeSourceImportItemStatus::Running.as_str(),
                now,
            ],
        )?;
        Ok(())
    }

    pub fn finish_source_import_item(
        &self,
        item_id: i64,
        status: KnowledgeSourceImportItemStatus,
        source_id: Option<&str>,
        duplicate_of_source_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_items
             SET status = ?2,
                 source_id = ?3,
                 duplicate_of_source_id = ?4,
                 error = ?5,
                 started_at = COALESCE(started_at, ?6),
                 finished_at = ?6,
                 updated_at = ?6
             WHERE id = ?1",
            params![
                item_id,
                status.as_str(),
                source_id,
                duplicate_of_source_id,
                error,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn finish_source_import_run(
        &self,
        run_id: &str,
        status: KnowledgeSourceImportRunStatus,
    ) -> Result<Option<KnowledgeSourceImportRun>> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_runs
             SET status = ?2, finished_at = ?3, updated_at = ?3
             WHERE id = ?1",
            params![run_id, status.as_str(), now],
        )?;
        drop(conn);
        self.get_source_import_run(run_id)
    }

    pub fn interrupt_running_source_import_runs(&self, reason: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_items
             SET status = ?1,
                 error = COALESCE(error, ?2),
                 started_at = COALESCE(started_at, ?3),
                 finished_at = COALESCE(finished_at, ?3),
                 updated_at = ?3
             WHERE status IN ('pending', 'running')
               AND run_id IN (
                 SELECT id FROM knowledge_source_import_runs WHERE status = 'running'
               )",
            params![
                KnowledgeSourceImportItemStatus::Failed.as_str(),
                reason,
                now
            ],
        )?;
        conn.execute(
            "UPDATE knowledge_source_import_runs
             SET status = ?1,
                 finished_at = COALESCE(finished_at, ?2),
                 updated_at = ?2
             WHERE status = 'running'",
            params![KnowledgeSourceImportRunStatus::Failed.as_str(), now],
        )?;
        Ok(())
    }

    // ── CRUD: knowledge_source_ocr_pages (scanned-PDF OCR fallback) ──

    pub fn insert_source_ocr_pages(
        &self,
        kb_id: &str,
        source_id: &str,
        total_pages: u32,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        for page_number in 1..=total_pages {
            conn.execute(
                "INSERT INTO knowledge_source_ocr_pages
                    (source_id, kb_id, page_number, status, attempt_count, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)
                 ON CONFLICT(source_id, page_number) DO NOTHING",
                params![
                    source_id,
                    kb_id,
                    page_number,
                    KnowledgeSourceOcrPageStatus::Pending.as_str(),
                    now,
                ],
            )?;
        }
        Ok(())
    }

    pub fn set_ocr_page_running(&self, id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_ocr_pages
             SET status = ?2,
                 attempt_count = attempt_count + 1,
                 started_at = ?3,
                 updated_at = ?3
             WHERE id = ?1",
            params![id, KnowledgeSourceOcrPageStatus::Running.as_str(), now],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_ocr_page(
        &self,
        id: i64,
        status: KnowledgeSourceOcrPageStatus,
        stage: Option<KnowledgeSourceOcrPageStage>,
        error: Option<&str>,
        model_label: Option<&str>,
        text_chars: Option<u32>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_ocr_pages
             SET status = ?2,
                 stage = ?3,
                 error = ?4,
                 model_label = ?5,
                 ocr_text_chars = ?6,
                 finished_at = ?7,
                 updated_at = ?7
             WHERE id = ?1",
            params![
                id,
                status.as_str(),
                stage.map(|s| s.as_str()),
                error,
                model_label,
                text_chars,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn list_source_ocr_pages(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Vec<KnowledgeSourceOcrPage>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, source_id, kb_id, page_number, status, stage, error, model_label,
                    attempt_count, created_at, updated_at
             FROM knowledge_source_ocr_pages
             WHERE kb_id = ?1 AND source_id = ?2
             ORDER BY page_number ASC",
        )?;
        let rows = stmt.query_map(params![kb_id, source_id], row_to_source_ocr_page)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn failed_source_ocr_pages(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> Result<Vec<KnowledgeSourceOcrPage>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, source_id, kb_id, page_number, status, stage, error, model_label,
                    attempt_count, created_at, updated_at
             FROM knowledge_source_ocr_pages
             WHERE kb_id = ?1 AND source_id = ?2 AND status = ?3
             ORDER BY page_number ASC",
        )?;
        let rows = stmt.query_map(
            params![
                kb_id,
                source_id,
                KnowledgeSourceOcrPageStatus::Failed.as_str()
            ],
            row_to_source_ocr_page,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn set_source_status(
        &self,
        kb_id: &str,
        source_id: &str,
        status: KnowledgeSourceStatus,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_sources SET status = ?3, updated_at = ?4 WHERE kb_id = ?1 AND id = ?2",
            params![kb_id, source_id, status.as_str(), now],
        )?;
        Ok(())
    }

    /// Orphaned `pending`/`running` OCR pages left over from a process that
    /// died mid-run (the owning `tokio::spawn` task is gone) are, by
    /// definition, unrecoverable in place — mark them `failed` so their
    /// source's status can be re-derived and the pages can be retried.
    /// Mirrors `interrupt_running_source_import_runs`.
    pub fn interrupt_running_source_ocr_pages(&self, reason: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_ocr_pages
             SET status = ?1,
                 error = COALESCE(error, ?2),
                 started_at = COALESCE(started_at, ?3),
                 finished_at = COALESCE(finished_at, ?3),
                 updated_at = ?3
             WHERE status IN ('pending', 'running')",
            params![KnowledgeSourceOcrPageStatus::Failed.as_str(), reason, now],
        )?;
        Ok(())
    }

    pub fn fail_active_source_import_items(
        &self,
        kb_id: &str,
        run_id: &str,
        reason: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.execute(
            "UPDATE knowledge_source_import_items
             SET status = ?3,
                 error = COALESCE(error, ?4),
                 started_at = COALESCE(started_at, ?5),
                 finished_at = COALESCE(finished_at, ?5),
                 updated_at = ?5
             WHERE kb_id = ?1
               AND run_id = ?2
               AND status IN ('pending', 'running')",
            params![
                kb_id,
                run_id,
                KnowledgeSourceImportItemStatus::Failed.as_str(),
                reason,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn list_source_import_runs(
        &self,
        kb_id: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeSourceImportRun>> {
        let limit = limit.clamp(1, 100) as i64;
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(SOURCE_IMPORT_RUN_SELECT)?;
        let rows = stmt.query_map(params![kb_id, limit], row_to_source_import_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_source_import_run(&self, run_id: &str) -> Result<Option<KnowledgeSourceImportRun>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        conn.query_row(
            SOURCE_IMPORT_RUN_BY_ID_SELECT,
            params![run_id],
            row_to_source_import_run,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_source_import_items(&self, run_id: &str) -> Result<Vec<KnowledgeSourceImportItem>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, kb_id, position, client_id, label, kind, status,
                    source_id, duplicate_of_source_id, error, created_at, started_at,
                    finished_at, updated_at
             FROM knowledge_source_import_items
             WHERE run_id = ?1
             ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![run_id], row_to_source_import_item)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn failed_source_import_items(
        &self,
        kb_id: &str,
        run_id: &str,
    ) -> Result<Vec<StoredSourceImportItem>> {
        let conn = self
            .session_db
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, run_id, kb_id, position, client_id, label, input_json, kind, status,
                    source_id, duplicate_of_source_id, error, created_at, started_at,
                    finished_at, updated_at
             FROM knowledge_source_import_items
             WHERE kb_id = ?1 AND run_id = ?2 AND status = 'failed'
             ORDER BY position ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![kb_id, run_id], |row| {
            let input_json: String = row.get(6)?;
            Ok(StoredSourceImportItem {
                item: row_to_source_import_item_with_offset(row, 1)?,
                input_json,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
                   AND s.archived_at IS NULL
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
                   AND s.archived_at IS NULL
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
                 WHERE t.kb_id = ?1 AND s.archived_at IS NULL
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
    let external_raw_sync = row
        .get::<_, Option<String>>(8)
        .unwrap_or(None)
        .map(|s| KnowledgeExternalRawSyncMode::from_str_lenient(&s))
        .unwrap_or_default();
    Ok(KnowledgeBase {
        id: row.get(0)?,
        name: row.get(1)?,
        emoji: row.get::<_, Option<String>>(2).unwrap_or(None),
        root_dir: row.get::<_, Option<String>>(3).unwrap_or(None),
        archived: row.get::<_, i64>(4).unwrap_or(0) != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        allow_external_writes: row.get::<_, i64>(7).unwrap_or(0) != 0,
        external_raw_sync,
    })
}

fn row_to_evidence_ref(row: &rusqlite::Row) -> rusqlite::Result<EvidenceRefIndexRow> {
    let cited_in_json: String = row.get(5)?;
    let cited_in = serde_json::from_str::<Vec<String>>(&cited_in_json).unwrap_or_default();
    Ok(EvidenceRefIndexRow {
        kb_id: row.get(0)?,
        rel_path: row.get(1)?,
        note_title: row.get(2)?,
        note_type: row.get::<_, Option<String>>(3).unwrap_or(None),
        source_id: row.get(4)?,
        cited_in,
        note_last_compiled_at: row.get::<_, Option<i64>>(6).unwrap_or(None),
        claim_count: row.get::<_, i64>(7).unwrap_or(0).max(0) as u32,
        updated_at: row.get(8)?,
    })
}

fn row_to_evidence_claim(row: &rusqlite::Row) -> rusqlite::Result<EvidenceClaimIndexRow> {
    Ok(EvidenceClaimIndexRow {
        kb_id: row.get(0)?,
        rel_path: row.get(1)?,
        note_title: row.get(2)?,
        source_id: row.get(3)?,
        claim_index: row.get::<_, i64>(4).unwrap_or(0).max(0) as u32,
        section: row.get(5)?,
        claim_text: row.get(6)?,
        note_last_compiled_at: row.get::<_, Option<i64>>(7).unwrap_or(None),
        updated_at: row.get(8)?,
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
    let status_s: String = row.get(9)?;
    let chunk_count: i64 = row.get(18).unwrap_or(0);
    Ok(KnowledgeSource {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        kind: KnowledgeSourceKind::from_str_lenient(&kind_s),
        title: row.get(3)?,
        origin_uri: row.get::<_, Option<String>>(4).unwrap_or(None),
        stored_path: row.get(5)?,
        external_raw_path: row.get::<_, Option<String>>(6).unwrap_or(None),
        content_hash: row.get(7)?,
        extracted_text_hash: row.get::<_, Option<String>>(8).unwrap_or(None),
        status: KnowledgeSourceStatus::from_str_lenient(&status_s),
        compiled_at: row.get::<_, Option<i64>>(10).unwrap_or(None),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        size: row.get::<_, i64>(13).unwrap_or(0),
        chunk_count: chunk_count.max(0) as u32,
        version_of_source_id: row.get::<_, Option<String>>(14).unwrap_or(None),
        version_index: row.get::<_, i64>(15).unwrap_or(1).max(1) as u32,
        superseded_by_source_id: row.get::<_, Option<String>>(16).unwrap_or(None),
        superseded_at: row.get::<_, Option<i64>>(17).unwrap_or(None),
        assets: None,
    })
}

fn insert_source_assets_tx(tx: &rusqlite::Transaction<'_>, source: &KnowledgeSource) -> Result<()> {
    let Some(assets) = source.assets.as_ref() else {
        return Ok(());
    };
    for asset in [assets.original.as_ref(), assets.thumbnail.as_ref()]
        .into_iter()
        .flatten()
    {
        tx.execute(
            "INSERT INTO knowledge_source_assets
                (id, kb_id, source_id, asset_kind, stored_path, file_name, mime_type,
                 size, width, height, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(source_id, asset_kind) DO UPDATE SET
                stored_path = excluded.stored_path,
                file_name = excluded.file_name,
                mime_type = excluded.mime_type,
                size = excluded.size,
                width = excluded.width,
                height = excluded.height,
                created_at = excluded.created_at",
            params![
                uuid::Uuid::new_v4().to_string(),
                source.kb_id,
                source.id,
                asset.kind.as_str(),
                asset.stored_path,
                asset.file_name,
                asset.mime_type,
                asset.size,
                asset.width.map(|v| v as i64),
                asset.height.map(|v| v as i64),
                asset.created_at,
            ],
        )?;
    }
    Ok(())
}

fn hydrate_source_assets_locked(
    conn: &rusqlite::Connection,
    sources: &mut [KnowledgeSource],
) -> Result<()> {
    for source in sources {
        let mut stmt = conn.prepare(
            "SELECT asset_kind, file_name, mime_type, size, width, height, stored_path, created_at
             FROM knowledge_source_assets
             WHERE kb_id = ?1 AND source_id = ?2
             ORDER BY asset_kind ASC",
        )?;
        let rows = stmt.query_map(params![source.kb_id, source.id], row_to_source_asset)?;
        let mut assets = KnowledgeSourceAssets::default();
        for row in rows {
            let mut asset = row?;
            set_asset_local_path(&source.kb_id, &mut asset);
            match asset.kind {
                KnowledgeSourceAssetKind::Original => assets.original = Some(asset),
                KnowledgeSourceAssetKind::Thumbnail => assets.thumbnail = Some(asset),
            }
        }
        if assets.original.is_some() || assets.thumbnail.is_some() {
            source.assets = Some(assets);
        }
    }
    Ok(())
}

fn row_to_source_asset(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeSourceAsset> {
    let kind_s: String = row.get(0)?;
    Ok(KnowledgeSourceAsset {
        kind: KnowledgeSourceAssetKind::from_str_lenient(&kind_s),
        file_name: row.get(1)?,
        mime_type: row.get(2)?,
        size: row.get::<_, i64>(3).unwrap_or(0),
        width: row
            .get::<_, Option<i64>>(4)
            .unwrap_or(None)
            .map(|v| v.max(0) as u32),
        height: row
            .get::<_, Option<i64>>(5)
            .unwrap_or(None)
            .map(|v| v.max(0) as u32),
        stored_path: row.get(6)?,
        local_path: None,
        created_at: row.get(7)?,
    })
}

fn set_asset_local_path(kb_id: &str, asset: &mut KnowledgeSourceAsset) {
    if let Ok(root) = crate::paths::knowledge_kb_sources_dir(kb_id) {
        asset.local_path = Some(root.join(&asset.stored_path).to_string_lossy().to_string());
    }
}

fn source_asset_stored_paths_conn(
    conn: &rusqlite::Connection,
    kb_id: &str,
    source_id: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT stored_path FROM knowledge_source_assets WHERE kb_id = ?1 AND source_id = ?2",
    )?;
    let rows = stmt.query_map(params![kb_id, source_id], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn source_asset_stored_paths_tx(
    tx: &rusqlite::Transaction<'_>,
    kb_id: &str,
    source_id: &str,
) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(
        "SELECT stored_path FROM knowledge_source_assets WHERE kb_id = ?1 AND source_id = ?2",
    )?;
    let rows = stmt.query_map(params![kb_id, source_id], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

const SOURCE_IMPORT_RUN_SELECT: &str = "
    SELECT r.id, r.kb_id, r.status, r.background_job_id,
           r.created_at, r.started_at, r.finished_at, r.updated_at,
           COUNT(i.id) AS total_count,
           SUM(CASE WHEN i.status = 'imported' THEN 1 ELSE 0 END) AS imported_count,
           SUM(CASE WHEN i.status = 'duplicate' THEN 1 ELSE 0 END) AS duplicate_count,
           SUM(CASE WHEN i.status = 'failed' THEN 1 ELSE 0 END) AS failed_count
    FROM knowledge_source_import_runs r
    LEFT JOIN knowledge_source_import_items i ON i.run_id = r.id
    WHERE r.kb_id = ?1
    GROUP BY r.id
    ORDER BY r.created_at DESC, r.id DESC
    LIMIT ?2";

const SOURCE_IMPORT_RUN_BY_ID_SELECT: &str = "
    SELECT r.id, r.kb_id, r.status, r.background_job_id,
           r.created_at, r.started_at, r.finished_at, r.updated_at,
           COUNT(i.id) AS total_count,
           SUM(CASE WHEN i.status = 'imported' THEN 1 ELSE 0 END) AS imported_count,
           SUM(CASE WHEN i.status = 'duplicate' THEN 1 ELSE 0 END) AS duplicate_count,
           SUM(CASE WHEN i.status = 'failed' THEN 1 ELSE 0 END) AS failed_count
    FROM knowledge_source_import_runs r
    LEFT JOIN knowledge_source_import_items i ON i.run_id = r.id
    WHERE r.id = ?1
    GROUP BY r.id";

fn row_to_source_import_run(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeSourceImportRun> {
    let status_s: String = row.get(2)?;
    let total_count: i64 = row.get(8).unwrap_or(0);
    let imported_count: i64 = row.get(9).unwrap_or(0);
    let duplicate_count: i64 = row.get(10).unwrap_or(0);
    let failed_count: i64 = row.get(11).unwrap_or(0);
    Ok(KnowledgeSourceImportRun {
        id: row.get(0)?,
        kb_id: row.get(1)?,
        status: KnowledgeSourceImportRunStatus::from_str_lenient(&status_s),
        background_job_id: row.get::<_, Option<String>>(3).unwrap_or(None),
        created_at: row.get(4)?,
        started_at: row.get::<_, Option<i64>>(5).unwrap_or(None),
        finished_at: row.get::<_, Option<i64>>(6).unwrap_or(None),
        updated_at: row.get(7)?,
        total_count: total_count.max(0) as u32,
        imported_count: imported_count.max(0) as u32,
        duplicate_count: duplicate_count.max(0) as u32,
        failed_count: failed_count.max(0) as u32,
    })
}

fn row_to_source_import_item(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeSourceImportItem> {
    row_to_source_import_item_with_offset(row, 0)
}

fn row_to_source_import_item_with_offset(
    row: &rusqlite::Row,
    offset_after_label: usize,
) -> rusqlite::Result<KnowledgeSourceImportItem> {
    let shifted = |idx: usize| {
        if idx >= 6 {
            idx + offset_after_label
        } else {
            idx
        }
    };
    let kind = row
        .get::<_, Option<String>>(shifted(6))
        .unwrap_or(None)
        .map(|s| KnowledgeSourceKind::from_str_lenient(&s));
    let status_s: String = row.get(shifted(7))?;
    let position: i64 = row.get(3)?;
    Ok(KnowledgeSourceImportItem {
        id: row.get(0)?,
        run_id: row.get(1)?,
        kb_id: row.get(2)?,
        position: position.max(0) as u32,
        client_id: row.get::<_, Option<String>>(4).unwrap_or(None),
        label: row.get::<_, Option<String>>(5).unwrap_or(None),
        kind,
        status: KnowledgeSourceImportItemStatus::from_str_lenient(&status_s),
        source_id: row.get::<_, Option<String>>(shifted(8)).unwrap_or(None),
        duplicate_of_source_id: row.get::<_, Option<String>>(shifted(9)).unwrap_or(None),
        error: row.get::<_, Option<String>>(shifted(10)).unwrap_or(None),
        created_at: row.get(shifted(11))?,
        started_at: row.get::<_, Option<i64>>(shifted(12)).unwrap_or(None),
        finished_at: row.get::<_, Option<i64>>(shifted(13)).unwrap_or(None),
        updated_at: row.get(shifted(14))?,
    })
}

fn row_to_source_ocr_page(row: &rusqlite::Row) -> rusqlite::Result<KnowledgeSourceOcrPage> {
    let status_s: String = row.get(4)?;
    let stage_s: Option<String> = row.get(5)?;
    let page_number: i64 = row.get(3)?;
    let attempt_count: i64 = row.get(8)?;
    Ok(KnowledgeSourceOcrPage {
        id: row.get(0)?,
        source_id: row.get(1)?,
        kb_id: row.get(2)?,
        page_number: page_number.max(0) as u32,
        status: KnowledgeSourceOcrPageStatus::from_str_lenient(&status_s),
        stage: stage_s.and_then(|s| KnowledgeSourceOcrPageStage::from_str_lenient(&s)),
        error: row.get(6)?,
        model_label: row.get(7)?,
        attempt_count: attempt_count.max(0) as u32,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
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
        let session_db = Arc::new(SessionDB::open_ephemeral_for_test(&db_path).unwrap());
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
            external_raw_path: Some("raw/src-1.md".into()),
            content_hash: "hash".into(),
            extracted_text_hash: Some("text-hash".into()),
            status: KnowledgeSourceStatus::Ready,
            compiled_at: None,
            created_at: now,
            updated_at: now,
            size: 42,
            chunk_count: 0,
            version_of_source_id: None,
            version_index: 1,
            superseded_by_source_id: None,
            superseded_at: None,
            assets: Some(KnowledgeSourceAssets {
                original: Some(KnowledgeSourceAsset {
                    kind: KnowledgeSourceAssetKind::Original,
                    file_name: "recording.mp3".into(),
                    mime_type: "audio/mpeg".into(),
                    size: 123,
                    width: None,
                    height: None,
                    stored_path: "assets/src-1/original.mp3".into(),
                    local_path: None,
                    created_at: now,
                }),
                thumbnail: None,
            }),
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
        let listed_asset = listed[0]
            .assets
            .as_ref()
            .and_then(|assets| assets.original.as_ref())
            .expect("source asset is hydrated");
        assert_eq!(listed_asset.file_name, "recording.mp3");
        assert!(listed_asset
            .local_path
            .as_deref()
            .unwrap_or("")
            .contains("original.mp3"));

        let fetched = reg.get_source(&kb.id, &source.id).unwrap().unwrap();
        assert_eq!(fetched.origin_uri.as_deref(), Some("https://example.com/a"));
        assert_eq!(fetched.size, 42);
        assert_eq!(fetched.chunk_count, 2);
        assert_eq!(
            fetched
                .assets
                .as_ref()
                .and_then(|assets| assets.original.as_ref())
                .map(|asset| asset.size),
            Some(123)
        );

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

        let deleted = reg.delete_source(&kb.id, &source.id).unwrap().unwrap();
        assert_eq!(deleted.stored_path, "src-1.md");
        assert_eq!(deleted.external_raw_path.as_deref(), Some("raw/src-1.md"));
        assert_eq!(deleted.asset_paths, vec!["assets/src-1/original.mp3"]);
        assert!(reg.get_source(&kb.id, &source.id).unwrap().is_none());
        assert!(reg.list_sources(&kb.id).unwrap().is_empty());
    }

    #[test]
    fn source_versions_hide_superseded_from_current_list() {
        let (_d, reg) = registry();
        let kb = reg
            .create(CreateKnowledgeBaseInput {
                name: "Versions".into(),
                emoji: None,
                root_dir: None,
            })
            .unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let source = KnowledgeSource {
            id: "src-1".into(),
            kb_id: kb.id.clone(),
            kind: KnowledgeSourceKind::UrlSnapshot,
            title: "Article".into(),
            origin_uri: Some("https://example.com/a".into()),
            stored_path: "src-1.md".into(),
            external_raw_path: None,
            content_hash: "hash-1".into(),
            extracted_text_hash: Some("text-hash-1".into()),
            status: KnowledgeSourceStatus::Ready,
            compiled_at: None,
            created_at: now,
            updated_at: now,
            size: 42,
            chunk_count: 0,
            version_of_source_id: None,
            version_index: 1,
            superseded_by_source_id: None,
            superseded_at: None,
            assets: None,
        };
        reg.insert_source(&source, &[]).unwrap();

        let version = KnowledgeSource {
            id: "src-2".into(),
            kb_id: kb.id.clone(),
            kind: KnowledgeSourceKind::UrlSnapshot,
            title: "Article".into(),
            origin_uri: Some("https://example.com/a".into()),
            stored_path: "src-2.md".into(),
            external_raw_path: None,
            content_hash: "hash-2".into(),
            extracted_text_hash: Some("text-hash-2".into()),
            status: KnowledgeSourceStatus::Ready,
            compiled_at: None,
            created_at: now + 1,
            updated_at: now + 1,
            size: 64,
            chunk_count: 0,
            version_of_source_id: Some(source.id.clone()),
            version_index: 2,
            superseded_by_source_id: None,
            superseded_at: None,
            assets: None,
        };
        reg.insert_source_version(&source.id, &version, &[])
            .unwrap();

        let listed = reg.list_sources(&kb.id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "src-2");

        let old = reg.get_source(&kb.id, "src-1").unwrap().unwrap();
        assert_eq!(old.superseded_by_source_id.as_deref(), Some("src-2"));
        let current = reg.current_source_for(&kb.id, "src-1").unwrap().unwrap();
        assert_eq!(current.id, "src-2");
        let versions = reg.source_versions(&kb.id, "src-2").unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version_index, 2);
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
            reg.insert_compile_proposals(&run.id, &kb.id, std::slice::from_ref(&proposal))
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

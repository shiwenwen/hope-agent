use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::sync::{Arc, Mutex};

use crate::memory::traits::EmbeddingProvider;
use crate::memory::types::*;

/// Number of read-only connections in the pool.
const READ_POOL_SIZE: usize = 4;

// ── SQLite Backend ──────────────────────────────────────────────

/// SQLite-based memory backend with FTS5 full-text search + optional vector search.
///
/// Uses a write connection (Mutex) + a pool of read-only connections for concurrency.
/// With WAL mode, readers never block the writer and vice versa.
pub struct SqliteMemoryBackend {
    /// Exclusive write connection (also used as fallback reader)
    pub(crate) writer: Mutex<Connection>,
    /// Pool of read-only connections for concurrent reads
    pub(crate) readers: Vec<Mutex<Connection>>,
    /// Round-robin index for reader pool
    pub(crate) reader_idx: std::sync::atomic::AtomicUsize,
    /// Optional embedding provider for vector search
    pub(crate) embedder: std::sync::RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    /// Embedding dimensions (set when embedder is configured)
    pub(crate) embedding_dims: std::sync::atomic::AtomicU32,
    /// DB path, retained so connections can be reopened if the reader pool
    /// needs to grow or a worker thread needs its own handle. Currently unused
    /// at the Rust layer (pool is sized at open-time) but kept to avoid
    /// re-plumbing the path through the trait surface when that lands.
    #[allow(dead_code)]
    pub(crate) db_path: std::path::PathBuf,
}

impl SqliteMemoryBackend {
    /// Open (or create) the memory database with sqlite-vec extension.
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        // Register sqlite-vec extension before opening connection. The
        // explicit type annotation silences `clippy::missing_transmute_annotations`
        // without changing behavior — `sqlite3_auto_extension` takes a raw
        // function pointer that sqlite-vec ships as `extern "C"`.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open memory DB at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        // Create tables
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_type TEXT NOT NULL DEFAULT 'user',
                scope_type TEXT NOT NULL DEFAULT 'global',
                scope_agent_id TEXT,
                content TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                source TEXT NOT NULL DEFAULT 'user',
                source_session_id TEXT,
                embedding BLOB,
                embedding_signature TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_memories_pinned
                ON memories(pinned DESC, updated_at DESC);

            CREATE INDEX IF NOT EXISTS idx_memories_scope
                ON memories(scope_type, scope_agent_id);
            CREATE INDEX IF NOT EXISTS idx_memories_type
                ON memories(memory_type);
            CREATE INDEX IF NOT EXISTS idx_memories_source
                ON memories(source);
            CREATE INDEX IF NOT EXISTS idx_memories_updated
                ON memories(updated_at DESC);

            -- Append-only owner audit stream for ordinary legacy memory rows.
            -- This is not a second source of truth: it carries a bounded preview
            -- and metadata so Settings/API users can see adds, edits, pins and
            -- deletes after the live row is gone.
            CREATE TABLE IF NOT EXISTS memory_history (
                id TEXT PRIMARY KEY,
                memory_id INTEGER NOT NULL,
                action TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                scope_type TEXT NOT NULL DEFAULT 'global',
                scope_agent_id TEXT,
                scope_project_id TEXT,
                source TEXT NOT NULL DEFAULT 'user',
                source_session_id TEXT,
                content_preview TEXT NOT NULL DEFAULT '',
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_history_created
                ON memory_history(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_history_memory
                ON memory_history(memory_id, created_at DESC);

            -- FTS5 full-text search
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content, tags,
                content='memories',
                content_rowid='id',
                tokenize='unicode61'
            );

            -- Embedding cache to reduce API calls for repeated texts
            CREATE TABLE IF NOT EXISTS embedding_cache (
                hash TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                signature TEXT NOT NULL,
                embedding BLOB NOT NULL,
                dimensions INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (hash, provider, model, signature)
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content, tags)
                VALUES (new.id, new.content, new.tags);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content, tags)
                VALUES ('delete', old.id, old.content, old.tags);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content, tags)
                VALUES ('delete', old.id, old.content, old.tags);
                INSERT INTO memories_fts(rowid, content, tags)
                VALUES (new.id, new.content, new.tags);
            END;",
        )?;

        // Migration: add attachment columns for multimodal embedding
        if conn
            .prepare("SELECT attachment_path FROM memories LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN attachment_path TEXT;
                 ALTER TABLE memories ADD COLUMN attachment_mime TEXT;",
            );
        }

        // Migration: add scope_project_id column for Project scope.
        // NULL for all pre-existing rows (Global / Agent scope).
        if conn
            .prepare("SELECT scope_project_id FROM memories LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN scope_project_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_memories_scope_project
                     ON memories(scope_type, scope_project_id);",
            );
        }

        // Migration: track which embedding model produced each vector so old
        // vectors are never mixed with a newly selected memory embedding model.
        if conn
            .prepare("SELECT embedding_signature FROM memories LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "ALTER TABLE memories ADD COLUMN embedding_signature TEXT;
                 CREATE INDEX IF NOT EXISTS idx_memories_embedding_signature
                     ON memories(embedding_signature);",
            );
        }

        // Migration: the old cache keyed only provider/model and could reuse
        // embeddings across different base URLs or dimensions. Recreate it
        // once when the signature column is absent.
        if conn
            .prepare("SELECT signature FROM embedding_cache LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "DROP TABLE IF EXISTS embedding_cache;
                 CREATE TABLE IF NOT EXISTS embedding_cache (
                    hash TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    signature TEXT NOT NULL,
                    embedding BLOB NOT NULL,
                    dimensions INTEGER NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (hash, provider, model, signature)
                 );",
            );
        }

        // Rebuildable substring shadow index. FTS5's trigram tokenizer keeps
        // CJK fragments and identifier infixes off the O(n) LIKE fallback for
        // normal (>=3 character) queries while unicode61 remains the primary
        // word/token ranker.
        let literal_fts_existed = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memories_literal_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_literal_fts USING fts5(
                content, tags, source, source_session_id,
                content='memories',
                content_rowid='id',
                tokenize='trigram'
            );

            CREATE TRIGGER IF NOT EXISTS memories_literal_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_literal_fts(rowid, content, tags, source, source_session_id)
                VALUES (new.id, new.content, new.tags, new.source, new.source_session_id);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_literal_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_literal_fts(
                    memories_literal_fts, rowid, content, tags, source, source_session_id
                ) VALUES (
                    'delete', old.id, old.content, old.tags, old.source, old.source_session_id
                );
            END;

            CREATE TRIGGER IF NOT EXISTS memories_literal_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_literal_fts(
                    memories_literal_fts, rowid, content, tags, source, source_session_id
                ) VALUES (
                    'delete', old.id, old.content, old.tags, old.source, old.source_session_id
                );
                INSERT INTO memories_literal_fts(rowid, content, tags, source, source_session_id)
                VALUES (new.id, new.content, new.tags, new.source, new.source_session_id);
            END;",
        )?;
        if !literal_fts_existed {
            let _ = conn.execute_batch(
                "INSERT INTO memories_literal_fts(memories_literal_fts) VALUES('rebuild');",
            );
        }

        // ── Dreaming durable state (next-gen Dreaming Phase 0) ──────────
        // Audit trail + cross-process coordination for the offline
        // consolidation pipeline. Co-located in memory.db (not a separate
        // dreaming.db) so future claim / evidence tables (Phase 2+) can sync
        // transactionally against `memories.id`. All timestamps are RFC3339
        // UTC strings to match the rest of this database; lexical comparison
        // (`lease_expires_at < now`) is therefore valid for lease expiry.
        //
        // Schema mirrors docs/architecture/dreaming.md §数据模型 with three
        // parity columns added on `dreaming_runs` (nominated_count,
        // duration_ms, diary_path) so a durable run losslessly carries what
        // the in-memory `DreamReport` shows in the Dashboard.
        //
        // NOTE: when Phase 1+ starts writing agent/project-scoped
        // `scope_key`s (e.g. "agent:<id>"), those columns must be registered
        // with `agent::migration` for the `default → ha-main` rename. Phase 0
        // only writes global scope, so nothing here needs the rename yet.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dreaming_runs (
                id TEXT PRIMARY KEY,
                trigger TEXT NOT NULL,
                phase TEXT NOT NULL,
                status TEXT NOT NULL,
                owner_instance_id TEXT,
                heartbeat_at TEXT,
                lease_expires_at TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                scope_json TEXT NOT NULL DEFAULT '{}',
                scanned_count INTEGER NOT NULL DEFAULT 0,
                nominated_count INTEGER NOT NULL DEFAULT 0,
                decision_count INTEGER NOT NULL DEFAULT 0,
                promoted_count INTEGER NOT NULL DEFAULT 0,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                diary_path TEXT,
                note TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_dreaming_runs_started
                ON dreaming_runs(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_dreaming_runs_status
                ON dreaming_runs(status);

            -- Cross-process lease. `lock_key` = phase+scope (e.g. light:global).
            -- The in-process AtomicBool guards same-process overlap; this row
            -- guards desktop / server / ACP multi-process overlap.
            CREATE TABLE IF NOT EXISTS dreaming_locks (
                lock_key TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                owner_instance_id TEXT NOT NULL,
                heartbeat_at TEXT NOT NULL,
                lease_expires_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_dreaming_locks_expires
                ON dreaming_locks(lease_expires_at);

            -- Cross-run scan progress (foundation; the Light scanner still
            -- uses a time window in Phase 0 and only writes these).
            CREATE TABLE IF NOT EXISTS dreaming_watermarks (
                scope_key TEXT NOT NULL,
                source_type TEXT NOT NULL,
                last_source_id TEXT,
                last_source_ts TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (scope_key, source_type)
            );

            -- Durable queue so high-frequency source capture is not lost when
            -- a lease is held by another run. `updated_at` doubles as the
            -- claim timestamp for stale-claim recovery.
            CREATE TABLE IF NOT EXISTS dreaming_pending_sources (
                id TEXT PRIMARY KEY,
                scope_key TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source_id TEXT NOT NULL,
                source_ts TEXT,
                payload_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_dreaming_pending_sources_scope_status
                ON dreaming_pending_sources(scope_key, status, created_at);

            -- Machine-readable decision log (turns the Dream Diary into an
            -- auditable event stream). Phase 0 writes only `promote` rows.
            CREATE TABLE IF NOT EXISTS dreaming_decisions (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                decision_type TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT,
                score REAL,
                rationale TEXT NOT NULL,
                before_json TEXT,
                after_json TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (run_id) REFERENCES dreaming_runs(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_dreaming_decisions_run
                ON dreaming_decisions(run_id);",
        )?;

        // ── Claim schema (next-gen Dreaming, design §3.2 / §3.3 / §3.3.1) ──
        //
        // Structured long-term memory lives in this same memory.db alongside
        // `memories` (design §9.1). These tables are the dual-track foundation:
        // claims + per-claim evidence + a link table that keeps legacy
        // `memories` rows in sync without state drift. This PR only creates the
        // schema + a read API; claim extraction / dual-write / canonicalize (and
        // the `memory_claims_fts` index that canonicalize needs) land in a later
        // PR.
        //
        // The `ON DELETE CASCADE` foreign keys encode intent but are NOT
        // enforced yet — `PRAGMA foreign_keys` is off on this connection (see
        // top of `open`). Cascade enforcement + the pragma are enabled when the
        // claim write/delete path lands; until then there is no claim deletion
        // path so nothing relies on the cascade.
        //
        // NOTE: `memory_claims.scope_id` (when `scope_type='agent'`) is an
        // agent-id-bearing column and is registered with `agent::migration` for
        // the `default → ha-main` rename, per the §11 contract. It is empty
        // until dual-write starts, so the rename is defensive.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_claims (
                id TEXT PRIMARY KEY,
                scope_type TEXT NOT NULL,
                scope_id TEXT,
                claim_type TEXT NOT NULL,
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                content TEXT NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 0.5,
                confidence_source TEXT NOT NULL DEFAULT 'derived',
                salience REAL NOT NULL DEFAULT 0.5,
                freshness_policy_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'active',
                valid_from TEXT,
                valid_until TEXT,
                supersedes_claim_id TEXT,
                source_run_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                embedding_signature TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_memory_claims_scope
                ON memory_claims(scope_type, scope_id);
            CREATE INDEX IF NOT EXISTS idx_memory_claims_status
                ON memory_claims(status);
            CREATE INDEX IF NOT EXISTS idx_memory_claims_type
                ON memory_claims(claim_type);
            CREATE INDEX IF NOT EXISTS idx_memory_claims_spo
                ON memory_claims(subject, predicate);
            CREATE INDEX IF NOT EXISTS idx_memory_claims_updated
                ON memory_claims(updated_at DESC);

            -- Per-claim provenance. `evidence_class` baseline drives confidence;
            -- `quote` is short + redacted; incognito sources never persisted.
            CREATE TABLE IF NOT EXISTS memory_evidence (
                id TEXT PRIMARY KEY,
                claim_id TEXT NOT NULL,
                source_type TEXT NOT NULL,
                evidence_class TEXT NOT NULL DEFAULT 'assistant_inferred',
                source_id TEXT NOT NULL,
                session_id TEXT,
                message_id TEXT,
                file_path TEXT,
                url TEXT,
                quote TEXT,
                redaction_status TEXT NOT NULL DEFAULT 'redacted',
                access_scope_json TEXT NOT NULL DEFAULT '{}',
                weight REAL NOT NULL DEFAULT 1.0,
                created_at TEXT NOT NULL,
                FOREIGN KEY (claim_id) REFERENCES memory_claims(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_memory_evidence_claim
                ON memory_evidence(claim_id);

            -- Sync relationship between a claim and the legacy `memories` row(s)
            -- it manages, so dual-track state can't drift (design §3.3.1).
            CREATE TABLE IF NOT EXISTS memory_claim_links (
                claim_id TEXT NOT NULL,
                memory_id INTEGER NOT NULL,
                sync_mode TEXT NOT NULL DEFAULT 'managed',
                last_synced_claim_status TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (claim_id, memory_id),
                FOREIGN KEY (claim_id) REFERENCES memory_claims(id) ON DELETE CASCADE,
                FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_memory_claim_links_memory
                ON memory_claim_links(memory_id);",
        )?;

        // ── Memory Profile snapshots (next-gen Dreaming Phase 4, design §3.5) ──
        //
        // Displayable + injectable profile summaries synthesised from active
        // claims, layered by scope (global / agent / project). One row per
        // (scope, version); `version` is allocated as `MAX(version)+1` inside a
        // write transaction so the latest snapshot per scope is the highest
        // version. Global rows store `scope_id = ''` (not NULL) so the UNIQUE
        // constraint actually guards against duplicate versions for the global
        // scope too (SQLite treats NULLs as distinct under UNIQUE).
        //
        // No FK to `dreaming_runs(id)`: a snapshot is a durable product whose
        // lifetime is independent of the audit run that produced it (runs are
        // retention-GC'd; snapshots are not). `source_run_id` is a plain
        // reference for provenance. (FKs on this connection are inert anyway —
        // `PRAGMA foreign_keys` is off, see top of `open`.)
        //
        // NOTE: `scope_id` (when `scope_type='agent'`) is agent-id-bearing and
        // is registered with `agent::migration` for the `default → ha-main`
        // rename (§11). Defensive: snapshots are only written once the user
        // opts into profile synthesis, long after the legacy id is gone.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_profile_snapshots (
                id TEXT PRIMARY KEY,
                scope_type TEXT NOT NULL,
                scope_id TEXT NOT NULL DEFAULT '',
                version INTEGER NOT NULL,
                body_md TEXT NOT NULL,
                source_run_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(scope_type, scope_id, version)
            );
            CREATE INDEX IF NOT EXISTS idx_memory_profile_snapshots_scope
                ON memory_profile_snapshots(scope_type, scope_id, version DESC);",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_profile_snapshot_sources (
                id TEXT PRIMARY KEY,
                snapshot_id TEXT NOT NULL,
                line_index INTEGER,
                claim_id TEXT NOT NULL,
                claim_type TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL,
                salience REAL NOT NULL,
                evidence_id TEXT,
                evidence_class TEXT,
                evidence_source_type TEXT,
                evidence_quote TEXT,
                evidence_session_id TEXT,
                evidence_message_id TEXT,
                evidence_file_path TEXT,
                evidence_url TEXT,
                created_at TEXT NOT NULL,
                UNIQUE(snapshot_id, line_index, claim_id)
            );
            CREATE INDEX IF NOT EXISTS idx_memory_profile_snapshot_sources_snapshot
                ON memory_profile_snapshot_sources(snapshot_id, line_index);
            CREATE INDEX IF NOT EXISTS idx_memory_profile_snapshot_sources_claim
                ON memory_profile_snapshot_sources(claim_id);",
        )?;
        // Additive migrations for dev/user DBs that created the profile source
        // sidecar before it carried evidence-level provenance.
        if conn
            .prepare("SELECT evidence_id FROM memory_profile_snapshot_sources LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_id TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_class TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_source_type TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_quote TEXT;",
            );
        }
        if conn
            .prepare("SELECT evidence_session_id FROM memory_profile_snapshot_sources LIMIT 0")
            .is_err()
        {
            let _ = conn.execute_batch(
                "ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_session_id TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_message_id TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_file_path TEXT;
                 ALTER TABLE memory_profile_snapshot_sources ADD COLUMN evidence_url TEXT;",
            );
        }

        // ── Episodic / procedural memory (P5 foundation) ───────────────
        //
        // Owner-plane memory of "what happened / what worked" and soft
        // workflow procedures. Procedures can be used as bounded soft prompt
        // guidance when the user/agent config allows it; episodes remain
        // trace-only. Agent tools do not write these tables.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_episodes (
                id TEXT PRIMARY KEY,
                scope_type TEXT NOT NULL DEFAULT 'global',
                scope_id TEXT,
                title TEXT NOT NULL,
                situation TEXT NOT NULL,
                actions_json TEXT NOT NULL DEFAULT '[]',
                outcome TEXT NOT NULL DEFAULT '',
                lesson TEXT NOT NULL DEFAULT '',
                source_session_id TEXT,
                source_message_ids_json TEXT NOT NULL DEFAULT '[]',
                success_score REAL NOT NULL DEFAULT 0.5,
                tags_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_episodes_scope
                ON memory_episodes(scope_type, scope_id, status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_episodes_status
                ON memory_episodes(status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_episodes_session
                ON memory_episodes(source_session_id, updated_at DESC);

            CREATE TABLE IF NOT EXISTS memory_procedures (
                id TEXT PRIMARY KEY,
                scope_type TEXT NOT NULL DEFAULT 'global',
                scope_id TEXT,
                title TEXT NOT NULL,
                trigger TEXT NOT NULL,
                steps_markdown TEXT NOT NULL,
                constraints_markdown TEXT NOT NULL DEFAULT '',
                confidence REAL NOT NULL DEFAULT 0.5,
                status TEXT NOT NULL DEFAULT 'active',
                source_episode_ids_json TEXT NOT NULL DEFAULT '[]',
                tags_json TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_procedures_scope
                ON memory_procedures(scope_type, scope_id, status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_procedures_status
                ON memory_procedures(status, updated_at DESC);

            -- Append-only owner audit stream for episodic / procedural memory.
            -- This is not a second source of truth and never participates in
            -- prompt injection or retrieval; it lets users inspect how a soft
            -- workflow that can affect replies changed over time.
            CREATE TABLE IF NOT EXISTS memory_experience_history (
                id TEXT PRIMARY KEY,
                target_kind TEXT NOT NULL,
                target_id TEXT NOT NULL,
                action TEXT NOT NULL,
                scope_type TEXT NOT NULL DEFAULT 'global',
                scope_id TEXT,
                title_preview TEXT NOT NULL DEFAULT '',
                content_preview TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_experience_history_target
                ON memory_experience_history(target_kind, target_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_experience_history_created
                ON memory_experience_history(created_at DESC);",
        )?;

        // ── Claim relevance search: FTS5 over memory_claims (PR #8 Context
        // Pack — "Relevant Claims" this turn). External-content FTS keyed on
        // memory_claims' implicit INTEGER rowid (the table PK is a TEXT uuid,
        // which can't be an FTS/vec rowid). Triggers keep it in sync; on first
        // creation we rebuild from rows written before this index existed.
        let claims_fts_existed = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_claims_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_claims_fts USING fts5(
                content, subject, object,
                content='memory_claims',
                content_rowid='rowid',
                tokenize='unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS memory_claims_ai AFTER INSERT ON memory_claims BEGIN
                INSERT INTO memory_claims_fts(rowid, content, subject, object)
                VALUES (new.rowid, new.content, new.subject, new.object);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_claims_ad AFTER DELETE ON memory_claims BEGIN
                INSERT INTO memory_claims_fts(memory_claims_fts, rowid, content, subject, object)
                VALUES ('delete', old.rowid, old.content, old.subject, old.object);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_claims_au AFTER UPDATE ON memory_claims BEGIN
                INSERT INTO memory_claims_fts(memory_claims_fts, rowid, content, subject, object)
                VALUES ('delete', old.rowid, old.content, old.subject, old.object);
                INSERT INTO memory_claims_fts(rowid, content, subject, object)
                VALUES (new.rowid, new.content, new.subject, new.object);
            END;",
        )?;
        if !claims_fts_existed {
            // Backfill claims written before this index existed (PR #3-7).
            let _ = conn.execute_batch(
                "INSERT INTO memory_claims_fts(memory_claims_fts) VALUES('rebuild');",
            );
        }

        let claims_literal_fts_existed = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_claims_literal_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_claims_literal_fts USING fts5(
                content, claim_type, subject, predicate, object, tags_json,
                content='memory_claims',
                content_rowid='rowid',
                tokenize='trigram'
            );
            CREATE TRIGGER IF NOT EXISTS memory_claims_literal_ai AFTER INSERT ON memory_claims BEGIN
                INSERT INTO memory_claims_literal_fts(
                    rowid, content, claim_type, subject, predicate, object, tags_json
                ) VALUES (
                    new.rowid, new.content, new.claim_type, new.subject,
                    new.predicate, new.object, new.tags_json
                );
            END;
            CREATE TRIGGER IF NOT EXISTS memory_claims_literal_ad AFTER DELETE ON memory_claims BEGIN
                INSERT INTO memory_claims_literal_fts(
                    memory_claims_literal_fts, rowid, content, claim_type,
                    subject, predicate, object, tags_json
                ) VALUES (
                    'delete', old.rowid, old.content, old.claim_type,
                    old.subject, old.predicate, old.object, old.tags_json
                );
            END;
            CREATE TRIGGER IF NOT EXISTS memory_claims_literal_au AFTER UPDATE ON memory_claims BEGIN
                INSERT INTO memory_claims_literal_fts(
                    memory_claims_literal_fts, rowid, content, claim_type,
                    subject, predicate, object, tags_json
                ) VALUES (
                    'delete', old.rowid, old.content, old.claim_type,
                    old.subject, old.predicate, old.object, old.tags_json
                );
                INSERT INTO memory_claims_literal_fts(
                    rowid, content, claim_type, subject, predicate, object, tags_json
                ) VALUES (
                    new.rowid, new.content, new.claim_type, new.subject,
                    new.predicate, new.object, new.tags_json
                );
            END;",
        )?;
        if !claims_literal_fts_existed {
            let _ = conn.execute_batch(
                "INSERT INTO memory_claims_literal_fts(memory_claims_literal_fts) VALUES('rebuild');",
            );
        }

        // Owner-plane claim search can match evidence metadata/quotes. Keep a
        // dedicated FTS index for those fields so large review queues do not
        // rely solely on bounded LIKE scans; the query path still keeps LIKE as
        // a literal fallback for CJK fragments and missing/corrupt FTS rows.
        let evidence_fts_existed = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_evidence_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_evidence_fts USING fts5(
                source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote,
                content='memory_evidence',
                content_rowid='rowid',
                tokenize='unicode61'
            );
            CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_ai AFTER INSERT ON memory_evidence BEGIN
                INSERT INTO memory_evidence_fts(rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                VALUES (new.rowid, new.source_type, new.evidence_class, new.source_id, new.session_id, new.message_id, new.file_path, new.url, new.quote);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_ad AFTER DELETE ON memory_evidence BEGIN
                INSERT INTO memory_evidence_fts(memory_evidence_fts, rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                VALUES ('delete', old.rowid, old.source_type, old.evidence_class, old.source_id, old.session_id, old.message_id, old.file_path, old.url, old.quote);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_evidence_fts_au AFTER UPDATE ON memory_evidence BEGIN
                INSERT INTO memory_evidence_fts(memory_evidence_fts, rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                VALUES ('delete', old.rowid, old.source_type, old.evidence_class, old.source_id, old.session_id, old.message_id, old.file_path, old.url, old.quote);
                INSERT INTO memory_evidence_fts(rowid, source_type, evidence_class, source_id, session_id, message_id, file_path, url, quote)
                VALUES (new.rowid, new.source_type, new.evidence_class, new.source_id, new.session_id, new.message_id, new.file_path, new.url, new.quote);
            END;",
        )?;
        if !evidence_fts_existed {
            let _ = conn.execute_batch(
                "INSERT INTO memory_evidence_fts(memory_evidence_fts) VALUES('rebuild');",
            );
        }

        // Claim embedding signature column (PR #8 claim vector retrieval).
        // Additive: dev DBs that created `memory_claims` before this column
        // existed (PR #2-7) get it backfilled here; the error on re-run (column
        // already present) is expected and ignored. The vec0 table
        // (`memory_claims_vec`) is created lazily on first embed, mirroring
        // `memories_vec`.
        let _ = conn.execute(
            "ALTER TABLE memory_claims ADD COLUMN embedding_signature TEXT",
            [],
        );

        // Create read-only connection pool for concurrent reads (WAL mode enables this)
        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let read_conn = Connection::open_with_flags(
                db_path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )
            .with_context(|| format!("Failed to open read connection at {}", db_path.display()))?;
            // Register sqlite-vec for read connections too
            read_conn.execute_batch("PRAGMA journal_mode=WAL;")?;
            read_conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
            read_conn.busy_timeout(std::time::Duration::from_secs(5))?;
            readers.push(Mutex::new(read_conn));
        }

        Ok(Self {
            writer: Mutex::new(conn),
            readers,
            reader_idx: std::sync::atomic::AtomicUsize::new(0),
            embedder: std::sync::RwLock::new(None),
            embedding_dims: std::sync::atomic::AtomicU32::new(0),
            db_path: db_path.to_path_buf(),
        })
    }

    /// Get a read connection from the pool (round-robin).
    /// Falls back to the writer connection if all readers are busy.
    pub(crate) fn read_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        let idx = self
            .reader_idx
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.readers.len();
        // Try the selected reader first, then cycle through others
        for i in 0..self.readers.len() {
            let target = (idx + i) % self.readers.len();
            if let Ok(guard) = self.readers[target].try_lock() {
                return Ok(guard);
            }
        }
        // All readers busy: block on the selected one
        self.readers[idx]
            .lock()
            .map_err(|e| anyhow::anyhow!("Read pool lock poisoned: {e}"))
    }

    /// Get the exclusive write connection.
    pub(crate) fn write_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|e| anyhow::anyhow!("Writer lock poisoned: {e}"))
    }

    /// Ensure the vec0 virtual table exists with the correct dimensions.
    pub(crate) fn ensure_vec_table(&self, conn: &Connection, dims: u32) -> Result<()> {
        let existing_sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_vec'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let expected_dim = format!("float[{}]", dims);
        if let Some(sql) = existing_sql {
            if !sql.contains(&expected_dim) {
                app_warn!(
                    "memory",
                    "embedding",
                    "Recreating memories_vec for embedding dimension change to {}",
                    dims
                );
                conn.execute_batch("DROP TABLE IF EXISTS memories_vec;")?;
            }
        }

        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(rowid INTEGER PRIMARY KEY, embedding float[{}])",
            dims
        );
        conn.execute_batch(&sql)?;
        Ok(())
    }

    /// Generate embedding for text, with optional caching to reduce API calls.
    pub(crate) fn generate_embedding(&self, text: &str) -> Option<Vec<f32>> {
        let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
        let embedder = guard.as_ref()?;

        let cache_cfg = crate::memory::helpers::load_embedding_cache_config();
        if !cache_cfg.enabled {
            return embedder.embed(text).ok();
        }

        // Compute content hash
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash_str = format!("{:016x}", hasher.finish());

        // Load provider/model/signature info from config for cache key
        let store = crate::config::cached_config();
        let (provider_key, model_key, signature_key) =
            crate::memory::resolve_memory_embedding_config(
                &store.memory_embedding,
                &store.embedding_models,
            )
            .ok()
            .flatten()
            .map(|(model, _runtime, signature)| {
                (
                    format!("{:?}", model.provider_type),
                    model.api_model.unwrap_or_default(),
                    signature,
                )
            })
            .unwrap_or_else(|| {
                (
                    format!("{:?}", store.embedding.provider_type),
                    store.embedding.api_model.clone().unwrap_or_default(),
                    "legacy".to_string(),
                )
            });

        // Check cache (read-only)
        if let Ok(conn) = self.read_conn() {
            let cached: Option<Vec<u8>> = conn.query_row(
                "SELECT embedding FROM embedding_cache WHERE hash = ?1 AND provider = ?2 AND model = ?3 AND signature = ?4",
                params![hash_str, provider_key, model_key, signature_key],
                |row| row.get(0),
            ).optional().unwrap_or(None);

            if let Some(bytes) = cached {
                // Deserialize f32 bytes
                let floats: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                if !floats.is_empty() {
                    return Some(floats);
                }
            }
        }

        // Cache miss: compute embedding
        let emb = embedder.embed(text).ok()?;

        // Store in cache (write)
        if let Ok(conn) = self.write_conn() {
            let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            let dims = emb.len() as i64;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embedding_cache (hash, provider, model, signature, embedding, dimensions, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
                params![hash_str, provider_key, model_key, signature_key, emb_bytes, dims],
            );

            // Prune is amortized: only ~1/PRUNE_SAMPLE writes run the COUNT+delete.
            // embedding_cache is an LRU cache (rebuildable, not a source of truth),
            // so transiently overshooting max_entries by a sample window is harmless
            // and avoids an O(n) COUNT(*) on every cache write (hot extraction path).
            if cache_cfg.max_entries > 0 {
                const PRUNE_SAMPLE: u64 = 64;
                static PRUNE_COUNTER: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let tick = PRUNE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if tick % PRUNE_SAMPLE == 0 {
                    let count: i64 = conn
                        .query_row("SELECT COUNT(*) FROM embedding_cache", [], |row| row.get(0))
                        .unwrap_or(0);
                    if count as usize > cache_cfg.max_entries {
                        let to_delete = (count as usize - cache_cfg.max_entries
                            + cache_cfg.max_entries / 10)
                            as i64;
                        let _ = conn.execute(
                            "DELETE FROM embedding_cache WHERE rowid IN (SELECT rowid FROM embedding_cache ORDER BY created_at ASC LIMIT ?1)",
                            params![to_delete],
                        );
                    }
                }
            }
        }

        Some(emb)
    }

    /// Generate multimodal embedding for a file attachment + text label.
    /// Falls back to text-only if provider doesn't support multimodal or file is invalid.
    pub(crate) fn generate_multimodal_embedding(
        &self,
        label: &str,
        file_path: &str,
        mime_type: &str,
    ) -> Option<Vec<f32>> {
        let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
        let embedder = guard.as_ref()?;

        // Check multimodal config
        let mm_cfg = crate::memory::helpers::load_multimodal_config();
        if !mm_cfg.enabled {
            return embedder.embed(label).ok();
        }

        if !embedder.supports_multimodal() {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "info",
                    "memory",
                    "embedding::multimodal",
                    "Embedding provider does not support multimodal, falling back to text-only",
                    None,
                    None,
                    None,
                );
            }
            return embedder.embed(label).ok();
        }

        // Validate file
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "memory",
                    "embedding::multimodal",
                    &format!("Attachment file not found: {}", file_path),
                    None,
                    None,
                    None,
                );
            }
            return embedder.embed(label).ok();
        }

        let metadata = std::fs::metadata(path).ok()?;
        if metadata.len() > mm_cfg.max_file_bytes {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "warn",
                    "memory",
                    "embedding::multimodal",
                    &format!(
                        "Attachment too large: {} bytes > {} max",
                        metadata.len(),
                        mm_cfg.max_file_bytes
                    ),
                    None,
                    None,
                    None,
                );
            }
            return embedder.embed(label).ok();
        }

        let file_data = std::fs::read(path).ok()?;
        let input = crate::memory::traits::MultimodalInput {
            label: label.to_string(),
            mime_type: mime_type.to_string(),
            file_data,
        };

        match embedder.embed_multimodal(&input) {
            Ok(emb) => Some(emb),
            Err(e) => {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "warn",
                        "memory",
                        "embedding::multimodal",
                        &format!("Multimodal embedding failed, falling back to text: {}", e),
                        None,
                        None,
                        None,
                    );
                }
                embedder.embed(label).ok()
            }
        }
    }

    /// Re-generate embeddings for a set of entries and update the DB.
    pub(crate) fn reembed_entries(&self, entries: &[MemoryEntry]) -> Result<usize> {
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims == 0 {
            return Err(anyhow::anyhow!("No embedding provider configured"));
        }
        let signature = crate::memory::helpers::active_embedding_signature()
            .ok_or_else(|| anyhow::anyhow!("No active memory embedding model configured"))?;
        let mut updates: Vec<(i64, Vec<u8>)> = Vec::new();

        // Try async Batch API for bulk re-embedding (cheaper + faster for large batches)
        let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
        let use_batch =
            guard.as_ref().map_or(false, |e| e.supports_batch_api()) && entries.len() >= 10;
        drop(guard);

        if use_batch {
            // Collect text-only entries (skip multimodal for batch)
            let batch_items: Vec<(String, String)> = entries
                .iter()
                .filter(|e| e.attachment_path.is_none())
                .map(|e| (e.id.to_string(), e.content.clone()))
                .collect();

            if !batch_items.is_empty() {
                if let Some(logger) = crate::get_logger() {
                    logger.log(
                        "info",
                        "memory",
                        "embedding::reembed",
                        &format!("Using async Batch API for {} entries", batch_items.len()),
                        None,
                        None,
                        None,
                    );
                }

                let guard = self.embedder.read().unwrap_or_else(|e| e.into_inner());
                if let Some(embedder) = guard.as_ref() {
                    match embedder.embed_batch_async(&batch_items) {
                        Ok(results) => {
                            for (id_str, emb) in &results {
                                let id: i64 = id_str.parse().unwrap_or(0);
                                if id == 0 {
                                    continue;
                                }
                                let emb_bytes: Vec<u8> =
                                    emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                                updates.push((id, emb_bytes));
                            }

                            // Handle multimodal entries with synchronous fallback before taking the
                            // SQLite writer lock, because providers may perform network I/O here.
                            for entry in entries.iter().filter(|e| e.attachment_path.is_some()) {
                                if let Some(emb) = self.generate_multimodal_embedding(
                                    &entry.content,
                                    entry.attachment_path.as_deref().unwrap_or(""),
                                    entry.attachment_mime.as_deref().unwrap_or(""),
                                ) {
                                    let emb_bytes: Vec<u8> =
                                        emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                                    updates.push((entry.id, emb_bytes));
                                }
                            }

                            return self.write_reembedded_entries(&updates, &signature, dims);
                        }
                        Err(e) => {
                            if let Some(logger) = crate::get_logger() {
                                logger.log(
                                    "warn",
                                    "memory",
                                    "embedding::reembed",
                                    &format!(
                                        "Batch API failed, falling back to synchronous: {}",
                                        e
                                    ),
                                    None,
                                    None,
                                    None,
                                );
                            }
                            // Fall through to synchronous path
                        }
                    }
                }
            }
        }

        // Synchronous fallback: embed one by one
        for entry in entries {
            let emb = if let (Some(ref att_path), Some(ref att_mime)) =
                (&entry.attachment_path, &entry.attachment_mime)
            {
                self.generate_multimodal_embedding(&entry.content, att_path, att_mime)
            } else {
                self.generate_embedding(&entry.content)
            };
            if let Some(emb) = emb {
                let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                updates.push((entry.id, emb_bytes));
            }
        }
        self.write_reembedded_entries(&updates, &signature, dims)
    }

    fn write_reembedded_entries(
        &self,
        updates: &[(i64, Vec<u8>)],
        signature: &str,
        dims: u32,
    ) -> Result<usize> {
        if updates.is_empty() {
            return Ok(0);
        }

        let conn = self.write_conn()?;
        self.ensure_vec_table(&conn, dims)?;

        conn.execute_batch("BEGIN")?;
        let write_result = (|| -> Result<usize> {
            let mut count = 0usize;
            for (id, emb_bytes) in updates {
                conn.execute(
                    "UPDATE memories SET embedding = ?1, embedding_signature = ?2 WHERE id = ?3",
                    params![emb_bytes, signature, id],
                )?;
                let _ = conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![id]);
                let _ = conn.execute(
                    "INSERT INTO memories_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![id, emb_bytes],
                );
                count += 1;
            }
            Ok(count)
        })();

        match write_result {
            Ok(count) => {
                conn.execute_batch("COMMIT")?;
                Ok(count)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Ensure the claim vec0 virtual table exists with the correct dimensions
    /// (PR #8 claim vector retrieval). Mirrors [`Self::ensure_vec_table`] but
    /// for `memory_claims_vec`, keyed on `memory_claims`' implicit INTEGER
    /// rowid (the table PK is a TEXT uuid). Recreated on dimension change.
    pub(crate) fn ensure_claims_vec_table(&self, conn: &Connection, dims: u32) -> Result<()> {
        let existing_sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memory_claims_vec'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let expected_dim = format!("float[{}]", dims);
        if let Some(sql) = existing_sql {
            if !sql.contains(&expected_dim) {
                app_warn!(
                    "memory",
                    "embedding",
                    "Recreating memory_claims_vec for embedding dimension change to {}",
                    dims
                );
                conn.execute_batch("DROP TABLE IF EXISTS memory_claims_vec;")?;
            }
        }
        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_claims_vec USING vec0(rowid INTEGER PRIMARY KEY, embedding float[{}])",
            dims
        );
        conn.execute_batch(&sql)?;
        Ok(())
    }

    /// Generate + persist one claim's embedding into `memory_claims_vec` and
    /// stamp `memory_claims.embedding_signature` (PR #8). Best-effort: a missing
    /// embedder, dim=0, or a vanished claim row is a silent no-op (FTS-only
    /// recall still works; the reembed job backfills later).
    ///
    /// MUST be called WITHOUT holding the write lock: `generate_embedding`
    /// itself acquires `write_conn` to persist the embedding cache, so calling
    /// this while the caller already holds `write_conn` would deadlock (the
    /// writer `Mutex` is not re-entrant). Callers that wrote the claim under a
    /// tx must `drop` their guard first.
    pub(crate) fn embed_and_index_claim(&self, claim_id: &str, content: &str) {
        let dims = self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed);
        if dims == 0 {
            return;
        }
        let Some(emb) = self.generate_embedding(content) else {
            return;
        };
        let Some(signature) = crate::memory::helpers::active_embedding_signature() else {
            return;
        };
        let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        let Ok(conn) = self.write_conn() else {
            return;
        };
        if self.ensure_claims_vec_table(&conn, dims).is_err() {
            return;
        }
        let rowid: Option<i64> = conn
            .query_row(
                "SELECT rowid FROM memory_claims WHERE id = ?1",
                params![claim_id],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None);
        let Some(rowid) = rowid else {
            return;
        };
        let _ = conn.execute(
            "UPDATE memory_claims SET embedding_signature = ?1 WHERE id = ?2",
            params![signature, claim_id],
        );
        let _ = conn.execute(
            "DELETE FROM memory_claims_vec WHERE rowid = ?1",
            params![rowid],
        );
        let _ = conn.execute(
            "INSERT INTO memory_claims_vec(rowid, embedding) VALUES (?1, ?2)",
            params![rowid, emb_bytes],
        );
    }

    /// Re-embed every active claim (PR #8). Claims share the memory embedding
    /// model, so the `MemoryReembed` job calls this after re-embedding memories
    /// on a model switch. Best-effort + cancel-aware; claim volume is small so
    /// a per-claim acquire/release of the write lock is acceptable.
    pub(crate) fn reembed_claims(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<usize> {
        if self
            .embedding_dims
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return Ok(0);
        }
        let claims: Vec<(String, String)> = {
            let conn = self.read_conn()?;
            let mut stmt =
                conn.prepare("SELECT id, content FROM memory_claims WHERE status = 'active'")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut count = 0usize;
        for (id, content) in claims {
            if cancel.is_cancelled() {
                break;
            }
            self.embed_and_index_claim(&id, &content);
            count += 1;
        }
        Ok(count)
    }
}

// ── Helper: scope -> SQL conditions ──────────────────────────────

/// Returns (where_clause, params) for scope filtering.
/// `agent_id` is an optional shorthand that means "global + this agent".
///
/// Note: the `agent_id` shorthand deliberately does **not** include project
/// memories. Project scope is narrower than agent scope and must be queried
/// explicitly via `MemoryScope::Project { id }` to avoid leaking project
/// context into unrelated sessions.
pub(crate) fn scope_where(
    scope: Option<&MemoryScope>,
    agent_id: Option<&str>,
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    if let Some(scope) = scope {
        match scope {
            MemoryScope::Global => ("scope_type = 'global'".to_string(), Vec::new()),
            MemoryScope::Agent { id } => (
                "scope_type = 'agent' AND scope_agent_id = ?".to_string(),
                vec![Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>],
            ),
            MemoryScope::Project { id } => (
                "scope_type = 'project' AND scope_project_id = ?".to_string(),
                vec![Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>],
            ),
        }
    } else if let Some(aid) = agent_id {
        (
            "(scope_type = 'global' OR (scope_type = 'agent' AND scope_agent_id = ?))".to_string(),
            vec![Box::new(aid.to_string()) as Box<dyn rusqlite::types::ToSql>],
        )
    } else {
        ("1=1".to_string(), Vec::new())
    }
}

/// Parse a row into MemoryEntry.
pub(crate) fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
    let scope_type: String = row.get("scope_type")?;
    let scope_agent_id: Option<String> = row.get("scope_agent_id")?;
    // scope_project_id may not be present on rows selected before the
    // migration-added column; tolerate its absence.
    let scope_project_id: Option<String> = row.get("scope_project_id").ok().flatten();
    let tags_json: String = row.get("tags")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

    let scope = match scope_type.as_str() {
        "agent" => MemoryScope::Agent {
            id: scope_agent_id.unwrap_or_default(),
        },
        "project" => MemoryScope::Project {
            id: scope_project_id.unwrap_or_default(),
        },
        _ => MemoryScope::Global,
    };

    let memory_type_str: String = row.get("memory_type")?;

    let pinned_int: i64 = row.get("pinned").unwrap_or(0);

    Ok(MemoryEntry {
        id: row.get("id")?,
        memory_type: MemoryType::from_str(&memory_type_str),
        scope,
        content: row.get("content")?,
        tags,
        source: row.get("source")?,
        source_session_id: row.get("source_session_id")?,
        pinned: pinned_int != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        relevance_score: None,
        retrieval_evidence: None,
        attachment_path: row.get("attachment_path").ok().flatten(),
        attachment_mime: row.get("attachment_mime").ok().flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_table_sql(conn: &Connection) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_vec'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("memories_vec sql")
    }

    #[test]
    fn ensure_vec_table_recreates_when_dimensions_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("memory.db");
        let backend = SqliteMemoryBackend::open(&db_path).expect("open backend");
        let conn = backend.write_conn().expect("write conn");

        backend.ensure_vec_table(&conn, 384).expect("create 384");
        assert!(vec_table_sql(&conn).contains("float[384]"));

        backend.ensure_vec_table(&conn, 768).expect("recreate 768");
        assert!(vec_table_sql(&conn).contains("float[768]"));
        assert!(!vec_table_sql(&conn).contains("float[384]"));
    }

    fn claims_vec_table_sql(conn: &Connection) -> String {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memory_claims_vec'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("memory_claims_vec sql")
    }

    #[test]
    fn ensure_claims_vec_table_recreates_when_dimensions_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("memory.db");
        let backend = SqliteMemoryBackend::open(&db_path).expect("open backend");
        let conn = backend.write_conn().expect("write conn");

        backend
            .ensure_claims_vec_table(&conn, 384)
            .expect("create 384");
        assert!(claims_vec_table_sql(&conn).contains("float[384]"));

        backend
            .ensure_claims_vec_table(&conn, 768)
            .expect("recreate 768");
        assert!(claims_vec_table_sql(&conn).contains("float[768]"));
        assert!(!claims_vec_table_sql(&conn).contains("float[384]"));
    }

    #[test]
    fn claims_table_exposes_embedding_signature_column() {
        // DDL + the defensive ALTER must leave `embedding_signature` queryable
        // (the claim vector retrieval signature filter depends on it).
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("memory.db");
        let backend = SqliteMemoryBackend::open(&db_path).expect("open backend");
        let conn = backend.write_conn().expect("write conn");
        assert!(
            conn.prepare("SELECT embedding_signature FROM memory_claims LIMIT 0")
                .is_ok(),
            "memory_claims must expose embedding_signature"
        );
    }
}
